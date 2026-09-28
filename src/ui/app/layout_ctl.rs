//! ---- ws:layout-modes-onboarding ----
//! Layout modes as infrastructure: the ACT_HANDLERS entry for Workspace1..6 / MaximizePane /
//! TogglePin / ToggleSource (declared in hotkeys.rs by this workstream) plus the two variants
//! command-palette declared for us to consume (ToggleLayoutMode, ShowWelcome); the pin/mode-aware
//! `surface`; the menu-bar workspace strip; `poll_popout` (the body wave-0b's `on_viewport` hook in
//! `layout::show` was pre-placed for, so Space/J/K/L work in a torn-off Preview); and the WINDOW_DRAWERS
//! glue for the welcome wizard (`ui::onboarding`) and the home screen (`ui::home`), which are App-free
//! on purpose. Everything here mutates Settings / Layout / UI state only - never the project, so no
//! `push_undo_labeled` anywhere except the one place the wizard applies a starting format to an EMPTY
//! project (that is a project edit and gets its own labelled undo entry).

use super::*;
use crate::ui::layout::{workspace_glyph, workspace_layout, Surfaced, WORKSPACES};
use crate::ui::onboarding::{self, Onboarding, Outcome};
use crate::ui::{home, tools::glyph_text_button};

/// Dynamic (the default) unless Settings say "granular" - one reading for every caller.
pub(super) fn is_dynamic(settings: &Settings) -> bool {
    settings.layout_mode != "granular"
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::Workspace1 => switch_workspace(app, WORKSPACES[0]),
        Action::Workspace2 => switch_workspace(app, WORKSPACES[1]),
        Action::Workspace3 => switch_workspace(app, WORKSPACES[2]),
        Action::Workspace4 => switch_workspace(app, WORKSPACES[3]),
        Action::Workspace5 => switch_workspace(app, WORKSPACES[4]),
        Action::Workspace6 => switch_workspace(app, WORKSPACES[5]),
        Action::MaximizePane => {
            if app.layout.maximized.is_some() {
                app.layout.unmaximize();
                app.layout_dirty = true;
            } else if let Some(p) = app.layout.hovered {
                app.layout.maximize(p);
                app.layout_dirty = true;
            } else {
                app.toast("Hover a pane, then press ` to maximise it (` again restores)");
            }
            true
        }
        Action::TogglePin => {
            match app.layout.hovered {
                Some(p) => {
                    let on = app.layout.toggle_pin(p);
                    app.layout_dirty = true;
                    app.toast(format!("{} {}", if on { "Pinned" } else { "Unpinned" }, p.title()));
                }
                None => app.toast("Hover a pane to pin or unpin it (or use its tab's pin)"),
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
                "Dynamic layout: a selection surfaces the panel that edits it (pin a tab to opt it out)"
            } else {
                "Granular layout: panels stay put, the helpful tab just glows"
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

/// Persist a mode change and re-evaluate the current selection under the new rules right away, so
/// Ctrl+Shift+G / the View menu / `layout.mode` all feel immediate.
pub(super) fn set_mode(app: &mut App, dynamic: bool) {
    app.settings.layout_mode = if dynamic { "dynamic" } else { "granular" }.into();
    app.settings.save();
    let kind = frame::selection_kind_of(app);
    app.tools.lead = if dynamic { lead_tool(kind) } else { None };
    surface_for_kind(app, kind);
}

/// Panes a selection kind would like in front, best first: the first one the layout can actually
/// switch to (or glow) wins. Audio prefers the Mixer but settles for the Inspector's audio section
/// when the Mixer is stacked away (the Simple workspace); `None`/`Mixed`/`EditPoint` ask for nothing -
/// the Timeline is always there and a mixed bag has no single home.
pub(super) fn panes_for(kind: SelectionKind) -> &'static [Pane] {
    match kind {
        SelectionKind::Text | SelectionKind::Video | SelectionKind::Shape | SelectionKind::Sequence | SelectionKind::Adjustment => {
            &[Pane::Inspector]
        }
        SelectionKind::Audio => &[Pane::Mixer, Pane::Inspector],
        SelectionKind::Transition => &[Pane::Transitions, Pane::Inspector],
        SelectionKind::Cue => &[Pane::Subtitles],
        SelectionKind::None | SelectionKind::Mixed | SelectionKind::EditPoint => &[],
    }
}

/// The tool the Dynamic-mode adaptive strip moves to the front for a selection kind (`None` = the
/// fixed order): text edits want the Text tool, shapes a shape tool, an edit point the razor.
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

/// Selection-driven surfacing, pure over a `Layout`: Dynamic switches the tab when `reveal_auto` says
/// it may and glows it when a pin refuses; Granular NEVER switches, it only glows the tab that would
/// have helped. `Pinned` is returned for "handled by a glow" in both modes so a caller trying several
/// candidate panes stops at the first one that is actually on screen.
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
/// the pin/mode-aware automatic variant frame::tick calls on a selection change.
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

/// Switch to a named workspace: the undo-preserving preset swap (`Layout::switch_to`), the name
/// remembered in Settings for the strip / View menu. Unknown names toast and return false.
pub(super) fn switch_workspace(app: &mut App, name: &str) -> bool {
    let Some(make) = workspace_layout(name) else {
        app.toast(format!("No workspace called '{name}' (try {})", WORKSPACES.join(", ")));
        return false;
    };
    app.layout.switch_to(make());
    app.layout_dirty = true;
    app.settings.workspace = name.to_string();
    app.settings.save();
    true
}

/// The menu-bar workspace strip: one small button per `WORKSPACES` entry, the active one lit. Meant
/// for a right-to-left region, so the buttons are added in reverse to read Simple … Deliver.
pub(super) fn workspace_strip(app: &mut App, ui: &mut egui::Ui) {
    let active = app.settings.workspace.clone();
    let mut pick = None;
    for (i, &name) in WORKSPACES.iter().enumerate().rev() {
        let on = name == active;
        let r = ui
            .scope(|ui| {
                if on {
                    let accent = ui.visuals().selection.bg_fill;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = accent;
                    ui.visuals_mut().widgets.inactive.fg_stroke.color = crate::ui::tools::on_accent(accent);
                }
                glyph_text_button(ui, workspace_glyph(name), name)
            })
            .inner
            .on_hover_text(format!("{name} workspace  (Alt+{})", i + 1));
        if r.clicked() && !on {
            pick = Some(name);
        }
    }
    if let Some(name) = pick {
        switch_workspace(app, name);
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

/// Finish: `onboarding::finish` (mode, workspace, consent flag, the guarded install) plus the two
/// things only the app can do - apply a starting format to an EMPTY project (one labelled undo entry)
/// and persist.
fn finish_onboarding(app: &mut App, st: &Onboarding) {
    let allowed = onboarding::install_allowed(cfg!(debug_assertions), crate::contextmenu::is_installed());
    let mut install_err = None;
    let installed = {
        let App { settings, layout, .. } = app;
        onboarding::finish(st, settings, layout, allowed, &mut || {
            if let Err(e) = crate::contextmenu::install() {
                install_err = Some(e.to_string());
            }
        })
    };
    app.layout_dirty = true;
    app.settings.save();
    if let Some(i) = st.template {
        apply_format(app, i);
    }
    match (installed, install_err) {
        (true, Some(e)) => app.toast(format!("Context menu: {e}")),
        (true, None) => app.toast("Added 'Edit with Simple Editor' to Explorer's right-click menu"),
        _ => {}
    }
    app.toast("Welcome! F1 lists every shortcut, Ctrl+K searches every command");
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
    let Some(action) = home::show(ctx, &app.settings, empty, app.home_dismissed) else { return };
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
        // Audio falls back to the Inspector's audio section where the Mixer is stacked away (Simple)
        assert_eq!(panes_for(SelectionKind::Audio), &[Pane::Mixer, Pane::Inspector]);
        let mut simple = Layout::simple_layout();
        let now = Instant::now();
        assert_eq!(react(&mut simple, true, Pane::Mixer, now), Surfaced::Hidden, "Mixer is stacked hidden in Simple");
        assert_eq!(react(&mut simple, true, Pane::Inspector, now), Surfaced::Shown);
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
            events: vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            ..Default::default()
        };
        for (key, want) in [(Key::Space, Action::PlayPause), (Key::J, Action::ShuttleBack), (Key::L, Action::ShuttleFwd), (Key::K, Action::Stop)] {
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
        assert!(is_dynamic(&s), "fresh settings are Dynamic");
        s.layout_mode = "granular".into();
        assert!(!is_dynamic(&s));
        s.layout_mode = "anything-else".into();
        assert!(is_dynamic(&s), "only an explicit 'granular' switches auto-surfacing off");
    }
}
