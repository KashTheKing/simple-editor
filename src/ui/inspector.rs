//! Inspector panel. Nothing selected → Project (name, resolution + format dropdown, FPS, duration,
//! background). Transitions selected → their editors. One or more clips selected → a header (label
//! dot · name · enable toggle, then one muted "track · start · duration" line) and collapsible sections,
//! only the first relevant one open (`primary_section` - the page's own on the Color / Audio pages):
//! Transform, Opacity & blend or Audio (inspector_audio.rs), Speed, Color (color_ui.rs), Effects (the
//! applied stack, `effects_ui::stack` - the one place it is edited; the Effects pane is the catalogue),
//! Text (inspector_text.rs, first for a Text clip), Shape, Mask, Path, Asset. Secondary verbs live on
//! each section title's right-click; every property row is label · value · ◆ (`ui::key_buttons`).
//!  * multi-selection: Enabled, Label, transform/opacity (or volume/pan), fades, Blend, Speed and the
//!    Effects stack's parameters edit every selected clip at once (absolute overwrite of whatever
//!    changed, diff-against-original - mirrors `transition_section`'s bulk-edit rule); everything else
//!    (Name included) is disabled and needs a single-clip selection.
//!  * Color and Mask follow the TRACK: a nested sequence's audio twin (a Sequence clip on an audio
//!    track) gets the Audio section and neither of them.
//! Hand-offs the app polls each frame (the show() signature has no room for them): Save/Export-style
//! Actions, font imports, node editor / mask / shader requests, and Previous/Next-key seeks.
//! Call `undo(project)` once per gesture (on `drag_started()` / first `changed()` of a widget) before mutating.
//! Returns true if the project changed.

use crate::hotkeys::Action;
use crate::model::{ClipKind, Id, Label, Mask, Project, ShapeKind, ShapeStyle};
use crate::settings::{Settings, TextPreset};
use crate::theme::Palette;
use crate::ui::inspector_text::{set_span, span_draft_at};
use crate::ui::markers_ui::x_button;
use crate::ui::tools::Glyph;
use crate::ui::{edit_start, mask_grid, menu, timecode, Gesture};
use eframe::egui::{self, DragValue, Grid, Response, RichText};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Test-only: remember a widget rect so headless tests can click the real button.
#[cfg(test)]
pub(super) fn mark(ui: &egui::Ui, name: &str, r: &Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("insp", name.to_string())), r.rect));
}
#[cfg(not(test))]
pub(super) fn mark(_ui: &egui::Ui, _name: &str, _r: &Response) {}

// Hand-offs to the app (the show() signature has no room for these; the app polls them each frame).
thread_local! {
    static PENDING_FONT: RefCell<Option<String>> = const { RefCell::new(None) };
    static EDIT_MASK: RefCell<Option<Id>> = const { RefCell::new(None) };
    static OPEN_NODES: RefCell<Option<Id>> = const { RefCell::new(None) };
    static UNLINK_NODES: RefCell<Option<Id>> = const { RefCell::new(None) };
    /// An Action a section asked for (Replace container media, Auto Duck, Normalize) - the app runs it
    /// through the same Action dispatch the menus use.
    static PENDING_ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
    /// (clip, effect index): draw that effect's mask in the viewport / edit that shader effect's GLSL.
    static EFFECT_MASK: RefCell<Option<(Id, usize)>> = const { RefCell::new(None) };
    static EDIT_SHADER: RefCell<Option<(Id, usize)>> = const { RefCell::new(None) };
    /// An effect row's "Save to Gallery…" (clip, stack index).
    static SAVE_EFFECT: RefCell<Option<(Id, usize)>> = const { RefCell::new(None) };
    /// Timeline time a keyframe menu's Previous / Next key asked for.
    static PENDING_SEEK: RefCell<Option<f64>> = const { RefCell::new(None) };
}

/// Clip the user asked to turn back into a plain effect stack (`Project::unlink_graph`).
pub fn take_unlink_nodes() -> Option<Id> {
    UNLINK_NODES.with(|p| p.borrow_mut().take())
}

/// Clip whose mask the user wants to draw in the viewport ("Edit in viewport") - the app switches the
/// active tool to a mask tool and points the preview at this clip.
pub fn take_edit_mask() -> Option<Id> {
    EDIT_MASK.with(|p| p.borrow_mut().take())
}

/// Clip the user asked to open in the node editor.
pub fn take_open_nodes() -> Option<Id> {
    OPEN_NODES.with(|p| p.borrow_mut().take())
}

/// (clip, effect index) whose own mask the user wants to draw in the viewport.
pub fn take_effect_mask() -> Option<(Id, usize)> {
    EFFECT_MASK.with(|p| p.borrow_mut().take())
}

/// (clip, effect index) of the Shader effect whose GLSL the user wants to edit.
pub fn take_save_effect() -> Option<(Id, usize)> {
    SAVE_EFFECT.with(|p| p.borrow_mut().take())
}

pub fn take_edit_shader() -> Option<(Id, usize)> {
    EDIT_SHADER.with(|p| p.borrow_mut().take())
}

/// Timeline time to seek to (a keyframe menu's Previous / Next key).
pub fn take_pending_seek() -> Option<f64> {
    PENDING_SEEK.with(|p| p.borrow_mut().take())
}

/// Font file (.ttf/.otf) the user picked with "Import font…" - the app adds it to settings.user_fonts
/// and reloads the text rasterizer.
pub fn take_pending_font_import() -> Option<String> {
    PENDING_FONT.with(|p| p.borrow_mut().take())
}

/// Set by inspector_text's "Import font…" button (PENDING_FONT is private to this file).
pub(super) fn set_pending_font_import(path: String) {
    PENDING_FONT.with(|p| *p.borrow_mut() = Some(path));
}

// ---- ws:audio-dsp-automation ----
/// Set by inspector_audio's Auto Duck / Normalize menu rows (PENDING_ACTION is private to this file); the
/// app drains it with `take_pending_action`.
pub(super) fn set_pending_action(a: Action) {
    PENDING_ACTION.with(|p| *p.borrow_mut() = Some(a));
}

/// Action a section asked to run - the app pushes it through the same `Action` dispatch the menus and
/// hotkeys use.
pub fn take_pending_action() -> Option<Action> {
    PENDING_ACTION.with(|p| p.borrow_mut().take())
}

// ---- ws:inspector-gallery ----
thread_local! {
    /// Asset the user asked to jump to via the collapsed Asset block's "Open in Library" link.
    static OPEN_ASSET: RefCell<Option<Id>> = const { RefCell::new(None) };
    /// Color section "Auto Colour" click: needs a live GPU-rendered frame (`FrameStats`), which only
    /// `App` can produce - `App::poll_panels` drains this into `run_tool_undoable("color.auto", …)`.
    static PENDING_COLOR_AUTO: RefCell<Option<Id>> = const { RefCell::new(None) };
    /// Color section "Match" click: (clip, reference clip) - same App-only reason as above.
    static PENDING_COLOR_MATCH: RefCell<Option<(Id, Id)>> = const { RefCell::new(None) };
    /// Color section "Eyedropper" click. ponytail: armed here, but nothing samples a pixel from it yet
    /// (that needs a canvas click handler in `preview.rs`, owned by canvas-handles-monitor) -
    /// `App::poll_panels` still drains it and toasts an honest "not wired yet".
    static PENDING_EYEDROP: RefCell<Option<Id>> = const { RefCell::new(None) };
    /// A slot's Media row: (clip, Some(asset) = a Library clip dropped on it, None = Unlink). `App` runs
    /// it through `container.replace` / `container.unlink` (one undo step each).
    static PENDING_SLOT: RefCell<Option<(Id, Option<Id>)>> = const { RefCell::new(None) };
}

pub fn take_pending_slot() -> Option<(Id, Option<Id>)> {
    PENDING_SLOT.with(|p| p.borrow_mut().take())
}

/// Asset the user asked to open in the Library pane (its own asset-details box is the only place
/// description/tags/label/folder are edited today - see `library.rs`'s doc comment).
pub fn take_open_asset() -> Option<Id> {
    OPEN_ASSET.with(|p| p.borrow_mut().take())
}

pub fn take_pending_color_auto() -> Option<Id> {
    PENDING_COLOR_AUTO.with(|p| p.borrow_mut().take())
}

pub fn take_pending_color_match() -> Option<(Id, Id)> {
    PENDING_COLOR_MATCH.with(|p| p.borrow_mut().take())
}

pub fn take_pending_eyedrop() -> Option<Id> {
    PENDING_EYEDROP.with(|p| p.borrow_mut().take())
}

/// A collapsible inspector block: `CollapsingState` keyed by `id` (stable across a clip/effect/kind
/// change, so `Settings.inspector_folds` remembers "Effects is open" independent of which clip is
/// selected), `default_open` used only the first time `id` is ever seen (no `folds` entry yet - after
/// that, the user's own last choice always wins over `default_open`, even across a restart, since
/// `folds` mirrors straight into `Settings.inspector_folds`). The whole title row toggles it and
/// carries the section's right-click `menu`; `body` draws the contents when expanded.
pub(crate) fn section(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    default_open: bool,
    folds: &mut BTreeMap<String, bool>,
    menu: Option<&mut dyn FnMut(&mut egui::Ui)>,
    body: impl FnOnce(&mut egui::Ui),
) {
    let open_default = folds.get(id).copied().unwrap_or(default_open);
    let cid = egui::Id::new(("insp_section", id));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), cid, open_default);
    let header = state.show_header(ui, |ui| {
        let h = ui.spacing().interact_size.y;
        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, title, font, ui.visuals().strong_text_color());
        r.on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    let mut now_open = header.is_open();
    let (_, title_r, _) = header.body(body);
    let title_r = title_r.inner;
    mark(ui, &format!("sec_{id}"), &title_r);
    if title_r.clicked() {
        if let Some(mut st) = egui::collapsing_header::CollapsingState::load(ui.ctx(), cid) {
            st.toggle(ui);
            st.store(ui.ctx());
            now_open = st.is_open();
        }
    }
    if let Some(m) = menu {
        menu::context(&title_r, |ui| m(ui));
    }
    if now_open != open_default {
        folds.insert(id.to_string(), now_open);
    }
}

/// Where the clip sections keep their fold state, and which of them starts open (`primary_section`).
pub(super) struct Folds<'a> {
    pub primary: &'static str,
    pub map: &'a mut BTreeMap<String, bool>,
}

/// A clip section, open by default only when it is `folds.primary`. The fold key says which ("color*"
/// where Color leads, "color" everywhere else), so closing a section where it leads keeps it closed
/// there, and opening it where it doesn't lead never changes where it does - the old
/// "effects_primary"/"effects_secondary" pair, generalised to every section.
pub(super) fn fold(
    ui: &mut egui::Ui,
    folds: &mut Folds,
    id: &str,
    title: &str,
    menu: Option<&mut dyn FnMut(&mut egui::Ui)>,
    body: impl FnOnce(&mut egui::Ui),
) {
    let primary = folds.primary == id;
    let key = if primary { format!("{id}*") } else { id.to_string() };
    section(ui, &key, title, primary, folds.map, menu, body);
}

/// The one clip section open by default: the page's own (Color / Audio) when the clip has it, else the
/// one a beginner needs first for this kind of clip. `audio` = the clip sits on an audio track.
pub(super) fn primary_section(page: &str, kind: ClipKind, audio: bool) -> &'static str {
    if audio {
        return "audio";
    }
    match (page, kind) {
        ("Color", _) => "color",
        (_, ClipKind::Text) => "text",
        (_, ClipKind::Shape) => "shape",
        (_, ClipKind::Adjustment) => "effects",
        (_, ClipKind::Browser) => "browser", // ws:browser-layer
        _ => "transform",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    sel_transitions: &[Id],
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let _s = crate::perf::span("inspector_draw");
    let first = selection.iter().copied().find(|&id| project.clip(id).is_some());
    // ws:animator: the block picked in the Animator leads, its clip's own sections follow; the keyframe
    // menus' "Link to layer" offers every other clip
    crate::ui::set_link_layers(project.all_clips().filter(|(_, c)| !selection.contains(&c.id)).map(|(_, c)| (c.id, c.name.clone())).collect());
    let block = crate::ui::animator::block_section(ui, project, palette, undo);
    let edited = block | match first {
        None if !sel_transitions.is_empty() => transition_section(ui, project, sel_transitions, undo),
        None => project_section(ui, project, settings, undo),
        Some(_) => clip_section(ui, project, selection, playhead, fonts, palette, settings, undo),
    };
    // ws:keyframe-blocks: a keyframe menu's Animate ▸ - one undo, every selected clip at the playhead
    let mut edited = edited;
    if let Some(b) = crate::ui::take_block_request().and_then(|n| crate::model::find_block(&settings.key_blocks, &n)) {
        undo(project);
        for &id in selection {
            let at = project.clip(id).filter(|c| c.contains(playhead)).map(|c| playhead - c.start);
            edited |= project.apply_key_block(id, &b, at, None).is_some();
        }
    }
    // a keyframe menu's Previous / Next key: clip-local → timeline time on the clip it was drawn for
    if let Some(kt) = crate::ui::take_key_seek() {
        if let Some(c) = first.and_then(|id| project.clip(id)) {
            PENDING_SEEK.with(|p| *p.borrow_mut() = Some(c.start + kt));
        }
    }
    // drawn from here (not project_section) so the open prompt survives selection changes
    rescale_prompt(ui, project, undo) || edited
}

/// Selected transitions (timeline bands): one set of editors; each field you change is written to
/// every selected transition, fields you leave alone keep their per-transition values.
pub(crate) fn transition_section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], undo: &mut dyn FnMut(&Project)) -> bool {
    use crate::model::{Ease, TransitionKind};
    let list: Vec<crate::model::Transition> = ids
        .iter()
        .filter_map(|&id| project.tracks.iter().flat_map(|t| &t.transitions).find(|t| t.id == id).cloned())
        .collect();
    let Some(first) = list.first().cloned() else {
        ui.label("Select a clip or a transition");
        return false;
    };
    let mut g = Gesture::default();
    if list.len() == 1 {
        ui.strong("Transition");
    } else {
        ui.strong(format!("Transitions ({} selected)", list.len()));
    }
    let mut e = first.clone();
    Grid::new("insp_transition").num_columns(2).show(ui, |ui| {
        ui.label("Kind");
        egui::ComboBox::from_id_salt("insp_tr_kind").selected_text(e.name()).show_ui(ui, |ui| {
            for k in TransitionKind::ALL {
                g.note(&ui.selectable_value(&mut e.kind, k, k.name()));
            }
        });
        ui.end_row();
        ui.label("Duration");
        // clamp_existing_to_range(false): merely drawing an out-of-range value must not fake an edit
        g.note(&ui.add(
            DragValue::new(&mut e.duration).range(0.1..=5.0).clamp_existing_to_range(false).speed(0.02).suffix(" s"),
        ));
        ui.end_row();
        if e.kind == TransitionKind::FadeToColor {
            ui.label("Color");
            g.note(&ui.color_edit_button_srgba_unmultiplied(&mut e.color));
            ui.end_row();
        }
        if e.kind.has_direction() {
            ui.label("Direction");
            ui.horizontal(|ui| {
                for (i, name) in ["Left", "Right", "Up", "Down"].iter().enumerate() {
                    g.note(&ui.selectable_value(&mut e.direction, i as u8, *name));
                }
            });
            ui.end_row();
        }
        // ws:shader-library: a shader transition's named controls
        if e.kind == TransitionKind::Shader {
            let specs = crate::model::shader_specs(&e.shader);
            crate::ui::effects_ui::plain_param_rows(ui, &specs, &mut e.params, &mut g);
        }
        ui.label("Ease");
        egui::ComboBox::from_id_salt("insp_tr_ease").selected_text(e.ease.name()).show_ui(ui, |ui| {
            for ea in Ease::ALL {
                g.note(&ui.selectable_value(&mut e.ease, ea, ea.name()));
            }
        });
        ui.end_row();
    });
    let mut removed = false;
    let label = if list.len() == 1 { "Remove transition".into() } else { format!("Remove {} transitions", list.len()) };
    let r = ui.button(label);
    mark(ui, "tr_remove", &r);
    if r.clicked() {
        g.click();
        removed = true;
    }
    if g.start {
        undo(project);
    }
    if !g.changed {
        return false;
    }
    if removed {
        for &id in ids {
            project.remove_transition(id);
        }
        return true;
    }
    for &id in ids {
        if let Some(t) = project.transition_mut(id) {
            if e.kind != first.kind {
                t.kind = e.kind;
            }
            if e.duration != first.duration {
                t.duration = e.duration;
            }
            if e.color != first.color {
                t.color = e.color;
            }
            if e.direction != first.direction {
                t.direction = e.direction;
            }
            if e.ease != first.ease {
                t.ease = e.ease;
            }
            if e.params != first.params && t.kind == TransitionKind::Shader {
                t.params.clone_from(&e.params); // ws:shader-library
            }
        }
    }
    true
}

/// Project settings (nothing selected): a short form of labelled dropdowns - Name, Format (the platform
/// presets and your saved formats, showing the current match or "Custom"), Resolution (quality tiers
/// that keep the aspect; "Custom…" reveals W × H), Frame rate (the presets; "Custom…" reveals a
/// number), Background. The "Project" title shows the duration; its right-click saves the current
/// format as a template or deletes saved ones. Save / Export live in the File menu and the Export page.
pub(crate) fn project_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    use crate::model::{clean_fps, STANDARD_FPS as FPS_PRESETS};
    use crate::ui::guides::{scale_to_tier, PRESETS, RES_TIERS};
    let mut edited = false;
    // resolution before any of this frame's widgets can touch it - diffed at the bottom to offer the
    // rescale prompt once, regardless of which control changed it.
    let size_before = (project.width, project.height);
    // dropdown picks, applied after the form: (w, h), fps, and the platform guide overlay a preset shows
    let (mut size, mut fps_pick, mut guide): (Option<(u32, u32)>, Option<f64>, Option<Option<&'static str>>) =
        (None, None, None);
    let (mut delete, mut save_as): (Option<usize>, Option<String>) = (None, None);
    let naming_id = egui::Id::new("insp_format_naming");
    let mut naming: Option<String> = ui.data(|d| d.get_temp(naming_id));
    let custom_id = egui::Id::new("insp_custom_size");
    let fps_custom_id = egui::Id::new("insp_custom_fps");

    let title = ui
        .horizontal(|ui| {
            let r = ui.add(egui::Label::new(RichText::new("Project").strong()).sense(egui::Sense::click()));
            ui.weak(timecode(project.duration(), project.fps)).on_hover_text("Duration");
            r
        })
        .inner;
    mark(ui, "project_title", &title);
    menu::context(&title.on_hover_text("Right-click: save this format, delete saved ones"), |ui| {
        if menu::row(ui, Some(Glyph::Template), "Save format as template…", "").clicked() {
            naming = Some(String::new());
        }
        ui.add_enabled_ui(!settings.project_templates.is_empty(), |ui| {
            menu::sub(ui, Some(Glyph::Cross), "Delete saved format", |ui| {
                for (i, t) in settings.project_templates.iter().enumerate() {
                    if menu::row(ui, None, &t.name, "").clicked() {
                        delete = Some(i);
                    }
                }
            });
        });
    });

    Grid::new("inspector_project").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        let mut name = project.name.clone(); // ponytail: per-frame clone so undo can snapshot before the write
        let r = ui.text_edit_singleline(&mut name);
        if r.gained_focus() {
            undo(project); // once per visit to the field, not per keystroke
        }
        if r.changed() {
            project.name = name;
            edited = true;
        }
        ui.end_row();

        ui.label("Format");
        let fps_now = project.fps;
        let same = |w: u32, h: u32, f: f64| project.width == w && project.height == h && (fps_now - f).abs() < 0.001;
        // presets can share a size (YouTube = General Long Form): the one whose guide is on wins
        let preset = PRESETS
            .iter()
            .find(|p| same(p.w, p.h, p.fps) && p.guide == settings.guide.as_deref())
            .or_else(|| PRESETS.iter().find(|p| same(p.w, p.h, p.fps)));
        let saved = settings.project_templates.iter().find(|t| same(t.width, t.height, t.fps));
        let current = preset.map(|p| p.name).or(saved.map(|t| t.name.as_str())).unwrap_or("Custom");
        let r = egui::ComboBox::from_id_salt("project_format")
            .selected_text(current)
            .width(170.0)
            .height(420.0)
            .show_ui(ui, |ui| {
                ui.visuals_mut().button_frame = false; // menu-style rows, not a stack of buttons
                for p in PRESETS {
                    let text = format!("{}×{} · {:.0} fps", p.w, p.h, p.fps);
                    if menu::row(ui, Some(p.glyph), p.name, &text).clicked() {
                        (size, fps_pick, guide) = (Some((p.w, p.h)), Some(p.fps), Some(p.guide));
                    }
                }
                if !settings.project_templates.is_empty() {
                    ui.separator();
                }
                for t in &settings.project_templates {
                    let text = format!("{}×{} · {} fps", t.width, t.height, fps_label(t.fps));
                    if menu::row(ui, Some(Glyph::Template), &t.name, &text).clicked() {
                        (size, fps_pick) = (Some((t.width, t.height)), Some(t.fps));
                    }
                }
            });
        mark(ui, "project_format", &r.response);
        ui.end_row();

        ui.label("Resolution");
        let tier = RES_TIERS
            .iter()
            .find(|&&(_, t)| scale_to_tier(project.width, project.height, t) == (project.width, project.height));
        // W × H show only while "Custom…" was picked, or the size matches no tier
        let mut want_custom: bool = ui.data(|d| d.get_temp(custom_id)).unwrap_or(false);
        let custom = want_custom || tier.is_none();
        {
            let text = match tier {
                Some((name, _)) if !custom => format!("{name} · {}×{}", project.width, project.height),
                _ => format!("Custom · {}×{}", project.width, project.height),
            };
            let r = egui::ComboBox::from_id_salt("project_res").selected_text(text).width(170.0).show_ui(ui, |ui| {
                ui.visuals_mut().button_frame = false;
                for &(name, t) in RES_TIERS {
                    let (w, h) = scale_to_tier(project.width, project.height, t);
                    if menu::row(ui, None, name, &format!("{w}×{h}"))
                        .on_hover_text("Keeps the current aspect")
                        .clicked()
                    {
                        size = Some((w, h));
                    }
                }
                ui.separator();
                if menu::row(ui, None, "Custom…", "").clicked() {
                    want_custom = true;
                }
            });
            mark(ui, "project_res", &r.response);
            ui.end_row();
            if custom {
                ui.label("");
                ui.horizontal(|ui| {
                    let (mut w, mut h) = (project.width as f64, project.height as f64);
                    let mut dg = Gesture::default();
                    for (name, v) in [("width", &mut w), ("height", &mut h)] {
                        if name == "height" {
                            ui.label("×");
                        }
                        let r = ui.add(DragValue::new(v).range(16.0..=8192.0).speed(1.0));
                        mark(ui, &format!("project_{name}"), &r);
                        dg.note(&r);
                    }
                    if dg.start {
                        undo(project);
                    }
                    if dg.changed {
                        (project.width, project.height) = (w as u32, h as u32);
                        edited = true;
                    }
                });
                ui.end_row();
            }
        }
        ui.data_mut(|d| d.insert_temp(custom_id, want_custom && size.is_none()));

        ui.label("Frame rate");
        let fps_text = |f: f64| format!("{} fps", fps_label(f));
        let preset_fps = FPS_PRESETS.iter().any(|&f| (project.fps - f).abs() < 0.001);
        let mut want_fps: bool = ui.data(|d| d.get_temp(fps_custom_id)).unwrap_or(false);
        let fps_custom = want_fps || !preset_fps;
        {
            let r = egui::ComboBox::from_id_salt("project_fps")
                .selected_text(fps_text(project.fps))
                .width(170.0)
                .show_ui(ui, |ui| {
                    ui.visuals_mut().button_frame = false;
                    for &f in FPS_PRESETS {
                        if menu::check(ui, (project.fps - f).abs() < 0.001, &fps_text(f), "").clicked() {
                            fps_pick = Some(f);
                        }
                    }
                    ui.separator();
                    if menu::row(ui, None, "Custom…", "").clicked() {
                        want_fps = true;
                    }
                });
            mark(ui, "project_fps", &r.response);
            ui.end_row();
            if fps_custom {
                ui.label("");
                let mut f = project.fps;
                let r = ui.add(DragValue::new(&mut f).range(1.0..=240.0).speed(0.1).max_decimals(3).suffix(" fps"));
                if edit_start(&r) {
                    undo(project);
                }
                if r.changed() {
                    project.fps = clean_fps(f);
                    edited = true;
                }
                ui.end_row();
            }
        }
        ui.data_mut(|d| d.insert_temp(fps_custom_id, want_fps && fps_pick.is_none()));

        // ---- ws:project-color-fps ----
        ui.label("Color");
        {
            use crate::model::ColorSpace;
            let tip = |c: ColorSpace| match c {
                ColorSpace::Sdr => "Standard video: looks the same on every screen and site",
                ColorSpace::Hdr10 => "10-bit HEVC or AV1 with PQ and HDR10 metadata, for HDR TVs, phones and YouTube HDR. Graphics sit at 203 nits; the Preview shows it tone-mapped",
                ColorSpace::Hlg => "10-bit HEVC or AV1 with HLG: broadcast HDR that still plays as SDR on SDR screens. The Preview shows it tone-mapped",
            };
            let r = egui::ComboBox::from_id_salt("project_color")
                .selected_text(project.color.name())
                .width(170.0)
                .show_ui(ui, |ui| {
                    ui.visuals_mut().button_frame = false;
                    for c in ColorSpace::ALL {
                        if menu::check(ui, project.color == c, c.name(), "").on_hover_text(tip(c)).clicked() && project.color != c {
                            undo(project);
                            project.color = c;
                            edited = true;
                        }
                    }
                });
            mark(ui, "project_color", &r.response);
            r.response.on_hover_text(tip(project.color));
        }
        ui.end_row();

        // ---- ws:max-duration ----
        ui.label("Max length");
        edited |= max_length_row(ui, project, undo);
        ui.end_row();

        // same setting as the preview's right-click Background submenu, editable from project settings
        // per the original ask ("this should save in project settings, and I should also be able to
        // edit it in the project settings tab")
        ui.label("Background");
        ui.horizontal(|ui| {
            use crate::model::BackgroundMode;
            let current = project.preview_bg;
            egui::ComboBox::from_id_salt("project_bg").selected_text(current.name()).width(170.0).show_ui(ui, |ui| {
                for mode in BackgroundMode::ALL {
                    if ui.selectable_label(current == mode, mode.name()).clicked() && current != mode {
                        undo(project);
                        project.preview_bg = mode;
                        edited = true;
                    }
                }
                let custom = matches!(current, BackgroundMode::Custom(_));
                if ui.selectable_label(custom, "Custom").clicked() && !custom {
                    undo(project);
                    project.preview_bg = BackgroundMode::Custom([0, 0, 0, 255]);
                    edited = true;
                }
            });
            if let BackgroundMode::Custom(mut rgba) = project.preview_bg {
                let cr = ui.color_edit_button_srgba_unmultiplied(&mut rgba);
                if edit_start(&cr) {
                    undo(project);
                }
                if cr.changed() {
                    project.preview_bg = BackgroundMode::Custom(rgba);
                    edited = true;
                }
            }
        });
        ui.end_row();
    });

    let r = ui
        .button("Copy from Other Project…")
        .on_hover_text("Take the format, frame rate, colour, Max length and background from another project file");
    mark(ui, "project_copy_from", &r);
    if r.clicked() {
        if let Some(path) = rfd::FileDialog::new().add_filter("Simple Editor project", &["sedit"]).pick_file() {
            match Project::load(&path) {
                Ok(o) => {
                    undo(project);
                    copy_project_settings(project, &o);
                    edited = true;
                }
                Err(e) => {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("insp_copy_err"), e));
                }
            }
        }
    }
    if let Some(e) = ui.ctx().data(|d| d.get_temp::<String>(egui::Id::new("insp_copy_err"))) {
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, format!("Couldn't read that project: {e}"));
            if crate::ui::markers_ui::x_button(ui).clicked() {
                ui.ctx().data_mut(|d| d.remove::<String>(egui::Id::new("insp_copy_err")));
            }
        });
    }

    // "Save format as template…" (the title's right-click): one inline name row until saved / cancelled
    if let Some(name) = naming.as_mut() {
        ui.horizontal(|ui| {
            let hint = format!("{}×{}", project.width, project.height);
            let r = ui.add(egui::TextEdit::singleline(name).hint_text(&hint).desired_width(140.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Save").clicked() || enter {
                save_as = Some(if name.trim().is_empty() { hint } else { name.trim().to_string() });
            }
            if ui.button("Cancel").clicked() {
                save_as = Some(String::new());
            }
        });
    }
    if save_as.is_some() {
        naming = None;
    }
    ui.data_mut(|d| match &naming {
        Some(n) => d.insert_temp(naming_id, n.clone()),
        None => d.remove::<String>(naming_id),
    });

    if size.is_some() || fps_pick.is_some() {
        undo(project);
        if let Some((w, h)) = size {
            // switching into short form with no limit yet: 1:00 (an existing limit is kept)
            let into_short = crate::ui::guides::is_short_form(w, h)
                && !crate::ui::guides::is_short_form(project.width, project.height);
            if into_short && project.max_duration.is_none() {
                project.max_duration = Some(crate::ui::guides::SHORT_FORM_MAX);
            }
            (project.width, project.height) = (w, h);
        }
        if let Some(f) = fps_pick {
            project.fps = clean_fps(f);
        }
        edited = true;
    }
    if let Some(g) = guide.filter(|g| *g != settings.guide.as_deref()) {
        settings.guide = g.map(str::to_string);
        settings.save();
    }
    if let Some(i) = delete {
        settings.project_templates.remove(i);
        settings.save();
    }
    if let Some(name) = save_as.filter(|n| !n.is_empty()) {
        settings.project_templates.retain(|t| t.name != name);
        settings.project_templates.push(crate::settings::ProjectTemplate {
            name,
            width: project.width,
            height: project.height,
            fps: project.fps,
        });
        settings.save();
    }

    // arm the prompt; the window itself is drawn by `rescale_prompt` from `show()` on EVERY frame, so
    // clicking a clip mid-decision (which switches the inspector to clip_section) can't vanish it
    if (project.width, project.height) != size_before
        && project.all_clips().any(|(_, c)| project.clip_native_size(c).is_some())
    {
        ui.ctx().data_mut(|d| d.insert_temp(rescale_prompt_id(), true));
    }

    edited
}

// ---- ws:project-color-fps ----
/// "29.97" / "60" / "26.022": at most 3 decimals, trailing zeros dropped (never a long float).
pub(crate) fn fps_label(f: f64) -> String {
    let s = format!("{f:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---- ws:max-duration ----
/// Quick picks for the Max length row: (seconds, where the limit comes from).
pub(crate) const MAX_LENGTHS: &[(f64, &str)] = &[
    (15.0, ""),
    (30.0, ""),
    (60.0, "Shorts, Reels"),
    (90.0, ""),
    (180.0, "TikTok, Shorts"),
    (600.0, ""),
    (3600.0, ""),
    (2.0 * 3600.0, ""),
    (12.0 * 3600.0, "YouTube upload"),
];

/// Seconds from "90", "1:30" (m:ss), "1:30:00" (h:mm:ss), "2d 1:00:00", or units: "90s", "45m", "9999h", "3d",
/// "1h 30m". No upper limit.
pub(crate) fn parse_length(s: &str) -> Option<f64> {
    let mut s = s.trim().to_ascii_lowercase();
    for u in ["d", "h", "m", "s"] {
        s = s.replace(&format!(" {u}"), u); // "45 s" / "2 h" = "45s" / "2h"
    }
    let mut total = 0.0;
    let mut any = false;
    for part in s.split_whitespace() {
        let v = if let Some(n) = part.strip_suffix('d') {
            n.parse::<f64>().ok()? * 86_400.0
        } else if let Some(n) = part.strip_suffix('h') {
            n.parse::<f64>().ok()? * 3600.0
        } else if let Some(n) = part.strip_suffix('m') {
            n.parse::<f64>().ok()? * 60.0
        } else {
            // plain seconds or colon form, read from the right: s, m:s, h:m:s
            let n = part.strip_suffix('s').unwrap_or(part);
            let mut v = 0.0;
            for (i, x) in n.rsplit(':').enumerate() {
                v += x.parse::<f64>().ok()? * [1.0, 60.0, 3600.0].get(i).copied()?;
            }
            v
        };
        total += v;
        any = true;
    }
    (any && total > 0.0 && total.is_finite()).then_some(total)
}

/// The project form's Max length row: [on/off] [m:ss field] [quick picks]. The timeline being edited's
/// `max_duration` (the open sequence's while one is open). Each change is one undo step.
fn max_length_row(ui: &mut egui::Ui, project: &mut Project, undo: &mut dyn FnMut(&Project)) -> bool {
    let was = project.max_duration;
    let mut set: Option<Option<f64>> = None;
    ui.horizontal(|ui| {
        let mut on = was.is_some();
        let r = ui.checkbox(&mut on, "").on_hover_text("Limit how long this timeline may be. Nothing is cut: the part past it is shaded and exports stop there.");
        mark(ui, "project_max_on", &r);
        if r.changed() {
            set = Some(on.then_some(60.0));
        }
        let mut v = project.max_duration.unwrap_or(60.0);
        let r = ui.add_enabled(
            on,
            DragValue::new(&mut v)
                .range(1.0..=f64::MAX)
                .speed(1.0)
                .custom_formatter(|v, _| crate::ui::length_text(v))
                .custom_parser(parse_length),
        );
        mark(ui, "project_max", &r);
        if edit_start(&r) {
            undo(project);
        }
        if r.changed() {
            project.max_duration = Some(v.max(1.0));
            return;
        }
        let r = egui::ComboBox::from_id_salt("project_max_pick").selected_text("").width(24.0).show_ui(ui, |ui| {
            ui.visuals_mut().button_frame = false;
            for &(t, hint) in MAX_LENGTHS {
                if menu::check(ui, project.max_duration == Some(t), &crate::ui::length_text(t), hint).clicked() {
                    set = Some(Some(t));
                }
            }
        });
        mark(ui, "project_max_pick", &r.response);
        let out = project.out_point.filter(|o| *o >= 1.0);
        let r = ui
            .add_enabled(out.is_some(), egui::Button::new("Set to Out"))
            .on_hover_text("Use the timeline's Out point as the Max length")
            .on_disabled_hover_text("Set an Out point on the timeline first (O)");
        mark(ui, "project_max_out", &r);
        if r.clicked() {
            set = out.map(Some);
        }
    });
    if let Some(m) = set.filter(|m| *m != project.max_duration) {
        undo(project);
        project.max_duration = m;
    }
    project.max_duration != was
}

/// "Copy from Other Project…": the project-settings fields of `from` (not its name, media or timeline).
fn copy_project_settings(to: &mut Project, from: &Project) {
    (to.width, to.height, to.fps) = (from.width, from.height, crate::model::clean_fps(from.fps));
    (to.color, to.max_duration, to.preview_bg) = (from.color, from.max_duration, from.preview_bg);
}

fn rescale_prompt_id() -> egui::Id {
    egui::Id::new("inspector_rescale_prompt")
}

/// "Resize existing media?" - non-blocking (plain egui::Window, never Modal). Armed by
/// `project_section` when a control changed the resolution and footage exists; stays open across
/// frames and selection changes (egui::Id-keyed temp) until answered or closed. Drawn by the Inspector
/// and the Export pane (both show the project form); only the first caller in a pass draws it.
pub(crate) fn rescale_prompt(ui: &mut egui::Ui, project: &mut Project, undo: &mut dyn FnMut(&Project)) -> bool {
    let pass = ui.ctx().cumulative_pass_nr();
    let drawn = egui::Id::new("inspector_rescale_prompt_pass");
    if ui.ctx().data_mut(|d| std::mem::replace(d.get_temp_mut_or(drawn, u64::MAX), pass)) == pass {
        return false;
    }
    let mut edited = false;
    let mut rescale_open: bool = ui.ctx().data(|d| d.get_temp(rescale_prompt_id()).unwrap_or(false));
    if rescale_open {
        let mut dismissed = false;
        egui::Window::new("Resize Media?").resizable(false).collapsible(false).open(&mut rescale_open).show(
            ui.ctx(),
            |ui| {
                ui.label("Resize existing media to fit the new project size?");
                ui.horizontal(|ui| {
                    let r = ui.button("Rescale");
                    mark(ui, "rescale_prompt_rescale", &r);
                    if r.clicked() {
                        undo(project);
                        let ids: Vec<Id> = project.all_clips().map(|(_, c)| c.id).collect();
                        for id in ids {
                            project.fit_clip_to_screen(id, false);
                        }
                        edited = true;
                        dismissed = true;
                    }
                    let r = ui.button("Keep As Is");
                    mark(ui, "rescale_prompt_keep", &r);
                    if r.clicked() {
                        dismissed = true;
                    }
                });
            },
        );
        if dismissed {
            rescale_open = false;
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(rescale_prompt_id(), rescale_open));
    edited
}

/// The label dot's menu: None + the project's labels (each with its colour in the gutter, a tick on the
/// current one), then "Edit labels…". Returns the picked label index; sets `edit` for the editor.
fn label_menu(ui: &mut egui::Ui, labels: &[Label], current: u8, palette: &Palette, edit: &mut bool) -> Option<u8> {
    let mut picked = None;
    for i in 0..=labels.len() {
        let (name, color) = match i {
            0 => ("None", palette.text_dim),
            _ => {
                let l = &labels[i - 1];
                (l.name.as_str(), egui::Color32::from_rgb(l.color[0], l.color[1], l.color[2]))
            }
        };
        let r = menu::row(ui, None, name, if current as usize == i { "✓" } else { "" });
        let dot = egui::pos2(r.rect.left() + ui.spacing().button_padding.x + 9.0, r.rect.center().y);
        ui.painter().circle_filled(dot, 5.0, color);
        if r.clicked() {
            picked = Some(i as u8);
        }
    }
    ui.separator();
    if menu::row(ui, None, "Edit labels…", "").clicked() {
        *edit = true;
    }
    picked
}

#[allow(clippy::too_many_arguments)]
fn clip_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    // Every selected id that is still a live clip. The first is the "representative" - its widgets
    // drive the section - and edits to the bulk-editable fields propagate to the rest with
    // diff-against-original, absolute-overwrite semantics (mirrors `transition_section`).
    let mut clip_ids: Vec<Id> = selection.iter().copied().filter(|&i| project.clip(i).is_some()).collect();
    // a video clip's linked audio: its sound rows (Volume, Pan, ...) show with the video's, and it does not
    // count as a second selected clip
    let on_audio = |i: Id| crate::ui::inspector_audio::on_audio_track(project, i);
    let visual: Vec<Id> = clip_ids.iter().copied().filter(|&i| !on_audio(i)).collect();
    let linked_audio: Vec<Id> = project.expand_links(&visual).into_iter().filter(|&i| on_audio(i)).collect();
    clip_ids.retain(|i| !linked_audio.contains(i));
    let n_selected = clip_ids.len();
    let Some(&id) = clip_ids.first() else {
        return false;
    };
    // ponytail: edit a per-frame clone of the clip and write it back at the end - lets `undo` snapshot the
    // untouched project first without borrow gymnastics. Upgrade: per-field scratch copies if it ever shows.
    // Owned (not borrowed) so it survives the later `project.clip_mut` write-backs - needed to diff the
    // bulk-editable fields against their pre-edit values for the sibling-propagation pass.
    let Some(orig) = project.clip(id).cloned() else {
        return false;
    };
    let mut clip = orig.clone();
    let track = project.track_of(id).map(|t| project.tracks[t].name.clone()).unwrap_or_default();
    let fps = project.fps;
    let lt = clip.local(playhead);
    let mut g = Gesture::default();
    // the Color section edits `clip.effects` through this clone; its own gesture says when to write
    // them back (the Effects stack below commits its own edits straight to the project)
    let mut cg = Gesture::default();
    // snapshots: the widgets edit a clone of the clip, so the project must not stay borrowed
    let labels: Vec<Label> = project.labels.clone();
    let labels_open_id = egui::Id::new("inspector_labels_open");
    let mut edit_labels: bool = ui.ctx().data(|d| d.get_temp(labels_open_id).unwrap_or(false));
    let mut label_ops: Vec<LabelOp> = Vec::new();
    let mut path_op: Option<PathOp> = None;
    let multi = n_selected > 1;
    // Color, Mask and the transform go by TRACK, not kind: a Sequence clip on an audio track is the
    // audio twin of a nested sequence and only sounds (#88)
    let audio = crate::ui::inspector_audio::on_audio_track(project, id);
    // folds live in Settings; taken out for the frame so section bodies can still borrow `settings`
    // (nothing below may `settings.save()` before they are put back - see `save_settings`)
    let mut fold_map = std::mem::take(&mut settings.inspector_folds);
    let mut folds = Folds { primary: primary_section(&settings.page, clip.kind, audio), map: &mut fold_map };
    let mut save_settings = false;

    // ---- header: label dot · name · enable, then one muted line ----
    ui.horizontal(|ui| {
        let color = labels
            .get((clip.label as usize).wrapping_sub(1))
            .map(|l| egui::Color32::from_rgb(l.color[0], l.color[1], l.color[2]))
            .unwrap_or(palette.text_dim);
        let chip = crate::ui::tools::color_chip(color, false, palette);
        let (r, _) = egui::containers::menu::MenuButton::from_button(chip).ui(ui, |ui| {
            if let Some(l) = label_menu(ui, &labels, clip.label, palette, &mut edit_labels) {
                if l != clip.label {
                    clip.label = l;
                    g.click();
                }
            }
        });
        mark(ui, "label_dot", &r);
        r.on_hover_text(format!("Label: {}", project.label_name(clip.label)));
        let eye_w = 24.0 + ui.spacing().item_spacing.x + 4.0; // the eye button, its gap, a margin
        ui.add_enabled_ui(!multi, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut clip.name).desired_width(ui.available_width() - eye_w));
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("test_name_field"), r.id);
                d.insert_temp(egui::Id::new("test_name_field_enabled"), r.enabled());
            });
            g.note_text(&r);
        });
        let (glyph, tip) = if clip.enabled {
            (Glyph::Eye, "Enabled - click to switch the clip off")
        } else {
            (Glyph::EyeOff, "Disabled - click to switch the clip back on")
        };
        let r = crate::ui::tools::icon_button(ui, palette, ui.id().with("clip_enabled"), glyph, tip, false);
        mark(ui, "enabled", &r);
        if r.clicked() {
            clip.enabled = !clip.enabled;
            g.click();
        }
    });
    if multi {
        // an honest line for mixed selections: props propagate by matching label, and audio clips
        // expose Volume/Pan where visual ones expose transform - so a mixed selection only bulk-edits
        // the clips matching the representative's kind, and the line must say so, not claim "all"
        let same = |i: &Id| crate::ui::inspector_audio::on_audio_track(project, *i) == audio;
        let matching = clip_ids.iter().filter(|i| same(i)).count();
        let text = if matching < n_selected {
            let (this, other) = if audio { ("audio", "video") } else { ("video", "audio") };
            format!(
                "{n_selected} clips - property edits apply to the {matching} {this} clip{}; the {} {other} \
                 clip{} keep their own (enabled, label and fades still edit all of them).",
                if matching == 1 { "" } else { "s" },
                n_selected - matching,
                if n_selected - matching == 1 { "" } else { "s" },
            )
        } else {
            format!("{n_selected} clips selected - edits apply to all of them; the rest needs one clip.")
        };
        ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
    } else {
        let mut tip =
            format!("Track {track} · starts {} · {}", timecode(clip.start, fps), timecode(clip.duration, fps));
        if clip.uses_asset() {
            tip.push_str(&format!(" long · source in {}", timecode(clip.src_in, fps)));
        }
        let line = format!("{track} · {} · {:.1} s", timecode(clip.start, fps), clip.duration);
        ui.add(egui::Label::new(RichText::new(line).weak()).truncate()).on_hover_text(tip);
    }
    if clip.kind == ClipKind::Adjustment {
        ui.weak("Adjustment layer - its effects apply to everything below it.");
    }
    if clip.container {
        ui.add_enabled_ui(!multi, |ui| {
            Grid::new("inspector_container").num_columns(2).show(ui, |ui| {
                ui.label("Slot label");
                let r = ui.text_edit_singleline(&mut clip.container_label);
                g.note_text(&r);
                ui.end_row();

                ui.label("Media");
                ui.horizontal(|ui| {
                    let text = match project.asset(clip.asset) {
                        _ if clip.asset == 0 => RichText::new("(Empty Slot)").weak(),
                        Some(a) => RichText::new(a.name()),
                        None => RichText::new("Missing asset").weak(),
                    };
                    let r = ui.add(egui::Label::new(text).truncate().sense(egui::Sense::hover())).on_hover_text("Drop a Library clip here to use it");
                    if let Some(pl) = r.dnd_release_payload::<crate::ui::DragPayload>() {
                        if let crate::ui::DragPayload::Asset(aid) = *pl {
                            PENDING_SLOT.with(|p| *p.borrow_mut() = Some((clip.id, Some(aid))));
                        }
                    }
                    if ui.small_button("Replace…").clicked() {
                        PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::ReplaceContainerMedia));
                    }
                });
                ui.end_row();
                let shared = if clip.media_link == 0 {
                    0
                } else {
                    project.all_clips().filter(|(_, o)| o.media_link == clip.media_link && o.kind != ClipKind::Audio).count()
                };
                if shared > 1 {
                    ui.label("Linked");
                    ui.horizontal(|ui| {
                        ui.label(format!("{shared} slots")).on_hover_text("Replacing this footage changes every slot linked to it");
                        if ui.small_button("Unlink").on_hover_text("Give this slot its own footage").clicked() {
                            PENDING_SLOT.with(|p| *p.borrow_mut() = Some((clip.id, None)));
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }
    ui.separator();

    // ---- ws:html-layer ---- an HTML layer is code, not footage: its source, file and fields lead
    let mut html_changed = false;
    if !multi && clip.uses_asset() && project.asset(clip.asset).is_some_and(|a| a.html.is_some()) {
        {
            // the page live (instant on every edit); the timeline keeps the frame-exact render
            let within = playhead >= clip.start && playhead < clip.start + clip.duration;
            let at = within.then(|| clip.src_time(playhead));
            section(ui, "html_live", "Preview", true, folds.map, None, |ui| {
                crate::ui::html_ui::live_section(ui, project, id, at);
            });
        }
        // ---- ws:thread-viewers ---- a chat / forum thread is edited as data; its page is generated
        let is_thread = crate::ui::thread_ui::is_thread(project, id);
        if is_thread {
            section(ui, "thread", "Thread", true, folds.map, None, |ui| {
                html_changed |= crate::ui::thread_ui::section(ui, project, id, fonts, undo);
            });
        }
        section(ui, "html_source", "HTML", !is_thread, folds.map, None, |ui| {
            html_changed |= crate::ui::html_ui::source_section(ui, project, id, undo);
        });
        if !is_thread {
            section(ui, "fields", "Fields", true, folds.map, None, |ui| {
                html_changed |= crate::ui::html_ui::fields_section(ui, project, id, fonts, undo);
            });
        }
        {
            let at = clip.src_time(playhead);
            section(ui, "html_mouse", "Mouse", false, folds.map, None, |ui| {
                html_changed |= crate::ui::html_ui::mouse_section(ui, project, id, at, palette, undo);
            });
        }
        if crate::ui::html_ui::has_params(project, id) {
            let at = clip.src_time(playhead);
            section(ui, "html_params", "Parameters", true, folds.map, None, |ui| {
                html_changed |= crate::ui::html_ui::params_section(ui, project, id, at, palette, undo);
            });
        }
    }

    // ---- ws:audio-visualizer ---- style picker for a visualizer layer
    let mut visualizer_changed = false;
    if !multi && crate::ui::visualizer_ui::style(project, id).is_some() {
        let backend = crate::media::Backend::parse(&settings.decoder);
        section(ui, "visualizer", "Visualizer", true, folds.map, None, |ui| {
            let last = settings.visualizer_style.clone();
            visualizer_changed =
                crate::ui::visualizer_ui::section(ui, project, id, backend, &mut settings.visualizer_style, undo);
            if settings.visualizer_style != last {
                settings.save();
            }
        });
        // a 3D style's camera (`// @camera` shaders)
        if project.clip(id).is_some_and(|c| c.effects.iter().any(|e| crate::engine::visualizer::camera_at(e, 0.0).is_some())) {
            section(ui, "visualizer_camera", "Camera", true, folds.map, None, |ui| {
                visualizer_changed |= crate::ui::visualizer_ui::camera_section(ui, project, id, lt, palette, undo);
            });
        }
    }

    // ---- ws:text-titles: primary-first ordering ----
    // For a Text clip, typography is what a beginner needs first, so its section leads (and is the one
    // open). The "Text style presets" sub-panel sits at its end: it needs `&mut Settings`, which
    // `inspector_text::section`'s signature has no room for.
    let mut text_changed = false;
    if clip.kind == ClipKind::Text {
        // every selected text clip at once (one undo step); the presets act on the first
        let text_ids: Vec<Id> =
            clip_ids.iter().copied().filter(|&i| project.clip(i).is_some_and(|c| c.kind == ClipKind::Text)).collect();
        fold(ui, &mut folds, "text", "Text", None, |ui| {
            text_changed = crate::ui::inspector_text::section(ui, project, &text_ids, playhead, fonts, palette, undo);
            ui.add_enabled_ui(!multi, |ui| {
                text_changed |= text_presets(ui, project, id, settings, undo, &mut save_settings);
            });
        });
    }

    // Transform + Blend and fades, or Audio - bulk-editable, committed by inspector_audio itself.
    let mut audio_changed =
        crate::ui::inspector_audio::section(ui, project, &clip_ids, playhead, palette, &mut folds, true, undo);
    if !linked_audio.is_empty() {
        // open like an audio clip's own Audio section (the same fold state)
        let primary = std::mem::replace(&mut folds.primary, "audio");
        audio_changed |=
            crate::ui::inspector_audio::section(ui, project, &linked_audio, playhead, palette, &mut folds, false, undo);
        folds.primary = primary;
    }

    // ws:time-remap: Time Remap - keys, holds, backward stretches; committed by its own ops
    let mut remap_changed = false;
    if !multi && matches!(clip.kind, ClipKind::Video | ClipKind::Audio | ClipKind::Sequence) {
        fold(ui, &mut folds, "time_remap", "Time Remap", None, |ui| {
            remap_changed = crate::ui::time_remap_ui::section(ui, project, id, playhead, palette, undo);
        });
    }

    // Color: Primaries/Curves/Levels/HueShift/Vignette, added lazily on first touch - nothing to grade
    // on an audio track
    if !audio {
        let others: Vec<(Id, String)> = clip_ids
            .iter()
            .copied()
            .filter(|&i| i != id)
            .filter_map(|i| project.clip(i).map(|c| (i, c.name.clone())))
            .collect();
        fold(ui, &mut folds, "color", "Color", None, |ui| {
            ui.add_enabled_ui(!multi, |ui| {
                let resp = crate::ui::color_ui::show(ui, &mut clip, &others, lt, palette, &mut cg);
                if resp.auto {
                    PENDING_COLOR_AUTO.with(|p| *p.borrow_mut() = Some(id));
                }
                if let Some(rid) = resp.match_ref {
                    PENDING_COLOR_MATCH.with(|p| *p.borrow_mut() = Some((id, rid)));
                }
                if resp.eyedrop {
                    PENDING_EYEDROP.with(|p| *p.borrow_mut() = Some(id));
                }
            });
        });
    }

    // Effects: the applied stack (reorder, enable, parameters; each row's right-click has the rest).
    // Parameter edits bulk-apply to the other selected clips, as they did in the Effects pane.
    let mut effects_changed = false;
    {
        let has_graph = clip.graph.is_some();
        let mut nodes_menu = |ui: &mut egui::Ui| {
            if menu::row(ui, Some(Glyph::Nodes), "Open in Node editor", "").clicked() {
                OPEN_NODES.with(|p| *p.borrow_mut() = Some(id));
            }
            if has_graph && menu::row(ui, None, "Unlink node graph", "").clicked() {
                UNLINK_NODES.with(|p| *p.borrow_mut() = Some(id));
            }
        };
        let title = match orig.effects.len() {
            0 => "Effects".to_string(),
            n => format!("Effects ({n})"),
        };
        fold(ui, &mut folds, "effects", &title, Some(&mut nodes_menu), |ui| {
            let r = crate::ui::effects_ui::stack(ui, project, &clip_ids, playhead, palette, undo);
            effects_changed = r.edited;
            if r.open_nodes {
                OPEN_NODES.with(|p| *p.borrow_mut() = Some(id));
            }
            if let Some(i) = r.mask_for {
                EFFECT_MASK.with(|p| *p.borrow_mut() = Some((id, i)));
            }
            if let Some(i) = r.save_effect {
                SAVE_EFFECT.with(|p| *p.borrow_mut() = Some((id, i)));
            }
            if let Some(i) = r.edit_shader {
                EDIT_SHADER.with(|p| *p.borrow_mut() = Some((id, i)));
            }
        });
    }

    // Zone 2: everything below is per-clip data that does not bulk-edit - greyed out and non-interactive
    // while more than one clip is selected, exactly like the Name field (the labels editor at the very
    // bottom is the one exception: it edits `Project.labels`, not this clip, so it stays live).
    // (skipped outright when it would draw nothing, so an audio clip gets no stray gap)
    let zone2 = clip.shape.is_some() || clip.kind == ClipKind::Shape || !audio || !project.paths.is_empty();
    if zone2 {
        ui.add_enabled_ui(!multi, |ui| {
            // ws:browser-layer
            if let Some(b) = &mut clip.browser {
                let pages: Vec<(Id, String)> = project
                    .assets
                    .iter()
                    .filter(|a| a.kind == ClipKind::Image)
                    .map(|a| (a.id, a.name()))
                    .collect();
                crate::ui::browser_ui::section(ui, b, lt, fonts, &pages, palette, &mut folds, &mut g);
            }
            // shape style
            if let Some(sh) = &mut clip.shape {
                fold(ui, &mut folds, "shape", "Shape", None, |ui| {
                    use crate::ui::{prop_field, prop_field_val, prop_row, PROP_VAL_W};
                    prop_field(ui, "Kind", |ui| {
                        egui::ComboBox::from_id_salt("shape_kind")
                            .selected_text(sh.kind.name())
                            .width(PROP_VAL_W)
                            .show_ui(ui, |ui| {
                                for k in ShapeKind::ALL {
                                    g.note(&ui.selectable_value(&mut sh.kind, k, k.name()));
                                }
                            });
                    });
                    prop_field(ui, "Fill", |ui| g.note(&ui.color_edit_button_srgba_unmultiplied(&mut sh.fill)));
                    prop_field(ui, "Stroke", |ui| {
                        g.note(&ui.color_edit_button_srgba_unmultiplied(&mut sh.stroke));
                        g.note(&ui.add(DragValue::new(&mut sh.stroke_width).range(0.0..=200.0).speed(0.2)));
                    });
                    g.note(&prop_field_val(ui, "Sides", DragValue::new(&mut sh.sides).range(3..=64)));
                    g.note(&prop_field_val(ui, "Corner", DragValue::new(&mut sh.corner).range(0.0..=500.0).speed(0.5)));
                    prop_row(ui, "Width", &mut sh.w, lt, palette, &mut g, &[], |v| {
                        DragValue::new(v).range(1.0..=20000.0).speed(1.0)
                    });
                    prop_row(ui, "Height", &mut sh.h, lt, palette, &mut g, &[], |v| {
                        DragValue::new(v).range(1.0..=20000.0).speed(1.0)
                    });
                    if sh.kind == ShapeKind::Draw {
                        prop_field(ui, "Draw rate", |ui| {
                            g.note(&ui.add(DragValue::new(&mut sh.draw_rate).range(0.0..=8.0).speed(0.05)));
                            ui.weak(format!("{} strokes · {:.1} s", sh.strokes.len(), sh.draw_duration()));
                        });
                        prop_field(ui, "Page", |ui| g.note(&ui.color_edit_button_srgba_unmultiplied(&mut sh.page)));
                    }
                });
            } else if clip.kind == ClipKind::Shape && ui.button("Add shape style").clicked() {
                clip.shape = Some(ShapeStyle::default());
                g.click();
            }

            // mask - it shapes pixels, so nothing on an audio track gets mask UI at all (not even a dead
            // button). The title's right-click and the body can't both hold `&mut clip`, so they report
            // intent through plain flags and the mutation happens after the section.
            if !audio {
                let has_mask = clip.mask.is_some();
                let (mut add_mask, mut edit_mask, mut remove_mask) = (false, false, false);
                let mut mask_menu = |ui: &mut egui::Ui| {
                    if !has_mask && menu::row(ui, Some(Glyph::Mask), "Add mask", "").clicked() {
                        add_mask = true;
                    }
                    if has_mask && menu::row(ui, Some(Glyph::Mask), "Edit in viewport", "").clicked() {
                        edit_mask = true;
                    }
                    if has_mask && menu::row(ui, Some(Glyph::Cross), "Remove mask", "").clicked() {
                        remove_mask = true;
                    }
                };
                let mut body_add = false;
                let mut body_edit = false;
                fold(ui, &mut folds, "mask", "Mask", Some(&mut mask_menu), |ui| match &mut clip.mask {
                    None => {
                        let r = ui.button("Add mask");
                        mark(ui, "add_mask", &r);
                        body_add = r.clicked();
                    }
                    Some(m) => {
                        let r = ui.button("Edit in viewport").on_hover_text("Drag the mask over the preview");
                        mark(ui, "edit_mask", &r);
                        body_edit = r.clicked();
                        mask_grid(ui, m, lt, palette, &mut g, egui::Id::new("inspector_mask"));
                    }
                });
                if add_mask || body_add {
                    clip.mask = Some(Mask::default());
                    g.click();
                }
                if edit_mask || body_edit {
                    EDIT_MASK.with(|p| *p.borrow_mut() = Some(id));
                }
                if remove_mask {
                    clip.mask = None;
                    g.click();
                }
            }

            // reusable paths: a drawing or a polygon outline is saved on the project, and any clip can then
            // travel along one (its X/Y become keyframes over the clip's own length)
            // ponytail: only the cheap "is there an outline" test runs per frame; the points are copied when
            // Save is actually clicked
            let outline = clip.shape.as_ref().is_some_and(|s| !s.strokes.is_empty() || s.points.len() >= 2);
            let paths: Vec<(Id, String)> = project.paths.iter().map(|p| (p.id, p.name.clone())).collect();
            if outline || !paths.is_empty() {
                fold(ui, &mut folds, "path", "Path", None, |ui| {
                    ui.horizontal(|ui| {
                        if outline {
                            let r = ui
                                .button("Save as path")
                                .on_hover_text("Keep this outline in the project as a reusable path");
                            mark(ui, "save_path", &r);
                            if r.clicked() {
                                path_op = Some(PathOp::Save);
                            }
                        }
                        if !paths.is_empty() {
                            egui::ComboBox::from_id_salt("clip_path")
                                .selected_text("Move along…")
                                .width(130.0)
                                .show_ui(ui, |ui| {
                                    for (pid, name) in &paths {
                                        if ui.selectable_label(false, name).clicked() {
                                            path_op = Some(PathOp::Apply(*pid));
                                        }
                                    }
                                });
                        }
                    });
                });
            }
        }); // end zone 2 (add_enabled_ui)
    }

    // asset details: a status line + "Open in Library" - description/tags/label/folder are edited ONLY
    // in Library's asset-details box (see library.rs's doc comment).
    if clip.uses_asset() {
        if let Some(a) = project.asset(clip.asset) {
            let proxy = crate::media::proxy::status(a, settings.use_proxies, settings.proxy_height);
            let uses = asset_use_count(project, a.id);
            fold(ui, &mut folds, "asset", "Asset", None, |ui| {
                ui.add(egui::Label::new(RichText::new(a.name()).weak()).truncate()).on_hover_text(&a.path);
                ui.weak(format!("Used in: {uses} clips"));
                // where this asset sits in the proxy pipeline (playback smoothness at 4K depends on it)
                match proxy {
                    crate::media::proxy::ProxyStatus::Ready => {
                        ui.weak("Proxy: ready (preview plays the low-res proxy)");
                    }
                    crate::media::proxy::ProxyStatus::Building(f) => {
                        ui.weak(format!("Proxy: building - {:.0} %", f * 100.0));
                    }
                    crate::media::proxy::ProxyStatus::Queued => {
                        ui.weak("Proxy: queued (builds run one at a time)");
                    }
                    crate::media::proxy::ProxyStatus::NotNeeded => {}
                }
                let r = ui.button("Open in Library").on_hover_text("Edit description, tags, label and folder there");
                mark(ui, "open_asset", &r);
                if r.clicked() {
                    OPEN_ASSET.with(|p| *p.borrow_mut() = Some(a.id));
                }
            });
        }
    }

    // compact labels editor (labels live on the project, not the clip) - project-wide, so it stays
    // interactive regardless of how many clips are selected (see the doc comment above zone 2).
    if edit_labels {
        ui.separator();
        // everything in the body: "Add"/"Done" and the per-label rows below both push into
        // `label_ops`, and a title menu/body pair can't both hold that mutably at once.
        let mut done = false;
        section(ui, "labels", "Labels", true, folds.map, None, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("Add").clicked() {
                    label_ops.push(LabelOp::Add);
                }
                if ui.small_button("Done").clicked() {
                    done = true;
                }
            });
            for (i, l) in labels.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut color = l.color;
                    if ui.color_edit_button_srgb(&mut color).changed() {
                        label_ops.push(LabelOp::Color(i, color));
                    }
                    let mut name = l.name.clone();
                    let w = (ui.available_width() - 30.0).max(50.0);
                    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(w)).changed() {
                        label_ops.push(LabelOp::Rename(i, name));
                    }
                    if x_button(ui).on_hover_text("Remove label").clicked() {
                        label_ops.push(LabelOp::Remove(i));
                    }
                });
            }
        });
        if done {
            edit_labels = false;
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(labels_open_id, edit_labels));
    settings.inspector_folds = fold_map;
    if save_settings {
        settings.save();
    }

    g.start |= cg.start;
    g.changed |= cg.changed;
    if g.start || !label_ops.is_empty() || path_op.is_some() {
        undo(project);
    }
    if g.changed {
        // Targeted field write-back: `clip` only holds this fn's own fields by now (name / enabled /
        // label / container_label / mask / shape, and effects when the Color section touched them) -
        // properties, fades, blend, speed, bus, text and the effects stack are owned by the sections
        // above, which already committed their own edits straight to `project`; overwriting the whole
        // clip here would revert those with this fn's stale pre-edit clone.
        if let Some(c) = project.clip_mut(id) {
            c.container_label = clip.container_label.clone();
            c.name = clip.name.clone();
            c.enabled = clip.enabled;
            c.label = clip.label;
            c.mask = clip.mask.clone();
            c.shape = clip.shape.clone();
            c.browser = clip.browser.clone(); // ws:browser-layer: its section edits the clone too
            if cg.changed {
                c.effects = clip.effects.clone();
            }
        }
        // Bulk propagation: only Enabled/Label are zone-1 (bulk-editable) among this fn's own fields;
        // everything else here is zone-2 (single-clip only, already unreachable while multi is true).
        if multi {
            for &sid in &clip_ids {
                if sid == id {
                    continue;
                }
                let Some(s) = project.clip_mut(sid) else { continue };
                if clip.enabled != orig.enabled {
                    s.enabled = clip.enabled;
                }
                if clip.label != orig.label {
                    s.label = clip.label;
                }
            }
        }
    }
    let labels_changed = !label_ops.is_empty();
    for op in label_ops {
        match op {
            LabelOp::Add => {
                project.add_label(format!("Label {}", project.labels.len() + 1), [160, 160, 160]);
            }
            LabelOp::Rename(i, name) => {
                if let Some(l) = project.labels.get_mut(i) {
                    l.name = name;
                }
            }
            LabelOp::Color(i, c) => {
                if let Some(l) = project.labels.get_mut(i) {
                    l.color = c;
                }
            }
            // remove_label() also re-points every clip / asset / marker using it
            LabelOp::Remove(i) => project.remove_label(i as u8 + 1),
        }
    }
    // after the write-back: applying a path overwrites the X/Y the clone still held
    match path_op {
        Some(PathOp::Save) => {
            let pts = project.path_from_clip(id);
            project.add_path(clip.name.clone(), pts);
        }
        Some(PathOp::Apply(pid)) => {
            project.link_path(id, pid);
        }
        None => {}
    }
    g.changed
        || labels_changed
        || path_op.is_some()
        || audio_changed
        || text_changed
        || effects_changed
        || html_changed // ws:html-layer
        || visualizer_changed // ws:audio-visualizer
        || remap_changed // ws:time-remap
}

/// Text style presets, at the end of the Text section: save the clip's (or the selected characters')
/// style, apply / rename / export / delete saved ones, import. Presets live in Settings (outside undo);
/// `save` asks the caller to write Settings once the frame's borrows are done.
fn text_presets(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: Id,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
    save: &mut bool,
) -> bool {
    let mut changed = false;
    let text_sel_id = egui::Id::new(("inspector_text_sel", id));
    let text_sel: Option<(usize, usize)> =
        ui.ctx().data(|d| d.get_temp::<Option<(usize, usize)>>(text_sel_id)).flatten();
    let presets_open_id = egui::Id::new("inspector_text_presets_open");
    let mut presets_open: bool = ui.ctx().data(|d| d.get_temp(presets_open_id).unwrap_or(false));
    ui.checkbox(&mut presets_open, "Text style presets");
    ui.ctx().data_mut(|d| d.insert_temp(presets_open_id, presets_open));
    if !presets_open {
        return false;
    }
    ui.horizontal(|ui| {
        let has_sel = matches!(text_sel, Some((a, b)) if b > a);
        let label = if has_sel { "Save selection's style as preset" } else { "Save current style as preset" };
        if ui.button(label).on_hover_text("Rename it in the list below").clicked() {
            // with a selection, capture its EFFECTIVE style (span overrides included) - the "I styled
            // this word, save that look" workflow; otherwise the clip style
            if let Some(style) = project.clip(id).and_then(|c| c.text.clone()) {
                let name = format!("Text style {}", settings.text_presets.len() + 1);
                let mut p = match text_sel {
                    Some((a, b)) if b > a => span_draft_at(&style, a, b),
                    _ => span_draft_at(&style, 0, 0),
                };
                p.name = name;
                settings.text_presets.push(p);
                *save = true;
            }
        }
    });
    let mut delete: Option<usize> = None;
    let mut apply_preset: Option<(TextPreset, bool)> = None; // (preset, to the whole clip)
    let armed_id = egui::Id::new("inspector_text_preset_delete_armed");
    let mut armed: Option<usize> = ui.ctx().data(|d| d.get_temp(armed_id)).flatten();
    for (i, p) in settings.text_presets.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(110.0));
            // renames used to vanish on quit: nothing saved settings after editing the name
            if r.lost_focus() {
                *save = true;
            }
            let can = matches!(text_sel, Some((a, b)) if b > a);
            if ui.add_enabled(can, egui::Button::new("Apply to Selection")).clicked() {
                apply_preset = Some((p.clone(), false));
            }
            if ui.button("Apply to Clip").clicked() {
                apply_preset = Some((p.clone(), true));
            }
            if ui.small_button("Export…").clicked() {
                if let Some(out) = rfd::FileDialog::new()
                    .add_filter("Simple Editor text style", &["sedit-textstyle"])
                    .set_file_name(format!("{}.sedit-textstyle", p.name))
                    .save_file()
                {
                    let _ = std::fs::write(&out, serde_json::to_string_pretty(p).unwrap_or_default());
                }
            }
            // presets live outside undo - two-click delete (Shift+click skips the confirm)
            if armed == Some(i) {
                if ui.small_button("Sure?").clicked() {
                    delete = Some(i);
                    armed = None;
                }
            } else if x_button(ui).on_hover_text("Delete preset (Shift+click: no confirm)").clicked() {
                if ui.input(|inp| inp.modifiers.shift) {
                    delete = Some(i);
                } else {
                    armed = Some(i);
                }
            }
        });
    }
    ui.ctx().data_mut(|d| d.insert_temp(armed_id, armed));
    if ui.button("Import…").clicked() {
        if let Some(p) =
            rfd::FileDialog::new().add_filter("Simple Editor text style", &["sedit-textstyle", "json"]).pick_file()
        {
            if let Ok(json) = std::fs::read_to_string(&p) {
                if let Ok(preset) = serde_json::from_str::<TextPreset>(&json) {
                    settings.text_presets.retain(|x| x.name != preset.name);
                    settings.text_presets.push(preset);
                    *save = true;
                }
            }
        }
    }
    if let Some(i) = delete {
        settings.text_presets.remove(i);
        *save = true;
    }
    match apply_preset {
        Some((p, true)) => {
            undo(project);
            if let Some(c) = project.clip_mut(id) {
                let style = c.text.get_or_insert_with(Default::default);
                style.font = p.font.clone();
                // ws:text-titles: size/letter_spacing are now Animated; "Apply to Clip" is a one-shot
                // discrete style change, so it sets the constant value and drops any existing keys -
                // same rule apply_clip_fields (tools_helpers.rs) follows.
                style.size.keys.clear();
                style.size.value = p.size as f64;
                style.bold = p.bold;
                style.italic = p.italic;
                style.color = p.color;
                style.letter_spacing.keys.clear();
                style.letter_spacing.value = p.letter_spacing as f64;
            }
            changed = true;
        }
        Some((p, false)) => {
            if let Some((a, b)) = text_sel {
                undo(project);
                if let Some(c) = project.clip_mut(id) {
                    let style = c.text.get_or_insert_with(Default::default);
                    set_span(style, a, b, &p);
                }
                changed = true;
            }
        }
        None => {}
    }
    changed
}

const LUAU_KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "type", "until", "while",
];

/// Minimal hand-rolled Luau tokenizer for the expression field - keywords, strings, numbers and
/// comments get a colour, everything else stays the default text colour. One expression at a time,
/// so a full syntax-highlighting crate would be a lot of dependency for one line of text.
pub(super) fn luau_highlight(ui: &egui::Ui, text: &str) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Color32, FontId};

    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let base = ui.visuals().text_color();
    let (keyword, string, number, comment) = (
        ui.visuals().hyperlink_color,
        Color32::from_rgb(0x9a, 0xc5, 0x6b),
        ui.visuals().warn_fg_color,
        base.gamma_multiply(0.6),
    );

    let mut job = LayoutJob::default();
    let mut push = |s: &str, color: Color32, font: &FontId| {
        job.append(s, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
    };

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            let s: String = chars[i..].iter().collect();
            push(&s, comment, &font);
            break;
        } else if c == '"' || c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            push(&s, string, &font);
            continue;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            push(&s, number, &font);
            continue;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            push(&s, if LUAU_KEYWORDS.contains(&s.as_str()) { keyword } else { base }, &font);
            continue;
        } else {
            push(&c.to_string(), base, &font);
            i += 1;
        }
    }
    job
}

/// Save the clip's outline as a project path, or animate it along one that was saved.
enum PathOp {
    Save,
    Apply(Id),
}

/// Edits to `Project.labels` collected during the frame (applied after the clip write-back).
enum LabelOp {
    Add,
    Rename(usize, String),
    Color(usize, [u8; 3]),
    Remove(usize),
}

/// How many clips (main timeline, stash, every sequence) use the asset.
fn asset_use_count(project: &Project, aid: Id) -> usize {
    let count = |tracks: &[crate::model::Track]| {
        tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.uses_asset() && c.asset == aid).count()
    };
    let mut n = count(&project.tracks);
    if let Some(st) = &project.main_stash {
        n += count(&st.tracks);
    }
    for s in &project.sequences {
        n += count(&s.tracks);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip};

    #[test]
    fn use_count_and_labels() {
        let a = Asset {
            id: 0,
            path: r"C:\m\a.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        };
        let mut p = Project::from_media(a);
        let aid = p.assets[0].id;
        assert_eq!(asset_use_count(&p, aid), 1);
        p.split_at(4.0, None);
        assert_eq!(asset_use_count(&p, aid), 2);
        assert_eq!(crate::ui::label_name(0), "None");
        assert_eq!(crate::ui::label_name(1), "Red");
        assert_eq!(crate::ui::label_name(200), "None");
    }

    /// A keyboard nudge on the Pan value of an audio clip pushes exactly one undo and moves the pan.
    #[test]
    fn pan_edit_pushes_one_undo() {
        let mut p = Project::new();
        let ai = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Audio).unwrap();
        let c = Clip::new(500, ClipKind::Audio, "a", 0.0, 5.0);
        let id = c.id;
        p.tracks[ai].clips.push(c);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        let mut undos = 0;
        let mut run = |ctx: &egui::Context, input: egui::RawInput, p: &mut Project, undos: &mut usize| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |pre: &Project| {
                        *undos += 1;
                        assert_eq!(pre.clip(id).unwrap().pan.value, 0.0, "undo sees the pre-edit project");
                    };
                    show(ui, p, &[id], &[], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo);
                });
            });
        };
        run(&ctx, egui::RawInput::default(), &mut p, &mut undos); // layout, records the pan slider id
        let slider = ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_pan_slider"))).expect("pan id");
        ctx.memory_mut(|m| m.request_focus(slider));
        run(&ctx, egui::RawInput::default(), &mut p, &mut undos); // focus settles
        assert_eq!(undos, 0);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run(&ctx, input, &mut p, &mut undos);
        assert_eq!(undos, 1, "exactly one undo per edit gesture");
        assert!(p.clip(id).unwrap().pan.value > 0.0, "pan moved right");
        run(&ctx, egui::RawInput::default(), &mut p, &mut undos);
        assert_eq!(undos, 1, "no undo without an edit");
    }

    /// The Opacity control is a 0-100% slider that writes straight into `clip.opacity` - the same
    /// `Animated` field the renderer reads via `props_mut()` - not a second, disconnected property.
    #[test]
    fn opacity_slider_drives_the_animated_field() {
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let c = Clip::new(500, ClipKind::Video, "v", 0.0, 5.0);
        let id = c.id;
        p.tracks[vi].clips.push(c);
        assert_eq!(p.clip(id).unwrap().opacity.value, 1.0, "clips start fully opaque");
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        let mut run = |ctx: &egui::Context, input: egui::RawInput, p: &mut Project| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    show(ui, p, &[id], &[], 1.0, &fonts, &palette, &mut open_all(), &mut undo);
                });
            });
        };
        run(&ctx, egui::RawInput::default(), &mut p); // layout, records the opacity slider id
        let slider =
            ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_opacity_slider"))).expect("opacity id");
        ctx.memory_mut(|m| m.request_focus(slider));
        run(&ctx, egui::RawInput::default(), &mut p); // focus settles
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run(&ctx, input, &mut p);
        let o = p.clip(id).unwrap().opacity.value;
        assert!(o < 1.0 && o >= 0.0, "opacity moved down off its default 1.0: {o}");
    }

    /// With 2+ clips selected, dragging the representative clip's Opacity slider propagates the new
    /// ABSOLUTE value to every selected clip - even one whose opacity started at a different value than
    /// the representative's (diff-against-original, absolute-overwrite: the same rule `transition_section`
    /// uses for bulk-editing transitions, not a relative delta).
    #[test]
    fn opacity_bulk_edit_propagates_absolute_value_to_every_selected_clip() {
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let a = Clip::new(500, ClipKind::Video, "a", 0.0, 5.0);
        let id_a = a.id;
        p.tracks[vi].clips.push(a);
        let mut b = Clip::new(501, ClipKind::Video, "b", 0.0, 5.0);
        b.opacity.value = 0.4; // starts at a different value than `a`'s default 1.0
        let id_b = b.id;
        p.tracks[vi].clips.push(b);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        let mut run = |ctx: &egui::Context, input: egui::RawInput, p: &mut Project| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    show(ui, p, &[id_a, id_b], &[], 1.0, &fonts, &palette, &mut open_all(), &mut undo);
                });
            });
        };
        run(&ctx, egui::RawInput::default(), &mut p); // layout, records `a`'s opacity slider id
        let slider =
            ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_opacity_slider"))).expect("opacity id");
        ctx.memory_mut(|m| m.request_focus(slider));
        run(&ctx, egui::RawInput::default(), &mut p); // focus settles
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run(&ctx, input, &mut p);
        let oa = p.clip(id_a).unwrap().opacity.value;
        let ob = p.clip(id_b).unwrap().opacity.value;
        assert!(oa < 1.0 && oa >= 0.0, "the representative clip's opacity moved down: {oa}");
        assert_eq!(oa, ob, "the new absolute value propagated to the other selected clip, not a relative delta");
    }

    /// With 2+ clips selected, the Name field (zone 1, but not bulk-editable) is disabled - it needs a
    /// single-clip selection, unlike the transform/opacity/enabled/label/fade/blend fields around it.
    #[test]
    fn name_field_disabled_when_multiple_clips_selected() {
        let mut p = Project::new();
        let a = Clip::new(500, ClipKind::Text, "a", 0.0, 5.0);
        let id_a = a.id;
        p.tracks[0].clips.push(a);
        let b = Clip::new(501, ClipKind::Text, "b", 0.0, 5.0);
        let id_b = b.id;
        p.tracks[0].clips.push(b);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut undo = |_: &Project| {};
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut p, &[id_a, id_b], &[], 1.0, &[], &palette, &mut settings, &mut undo);
            });
        });
        let enabled = ctx
            .data_mut(|d| d.get_temp::<bool>(egui::Id::new("test_name_field_enabled")))
            .expect("name field enabled-flag not recorded");
        assert!(!enabled, "Name is disabled while multiple clips are selected");
        // sanity: narrowing back to one clip re-enables it
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut p, &[id_a], &[], 1.0, &[], &palette, &mut settings, &mut undo);
            });
        });
        let enabled = ctx
            .data_mut(|d| d.get_temp::<bool>(egui::Id::new("test_name_field_enabled")))
            .expect("name field enabled-flag not recorded");
        assert!(enabled, "Name is enabled again once only one clip is selected");
    }

    /// With 2+ clips selected, a zone-2 control (here: "Add mask", per-clip data that does not bulk-edit)
    /// is disabled - the click lands on the widget rect but registers no edit at all.
    #[test]
    fn zone2_controls_disabled_when_multiple_clips_selected() {
        let mut p = Project::new();
        let a = p.add_shape_clip(crate::model::ShapeKind::Star, 0.0, 3.0);
        let b = p.add_shape_clip(crate::model::ShapeKind::Star, 3.0, 3.0);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut settings = open_all();
        let mut undos = 0;
        let mut frame = |events: Vec<egui::Event>, p: &mut Project, undos: &mut usize| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 4000.0))),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, p, &[a, b], &[], 1.0, &[], &palette, &mut settings, &mut undo);
                });
            });
            changed
        };
        frame(vec![], &mut p, &mut undos); // layout, records the (disabled) "Add mask" rect
        let r = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", "add_mask".to_string()))))
            .expect("add_mask rect recorded even while disabled");
        let pos = r.center();
        let mut changed = frame(vec![egui::Event::PointerMoved(pos)], &mut p, &mut undos);
        changed |= frame(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut p,
            &mut undos,
        );
        changed |= frame(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut p,
            &mut undos,
        );
        assert!(!changed, "a disabled control must not register a click");
        assert!(p.clip(a).unwrap().mask.is_none(), "Add mask did not fire while multiple clips were selected");
        assert_eq!(undos, 0);
    }

    /// The project panel is a short form of dropdowns (no wall of preset buttons): a Resolution tier
    /// resizes keeping the aspect, a Format preset sets size and frame rate - one undo each; W × H only
    /// show once "Custom…" is picked.
    #[test]
    fn project_dropdowns_apply_with_one_undo_each() {
        let mut h = H::project();
        h.frame(vec![]);
        assert!(h.maybe_rect("project_width").is_none(), "1920×1080 is a tier: no W × H fields");
        let pick = |h: &mut H, combo: &str, row: &str| {
            h.time += 1.0;
            let r = h.rect(combo);
            h.click(r.center());
            h.frame(vec![]);
            let item = h.text_at(row).unwrap_or_else(|| panic!("{combo} lists {row}"));
            h.time += 1.0;
            h.click(item)
        };
        assert!(pick(&mut h, "project_res", "720p"), "picking a tier edits the project");
        assert_eq!((h.project.width, h.project.height), (1280, 720), "16:9 kept, shorter edge 720");
        assert_eq!(h.undos, 1);
        assert!(pick(&mut h, "project_format", "General Short Form"));
        assert_eq!((h.project.width, h.project.height, h.project.fps), (1080, 1920, 60.0));
        assert_eq!(h.undos, 2);
        assert_eq!(h.project.max_duration, Some(60.0), "short form starts with a 1:00 Max length");
        assert!(crate::ui::guides::is_short_form(720, 1280) && !crate::ui::guides::is_short_form(1080, 1350));
        pick(&mut h, "project_res", "Custom…");
        h.frame(vec![]);
        assert!(h.maybe_rect("project_width").is_some(), "Custom… reveals W × H");
        assert_eq!(h.undos, 2, "revealing the fields is not an edit");
    }

    /// ws:max-duration: the Max length row turns on at 60 s and takes a quick pick, one undo each.
    #[test]
    fn max_length_row_toggles_and_picks_with_one_undo_each() {
        let mut h = H::project();
        h.frame(vec![]);
        h.time += 1.0;
        let r = h.rect("project_max_on");
        assert!(h.click(r.center()));
        assert_eq!(h.project.max_duration, Some(60.0));
        assert_eq!(h.undos, 1);
        h.time += 1.0;
        let r = h.rect("project_max_pick");
        h.click(r.center());
        h.frame(vec![]);
        let item = h.text_at("3:00").expect("the picks list 3:00");
        h.time += 1.0;
        assert!(h.click(item));
        assert_eq!(h.project.max_duration, Some(180.0));
        assert_eq!(h.undos, 2);
        // Set to Out: disabled without an Out point, then takes it as the limit
        h.project.out_point = Some(42.0);
        h.time += 1.0;
        h.frame(vec![]);
        let r = h.rect("project_max_out");
        assert!(h.click(r.center()));
        assert_eq!(h.project.max_duration, Some(42.0));
        assert_eq!(h.undos, 3);
        // Copy from Other Project: settings only
        let mut other = Project::new();
        (other.width, other.height, other.fps, other.max_duration) = (1080, 1920, 30.0, Some(15.0));
        other.name = "other".into();
        copy_project_settings(&mut h.project, &other);
        assert_eq!((h.project.width, h.project.height, h.project.fps, h.project.max_duration), (1080, 1920, 30.0, Some(15.0)));
        assert_ne!(h.project.name, "other");
        assert_eq!(parse_length("1:30"), Some(90.0));
        assert_eq!(parse_length(" 45 s"), Some(45.0));
        assert_eq!(parse_length("0"), None);
        // hours and days, no upper limit
        assert_eq!(parse_length("1:30:00"), Some(5400.0));
        assert_eq!(parse_length("9999h"), Some(9999.0 * 3600.0));
        assert_eq!(parse_length("1h 30m"), Some(5400.0));
        assert_eq!(parse_length("2d 1:00:00"), Some(2.0 * 86_400.0 + 3600.0));
        assert_eq!(parse_length("2 h"), Some(7200.0));
        assert_eq!(crate::ui::length_text(45.0), "0:45");
        assert_eq!(crate::ui::length_text(5400.0), "1:30:00");
        assert_eq!(crate::ui::length_text(9999.0 * 3600.0), "416d 15:00:00");
    }

    /// Presets can share a size (YouTube and General Long Form are both 1920×1080 at 60): the Format
    /// dropdown names the one whose guide overlay is on.
    #[test]
    fn format_names_the_preset_whose_guide_is_on() {
        let mut h = H::project();
        h.project.fps = 60.0;
        h.settings.guide = Some("yt_long".into());
        h.frame(vec![]);
        assert!(h.text_at("YouTube").is_some() && h.text_at("General Long Form").is_none());
        h.settings.guide = None;
        h.frame(vec![]);
        assert!(h.text_at("General Long Form").is_some());
    }

    /// Changing the project's Width via its DragValue opens the non-blocking "Resize existing media?"
    /// prompt whenever the project has a clip with a native size; "Rescale" applies Fit to Screen to it,
    /// "Keep As Is" leaves its transform untouched.
    #[test]
    fn resolution_change_offers_rescale_prompt() {
        fn project_with_video_clip() -> (Project, Id) {
            let mut p = Project::new(); // 1920x1080
            let aid = p.add_asset(Asset {
                id: 0,
                path: "C:/v.mp4".into(),
                kind: ClipKind::Video,
                duration: 5.0,
                width: 1280,
                height: 720,
                fps: 30.0,
                audio_streams: Vec::new(),
                codec: String::new(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
                rel_path: None,
                parent: None,
                range: None,
                effects: Vec::new(),
                html: None,
                proxy: None,
            });
            let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
            let mut c = Clip::new(500, ClipKind::Video, "v", 0.0, 5.0);
            c.asset = aid;
            c.x.value = 42.0; // pre-existing transform: "Keep As Is" must leave this alone
            let id = c.id;
            p.tracks[vi].clips.push(c);
            (p, id)
        }

        // drives the Width DragValue, then clicks whichever prompt button `rescale` names; returns the
        // project afterwards.
        let run = |rescale: bool| -> Project {
            let (mut p, _id) = project_with_video_clip();
            let palette = Palette::new(true, egui::Color32::WHITE);
            let fonts: Vec<String> = Vec::new();
            let ctx = egui::Context::default();
            let mut settings = Settings::default();
            let mut frame = |events: Vec<egui::Event>, p: &mut Project| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 900.0))),
                    events,
                    ..Default::default()
                };
                let mut undo = |_: &Project| {};
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show(ui, p, &[], &[], 1.0, &fonts, &palette, &mut settings, &mut undo);
                    });
                });
            };
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("insp_custom_size"), true)); // Resolution ▸ Custom…
            frame(vec![], &mut p); // layout, records the Width DragValue rect
            let wr = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", "project_width".to_string()))))
                .expect("width rect recorded");
            let (from, to) = (wr.center(), wr.center() + egui::vec2(60.0, 0.0));
            // drag the DragValue right, in a few steps like a real pointer move
            frame(vec![egui::Event::PointerMoved(from)], &mut p);
            frame(
                vec![egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            for i in 1..=4 {
                let pos = from + (to - from) * (i as f32 / 4.0);
                frame(vec![egui::Event::PointerMoved(pos)], &mut p);
            }
            frame(
                vec![egui::Event::PointerButton {
                    pos: to,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            frame(vec![], &mut p);
            assert_ne!(p.width, 1920, "dragging the Width control must change it");
            let prompt_open =
                ctx.data(|d| d.get_temp::<bool>(egui::Id::new("inspector_rescale_prompt"))).unwrap_or(false);
            assert!(prompt_open, "a resolution change with native-size media must open the rescale prompt");

            let btn = if rescale { "rescale_prompt_rescale" } else { "rescale_prompt_keep" };
            let br = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", btn.to_string()))))
                .unwrap_or_else(|| panic!("no rect recorded for {btn}"));
            let bp = br.center();
            frame(vec![egui::Event::PointerMoved(bp)], &mut p);
            frame(
                vec![egui::Event::PointerButton {
                    pos: bp,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            frame(
                vec![egui::Event::PointerButton {
                    pos: bp,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            p
        };

        let rescaled = run(true);
        let c = rescaled.clip(500).unwrap();
        assert_eq!(c.x.value, 0.0, "Rescale resets the transform (Fit to Screen)");
        assert_eq!((c.scale_x.value, c.scale_y.value), (1.0, 1.0));

        let kept = run(false);
        let c = kept.clip(500).unwrap();
        assert_eq!(c.x.value, 42.0, "Keep As Is must leave the clip's transform untouched");
    }

    /// Typing in a text field snapshots once when the field is entered, not once per character.
    #[test]
    fn name_edit_pushes_one_undo_per_visit() {
        let mut p = Project::new();
        let c = Clip::new(0, ClipKind::Text, "a", 0.0, 5.0);
        let id = c.id;
        p.tracks[0].clips.push(c);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut undos = 0;
        // focus is requested inside the pass: done between passes it counts as "had focus last frame"
        let mut run = |input: egui::RawInput, focus: Option<egui::Id>, p: &mut Project, undos: &mut usize| {
            let mut edited = false;
            let _ = ctx.run(input, |ctx| {
                if let Some(f) = focus {
                    ctx.memory_mut(|m| m.request_focus(f));
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    edited = show(ui, p, &[id], &[], 1.0, &[], &palette, &mut Settings::default(), &mut undo);
                });
            });
            edited
        };
        run(egui::RawInput::default(), None, &mut p, &mut undos); // layout, records the name field id
        assert_eq!(undos, 0);
        let field = ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_name_field"))).expect("name id");
        assert!(!run(egui::RawInput::default(), Some(field), &mut p, &mut undos), "entering a field is not an edit");
        assert_eq!(undos, 1, "one snapshot when the field is entered");
        for ch in ["h", "i"] {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Text(ch.into()));
            assert!(run(input, None, &mut p, &mut undos));
        }
        assert_eq!(undos, 1, "no extra snapshot per keystroke");
        assert!(p.clip(id).unwrap().name.contains("hi"), "{}", p.clip(id).unwrap().name);
    }

    /// Settings with every clip section open, primary or not - for tests about a section's contents.
    fn open_all() -> Settings {
        let mut s = Settings::default();
        for id in ["transform", "opacity", "audio", "color", "effects", "text", "shape", "mask", "path", "asset"] {
            s.inspector_folds.insert(id.into(), true);
            s.inspector_folds.insert(format!("{id}*"), true);
        }
        s
    }

    struct H {
        ctx: egui::Context,
        project: Project,
        id: Id,
        /// Selected transitions handed to show(); when non-empty the clip selection is left empty.
        sel_trans: Vec<Id>,
        /// Nothing selected: the project panel.
        none: bool,
        undos: usize,
        time: f64,
        playhead: f64,
        settings: Settings,
        shapes: Vec<egui::epaint::ClippedShape>,
        /// More selected clips after `id`.
        extra: Vec<Id>,
    }

    impl H {
        fn with(project: Project, id: Id) -> Self {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            let (sel_trans, none, undos, time, playhead) = (Vec::new(), false, 0, 0.0, 1.0);
            let (settings, shapes, extra) = (open_all(), Vec::new(), Vec::new());
            Self { ctx, project, id, sel_trans, none, undos, time, playhead, settings, shapes, extra }
        }
        fn shape_clip() -> Self {
            let mut project = Project::new();
            let id = project.add_shape_clip(crate::model::ShapeKind::Star, 0.0, 3.0);
            Self::with(project, id)
        }
        fn audio_clip() -> Self {
            let mut project = Project::new();
            let id = project.new_id();
            project.tracks[1].clips.push(Clip::new(id, ClipKind::Audio, "a", 0.0, 3.0));
            Self::with(project, id)
        }
        /// A video clip at 0..3 whose Position X is keyed at 0.5 s and 2.5 s (the playhead sits at 1 s).
        fn video_clip() -> Self {
            let mut project = Project::new();
            let id = project.new_id();
            let mut c = Clip::new(id, ClipKind::Video, "v", 0.0, 3.0);
            c.x.toggle_key(0.5);
            c.x.toggle_key(2.5);
            project.tracks[0].clips.push(c);
            Self::with(project, id)
        }
        fn project() -> Self {
            let mut h = Self::with(Project::new(), 0);
            h.none = true;
            h
        }
        /// Two abutting video clips with a cut transition and an edge transition, both selected.
        fn transitions() -> Self {
            use crate::model::TransitionKind;
            let mut project = Project::new();
            let a = project.new_id();
            project.tracks[0].clips.push(Clip::new(a, ClipKind::Video, "a", 0.0, 2.0));
            let b = project.new_id();
            project.tracks[0].clips.push(Clip::new(b, ClipKind::Video, "b", 2.0, 2.0));
            let t1 = project.add_transition(b, TransitionKind::CrossFade, 1.0).unwrap();
            let t2 = project.add_edge_transition(a, TransitionKind::CrossFade, 1.0, false).unwrap();
            let mut h = Self::with(project, a);
            h.sel_trans = vec![t1, t2];
            h
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 4000.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, egui::Color32::WHITE);
            let H { ctx, project, id, sel_trans, none, undos, playhead, settings, extra, .. } = self;
            let sel: Vec<Id> = if sel_trans.is_empty() && !*none {
                std::iter::once(*id).chain(extra.iter().copied()).collect()
            } else {
                vec![]
            };
            let sel = &sel[..];
            let mut changed = false;
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, project, sel, sel_trans, *playhead, &[], &pal, settings, &mut undo);
                });
            });
            self.shapes = out.shapes;
            changed
        }
        fn maybe_rect(&self, name: &str) -> Option<egui::Rect> {
            self.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", name.to_string()))))
        }
        fn rect(&self, name: &str) -> egui::Rect {
            self.maybe_rect(name).unwrap_or_else(|| panic!("no widget rect for {name}"))
        }
        /// Centre of the painted text that reads exactly `label` - how a menu's rows are found.
        fn text_at(&self, label: &str) -> Option<egui::Pos2> {
            self.shapes.iter().rev().find_map(|c| match &c.shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
        }
        fn press(&mut self, pos: egui::Pos2, button: egui::PointerButton) -> bool {
            let mut e = self.frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                e |= self.frame(vec![egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            e
        }
        fn click(&mut self, pos: egui::Pos2) -> bool {
            self.press(pos, egui::PointerButton::Primary)
        }
        /// Drag from `pos` to the right by `dx` (a DragValue edit). True if anything edited the project.
        fn drag(&mut self, pos: egui::Pos2, dx: f32) -> bool {
            self.time += 1.0;
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut e = self.frame(vec![egui::Event::PointerMoved(pos)]);
            e |= self.frame(vec![press(true)]);
            for i in 1..=4 {
                e |= self.frame(vec![egui::Event::PointerMoved(pos + egui::vec2(dx * i as f32 / 4.0, 0.0))]);
            }
            e | self.frame(vec![press(false)])
        }
        /// How many painted texts read exactly `label`.
        fn count_text(&self, label: &str) -> usize {
            self.shapes
                .iter()
                .filter(|c| matches!(&c.shape, egui::epaint::Shape::Text(t) if t.galley.text() == label))
                .count()
        }
        /// Right-click `pos`, let the menu draw, then click its `row`. True if anything edited the project.
        fn menu_pick(&mut self, pos: egui::Pos2, row: &str) -> bool {
            let mut e = self.press(pos, egui::PointerButton::Secondary);
            e |= self.frame(vec![]);
            let at = self.text_at(row).unwrap_or_else(|| panic!("no menu row '{row}'"));
            self.time += 1.0;
            e | self.click(at)
        }
    }

    /// Selected transitions get their own inspector section, and "Remove" deletes them all in one undo.
    #[test]
    fn transition_section_shows_and_removes_all_selected() {
        let mut h = H::transitions();
        h.frame(vec![]);
        let r = h.rect("tr_remove");
        assert!(h.click(r.center()), "removing transitions edits the project");
        assert!(h.project.tracks[0].transitions.is_empty(), "both selected transitions removed");
        assert_eq!(h.undos, 1, "one undo for the whole removal");
        // redrawing with the stale selection is not an edit (the section shows a hint instead)
        assert!(!h.frame(vec![]));
        assert_eq!(h.undos, 1);
    }

    /// The mask section appears, "Add mask" creates one with a single undo, and the follow-up
    /// "Edit in viewport" hands the clip to the app.
    #[test]
    fn mask_section_adds_and_hands_off() {
        let mut h = H::shape_clip();
        h.frame(vec![]);
        let r = h.rect("add_mask");
        assert!(h.click(r.center()), "adding a mask edits the project");
        assert!(h.project.clip(h.id).unwrap().mask.is_some());
        assert_eq!(h.undos, 1, "one undo per gesture");
        h.frame(vec![]);
        let r = h.rect("edit_mask");
        let _ = take_edit_mask();
        h.click(r.center());
        assert_eq!(take_edit_mask(), Some(h.id), "the app is told which clip to mask");
        assert_eq!(h.undos, 1, "asking to edit in the viewport is not an edit");
    }

    /// A mask shapes pixels, so an audio clip shows no mask section at all - not even a dead button,
    /// and not even for a mask a hand-edited project smuggled in.
    #[test]
    fn audio_clips_get_no_mask_section() {
        let mut h = H::audio_clip();
        h.frame(vec![]);
        assert!(h.maybe_rect("add_mask").is_none(), "no Add mask on an audio clip");
        h.project.clip_mut(h.id).unwrap().mask = Some(Mask::default());
        h.frame(vec![]);
        assert!(h.maybe_rect("edit_mask").is_none(), "and no editor for one that got in anyway");
    }

    /// Shape clips get the style section; clip markers are the Markers pane's / M's, not the inspector's.
    #[test]
    fn shape_section_and_no_clip_markers() {
        let mut h = H::shape_clip();
        h.frame(vec![]);
        assert!(h.project.clip(h.id).unwrap().shape.is_some(), "the shape style exists");
        assert!(h.text_at("Kind").is_some(), "the Shape section draws its style grid");
        assert!(h.maybe_rect("add_marker").is_none(), "no second home for clip markers");
    }

    /// "Open in Node editor" lives on the Effects section's right-click (no header button any more).
    #[test]
    fn effects_title_menu_opens_the_node_editor() {
        let mut h = H::shape_clip();
        h.frame(vec![]);
        let _ = take_open_nodes();
        let title = h.rect("sec_effects").left_center() + egui::vec2(10.0, 0.0);
        assert!(!h.menu_pick(title, "Open in Node editor"), "asking for the node editor is not an edit");
        assert_eq!(take_open_nodes(), Some(h.id));
        assert_eq!(h.undos, 0);
    }

    /// The label combo lists the project's own labels, and removing one re-points the clips using it.
    #[test]
    fn labels_come_from_the_project() {
        let mut p = Project::new();
        assert!(!p.labels.is_empty(), "a new project ships the default labels");
        let n = p.labels.len();
        let idx = p.add_label("Hero", [10, 20, 30]);
        assert_eq!(idx as usize, n + 1);
        assert_eq!(p.label_name(idx), "Hero");
        assert_eq!(p.label_color(idx), Some([10, 20, 30]));
        let cid = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 1.0);
        p.clip_mut(cid).unwrap().label = idx;
        p.remove_label(idx);
        assert_eq!(p.clip(cid).unwrap().label, 0, "the clip falls back to no label");
    }

    /// The Speed row (in Transform): right-click the field for Retime… / Freeze (queued like any menu Action -
    /// the old "Retime… Ctrl+R" button is gone), and dragging it retimes the clip with one undo.
    #[test]
    fn speed_menu_queues_retime_and_the_field_retimes() {
        let mut h = H::video_clip();
        h.frame(vec![]);
        let _ = crate::ui::menu::take_queued();
        let field = h.rect("speed").center();
        let name = egui::pos2(h.rect("speed").left() - 40.0, field.y);
        assert!(!h.menu_pick(name, Action::Retime.label()), "opening Retime is not an edit");
        assert_eq!(crate::ui::menu::take_queued(), vec![Action::Retime]);
        assert_eq!(h.undos, 0);

        h.time += 1.0;
        let press = |pressed| egui::Event::PointerButton {
            pos: field,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(vec![egui::Event::PointerMoved(field)]);
        h.frame(vec![press(true)]);
        let mut edited = false;
        for i in 1..=4 {
            edited |= h.frame(vec![egui::Event::PointerMoved(field + egui::vec2(10.0 * i as f32, 0.0))]);
        }
        edited |= h.frame(vec![press(false)]);
        assert!(edited, "dragging the speed field edits the project");
        let c = h.project.clip(h.id).unwrap();
        assert!(c.speed > 1.0, "faster: {}", c.speed);
        assert!((c.duration - 3.0 / c.speed).abs() < 1e-6, "Project::set_speed shortened it: {}", c.duration);
        assert_eq!(h.undos, 1, "one drag = one undo");
    }

    /// Only one section starts open - the one this clip needs first, or the page's own on the Color /
    /// Audio pages - and each (section, leads-here) pair remembers its own fold, so seeing a section
    /// closed where it doesn't lead never closes it where it does (the old effects_primary bug).
    #[test]
    fn the_primary_section_opens_per_kind_and_page() {
        let mut p = Project::new();
        let text = Clip::new(500, ClipKind::Text, "t", 0.0, 3.0);
        p.tracks[0].clips.push(text);
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        p.tracks[vi].clips.push(Clip::new(501, ClipKind::Video, "v", 4.0, 3.0));
        let ai = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Audio).unwrap();
        p.tracks[ai].clips.push(Clip::new(502, ClipKind::Audio, "a", 4.0, 3.0));
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut undo = |_: &Project| {};
        let mut draw = |sel: Id, settings: &mut Settings, p: &mut Project| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, p, &[sel], &[], 1.0, &[], &palette, settings, &mut undo);
                });
            });
        };
        let open = |key: &str| {
            egui::collapsing_header::CollapsingState::load(&ctx, egui::Id::new(("insp_section", key)))
                .is_some_and(|s| s.is_open())
        };
        // a Text clip first: its Text section leads, Transform does not
        draw(500, &mut settings, &mut p);
        assert!(open("text*") && !open("transform"), "Text leads for a Text clip");
        // then a video clip: Transform leads - not stuck closed by the Text clip's closed Transform
        draw(501, &mut settings, &mut p);
        assert!(open("transform*"), "Transform leads for a video clip");
        assert!(!open("color"), "Color stays closed on the Edit page");
        // the Color page leads with Color, the Audio page (an audio clip) with Audio
        settings.page = "Color".into();
        draw(501, &mut settings, &mut p);
        assert!(open("color*"), "Color leads on the Color page");
        settings.page = "Audio".into();
        draw(502, &mut settings, &mut p);
        assert!(open("audio*"), "Audio leads on the Audio page");
    }

    /// A fold toggle (the real write path in `section()`: the persisted `CollapsingState` disagreeing
    /// with `folds`' remembered default) survives a `Settings` JSON round-trip - the same serialize/
    /// deserialize `Settings::save`/`load` do (see `settings.rs`'s own round-trip tests for the pattern).
    #[test]
    fn fold_state_persists_in_settings() {
        let ctx = egui::Context::default();
        let mut folds: BTreeMap<String, bool> = BTreeMap::new();
        // First draw: "color" has never been seen before, defaults closed - no entry written yet.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                section(ui, "color", "Color", false, &mut folds, None, |_| {});
            });
        });
        assert!(!folds.contains_key("color"), "an untouched fold writes no entry");

        // Flip it open - exactly what clicking the header does - then redraw so `section()` notices
        // `now_open != open_default` and records it.
        let cid = egui::Id::new(("insp_section", "color"));
        let mut state =
            egui::collapsing_header::CollapsingState::load(&ctx, cid).expect("state exists after the first draw");
        state.set_open(true);
        state.store(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                section(ui, "color", "Color", false, &mut folds, None, |_| {});
            });
        });
        assert_eq!(folds.get("color"), Some(&true), "the toggle must be written into the folds map");

        let mut settings = Settings::default();
        settings.inspector_folds = folds;
        let restored: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            restored.inspector_folds.get("color"),
            Some(&true),
            "the fold state must survive a Settings JSON round-trip"
        );
    }

    /// One ◆ per property; its right-click has what used to be inline: Previous / Next key (a seek on
    /// the timeline), Clear keyframes, and the live links (the old ∿ button).
    #[test]
    fn keyframe_menu_seeks_clears_and_links() {
        let mut h = H::video_clip();
        h.frame(vec![]);
        let kf = h.rect("kf_Position X").center();
        let _ = take_pending_seek();
        assert!(!h.menu_pick(kf, "Next key"), "a seek is not an edit");
        assert_eq!(take_pending_seek(), Some(2.5), "the next Position X key, in timeline time");
        assert!(!h.menu_pick(kf, "Previous key"));
        assert_eq!(take_pending_seek(), Some(0.5));
        assert_eq!(h.undos, 0);

        assert!(h.menu_pick(kf, "Clear keyframes"));
        assert!(!h.project.clip(h.id).unwrap().x.is_animated(), "keys cleared");
        assert_eq!(h.undos, 1);

        assert!(h.menu_pick(kf, "Link to expression…"));
        assert!(matches!(h.project.clip(h.id).unwrap().x.link, crate::model::AnimLink::Expr(_)));
        assert_eq!(h.undos, 2);
        // right-clicking the VALUE opens the same menu (a linked value is greyed, so use Y's)
        let y = h.rect("val_Position Y").center();
        assert!(h.menu_pick(y, "Link to expression…"));
        assert!(matches!(h.project.clip(h.id).unwrap().y.link, crate::model::AnimLink::Expr(_)));
        // ws:keyframe-blocks: Animate ▸ applies a block at the playhead with one undo
        let before = h.undos;
        crate::ui::BLOCK_REQ.with(|s| *s.borrow_mut() = Some("Spin".into()));
        h.frame(vec![]);
        assert!(h.project.clip(h.id).unwrap().rotation.is_animated(), "Spin wrote rotation keys");
        assert_eq!(h.project.clip(h.id).unwrap().blocks.len(), 1);
        assert_eq!(h.undos, before + 1);
    }

    /// The property list is a table: the value box and every control sit at the same x on a row with a
    /// short value and on one with a long value, and a double-click on a name puts a key at the playhead.
    #[test]
    fn property_rows_keep_their_columns_whatever_the_value() {
        let mut h = H::video_clip();
        {
            let c = h.project.clip_mut(h.id).unwrap();
            c.y.value = -123456.789;
            c.rotation.value = 3.0;
        }
        h.frame(vec![]);
        let labels = ["Position X", "Position Y", "Scale X", "Scale Y", "Rotation", "Opacity"];
        let col = |h: &H, what: &str| -> Vec<(f32, f32)> {
            labels.iter().map(|l| h.rect(&format!("{what}_{l}"))).map(|r| (r.left(), r.right())).collect()
        };
        for what in ["val", "kf"] {
            let c = col(&h, what);
            assert!(c.iter().all(|x| *x == c[0]), "{what} column moves between rows: {c:?}");
        }
        let (val, kf) = (col(&h, "val")[0], col(&h, "kf")[0]);
        assert!(val.1 <= kf.0 && val.1 - val.0 > 60.0, "the value box ends before the controls: {val:?} {kf:?}");
        // the value box starts right after the fixed name column, the keys right after it: nothing pushed right
        assert!(val.0 - h.rect("sec_transform*").left() < crate::ui::PROP_LABEL_W + 40.0, "{val:?}");
        assert!(kf.0 - val.1 < 30.0, "the keys follow the value box: {val:?} {kf:?}");
        // the link sits between its two rows, right of the controls, the same column for both pairs
        assert_eq!(h.rect("pos_xy").left(), h.rect("scale_xy").left());
        assert!(h.rect("pos_xy").left() > kf.1);
        let (px, py) = (h.rect("val_Position X"), h.rect("val_Position Y"));
        let link = h.rect("pos_xy");
        assert!(link.top() <= px.center().y + 1.0 && link.bottom() >= py.center().y - 1.0, "{link:?}");
        // a double-click on the name (left of the value box) keys the property at the playhead
        let row = h.rect("val_Rotation");
        let name = egui::pos2(row.left() - 60.0, row.center().y);
        assert!(!h.project.clip(h.id).unwrap().rotation.is_animated());
        h.click(name);
        h.click(name);
        let rot = &h.project.clip(h.id).unwrap().rotation;
        assert!(rot.has_key_at(1.0) && rot.keys.len() == 1, "{:?}", rot.keys);
    }

    /// A row's controls start at the same x whatever its name: a long name is cut short ("…"), never
    /// pushing the value or the keys right.
    #[test]
    fn prop_rows_align_whatever_the_name() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let mut rects = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut g = crate::ui::Gesture::default();
                for name in ["X", "Rotation", "A property name far too long for the name column of the Inspector"] {
                    let mut a = crate::model::Animated::new(1.0);
                    rects.push(crate::ui::prop_row(ui, name, &mut a, 0.0, &pal, &mut g, &[], |v| DragValue::new(v)));
                }
            });
        });
        let xs: Vec<(f32, f32)> = rects.iter().map(|(v, k)| (v.rect.left(), k.rect.left())).collect();
        assert!(xs.iter().all(|x| *x == xs[0]), "{xs:?}");
    }

    /// A visual clip's properties are two sections, Transform and Blend and fades (Opacity there), with no
    /// "Properties" fold and no single Scale row.
    #[test]
    fn transform_and_blend_sections_only() {
        let mut h = H::video_clip();
        h.frame(vec![]);
        assert!(h.maybe_rect("sec_transform*").is_some() && h.maybe_rect("sec_opacity").is_some());
        assert!(h.text_at("Transform").is_some() && h.text_at("Blend and fades").is_some());
        assert!(h.text_at("Properties").is_none() && h.text_at("Scale").is_none());
        let (op, sec) = (h.rect("val_Opacity"), h.rect("sec_opacity"));
        assert!(op.top() > sec.top(), "Opacity is in Blend and fades");
    }

    /// Scale X / Y are linked by default (editing one sets the other, through the uniform scale); unlinked
    /// each edits its own axis. Position X / Y are not linked by default; linked, a key on X lands on Y too.
    #[test]
    fn scale_links_by_default_position_does_not() {
        let mut h = H::video_clip();
        h.frame(vec![]);
        assert!(!h.project.clip(h.id).unwrap().scale_unlinked && !h.project.clip(h.id).unwrap().pos_linked);
        let sx = h.rect("val_Scale X").center();
        assert!(h.drag(sx, 40.0));
        let c = h.project.clip(h.id).unwrap().clone();
        assert!(c.scale.value > 1.0 && c.scale_x.value == 1.0 && c.scale_y.value == 1.0, "linked: the uniform scale");
        assert_eq!(h.undos, 1);
        // unlink: Scale X moves alone
        let link = h.rect("scale_xy").center();
        assert!(h.click(link));
        assert!(h.project.clip(h.id).unwrap().scale_unlinked);
        let sx = h.rect("val_Scale X").center();
        let s0 = h.project.clip(h.id).unwrap().scale.value;
        assert!(h.drag(sx, 40.0));
        let c = h.project.clip(h.id).unwrap().clone();
        assert!(c.scale_x.value > 1.0 && c.scale_y.value == 1.0 && c.scale.value == s0, "unlinked: X alone");
        // Position: a key on X stays on X
        let ky = |h: &H| h.project.clip(h.id).unwrap().y.is_animated();
        let kx = h.rect("kf_Position X").center();
        h.click(kx); // removes the key at the playhead? no key at 1 s: adds one
        assert!(!ky(&h), "Position is not linked by default");
        let link = h.rect("pos_xy").center();
        h.click(link);
        assert!(h.project.clip(h.id).unwrap().pos_linked);
        let kx = h.rect("kf_Position X").center();
        h.time += 1.0;
        h.click(kx);
        let y = &h.project.clip(h.id).unwrap().y;
        let x = &h.project.clip(h.id).unwrap().x;
        assert_eq!(x.has_key_at(1.0), y.has_key_at(1.0), "linked: Y follows X's key");
    }

    /// An old project's uniform scale reads as linked Scale X = Scale Y.
    #[test]
    fn old_uniform_scale_reads_as_linked_x_and_y() {
        let mut p = Project::new();
        let id = p.new_id();
        let mut c = Clip::new(id, ClipKind::Video, "v", 0.0, 3.0);
        c.scale.value = 2.0;
        p.tracks[0].clips.push(c);
        let json = p.to_json();
        assert!(!json.contains("scale_unlinked") && !json.contains("pos_linked"), "defaults are not written");
        let p = Project::from_json(&json).unwrap();
        let mut h = H::with(p, id);
        h.frame(vec![]);
        assert!(!h.project.clip(id).unwrap().scale_unlinked);
        // the value boxes format with decimals ("2.00"), so match the number, not the text
        let twos = h
            .shapes
            .iter()
            .filter(|c| matches!(&c.shape, egui::epaint::Shape::Text(t) if t.galley.text().trim().parse::<f64>() == Ok(2.0)))
            .count();
        assert!(twos >= 2, "Scale X and Scale Y both read 2");
    }

    /// An audio clip has exactly one Speed row and no Speed (or Sound) fold.
    #[test]
    fn audio_clip_has_one_speed_row() {
        let mut h = H::audio_clip();
        h.frame(vec![]);
        assert_eq!(h.count_text("Speed"), 1, "one Speed row");
        assert!(h.maybe_rect("sec_speed").is_none() && h.maybe_rect("sec_speed*").is_none());
        assert!(h.text_at("Sound").is_none());
        assert!(h.maybe_rect("speed").is_some());
    }

    /// A video clip with linked audio shows the audio's Volume / Pan with it, the pair counts as one clip,
    /// and there is still one Speed row (the video's, which retimes both).
    #[test]
    fn linked_audio_props_show_with_the_video() {
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let ai = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Audio).unwrap();
        let (v, a) = (p.new_id(), p.new_id());
        let mut vc = Clip::new(v, ClipKind::Video, "v", 0.0, 3.0);
        let mut ac = Clip::new(a, ClipKind::Audio, "a", 0.0, 3.0);
        vc.link = 99;
        ac.link = 99;
        p.tracks[vi].clips.push(vc);
        p.tracks[ai].clips.push(ac);
        for extra in [vec![], vec![a]] {
            let mut h = H::with(p.clone(), v);
            h.extra = extra;
            h.frame(vec![]);
            assert!(h.maybe_rect("sec_transform*").is_some(), "the video's Transform");
            assert!(h.maybe_rect("sec_audio*").is_some(), "and the linked audio's Audio section");
            assert!(h.ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("test_pan_slider"))).is_some());
            assert_eq!(h.count_text("Speed"), 1, "the one Speed row, in Transform");
            assert!(h.maybe_rect("sec_speed").is_none() && h.maybe_rect("sec_speed*").is_none());
            let name = h.ctx.data(|d| d.get_temp::<bool>(egui::Id::new("test_name_field_enabled")));
            assert_eq!(name, Some(true), "the pair is one clip");
            // a Volume edit lands on the audio clip, one undo
            let vol = h.rect("val_Volume").center();
            assert!(h.drag(vol, -40.0));
            assert!(h.project.clip(a).unwrap().volume.value < 1.0);
            assert_eq!(h.undos, 1);
        }
    }

    /// Two text clips selected: a style edit applies to both with one undo, and a field they disagree on
    /// reads "Mixed" until it is changed.
    #[test]
    fn multi_text_edit_applies_to_all() {
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let (a, b) = (p.new_id(), p.new_id());
        for (id, at, size, words) in [(a, 0.0, 40.0, "one"), (b, 4.0, 90.0, "two")] {
            let mut c = Clip::new(id, ClipKind::Text, "t", at, 3.0);
            let mut st = crate::model::TextStyle::default();
            st.text = words.into();
            st.size = crate::model::Animated::new(size);
            c.text = Some(st);
            p.tracks[vi].clips.push(c);
        }
        let mut h = H::with(p, a);
        h.extra = vec![b];
        h.frame(vec![]);
        assert!(h.count_text("Mixed") >= 2, "size and text read Mixed");
        // Bold on: both
        let bold = h.text_at("Bold").unwrap() - egui::vec2(20.0, 0.0);
        assert!(h.click(bold));
        let st = |h: &H, id: Id| h.project.clip(id).unwrap().text.clone().unwrap();
        assert!(st(&h, a).bold && st(&h, b).bold);
        assert_eq!(h.undos, 1, "one undo for both");
        // the size: one value for both after a drag
        let size = h.rect("text_size").center();
        assert!(h.drag(size, 30.0));
        assert_eq!(st(&h, a).size, st(&h, b).size);
        assert_eq!(h.undos, 2);
        assert_eq!(st(&h, a).text, "one", "the wording stays each clip's own");
        assert_eq!(st(&h, b).text, "two");
    }

    /// The header is label dot · name · enable: the dot's menu sets the label, the eye switches the
    /// clip off - one undo each.
    #[test]
    fn header_label_dot_and_enable_toggle() {
        let mut h = H::video_clip();
        h.frame(vec![]);
        let name = h.project.labels[0].name.clone();
        let dot = h.rect("label_dot").center();
        h.click(dot);
        h.frame(vec![]);
        let row = h.text_at(&name).expect("the label menu lists the project's labels");
        h.time += 1.0;
        assert!(h.click(row));
        assert_eq!(h.project.clip(h.id).unwrap().label, 1);
        assert_eq!(h.undos, 1);
        h.time += 1.0;
        let eye = h.rect("enabled").center();
        assert!(h.click(eye));
        assert!(!h.project.clip(h.id).unwrap().enabled, "switched off");
        assert_eq!(h.undos, 2);
    }

    /// A nested sequence's audio twin (a Sequence clip on an audio track) is audio: it gets the Audio
    /// section, and neither Color nor Mask - they follow the track, not `Clip::is_visual()` (#82).
    #[test]
    fn audio_twin_gets_audio_not_color_or_mask() {
        let mut p = Project::new();
        let ai = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Audio).unwrap();
        let id = p.new_id();
        p.tracks[ai].clips.push(Clip::new(id, ClipKind::Sequence, "nest", 0.0, 3.0));
        let mut h = H::with(p, id);
        h.frame(vec![]);
        assert!(h.maybe_rect("sec_audio*").is_some(), "the Audio section leads");
        assert!(h.ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("test_pan_slider"))).is_some());
        for key in ["sec_color", "sec_color*", "sec_mask", "sec_mask*", "sec_transform", "sec_transform*"] {
            assert!(h.maybe_rect(key).is_none(), "{key} must not show on an audio track");
        }
        // the same Sequence clip on a video track is the picture half: Color and Mask are back
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let id = p.new_id();
        p.tracks[vi].clips.push(Clip::new(id, ClipKind::Sequence, "nest", 0.0, 3.0));
        let mut h = H::with(p, id);
        h.frame(vec![]);
        assert!(h.maybe_rect("sec_color").is_some() && h.maybe_rect("sec_mask").is_some());
    }

    /// Headless: sections for effects / retime / audio fades lay out without panicking.
    #[test]
    fn show_headless_sections() {
        let a = Asset {
            id: 0,
            path: r"C:\m\a.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: vec![Default::default()],
            codec: String::new(),
            folder: String::new(),
            tags: vec!["x".into()],
            label: 2,
            description: "d".into(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        };
        let mut p = Project::from_media(a);
        let vid = p.tracks[0].clips[0].id;
        p.clip_mut(vid).unwrap().effects.push(crate::model::Effect::new(crate::model::EffectKind::Blur));
        p.clip_mut(vid).unwrap().speed = 2.0;
        let right = p.split_at(4.0, None)[0];
        p.add_transition(right, crate::model::TransitionKind::CrossFade, 0.5);
        let aud = p.tracks[1].clips[0].id;
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        for sel in [vid, right, aud] {
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| panic!("no undo without edits");
                        assert!(!show(
                            ui,
                            &mut p,
                            &[sel],
                            &[],
                            1.0,
                            &fonts,
                            &palette,
                            &mut Settings::default(),
                            &mut undo
                        ));
                    });
                });
            }
        }
    }
}
