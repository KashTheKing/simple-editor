//! ---- ws:music-burn ----
//! The music panels: the Playlists pane, the Tags section the Inspector shows for library songs and
//! videos, and the Burn to Disc window. Pure drawing: each returns what the user asked for and
//! `ui::app::music_ctl` does it (threads, files, undo).

use crate::engine::disc::{imapi::Drive, imapi::MediaInfo, DiscSize};
use crate::engine::tags::{self, CoverEdit, MediaTags};
use crate::model::Id;
use crate::ui::list_rows::{Ctrl, Moved, RowSpec, Rows};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::PathBuf;

// ------------------------------------------------------------------------------------------------ state

#[derive(Default)]
pub struct MusicState {
    /// The playlist the pane shows.
    pub playlist: Option<Id>,
    /// Rename box: (playlist, text).
    pub rename: Option<(Id, String)>,
    /// "Artist - Title" by asset, filled from tag reads (rows, m3u8, CD-Text, title cards).
    pub titles: BTreeMap<Id, (String, String)>,
    pub tags: TagState,
    pub burn: BurnState,
    /// The Library was clicked after the timeline: the Inspector shows its items' tags.
    pub lib_focus: bool,
    pub seen_lib: Vec<Id>,
    pub seen_sel: Vec<Id>,
    /// Visualizer video options (the pane's footer).
    pub vis_style: String,
    pub vis_cover: bool,
    pub vis_title: bool,
    /// For actions (they get no context of their own).
    pub ctx: Option<egui::Context>,
}

impl MusicState {
    pub fn new() -> Self {
        MusicState { vis_cover: true, vis_title: true, ..Default::default() }
    }
}

/// The Inspector's tag editor for the library selection.
#[derive(Default)]
pub struct TagState {
    /// The assets these tags are for (the library selection when they were read).
    pub ids: Vec<Id>,
    pub loading: bool,
    /// Tags read, by asset (an error: that file can't be read).
    pub loaded: Vec<(Id, Result<MediaTags, String>)>,
    /// Fields the user changed: key -> new value ("" clears).
    pub draft: BTreeMap<String, String>,
    pub cover: CoverEdit,
    /// The picture shown: the first file's cover, or the one picked.
    pub preview: Option<(PathBuf, egui::TextureHandle)>,
    pub preview_for: Option<PathBuf>,
    pub saving: bool,
    pub status: String,
}

impl TagState {
    pub fn dirty(&self) -> bool {
        !self.draft.is_empty() || self.cover != CoverEdit::Keep
    }
    pub fn reset(&mut self) {
        self.draft.clear();
        self.cover = CoverEdit::Keep;
        self.status.clear();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiscKind {
    #[default]
    Audio,
    Data,
}

pub struct BurnState {
    pub open: bool,
    pub kind: DiscKind,
    pub playlist: Option<Id>,
    /// Data disc: more files and folders.
    pub extra: Vec<PathBuf>,
    pub size: DiscSize,
    pub title: String,
    pub performer: String,
    pub gap: f64,
    /// None = write an image file; Some(i) = drive i.
    pub drive: Option<usize>,
    pub drives: Vec<Drive>,
    pub drives_loading: bool,
    pub drives_error: String,
    pub media: Option<Result<MediaInfo, String>>,
    pub media_loading: bool,
    pub image: PathBuf,
    pub speed: i32,
    pub verify: bool,
    pub eject: bool,
    pub test_only: bool,
    pub running: Option<std::sync::Arc<crate::engine::export::Progress>>,
    pub result: String,
}

impl Default for BurnState {
    fn default() -> Self {
        BurnState {
            open: false,
            kind: DiscKind::Audio,
            playlist: None,
            extra: Vec::new(),
            size: DiscSize::Cd80,
            title: String::new(),
            performer: String::new(),
            gap: 2.0,
            drive: None,
            drives: Vec::new(),
            drives_loading: false,
            drives_error: String::new(),
            media: None,
            media_loading: false,
            image: PathBuf::new(),
            speed: 0,
            verify: true,
            eject: true,
            test_only: false,
            running: None,
            result: String::new(),
        }
    }
}

// ------------------------------------------------------------------------------------------------ playlists

/// A row of the playlist: label and length.
pub struct PlRow {
    pub label: String,
    pub secs: f64,
}

pub enum PlAct {
    New,
    Pick(Id),
    Rename(Id, String),
    Delete(Id),
    Add { assets: Vec<Id>, at: Option<usize> },
    Remove(usize),
    Move { from: usize, to: usize },
    Spacing { gap: Option<f64>, crossfade: Option<f64> },
    Play(usize),
    ToTimeline,
    Visualizer,
    ExportM3u8,
    Burn,
    OpenLibrary,
    AddSelection,
}

const ROW_CTRLS: &[Ctrl] = &[Ctrl { glyph: "▶", hint: "Play from this song" }, Ctrl { glyph: "✕", hint: "Remove from the playlist" }];

/// The Playlists pane. `lists`: (id, name) of every playlist; `rows`: the shown playlist's items.
#[allow(clippy::too_many_arguments)]
pub fn playlists_pane(
    ui: &mut egui::Ui,
    st: &mut MusicState,
    lists: &[(Id, String)],
    rows: &[PlRow],
    total: f64,
    spacing: (f64, f64),
    styles: &[String],
    lib_selected: usize,
) -> Option<PlAct> {
    let mut act = None;
    let cur = st.playlist.filter(|id| lists.iter().any(|(l, _)| l == id));
    ui.horizontal(|ui| {
        let name = cur.and_then(|id| lists.iter().find(|(l, _)| *l == id)).map_or("No playlist", |(_, n)| n.as_str());
        egui::ComboBox::from_id_salt("pl_pick").width(180.0).selected_text(name).show_ui(ui, |ui| {
            for (id, n) in lists {
                if ui.selectable_label(cur == Some(*id), n).clicked() {
                    act = Some(PlAct::Pick(*id));
                }
            }
        });
        if ui.button("New").on_hover_text("A new, empty playlist").clicked() {
            act = Some(PlAct::New);
        }
        if let Some(id) = cur {
            if ui.button("Rename").clicked() {
                let n = lists.iter().find(|(l, _)| *l == id).map(|(_, n)| n.clone()).unwrap_or_default();
                st.rename = Some((id, n));
            }
            if ui.button("Delete").on_hover_text("Delete this playlist (the songs stay in the Library)").clicked() {
                act = Some(PlAct::Delete(id));
            }
        }
    });
    if let Some((id, text)) = st.rename.as_mut() {
        let id = *id;
        let mut done = None;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
            r.request_focus();
            if ui.button("OK").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        match done {
            Some(true) => {
                act = Some(PlAct::Rename(id, text.clone()));
                st.rename = None;
            }
            Some(false) => st.rename = None,
            None => {}
        }
    }
    let Some(_pl) = cur else {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label("No playlists yet.");
            if ui.button("New playlist").clicked() {
                act = Some(PlAct::New);
            }
        });
        return act;
    };
    ui.separator();
    // the list: a drop target for Library drags
    let avail = (ui.available_height() - 96.0).max(60.0);
    let frame = egui::Frame::NONE.inner_margin(2.0);
    let inner = frame.show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("pl_rows").max_height(avail).auto_shrink([false, true]).show(ui, |ui| {
            if rows.is_empty() {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label("Drag songs here from the Library.");
                    ui.horizontal(|ui| {
                        if ui.button("Open Library").clicked() {
                            act = Some(PlAct::OpenLibrary);
                        }
                        if lib_selected > 0 && ui.button(format!("Add {lib_selected} selected")).clicked() {
                            act = Some(PlAct::AddSelection);
                        }
                    });
                });
                return;
            }
            let mut list = Rows::new(ui, "playlist_rows", ROW_CTRLS, &[52.0], false);
            for (i, r) in rows.iter().enumerate() {
                let len = crate::engine::disc::clock(r.secs);
                let num = format!("{}. {}", i + 1, r.label);
                let resp = list.row(ui, i, &RowSpec { key: i as u64, label: &num, values: &[len.as_str()], ..Default::default() });
                match resp.ctrl {
                    Some(0) => act = Some(PlAct::Play(i)),
                    Some(1) => act = Some(PlAct::Remove(i)),
                    _ => {}
                }
                if resp.body.double_clicked() {
                    act = Some(PlAct::Play(i));
                }
            }
            if let Some(m) = list.finish(ui) {
                let n = rows.len();
                let mv = match m {
                    Moved::Slot { from, slot, .. } => Some((from, if slot > from { slot - 1 } else { slot })),
                    Moved::Step { from, delta } => Some((from, (from as i64 + delta as i64).clamp(0, n as i64 - 1) as usize)),
                    Moved::Indent { .. } => None,
                };
                if let Some((from, to)) = mv.filter(|(f, t)| f != t) {
                    act = Some(PlAct::Move { from, to });
                }
            }
        });
    });
    // a Library drag lands on the list (the playlist's own row drags are list_rows', not a payload)
    let resp = &inner.response;
    if resp.dnd_hover_payload::<crate::ui::DragPayload>().is_some_and(|p| matches!(*p, crate::ui::DragPayload::Asset(_) | crate::ui::DragPayload::Assets(_))) {
        ui.painter().rect_stroke(resp.rect, 4.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
    }
    if let Some(p) = resp.dnd_release_payload::<crate::ui::DragPayload>() {
        match &*p {
            crate::ui::DragPayload::Asset(id) => act = Some(PlAct::Add { assets: vec![*id], at: None }),
            crate::ui::DragPayload::Assets(ids) => act = Some(PlAct::Add { assets: ids.clone(), at: None }),
            _ => {}
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong(format!("{} songs · {}", rows.len(), crate::engine::disc::clock(total)));
        if lib_selected > 0 && ui.button(format!("Add {lib_selected} selected")).on_hover_text("Add the songs selected in the Library").clicked() {
            act = Some(PlAct::AddSelection);
        }
    });
    egui::Grid::new("pl_spacing").num_columns(2).show(ui, |ui| {
        let (mut gap, mut xf) = spacing;
        ui.label("Gap");
        if ui.add(egui::DragValue::new(&mut gap).range(0.0..=30.0).speed(0.1).suffix(" s")).on_hover_text("Silence between songs on the timeline and on an audio CD").changed() {
            act = Some(PlAct::Spacing { gap: Some(gap), crossfade: None });
        }
        ui.end_row();
        ui.label("Crossfade");
        if ui.add(egui::DragValue::new(&mut xf).range(0.0..=30.0).speed(0.1).suffix(" s")).on_hover_text("Songs overlap and fade into each other on the timeline (0 = off)").changed() {
            act = Some(PlAct::Spacing { gap: None, crossfade: Some(xf) });
        }
        ui.end_row();
        ui.label("Visualizer");
        let sel = if st.vis_style.is_empty() { styles.first().cloned().unwrap_or_default() } else { st.vis_style.clone() };
        egui::ComboBox::from_id_salt("pl_vis_style").width(150.0).selected_text(&sel).show_ui(ui, |ui| {
            for s in styles {
                if ui.selectable_label(*s == sel, s).clicked() {
                    st.vis_style = s.clone();
                }
            }
        });
        ui.end_row();
        ui.label("");
        ui.horizontal(|ui| {
            ui.checkbox(&mut st.vis_cover, "Cover art");
            ui.checkbox(&mut st.vis_title, "Title card");
        });
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        let some = !rows.is_empty();
        if ui.add_enabled(some, egui::Button::new("▶ Play")).on_hover_text("Play the playlist in the Source monitor").clicked() {
            act = Some(PlAct::Play(0));
        }
        if ui.add_enabled(some, egui::Button::new("To timeline")).on_hover_text("Put the songs on the timeline at the playhead, back to back").clicked() {
            act = Some(PlAct::ToTimeline);
        }
        if ui.add_enabled(some, egui::Button::new("Make visualizer video")).on_hover_text("The songs on the timeline with a visualizer, their cover art and a title card").clicked() {
            act = Some(PlAct::Visualizer);
        }
        if ui.add_enabled(some, egui::Button::new("Export .m3u8")).clicked() {
            act = Some(PlAct::ExportM3u8);
        }
        if ui.add_enabled(some, egui::Button::new("Burn…")).on_hover_text("Burn to Disc: an audio CD or a data disc").clicked() {
            act = Some(PlAct::Burn);
        }
    });
    act
}

// ------------------------------------------------------------------------------------------------ tags

pub enum TagAct {
    Save,
    Revert,
    CoverFile,
    CoverFrame,
    CoverRemove,
}

/// What kind of files are selected (decides the fields shown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kinds {
    pub audio: bool,
    pub video: bool,
}

/// The Tags section. `n` = files selected; `frame_from`: a video to take a cover frame from, if any.
pub fn tags_section(ui: &mut egui::Ui, st: &mut TagState, kinds: Kinds, n: usize, can_frame: bool) -> Option<TagAct> {
    let mut act = None;
    ui.horizontal(|ui| {
        ui.heading("Tags");
        ui.weak(if n == 1 { "1 file".to_string() } else { format!("{n} files") });
    });
    if st.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Reading tags");
        });
        return None;
    }
    let ok: Vec<&MediaTags> = st.loaded.iter().filter_map(|(_, r)| r.as_ref().ok()).collect();
    for (_, r) in &st.loaded {
        if let Err(e) = r {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
    }
    if ok.is_empty() {
        return None;
    }
    let merged = tags::merged(&ok);
    // cover
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let size = egui::vec2(96.0, 96.0);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let covers = ok.iter().filter(|t| t.cover.is_some()).count();
        match (&st.cover, &st.preview) {
            (CoverEdit::Remove, _) => {
                ui.painter().rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "No cover", egui::FontId::proportional(12.0), ui.visuals().weak_text_color());
            }
            (_, Some((_, tex))) => {
                let s = tex.size_vec2();
                let k = (size.x / s.x).min(size.y / s.y);
                let r = egui::Rect::from_center_size(rect.center(), s * k);
                ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            _ => {
                ui.painter().rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                let t = if covers > 0 && covers < ok.len() { "Mixed" } else { "No cover" };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(12.0), ui.visuals().weak_text_color());
            }
        }
        ui.vertical(|ui| {
            ui.label(if kinds.video && !kinds.audio { "Thumbnail" } else { "Cover art" });
            if ui.button("From file…").clicked() {
                act = Some(TagAct::CoverFile);
            }
            if ui
                .add_enabled(can_frame, egui::Button::new("From video frame"))
                .on_hover_text("The frame under the Source monitor's playhead (open the video there first)")
                .on_disabled_hover_text("Open a video in the Source monitor and park its playhead on the frame you want")
                .clicked()
            {
                act = Some(TagAct::CoverFrame);
            }
            if ui.button("Remove").clicked() {
                act = Some(TagAct::CoverRemove);
            }
        });
    });
    ui.add_space(4.0);
    egui::Grid::new("tag_fields").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for f in tags::FIELDS.iter().filter(|f| (kinds.audio && f.audio) || (kinds.video && f.video)) {
            ui.label(f.label);
            let current = merged.get(f.key).cloned().flatten();
            let edited = st.draft.contains_key(f.key);
            let mut text = st.draft.get(f.key).cloned().or_else(|| current.clone()).unwrap_or_default();
            let hint = if current.is_none() && !edited { "Mixed" } else { "" };
            let w = ui.available_width().max(120.0);
            let r = if f.multiline {
                ui.add(egui::TextEdit::multiline(&mut text).hint_text(hint).desired_rows(if f.key == "lyrics" { 4 } else { 2 }).desired_width(w))
            } else if f.key == "rating" {
                rating(ui, &mut text)
            } else {
                ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(w))
            };
            if r.changed() {
                if current.as_deref() == Some(text.as_str()) {
                    st.draft.remove(f.key);
                } else {
                    st.draft.insert(f.key.to_string(), text);
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let dirty = st.dirty();
        if ui.add_enabled(dirty && !st.saving, egui::Button::new("Save")).on_hover_text("Write the tags into the files (no re-encode; a .bak is kept until it worked)").clicked() {
            act = Some(TagAct::Save);
        }
        if ui.add_enabled(dirty && !st.saving, egui::Button::new("Revert")).clicked() {
            act = Some(TagAct::Revert);
        }
        if st.saving {
            ui.spinner();
        }
    });
    if !st.status.is_empty() {
        ui.label(&st.status);
    }
    act
}

/// 0..5 stars stored as the number ("" = none).
fn rating(ui: &mut egui::Ui, text: &mut String) -> egui::Response {
    let cur: u8 = text.trim().parse().unwrap_or(0);
    let mut r = ui.horizontal(|ui| {
        let mut changed = None;
        for k in 1..=5u8 {
            let star = if k <= cur { "★" } else { "☆" };
            if ui.add(egui::Button::new(star).frame(false)).clicked() {
                changed = Some(if k == cur { 0 } else { k });
            }
        }
        changed
    });
    if let Some(k) = r.inner {
        *text = if k == 0 { String::new() } else { k.to_string() };
        r.response.mark_changed();
    }
    r.response
}

// ------------------------------------------------------------------------------------------------ burn

pub enum BurnAct {
    RefreshDrives,
    CheckDisc,
    Erase,
    Eject,
    ChooseImage,
    AddFiles,
    AddFolder,
    RemoveExtra(usize),
    Start,
    Cancel,
    Close,
    OpenJobs,
}

/// What the dialog needs to say about the content: total length (audio) / bytes (data), a problem.
pub struct BurnSummary {
    pub audio_secs: f64,
    pub data_bytes: u64,
    pub problem: Option<String>,
}

pub fn burn_window(ctx: &egui::Context, st: &mut BurnState, lists: &[(Id, String)], sum: &BurnSummary) -> Option<BurnAct> {
    if !st.open {
        return None;
    }
    let mut open = true;
    let mut act = None;
    egui::Window::new("Burn to Disc")
        .id(egui::Id::new("burn_disc"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let busy = st.running.is_some();
            ui.add_enabled_ui(!busy, |ui| {
                egui::Grid::new("burn_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Disc");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut st.kind, DiscKind::Audio, "Audio CD");
                        ui.radio_value(&mut st.kind, DiscKind::Data, "Data disc");
                        ui.add_enabled(false, egui::RadioButton::new(false, "Video DVD")).on_disabled_hover_text(
                            "Not available: a DVD player needs an authored VIDEO_TS disc, which Simple Editor can't make. Put the video files on a data disc instead.",
                        );
                    });
                    ui.end_row();
                    ui.label("Playlist");
                    let name = st.playlist.and_then(|id| lists.iter().find(|(l, _)| *l == id)).map_or("None", |(_, n)| n.as_str());
                    egui::ComboBox::from_id_salt("burn_pl").width(220.0).selected_text(name).show_ui(ui, |ui| {
                        if st.kind == DiscKind::Data && ui.selectable_label(st.playlist.is_none(), "None").clicked() {
                            st.playlist = None;
                        }
                        for (id, n) in lists {
                            if ui.selectable_label(st.playlist == Some(*id), n).clicked() {
                                st.playlist = Some(*id);
                            }
                        }
                    });
                    ui.end_row();
                    if st.kind == DiscKind::Data {
                        ui.label("Also");
                        ui.vertical(|ui| {
                            let mut remove = None;
                            for (i, p) in st.extra.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                                    ui.add_sized([280.0, 18.0], egui::Label::new(name).truncate()).on_hover_text(p.display().to_string());
                                    if ui.small_button("✕").on_hover_text("Leave it off the disc").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = remove {
                                act = Some(BurnAct::RemoveExtra(i));
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Add files…").clicked() {
                                    act = Some(BurnAct::AddFiles);
                                }
                                if ui.button("Add folder…").clicked() {
                                    act = Some(BurnAct::AddFolder);
                                }
                            });
                        });
                        ui.end_row();
                    } else {
                        ui.label("Album");
                        ui.add(egui::TextEdit::singleline(&mut st.title).hint_text("CD-Text title").desired_width(220.0));
                        ui.end_row();
                        ui.label("Artist");
                        ui.add(egui::TextEdit::singleline(&mut st.performer).hint_text("CD-Text artist").desired_width(220.0));
                        ui.end_row();
                        ui.label("Gap");
                        ui.add(egui::DragValue::new(&mut st.gap).range(0.0..=10.0).speed(0.1).suffix(" s"))
                            .on_hover_text("Silence between tracks. A drive burn always leaves at least 2 s (track-at-once).");
                        ui.end_row();
                    }
                    ui.label("Size");
                    egui::ComboBox::from_id_salt("burn_size").width(220.0).selected_text(st.size.label()).show_ui(ui, |ui| {
                        for s in DiscSize::ALL {
                            if st.kind == DiscKind::Audio && !s.is_cd() {
                                continue;
                            }
                            ui.selectable_value(&mut st.size, s, s.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Uses");
                    let (used, have, text) = if st.kind == DiscKind::Audio {
                        let have = st.size.audio_sectors() as f64 / 75.0;
                        (sum.audio_secs, have, format!("{} of {}", crate::engine::disc::clock(sum.audio_secs), crate::engine::disc::clock(have)))
                    } else {
                        let have = (st.size.data_blocks() * 2048) as f64;
                        (sum.data_bytes as f64, have, format!("{} MB of {} MB", sum.data_bytes / 1_000_000, (have / 1e6) as u64))
                    };
                    let frac = if have > 0.0 { (used / have) as f32 } else { 0.0 };
                    let mut bar = egui::ProgressBar::new(frac.min(1.0)).text(text).desired_width(220.0);
                    if frac > 1.0 {
                        bar = bar.fill(ui.visuals().error_fg_color);
                    }
                    ui.add(bar);
                    ui.end_row();
                    ui.label("Burn to");
                    let target = match st.drive.and_then(|i| st.drives.get(i)) {
                        Some(d) => d.name.clone(),
                        None => "Disc image file".into(),
                    };
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("burn_target").width(220.0).selected_text(target).show_ui(ui, |ui| {
                            if ui.selectable_label(st.drive.is_none(), "Disc image file").on_hover_text("No drive needed: a .iso (data) or .bin + .cue (audio CD) any burning program can write later").clicked() {
                                st.drive = None;
                                st.media = None;
                            }
                            for (i, d) in st.drives.iter().enumerate() {
                                if ui.selectable_label(st.drive == Some(i), &d.name).clicked() {
                                    st.drive = Some(i);
                                    st.media = None;
                                    act = Some(BurnAct::CheckDisc);
                                }
                            }
                        });
                        if st.drives_loading {
                            ui.spinner();
                        } else if ui.small_button("⟳").on_hover_text("Look for drives again").clicked() {
                            act = Some(BurnAct::RefreshDrives);
                        }
                    });
                    ui.end_row();
                    if st.drive.is_none() {
                        ui.label("File");
                        ui.horizontal(|ui| {
                            let shown = if st.image.as_os_str().is_empty() { "Choose where to save".to_string() } else { st.image.display().to_string() };
                            ui.add_sized([220.0, 18.0], egui::Label::new(shown).truncate());
                            if ui.button("Choose…").clicked() {
                                act = Some(BurnAct::ChooseImage);
                            }
                        });
                        ui.end_row();
                    } else {
                        ui.label("In the drive");
                        ui.horizontal(|ui| {
                            match &st.media {
                                _ if st.media_loading => {
                                    ui.spinner();
                                }
                                Some(Ok(m)) => {
                                    ui.label(m.describe());
                                }
                                Some(Err(e)) => {
                                    ui.colored_label(ui.visuals().warn_fg_color, e);
                                }
                                None => {
                                    ui.weak("Not checked");
                                }
                            }
                            if ui.small_button("Check").clicked() {
                                act = Some(BurnAct::CheckDisc);
                            }
                            let rw = matches!(&st.media, Some(Ok(m)) if m.rewritable && !m.blank);
                            if ui.add_enabled(rw, egui::Button::new("Erase").small()).on_hover_text("Quick-erase the rewritable disc").clicked() {
                                act = Some(BurnAct::Erase);
                            }
                            if ui.small_button("Eject").clicked() {
                                act = Some(BurnAct::Eject);
                            }
                        });
                        ui.end_row();
                        ui.label("Speed");
                        let speeds: Vec<i32> = match &st.media {
                            Some(Ok(m)) => m.speeds.clone(),
                            _ => Vec::new(),
                        };
                        let x = |s: i32| if st.size.is_cd() { format!("{}x", s / 75) } else { format!("{:.1}x", s as f64 / 675.0) };
                        let shown = if st.speed <= 0 { "Fastest".to_string() } else { x(st.speed) };
                        egui::ComboBox::from_id_salt("burn_speed").width(120.0).selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(&mut st.speed, 0, "Fastest");
                            for s in speeds {
                                ui.selectable_value(&mut st.speed, s, x(s));
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.add_enabled(st.kind == DiscKind::Data, egui::Checkbox::new(&mut st.verify, "Verify after burning"));
                            ui.checkbox(&mut st.eject, "Eject when done");
                            ui.checkbox(&mut st.test_only, "Test only (check everything, write nothing)");
                        });
                        ui.end_row();
                    }
                });
            });
            if let Some(p) = &sum.problem {
                ui.colored_label(ui.visuals().error_fg_color, p);
            }
            ui.add_space(6.0);
            if let Some(prog) = &st.running {
                ui.add(egui::ProgressBar::new(prog.fraction()).text(prog.status()).desired_width(ui.available_width()));
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        act = Some(BurnAct::Cancel);
                    }
                    if ui.button("Jobs").on_hover_text("Open the Jobs panel").clicked() {
                        act = Some(BurnAct::OpenJobs);
                    }
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            } else {
                if !st.result.is_empty() {
                    ui.label(&st.result);
                }
                ui.horizontal(|ui| {
                    let label = match (st.drive.is_some(), st.test_only) {
                        (false, _) => "Write image",
                        (true, true) => "Test",
                        (true, false) => "Burn",
                    };
                    if ui.add_enabled(sum.problem.is_none(), egui::Button::new(label)).clicked() {
                        act = Some(BurnAct::Start);
                    }
                    if ui.button("Close").clicked() {
                        act = Some(BurnAct::Close);
                    }
                });
            }
        });
    if !open {
        act = Some(if st.running.is_some() { BurnAct::Close } else { BurnAct::Close });
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &egui::Context, f: &mut dyn FnMut(&egui::Context)) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| f(ctx));
    }

    fn rows() -> Vec<PlRow> {
        vec![PlRow { label: "Short".into(), secs: 61.0 }, PlRow { label: "A much longer song title · An Artist With A Long Name".into(), secs: 245.0 }]
    }

    #[test]
    fn assert_no_idle_repaint_playlists_pane() {
        let ctx = egui::Context::default();
        let mut st = MusicState::new();
        st.playlist = Some(7);
        let lists = vec![(7, "Road trip".to_string())];
        let styles = vec!["Neon Orb".to_string()];
        let rows = rows();
        for _ in 0..30 {
            run(&ctx, &mut |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = playlists_pane(ui, &mut st, &lists, &rows, 306.0, (2.0, 0.0), &styles, 0);
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Playlists pane must not repaint");
    }

    #[test]
    fn assert_no_idle_repaint_tags_section() {
        let ctx = egui::Context::default();
        let mut st = TagState::default();
        let mut t = MediaTags::default();
        t.values.insert("title".into(), "Song".into());
        st.loaded = vec![(1, Ok(t.clone())), (2, Ok(MediaTags::default()))];
        for _ in 0..30 {
            run(&ctx, &mut |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = tags_section(ui, &mut st, Kinds { audio: true, video: true }, 2, false);
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle Tags must not repaint");
    }

    #[test]
    fn assert_no_idle_repaint_burn_window() {
        let ctx = egui::Context::default();
        let mut st = BurnState { open: true, ..Default::default() };
        let lists = vec![(7, "Road trip".to_string())];
        let sum = BurnSummary { audio_secs: 1800.0, data_bytes: 0, problem: None };
        for kind in [DiscKind::Audio, DiscKind::Data] {
            st.kind = kind;
            for _ in 0..30 {
                run(&ctx, &mut |ctx| {
                    let _ = burn_window(ctx, &mut st, &lists, &sum);
                });
            }
            assert!(!ctx.has_requested_repaint(), "an idle Burn window must not repaint ({kind:?})");
        }
    }

    /// Mixed values show as an empty box with a "Mixed" hint; typing makes a draft, typing the old
    /// value back removes it again.
    #[test]
    fn mixed_fields_and_drafts() {
        let mut a = MediaTags::default();
        a.values.insert("artist".into(), "X".into());
        a.values.insert("album".into(), "One".into());
        let mut b = a.clone();
        b.values.insert("album".into(), "Two".into());
        let m = tags::merged(&[&a, &b]);
        assert_eq!(m["album"], None);
        let mut st = TagState::default();
        st.draft.insert("album".into(), "Both".into());
        assert!(st.dirty());
        st.reset();
        assert!(!st.dirty());
        st.cover = CoverEdit::Remove;
        assert!(st.dirty());
    }

    /// The playlist rows are a table: the controls of a short row and a long row are at the same x.
    #[test]
    fn playlist_row_controls_line_up() {
        let ctx = egui::Context::default();
        let mut xs = Vec::new();
        run(&ctx, &mut |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut list = Rows::new(ui, "t", ROW_CTRLS, &[52.0], false);
                for (i, r) in rows().iter().enumerate() {
                    xs.push(list.row(ui, i, &RowSpec { key: i as u64, label: &r.label, values: &["1:00"], ..Default::default() }).ctrl_x);
                }
                let _ = list.finish(ui);
            });
        });
        assert_eq!(xs.len(), 2);
        assert_eq!(xs[0], xs[1]);
    }
}
