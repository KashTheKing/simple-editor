//! Left panel: the project browser — Premiere's project panel, in two sub-tabs.
//!
//! "Imported" is what the project contains, drawn as a real file explorer: nested folders, the assets
//! under them, a "+ New" button (folder / import / sequence / adjustment layer), rename and delete from
//! the context menu, and move-by-drag onto a folder row. "Global" browses the disk instead: the recent
//! files from settings.json plus the folders the user linked, one directory level read per expansion
//! (never an eager recursive walk — that hangs the UI on a media drive) and only inside a node the user
//! linked or opened.
//!
//! Selection is multi: a click replaces it, Ctrl+click toggles one item, Shift+click takes the range
//! over the rows in the order they were drawn, and a drag across empty space rubber-bands.
//! `LibraryState.selected` / `sel_path` are the anchor inside the set — app.rs writes `selected` after
//! an import, and `show` notices that write and collapses the set onto it. With something selected the
//! toolbar grows a batch strip (Import, Convert To, Convert…, Compress…, Remove) that routes through the
//! same `LibraryResponse` fields the per-item menus use, so App needs no new plumbing.
//!
//! The bottom of the pane is a dedicated asset preview: a big picture of the anchor, its format line,
//! and the ONLY place its description, tags, label and folder are edited — never inline in the list.
//! Every file row and tile carries a picture: the cached thumbnail, or a painted glyph while there is
//! none — a speaker for audio, a film strip for video, a camera for stills, a sheet stack for the rest.
//! The toolbar works over both tabs: search (name/tags/description/folder), kind chips (All/Video/Audio/
//! Image/Seq, Short SFX ≤ 10 s, Music > 10 s), label colours, "Unused only", sort, the List/Gallery
//! switch (`LibraryState.view`) and the zoom (`LibraryState.zoom`, the +/- buttons and Ctrl+Scroll) that
//! scales both gallery tiles and row thumbnails. A search or a filter flattens "Imported" into its hits,
//! the way an explorer shows search results.
//! Under the tree, the Imported tab also lists what the project can reuse: its sequences (drag
//! `DragPayload::Sequence`, double-click opens), saved templates, and the effects / node graphs /
//! adjustment layers already in use or saved in settings.json.

use crate::media::thumbs::ThumbCache;
use crate::model::{ClipKind, EffectKind, Id, Project};
use crate::settings::{RecentAsset, Settings};
use crate::theme::Palette;
use crate::ui::tools::{draw_glyph, glyph_text_button, icon_button, Glyph};
use crate::ui::{duration_text, label_color, DragPayload};
use eframe::egui::{self, RichText};
use std::path::PathBuf;

#[derive(Default)]
pub struct LibraryState {
    /// 0 = Imported (what the project contains), 1 = Global (recent files + the linked folders).
    pub tab: usize,
    /// Anchor of the selection: the asset a click landed on, and the one the preview box shows.
    /// app.rs writes it straight after an import — `show` spots that and collapses the set onto it.
    pub selected: Option<Id>,
    /// Same, for a selected file that is not a project asset.
    pub sel_path: Option<String>,
    /// The whole selection (Ctrl / Shift click, rubber band); the anchor above is one of these.
    pub sel_ids: Vec<Id>,
    pub sel_paths: Vec<String>,
    /// `selected` as of the end of the last frame — the only way to tell app.rs's write from ours.
    pub seen_selected: Option<Id>,
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
    /// Thumbnail scale, `ZOOM_MIN..=ZOOM_MAX`. 0 = never set, read as 1.
    pub zoom: f32,
    /// The folder clicked in the tree — highlighted, and the subtree a search is limited to.
    /// None = all folders, Some("") = root only, Some(name) = that folder (+ subfolders)
    pub folder: Option<String>,
    /// Tree nodes whose open state differs from the default (project folders start open, Recent and
    /// folders on disk start closed). Keys: "recent", "f:<folder>", `dir_key(path)`.
    pub flipped: Vec<String>,
    /// (folder being renamed, edit buffer = new last segment)
    pub rename_folder: Option<(String, String)>,
    /// (parent, edit buffer) — "" parent = top level
    pub new_folder: Option<(String, String)>,
    /// Tag edit buffer for the previewed asset (avoids the comma being eaten while typing).
    pub tags_for: Option<Id>,
    pub tags_buf: String,
    pub rename_seq: Option<(Id, String)>,
    pub rename_template: Option<(usize, String)>,
    /// Same, for the previewed file on disk (its tags live on its `RecentAsset`).
    pub recent_tags_for: Option<String>,
    pub recent_tags_buf: String,
    /// One-level directory listings, keyed by folder path: (entry path, is a folder), folders first,
    /// None = unreadable. Only ever filled for a node the user expanded; dropped on Refresh / unlink.
    pub dirs: Vec<(String, Option<Vec<(String, bool)>>)>,
}

#[derive(Default)]
pub struct LibraryResponse {
    pub import: bool,
    pub add_to_timeline: Vec<Id>,
    /// Files to import into the library (and select).
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
    /// An effect kind picked in the reuse sections — the app adds it to every selected visual clip.
    pub add_effect: Option<EffectKind>,
    /// Index into `Settings::effect_presets` to apply to the selection (chain or node graph).
    pub apply_preset: Option<usize>,
    /// Clip whose node graph should be copied onto the selection.
    pub copy_graph: Option<Id>,
    /// The file the anchor of the selection now points at — the app may show it in the source viewer.
    /// The library previews it itself either way.
    pub preview: Option<PathBuf>,
}

/// (name, colour) of every `Project.labels` entry, snapshotted once per frame.
type Labels = Vec<(String, [u8; 3])>;

/// What a label menu returned.
enum LabelPick {
    Set(u8),
    Edit,
}

/// One selectable thing in the browser: a project asset, or a file on disk.
#[derive(Clone, PartialEq, Debug)]
enum Pick {
    Asset(Id),
    Path(String),
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

/// `thumbs`: the app's ThumbCache, so rows and the preview can show a picture. None (no cache /
/// headless) draws the painted fallback instead.
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
    state.zoom = if state.zoom > 0.0 { state.zoom.clamp(ZOOM_MIN, ZOOM_MAX) } else { 1.0 };
    external_select(state);
    state.sel_ids.retain(|id| project.asset(*id).is_some());
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, 0, "Imported");
        ui.selectable_value(&mut state.tab, 1, "Global");
    });
    ui.separator();
    browser(ui, state, project, settings, &mut thumbs, &labels, palette, ytdlp, &mut resp, undo);
    state.seen_selected = state.selected;
    resp
}

/// app.rs sets `selected` on its own after an import or a "reveal in library"; the multi-selection
/// follows that write (and only that one — our own writes go through `set_anchor`).
fn external_select(state: &mut LibraryState) {
    if state.selected == state.seen_selected {
        return;
    }
    state.sel_ids.clear();
    state.sel_paths.clear();
    state.sel_path = None;
    if let Some(id) = state.selected {
        state.sel_ids.push(id);
    }
}

impl LibraryState {
    fn has(&self, p: &Pick) -> bool {
        match p {
            Pick::Asset(id) => self.sel_ids.contains(id),
            Pick::Path(s) => self.sel_paths.iter().any(|x| x == s),
        }
    }

    /// The item the preview box shows and Shift-click ranges from.
    fn anchor(&self) -> Option<Pick> {
        match (self.selected, &self.sel_path) {
            (Some(id), _) => Some(Pick::Asset(id)),
            (None, Some(p)) => Some(Pick::Path(p.clone())),
            _ => None,
        }
    }

    fn set_anchor(&mut self, p: &Pick) {
        match p {
            Pick::Asset(id) => (self.selected, self.sel_path) = (Some(*id), None),
            Pick::Path(s) => (self.selected, self.sel_path) = (None, Some(s.clone())),
        }
        self.seen_selected = self.selected;
    }

    fn add_sel(&mut self, p: &Pick) {
        if self.has(p) {
            return;
        }
        match p {
            Pick::Asset(id) => self.sel_ids.push(*id),
            Pick::Path(s) => self.sel_paths.push(s.clone()),
        }
    }

    fn drop_sel(&mut self, p: &Pick) {
        match p {
            Pick::Asset(id) => self.sel_ids.retain(|x| x != id),
            Pick::Path(s) => self.sel_paths.retain(|x| x != s),
        }
    }

    fn clear_sel(&mut self) {
        self.sel_ids.clear();
        self.sel_paths.clear();
        self.selected = None;
        self.sel_path = None;
        self.seen_selected = None;
    }
}

/// One click on a row/tile, resolved against the rows as they were drawn this frame.
/// Plain replaces the selection, Ctrl toggles one item, Shift takes the range from the anchor (which
/// stays put, so a second Shift+click re-ranges from the same place, like every file explorer).
fn apply_click(state: &mut LibraryState, rows: &[(Pick, egui::Rect)], pick: &Pick, ctrl: bool, shift: bool) {
    let at = |p: &Pick| rows.iter().position(|(q, _)| q == p);
    if shift {
        if let (Some(i), Some(j)) = (state.anchor().as_ref().and_then(&at), at(pick)) {
            state.sel_ids.clear();
            state.sel_paths.clear();
            for (q, _) in &rows[i.min(j)..=i.max(j)] {
                state.add_sel(q);
            }
            return;
        }
    }
    if ctrl {
        if state.has(pick) {
            state.drop_sel(pick);
        } else {
            state.add_sel(pick);
        }
    } else {
        state.sel_ids.clear();
        state.sel_paths.clear();
        state.add_sel(pick);
    }
    state.set_anchor(pick);
}

/// Rubber band over empty space: press, move, and every row the band touches is selected. Derived from
/// the pointer each frame — no state to keep. A press that landed on a row is left alone: that gesture
/// belongs to the drag-and-drop source.
fn band_select(ui: &egui::Ui, state: &mut LibraryState, rows: &[(Pick, egui::Rect)], palette: &Palette) {
    let (down, origin, pos) =
        ui.input(|i| (i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.interact_pos()));
    let (Some(origin), Some(pos)) = (origin, pos) else { return };
    let area = ui.clip_rect();
    if !down || !area.contains(origin) || (pos - origin).length() < 8.0 {
        return;
    }
    if egui::DragAndDrop::has_any_payload(ui.ctx()) || rows.iter().any(|(_, r)| r.contains(origin)) {
        return;
    }
    let band = egui::Rect::from_two_pos(origin, pos);
    ui.painter().rect(
        band,
        0.0,
        palette.selection.gamma_multiply(0.15),
        egui::Stroke::new(1.0, palette.selection),
        egui::StrokeKind::Inside,
    );
    state.sel_ids.clear();
    state.sel_paths.clear();
    for (p, r) in rows {
        if band.intersects(*r) {
            state.add_sel(p);
        }
    }
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

// ---------- files on disk ----------

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

/// Search + kind chips against a bare path (recents and files on disk have no duration): the SFX/Music
/// chips can only mean "audio" there.
fn path_matches(path: &str, q: &str, kind: u8) -> bool {
    let class_ok = match kind {
        1 | 3 => ext_class(path) == kind,
        2 | 5 | 6 => ext_class(path) == 2,
        4 => false,
        _ => true,
    };
    class_ok && (q.is_empty() || path.to_lowercase().contains(&q.to_lowercase()))
}

/// Same, plus the recent entry's own tags and colour label.
fn recent_matches(r: &RecentAsset, q: &str, kind: u8, label: u8) -> bool {
    if label != 0 && r.label != label {
        return false;
    }
    if !path_matches(&r.path, "", kind) {
        return false;
    }
    q.is_empty()
        || path_matches(&r.path, q, kind)
        || r.tags.iter().any(|t| t.to_lowercase().contains(&q.to_lowercase()))
}

fn recent_remove(settings: &mut Settings, resp: &mut LibraryResponse, path: &str) {
    settings.remove_recent(path);
    resp.settings_changed = true;
}

fn recent_clear(settings: &mut Settings, resp: &mut LibraryResponse) {
    settings.recent_assets.clear();
    resp.settings_changed = true;
}

/// One level of a folder on disk: subfolders first, then media files. None = unreadable.
fn scan_dir(dir: &str) -> Option<Vec<(String, bool)>> {
    let mut v: Vec<(String, bool)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let p = e.path().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (is_dir || ext_class(&p) != 0).then_some((p, is_dir))
        })
        .collect();
    // folders first, then files — a file explorer, not an alphabetical dump
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    Some(v)
}

fn kind_tag_for_class(c: u8) -> &'static str {
    match c {
        1 => "V",
        2 => "A",
        3 => "I",
        _ => "",
    }
}

/// ("file.mp4", "C:\dir") — pure string split on the last path separator.
fn split_path(p: &str) -> (&str, &str) {
    match p.rfind(['\\', '/']) {
        Some(i) => (&p[i + 1..], &p[..i]),
        None => (p, ""),
    }
}

// ---------- pictures ----------

/// Lay `items` out as a grid of tiles, wrapping every time `tile_w` more would not fit the pane.
/// `horizontal_wrapped` cannot do this for tiles: a tile is a drag source whose size is not known until
/// after its contents are drawn, so the wrap test always says the row still has room and a gallery grows
/// one row deep, sideways, forever. Compute the columns instead.
fn tile_grid<T>(ui: &mut egui::Ui, indent: f32, items: &[T], tile_w: f32, mut draw: impl FnMut(&mut egui::Ui, &T)) {
    if items.is_empty() {
        return;
    }
    let spacing = ui.spacing().item_spacing.x;
    let cols = ((ui.available_width() - indent + spacing) / (tile_w + spacing)).floor().max(1.0) as usize;
    for row in items.chunks(cols) {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            for item in row {
                draw(ui, item);
            }
        });
    }
}

/// What fills a picture box.
#[derive(PartialEq, Debug)]
enum Art {
    /// A cached picture (asset thumbnail, effect catalogue card) fitted into the box.
    Image(egui::TextureId, [u32; 2]),
    /// Nothing to photograph (or nothing cached yet): a painted icon.
    Icon(Glyph),
}

/// The picture of a media file `h` px tall: its cached thumbnail, or the painted fallback for its kind.
/// ponytail: asking for a thumbnail is what queues the decode, so expanding a folder of 500 clips queues
/// 500 of them (LIFO, so what you look at wins). Upgrade: only ask for rows inside the viewport.
fn file_art(ui: &egui::Ui, thumbs: &mut Option<&mut ThumbCache>, path: &str, h: u32) -> Art {
    if matches!(ext_class(path), 1 | 3) {
        if let Some((tex, size)) = thumbs.as_deref_mut().and_then(|c| c.texture(ui.ctx(), path, 0.0, h)) {
            if size[1] > 0 {
                return Art::Image(tex, size);
            }
        }
    }
    Art::Icon(fallback_glyph(ext_class(path)))
}

/// The painted stand-in for a file with no thumbnail: a speaker for audio, a film strip for video, a
/// camera for stills, a stack of sheets for everything else.
fn fallback_glyph(class: u8) -> Glyph {
    match class {
        1 => Glyph::FilmStrip,
        2 => Glyph::SpeakerOn,
        3 => Glyph::Camera,
        _ => Glyph::Layers,
    }
}

/// Paint `art` inside `rect` on the panel fill. The box is drawn even when empty, so names line up.
fn paint_art(ui: &egui::Ui, rect: egui::Rect, art: Art, palette: &Palette) {
    let p = ui.painter();
    p.rect_filled(rect, 2.0, palette.panel);
    match art {
        Art::Image(tex, [w, h]) if h > 0 => {
            let k = (rect.width() / w as f32).min(rect.height() / h as f32);
            p.image(
                tex,
                egui::Rect::from_center_size(rect.center(), egui::vec2(w as f32 * k, h as f32 * k)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        Art::Icon(g) => draw_glyph(p, rect, g, palette.text_dim),
        _ => {}
    }
}

/// The picture in front of a file row, `h` points tall (16:9 box, so every row lines up).
fn row_art(ui: &mut egui::Ui, thumbs: &mut Option<&mut ThumbCache>, path: &str, palette: &Palette, h: f32) {
    let art = file_art(ui, thumbs, path, (h * 2.0) as u32);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(h * 16.0 / 9.0, h), egui::Sense::hover());
    paint_art(ui, rect, art, palette);
}

/// Gallery cell width and row thumbnail height at zoom 1.
const TILE: f32 = 108.0;
const ROW_H: f32 = 18.0;
const ZOOM_MIN: f32 = 0.6;
const ZOOM_MAX: f32 = 3.0;

// ---------- shared widgets ----------

/// Yes/No dialog for destructive, non-undoable actions.
pub(crate) fn confirm(title: &str, description: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// A filled dot in the label's colour — painted, because ● is tofu in half the shipped fonts.
fn dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// "Label ▸" submenu over the project's own labels; returns the pick ("Edit labels…" included).
fn label_menu(ui: &mut egui::Ui, current: u8, labels: &Labels, palette: &Palette) -> Option<LabelPick> {
    let mut picked = None;
    ui.menu_button("Label", |ui| {
        if ui.selectable_label(current == 0, "None").clicked() {
            picked = Some(LabelPick::Set(0));
            ui.close();
        }
        for i in 1..=labels.len() as u8 {
            let r = ui.horizontal(|ui| {
                dot(ui, lbl_color(labels, i, palette));
                ui.selectable_label(current == i, lbl_name(labels, i)).clicked()
            });
            if r.inner {
                picked = Some(LabelPick::Set(i));
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

/// Gallery cell: a picture box with the kind tag in its corner and the name under it. Shared by the
/// asset gallery and the reuse sections; the caller decides what the click means.
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
    w: f32,
) -> egui::Response {
    let src = ui.dnd_drag_source(id, payload, |ui| {
        ui.set_max_width(w);
        ui.vertical(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, w * 0.56), egui::Sense::hover());
            paint_art(ui, rect, art, palette);
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

/// Duration cell of an asset — "Loading…" while its import probe is still out (see engine::import).
fn dur_cell(a: &crate::model::Asset) -> String {
    if crate::engine::import::is_probing(&a.path) {
        "Loading…".to_string()
    } else {
        duration_text(a.duration)
    }
}

// ---------- the browser ----------

#[allow(clippy::too_many_arguments)]
fn browser(
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
    let imported = state.tab == 0;

    let mut import = false;
    let mut new_folder = false;
    let mut resp_open = false;
    let mut sort = state.sort;
    let mut import_url = false;
    let mut new_seq = false;
    let mut new_adj = false;
    let mut link = false;
    let mut clear_recent = false;
    toolbar(ui, state, labels, palette, |ui, state| {
        if imported {
            let r = glyph_text_button(ui, Glyph::Letter('+'), "New");
            egui::Popup::menu(&r).show(|ui| {
                if ui.button("Import files…").clicked() {
                    import = true;
                    ui.close();
                }
                if ui.button("Folder…").clicked() {
                    new_folder = true;
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
            });
        } else if ui.button("Link folder…").clicked() {
            link = true;
        }
        if ui.button("Open…").clicked() {
            resp_open = true;
        }
        if ui.button("Import…").clicked() {
            import = true;
        }
        // only when yt-dlp is installed — the whole URL import is optional
        if ytdlp && ui.button("Import URL…").on_hover_text("Download media from a link with yt-dlp").clicked() {
            import_url = true;
        }
        if !imported && ui.button("Clear recent").clicked() && confirm("Clear recent", CLEAR_RECENT) {
            clear_recent = true;
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
    resp.import |= import;
    resp.import_url |= import_url;
    if new_seq {
        ops.push(LibOp::SeqNew);
        op_start = true;
    }
    if new_folder {
        state.new_folder = Some((state.folder.clone().unwrap_or_default(), String::new()));
    }
    if link {
        if let Some(p) = rfd::FileDialog::new().pick_folder() {
            ops.push(LibOp::LinkFolder(p.to_string_lossy().into_owned()));
            op_start = true;
        }
    }
    if clear_recent {
        recent_clear(settings, resp);
        resp.clear_recent = true;
    }
    batch_strip(ui, state, resp);

    // ponytail: both walk every clip / plan item per frame. Upgrade: cache them behind a generation
    // counter bumped by App::after_edit() if a big project ever shows it.
    let used = project.used_assets();
    let planned = project.plan_assets();
    // a search or a filter chip flattens "Imported" into its hits, the way an explorer shows search results
    let flat = !state.search.is_empty() || state.kind_filter != 0 || state.label_filter != 0 || state.unused_only;

    // the preview owns the bottom of the pane, so it never scrolls away with the list
    preview_panel(ui, state, project, settings, thumbs, labels, palette, resp, &mut ops, &mut op_start);

    let mut rows: Vec<(Pick, egui::Rect)> = Vec::new();
    let mut click: Option<(Pick, bool, bool)> = None;
    let mut rec: Option<RecOp> = None;
    // no drag-to-scroll: a drag across the list is a rubber band or a drag-and-drop, never a scroll
    let source = egui::scroll_area::ScrollSource { drag: false, ..Default::default() };
    egui::ScrollArea::vertical().auto_shrink(false).scroll_source(source).show(ui, |ui| {
        zoom_scroll(ui, state);
        // filtered + sorted assets; the tree hangs each of them under its own folder
        let mut order: Vec<usize> = (0..project.assets.len())
            .filter(|&i| {
                let a = &project.assets[i];
                // the folder chosen in the tree only narrows a search — the tree itself shows every folder
                (!flat || folder_ok(state.folder.as_deref(), &a.folder))
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
        {
            let mut tree = Tree {
                state: &mut *state,
                project: &*project,
                settings: &*settings,
                used: &used,
                labels,
                palette,
                thumbs: &mut *thumbs,
                resp: &mut *resp,
                ops: &mut ops,
                op_start: &mut op_start,
                rows: &mut rows,
                click: &mut click,
                rec: &mut rec,
            };
            if imported {
                tree.imported(ui, &order, flat);
            } else {
                tree.global(ui);
            }
        }
        if let Some((pick, ctrl, shift)) = click.take() {
            apply_click(state, &rows, &pick, ctrl, shift);
        }

        if imported {
            ui.add_space(4.0);
            let unused = project.assets.iter().filter(|a| !used.contains(&a.id) && !planned.contains(&a.id)).count();
            if ui.add_enabled(unused > 0, egui::Button::new(format!("Remove unused ({unused})"))).clicked()
                && confirm("Remove unused", &format!("Remove {unused} unused assets from the project?"))
            {
                ops.push(LibOp::RemoveUnused);
                op_start = true;
            }
            sequences_section(ui, state, project, palette, resp, &mut ops, &mut op_start);
            templates_section(ui, state, settings, palette, resp);
            reuse_ui(ui, state.view, state.zoom, project, settings, palette, resp);
        }
        // clicking the empty space below the tree drops the selection, and the preview with it
        let rest = ui.available_size();
        if rest.y > 1.0 && ui.allocate_response(rest, egui::Sense::click()).clicked() {
            state.clear_sel();
            state.folder = None;
        }
        band_select(ui, state, &rows, palette);
    });
    match rec {
        Some(RecOp::Pin(path)) => {
            if let Some(r) = settings.recent_assets.iter_mut().find(|r| r.path == path) {
                r.pinned = !r.pinned;
            }
            settings.sort_recent();
            resp.settings_changed = true;
        }
        Some(RecOp::Label(path, l)) => {
            if let Some(r) = settings.recent_assets.iter_mut().find(|r| r.path == path) {
                r.label = l;
            }
            resp.settings_changed = true;
        }
        Some(RecOp::Remove(path)) => recent_remove(settings, resp, &path),
        None => {}
    }

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
                    state.dirs.retain(|(f, _)| !f.starts_with(&p));
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

/// Ctrl+Scroll (and pinch) over the list scales the thumbnails, like every asset browser.
fn zoom_scroll(ui: &egui::Ui, state: &mut LibraryState) {
    let hovering = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| ui.clip_rect().contains(p));
    if !hovering {
        return;
    }
    // egui folds Ctrl+Scroll into zoom_delta and keeps it out of the scroll delta
    let z = ui.input(|i| i.zoom_delta());
    if (z - 1.0).abs() > 1e-4 {
        state.zoom = (state.zoom * z).clamp(ZOOM_MIN, ZOOM_MAX);
    }
}

/// What to do with everything selected. Every button routes through the `LibraryResponse` fields the
/// per-item menus already use, so App needs no new plumbing.
/// ponytail: the Convert and Compress windows each take one file, so those two open on the first asset
/// of the selection. Upgrade: queue one job per asset once those windows accept a list.
fn batch_strip(ui: &mut egui::Ui, state: &LibraryState, resp: &mut LibraryResponse) {
    let n = state.sel_ids.len() + state.sel_paths.len();
    if n == 0 {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.weak(RichText::new(format!("{n} selected")).small());
        if !state.sel_paths.is_empty() && ui.small_button("Import").clicked() {
            resp.open_paths.extend(state.sel_paths.iter().map(PathBuf::from));
        }
        if state.sel_ids.is_empty() {
            return;
        }
        if !state.sel_ids.is_empty() && ui.small_button("Add to timeline").clicked() {
            resp.add_to_timeline.extend(state.sel_ids.iter().copied());
        }
        ui.menu_button("Convert To", |ui| {
            for t in crate::engine::convert::TARGETS {
                if ui.button(*t).clicked() {
                    resp.convert.extend(state.sel_ids.iter().map(|id| (*id, (*t).to_string())));
                    ui.close();
                }
            }
        });
        if ui.small_button("Convert…").clicked() {
            resp.convert_dialog = state.sel_ids.first().copied();
        }
        if ui.small_button("Compress…").clicked() {
            resp.compress = state.sel_ids.first().copied();
        }
        if ui.small_button("Remove from project").clicked() {
            resp.remove.extend(state.sel_ids.iter().copied());
        }
    });
    ui.separator();
}

/// The search + filter toolbar: a framed band in the panel fill with a separator under it, so it never
/// reads as content. `head` draws the tab-specific leading row (New / Open / Import / sort).
fn toolbar(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    labels: &Labels,
    palette: &Palette,
    head: impl FnOnce(&mut egui::Ui, &mut LibraryState),
) {
    egui::Frame::new().fill(palette.panel).inner_margin(egui::Margin::symmetric(4, 3)).show(ui, |ui| {
        // wrapped: a narrow pane must stack the buttons, not push them past the right edge
        ui.horizontal_wrapped(|ui| head(ui, state));
        ui.horizontal(|ui| {
            let (mag, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            draw_glyph(ui.painter(), mag, Glyph::Zoom, palette.text_dim);
            let clear_w = 26.0;
            // TextEdit::desired_width is the *text* width; its frame adds a margin on top, so the row
            // used to be ~8px wider than the pane and dragged every later widget out with it.
            let margin = ui.spacing().button_padding.x * 2.0;
            let w = (ui.available_width() - clear_w - ui.spacing().item_spacing.x - margin).max(24.0);
            let r = ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("search").desired_width(w));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("lib_search_rect"), r.rect));
            let _ = r;
            let id = egui::Id::new("lib_clear_search");
            if icon_button(ui, palette, id, Glyph::Letter('X'), "Clear search", false).clicked() {
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
            if icon_button(ui, palette, egui::Id::new("lib_zoom_out"), Glyph::Letter('-'), "Smaller", false).clicked() {
                state.zoom = (state.zoom / 1.25).clamp(ZOOM_MIN, ZOOM_MAX);
            }
            if icon_button(ui, palette, egui::Id::new("lib_zoom_in"), Glyph::Letter('+'), "Bigger", false).clicked() {
                state.zoom = (state.zoom * 1.25).clamp(ZOOM_MIN, ZOOM_MAX);
            }
            ui.separator();
            ui.weak(RichText::new("Filters").small());
            for (i, n) in ["All", "Video", "Audio", "Image", "Seq", "Short SFX", "Music"].iter().enumerate() {
                if ui.selectable_label(state.kind_filter == i as u8, RichText::new(*n).small()).clicked() {
                    state.kind_filter = i as u8;
                }
            }
            for i in 1..=labels.len() as u8 {
                let id = egui::Id::new(("lib_label_chip", i));
                let on = state.label_filter == i;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                let r = ui.interact(rect, id, egui::Sense::click());
                ui.painter().circle_filled(rect.center(), if on { 6.0 } else { 4.5 }, lbl_color(labels, i, palette));
                if r.on_hover_text(lbl_name(labels, i)).clicked() {
                    state.label_filter = if on { 0 } else { i };
                }
            }
            ui.toggle_value(&mut state.unused_only, RichText::new("Unused").small());
        });
    });
    ui.separator();
}

// ---------- the preview ----------

/// The dedicated asset preview: the bottom of the pane, and the only place tags / description / label /
/// folder are edited.
#[allow(clippy::too_many_arguments)]
fn preview_panel(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    settings: &mut Settings,
    thumbs: &mut Option<&mut ThumbCache>,
    labels: &Labels,
    palette: &Palette,
    resp: &mut LibraryResponse,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    egui::TopBottomPanel::bottom("lib_preview")
        .resizable(true)
        .height_range(48.0..=420.0)
        .default_height(178.0)
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match state.anchor() {
                Some(Pick::Asset(id)) if project.asset(id).is_some() => {
                    asset_preview(ui, state, project, id, thumbs, labels, palette, resp, ops, op_start)
                }
                Some(Pick::Path(p)) => path_preview(ui, state, settings, thumbs, palette, resp, &p),
                _ => {
                    ui.weak("Select a file to preview it");
                }
            });
        });
}

/// Name / path / format, over the picture — shared by both previews.
fn preview_head(
    ui: &mut egui::Ui,
    thumbs: &mut Option<&mut ThumbCache>,
    palette: &Palette,
    path: &str,
    meta: &str,
    extra: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        let h = 84.0;
        let art = file_art(ui, thumbs, path, (h * 2.0) as u32);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(h * 16.0 / 9.0, h), egui::Sense::hover());
        paint_art(ui, rect, art, palette);
        ui.vertical(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.strong(split_path(path).0);
            ui.add(egui::Label::new(RichText::new(path).weak().small()).truncate()).on_hover_text(path);
            ui.weak(meta);
            extra(ui);
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn asset_preview(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    id: Id,
    thumbs: &mut Option<&mut ThumbCache>,
    labels: &Labels,
    palette: &Palette,
    resp: &mut LibraryResponse,
    ops: &mut Vec<LibOp>,
    op_start: &mut bool,
) {
    let Some(a) = project.asset(id) else { return };
    if state.tags_for != Some(a.id) {
        state.tags_for = Some(a.id);
        state.tags_buf = a.tags.join(", ");
    }
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
    preview_head(ui, thumbs, palette, &a.path, &line, |ui| {
        ui.horizontal(|ui| {
            dot(ui, lbl_color(labels, a.label, palette));
            egui::ComboBox::from_id_salt("asset_label").selected_text(lbl_name(labels, a.label)).show_ui(ui, |ui| {
                let mut pick = |l: u8, ui: &mut egui::Ui| {
                    ops.push(LibOp::AssetLabel(a.id, l));
                    *op_start = true;
                    ui.close();
                };
                if ui.selectable_label(a.label == 0, "None").clicked() {
                    pick(0, ui);
                }
                for i in 1..=labels.len() as u8 {
                    let hit = ui.horizontal(|ui| {
                        dot(ui, lbl_color(labels, i, palette));
                        ui.selectable_label(a.label == i, lbl_name(labels, i)).clicked()
                    });
                    if hit.inner {
                        pick(i, ui);
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
        ops.push(LibOp::AssetTags(a.id, split_tags(&state.tags_buf)));
    }
    *op_start |= r.gained_focus();
}

fn split_tags(s: &str) -> Vec<String> {
    s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// Preview of a file that is not in the project yet: import it from here, and edit the tags it carries
/// as a recent file (the only place a file outside the project can keep any).
fn path_preview(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    settings: &mut Settings,
    thumbs: &mut Option<&mut ThumbCache>,
    palette: &Palette,
    resp: &mut LibraryResponse,
    path: &str,
) {
    let meta = kind_tag_for_class(ext_class(path)).to_string();
    preview_head(ui, thumbs, palette, path, &meta, |ui| {
        if ui.button("Import to project").clicked() {
            resp.open_paths.push(PathBuf::from(path));
        }
    });
    let Some(i) = settings.recent_assets.iter().position(|r| r.path.eq_ignore_ascii_case(path)) else { return };
    if state.recent_tags_for.as_deref() != Some(path) {
        state.recent_tags_for = Some(path.to_string());
        state.recent_tags_buf = settings.recent_assets[i].tags.join(", ");
    }
    let r = ui.add(
        egui::TextEdit::singleline(&mut state.recent_tags_buf)
            .desired_width(f32::INFINITY)
            .hint_text("tags, comma, separated"),
    );
    if r.changed() {
        settings.recent_assets[i].tags = split_tags(&state.recent_tags_buf);
        resp.settings_changed = true;
    }
}

// ---------- the file tree ----------

/// Indent of a row `depth` levels into the tree. Rows that hold no disclosure triangle (files, hints)
/// add `ARROW`, so their icon lines up with the folder icons beside them.
fn indent(depth: usize) -> f32 {
    4.0 + depth as f32 * 14.0
}

/// Width of the disclosure column.
const ARROW: f32 = 14.0;

/// The folder that directly contains `path`; "" for a top-level folder.
fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

/// Every project folder plus the parents only an asset implies ("A/B" with no "A" in `folders`), so no
/// branch of the tree hangs off a node that is never drawn.
fn folder_tree_names(project: &Project) -> Vec<String> {
    let mut names = project.folder_names();
    let mut implied: Vec<String> = Vec::new();
    for n in &names {
        let mut p = parent_of(n);
        while !p.is_empty() {
            implied.push(p.to_string());
            p = parent_of(p);
        }
    }
    names.append(&mut implied);
    names.sort();
    names.dedup();
    names
}

/// Tree key of a folder on disk (one place, so the tests build the same key).
fn dir_key(path: &str) -> String {
    format!("d:{path}")
}

/// Shared borrows for the tree rows: they recurse, and would otherwise thread a dozen arguments each.
struct Tree<'a, 'b> {
    state: &'a mut LibraryState,
    project: &'a Project,
    settings: &'a Settings,
    used: &'a std::collections::HashSet<Id>,
    labels: &'a Labels,
    palette: &'a Palette,
    thumbs: &'a mut Option<&'b mut ThumbCache>,
    resp: &'a mut LibraryResponse,
    ops: &'a mut Vec<LibOp>,
    op_start: &'a mut bool,
    /// Every selectable row of this frame, in draw order — Shift+click ranges and the band read it.
    rows: &'a mut Vec<(Pick, egui::Rect)>,
    /// The click to resolve once every row is known.
    click: &'a mut Option<(Pick, bool, bool)>,
    /// settings.json edit asked for by a recent file's menu; `settings` is only borrowed shared here,
    /// so the browser applies it after the tree.
    rec: &'a mut Option<RecOp>,
}

impl Tree<'_, '_> {
    /// Is this node open? Only the flip away from `dflt` is remembered, so a new folder inherits it.
    fn is_open(&self, key: &str, dflt: bool) -> bool {
        self.state.flipped.iter().any(|k| k == key) != dflt
    }

    fn flip(&mut self, key: &str) {
        match self.state.flipped.iter().position(|k| k == key) {
            Some(i) => {
                self.state.flipped.remove(i);
            }
            None => self.state.flipped.push(key.to_string()),
        }
    }

    /// The disclosure triangle at the head of a tree row; returns the node's state after this frame's
    /// click. Painted, not written: ▸ and ▾ are tofu boxes in Segoe UI.
    fn arrow(&mut self, ui: &mut egui::Ui, key: &str, dflt: bool, children: bool) -> bool {
        let open = self.is_open(key, dflt);
        let (rect, r) = ui.allocate_exact_size(egui::vec2(ARROW, 14.0), egui::Sense::click());
        if !children {
            return false;
        }
        let c = rect.center();
        let pts = if open {
            vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.5)]
        } else {
            vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.5, 0.0), c + egui::vec2(-2.0, 4.0)]
        };
        let col = if r.hovered() { self.palette.text } else { self.palette.text_dim };
        ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
        if r.clicked() {
            self.flip(key);
            return !open;
        }
        open
    }

    fn folder_icon(&self, ui: &mut egui::Ui, g: Glyph) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
        draw_glyph(ui.painter(), rect, g, self.palette.text_dim);
    }

    /// Move what was dropped onto a folder row into that folder — the whole selection when the dragged
    /// asset was part of it, so a marquee of clips moves in one gesture.
    fn drop_asset(&mut self, r: &egui::Response, folder: &str) {
        let Some(p) = r.dnd_release_payload::<DragPayload>() else { return };
        let DragPayload::Asset(id) = *p else { return };
        let ids: Vec<Id> = if self.state.sel_ids.contains(&id) { self.state.sel_ids.clone() } else { vec![id] };
        for id in ids {
            self.ops.push(LibOp::AssetFolder(id, folder.to_string()));
        }
        *self.op_start = true;
    }

    /// Register a drawn row and route its click (the selection is resolved once every row is known).
    fn hit(&mut self, ui: &egui::Ui, r: &egui::Response, pick: Pick, path: &str) {
        self.rows.push((pick.clone(), r.rect));
        if r.clicked() {
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            if !shift {
                self.resp.preview = Some(PathBuf::from(path));
            }
            *self.click = Some((pick, ctrl, shift));
        } else if r.secondary_clicked() && !self.state.has(&pick) {
            // right-clicking outside the selection selects that one item first, like every explorer
            *self.click = Some((pick, false, false));
        }
    }

    /// Root 1 — "Imported": the folders and files this project contains.
    fn imported(&mut self, ui: &mut egui::Ui, order: &[usize], flat: bool) {
        if flat {
            self.assets(ui, 0, order);
            return;
        }
        let names = folder_tree_names(self.project);
        self.folder(ui, "", 0, &names, order);
    }

    /// The subfolders of `path` (folders first, like an explorer), then the assets that live in it.
    fn folder(&mut self, ui: &mut egui::Ui, path: &str, depth: usize, names: &[String], order: &[usize]) {
        for child in names.iter().filter(|n| parent_of(n) == path) {
            let key = format!("f:{child}");
            let last = child.rsplit('/').next().unwrap_or(child).to_string();
            let kids = names.iter().any(|n| parent_of(n) == child.as_str())
                || order.iter().any(|&i| self.project.assets[i].folder == *child);
            let mut open = false;
            let mut renamed: Option<String> = None;
            ui.horizontal(|ui| {
                ui.add_space(indent(depth));
                open = self.arrow(ui, &key, true, kids);
                self.folder_icon(ui, Glyph::Folder);
                if self.state.rename_folder.as_ref().is_some_and(|(o, _)| o == child) {
                    let (_, buf) = self.state.rename_folder.as_mut().unwrap();
                    renamed = inline_edit(ui, buf);
                    return;
                }
                let r = ui.selectable_label(self.state.folder.as_deref() == Some(child.as_str()), &last);
                if r.clicked() {
                    self.state.folder = Some(child.clone());
                }
                r.context_menu(|ui| {
                    if ui.button("New folder").clicked() {
                        self.state.new_folder = Some((child.clone(), String::new()));
                        ui.close();
                    }
                    if ui.button("Rename").clicked() {
                        self.state.rename_folder = Some((child.clone(), last.clone()));
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        self.ops.push(LibOp::FolderDelete(child.clone()));
                        *self.op_start = true;
                        ui.close();
                    }
                });
                self.drop_asset(&r, child);
            });
            if let Some(new_last) = renamed {
                if !new_last.is_empty() && new_last != last {
                    let parent = parent_of(child);
                    let new = if parent.is_empty() { new_last } else { format!("{parent}/{new_last}") };
                    self.ops.push(LibOp::FolderRename(child.clone(), new));
                    *self.op_start = true;
                }
                self.state.rename_folder = None;
            }
            if open {
                self.folder(ui, child, depth + 1, names, order);
            }
        }
        // the "New folder" field appears under the folder it was asked for
        if self.state.new_folder.as_ref().is_some_and(|(p, _)| p == path) {
            let mut done = None;
            ui.horizontal(|ui| {
                ui.add_space(indent(depth) + ARROW);
                self.folder_icon(ui, Glyph::Folder);
                let (_, buf) = self.state.new_folder.as_mut().unwrap();
                done = inline_edit(ui, buf);
            });
            if let Some(name) = done {
                if !name.is_empty() {
                    let full = if path.is_empty() { name } else { format!("{path}/{name}") };
                    self.ops.push(LibOp::FolderNew(full));
                    *self.op_start = true;
                }
                self.state.new_folder = None;
            }
        }
        let here: Vec<usize> = order.iter().copied().filter(|&i| self.project.assets[i].folder == *path).collect();
        self.assets(ui, depth, &here);
    }

    /// A run of assets, as rows or as gallery tiles (already filtered and sorted by the caller).
    fn assets(&mut self, ui: &mut egui::Ui, depth: usize, list: &[usize]) {
        if self.state.view == 1 {
            let w = TILE * self.state.zoom;
            tile_grid(ui, indent(depth) + ARROW, list, w, |ui, &i| self.asset_tile(ui, i));
        } else {
            for &i in list {
                self.asset_row(ui, depth, i);
            }
        }
    }

    fn asset_row(&mut self, ui: &mut egui::Ui, depth: usize, i: usize) {
        let a = &self.project.assets[i];
        let selected = self.state.sel_ids.contains(&a.id);
        let tint = (a.label != 0).then(|| lbl_color(self.labels, a.label, self.palette));
        let used = self.used.contains(&a.id);
        let (palette, thumbs, h) = (self.palette, &mut *self.thumbs, ROW_H * self.state.zoom);
        let (r, add) = row(
            ui,
            egui::Id::new(("asset", a.id)),
            DragPayload::Asset(a.id),
            selected,
            selected.then_some("Add"),
            |ui| {
                ui.add_space(indent(depth) + ARROW);
                // label tint: a colour bar on the left and the name in the same colour
                if let Some(c) = tint {
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, h), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 1.0, c);
                }
                row_art(ui, thumbs, &a.path, palette, h);
                let mut name = RichText::new(a.name());
                if let Some(c) = tint {
                    name = name.color(c);
                }
                ui.label(if selected { name.strong() } else { name });
                ui.weak(kind_tag(a.kind));
                ui.weak(dur_cell(a));
                if used {
                    let (d, _) = ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
                    ui.painter().circle_filled(d.center(), 3.0, palette.accent);
                    ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()).on_hover_text("Used in the timeline");
                }
                if !a.tags.is_empty() {
                    ui.add(egui::Label::new(RichText::new(a.tags.join(" · ")).weak().small()).truncate());
                }
            },
        );
        let (id, path) = (a.id, a.path.clone());
        self.hit(ui, &r, Pick::Asset(id), &path);
        if add || r.double_clicked() {
            self.resp.add_to_timeline.push(id);
        }
        self.asset_menu(&r, i);
    }

    fn asset_tile(&mut self, ui: &mut egui::Ui, i: usize) {
        let a = &self.project.assets[i];
        let selected = self.state.sel_ids.contains(&a.id);
        let tint = (a.label != 0).then(|| lbl_color(self.labels, a.label, self.palette)).unwrap_or(self.palette.text);
        let w = TILE * self.state.zoom;
        let art = file_art(ui, self.thumbs, &a.path, (w * 0.56 * 2.0) as u32);
        let id = egui::Id::new(("tile", a.id));
        let payload = DragPayload::Asset(a.id);
        let r = tile(ui, id, payload, selected, kind_tag(a.kind), &a.name(), tint, self.palette, art, w);
        let (aid, path) = (a.id, a.path.clone());
        self.hit(ui, &r, Pick::Asset(aid), &path);
        if r.double_clicked() {
            self.resp.add_to_timeline.push(aid);
        }
        self.asset_menu(&r, i);
        r.on_hover_text(&path);
    }

    /// Every id the menu acts on: the whole selection when the item is part of it, else just it.
    fn menu_targets(&self, id: Id) -> Vec<Id> {
        if self.state.sel_ids.contains(&id) {
            self.state.sel_ids.clone()
        } else {
            vec![id]
        }
    }

    fn asset_menu(&mut self, r: &egui::Response, i: usize) {
        let a = &self.project.assets[i];
        let (labels, palette) = (self.labels, self.palette);
        let ids = self.menu_targets(a.id);
        r.context_menu(|ui| {
            if ui.button("Add to timeline at playhead").clicked() {
                self.resp.add_to_timeline.extend(ids.iter().copied());
                ui.close();
            }
            if ui.button("Reveal folder").clicked() {
                let _ = std::process::Command::new("explorer").arg(format!("/select,{}", a.path)).spawn();
                ui.close();
            }
            match label_menu(ui, a.label, labels, palette) {
                Some(LabelPick::Set(l)) => {
                    for id in &ids {
                        self.ops.push(LibOp::AssetLabel(*id, l));
                    }
                    *self.op_start = true;
                }
                Some(LabelPick::Edit) => self.resp.edit_labels = true,
                None => {}
            }
            ui.menu_button("Convert To", |ui| {
                for t in crate::engine::convert::TARGETS {
                    if ui.button(*t).clicked() {
                        self.resp.convert.extend(ids.iter().map(|id| (*id, (*t).to_string())));
                        ui.close();
                    }
                }
            });
            if ui.button("Convert To… (options)").clicked() {
                self.resp.convert_dialog = Some(a.id);
                ui.close();
            }
            if ui.button("Compress…").clicked() {
                self.resp.compress = Some(a.id);
                ui.close();
            }
            if ui.button("Remove from project").clicked() {
                self.resp.remove.extend(ids.iter().copied());
                ui.close();
            }
        });
    }

    /// Root 2 — "Global": Recent, and the folders the user linked, browsed straight from disk.
    fn global(&mut self, ui: &mut egui::Ui) {
        self.recent(ui, 0);
        let linked: &[String] = &self.project.linked_folders;
        for folder in linked {
            self.dir(ui, folder, 0, true);
        }
        if linked.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(indent(0) + ARROW);
                ui.weak("(no linked folders — \"Link folder…\" above)");
            });
        }
    }

    /// Recent files, from settings.json — everything opened recently, across every project.
    fn recent(&mut self, ui: &mut egui::Ui, depth: usize) {
        let mut open = false;
        ui.horizontal(|ui| {
            ui.add_space(indent(depth));
            open = self.arrow(ui, "recent", false, true);
            self.folder_icon(ui, Glyph::Hourglass);
            let r = ui.selectable_label(false, "Recent").on_hover_text("Files opened recently, across every project");
            if r.clicked() {
                self.flip("recent");
                open = !open;
            }
        });
        if !open {
            return;
        }
        let (q, kind, label) = (self.state.search.clone(), self.state.kind_filter, self.state.label_filter);
        let paths: Vec<String> = self
            .settings
            .recent_assets
            .iter()
            .filter(|r| recent_matches(r, &q, kind, label))
            .map(|r| r.path.clone())
            .collect();
        if paths.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(indent(depth + 1) + ARROW);
                ui.weak("(none)");
            });
            return;
        }
        self.files(ui, depth + 1, &paths, true);
    }

    /// One folder on disk. `root` marks a linked folder — Refresh / Unlink live in its menu.
    fn dir(&mut self, ui: &mut egui::Ui, path: &str, depth: usize, root: bool) {
        let key = dir_key(path);
        let name = path.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or(path).to_string();
        let mut open = false;
        ui.horizontal(|ui| {
            ui.add_space(indent(depth));
            open = self.arrow(ui, &key, false, true);
            self.folder_icon(ui, Glyph::Folder);
            let r = ui.selectable_label(false, &name);
            if r.clicked() {
                self.flip(&key);
                open = !open;
            }
            r.context_menu(|ui| {
                if ui.button("Refresh").clicked() {
                    self.state.dirs.retain(|(p, _)| !p.starts_with(path));
                    ui.close();
                }
                if root && ui.button("Unlink folder").clicked() {
                    self.ops.push(LibOp::UnlinkFolder(path.to_string()));
                    *self.op_start = true;
                    ui.close();
                }
            });
            r.on_hover_text(path);
        });
        if !open {
            return;
        }
        let Some(entries) = self.listing(path) else {
            ui.horizontal(|ui| {
                ui.add_space(indent(depth + 1) + ARROW);
                ui.weak("(folder unavailable)");
            });
            return;
        };
        if entries.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(indent(depth + 1) + ARROW);
                ui.weak("(empty)");
            });
        }
        for (p, _) in entries.iter().filter(|(_, d)| *d) {
            self.dir(ui, p, depth + 1, false);
        }
        let files: Vec<String> = entries.into_iter().filter(|(_, d)| !*d).map(|(p, _)| p).collect();
        self.files(ui, depth + 1, &files, false);
    }

    /// One level of a folder, read the first time the node is expanded and cached until Refresh.
    /// ponytail: a blocking read_dir on the UI thread, one level per expansion — an eager recursive walk
    /// of a media drive would hang the editor. Upgrade: scan on a worker + mpsc, like media/waveform.rs.
    fn listing(&mut self, path: &str) -> Option<Vec<(String, bool)>> {
        if let Some((_, v)) = self.state.dirs.iter().find(|(p, _)| p.as_str() == path) {
            return v.clone();
        }
        let v = scan_dir(path);
        self.state.dirs.push((path.to_string(), v.clone()));
        v
    }

    /// A run of files that are not in the project. `recent` shows their folder and Recent label colour.
    fn files(&mut self, ui: &mut egui::Ui, depth: usize, paths: &[String], recent: bool) {
        if self.state.view == 1 {
            let w = TILE * self.state.zoom;
            tile_grid(ui, indent(depth) + ARROW, paths, w, |ui, p| self.file_tile(ui, p, recent));
        } else {
            for p in paths {
                self.file(ui, p, depth, recent);
            }
        }
    }

    /// Colour label of a recent file, if it has one.
    fn recent_tint(&self, path: &str) -> Option<egui::Color32> {
        let r = self.settings.recent_assets.iter().find(|r| r.path.eq_ignore_ascii_case(path))?;
        (r.label != 0).then(|| lbl_color(self.labels, r.label, self.palette))
    }

    /// One media file on disk: thumbnail, name, kind. Single click previews it, double click imports it.
    fn file(&mut self, ui: &mut egui::Ui, path: &str, depth: usize, recent: bool) {
        if !path_matches(path, &self.state.search, self.state.kind_filter) {
            return;
        }
        let selected = self.state.sel_paths.iter().any(|p| p == path);
        let tint = if recent { self.recent_tint(path) } else { None };
        let (name, dir) = split_path(path);
        let (name, dir) = (name.to_string(), dir.to_string());
        let (palette, thumbs, h) = (self.palette, &mut *self.thumbs, ROW_H * self.state.zoom);
        let id = egui::Id::new(("file", depth, recent, path));
        let (r, _) = row(ui, id, DragPayload::Path(path.to_string()), selected, None, |ui| {
            ui.add_space(indent(depth) + ARROW);
            if let Some(c) = tint {
                let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, h), egui::Sense::hover());
                ui.painter().rect_filled(bar, 1.0, c);
            }
            row_art(ui, thumbs, path, palette, h);
            match tint {
                Some(c) => ui.label(RichText::new(&name).color(c)),
                None => ui.label(&name),
            };
            ui.weak(kind_tag_for_class(ext_class(path)));
            if recent {
                ui.add(egui::Label::new(RichText::new(&dir).weak().small()).truncate());
            }
        });
        self.file_click(ui, &r, path, recent);
        r.on_hover_text(path);
    }

    fn file_tile(&mut self, ui: &mut egui::Ui, path: &str, recent: bool) {
        if !path_matches(path, &self.state.search, self.state.kind_filter) {
            return;
        }
        let selected = self.state.sel_paths.iter().any(|p| p == path);
        let tint = if recent { self.recent_tint(path) } else { None };
        let class = ext_class(path);
        let w = TILE * self.state.zoom;
        let art = file_art(ui, self.thumbs, path, (w * 0.56 * 2.0) as u32);
        let name = split_path(path).0.to_string();
        let id = egui::Id::new(("file_tile", recent, path));
        let payload = DragPayload::Path(path.to_string());
        let tint = tint.unwrap_or(self.palette.text);
        let r = tile(ui, id, payload, selected, kind_tag_for_class(class), &name, tint, self.palette, art, w);
        self.file_click(ui, &r, path, recent);
        r.on_hover_text(path);
    }

    /// Select + preview on a single click, import on a double click, plus the file menu (pins, labels
    /// and the recent list live here now that "Global" is where recent files are shown).
    fn file_click(&mut self, ui: &egui::Ui, r: &egui::Response, path: &str, recent: bool) {
        self.hit(ui, r, Pick::Path(path.to_string()), path);
        if r.double_clicked() {
            self.resp.open_paths.push(PathBuf::from(path));
        }
        let paths: Vec<String> = if self.state.sel_paths.iter().any(|p| p == path) {
            self.state.sel_paths.clone()
        } else {
            vec![path.to_string()]
        };
        let pinned = self.settings.recent_assets.iter().any(|r| r.path.eq_ignore_ascii_case(path) && r.pinned);
        let label =
            self.settings.recent_assets.iter().find(|r| r.path.eq_ignore_ascii_case(path)).map_or(0, |r| r.label);
        let (labels, palette) = (self.labels, self.palette);
        r.context_menu(|ui| {
            if ui.button("Add to the project").clicked() {
                self.resp.open_paths.extend(paths.iter().map(PathBuf::from));
                ui.close();
            }
            if ui.button("Reveal folder").clicked() {
                let _ = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn();
                ui.close();
            }
            if !recent {
                return;
            }
            if ui.button(if pinned { "Unpin" } else { "Pin" }).clicked() {
                *self.rec = Some(RecOp::Pin(path.to_string()));
                ui.close();
            }
            match label_menu(ui, label, labels, palette) {
                Some(LabelPick::Set(l)) => *self.rec = Some(RecOp::Label(path.to_string(), l)),
                Some(LabelPick::Edit) => self.resp.edit_labels = true,
                None => {}
            }
            if ui.button("Remove from recent").clicked() {
                *self.rec = Some(RecOp::Remove(path.to_string()));
                ui.close();
            }
        });
    }
}

// ---------- reusable things (bottom of the Imported tab) ----------

enum RecOp {
    Pin(String),
    Label(String, u8),
    Remove(String),
}

const CLEAR_RECENT: &str = "Clear the whole recent list? Pins, labels and tags are lost (no undo).";

fn sequences_section(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &Project,
    palette: &Palette,
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
                glyph(ui, Glyph::FilmStrip, palette);
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
            glyph(ui, Glyph::FilmStrip, palette);
            ui.label(&s.name);
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

/// A painted icon the size of a text label, for the rows that are not files.
fn glyph(ui: &mut egui::Ui, g: Glyph, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
    draw_glyph(ui.painter(), rect, g, palette.text_dim);
}

fn templates_section(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    settings: &mut Settings,
    palette: &Palette,
    resp: &mut LibraryResponse,
) {
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
                glyph(ui, Glyph::Layers, palette);
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
                glyph(ui, Glyph::Layers, palette);
                ui.label(&name);
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

/// One reusable thing the project already holds.
enum Reuse {
    /// An effect kind already used somewhere in the project.
    Effect(EffectKind),
    /// Index into `Settings::effect_presets` — a saved chain or a saved node graph.
    Preset(usize),
    /// A clip that renders from a node graph.
    Graph(Id),
    /// Index into `Settings::templates` holding a saved adjustment layer.
    Adjustment(usize),
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
        // uses_graph, not graph.is_some(): a bare Input->Output pass-through is not worth reusing, and
        // since the node editor became opt-in it no longer means the clip renders from a graph
        if c.uses_graph() {
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
    vec![("Effects", fx), ("Node graphs", graphs), ("Adjustment layers", adj)]
}

/// (name, kind tag, icon, drag payload) of one reusable item.
/// ponytail: only adjustment layers have a drop target today — the others carry their name as a
/// `Template` payload because `row()`/`tile()` are drag sources, and a stray drop just misses.
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
    }
}

/// What a click on a reusable item asks the app to do.
fn reuse_pick(item: &Reuse, name: &str, resp: &mut LibraryResponse) {
    match *item {
        Reuse::Effect(k) => resp.add_effect = Some(k),
        Reuse::Preset(i) => resp.apply_preset = Some(i),
        Reuse::Graph(id) => resp.copy_graph = Some(id),
        Reuse::Adjustment(_) => resp.place_template.push(name.to_string()),
    }
}

/// The reusable sections under the tree, in whichever view the toolbar selected.
fn reuse_ui(
    ui: &mut egui::Ui,
    view: u8,
    zoom: f32,
    project: &Project,
    settings: &Settings,
    palette: &Palette,
    resp: &mut LibraryResponse,
) {
    for (title, items) in reuse_sections(project, settings) {
        ui.add_space(4.0);
        ui.strong(title);
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
                tile(ui, id, payload, false, tag, &name, palette.text, palette, art, TILE * zoom)
            } else {
                row(ui, id, payload, false, None, |ui| {
                    glyph(ui, icon, palette);
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
            let indexed: Vec<(usize, &Reuse)> = items.iter().enumerate().collect();
            tile_grid(ui, 0.0, &indexed, TILE * zoom, |ui, &(i, item)| draw(ui, i, item));
        } else {
            for (i, item) in items.iter().enumerate() {
                draw(ui, i, item);
            }
        }
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
    fn scan_dir_reports_unreadable_folders() {
        assert!(scan_dir(r"C:\does\not\exist\at\all").is_none());
        assert!(scan_dir(&std::env::temp_dir().to_string_lossy()).is_some());
    }

    #[test]
    fn folder_parents_are_implied() {
        assert_eq!(parent_of("A/B/C"), "A/B");
        assert_eq!(parent_of("A"), "");
        let mut p = Project::new();
        // an asset can sit in "A/B" without "A" ever being created
        let id = p.add_asset(asset(0, ClipKind::Video, 1.0));
        p.asset_mut(id).unwrap().folder = "A/B".into();
        assert_eq!(folder_tree_names(&p), vec!["A".to_string(), "A/B".to_string()]);
    }

    /// Every file gets a picture: the cached thumbnail when there is one, and a painted object when
    /// there is not — never a font character, which Segoe UI draws as a tofu box.
    #[test]
    fn every_file_kind_has_a_painted_fallback() {
        assert_eq!(fallback_glyph(ext_class("a.mp4")), Glyph::FilmStrip);
        assert_eq!(fallback_glyph(ext_class("a.wav")), Glyph::SpeakerOn);
        assert_eq!(fallback_glyph(ext_class("a.png")), Glyph::Camera);
        assert_eq!(fallback_glyph(ext_class("a.sedit")), Glyph::Layers);
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // no cache at all: every kind still gets its object
                assert_eq!(file_art(ui, &mut None, "a.mov", 32), Art::Icon(Glyph::FilmStrip));
                assert_eq!(file_art(ui, &mut None, "a.mp3", 32), Art::Icon(Glyph::SpeakerOn));
            });
        });
    }

    /// Ctrl adds and removes one item, Shift takes the range from the anchor, a plain click replaces.
    #[test]
    fn click_modifiers_build_the_selection() {
        let rows: Vec<(Pick, egui::Rect)> = (1..=4)
            .map(|i| {
                (Pick::Asset(i), egui::Rect::from_min_size(egui::pos2(0.0, i as f32 * 20.0), egui::vec2(100.0, 18.0)))
            })
            .collect();
        let mut s = LibraryState::default();
        apply_click(&mut s, &rows, &Pick::Asset(1), false, false);
        assert_eq!(s.sel_ids, vec![1]);
        assert_eq!(s.selected, Some(1), "a plain click moves the anchor");
        apply_click(&mut s, &rows, &Pick::Asset(3), true, false);
        assert_eq!(s.sel_ids, vec![1, 3], "ctrl adds");
        apply_click(&mut s, &rows, &Pick::Asset(3), true, false);
        assert_eq!(s.sel_ids, vec![1], "ctrl again removes");
        // the anchor is item 3 now, so the range runs 2..=3
        apply_click(&mut s, &rows, &Pick::Asset(2), false, true);
        assert_eq!(s.sel_ids, vec![2, 3]);
        assert_eq!(s.selected, Some(3), "shift leaves the anchor alone");
        apply_click(&mut s, &rows, &Pick::Asset(4), false, false);
        assert_eq!(s.sel_ids, vec![4], "a plain click replaces the whole set");
    }

    /// A write to `selected` from app.rs (import / reveal) collapses the multi-selection onto it; our
    /// own writes go through `set_anchor` and are left alone.
    #[test]
    fn app_writing_selected_collapses_the_selection() {
        let mut s =
            LibraryState { sel_ids: vec![1, 2, 3], selected: Some(1), seen_selected: Some(1), ..Default::default() };
        external_select(&mut s);
        assert_eq!(s.sel_ids, vec![1, 2, 3], "nothing changed: the set stands");
        s.selected = Some(9); // what app.rs does after open_or_import
        external_select(&mut s);
        assert_eq!(s.sel_ids, vec![9]);
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
                            ui.label("a template name far too long to ever fit into this narrow pane");
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
        let run = |steal: bool, buf: &mut String, out: &mut Option<String>| {
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
                    click_at(&mut input, at);
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
    /// right edge once the path/tag labels were long).
    #[test]
    fn action_button_stays_inside_the_pane() {
        let mut settings = Settings::default();
        settings.templates.push(crate::settings::Template {
            name: "a template with a name that keeps going and going".into(),
            json: "{}".into(),
        });
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for w in [180.0_f32, 260.0, 420.0] {
            let mut state = LibraryState::default();
            let (mut right, mut pane_right) = (f32::NAN, f32::NAN);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let pane = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(w, 600.0));
                    pane_right = pane.right();
                    ui.scope_builder(egui::UiBuilder::new().max_rect(pane), |ui| {
                        let mut resp = LibraryResponse::default();
                        templates_section(ui, &mut state, &mut settings, &palette, &mut resp);
                        right = ui.ctx().data(|d| d.get_temp(egui::Id::new("row_btn_right")).unwrap_or(f32::NAN));
                    });
                });
            });
            assert!(right <= pane_right + 1.0, "the Place button overflows a {w}px pane: right={right}");
        }
    }

    /// Rect of the painted text `label` in a frame's shapes (None when it was never drawn).
    fn text_rect(shapes: &[egui::epaint::ClippedShape], label: &str) -> Option<egui::Rect> {
        shapes.iter().find_map(|c| match &c.shape {
            egui::epaint::Shape::Text(t) if t.galley.text().contains(label) => Some(t.visual_bounding_rect()),
            _ => None,
        })
    }

    /// A press + release at `at`.
    fn click_at(input: &mut egui::RawInput, at: egui::Pos2) {
        input.events.push(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            input.events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
    }

    /// Headless: both sub-tabs lay out with folders, sequences, templates and a selected asset,
    /// reporting nothing when nothing was clicked.
    #[test]
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
            // app.rs's write to `selected` became the whole selection
            assert_eq!(state.sel_ids, vec![project.assets[1].id]);
            // the linked folder is collapsed: nothing was read from disk
            assert!(state.dirs.is_empty(), "collapsed linked folders must not be scanned");
        }
    }

    /// Global browses one level at a time: expanding a node reads that directory and nothing else, and
    /// a single click on a file selects it, previews it, and fills the preview box.
    #[test]
    fn linked_folders_are_read_one_level_per_expansion() {
        let root = std::env::temp_dir().join(format!("se_lib_fs_{}", std::process::id()));
        let sub = root.join("sub");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("a.mp4"), b"x").unwrap();
        std::fs::write(sub.join("b.mp4"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        let (root, sub) = (root.to_string_lossy().into_owned(), sub.to_string_lossy().into_owned());

        let mut project = Project::new();
        project.linked_folders.push(root.clone());
        let mut settings = Settings::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut state = LibraryState { tab: 1, ..Default::default() };
        let mut run = |state: &mut LibraryState, project: &mut Project, click: Option<egui::Pos2>| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 1200.0))),
                ..Default::default()
            };
            if let Some(pos) = click {
                click_at(&mut input, pos);
            }
            let mut got = None;
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    let r = show(ui, state, project, &mut settings, None, &palette, false, &mut undo);
                    got = r.preview;
                });
            });
            (out.shapes, got)
        };

        // collapsed: the linked folder is listed but never read
        run(&mut state, &mut project, None);
        assert!(state.dirs.is_empty(), "a collapsed folder must not be scanned");

        // expanding the root reads exactly one level
        state.flipped.push(dir_key(&root));
        run(&mut state, &mut project, None);
        let listed = |state: &LibraryState, p: &str| {
            state.dirs.iter().find(|(k, _)| k == p).map(|(_, v)| v.clone().unwrap_or_default())
        };
        let top = listed(&state, &root).expect("the root was read");
        assert_eq!(top.len(), 2, "only the sub-folder and the media file: {top:?}");
        assert!(top[0].1 && top[0].0 == sub, "folders come first: {top:?}");
        assert!(listed(&state, &sub).is_none(), "a collapsed sub-folder must not be scanned");

        // and expanding the sub-folder reads the next one
        state.flipped.push(dir_key(&sub));
        let (shapes, _) = run(&mut state, &mut project, None);
        assert_eq!(listed(&state, &sub).map(|v| v.len()), Some(1));

        // a single click on the file selects it and reports it for the preview
        let at = text_rect(&shapes, "a.mp4").expect("the file row was drawn").center();
        let (_, previewed) = run(&mut state, &mut project, Some(at));
        let file = std::path::Path::new(&root).join("a.mp4");
        assert_eq!(previewed.as_deref(), Some(file.as_path()), "a single click previews the file");
        assert_eq!(state.sel_path.as_deref(), file.to_str());
        assert_eq!(state.sel_paths.len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Tags and description are edited in the preview box and nowhere else, and the box only exists
    /// while something is selected.
    #[test]
    fn the_preview_box_follows_the_selection() {
        let mut project = Project::new();
        let id = project.add_asset(asset(0, ClipKind::Video, 5.0));
        let mut settings = Settings::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut state = LibraryState::default();
        let mut drawn = |state: &mut LibraryState, project: &mut Project, label: &str| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 900.0))),
                ..Default::default()
            };
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    show(ui, state, project, &mut settings, None, &palette, false, &mut undo);
                });
            });
            text_rect(&out.shapes, label).is_some()
        };
        assert!(drawn(&mut state, &mut project, "Select a file to preview it"));
        assert!(!drawn(&mut state, &mut project, "description"), "nothing selected: no editors");
        state.selected = Some(id);
        assert!(drawn(&mut state, &mut project, "description"), "selected: the preview box edits it");
        assert!(drawn(&mut state, &mut project, "tags, comma, separated"));
        state.clear_sel();
        assert!(!drawn(&mut state, &mut project, "description"), "clicked away: the box is gone");
    }

    /// A selection offers the batch operations, and each one reports every selected item.
    #[test]
    fn batch_strip_acts_on_the_whole_selection() {
        let mut project = Project::new();
        let a = project.add_asset(asset(0, ClipKind::Video, 5.0));
        let b = project.add_asset(asset(0, ClipKind::Video, 5.0));
        let mut settings = Settings::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut state =
            LibraryState { sel_ids: vec![a, b], selected: Some(a), seen_selected: Some(a), ..Default::default() };
        let mut at = egui::Pos2::ZERO;
        let mut removed = Vec::new();
        for frame in 0..2 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 900.0))),
                ..Default::default()
            };
            if frame == 1 {
                assert_ne!(at, egui::Pos2::ZERO, "the batch strip must show a Remove button");
                click_at(&mut input, at);
            }
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    let r = show(ui, &mut state, &mut project, &mut settings, None, &palette, false, &mut undo);
                    removed = r.remove.clone();
                });
            });
            assert!(text_rect(&out.shapes, "2 selected").is_some(), "the strip counts the selection");
            if let Some(rect) = text_rect(&out.shapes, "Remove from project") {
                at = rect.center();
            }
        }
        assert_eq!(removed, vec![a, b], "Remove takes the whole selection");
    }

    /// The zoom scales both views and is clamped; a fresh state reads as 1.
    #[test]
    fn zoom_scales_and_clamps() {
        let mut project = Project::new();
        let mut settings = Settings::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut state = LibraryState { zoom: 99.0, ..Default::default() };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut undo = |_: &Project| {};
                show(ui, &mut state, &mut project, &mut settings, None, &palette, false, &mut undo);
            });
        });
        assert_eq!(state.zoom, ZOOM_MAX, "an out-of-range zoom is clamped");
        let mut state = LibraryState::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut undo = |_: &Project| {};
                show(ui, &mut state, &mut project, &mut settings, None, &palette, false, &mut undo);
            });
        });
        assert_eq!(state.zoom, 1.0, "a default state reads as 1");
        assert!(TILE * ZOOM_MAX > TILE && ROW_H * ZOOM_MAX > ROW_H);
    }

    /// The reusable sections under the tree: effects in use, saved presets, node graphs and saved
    /// adjustment layers, each handing the right intent back to the app.
    /// Regression: a gallery must drop to a second row instead of running off to the right forever.
    /// `horizontal_wrapped` cannot be trusted here on its own — a drag source's size is not known until
    /// after its contents are drawn, so the wrap test can pass every time and the row never ends.
    #[test]
    fn gallery_wraps_into_a_grid() {
        let ctx = egui::Context::default();
        let mut ys = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let pane = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(260.0, 4000.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(pane), |ui| {
                    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                        tile_grid(ui, 0.0, &(0..8).collect::<Vec<usize>>(), TILE, |ui, &i| {
                            let r = tile(
                                    ui,
                                    egui::Id::new(("g", i)),
                                    DragPayload::Template(String::new()),
                                    false,
                                    "V",
                                    "clip",
                                    egui::Color32::WHITE,
                                    &Palette::new(true, egui::Color32::WHITE),
                                Art::Icon(Glyph::FilmStrip),
                                TILE,
                            );
                            ys.push(r.rect.top());
                        });
                    });
                });
            });
        });
        let rows: std::collections::BTreeSet<i32> = ys.iter().map(|y| y.round() as i32).collect();
        assert!(rows.len() > 1, "8 tiles in a 260px pane must wrap to more than one row: {ys:?}");
    }

    #[test]
    fn reuse_sections_list_what_the_project_holds() {
        let mut project = Project::new();
        project.tracks[0].clips.push(crate::model::Clip::new(7, ClipKind::Video, "v", 0.0, 4.0));
        project.tracks[0].clips[0].effects.push(crate::model::Effect::new(EffectKind::Blur));
        let adj = project.add_adjustment_clip(0.0, 2.0);
        // a real effect first: ensure_graph on an empty stack yields a bare Input->Output pass-through,
        // which uses_graph() (rightly) does not count as a graph worth reusing
        if let Some(c) = project.clip_mut(adj) {
            c.effects.push(crate::model::Effect::new(EffectKind::Blur));
        }
        project.ensure_graph(adj);
        let mut settings = Settings::default();
        settings.touch_recent(r"C:\media\old.mp4");
        settings.effect_presets.push(crate::settings::EffectPreset { name: "Look".into(), json: "[]".into() });
        settings
            .effect_presets
            .push(crate::settings::EffectPreset { name: "Graph".into(), json: "{\"nodes\":[]}".into() });
        settings.templates.push(crate::engine::presets::capture_template("Grade", &project, &[adj]));

        let counts: Vec<(&str, usize)> =
            reuse_sections(&project, &settings).iter().map(|(t, v)| (*t, v.len())).collect();
        assert_eq!(counts, vec![("Effects", 2), ("Node graphs", 2), ("Adjustment layers", 1)]);

        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for view in [0, 1] {
            let mut state = LibraryState { view, ..Default::default() };
            let mut titles = Vec::new();
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 1600.0))),
                    ..Default::default()
                };
                let out = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| panic!("no undo without edits");
                        let r = show(ui, &mut state, &mut project, &mut settings, None, &palette, false, &mut undo);
                        assert!(r.add_effect.is_none() && r.apply_preset.is_none() && r.copy_graph.is_none());
                    });
                });
                titles = ["Sequences", "Templates", "Effects", "Node graphs", "Adjustment layers"]
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
        assert_eq!(resp.add_effect, Some(EffectKind::Blur));
        assert_eq!(resp.apply_preset, Some(1));
        assert_eq!(resp.copy_graph, Some(adj));
        assert_eq!(resp.place_template, vec!["Grade".to_string()]);
        assert_eq!(reuse_face(&Reuse::Effect(EffectKind::Blur), &project, &settings).0, "Blur");
    }
}
