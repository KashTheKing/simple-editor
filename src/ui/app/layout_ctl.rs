//! ---- ws:layout-modes-onboarding ----
//! Layout control: the ACT_HANDLERS entry for the page actions (Workspace1..4) / MaximizePane /
//! TogglePin / ToggleSource plus the two variants command-palette declared for us to consume
//! (ToggleLayoutMode, ShowWelcome); the pin/follow-aware `surface`; `poll_popout` (the body wave-0b's
//! `on_viewport` hook in `layout::show` was pre-placed for, so Space/J/K/L work in a torn-off Preview);
//! and the WINDOW_DRAWERS glue for the welcome wizard (`ui::onboarding`) and the home screen
//! (`ui::home`), which are App-free on purpose.
//!
//! ---- ws:pages ----
//! The pages: `switch_page` (stash this page's tree, bring the other one back), `reset_page`, the
//! menu-bar `page_switcher`, and the one-time `migrate_to_pages` of a pre-pages settings file.
//! Everything here mutates Settings / Layout / UI state only - never the project, so no
//! `push_undo_labeled` anywhere except the one place the wizard applies a starting format to an EMPTY
//! project (that is a project edit and gets its own labelled undo entry).

use super::*;
use crate::ui::layout::{page_layout, page_name, Surfaced, PAGES};
use crate::ui::onboarding::{self, Onboarding, Outcome};
use crate::ui::{home, menu};

/// The name the pre-pages arrangement is kept under (Window ▸ Layout ▸ Load profile).
pub(super) const BEFORE_UPDATE: &str = "Before update";

/// Panel tabs follow the selection unless Settings say "granular" - one reading for every caller.
pub(super) fn is_dynamic(settings: &Settings) -> bool {
    settings.layout_mode != "granular"
}

/// The page actions, in `PAGES` order (Alt+1..4).
pub(super) const PAGE_ACTIONS: [Action; 4] =
    [Action::Workspace1, Action::Workspace2, Action::Workspace3, Action::Workspace4];

pub(super) fn act(app: &mut App, a: Action) -> bool {
    if let Some(i) = PAGE_ACTIONS.iter().position(|&p| p == a) {
        switch_page(app, PAGES[i]);
        return true;
    }
    match a {
        Action::MaximizePane => {
            if app.layout.maximized.is_some() {
                app.layout.unmaximize();
                app.layout_dirty = true;
            } else if let Some(p) = app.layout.hovered {
                app.layout.maximize(p);
                app.layout_dirty = true;
            } else {
                let key = app.hotkeys.text(Action::MaximizePane);
                app.toast(format!("Hover a panel, then press {key} to maximise it ({key} again restores)"));
            }
            true
        }
        Action::TogglePin => {
            match app.layout.hovered {
                Some(p) => {
                    let on = app.layout.toggle_pin(p);
                    app.layout_dirty = true;
                    let state = if on { "stays in front" } else { "follows the selection again" };
                    app.toast(format!("{} {state}", p.title()));
                }
                None => app.toast("Hover a panel first (or right-click its tab ▸ Stay on this tab)"),
            }
            true
        }
        Action::ToggleSource => {
            app.toggle_pane(Pane::Source);
            true
        }
        // declared by ws:command-palette in hotkeys.rs, consumed here (audit fix 2)
        Action::ToggleLayoutMode => {
            let now_dynamic = !is_dynamic(&app.settings);
            set_mode(app, now_dynamic);
            app.toast(if now_dynamic {
                "Panel tabs follow the selection (right-click a tab ▸ Stay on this tab to keep it in front)"
            } else {
                "Panel tabs stay put; the tab that could help just glows"
            });
            true
        }
        Action::ShowWelcome => {
            app.settings.onboarded = false;
            app.settings.save();
            app.onboarding = Some(Onboarding::new(&app.settings));
            true
        }
        _ => false,
    }
}

/// Persist a follow-the-selection change and re-evaluate the current selection under the new rules
/// right away, so Settings ▸ General / the palette / `layout.mode` all feel immediate.
pub(super) fn set_mode(app: &mut App, dynamic: bool) {
    app.settings.layout_mode = if dynamic { "dynamic" } else { "granular" }.into();
    app.settings.save();
    let kind = frame::selection_kind_of(app);
    app.tools.lead = if dynamic { lead_tool(kind) } else { None };
    surface_for_kind(app, kind);
}

/// Panes a selection kind would like in front, best first: the first one the layout can actually
/// switch to (or glow) wins. Audio prefers the Mixer but settles for the Inspector's audio section
/// when the Mixer is stacked away (the Edit page); `None`/`Mixed`/`EditPoint` ask for nothing - the
/// Timeline is always there and a mixed bag has no single home.
pub(super) fn panes_for(kind: SelectionKind) -> &'static [Pane] {
    match kind {
        SelectionKind::Text
        | SelectionKind::Video
        | SelectionKind::Shape
        | SelectionKind::Sequence
        | SelectionKind::Adjustment => &[Pane::Inspector],
        SelectionKind::Audio => &[Pane::Mixer, Pane::Inspector],
        SelectionKind::Transition => &[Pane::Transitions, Pane::Inspector],
        SelectionKind::Cue => &[Pane::Subtitles],
        SelectionKind::None | SelectionKind::Mixed | SelectionKind::EditPoint => &[],
    }
}

/// The tool the adaptive strip moves to the front for a selection kind (`None` = the fixed order): text
/// edits want the Text tool, shapes a shape tool, an edit point the razor.
pub(super) fn lead_tool(kind: SelectionKind) -> Option<crate::ui::tools::Tool> {
    use crate::ui::tools::Tool;
    match kind {
        SelectionKind::Text | SelectionKind::Cue => Some(Tool::Text),
        SelectionKind::Shape => Some(Tool::Shape(crate::model::ShapeKind::Rect)),
        SelectionKind::Adjustment => Some(Tool::Mask(MaskShape::Rect)),
        SelectionKind::EditPoint => Some(Tool::Cut),
        _ => None,
    }
}

/// Selection-driven surfacing, pure over a `Layout`: following the selection switches the tab when
/// `reveal_auto` says it may and glows it when a pin refuses; not following NEVER switches, it only
/// glows the tab that would have helped. `Pinned` is returned for "handled by a glow" either way so a
/// caller trying several candidate panes stops at the first one that is actually on screen.
pub(super) fn react(layout: &mut Layout, dynamic: bool, pane: Pane, now: Instant) -> Surfaced {
    if dynamic {
        let r = layout.reveal_auto(pane);
        if r == Surfaced::Pinned {
            layout.push_glow(pane, now);
        }
        r
    } else {
        match layout.can_surface(pane) {
            Surfaced::Shown | Surfaced::Pinned => {
                layout.push_glow(pane, now);
                Surfaced::Pinned
            }
            other => other,
        }
    }
}

/// Upgrades wave-0b's `App::surface` (which stays the EXPLICIT "Show X" reveal the palette uses) with
/// the pin/follow-aware automatic variant frame::tick calls on a selection change.
pub(super) fn surface(app: &mut App, pane: Pane) -> Surfaced {
    let r = react(&mut app.layout, is_dynamic(&app.settings), pane, Instant::now());
    if r == Surfaced::Shown {
        app.layout_dirty = true;
    }
    r
}

/// Surface the first candidate pane of `kind` that is on screen (see `panes_for`).
pub(super) fn surface_for_kind(app: &mut App, kind: SelectionKind) {
    for &p in panes_for(kind) {
        if matches!(surface(app, p), Surfaced::Shown | Surfaced::Pinned) {
            return;
        }
    }
}

// ---- ws:pages ----

/// A page's default arrangement (the Edit page's for a name that isn't one).
pub(super) fn page_default(page: &str) -> Layout {
    page_layout(page).unwrap_or(Layout::default_layout)()
}

/// One-time move to pages, before the layout loads: a settings file from before them (no `page`)
/// keeps its old arrangement as the layout profile "Before update" and starts on the page its old
/// workspace maps to, from that page's new default. True when it changed anything.
pub(super) fn migrate_to_pages(settings: &mut Settings) -> bool {
    if PAGES.contains(&settings.page.as_str()) {
        return false;
    }
    if !settings.layout.is_empty() {
        let json = std::mem::take(&mut settings.layout);
        settings.layout_profiles.retain(|p| p.name != BEFORE_UPDATE);
        settings.layout_profiles.push(crate::settings::LayoutProfile { name: BEFORE_UPDATE.into(), json });
    }
    settings.page = page_name(&settings.workspace).unwrap_or("Edit").into();
    true
}

/// The pure core of `switch_page`: unmaximise, stash the current tree under the current page, bring
/// `page`'s own back (or its default), and forget layout history - the layout's own undo stack and
/// the app's "Rearranged panels" markers (`LAYOUT_STEP`) replay moves of the tree they were made on,
/// so after a switch Ctrl+Z would put another page's arrangement back. False = already there.
pub(super) fn swap_page(
    layout: &mut Layout,
    settings: &mut Settings,
    undo: &mut Vec<UndoEntry>,
    redo: &mut Vec<UndoEntry>,
    page: &'static str,
) -> bool {
    if settings.page == page {
        return false;
    }
    layout.unmaximize();
    let old = std::mem::replace(&mut settings.page, page.to_string());
    if !old.is_empty() {
        // popped windows ride along in the stash: they belong to the page they were popped on
        settings.page_layouts.insert(old, layout.to_json());
    }
    let stored = settings.page_layouts.remove(page);
    *layout = stored.as_deref().and_then(Layout::from_json).unwrap_or_else(|| page_default(page));
    settings.layout = layout.to_json();
    undo.retain(|e| e.json != LAYOUT_STEP);
    redo.retain(|e| e.json != LAYOUT_STEP);
    true
}

/// Switch to a page by name (a `PAGES` name or an old workspace name); unknown names toast and
/// return false.
pub(super) fn switch_page(app: &mut App, name: &str) -> bool {
    let Some(page) = page_name(name) else {
        app.toast(format!("No page called '{name}' (try {})", PAGES.join(", ")));
        return false;
    };
    let App { layout, settings, undo, redo, .. } = app;
    if swap_page(layout, settings, undo, redo, page) {
        app.layout_json = app.settings.layout.clone();
        app.settings.save();
    }
    true
}

/// Back to `page`'s default arrangement. The page on screen resets as one undoable layout step; another
/// page just forgets its stored tree.
pub(super) fn reset_page(app: &mut App, page: &'static str) {
    if app.settings.page == page {
        let before = app.layout.to_json();
        app.layout.reset(page_layout(page).unwrap_or(Layout::default_layout));
        app.layout.push_undo(before);
        push_undo_json(&mut app.undo, &mut app.redo, LAYOUT_STEP.to_owned());
        app.layout_dirty = true;
    } else if app.settings.page_layouts.remove(page).is_some() {
        app.settings.save();
    }
    app.toast(format!("{page} page: back to its default layout"));
}

/// The menu bar's page switcher: one text button per page, centred in the bar, the page on screen
/// filled with the accent. Right-click a page to reset its layout.
pub(super) fn page_switcher(app: &mut App, ui: &mut egui::Ui) {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let pad = ui.spacing().button_padding.x * 2.0 + 16.0;
    let widths: Vec<f32> = PAGES
        .iter()
        .map(|p| ui.painter().layout_no_wrap(p.to_string(), font.clone(), egui::Color32::PLACEHOLDER).size().x + pad)
        .collect();
    let gap = 2.0;
    let total = widths.iter().sum::<f32>() + gap * (PAGES.len() - 1) as f32;
    // centred in the whole bar (where the eye expects it), but never over the menus on a narrow window
    let bar = ui.max_rect();
    let left = (bar.center().x - total / 2.0).max(ui.cursor().left() + 8.0);
    let rect = egui::Rect::from_min_size(egui::pos2(left, bar.top()), egui::vec2(total, bar.height()));
    let (mut pick, mut reset) = (None, None);
    let accent = ui.visuals().selection.bg_fill;
    ui.painter().rect_filled(rect.expand2(egui::vec2(3.0, 0.0)), 5.0, ui.visuals().extreme_bg_color);
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for ((&page, &w), &a) in PAGES.iter().zip(&widths).zip(&PAGE_ACTIONS) {
                let on = app.settings.page == page;
                let color = if on { crate::ui::tools::on_accent(accent) } else { ui.visuals().text_color() };
                let mut b = egui::Button::new(egui::RichText::new(page).color(color))
                    .min_size(egui::vec2(w, 0.0))
                    .frame_when_inactive(on);
                if on {
                    b = b.fill(accent);
                }
                let r = ui.add(b).on_hover_text(format!("{page} page   {}", app.hotkeys.text(a)));
                if r.clicked() && !on {
                    pick = Some(page);
                }
                r.context_menu(|ui| {
                    if menu::row(ui, None, "Reset page layout", "").clicked() {
                        reset = Some(page);
                    }
                });
            }
        },
    );
    if let Some(page) = pick {
        switch_page(app, page);
    }
    if let Some(page) = reset {
        reset_page(app, page);
    }
}

/// The body of wave-0b's pre-placed `on_viewport` hook in `layout::show`: poll the action table on a
/// popped pane's OWN ctx (each immediate viewport has its own input state, so a keypress in a torn-off
/// Preview is only ever seen here - never by the root ctx's poll at the top of `App::update` - and
/// fires exactly once). Early pass only: the late pass exists so a hovered curve/node editor can claim
/// Delete/copy/paste first, and this hook runs BEFORE the popped pane draws.
pub(super) fn poll_popout(hotkeys: &Hotkeys, ctx: &egui::Context) -> Vec<Action> {
    hotkeys.poll(ctx)
}

// ---------------- WINDOW_DRAWERS glue ----------------

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    onboarding_window(app, ctx);
    home_window(app, ctx);
}

fn onboarding_window(app: &mut App, ctx: &egui::Context) {
    let Some(mut st) = app.onboarding.take() else { return };
    let out = onboarding::show(ctx, &mut st, &app.settings, &app.hotkeys);
    match out {
        None => app.onboarding = Some(st),
        Some(Outcome::ShowCheatSheet) => {
            app.cheat_sheet_open = true;
            app.onboarding = Some(st);
        }
        Some(Outcome::Dismiss) => {
            app.settings.onboarded = true;
            app.settings.save();
        }
        Some(Outcome::Finish) => finish_onboarding(app, &st),
    }
}

/// Finish: `onboarding::finish` (consent flag, the guarded install) plus the two things only the app
/// can do - apply a starting format to an EMPTY project (one labelled undo entry) and persist.
fn finish_onboarding(app: &mut App, st: &Onboarding) {
    let allowed = onboarding::install_allowed(cfg!(debug_assertions), crate::contextmenu::is_installed());
    let mut install_err = None;
    let installed = onboarding::finish(st, &mut app.settings, allowed, &mut || {
        if let Err(e) = crate::contextmenu::install() {
            install_err = Some(e.to_string());
        }
    });
    app.settings.save();
    if let Some(i) = st.template {
        apply_format(app, i);
    }
    match (installed, install_err) {
        (true, Some(e)) => app.toast(format!("Context menu: {e}")),
        (true, None) => app.toast("Added 'Edit with Simple Editor' to Explorer's right-click menu"),
        _ => {}
    }
    // the live bindings, or where the Help menu has them if the user unbound one
    let key =
        |a: Action| Some(app.hotkeys.text(a)).filter(|k| !k.is_empty()).unwrap_or(format!("Help ▸ {}", a.label()));
    let (keys, palette) = (key(Action::CheatSheet), key(Action::CommandPalette));
    app.toast(format!("Welcome! {keys} lists every shortcut, {palette} searches every command"));
}

/// Set the project's format from `guides::PRESETS[i]` - only on an empty project (the wizard / home
/// screen both gate on that), as one labelled undo step like the inspector's own format buttons.
fn apply_format(app: &mut App, i: usize) {
    let Some(p) = crate::ui::guides::PRESETS.get(i) else { return };
    if !(app.project.is_empty() && app.project.assets.is_empty()) {
        return;
    }
    let before = app.project.to_json();
    app.project.width = p.w;
    app.project.height = p.h;
    app.project.fps = p.fps;
    app.push_undo_labeled(before, "Project format");
    app.after_edit();
    if app.settings.guide != p.guide {
        app.settings.guide = p.guide;
        app.settings.save();
    }
    app.toast(format!("Project format: {} ({}×{} at {:.0} fps)", p.name, p.w, p.h, p.fps));
}

fn home_window(app: &mut App, ctx: &egui::Context) {
    if app.onboarding.is_some() {
        return; // the wizard comes first; the cards appear once it is done
    }
    let empty = app.project.is_empty() && app.project.assets.is_empty();
    let Some(action) = home::show(ctx, &app.settings, &app.hotkeys, empty, app.home_dismissed) else { return };
    match action {
        home::HomeAction::Open => app.pending_actions.push(Action::OpenFile),
        home::HomeAction::Import => app.pending_actions.push(Action::ImportMedia),
        home::HomeAction::New(Some(i)) => {
            apply_format(app, i);
            app.home_dismissed = true;
        }
        home::HomeAction::New(None) | home::HomeAction::Dismiss => app.home_dismissed = true,
        home::HomeAction::OpenRecent(path) => {
            let path = PathBuf::from(path);
            app.confirm_discard_then(move |app| app.open_project(&path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers};

    /// Table-driven: the pane a selection kind auto-surfaces (first candidate), and the kinds that
    /// deliberately surface nothing.
    #[test]
    fn selection_kind_maps_to_expected_pane() {
        for (kind, want) in [
            (SelectionKind::Text, Some(Pane::Inspector)),
            (SelectionKind::Video, Some(Pane::Inspector)),
            (SelectionKind::Shape, Some(Pane::Inspector)),
            (SelectionKind::Sequence, Some(Pane::Inspector)),
            (SelectionKind::Adjustment, Some(Pane::Inspector)),
            (SelectionKind::Audio, Some(Pane::Mixer)),
            (SelectionKind::Transition, Some(Pane::Transitions)),
            (SelectionKind::Cue, Some(Pane::Subtitles)),
            (SelectionKind::None, None),
            (SelectionKind::Mixed, None),
            (SelectionKind::EditPoint, None),
        ] {
            assert_eq!(panes_for(kind).first().copied(), want, "{kind:?}");
        }
        // Audio falls back to the Inspector's audio section where the Mixer is stacked away (Edit)
        assert_eq!(panes_for(SelectionKind::Audio), &[Pane::Mixer, Pane::Inspector]);
        let mut edit = Layout::default_layout();
        let now = Instant::now();
        assert_eq!(react(&mut edit, true, Pane::Mixer, now), Surfaced::Hidden, "Mixer is stacked hidden on Edit");
        assert_eq!(react(&mut edit, true, Pane::Inspector, now), Surfaced::Shown);
        // the adaptive strip's lead tool
        use crate::ui::tools::Tool;
        assert_eq!(lead_tool(SelectionKind::Text), Some(Tool::Text));
        assert_eq!(lead_tool(SelectionKind::EditPoint), Some(Tool::Cut));
        assert_eq!(lead_tool(SelectionKind::Video), None);
        assert_eq!(lead_tool(SelectionKind::None), None);
    }

    /// A synthetic keydown on a popped viewport's ctx yields exactly one Action from `poll_popout`,
    /// and the same frame's root-style `hotkeys.poll(ctx)` does not emit it again (the event was
    /// consumed) - so a torn-off Preview's Space/J/K/L never double-fire.
    #[test]
    fn popout_hotkeys_reach_pending_actions_once() {
        let hk = Hotkeys::defaults();
        let ctx = egui::Context::default();
        let press = |key: Key| egui::RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        for (key, want) in [
            (Key::Space, Action::PlayPause),
            (Key::J, Action::ShuttleBack),
            (Key::L, Action::ShuttleFwd),
            (Key::K, Action::Stop),
        ] {
            let (mut popped, mut root) = (Vec::new(), Vec::new());
            let _ = ctx.run(press(key), |ctx| {
                popped = poll_popout(&hk, ctx);
                root = hk.poll(ctx);
            });
            assert_eq!(popped, vec![want], "{key:?} on the popped ctx");
            assert!(root.is_empty(), "{key:?} must not fire a second time from the root poll");
        }
        // and a second, separate context (the root viewport) sees nothing of it at all
        let other = egui::Context::default();
        let mut root = Vec::new();
        let _ = other.run(egui::RawInput::default(), |ctx| root = hk.poll(ctx));
        assert!(root.is_empty());
    }

    #[test]
    fn is_dynamic_reads_the_mode_string() {
        let mut s = Settings::default();
        assert!(is_dynamic(&s), "fresh settings follow the selection");
        s.layout_mode = "granular".into();
        assert!(!is_dynamic(&s));
        s.layout_mode = "anything-else".into();
        assert!(is_dynamic(&s), "only an explicit 'granular' switches auto-surfacing off");
    }

    // ---- ws:pages ----

    fn tree_value(l: &Layout) -> serde_json::Value {
        let mut v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&l.tree).unwrap()).unwrap();
        if let Some(inv) = v.pointer_mut("/tiles/invisible").and_then(|i| i.as_array_mut()) {
            inv.sort_by_key(|x| x.as_u64());
        }
        v
    }

    fn layout_step() -> UndoEntry {
        UndoEntry { json: LAYOUT_STEP.to_owned(), label: String::new(), at: 0.0, category: HistoryCategory::Layout }
    }

    /// Every page keeps its own arrangement through switches: edit A, visit B, come back - A's edit
    /// (and its popped-out window) is still there, B came up as its default, and the page names land
    /// in Settings the way `App::new` reads them back.
    #[test]
    fn each_page_round_trips_its_own_layout() {
        let mut s = Settings { page: "Edit".into(), ..Default::default() };
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        let mut l = Layout::default_layout();
        l.toggle(Pane::Inspector);
        l.popout(Pane::Library);
        let edited = tree_value(&l);
        for &other in &PAGES[1..] {
            assert!(swap_page(&mut l, &mut s, &mut undo, &mut redo, other));
            assert_eq!(s.page, other);
            assert_eq!(tree_value(&l), tree_value(&page_default(other)), "{other} starts from its default");
            assert!(l.popped.is_empty(), "Edit's popped window stays with Edit");
            assert_eq!(s.layout, l.to_json(), "Settings.layout is the page on screen");
            assert!(s.page_layouts.contains_key("Edit") && !s.page_layouts.contains_key(other));
            // an edit on this page too, to prove it survives the next hop
            l.toggle(Pane::Timeline);
            assert!(swap_page(&mut l, &mut s, &mut undo, &mut redo, "Edit"));
            assert_eq!(tree_value(&l), edited, "Edit's own edits are back after visiting {other}");
            assert_eq!(l.popped, vec![Pane::Library]);
            assert!(swap_page(&mut l, &mut s, &mut undo, &mut redo, other));
            assert!(!l.is_visible(Pane::Timeline), "{other}'s edit survived too");
            l.toggle(Pane::Timeline);
            assert!(swap_page(&mut l, &mut s, &mut undo, &mut redo, "Edit"));
        }
        assert!(!swap_page(&mut l, &mut s, &mut undo, &mut redo, "Edit"), "already there");
        // a maximised page is restored before it is stashed
        l.maximize(Pane::Timeline);
        swap_page(&mut l, &mut s, &mut undo, &mut redo, "Color");
        swap_page(&mut l, &mut s, &mut undo, &mut redo, "Edit");
        assert!(l.maximized.is_none());
        assert_eq!(tree_value(&l), edited);
    }

    /// After a switch neither Ctrl+Z path can bring another page's tree onto this one: the layout's own
    /// history is gone and the app's "Rearranged panels" markers are stripped (project edits stay).
    #[test]
    fn undo_after_a_switch_cannot_restore_another_pages_tree() {
        let mut s = Settings { page: "Edit".into(), ..Default::default() };
        let mut l = Layout::default_layout();
        let edit_json = l.to_json();
        l.toggle(Pane::Library); // a rearrangement on Edit, undoable while we stay here
        l.push_undo(edit_json);
        let project =
            UndoEntry { json: "{}".into(), label: String::new(), at: 0.0, category: HistoryCategory::Editing };
        let (mut undo, mut redo) = (vec![project.clone(), layout_step()], vec![layout_step()]);
        swap_page(&mut l, &mut s, &mut undo, &mut redo, "Audio");
        assert!(undo.iter().chain(&redo).all(|e| e.json != LAYOUT_STEP), "layout markers stripped");
        assert_eq!(undo.len(), 1, "the project edit stays undoable");
        assert!(!l.undo() && !l.redo(), "no layout history crossed the switch");
        assert_eq!(tree_value(&l), tree_value(&Layout::audio_layout()));
    }

    /// A settings file from before pages keeps its arrangement as the "Before update" profile and
    /// starts on the page its workspace maps to, from that page's default; a second run changes nothing.
    #[test]
    fn old_workspace_settings_migrate_to_pages() {
        let old_tree = Layout::default_layout().to_json();
        for (workspace, page) in [
            ("Simple", "Edit"),
            ("Edit", "Edit"),
            ("Text", "Edit"),
            ("Color", "Color"),
            ("Audio", "Audio"),
            ("Deliver", "Export"),
        ] {
            let json = serde_json::json!({ "workspace": workspace, "layout": old_tree, "layout_profiles": [
                { "name": BEFORE_UPDATE, "json": "stale" }, { "name": "Mine", "json": "{}" } ] });
            let mut s: Settings = serde_json::from_value(json).unwrap();
            assert!(migrate_to_pages(&mut s), "{workspace}");
            assert_eq!(s.page, page, "{workspace} maps to {page}");
            assert!(s.layout.is_empty(), "{workspace}: the page starts from its new default");
            let names: Vec<&str> = s.layout_profiles.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, ["Mine", BEFORE_UPDATE], "one 'Before update', the user's own profile kept");
            assert_eq!(s.layout_profiles[1].json, old_tree, "the old arrangement itself is kept");
            assert!(Layout::from_json_migrating(&s.layout_profiles[1].json).is_some(), "and loads as a profile");
            assert!(!migrate_to_pages(&mut s), "one-time");
        }
        // a fresh install: Edit, no profile
        let mut s = Settings::default();
        assert!(migrate_to_pages(&mut s));
        assert_eq!(s.page, "Edit");
        assert!(s.layout_profiles.is_empty());
    }
}
