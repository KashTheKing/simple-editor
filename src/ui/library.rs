//! Left panel: tabs "Library" and "Recent".
//! The search + filter strip is a TOOLBAR: a framed band in the panel fill with a separator under it,
//! a "Filters" caption, small chips and a search box with a magnifier prefix and a clear button — so it
//! reads as chrome, not as content. Asset rows carry a thumbnail (when the app passes its ThumbCache)
//! and are tinted with their colour label (a left bar + the name in that colour).
//! Library: search (name/tags/description/folder), filter chips (All/Video/Audio/Image/Sequences,
//! Short SFX ≤ 10 s, Music > 10 s, label colour, Unused only), sort (name/duration/kind/recent), a
//! collapsible folder tree (click filters, right-click new/rename/delete, drag an asset onto a folder),
//! asset rows (kind tag, label dot, tags, used dot; drag source `DragPayload::Asset`; double-click / "Add"
//! → timeline at the playhead; right-click → add / reveal / label / Convert To / remove), a details box for
//! the selected asset (path, format, description, tags, label, folder), "Remove unused", linked disk
//! folders (listed when expanded, cached until "Refresh"; drag `DragPayload::Path`), the sequences
//! (drag `DragPayload::Sequence`,
//! double-click opens) and saved templates (drag `DragPayload::Template`, place at playhead).
//! Recent: everything reusable, in sections — "Files" (settings.recent_assets across all projects, with
//! the same search/chips, pins, labels and tags) then "Effects", "Node graphs", "Adjustment layers" and
//! "Sequences", fed from the project (effect kinds and graphs in use, its sequences) and from the presets
//! store in settings.json. Both tabs honour `LibraryState.view`: 0 = `row()` list, 1 = `tile()` gallery.
//! Clicking a reusable tile applies it through `LibraryResponse` (add_effect / apply_preset / copy_graph /
//! place_template / open_sequence).

use crate::media::thumbs::ThumbCache;
use crate::model::{ClipKind, EffectKind, Id, Project};
use crate::settings::{RecentAsset, Settings};
use crate::theme::Palette;
use crate::ui::tools::{draw_glyph, Glyph};
use crate::ui::{duration_text, label_color, DragPayload};
use eframe::egui::load::SizedTexture;
use eframe::egui::{self, RichText};
use std::path::PathBuf;

#[derive(Default)]
pub struct LibraryState {
    /// 0 = Library, 1 = Recent
    pub tab: usize,
    pub selected: Option<Id>,
    pub search: String,
    /// 0 All, 1 Video, 2 Audio, 3 Image, 4 Sequences, 5 Short SFX, 6 Music
    pub kind_filter: u8,
    /// 0 = any, 1..=8 = LABEL_COLORS index + 1
    pub label_filter: u8,
    pub unused_only: bool,
    /// 0 name, 1 duration, 2 kind, 3 recently added
    pub sort: u8,
    /// 0 = list rows, 1 = thumbnail gallery. Applies to both tabs.
    pub view: u8,
    /// None = all folders, Some("") = root only, Some(name) = that folder (+ subfolders)
    pub folder: Option<String>,
    pub collapsed: Vec<String>,
    /// (folder being renamed, edit buffer = new last segment)
    pub rename_folder: Option<(String, String)>,
    /// (parent, edit buffer) — "" parent = top level
    pub new_folder: Option<(String, String)>,
    /// Tag edit buffer for the selected asset (avoids the comma being eaten while typing).
    pub tags_for: Option<Id>,
    pub tags_buf: String,
    pub rename_seq: Option<(Id, String)>,
    pub rename_template: Option<(usize, String)>,
    pub recent_tags_for: Option<String>,
    pub recent_tags_buf: String,
    /// Linked-folder listing cache (folder → media file paths, None = folder unreadable). Filled the
    /// first time a section is expanded, dropped on Refresh / unlink.
    pub linked: Vec<(String, Option<Vec<String>>)>,
}

#[derive(Default)]
pub struct LibraryResponse {
    pub import: bool,
    pub add_to_timeline: Vec<Id>,
    /// Recent files to import into the library (and select).
    pub open_paths: Vec<PathBuf>,
    pub remove: Vec<Id>,
    pub clear_recent: bool,
    /// The project changed (folders / tags / labels edited) — app pushes undo via `undo` first.
    pub edited: bool,
    /// settings.recent_assets changed (tags / labels / pins / removals) — app saves settings.
    pub settings_changed: bool,
    /// (asset id, target extension) — start a Convert To… with the default options.
    pub convert: Vec<(Id, String)>,
    /// Asset for which the app should open the Convert To… options dialog.
    pub convert_dialog: Option<Id>,
    /// Asset for which the app should open the Compress… dialog.
    pub compress: Option<Id>,
    /// Sequence to open for editing (double-clicked in the Sequences section).
    pub open_sequence: Option<Id>,
    /// Template names to place at the playhead.
    pub place_template: Vec<String>,
    /// The user asked to import media from a URL (the app opens the Import URL window).
    pub import_url: bool,
    /// "Edit labels…" was picked in a label menu — the app opens the label editor (Inspector).
    pub edit_labels: bool,
    /// "New ▸ Adjustment layer" — the app runs `Action::AddAdjustment`.
    pub new_adjustment: bool,
    /// "Open…" — the app runs `Action::OpenFile` (its own file dialog / recents handling).
    pub open_dialog: bool,
    /// An effect kind picked in the Recent tab — the app adds it to every selected visual clip.
    pub add_effect: Option<EffectKind>,
    /// Index into `Settings::effect_presets` to apply to the selection (chain or node graph).
    pub apply_preset: Option<usize>,
    /// Clip whose node graph should be copied onto the selection.
    pub copy_graph: Option<Id>,
}

/// (name, colour) of every `Project.labels` entry, snapshotted once per frame.
type Labels = Vec<(String, [u8; 3])>;

/// What a label menu returned.
enum LabelPick {
    Set(u8),
    Edit,
}

/// Project mutations collected during the frame, applied after all widgets (one undo per gesture).
enum LibOp {
    AssetLabel(Id, u8),
    AssetFolder(Id, String),
    AssetDesc(Id, String),
    AssetTags(Id, Vec<String>),
    FolderNew(String),
    FolderRename(String, String),
    FolderDelete(String),
    LinkFolder(String),
    UnlinkFolder(String),
    SeqNew,
    SeqRename(Id, String),
    SeqDelete(Id),
    RemoveUnused,
}

/// `thumbs`: the app's ThumbCache, so asset rows can show a picture. None (no cache / headless) simply
/// draws the rows without thumbnails.
/// `ytdlp` = yt-dlp was found at start-up; the "Import URL…" button only exists when it is installed.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &mut Project,
    settings: &mut Settings,
    thumbs: Option<&mut ThumbCache>,
    palette: &Palette,
    ytdlp: bool,
    undo: &mut dyn FnMut(&Project),
) -> LibraryResponse {
    let mut resp = LibraryResponse::default();
    let labels: Labels = project.labels.iter().map(|l| (l.name.clone(), l.color)).collect();
    let mut thumbs = thumbs;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, 0, "Library");
        ui.selectable_value(&mut state.tab, 1, "Recent");
    });
    ui.separator();
    if state.tab == 0 {
        library_tab(ui, state, project, settings, &mut thumbs, &labels, palette, ytdlp, &mut resp, undo);
    } else {
        recent_tab(ui, state, project, settings, &mut thumbs, &labels, palette, &mut resp);
    }
    resp
}

/// One asset thumbnail, `h` points tall, or nothing when the cache has none yet.
fn thumb(ui: &mut egui::Ui, thumbs: &mut Option<&mut ThumbCache>, path: &str, h: f32) {
    let Some(cache) = thumbs.as_deref_mut() else { return };
    let Some((tex, [w, th])) = cache.texture(ui.ctx(), path, 0.0, (h * 2.0) as u32) else { return };
    if th == 0 {
        return;
    }
    let size = egui::vec2(w as f32 * (h / th as f32), h);
    ui.add(egui::Image::new(SizedTexture::new(tex, size)));
}

// ---------- pure filter helpers ----------

fn kind_tag(k: ClipKind) -> &'static str {
    match k {
        ClipKind::Video => "V",
        ClipKind::Audio => "A",
        ClipKind::Image => "I",
        ClipKind::Text => "T",
        ClipKind::Sequence => "S",
        ClipKind::Shape => "Sh",
        ClipKind::Adjustment => "Adj",
    }
}

/// Case-insensitive match of the search text against name, tags, description and folder.
fn matches_search(a: &crate::model::Asset, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    a.name().to_lowercase().contains(&q)
        || a.folder.to_lowercase().contains(&q)
        || a.description.to_lowercase().contains(&q)
        || a.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

/// Kind chip: 0 All, 1 Video, 2 Audio, 3 Image, 4 Sequences (assets never match), 5 Short SFX (audio
/// ≤ 10 s), 6 Music (audio > 10 s).
fn matches_kind(kind: ClipKind, duration: f64, f: u8) -> bool {
    match f {
        1 => kind == ClipKind::Video,
        2 => kind == ClipKind::Audio,
        3 => kind == ClipKind::Image,
        4 => false,
        5 => kind == ClipKind::Audio && duration <= 10.0,
        6 => kind == ClipKind::Audio && duration > 10.0,
        _ => true,
    }
}

/// Folder filter: None = all, "" = root only, name = that folder and its subfolders.
fn folder_ok(sel: Option<&str>, folder: &str) -> bool {
    match sel {
        None => true,
        Some("") => folder.is_empty(),
        Some(s) => {
            folder == s || (folder.len() > s.len() && folder.starts_with(s) && folder.as_bytes()[s.len()] == b'/')
        }
    }
}

fn kind_rank(k: ClipKind) -> u8 {
    match k {
        ClipKind::Video => 0,
        ClipKind::Audio => 1,
        ClipKind::Image => 2,
        ClipKind::Text => 3,
        ClipKind::Sequence => 4,
        ClipKind::Shape => 5,
        ClipKind::Adjustment => 6,
    }
}

/// Rename a folder (and subfolders); assets follow.
fn rename_folder(project: &mut Project, old: &str, new: &str) {
    let swap = |s: &mut String| {
        if s == old {
            *s = new.to_string();
        } else if let Some(rest) = s.strip_prefix(&format!("{old}/")) {
            *s = format!("{new}/{rest}");
        }
    };
    for f in &mut project.folders {
        swap(f);
    }
    for a in &mut project.assets {
        swap(&mut a.folder);
    }
    project.folders.sort();
    project.folders.dedup();
}

/// Remove a sequence and every clip that uses it (main timeline, stash, other sequences).
fn delete_sequence(project: &mut Project, id: Id) {
    if project.editing == Some(id) {
        project.close_sequence();
    }
    project.sequences.retain(|s| s.id != id);
    let rm = |tracks: &mut Vec<crate::model::Track>| {
        for t in tracks {
            t.clips.retain(|c| !(c.kind == ClipKind::Sequence && c.sequence == id));
        }
    };
    rm(&mut project.tracks);
    if let Some(st) = &mut project.main_stash {
        rm(&mut st.tracks);
    }
    for s in &mut project.sequences {
        rm(&mut s.tracks);
    }
    project.tidy();
}

// ---------- recent helpers ----------

// ponytail: duplicated from app.rs's private MEDIA_EXTS split by kind — keep in sync by hand.
const VIDEO_EXTS: &[&str] =
    &["mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "ts", "m2ts", "mts", "flv", "3gp", "mpg", "mpeg", "gif"];
const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "wma"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"];

/// 1 video, 2 audio, 3 image, 0 unknown — by extension only.
fn ext_class(path: &str) -> u8 {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    if VIDEO_EXTS.contains(&ext.as_str()) {
        1
    } else if AUDIO_EXTS.contains(&ext.as_str()) {
        2
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        3
    } else {
        0
    }
}

/// Recents can only filter kind by extension (duration unknown): SFX/Music chips behave as Audio.
fn recent_matches(r: &RecentAsset, q: &str, kind: u8, label: u8) -> bool {
    if label != 0 && r.label != label {
        return false;
    }
    let class_ok = match kind {
        1 | 3 => ext_class(&r.path) == kind,
        2 | 5 | 6 => ext_class(&r.path) == 2,
        4 => false,
        _ => true,
    };
    if !class_ok {
        return false;
    }
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    r.path.to_lowercase().contains(&q) || r.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

fn recent_remove(settings: &mut Settings, resp: &mut LibraryResponse, path: &str) {
    settings.remove_recent(path);
    resp.settings_changed = true;
}

fn recent_clear(settings: &mut Settings, resp: &mut LibraryResponse) {
    settings.recent_assets.clear();
    resp.settings_changed = true;
}

// ---------- shared row widget ----------

/// Yes/No dialog for destructive, non-undoable actions.
pub(crate) fn confirm(title: &str, description: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// "Label ▸" submenu over the project's own labels; returns the pick ("Edit labels…" included).
fn label_menu(ui: &mut egui::Ui, current: u8, labels: &Labels) -> Option<LabelPick> {
    let mut picked = None;
    ui.menu_button("Label", |ui| {
        if ui.selectable_label(current == 0, "None").clicked() {
            picked = Some(LabelPick::Set(0));
            ui.close();
        }
        for (i, (name, [r, g, b])) in labels.iter().enumerate() {
            let t = RichText::new(format!("● {name}")).color(egui::Color32::from_rgb(*r, *g, *b));
            if ui.selectable_label(current == i as u8 + 1, t).clicked() {
                picked = Some(LabelPick::Set(i as u8 + 1));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Edit labels…").clicked() {
            picked = Some(LabelPick::Edit);
            ui.close();
        }
    });
    picked
}

/// Colour of a label index using the project's palette (falls back to the built-in colours).
fn lbl_color(labels: &Labels, idx: u8, palette: &Palette) -> egui::Color32 {
    match labels.get(idx.wrapping_sub(1) as usize) {
        Some((_, [r, g, b])) if idx > 0 => egui::Color32::from_rgb(*r, *g, *b),
        _ => label_color(idx, palette),
    }
}

/// Name of a label index in the project's palette.
fn lbl_name(labels: &Labels, idx: u8) -> &str {
    match labels.get(idx.wrapping_sub(1) as usize) {
        Some((n, _)) if idx > 0 => n.as_str(),
        _ => "None",
    }
}

/// One list row: a drag source with a click-sensing overlay (select / double-click / context menu) and
/// an optional action button on the right. Returns (row response, button clicked).
fn row(
    ui: &mut egui::Ui,
    id: egui::Id,
    payload: DragPayload,
    selected: bool,
    button: Option<&str>,
    contents: impl FnOnce(&mut egui::Ui),
) -> (egui::Response, bool) {
    ui.horizontal(|ui| {
        // Width of the trailing button (+ spacing) so the row content stops before it.
        let reserve = button.map_or(0.0, |b| {
            let font = egui::TextStyle::Button.resolve(ui.style());
            ui.painter().layout_no_wrap(b.to_owned(), font, egui::Color32::PLACEHOLDER).size().x
                + ui.spacing().button_padding.x * 2.0
                + ui.spacing().item_spacing.x
        });
        let content_right = ui.max_rect().right() - reserve;
        let src = ui.dnd_drag_source(id, payload, |ui| {
            // Hard-cap the row: an over-wide row widens the parent's max_rect, so every later row would
            // grow too, and long labels must truncate at the pane edge instead of pushing the button out.
            ui.set_max_width((content_right - ui.max_rect().left()).max(0.0));
            // Truncated labels still keep an ellipsis-wide minimum, so a row of them can overrun the cap
            // anyway: clip so the spill is never painted over the button.
            ui.set_clip_rect(ui.clip_rect().intersect(egui::Rect::everything_left_of(content_right)));
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let bg = ui.painter().add(egui::Shape::Noop);
            ui.horizontal(|ui| {
                contents(ui);
                // fill the row without `add_space`, which would add an item spacing past the right edge
                ui.expand_to_include_x(ui.max_rect().right());
            });
            let rect = ui.min_rect();
            let v = ui.visuals();
            let fill = if selected {
                v.selection.bg_fill.gamma_multiply(0.35)
            } else if ui.rect_contains_pointer(rect) {
                v.widgets.hovered.weak_bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().set(bg, egui::Shape::rect_filled(rect, 2.0, fill));
        });
        // Registered after the drag-only widget so clicks reach it (egui gives the topmost click-sensing widget
        // the click and the drag-sensing one the drag).
        let r = ui.interact(src.response.rect, id.with("click"), egui::Sense::click());
        // Put the button in the reserved slot rather than after the contents: the contents can be wider
        // than their cap, and appending would then push the button off the pane edge.
        let clicked = button.is_some_and(|b| {
            let gap = ui.spacing().item_spacing.x;
            let slot = egui::Rect::from_min_size(
                egui::pos2(content_right + gap, src.response.rect.top()),
                egui::vec2(reserve - gap, src.response.rect.height()),
            );
            let r = ui.put(slot, egui::Button::new(b).small());
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("row_btn_right"), r.rect.right()));
            r.clicked()
        });
        (r, clicked)
    })
    .inner
}

/// TextEdit for inline renames; Some(text) once editing finishes with a non-empty name.
pub(crate) fn inline_edit(ui: &mut egui::Ui, buf: &mut String) -> Option<String> {
    let r = ui.add(egui::TextEdit::singleline(buf).desired_width(120.0));
    // Read the flag before touching focus: request_focus() makes has_focus() true, and lost_focus() is
    // `had_focus_last_frame && !has_focus` — asking for focus first would make it permanently false.
    if r.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return Some(String::new()); // cancel
        }
        return Some(buf.trim().to_string()); // empty = cancel, caller decides
    }
    if !r.has_focus() {
        r.request_focus();
    }
    None
}

// ---------- library tab ----------

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn library_tab(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &mut Project,
    settings: &mut Settings,
    thumbs: &mut Option<&mut ThumbCache>,
    labels: &Labels,
    palette: &Palette,
    ytdlp: bool,
    resp: &mut LibraryResponse,
    undo: &mut dyn FnMut(&Project),
) {
    let mut ops: Vec<LibOp> = Vec::new();
    let mut op_start = false; // true when this frame starts an undo-worthy gesture

    let mut import = false;
    let mut new_folder = false;
    let mut resp_open = false;
    let mut sort = state.sort;
    let mut import_url = false;
    let mut new_seq = false;
    let mut new_adj = false;
    toolbar(ui, state, labels, palette, |ui, state| {
        if ui.button("Open…").clicked() {
            resp_open = true;
        }
        ui.menu_button("New ▾", |ui| {
            if ui.button("Import files…").clicked() {
                import = true;
                ui.close();
            }
            if ui.button("Sequence").clicked() {
                new_seq = true;
                ui.close();
            }
            if ui.button("Adjustment layer").clicked() {
                new_adj = true;
                ui.close();
            }
            if ui.button("Folder…").clicked() {
                new_folder = true;
                ui.close();
            }
        });
        if ui.button("Import…").clicked() {
            import = true;
        }
        // only when yt-dlp is installed — the whole URL import is optional
        if ytdlp && ui.button("Import URL…").on_hover_text("Download media from a link with yt-dlp").clicked() {
            import_url = true;
        }
        egui::ComboBox::from_id_salt("lib_sort")
            .selected_text(["Name", "Duration", "Kind", "Recent"][state.sort.min(3) as usize])
            .width(80.0)
            .show_ui(ui, |ui| {
                for (i, n) in ["Name", "Duration", "Kind", "Recent"].iter().enumerate() {
                    ui.selectable_value(&mut sort, i as u8, *n);
                }
            });
    });
    state.sort = sort;
    resp.open_dialog |= resp_open;
    resp.new_adjustment |= new_adj;
    if new_seq {
        ops.push(LibOp::SeqNew);
        op_start = true;
    }
    resp.import |= import;
    resp.import_url |= import_url;
    if new_folder {
        state.new_folder = Some((String::new(), String::new()));
    }

    // ponytail: both walk every clip / plan item per frame. Upgrade: cache them behind a generation
    // counter bumped by App::after_edit() if a big project ever shows it.
    let used = project.used_assets();
    let planned = project.plan_assets();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        folder_tree(ui, state, project, &mut ops, &mut op_start);

        // filtered + sorted asset rows
        let mut order: Vec<usize> = (0..project.assets.len())
            .filter(|&i| {
                let a = &project.assets[i];
                folder_ok(state.folder.as_deref(), &a.folder)
                    && matches_search(a, &state.search)
                    && matches_kind(a.kind, a.duration, state.kind_filter)
                    && (state.label_filter == 0 || a.label == state.label_filter)
                    // same predicate as "Remove unused" below: moodboard assets are not unused
                    && (!state.unused_only || !(used.contains(&a.id) || planned.contains(&a.id)))
            })
            .collect();
        match state.sort {
            1 => order.sort_by(|&x, &y| project.assets[x].duration.total_cmp(&project.assets[y].duration)),
            2 => order.sort_by_key(|&i| kind_rank(project.assets[i].kind)),
            3 => order.sort_by_key(|&i| std::cmp::Reverse(project.assets[i].id)),
            _ => order.sort_by_cached_key(|&i| project.assets[i].name().to_lowercase()),
        }
        if state.view == 1 {
            ui.horizontal_wrapped(|ui| {
                for i in order {
                    asset_tile(ui, state, project, i, thumbs, labels, palette, resp);
                }
            });
        } else {
            for i in order {
                asset_row(ui, state, project, i, &used, thumbs, labels, palette, resp, &mut ops, &mut op_start);
            }
        }

        details_box(ui, state, project, labels, palette, resp, &mut ops, &mut op_start);

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let unused = project.assets.iter().filter(|a| !used.contains(&a.id) && !planned.contains(&a.id)).count();
            if ui.add_enabled(unused > 0, egui::Button::new(format!("Remove unused ({unused})"))).clicked()
                && confirm("Remove unused", &format!("Remove {unused} unused assets from the project?"))
            {
                ops.push(LibOp::RemoveUnused);
                op_start = true;
            }
            if ui.button("Link folder…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    ops.push(LibOp::LinkFolder(p.to_string_lossy().into_owned()));
                    op_start = true;
                }
            }
        });
        linked_sections(ui, state, project, resp, &mut ops, &mut op_start);
        sequences_section(ui, state, project, resp, &mut ops, &mut op_start);
        templates_section(ui, state, settings, resp);
    });

    // apply project mutations with a single undo per gesture (text fields snapshot on focus, one frame
    // before the first keystroke, so the snapshot is taken even when no op lands this frame)
    if op_start {
        undo(project);
    }
    if !ops.is_empty() {
        for op in ops {
            match op {
                LibOp::AssetLabel(id, l) => {
                    if let Some(a) = project.asset_mut(id) {
                        a.label = l;
                    }
                }
                LibOp::AssetFolder(id, f) => {
                    if let Some(a) = project.asset_mut(id) {
                        a.folder = f;
                    }
                }
                LibOp::AssetDesc(id, d) => {
                    if let Some(a) = project.asset_mut(id) {
                        a.description = d;
                    }
                }
                LibOp::AssetTags(id, t) => {
                    if let Some(a) = project.asset_mut(id) {
                        a.tags = t;
                    }
                }
                LibOp::FolderNew(name) => {
                    project.add_folder(&name);
                }
                LibOp::FolderRename(old, new) => rename_folder(project, &old, &new),
                LibOp::FolderDelete(name) => {
                    project.remove_folder(&name);
                    if state
                        .folder
                        .as_deref()
                        .is_some_and(|f| !folder_ok(Some(""), f) && !project.folder_names().iter().any(|n| n == f))
                    {
                        state.folder = None;
                    }
                }
                LibOp::LinkFolder(p) => {
                    if !project.linked_folders.contains(&p) {
                        project.linked_folders.push(p);
                    }
                }
                LibOp::UnlinkFolder(p) => {
                    project.linked_folders.retain(|f| *f != p);
                    state.linked.retain(|(f, _)| *f != p);
                }
                LibOp::SeqNew => {
                    let n = project.sequences.len() + 1;
                    let (w, h, fps) = (project.width, project.height, project.fps);
                    project.new_sequence(format!("Sequence {n}"), w, h, fps);
                }
                LibOp::SeqRename(id, name) => {
                    if let Some(s) = project.sequence_mut(id) {
                        s.name = name;
                    }
                }
                LibOp::SeqDelete(id) => delete_sequence(project, id),
                LibOp::RemoveUnused => {
                    project.remove_unused_assets();
                }
            }
        }
        resp.edited = true;
    }
}

/// The search + filter toolbar: a framed band in the panel fill with a separator under it, so it never
/// reads as content. `head` draws the tab-specific leading row (Import / Clear / sort).
fn toolbar(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    labels: &Labels,
    palette: &Palette,
    head: impl FnOnce(&mut egui::Ui, &mut LibraryState),
) {
    egui::Frame::new().fill(palette.panel).inner_margin(egui::Margin::symmetric(4, 3)).show(ui, |ui| {
        // wrapped: a narrow pane must stack the buttons, not push them past the right edge
        // wrapped: a narrow pane must stack the buttons, not push them past the right edge
        ui.horizontal_wrapped(|ui| head(ui, state));
        ui.horizontal(|ui| {
            ui.label("🔍");
            let clear_w = 22.0;
            // TextEdit::desired_width is the *text* width; its frame adds a margin on top, so the row
            // used to be ~8px wider than the pane and dragged every later widget out with it.
            let margin = ui.spacing().button_padding.x * 2.0;
            let w = (ui.available_width() - clear_w - ui.spacing().item_spacing.x - margin).max(24.0);
            let r = ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("search").desired_width(w));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("lib_search_rect"), r.rect));
            let _ = r;
            if ui
                .add_enabled(!state.search.is_empty(), egui::Button::new("✕").small())
                .on_hover_text("Clear search")
                .clicked()
            {
                state.search.clear();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.weak(RichText::new("View").small());
            for (i, n) in ["List", "Gallery"].iter().enumerate() {
                if ui.selectable_label(state.view == i as u8, RichText::new(*n).small()).clicked() {
                    state.view = i as u8;
                }
            }
            ui.separator();
            ui.weak(RichText::new("Filters").small());
            for (i, n) in ["All", "Video", "Audio", "Image", "Seq", "Short SFX", "Music"].iter().enumerate() {
                if ui.selectable_label(state.kind_filter == i as u8, RichText::new(*n).small()).clicked() {
                    state.kind_filter = i as u8;
                }
            }
            for i in 1..=labels.len() as u8 {
                let dot = RichText::new("●").color(lbl_color(labels, i, palette));
                if ui.selectable_label(state.label_filter == i, dot).on_hover_text(lbl_name(labels, i)).clicked() {
                    state.label_filter = if state.label_filter == i { 0 } else { i };
                }
            }
            ui.toggle_value(&mut state.unused_only, RichText::new("Unused").small());
        });
    });
    ui.separator();
}

fn folder_tree(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    let names = project.folder_names();
    // file-explorer root: everything in the project hangs off "Library"
    ui.label(RichText::new("Library").strong());
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        if ui.selectable_label(state.folder.is_none(), "Imported").clicked() {
            state.folder = None;
        }
        if ui.selectable_label(false, "Recent").clicked() {
            state.tab = 1;
        }
        let r = ui.selectable_label(state.folder.as_deref() == Some(""), "▤ Root");
        if r.clicked() {
            state.folder = Some(String::new());
        }
        if let Some(p) = r.dnd_release_payload::<DragPayload>() {
            if let DragPayload::Asset(aid) = *p {
                ops.push(LibOp::AssetFolder(aid, String::new()));
                *op_start = true;
            }
        }
    });
    let mut skip: Option<String> = None;
    for name in &names {
        if let Some(s) = &skip {
            if name.starts_with(s.as_str()) {
                continue;
            }
            skip = None;
        }
        let depth = name.matches('/').count();
        let last = name.rsplit('/').next().unwrap_or(name);
        let has_children =
            names.iter().any(|n| n.len() > name.len() && n.starts_with(name) && n.as_bytes()[name.len()] == b'/');
        let closed = state.collapsed.contains(name);
        let mut rename_done: Option<String> = None;
        ui.horizontal(|ui| {
            ui.add_space(8.0 + depth as f32 * 12.0);
            if has_children {
                if ui.small_button(if closed { "▸" } else { "▾" }).clicked() {
                    if closed {
                        state.collapsed.retain(|c| c != name);
                    } else {
                        state.collapsed.push(name.clone());
                    }
                }
            } else {
                ui.add_space(18.0);
            }
            if state.rename_folder.as_ref().is_some_and(|(o, _)| o == name) {
                let (_, buf) = state.rename_folder.as_mut().unwrap();
                rename_done = inline_edit(ui, buf);
            } else {
                let r = ui.selectable_label(state.folder.as_deref() == Some(name.as_str()), format!("▤ {last}"));
                if r.clicked() {
                    state.folder = Some(name.clone());
                }
                r.context_menu(|ui| {
                    if ui.button("New folder").clicked() {
                        state.new_folder = Some((name.clone(), String::new()));
                        ui.close();
                    }
                    if ui.button("Rename").clicked() {
                        state.rename_folder = Some((name.clone(), last.to_string()));
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        ops.push(LibOp::FolderDelete(name.clone()));
                        *op_start = true;
                        ui.close();
                    }
                });
                if let Some(p) = r.dnd_release_payload::<DragPayload>() {
                    if let DragPayload::Asset(aid) = *p {
                        ops.push(LibOp::AssetFolder(aid, name.clone()));
                        *op_start = true;
                    }
                }
            }
        });
        if let Some(new_last) = rename_done {
            if !new_last.is_empty() && new_last != *last {
                let parent = name.rfind('/').map(|i| &name[..i]);
                let new = match parent {
                    Some(p) => format!("{p}/{new_last}"),
                    None => new_last,
                };
                ops.push(LibOp::FolderRename(name.clone(), new));
                *op_start = true;
            }
            state.rename_folder = None;
        }
        if closed {
            skip = Some(format!("{name}/"));
        }
    }
    if let Some((parent, buf)) = &mut state.new_folder {
        let parent = parent.clone();
        let mut done = None;
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            ui.label("▤");
            done = inline_edit(ui, buf);
        });
        if let Some(name) = done {
            if !name.is_empty() {
                let full = if parent.is_empty() { name } else { format!("{parent}/{name}") };
                ops.push(LibOp::FolderNew(full));
                *op_start = true;
            }
            state.new_folder = None;
        }
    }
}

/// Duration cell of an asset — "Loading…" while its import probe is still out (see engine::import).
fn dur_cell(a: &crate::model::Asset) -> String {
    if crate::engine::import::is_probing(&a.path) {
        "Loading…".to_string()
    } else {
        duration_text(a.duration)
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn asset_row(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    i: usize,
    used: &std::collections::HashSet<Id>,
    thumbs: &mut Option<&mut ThumbCache>,
    labels: &Labels,
    palette: &Palette,
    resp: &mut LibraryResponse,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    let a = &project.assets[i];
    let selected = state.selected == Some(a.id);
    let tint = (a.label != 0).then(|| lbl_color(labels, a.label, palette));
    let (r, add) =
        row(ui, egui::Id::new(("asset", a.id)), DragPayload::Asset(a.id), selected, selected.then_some("Add"), |ui| {
            // label tint: a colour bar on the left and the name in the same colour
            if let Some(c) = tint {
                let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, 1.0, c);
            }
            if a.has_video() {
                thumb(ui, thumbs, &a.path, 18.0);
            }
            let mut name = RichText::new(a.name());
            if let Some(c) = tint {
                name = name.color(c);
            }
            ui.label(if selected { name.strong() } else { name });
            ui.weak(kind_tag(a.kind));
            ui.weak(dur_cell(a));
            if used.contains(&a.id) {
                ui.label(RichText::new("•").color(palette.accent)).on_hover_text("Used in the timeline");
            }
            if !a.tags.is_empty() {
                ui.add(egui::Label::new(RichText::new(a.tags.join(" · ")).weak().small()).truncate());
            }
        });
    if r.clicked() {
        state.selected = Some(a.id);
    }
    if add || r.double_clicked() {
        state.selected = Some(a.id);
        resp.add_to_timeline.push(a.id);
    }
    r.context_menu(|ui| {
        if ui.button("Add to timeline at playhead").clicked() {
            resp.add_to_timeline.push(a.id);
            ui.close();
        }
        if ui.button("Reveal folder").clicked() {
            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", a.path)).spawn();
            ui.close();
        }
        match label_menu(ui, a.label, labels) {
            Some(LabelPick::Set(l)) => {
                ops.push(LibOp::AssetLabel(a.id, l));
                *op_start = true;
            }
            Some(LabelPick::Edit) => resp.edit_labels = true,
            None => {}
        }
        ui.menu_button("Convert To", |ui| {
            for t in crate::engine::convert::TARGETS {
                if ui.button(*t).clicked() {
                    resp.convert.push((a.id, (*t).to_string()));
                    ui.close();
                }
            }
        });
        if ui.button("Convert To… (options)").clicked() {
            resp.convert_dialog = Some(a.id);
            ui.close();
        }
        if ui.button("Compress…").clicked() {
            resp.compress = Some(a.id);
            ui.close();
        }
        if ui.button("Remove from project").clicked() {
            resp.remove.push(a.id);
            ui.close();
        }
    });
}

/// What fills a gallery tile's picture box.
enum Art {
    /// A cached picture (asset filmstrip, effect catalogue card) fitted into the box.
    Image(egui::TextureId, [u32; 2]),
    /// Nothing to photograph (effects, graphs, adjustment layers, sequences): a painted icon.
    Icon(Glyph),
    None,
}

/// Gallery cell: a picture box with the kind tag in its corner and the name under it. Shared by the
/// asset gallery and the Recent tab's sections; the caller decides what the click means.
#[allow(clippy::too_many_arguments)]
fn tile(
    ui: &mut egui::Ui,
    id: egui::Id,
    payload: DragPayload,
    selected: bool,
    tag: &str,
    name: &str,
    tint: egui::Color32,
    palette: &Palette,
    art: Art,
) -> egui::Response {
    let src = ui.dnd_drag_source(id, payload, |ui| {
        ui.set_max_width(TILE);
        ui.vertical(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(TILE, TILE * 0.56), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, palette.panel);
            match art {
                Art::Image(tex, [w, h]) if h > 0 => {
                    let k = (rect.width() / w as f32).min(rect.height() / h as f32);
                    let size = egui::vec2(w as f32 * k, h as f32 * k);
                    ui.painter().image(
                        tex,
                        egui::Rect::from_center_size(rect.center(), size),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                Art::Icon(g) => draw_glyph(ui.painter(), rect, g, palette.text_dim),
                _ => {}
            }
            ui.painter().text(
                rect.left_bottom() + egui::vec2(3.0, -3.0),
                egui::Align2::LEFT_BOTTOM,
                tag,
                egui::TextStyle::Small.resolve(ui.style()),
                palette.text_dim,
            );
            let name = RichText::new(name).color(tint);
            ui.add(egui::Label::new(if selected { name.strong() } else { name }).truncate());
        });
    });
    let r = ui.interact(src.response.rect, id.with("click"), egui::Sense::click());
    if selected {
        ui.painter().rect_stroke(
            src.response.rect,
            2.0,
            egui::Stroke::new(1.0, palette.selection),
            egui::StrokeKind::Inside,
        );
    }
    r
}

/// The cached thumbnail of a media file at the gallery size, if the cache already has one.
fn tile_art(ui: &egui::Ui, thumbs: &mut Option<&mut ThumbCache>, path: &str) -> Art {
    match thumbs.as_deref_mut().and_then(|c| c.texture(ui.ctx(), path, 0.0, TILE as u32)) {
        Some((tex, size)) => Art::Image(tex, size),
        None => Art::None,
    }
}

/// Gallery cell: a thumbnail with the name under it. Same click/drag behaviour as `asset_row`,
/// without the context menu (the list view keeps that).
fn asset_tile(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    i: usize,
    thumbs: &mut Option<&mut ThumbCache>,
    labels: &Labels,
    palette: &Palette,
    resp: &mut LibraryResponse,
) {
    let a = &project.assets[i];
    let selected = state.selected == Some(a.id);
    let tint = (a.label != 0).then(|| lbl_color(labels, a.label, palette)).unwrap_or(palette.text);
    let art = if a.has_video() { tile_art(ui, thumbs, &a.path) } else { Art::None };
    let id = egui::Id::new(("tile", a.id));
    let r = tile(ui, id, DragPayload::Asset(a.id), selected, kind_tag(a.kind), &a.name(), tint, palette, art);
    if r.clicked() {
        state.selected = Some(a.id);
    }
    if r.double_clicked() {
        state.selected = Some(a.id);
        resp.add_to_timeline.push(a.id);
    }
    r.on_hover_text(&a.path);
}

/// Gallery cell width (points). Thumbnails are 16:9 inside it.
const TILE: f32 = 108.0;

/// Details of the selected asset: path, format, description, tags, label, folder.
#[allow(clippy::too_many_arguments)]
fn details_box(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    labels: &Labels,
    palette: &Palette,
    resp: &mut LibraryResponse,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    let Some(a) = state.selected.and_then(|id| project.asset(id)) else { return };
    if state.tags_for != Some(a.id) {
        state.tags_for = Some(a.id);
        state.tags_buf = a.tags.join(", ");
    }
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(&a.path).weak()).truncate()).on_hover_text(&a.path);
        let mut line = format!("{} · {}", kind_tag(a.kind), dur_cell(a));
        if a.width > 0 {
            line.push_str(&format!(" · {}×{}", a.width, a.height));
        }
        if a.kind == ClipKind::Video && a.fps > 0.0 {
            line.push_str(&format!(" · {:.3} fps", a.fps));
        }
        if !a.audio_streams.is_empty() {
            line.push_str(&format!(" · {} audio", a.audio_streams.len()));
        }
        ui.weak(line);
        let mut desc = a.description.clone();
        let r = ui.add(
            egui::TextEdit::multiline(&mut desc).desired_rows(2).desired_width(f32::INFINITY).hint_text("description"),
        );
        if r.changed() {
            ops.push(LibOp::AssetDesc(a.id, desc));
        }
        // Snapshot when the field is entered, not per keystroke: one undo entry per visit instead of one
        // per character (which used to evict the whole 200-entry stack while typing a paragraph).
        *op_start |= r.gained_focus();
        let r = ui.add(
            egui::TextEdit::singleline(&mut state.tags_buf)
                .desired_width(f32::INFINITY)
                .hint_text("tags, comma, separated"),
        );
        if r.changed() {
            let tags = state.tags_buf.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
            ops.push(LibOp::AssetTags(a.id, tags));
        }
        *op_start |= r.gained_focus();
        ui.horizontal(|ui| {
            let name = lbl_name(labels, a.label);
            egui::ComboBox::from_id_salt("asset_label")
                .selected_text(RichText::new(format!("● {name}")).color(lbl_color(labels, a.label, palette)))
                .show_ui(ui, |ui| {
                    let mut pick = |l: u8, ui: &mut egui::Ui| {
                        ops.push(LibOp::AssetLabel(a.id, l));
                        *op_start = true;
                        ui.close();
                    };
                    if ui.selectable_label(a.label == 0, "None").clicked() {
                        pick(0, ui);
                    }
                    for (i, (n, [r, g, b])) in labels.iter().enumerate() {
                        let t = RichText::new(format!("● {n}")).color(egui::Color32::from_rgb(*r, *g, *b));
                        if ui.selectable_label(a.label == i as u8 + 1, t).clicked() {
                            pick(i as u8 + 1, ui);
                        }
                    }
                    ui.separator();
                    if ui.button("Edit labels…").clicked() {
                        resp.edit_labels = true;
                        ui.close();
                    }
                });
            let folder_text = if a.folder.is_empty() { "Root" } else { a.folder.as_str() };
            egui::ComboBox::from_id_salt("asset_folder").selected_text(folder_text).show_ui(ui, |ui| {
                if ui.selectable_label(a.folder.is_empty(), "Root").clicked() {
                    ops.push(LibOp::AssetFolder(a.id, String::new()));
                    *op_start = true;
                    ui.close();
                }
                for f in project.folder_names() {
                    if ui.selectable_label(a.folder == f, &f).clicked() {
                        ops.push(LibOp::AssetFolder(a.id, f.clone()));
                        *op_start = true;
                        ui.close();
                    }
                }
            });
        });
    });
}

/// List media files of a folder on disk (non-recursive, known extensions only). None = unreadable.
fn scan_one(folder: &str) -> Option<Vec<String>> {
    let mut files: Vec<String> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_lowercase();
            (VIDEO_EXTS.contains(&ext.as_str())
                || AUDIO_EXTS.contains(&ext.as_str())
                || IMAGE_EXTS.contains(&ext.as_str()))
            .then(|| p.to_string_lossy().into_owned())
        })
        .collect();
    files.sort();
    Some(files)
}

fn linked_sections(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    resp: &mut LibraryResponse,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    for (i, folder) in project.linked_folders.iter().enumerate() {
        let title = folder.rsplit(['\\', '/']).next().unwrap_or(folder);
        egui::CollapsingHeader::new(format!("⤷ {title}")).id_salt(("linked", i)).show(ui, |ui| {
            // ponytail: blocking read_dir, cached until Refresh and only for expanded sections — a dead
            // network share used to freeze the whole editor every 5 s. Upgrade: scan on a thread + mpsc
            // (media/waveform.rs) if the first expand of a slow share is still too long.
            let k = match state.linked.iter().position(|(f, _)| f == folder) {
                Some(k) => k,
                None => {
                    state.linked.push((folder.clone(), scan_one(folder)));
                    state.linked.len() - 1
                }
            };
            let mut refresh = false;
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(folder).weak().small()).truncate()).on_hover_text(folder);
                refresh = ui.small_button("Refresh").clicked();
                if ui.small_button("✕").on_hover_text("Unlink folder").clicked() {
                    ops.push(LibOp::UnlinkFolder(folder.clone()));
                    *op_start = true;
                }
            });
            match &state.linked[k].1 {
                None => {
                    ui.weak("(folder unavailable)");
                }
                Some(files) if files.is_empty() => {
                    ui.weak("(no media files)");
                }
                Some(files) => {
                    for (j, path) in files.iter().enumerate() {
                        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
                        let id = egui::Id::new(("linked_file", i, j));
                        let (r, _) = row(ui, id, DragPayload::Path(path.clone()), false, None, |ui| {
                            ui.label(name);
                            ui.weak(kind_tag_for_class(ext_class(path)));
                        });
                        if r.double_clicked() {
                            resp.open_paths.push(PathBuf::from(path));
                        }
                        r.on_hover_text(path);
                    }
                }
            }
            if refresh {
                state.linked.remove(k);
            }
        });
    }
}

fn kind_tag_for_class(c: u8) -> &'static str {
    match c {
        1 => "V",
        2 => "A",
        3 => "I",
        _ => "",
    }
}

fn sequences_section(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    resp: &mut LibraryResponse,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.strong("Sequences");
        if ui.small_button("New sequence…").clicked() {
            ops.push(LibOp::SeqNew);
            *op_start = true;
        }
    });
    for s in &project.sequences {
        if state.rename_seq.as_ref().is_some_and(|(id, _)| *id == s.id) {
            let (_, buf) = state.rename_seq.as_mut().unwrap();
            let mut done = None;
            ui.horizontal(|ui| {
                ui.label("⧉");
                done = inline_edit(ui, buf);
            });
            if let Some(name) = done {
                if !name.is_empty() {
                    ops.push(LibOp::SeqRename(s.id, name));
                    *op_start = true;
                }
                state.rename_seq = None;
            }
            continue;
        }
        let (r, _) = row(ui, egui::Id::new(("seq", s.id)), DragPayload::Sequence(s.id), false, None, |ui| {
            ui.label(format!("⧉ {}", s.name));
            ui.weak(duration_text(project.sequence_duration(s.id)));
        });
        if r.double_clicked() {
            resp.open_sequence = Some(s.id);
        }
        r.context_menu(|ui| {
            if ui.button("Open").clicked() {
                resp.open_sequence = Some(s.id);
                ui.close();
            }
            if ui.button("Rename").clicked() {
                state.rename_seq = Some((s.id, s.name.clone()));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                ops.push(LibOp::SeqDelete(s.id));
                *op_start = true;
                ui.close();
            }
        });
    }
    if project.sequences.is_empty() {
        ui.weak("(none — nest clips or \"New sequence…\")");
    }
}

fn templates_section(ui: &mut egui::Ui, state: &mut LibraryState, settings: &mut Settings, resp: &mut LibraryResponse) {
    if settings.templates.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.strong("Templates");
    let mut rename: Option<(usize, String)> = None;
    let mut delete: Option<usize> = None;
    for i in 0..settings.templates.len() {
        if state.rename_template.as_ref().is_some_and(|(j, _)| *j == i) {
            let (_, buf) = state.rename_template.as_mut().unwrap();
            let mut done = None;
            ui.horizontal(|ui| {
                ui.label("▣");
                done = inline_edit(ui, buf);
            });
            if let Some(name) = done {
                if !name.is_empty() {
                    rename = Some((i, name));
                }
                state.rename_template = None;
            }
            continue;
        }
        let name = settings.templates[i].name.clone();
        let (r, place) =
            row(ui, egui::Id::new(("template", i)), DragPayload::Template(name.clone()), false, Some("Place"), |ui| {
                ui.label(format!("▣ {name}"));
            });
        if place || r.double_clicked() {
            resp.place_template.push(name.clone());
        }
        r.context_menu(|ui| {
            if ui.button("Place at playhead").clicked() {
                resp.place_template.push(name.clone());
                ui.close();
            }
            if ui.button("Rename").clicked() {
                state.rename_template = Some((i, name.clone()));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                delete = Some(i);
                ui.close();
            }
        });
    }
    if let Some((i, name)) = rename {
        settings.templates[i].name = name;
        resp.settings_changed = true;
    }
    if let Some(i) = delete {
        // saved templates live in settings.json — no undo, so ask first
        if confirm("Delete template", &format!("Delete the saved template \"{}\"?", settings.templates[i].name)) {
            settings.templates.remove(i);
            resp.settings_changed = true;
        }
    }
}

// ---------- recent tab ----------

enum RecOp {
    Pin(String),
    Label(String, u8),
    Remove(String),
    Clear,
}

const CLEAR_RECENT: &str = "Clear the whole recent list? Pins, labels and tags are lost (no undo).";

/// One reusable thing the Recent tab lists beside the files.
enum Reuse {
    /// An effect kind already used somewhere in the project.
    Effect(EffectKind),
    /// Index into `Settings::effect_presets` — a saved chain or a saved node graph.
    Preset(usize),
    /// A clip that renders from a node graph.
    Graph(Id),
    /// Index into `Settings::templates` holding a saved adjustment layer.
    Adjustment(usize),
    Sequence(Id),
}

/// The reusable sections, read straight off the project and settings.json each frame.
/// ponytail: linear scans + a JSON decode per template (`is_adjustment_template`), same as the Presets
/// pane — the lists are short. A dedicated presets store plugs in here as extra `Reuse` items.
fn reuse_sections(project: &Project, settings: &Settings) -> Vec<(&'static str, Vec<Reuse>)> {
    let mut kinds: Vec<EffectKind> = Vec::new();
    let mut graphs: Vec<Reuse> = Vec::new();
    for (_, c) in project.all_clips() {
        for e in &c.effects {
            if !kinds.contains(&e.kind) {
                kinds.push(e.kind);
            }
        }
        if c.graph.is_some() {
            graphs.push(Reuse::Graph(c.id));
        }
    }
    let mut fx: Vec<Reuse> = kinds.into_iter().map(Reuse::Effect).collect();
    for (i, p) in settings.effect_presets.iter().enumerate() {
        if p.is_graph() {
            graphs.push(Reuse::Preset(i));
        } else {
            fx.push(Reuse::Preset(i));
        }
    }
    let adj = settings
        .templates
        .iter()
        .enumerate()
        .filter(|(_, t)| crate::engine::presets::is_adjustment_template(t))
        .map(|(i, _)| Reuse::Adjustment(i))
        .collect();
    let seqs = project.sequences.iter().map(|s| Reuse::Sequence(s.id)).collect();
    vec![("Effects", fx), ("Node graphs", graphs), ("Adjustment layers", adj), ("Sequences", seqs)]
}

/// (name, kind tag, icon, drag payload) of one reusable item.
/// ponytail: only sequences and adjustment layers have a drop target today — the others carry their name
/// as a `Template` payload because `row()`/`tile()` are drag sources, and a stray drop just misses.
fn reuse_face(item: &Reuse, project: &Project, settings: &Settings) -> (String, &'static str, Glyph, DragPayload) {
    let by_name = |name: String, tag, icon| (name.clone(), tag, icon, DragPayload::Template(name));
    match *item {
        Reuse::Effect(k) => by_name(k.name().to_string(), "FX", Glyph::Star),
        Reuse::Preset(i) => match settings.effect_presets.get(i) {
            Some(p) if p.is_graph() => by_name(p.name.clone(), "Graph", Glyph::Nodes),
            Some(p) => by_name(p.name.clone(), "FX", Glyph::Star),
            None => by_name(String::new(), "FX", Glyph::Star),
        },
        Reuse::Graph(id) => {
            let name = project.clip(id).map(|c| c.name.clone()).unwrap_or_default();
            by_name(name, "Graph", Glyph::Nodes)
        }
        Reuse::Adjustment(i) => {
            by_name(settings.templates.get(i).map(|t| t.name.clone()).unwrap_or_default(), "Adj", Glyph::Layers)
        }
        Reuse::Sequence(id) => {
            let name = project.sequences.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
            (name, "Seq", Glyph::FilmStrip, DragPayload::Sequence(id))
        }
    }
}

/// What a click on a reusable item asks the app to do.
fn reuse_pick(item: &Reuse, name: &str, resp: &mut LibraryResponse) {
    match *item {
        Reuse::Effect(k) => resp.add_effect = Some(k),
        Reuse::Preset(i) => resp.apply_preset = Some(i),
        Reuse::Graph(id) => resp.copy_graph = Some(id),
        Reuse::Adjustment(_) => resp.place_template.push(name.to_string()),
        Reuse::Sequence(id) => resp.open_sequence = Some(id),
    }
}

/// The reusable sections under the recent files, in whichever view the toolbar selected.
fn reuse_ui(
    ui: &mut egui::Ui,
    view: u8,
    project: &Project,
    settings: &Settings,
    palette: &Palette,
    resp: &mut LibraryResponse,
) {
    for (title, items) in reuse_sections(project, settings) {
        section_head(ui, title);
        if items.is_empty() {
            ui.weak("(none)");
            continue;
        }
        let mut draw = |ui: &mut egui::Ui, i: usize, item: &Reuse| {
            let (name, tag, icon, payload) = reuse_face(item, project, settings);
            let id = egui::Id::new((title, i));
            let r = if view == 1 {
                let art = match *item {
                    Reuse::Effect(k) => match crate::ui::effects_ui::thumbnail(k) {
                        Some((tex, size)) => Art::Image(tex, size),
                        None => Art::Icon(icon),
                    },
                    _ => Art::Icon(icon),
                };
                tile(ui, id, payload, false, tag, &name, palette.text, palette, art)
            } else {
                row(ui, id, payload, false, None, |ui| {
                    let (box_, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
                    draw_glyph(ui.painter(), box_, icon, palette.text_dim);
                    ui.label(&name);
                    ui.weak(tag);
                })
                .0
            };
            if r.clicked() {
                reuse_pick(item, &name, resp);
            }
        };
        if view == 1 {
            ui.horizontal_wrapped(|ui| {
                for (i, item) in items.iter().enumerate() {
                    draw(ui, i, item);
                }
            });
        } else {
            for (i, item) in items.iter().enumerate() {
                draw(ui, i, item);
            }
        }
    }
}

fn section_head(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.strong(title);
}

#[allow(clippy::too_many_arguments)]
fn recent_tab(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    settings: &mut Settings,
    thumbs: &mut Option<&mut ThumbCache>,
    labels: &Labels,
    palette: &Palette,
    resp: &mut LibraryResponse,
) {
    let mut clear = false;
    let (mut open_dialog, mut import) = (false, false);
    toolbar(ui, state, labels, palette, |ui, _state| {
        open_dialog = ui.button("Open…").clicked();
        import = ui.button("Import…").clicked();
        if ui.button("Clear").clicked() && confirm("Clear recent", CLEAR_RECENT) {
            clear = true;
        }
    });
    resp.open_dialog |= open_dialog;
    resp.import |= import;
    if clear {
        recent_clear(settings, resp);
        resp.clear_recent = true;
    }
    if let Some(path) = state.recent_tags_for.clone() {
        ui.horizontal(|ui| {
            ui.weak("Tags:");
            let r = ui.add(egui::TextEdit::singleline(&mut state.recent_tags_buf).desired_width(160.0));
            if r.changed() {
                if let Some(rec) = settings.recent_assets.iter_mut().find(|r| r.path == path) {
                    rec.tags = state
                        .recent_tags_buf
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                    resp.settings_changed = true;
                }
            }
            if ui.small_button("Done").clicked() {
                state.recent_tags_for = None;
            }
        });
    }
    let mut op: Option<RecOp> = None;
    let view = state.view;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        section_head(ui, "Files");
        // ponytail: no existence check here (would stat 200 files per frame) — the open path reports errors.
        let hits: Vec<usize> = settings
            .recent_assets
            .iter()
            .enumerate()
            .filter(|(_, r)| recent_matches(r, &state.search, state.kind_filter, state.label_filter))
            .map(|(i, _)| i)
            .collect();
        let mut file = |ui: &mut egui::Ui, i: usize| {
            let rec = &settings.recent_assets[i];
            let (name, dir) = split_path(&rec.path);
            let tint = (rec.label != 0).then(|| lbl_color(labels, rec.label, palette));
            let id = egui::Id::new(("recent", i));
            let payload = DragPayload::Path(rec.path.clone());
            let media = matches!(ext_class(&rec.path), 1 | 3);
            let (r, open) = if view == 1 {
                let art = if media { tile_art(ui, thumbs, &rec.path) } else { Art::None };
                let tag = kind_tag_for_class(ext_class(&rec.path));
                (tile(ui, id, payload, false, tag, name, tint.unwrap_or(palette.text), palette, art), false)
            } else {
                row(ui, id, payload, false, Some("Open"), |ui| {
                    if rec.pinned {
                        ui.weak("★");
                    }
                    if let Some(c) = tint {
                        let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(bar, 1.0, c);
                    }
                    // videos and stills get the same filmstrip thumbnail the library rows use
                    if media {
                        thumb(ui, thumbs, &rec.path, 18.0);
                    }
                    match tint {
                        Some(c) => ui.label(RichText::new(name).color(c)),
                        None => ui.label(name),
                    };
                    ui.add(egui::Label::new(RichText::new(dir).weak()).truncate());
                    if !rec.tags.is_empty() {
                        ui.add(egui::Label::new(RichText::new(rec.tags.join(" · ")).weak().small()).truncate());
                    }
                })
            };
            if open || r.double_clicked() {
                resp.open_paths.push(PathBuf::from(&rec.path));
            }
            r.context_menu(|ui| {
                // one entry only: "Add to library" ran the same open_or_import, and `||` skipped drawing
                // it on the frame "Open" was clicked
                if ui.button("Open").clicked() {
                    resp.open_paths.push(PathBuf::from(&rec.path));
                    ui.close();
                }
                if ui.button(if rec.pinned { "Unpin" } else { "Pin" }).clicked() {
                    op = Some(RecOp::Pin(rec.path.clone()));
                    ui.close();
                }
                match label_menu(ui, rec.label, labels) {
                    Some(LabelPick::Set(l)) => op = Some(RecOp::Label(rec.path.clone(), l)),
                    Some(LabelPick::Edit) => resp.edit_labels = true,
                    None => {}
                }
                if ui.button("Tags…").clicked() {
                    state.recent_tags_for = Some(rec.path.clone());
                    state.recent_tags_buf = rec.tags.join(", ");
                    ui.close();
                }
                if ui.button("Remove from recent").clicked() {
                    op = Some(RecOp::Remove(rec.path.clone()));
                    ui.close();
                }
                if ui.button("Clear history").clicked() {
                    op = Some(RecOp::Clear);
                    ui.close();
                }
            });
            r.on_hover_text(&rec.path);
        };
        if view == 1 {
            ui.horizontal_wrapped(|ui| {
                for i in hits {
                    file(ui, i);
                }
            });
        } else {
            for i in hits {
                file(ui, i);
            }
        }
        reuse_ui(ui, view, project, settings, palette, resp);
    });
    match op {
        Some(RecOp::Pin(path)) => {
            if let Some(rec) = settings.recent_assets.iter_mut().find(|r| r.path == path) {
                rec.pinned = !rec.pinned;
            }
            settings.sort_recent();
            resp.settings_changed = true;
        }
        Some(RecOp::Label(path, l)) => {
            if let Some(rec) = settings.recent_assets.iter_mut().find(|r| r.path == path) {
                rec.label = l;
            }
            resp.settings_changed = true;
        }
        Some(RecOp::Remove(path)) => recent_remove(settings, resp, &path),
        Some(RecOp::Clear) => {
            if confirm("Clear recent", CLEAR_RECENT) {
                recent_clear(settings, resp);
            }
        }
        None => {}
    }
}

/// ("file.mp4", "C:\dir") — pure string split on the last path separator.
fn split_path(p: &str) -> (&str, &str) {
    match p.rfind(['\\', '/']) {
        Some(i) => (&p[i + 1..], &p[..i]),
        None => (p, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Asset;

    fn asset(id: Id, kind: ClipKind, duration: f64) -> Asset {
        Asset {
            id,
            path: format!(r"C:\media\file{id}.mp4"),
            kind,
            duration,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    #[test]
    fn split_path_cases() {
        assert_eq!(split_path(r"C:\Videos\clip.mp4"), ("clip.mp4", r"C:\Videos"));
        assert_eq!(split_path("/home/u/a.mov"), ("a.mov", "/home/u"));
        assert_eq!(split_path("a.mp4"), ("a.mp4", ""));
        assert_eq!(split_path(r"C:\x\"), ("", r"C:\x"));
    }

    #[test]
    fn kind_tags() {
        assert_eq!(kind_tag(ClipKind::Video), "V");
        assert_eq!(kind_tag(ClipKind::Audio), "A");
        assert_eq!(kind_tag(ClipKind::Image), "I");
    }

    #[test]
    fn search_matches_name_tags_description_folder() {
        let mut a = asset(1, ClipKind::Video, 5.0);
        a.tags = vec!["Drone".into()];
        a.description = "Sunset flyover".into();
        a.folder = "Footage/Day 1".into();
        assert!(matches_search(&a, ""));
        assert!(matches_search(&a, "FILE1"));
        assert!(matches_search(&a, "drone"));
        assert!(matches_search(&a, "sunset"));
        assert!(matches_search(&a, "day 1"));
        assert!(!matches_search(&a, "nope"));
    }

    #[test]
    fn kind_chips_incl_sfx_music() {
        assert!(matches_kind(ClipKind::Video, 5.0, 0));
        assert!(matches_kind(ClipKind::Video, 5.0, 1));
        assert!(!matches_kind(ClipKind::Video, 5.0, 2));
        // Short SFX: audio ≤ 10 s
        assert!(matches_kind(ClipKind::Audio, 3.0, 5));
        assert!(!matches_kind(ClipKind::Audio, 30.0, 5));
        assert!(!matches_kind(ClipKind::Video, 3.0, 5));
        // Music: audio > 10 s
        assert!(matches_kind(ClipKind::Audio, 30.0, 6));
        assert!(!matches_kind(ClipKind::Audio, 3.0, 6));
        // Sequences chip never matches assets
        assert!(!matches_kind(ClipKind::Video, 5.0, 4));
    }

    #[test]
    fn folder_filter_subtree() {
        assert!(folder_ok(None, "anything"));
        assert!(folder_ok(Some(""), ""));
        assert!(!folder_ok(Some(""), "F"));
        assert!(folder_ok(Some("F"), "F"));
        assert!(folder_ok(Some("F"), "F/Sub"));
        assert!(!folder_ok(Some("F"), "Fx"));
    }

    #[test]
    fn rename_folder_moves_assets() {
        let mut p = Project::new();
        p.add_folder("A/B");
        let id = p.add_asset(asset(0, ClipKind::Video, 5.0));
        p.asset_mut(id).unwrap().folder = "A/B".into();
        rename_folder(&mut p, "A", "Z");
        assert_eq!(p.asset(id).unwrap().folder, "Z/B");
        assert!(p.folder_names().iter().any(|f| f == "Z/B"));
    }

    #[test]
    fn recent_ext_class_and_filters() {
        assert_eq!(ext_class("a.mp4"), 1);
        assert_eq!(ext_class("a.WAV"), 2);
        assert_eq!(ext_class("a.png"), 3);
        assert_eq!(ext_class("a.xyz"), 0);
        let rec = RecentAsset { path: r"C:\m\clip.mp3".into(), tags: vec!["voice".into()], ..Default::default() };
        assert!(recent_matches(&rec, "", 0, 0));
        assert!(recent_matches(&rec, "voice", 2, 0));
        assert!(recent_matches(&rec, "", 5, 0)); // SFX chip = audio by extension
        assert!(!recent_matches(&rec, "", 1, 0));
        assert!(!recent_matches(&rec, "", 0, 3)); // label filter
    }

    #[test]
    fn recent_remove_and_clear_flag_settings() {
        let mut s = Settings::default();
        s.touch_recent("a.mp4");
        s.touch_recent("b.mp4");
        let mut resp = LibraryResponse::default();
        recent_remove(&mut s, &mut resp, "a.mp4");
        assert!(resp.settings_changed);
        assert_eq!(s.recent_assets.len(), 1);
        let mut resp = LibraryResponse::default();
        recent_clear(&mut s, &mut resp);
        assert!(resp.settings_changed);
        assert!(s.recent_assets.is_empty());
    }

    #[test]
    fn delete_sequence_removes_clips() {
        let mut p = Project::new();
        let seq = p.new_sequence("S", 1280, 720, 30.0);
        p.insert_sequence_clip(seq, 0.0, None);
        assert!(p.all_clips().any(|(_, c)| c.kind == ClipKind::Sequence));
        delete_sequence(&mut p, seq);
        assert!(p.sequences.is_empty());
        assert!(!p.all_clips().any(|(_, c)| c.kind == ClipKind::Sequence));
    }

    #[test]
    fn scan_one_reports_unreadable_folders() {
        assert!(scan_one(r"C:\does\not\exist\at\all").is_none());
        assert!(scan_one(&std::env::temp_dir().to_string_lossy()).is_some());
    }

    /// A row that does not fit used to widen the parent Ui, so every row below it grew and its action
    /// button ended up outside the pane.
    #[test]
    fn rows_keep_a_constant_width() {
        let ctx = egui::Context::default();
        let mut widths = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let pane = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(300.0, 4000.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(pane), |ui| {
                    for i in 0..30 {
                        let id = egui::Id::new(("t", i));
                        let (r, _) = row(ui, id, DragPayload::Template(String::new()), false, Some("Place"), |ui| {
                            ui.label("▣ a template name far too long to ever fit into this narrow pane");
                        });
                        widths.push(r.rect.width());
                    }
                });
            });
        });
        let first = widths[0];
        assert!(first <= 300.0, "row wider than the pane: {first}");
        assert!(widths.iter().all(|w| (w - first).abs() < 1.0), "rows grew down the list: {widths:?}");
    }

    /// The inline rename/create field commits when it loses focus (it used to re-grab focus first, so
    /// `lost_focus()` could never fire and nothing was ever committed).
    #[test]
    fn inline_edit_commits_on_focus_loss() {
        let ctx = egui::Context::default();
        let mut buf = "Footage".to_string();
        let mut out = None;
        // focus is moved inside the pass: done between passes it counts as "had focus last frame"
        let mut run = |steal: bool, buf: &mut String, out: &mut Option<String>| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                if steal {
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("somewhere else")));
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    *out = inline_edit(ui, buf);
                });
            });
        };
        run(false, &mut buf, &mut out);
        assert!(out.is_none());
        run(false, &mut buf, &mut out);
        assert!(out.is_none(), "still editing while focused");
        run(true, &mut buf, &mut out);
        assert_eq!(out.as_deref(), Some("Footage"), "commits when focus moves away");
    }

    /// The filter/search strip renders as a toolbar: the search box sits inside it, a clear button
    /// empties the query, and the label chips come from the project's own labels.
    #[test]
    fn filter_toolbar_renders_and_clears() {
        let mut project = Project::new();
        let mut a = asset(0, ClipKind::Video, 5.0);
        a.path = r"C:\media\one.mp4".into();
        project.add_asset(a);
        project.labels.truncate(2);
        let mut settings = Settings::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut state = LibraryState { search: "one".into(), ..Default::default() };
        let mut search_rect = egui::Rect::NOTHING;
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    show(ui, &mut state, &mut project, &mut settings, None, &palette, true, &mut undo);
                });
            });
            search_rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("lib_search_rect")))
                .expect("the toolbar drew its search box");
        }
        assert!(search_rect.width() > 20.0, "search box has room: {search_rect:?}");
        assert!(search_rect.top() < 120.0, "the toolbar sits at the top of the panel: {search_rect:?}");
        // the label chips only offer the labels the project actually has
        let labels: Labels = project.labels.iter().map(|l| (l.name.clone(), l.color)).collect();
        assert_eq!(labels.len(), 2);
        assert_eq!(lbl_name(&labels, 1), project.labels[0].name);
        assert_eq!(lbl_name(&labels, 0), "None");
        assert_eq!(lbl_name(&labels, 9), "None", "out of range falls back");
        // clearing the search empties the field
        state.search.clear();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut undo = |_: &Project| {};
                show(ui, &mut state, &mut project, &mut settings, None, &palette, true, &mut undo);
            });
        });
        assert!(state.search.is_empty());
    }

    /// Rows are tinted with the project's label colour, and a recoloured label changes the tint.
    #[test]
    fn rows_use_the_project_label_colour() {
        let mut project = Project::new();
        let palette = Palette::new(true, egui::Color32::WHITE);
        project.labels[0].color = [1, 2, 3];
        let labels: Labels = project.labels.iter().map(|l| (l.name.clone(), l.color)).collect();
        assert_eq!(lbl_color(&labels, 1, &palette), egui::Color32::from_rgb(1, 2, 3));
        // 0 / unknown fall back to the dim "no label" colour
        assert_eq!(lbl_color(&labels, 0, &palette), label_color(0, &palette));
        assert_eq!(lbl_color(&labels, 250, &palette), label_color(250, &palette));
    }

    /// Headless with a thumbnail cache attached: rows still lay out (no decode is required).
    #[test]
    fn rows_lay_out_with_a_thumb_cache() {
        let mut project = Project::new();
        let mut a = asset(0, ClipKind::Video, 5.0);
        a.path = r"C:\media\one.mp4".into();
        project.add_asset(a);
        let mut settings = Settings::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut cache = ThumbCache::new(ctx.clone(), crate::media::Backend::Ffmpeg);
        let mut state = LibraryState::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| panic!("no undo without edits");
                    let r =
                        show(ui, &mut state, &mut project, &mut settings, Some(&mut cache), &palette, true, &mut undo);
                    assert!(!r.edited && !r.edit_labels);
                });
            });
        }
    }

    /// Headless: both tabs lay out with folders, sequences, templates and a selected asset,
    /// reporting nothing when nothing was clicked.
    #[test]
    /// The "Import URL…" button exists only when yt-dlp was found, and clicking it asks the app to
    /// open the Import URL window.
    #[test]
    fn import_url_button_is_gated_on_ytdlp() {
        let run = |ytdlp: bool, click: bool| -> (bool, bool) {
            let mut project = Project::new();
            let mut settings = Settings::default();
            let palette = Palette::new(true, egui::Color32::WHITE);
            let ctx = egui::Context::default();
            let mut state = LibraryState::default();
            let mut seen = false;
            let mut asked = false;
            // frame 1 lays the button out, frame 2 can click it at the position it landed on
            let mut at = egui::Pos2::ZERO;
            for frame in 0..2 {
                let mut input = egui::RawInput::default();
                if click && frame == 1 && at != egui::Pos2::ZERO {
                    input.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                    input.events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                }
                let out = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| {};
                        let r = show(ui, &mut state, &mut project, &mut settings, None, &palette, ytdlp, &mut undo);
                        asked |= r.import_url;
                    });
                });
                // find the button by its label among the frame's shapes
                if let Some(rect) = text_rect(&out.shapes, "Import URL…") {
                    seen = true;
                    at = rect.center();
                }
            }
            (seen, asked)
        };
        assert_eq!(run(false, false), (false, false), "button must be hidden without yt-dlp");
        let (seen, asked) = run(true, true);
        assert!(seen, "button must be shown when yt-dlp is installed");
        assert!(asked, "clicking it must ask the app to open the Import URL window");
    }

    /// The trailing action button must stay inside the pane at any width (it used to be pushed off the
    /// right edge in the recent tab once the path/tag labels were long).
    #[test]
    fn recent_open_button_stays_inside_the_pane() {
        let mut settings = Settings::default();
        settings.touch_recent(
            r"C:ery\long\directory
ame	hat\keeps\going\clip-with-a-long-name.mp4",
        );
        settings.recent_assets[0].tags = vec!["one".into(), "two".into(), "three".into()];
        let labels = Labels::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for w in [180.0_f32, 260.0, 420.0] {
            let mut state = LibraryState { tab: 1, ..Default::default() };
            let (mut right, mut pane_right) = (f32::NAN, f32::NAN);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let pane = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(w, 600.0));
                    pane_right = pane.right();
                    ui.scope_builder(egui::UiBuilder::new().max_rect(pane), |ui| {
                        let mut resp = LibraryResponse::default();
                        let project = Project::new();
                        recent_tab(ui, &mut state, &project, &mut settings, &mut None, &labels, &palette, &mut resp);
                        right = ui.ctx().data(|d| d.get_temp(egui::Id::new("row_btn_right")).unwrap_or(f32::NAN));
                    });
                });
            });
            assert!(right <= pane_right + 1.0, "the Open button overflows a {w}px pane: right={right}");
        }
    }

    /// Rect of the painted text `label` in a frame's shapes (None when it was never drawn).
    fn text_rect(shapes: &[egui::epaint::ClippedShape], label: &str) -> Option<egui::Rect> {
        shapes.iter().find_map(|c| match &c.shape {
            egui::epaint::Shape::Text(t) if t.galley.text().contains(label) => Some(t.visual_bounding_rect()),
            _ => None,
        })
    }

    fn show_headless() {
        let mut project = Project::new();
        for (i, kind) in [ClipKind::Video, ClipKind::Audio, ClipKind::Image].iter().enumerate() {
            let mut a = asset(0, *kind, 12.5 * i as f64);
            a.path = format!(r"C:\media\file{i}.mp4");
            a.tags = vec!["tag".into()];
            a.label = (i % 3) as u8;
            project.add_asset(a);
        }
        project.add_folder("Footage/Day 1");
        project.new_sequence("Intro", 1280, 720, 30.0);
        project.linked_folders.push(r"C:\does\not\exist".into());
        let mut settings = Settings::default();
        settings.touch_recent(r"C:\media\old.mp4");
        settings.touch_recent(r"D:\clips\new.mov");
        settings.recent_assets[0].pinned = true;
        settings.recent_assets[0].label = 2;
        settings.templates.push(crate::settings::Template { name: "Intro pack".into(), json: "{}".into() });
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for tab in [0, 1] {
            let mut state = LibraryState { tab, selected: Some(project.assets[1].id), ..Default::default() };
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| panic!("no undo without edits");
                        let r = show(ui, &mut state, &mut project, &mut settings, None, &palette, true, &mut undo);
                        assert!(!r.import && !r.clear_recent && !r.edited && !r.settings_changed);
                        assert!(r.add_to_timeline.is_empty() && r.open_paths.is_empty() && r.remove.is_empty());
                        assert!(r.convert.is_empty() && r.open_sequence.is_none() && r.place_template.is_empty());
                    });
                });
            }
            assert_eq!(state.tab, tab);
            // the linked section is collapsed: nothing was read from disk
            assert!(state.linked.is_empty(), "collapsed linked folders must not be scanned");
        }
    }

    /// Recent is a gallery of everything reusable: the files plus effects, node graphs, saved adjustment
    /// layers and sequences. Both views lay out, every section is titled, and a click hands the right
    /// intent back to the app.
    #[test]
    fn recent_tab_lists_everything_reusable() {
        let mut project = Project::new();
        project.tracks[0].clips.push(crate::model::Clip::new(7, ClipKind::Video, "v", 0.0, 4.0));
        project.tracks[0].clips[0].effects.push(crate::model::Effect::new(EffectKind::Blur));
        let adj = project.add_adjustment_clip(0.0, 2.0);
        project.ensure_graph(adj);
        let seq = project.new_sequence("Intro", 1280, 720, 30.0);
        let mut settings = Settings::default();
        settings.touch_recent(r"C:\media\old.mp4");
        settings.effect_presets.push(crate::settings::EffectPreset { name: "Look".into(), json: "[]".into() });
        settings
            .effect_presets
            .push(crate::settings::EffectPreset { name: "Graph".into(), json: "{\"nodes\":[]}".into() });
        settings.templates.push(crate::engine::presets::capture_template("Grade", &project, &[adj]));

        // project + settings feed the same four sections
        let counts: Vec<(&str, usize)> =
            reuse_sections(&project, &settings).iter().map(|(t, v)| (*t, v.len())).collect();
        assert_eq!(counts, vec![("Effects", 2), ("Node graphs", 2), ("Adjustment layers", 1), ("Sequences", 1)]);

        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for view in [0, 1] {
            let mut state = LibraryState { tab: 1, view, ..Default::default() };
            let mut titles = Vec::new();
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 1200.0))),
                    ..Default::default()
                };
                let out = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| panic!("no undo without edits");
                        let r = show(ui, &mut state, &mut project, &mut settings, None, &palette, false, &mut undo);
                        assert!(r.add_effect.is_none() && r.apply_preset.is_none() && r.copy_graph.is_none());
                    });
                });
                titles = ["Files", "Effects", "Node graphs", "Adjustment layers", "Sequences"]
                    .iter()
                    .filter(|t| text_rect(&out.shapes, t).is_some())
                    .copied()
                    .collect();
            }
            assert_eq!(titles.len(), 5, "view {view} is missing sections: {titles:?}");
        }

        // clicking each kind of item
        let mut resp = LibraryResponse::default();
        reuse_pick(&Reuse::Effect(EffectKind::Blur), "Blur", &mut resp);
        reuse_pick(&Reuse::Preset(1), "Graph", &mut resp);
        reuse_pick(&Reuse::Graph(adj), "", &mut resp);
        reuse_pick(&Reuse::Adjustment(0), "Grade", &mut resp);
        reuse_pick(&Reuse::Sequence(seq), "Intro", &mut resp);
        assert_eq!(resp.add_effect, Some(EffectKind::Blur));
        assert_eq!(resp.apply_preset, Some(1));
        assert_eq!(resp.copy_graph, Some(adj));
        assert_eq!(resp.place_template, vec!["Grade".to_string()]);
        assert_eq!(resp.open_sequence, Some(seq));
        assert_eq!(reuse_face(&Reuse::Effect(EffectKind::Blur), &project, &settings).0, "Blur");
    }
}
