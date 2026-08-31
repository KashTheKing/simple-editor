//! Standalone Moodboard pane (`Project.moodboard`): a project-wide gallery of reference assets with
//! free-form label tags — separate from the per-task moodboards already on the Planner's items
//! (`PlanItem::assets`, `ui::planner`).
//!
//! Three views, toggled the same way library.rs's List/Gallery buttons work (same Glyph icons): List
//! (rows, with an editable comma-separated tag field), Gallery (thumbnail grid, tags shown read-only)
//! and Slideshow (one big item at a time with Prev/Next, tags editable). A text filter hides items whose
//! tags don't match (`label_matches`).
//!
//! Drag-and-drop, two paths onto the same de-dup (`moodboard_add`): dropping an existing library asset
//! (`DragPayload::Asset`, egui's own drag-and-drop) onto empty pane space adds a `MoodItem` for it
//! directly; dropping an OS file (from Explorer) is routed here by `App::handle_drops`, which imports it
//! as a project asset first (deduped by path, the same flow every other import uses) and then calls
//! `moodboard_add` too — "if it isn't already one" is `moodboard_add`'s own check either way.
//!
//! Right-click (or the inline button) on an item: "Add at Playhead" places that asset on the timeline
//! via `add_to_timeline`, the same plumbing Library and Planner already use
//! (`Project::insert_asset_clips`). `Project::plan_assets()` already protects every `MoodItem.asset`
//! from "Remove unused assets" (see model.rs).

use crate::media::thumbs::ThumbCache;
use crate::model::{ClipKind, Id, MoodItem, Project};
use crate::theme::Palette;
use crate::ui::markers_ui::x_button;
use crate::ui::tools::{draw_glyph, icon_button, Dir, Glyph};
use crate::ui::DragPayload;
use eframe::egui::{self, RichText, TextEdit};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    List,
    Gallery,
    Slideshow,
}

pub struct MoodboardState {
    view: View,
    pub filter: String,
    slide_index: usize,
    /// This pane's content rect as of the last frame it was drawn — `App::handle_drops` checks an OS
    /// file drop's cursor position against it before importing, one-frame-stale, the same trick
    /// `TimelineState::lanes_rect` uses. `App::update` resets it to `NOTHING` right after `handle_drops`
    /// runs each frame, so a different tab sharing this pane's screen area (Markers/Planner) is never
    /// mistaken for the Moodboard on a frame this pane isn't actually the active tab.
    pub content_rect: egui::Rect,
}

impl Default for MoodboardState {
    fn default() -> Self {
        Self { view: View::Gallery, filter: String::new(), slide_index: 0, content_rect: egui::Rect::NOTHING }
    }
}

#[derive(Default)]
pub struct MoodboardResponse {
    pub edited: bool,
    /// Asset ids the user asked to place on the timeline at the playhead.
    pub add_to_timeline: Vec<Id>,
}

/// Test-only registry of widget rects, so headless tests can click real widgets without pixel-guessing
/// (same trick as markers_ui.rs's `mark`).
#[cfg(test)]
fn mark(ui: &egui::Ui, name: impl std::fmt::Display, r: &egui::Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("mb", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: impl std::fmt::Display, _r: &egui::Response) {}

/// Add a `MoodItem` for `asset` if it isn't already on the board. Returns whether it changed anything —
/// the one place both drop paths (internal drag here, OS-file-drop-then-import in `App::handle_drops`)
/// funnel through, so "already a project asset" and "already on the board" are each checked in exactly
/// one spot.
pub(crate) fn moodboard_add(project: &mut Project, asset: Id) -> bool {
    if project.moodboard.iter().any(|m| m.asset == asset) {
        return false;
    }
    project.moodboard.push(MoodItem { asset, labels: Vec::new() });
    true
}

/// Does this item's tags match the filter? Case-insensitive substring on any tag; an empty filter shows
/// everything. `filter` is expected already-lowercased (the caller lowercases it once, not per item).
fn label_matches(m: &MoodItem, filter: &str) -> bool {
    filter.is_empty() || m.labels.iter().any(|l| l.to_lowercase().contains(filter))
}

fn split_tags(s: &str) -> Vec<String> {
    s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

const TILE: f32 = 110.0;
const THUMB: f32 = 40.0;

/// A thumbnail (video/image) or a kind glyph placeholder (audio, or no thumbnail yet), `size` square.
fn asset_thumb(ui: &mut egui::Ui, project: &Project, thumbs: &mut ThumbCache, asset: Id, size: f32, palette: &Palette) {
    if let Some(a) = project.asset(asset) {
        if a.has_video() {
            if let Some((tex, [w, h])) = thumbs.texture(ui.ctx(), &a.path, 0.0, size as u32) {
                let scale = size / (w.max(h).max(1) as f32);
                ui.add(egui::Image::new(egui::load::SizedTexture::new(tex, egui::vec2(w as f32 * scale, h as f32 * scale))));
                return;
            }
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let icon = if a.kind == ClipKind::Audio { Glyph::MusicNote } else { Glyph::FilmStrip };
        draw_glyph(ui.painter(), rect, icon, palette.text_dim);
    } else {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, palette.border);
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut MoodboardState,
    project: &mut Project,
    thumbs: &mut ThumbCache,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> MoodboardResponse {
    let mut resp = MoodboardResponse::default();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.weak(RichText::new("View").small());
        for (v, glyph, tip) in [
            (View::List, Glyph::ListIcon, "List view"),
            (View::Gallery, Glyph::GridIcon, "Gallery view"),
            (View::Slideshow, Glyph::PlayRect, "Slideshow"),
        ] {
            let r = icon_button(ui, palette, egui::Id::new(("mood_view", tip)), glyph, tip, state.view == v);
            mark(ui, format!("view_{tip}"), &r);
            if r.clicked() {
                state.view = v;
                state.slide_index = 0;
            }
        }
        ui.separator();
        let w = (ui.available_width() - 10.0).max(60.0);
        ui.add(TextEdit::singleline(&mut state.filter).hint_text("filter by tag").desired_width(w));
    });
    ui.separator();

    let filter = state.filter.to_lowercase();
    let visible: Vec<usize> =
        (0..project.moodboard.len()).filter(|&i| label_matches(&project.moodboard[i], &filter)).collect();

    let mut remove: Option<usize> = None;
    let mut relabel: Option<(usize, Vec<String>)> = None;
    let content_rect = ui.available_rect_before_wrap();

    let (frame_r, payload) = ui.dnd_drop_zone::<DragPayload, ()>(egui::Frame::group(ui.style()), |ui| {
        if project.moodboard.is_empty() {
            ui.weak("Drag files here to add them to the moodboard.");
        } else if visible.is_empty() {
            ui.weak("No items match this filter.");
        }
        match state.view {
            View::List => {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    list(ui, project, &visible, thumbs, palette, &mut resp, &mut remove, &mut relabel);
                });
            }
            View::Gallery => {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    gallery(ui, project, &visible, thumbs, palette, &mut resp, &mut remove);
                });
            }
            View::Slideshow => {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    slideshow(ui, state, project, &visible, thumbs, palette, &mut resp, &mut remove, &mut relabel);
                });
            }
        }
    });
    let _ = frame_r;
    state.content_rect = content_rect;

    if let Some(p) = payload {
        if let DragPayload::Asset(aid) = *p {
            if project.asset(aid).is_some() {
                undo(project);
                if moodboard_add(project, aid) {
                    resp.edited = true;
                }
            }
        }
    }
    if let Some(i) = remove {
        undo(project);
        project.moodboard.remove(i);
        resp.edited = true;
    }
    if let Some((i, labels)) = relabel {
        if project.moodboard.get(i).is_some_and(|m| m.labels != labels) {
            undo(project);
            if let Some(m) = project.moodboard.get_mut(i) {
                m.labels = labels;
            }
            resp.edited = true;
        }
    }
    resp
}

#[allow(clippy::too_many_arguments)]
fn list(
    ui: &mut egui::Ui,
    project: &Project,
    visible: &[usize],
    thumbs: &mut ThumbCache,
    palette: &Palette,
    resp: &mut MoodboardResponse,
    remove: &mut Option<usize>,
    relabel: &mut Option<(usize, Vec<String>)>,
) {
    for &i in visible {
        let asset_id = project.moodboard[i].asset;
        let mut tags_text = project.moodboard[i].labels.join(", ");
        let Some(a) = project.asset(asset_id) else { continue };
        let row = ui
            .horizontal(|ui| {
                asset_thumb(ui, project, thumbs, asset_id, THUMB, palette);
                ui.label(a.name());
                let r = ui.add(
                    TextEdit::singleline(&mut tags_text).desired_width(140.0).hint_text("tags, comma, separated"),
                );
                if r.changed() {
                    *relabel = Some((i, split_tags(&tags_text)));
                }
                let add_r = ui.small_button("Add at Playhead");
                mark(ui, format!("add_{asset_id}"), &add_r);
                if add_r.clicked() {
                    resp.add_to_timeline.push(asset_id);
                }
                let rm_r = x_button(ui);
                mark(ui, format!("rm_{asset_id}"), &rm_r);
                if rm_r.on_hover_text("Remove from moodboard").clicked() {
                    *remove = Some(i);
                }
            })
            .response;
        row.context_menu(|ui| {
            if ui.button("Add at Playhead").clicked() {
                resp.add_to_timeline.push(asset_id);
                ui.close();
            }
            if ui.button("Remove from moodboard").clicked() {
                *remove = Some(i);
                ui.close();
            }
        });
    }
}

fn gallery(
    ui: &mut egui::Ui,
    project: &Project,
    visible: &[usize],
    thumbs: &mut ThumbCache,
    palette: &Palette,
    resp: &mut MoodboardResponse,
    remove: &mut Option<usize>,
) {
    ui.horizontal_wrapped(|ui| {
        for &i in visible {
            let asset_id = project.moodboard[i].asset;
            let labels = project.moodboard[i].labels.join(", ");
            let Some(a) = project.asset(asset_id) else { continue };
            let tile = ui
                .group(|ui| {
                    ui.set_width(TILE);
                    ui.vertical(|ui| {
                        asset_thumb(ui, project, thumbs, asset_id, TILE - 12.0, palette);
                        ui.add(egui::Label::new(RichText::new(a.name()).small()).truncate());
                        if !labels.is_empty() {
                            ui.add(egui::Label::new(RichText::new(labels).weak().small()).truncate());
                        }
                        ui.horizontal(|ui| {
                            let add_r = ui.small_button("+").on_hover_text("Add at Playhead");
                            mark(ui, format!("add_{asset_id}"), &add_r);
                            if add_r.clicked() {
                                resp.add_to_timeline.push(asset_id);
                            }
                            let rm_r = x_button(ui);
                            mark(ui, format!("rm_{asset_id}"), &rm_r);
                            if rm_r.on_hover_text("Remove from moodboard").clicked() {
                                *remove = Some(i);
                            }
                        });
                    });
                })
                .response;
            tile.context_menu(|ui| {
                if ui.button("Add at Playhead").clicked() {
                    resp.add_to_timeline.push(asset_id);
                    ui.close();
                }
                if ui.button("Remove from moodboard").clicked() {
                    *remove = Some(i);
                    ui.close();
                }
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn slideshow(
    ui: &mut egui::Ui,
    state: &mut MoodboardState,
    project: &Project,
    visible: &[usize],
    thumbs: &mut ThumbCache,
    palette: &Palette,
    resp: &mut MoodboardResponse,
    remove: &mut Option<usize>,
    relabel: &mut Option<(usize, Vec<String>)>,
) {
    if visible.is_empty() {
        return;
    }
    state.slide_index = state.slide_index.min(visible.len() - 1);
    let i = visible[state.slide_index];
    let asset_id = project.moodboard[i].asset;
    let mut tags_text = project.moodboard[i].labels.join(", ");

    ui.horizontal(|ui| {
        let prev_r = icon_button(ui, palette, ui.id().with("mb_prev"), Glyph::Skip(Dir::Left), "Previous", false);
        mark(ui, "prev", &prev_r);
        if prev_r.clicked() {
            state.slide_index = (state.slide_index + visible.len() - 1) % visible.len();
        }
        ui.label(format!("{} / {}", state.slide_index + 1, visible.len()));
        let next_r = icon_button(ui, palette, ui.id().with("mb_next"), Glyph::Skip(Dir::Right), "Next", false);
        mark(ui, "next", &next_r);
        if next_r.clicked() {
            state.slide_index = (state.slide_index + 1) % visible.len();
        }
    });
    ui.vertical_centered(|ui| {
        asset_thumb(ui, project, thumbs, asset_id, 220.0, palette);
        if let Some(a) = project.asset(asset_id) {
            ui.label(a.name());
        }
    });
    let r = ui.add(TextEdit::singleline(&mut tags_text).desired_width(f32::INFINITY).hint_text("tags, comma, separated"));
    if r.changed() {
        *relabel = Some((i, split_tags(&tags_text)));
    }
    ui.horizontal(|ui| {
        let add_r = ui.button("Add at Playhead");
        mark(ui, "slide_add", &add_r);
        if add_r.clicked() {
            resp.add_to_timeline.push(asset_id);
        }
        if ui.button("Remove from moodboard").clicked() {
            *remove = Some(i);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo};

    fn asset(id: Id, kind: ClipKind) -> Asset {
        Asset {
            id,
            path: format!("C:/mood-{id}.mp4"),
            kind,
            duration: 4.0,
            width: 640,
            height: 360,
            fps: 30.0,
            audio_streams: if kind == ClipKind::Audio {
                vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }]
            } else {
                Vec::new()
            },
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    struct H {
        ctx: egui::Context,
        state: MoodboardState,
        project: Project,
        thumbs: ThumbCache,
        undos: usize,
    }

    impl H {
        fn new() -> Self {
            let ctx = egui::Context::default();
            Self {
                thumbs: ThumbCache::new(ctx.clone(), crate::media::Backend::Auto),
                ctx,
                state: MoodboardState::default(),
                project: Project::new(),
                undos: 0,
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> MoodboardResponse {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(520.0, 600.0))),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, egui::Color32::WHITE);
            let H { ctx, state, project, thumbs, undos } = self;
            let mut out = MoodboardResponse::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    out = show(ui, state, project, thumbs, &pal, &mut undo);
                });
            });
            out
        }
        fn rect(&self, name: &str) -> egui::Rect {
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("mb", name.to_string()))))
                .unwrap_or_else(|| panic!("no widget rect recorded for {name}"))
        }
        fn click(&mut self, pos: egui::Pos2) -> MoodboardResponse {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            let out = self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.frame(vec![]);
            out
        }
    }

    #[test]
    fn moodboard_add_dedupes() {
        let mut p = Project::new();
        let aid = p.add_asset(asset(1, ClipKind::Video));
        assert!(moodboard_add(&mut p, aid));
        assert_eq!(p.moodboard.len(), 1);
        assert!(!moodboard_add(&mut p, aid), "already on the board");
        assert_eq!(p.moodboard.len(), 1);
    }

    /// The internal-drag and OS-file-drop paths both end up here (see the module doc comment) — this is
    /// the actual "drop imports and adds" logic; App::handle_drops's own OS-file half is exercised by
    /// hand (a real drag from Explorer isn't something a headless test can simulate).
    #[test]
    fn drop_imports_and_adds() {
        let mut p = Project::new();
        let aid = p.add_asset(asset(1, ClipKind::Video));
        assert!(p.moodboard.is_empty());
        assert!(moodboard_add(&mut p, aid));
        assert_eq!(p.moodboard[0].asset, aid);
        assert!(p.moodboard[0].labels.is_empty());
    }

    #[test]
    fn filter_hides_non_matching_labels() {
        let a = MoodItem { asset: 1, labels: vec!["hero".into(), "night".into()] };
        let b = MoodItem { asset: 2, labels: vec!["b-roll".into()] };
        assert!(label_matches(&a, ""));
        assert!(label_matches(&a, "hero"));
        assert!(label_matches(&a, "NIGHT".to_lowercase().as_str()));
        assert!(!label_matches(&a, "broll"));
        assert!(label_matches(&b, "b-roll"));
    }

    #[test]
    fn split_tags_trims_and_drops_empties() {
        assert_eq!(split_tags("hero, night,  , b-roll"), vec!["hero", "night", "b-roll"]);
        assert_eq!(split_tags(""), Vec::<String>::new());
    }

    /// Clicking "Add at Playhead" in List view reports the asset id — the same `add_to_timeline`
    /// plumbing app.rs already wires up for Library and Planner.
    #[test]
    fn add_at_playhead_headless() {
        let mut h = H::new();
        let aid = h.project.add_asset(asset(1, ClipKind::Video));
        moodboard_add(&mut h.project, aid);
        h.state.view = View::List;
        h.frame(vec![]);
        let add = h.rect(&format!("add_{aid}"));
        let out = h.click(add.center());
        assert_eq!(out.add_to_timeline, vec![aid]);
    }

    /// Headless: every view (List, Gallery, Slideshow) lays out a mixed video/audio board without
    /// panicking, and reports no edits without interaction.
    #[test]
    fn show_headless_every_view() {
        let mut h = H::new();
        let video = h.project.add_asset(asset(1, ClipKind::Video));
        let audio = h.project.add_asset(asset(2, ClipKind::Audio));
        moodboard_add(&mut h.project, video);
        moodboard_add(&mut h.project, audio);
        for view in [View::List, View::Gallery, View::Slideshow] {
            h.state.view = view;
            let out = h.frame(vec![]);
            assert!(!out.edited && out.add_to_timeline.is_empty());
        }
    }
}
