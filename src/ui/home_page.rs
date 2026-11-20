//! ---- ws:home-page ----
//! The Home page: a full page in place of the panes (not a card over them) with two separate sections.
//! **New** is a row of cards - a big + for a blank project, then one card per format preset
//! (`guides::PRESETS`) and per user-saved format (`Settings.project_templates`); a click makes the project.
//! **Projects** is the recent-projects list: an aligned table, a thumbnail, the title, then Rename /
//! Open Folder / Open at the same x on every row; a double-click opens. A project whose file is gone is
//! greyed out with Remove in place of Rename.
//!
//! No `App` in here (`ui::app::home_ctl` is the glue): drawing, the cards' formats, reading a project's
//! size and thumbnail source off the UI thread (`scan`), and the safe rename (`rename_project`).
//! `Settings.home_page` off = none of this; the old start-screen card (`ui::home`) stays as it was.

use crate::model::Project;
use crate::settings::Settings;
use crate::ui::guides::{self, PRESETS};
use crate::ui::tools::Glyph;
use eframe::egui::{self, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind};
use std::path::{Path, PathBuf};

/// A card in the New section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Card {
    /// The + card: `Project::new()` as it is.
    Blank,
    /// `guides::PRESETS[i]`.
    Preset(usize),
    /// `Settings.project_templates[i]` (a format the user saved from the inspector).
    Saved(usize),
}

/// What a card shows.
pub struct CardInfo {
    pub card: Card,
    pub name: String,
    pub glyph: Option<Glyph>,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
}

/// The New section, in order: +, the built-in formats, then the user's own.
pub fn cards(settings: &Settings) -> Vec<CardInfo> {
    let blank = Project::new();
    let mut out = vec![CardInfo { card: Card::Blank, name: "Blank".into(), glyph: None, w: blank.width, h: blank.height, fps: blank.fps }];
    out.extend(PRESETS.iter().enumerate().map(|(i, p)| CardInfo {
        card: Card::Preset(i),
        name: p.name.into(),
        glyph: Some(p.glyph),
        w: p.w,
        h: p.h,
        fps: p.fps,
    }));
    out.extend(settings.project_templates.iter().enumerate().map(|(i, t)| CardInfo {
        card: Card::Saved(i),
        name: t.name.clone(),
        glyph: None,
        w: t.width,
        h: t.height,
        fps: t.fps,
    }));
    out
}

/// The empty project a user-saved format card makes (the built-in cards go through `App::new_project`,
/// the same path as File ▸ New). 9:16 starts with the short-form Max length, like the presets.
pub fn saved_project(settings: &Settings, i: usize) -> Option<Project> {
    let t = settings.project_templates.get(i)?;
    let mut p = Project::new();
    (p.width, p.height, p.fps) = (t.width.max(2), t.height.max(2), crate::model::clean_fps(t.fps));
    if guides::is_short_form(p.width, p.height) {
        p.max_duration = Some(guides::SHORT_FORM_MAX);
    }
    Some(p)
}

/// `1920×1080 · 60 fps`
pub fn format_text(w: u32, h: u32, fps: f64) -> String {
    let fps = if (fps - fps.round()).abs() < 0.005 { format!("{fps:.0}") } else { format!("{fps:.2}") };
    format!("{w}×{h} · {fps} fps")
}

// ------------------------------------------------------------------------------------ recent rows

/// What `scan` read from a recent project's file (off the UI thread).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentInfo {
    pub exists: bool,
    /// Width, height, fps.
    pub size: Option<(u32, u32, f64)>,
    /// The media file and source time a thumbnail is taken from (the first picture on the timeline).
    pub thumb: Option<(String, f64)>,
}

/// Read one recent project: does it exist, its format, and where its thumbnail comes from. Plain JSON
/// (not `Project::load`, which migrates and builds the whole model) - only a few fields are needed.
pub fn scan(path: &Path) -> RecentInfo {
    let Ok(text) = std::fs::read_to_string(path) else {
        return RecentInfo { exists: path.is_file(), ..Default::default() };
    };
    let (size, thumb) = scan_json(&text);
    let thumb = thumb.filter(|(p, _)| Path::new(p).is_file());
    RecentInfo { exists: true, size, thumb }
}

/// The format and thumbnail source in a `.sedit`'s text: the earliest video / image clip on a video track
/// (a moment into it), else the first video / image in the library.
pub fn scan_json(text: &str) -> (Option<(u32, u32, f64)>, Option<(String, f64)>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return (None, None) };
    let num = |k: &str| v.get(k).and_then(serde_json::Value::as_f64);
    let size = match (num("width"), num("height")) {
        (Some(w), Some(h)) => Some((w as u32, h as u32, num("fps").unwrap_or(30.0))),
        _ => None,
    };
    let visual = |k: Option<&serde_json::Value>| matches!(k.and_then(serde_json::Value::as_str), Some("Video" | "Image"));
    let assets = v.get("assets").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
    let asset_path = |id: u64| {
        assets
            .iter()
            .find(|a| a.get("id").and_then(serde_json::Value::as_u64) == Some(id) && visual(a.get("kind")))
            .and_then(|a| a.get("path").and_then(serde_json::Value::as_str))
            .map(str::to_string)
    };
    let f = |c: &serde_json::Value, k: &str| c.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let mut best: Option<(f64, String, f64)> = None; // (timeline start, path, source time)
    for t in v.get("tracks").and_then(serde_json::Value::as_array).into_iter().flatten() {
        if t.get("kind").and_then(serde_json::Value::as_str) != Some("Video") {
            continue;
        }
        for c in t.get("clips").and_then(serde_json::Value::as_array).into_iter().flatten() {
            if !visual(c.get("kind")) || best.as_ref().is_some_and(|b| b.0 <= f(c, "start")) {
                continue;
            }
            let Some(path) = c.get("asset").and_then(serde_json::Value::as_u64).and_then(asset_path) else { continue };
            // a moment in, past a fade from black; no further than the clip's own source window
            let t = f(c, "src_in") + (f(c, "duration") * 0.5).min(1.0);
            best = Some((f(c, "start"), path, t));
        }
    }
    let thumb = best.map(|(_, p, t)| (p, t)).or_else(|| {
        assets.iter().find(|a| visual(a.get("kind"))).and_then(|a| {
            let p = a.get("path")?.as_str()?.to_string();
            Some((p, (f(a, "duration") * 0.5).min(1.0)))
        })
    });
    (size, thumb)
}

/// The title a row shows: the file name without `.sedit`.
pub fn title_of(path: &str) -> String {
    let p = Path::new(path);
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

// ------------------------------------------------------------------------------------------ rename

/// A file name Windows accepts, trimmed - or why not.
pub fn check_name(stem: &str) -> Result<String, String> {
    let s = stem.trim().trim_end_matches('.').trim_end().to_string();
    let s = s.strip_suffix(".sedit").map(str::to_string).unwrap_or(s);
    if s.is_empty() {
        return Err("Enter a name".into());
    }
    if let Some(c) = s.chars().find(|c| r#"<>:"/\|?*"#.contains(*c) || c.is_control()) {
        return Err(format!("A name can't contain {c}"));
    }
    let upper = s.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&base)
        || (base.len() == 4 && (base.starts_with("COM") || base.starts_with("LPT")) && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err(format!("{s} is a name Windows reserves"));
    }
    Ok(s)
}

/// `<project>.lock`, the sidecar a Simple Editor window writes while it has the project open.
pub fn lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

/// The process id of ANOTHER running Simple Editor that has `path` open (its lock names a live
/// simple-editor process that isn't `own_pid`). A stale lock (its window closed) is no one.
pub fn open_elsewhere(path: &Path, own_pid: u32) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(lock_path(path)).ok()?.trim().parse().ok()?;
    if pid == own_pid || pid == 0 {
        return None;
    }
    // "" when the process is gone; the test binary is simple_editor-<hash>.exe
    let exe = crate::engine::wgc::exe_name(pid).to_ascii_lowercase().replace('_', "-");
    exe.starts_with("simple-editor").then_some(pid)
}

/// Rename a project file in its own folder to `<stem>.sedit`: refuses a missing file, a name that's
/// taken, and a project another Simple Editor window has open. The project's `<stem> LUTs` folder
/// moves with it; a lock this window (or a closed one) left is dropped. Returns the new path.
pub fn rename_project(old: &Path, stem: &str, own_pid: u32) -> Result<PathBuf, String> {
    let stem = check_name(stem)?;
    if !old.is_file() {
        return Err("The project file is missing".into());
    }
    let new = old.with_file_name(format!("{stem}.sedit"));
    if new == old {
        return Ok(new);
    }
    // a change of case only is the same file on Windows: not a collision
    let same_file = new.to_string_lossy().eq_ignore_ascii_case(&old.to_string_lossy());
    if !same_file && std::fs::symlink_metadata(&new).is_ok() {
        return Err(format!("{stem} already exists in that folder"));
    }
    if open_elsewhere(old, own_pid).is_some() {
        return Err("It's open in another Simple Editor window".into());
    }
    std::fs::rename(old, &new).map_err(|e| format!("Couldn't rename it: {e}"))?;
    let _ = std::fs::remove_file(lock_path(old));
    let (luts, new_luts) = (crate::engine::lut::project_luts_dir(old), crate::engine::lut::project_luts_dir(&new));
    if luts.is_dir() && !new_luts.exists() {
        let _ = std::fs::rename(luts, new_luts);
    }
    Ok(new)
}

/// The recents list after a rename: the new path takes the old one's place.
pub fn rename_in_recents(list: &mut Vec<String>, old: &str, new: &str) {
    list.retain(|r| r.eq_ignore_ascii_case(old) || !r.eq_ignore_ascii_case(new));
    for r in list.iter_mut() {
        if r.eq_ignore_ascii_case(old) {
            *r = new.to_string();
        }
    }
}

/// ws:parity-io: search in recent files. Every word of `query` must appear (any case) in the file's name or
/// its folder; an empty query matches everything.
pub fn matches_recent(path: &str, query: &str) -> bool {
    let hay = path.replace('\\', "/").to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// Drop one project from the recents list.
pub fn remove_from_recents(list: &mut Vec<String>, path: &str) -> bool {
    let n = list.len();
    list.retain(|r| !r.eq_ignore_ascii_case(path));
    list.len() != n
}

// -------------------------------------------------------------------------------------------- draw

/// What a click asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum PageAction {
    Create(Card),
    Open(String),
    Rename(String),
    OpenFolder(String),
    Remove(String),
    /// The Projects header's Open Project… button (File ▸ Open Project).
    Browse,
    /// Open Media…: a video or sound file opened as a new project (File ▸ Open Video / Media).
    OpenMedia,
    /// The Recover tab: run the offer's choice at this index.
    Recover(usize),
    /// The Recover tab's Dismiss: drop the offer (the autosave stays, File ▸ Restore Autosave… lists it).
    DismissRecover,
}

/// The startup recovery offer, shown as the Recover tab instead of a dialog: title, text, (label, tip) choices.
pub struct RecoverView<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub choices: &'a [(&'a str, &'a str)],
}

/// One row of the Projects list.
pub struct RowView<'a> {
    pub path: &'a str,
    /// None = not read yet (drawn as an existing project with a plain placeholder).
    pub info: Option<&'a RecentInfo>,
    pub thumb: Option<(egui::TextureId, [u32; 2])>,
}

const CARD_W: f32 = 132.0;
const CARD_H: f32 = 124.0;
const PIC_H: f32 = 74.0;
pub const ROW_H: f32 = 64.0;
const THUMB: egui::Vec2 = vec2(96.0, 54.0);
/// The row's buttons, right to left: Open, Open Folder, Rename / Remove.
pub const BUTTONS: [(&str, f32); 3] = [("Rename", 76.0), ("Open Folder", 96.0), ("Open", 64.0)];
const MAX_W: f32 = 980.0;

/// The page, centred in `ui` (the whole space the panes would take). `Some` on a click.
pub fn show(ui: &mut egui::Ui, cards: &[CardInfo], rows: &[RowView], recover: Option<&RecoverView>) -> Option<PageAction> {
    let mut out = None;
    // Projects | Recover (the tab only while there is something to recover)
    let tid = egui::Id::new("home_recover_tab");
    let mut on_recover: bool = recover.is_some() && ui.data(|d| d.get_temp(tid)).unwrap_or(false);
    // ws:parity-io: the Projects search (kept in egui's memory for the visit)
    let qid = egui::Id::new("home_recent_search");
    let mut query: String = ui.data(|d| d.get_temp(qid)).unwrap_or_default();
    egui::ScrollArea::vertical().id_salt("home_page").auto_shrink([false, false]).show(ui, |ui| {
        let full = ui.available_width();
        let w = full.min(MAX_W) - 32.0;
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(((full - w) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.heading("New");
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for c in cards {
                        if card(ui, c).clicked() {
                            out = Some(PageAction::Create(c.card));
                        }
                    }
                });
                ui.add_space(18.0);
                ui.separator();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if recover.is_none() {
                        ui.heading("Projects");
                    } else {
                        let big = |t: &str| egui::RichText::new(t).heading();
                        if ui.selectable_label(!on_recover, big("Projects")).clicked() {
                            on_recover = false;
                        }
                        if ui.selectable_label(on_recover, big("Recover")).on_hover_text("Unsaved work from the last session").clicked() {
                            on_recover = true;
                        }
                        ui.data_mut(|d| d.insert_temp(tid, on_recover));
                    }
                    if on_recover {
                        return;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // right to left: Open Project… sits right-most, Open Media… on its left
                        if ui.button("Open Project…").on_hover_text("Open a project that isn't in the list").clicked() {
                            out = Some(PageAction::Browse);
                        }
                        if ui.button("Open Media…").on_hover_text("Open a video or sound file as a new project").clicked() {
                            out = Some(PageAction::OpenMedia);
                        }
                        if rows.len() > 1 {
                            let r = ui.add(egui::TextEdit::singleline(&mut query).hint_text("Search").desired_width(180.0));
                            if r.on_hover_text("Find a recent project by its name or folder").changed() {
                                ui.data_mut(|d| d.insert_temp(qid, query.clone()));
                            }
                        }
                    });
                });
                ui.add_space(6.0);
                if let Some(r) = recover.filter(|_| on_recover) {
                    ui.strong(r.title);
                    ui.add_space(4.0);
                    ui.label(r.body);
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        for (i, (label, tip)) in r.choices.iter().enumerate() {
                            if ui.button(*label).on_hover_text(*tip).clicked() {
                                out = Some(PageAction::Recover(i));
                            }
                        }
                        let tip = "Keep the autosave without opening it (File ▸ Restore Autosave… lists it)";
                        if ui.button("Dismiss").on_hover_text(tip).clicked() {
                            out = Some(PageAction::DismissRecover);
                        }
                    });
                    ui.add_space(20.0);
                    return;
                }
                if rows.is_empty() {
                    ui.weak("No recent projects");
                } else if !rows.iter().any(|r| matches_recent(r.path, &query)) {
                    ui.weak("No recent projects match");
                }
                for r in rows.iter().filter(|r| matches_recent(r.path, &query)) {
                    if let Some(a) = row(ui, r) {
                        out = Some(a);
                    }
                }
                ui.add_space(20.0);
            });
        });
    });
    out
}

/// A New card: the picture (a big + or the format's shape with its glyph), the name, the size in small text.
fn card(ui: &mut egui::Ui, c: &CardInfo) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::click());
    let v = ui.visuals().clone();
    let p = ui.painter_at(rect);
    let fill = if resp.hovered() { v.widgets.hovered.weak_bg_fill } else { v.faint_bg_color };
    let border = if resp.hovered() { v.selection.stroke.color } else { v.widgets.noninteractive.bg_stroke.color };
    p.rect(rect, 6.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    let pic = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(CARD_W - 16.0, PIC_H));
    p.rect_filled(pic, 4.0, v.extreme_bg_color);
    let ink = v.text_color();
    match c.card {
        Card::Blank => {
            let (cx, r) = (pic.center(), 18.0);
            let s = Stroke::new(4.0, ink);
            p.line_segment([cx - vec2(r, 0.0), cx + vec2(r, 0.0)], s);
            p.line_segment([cx - vec2(0.0, r), cx + vec2(0.0, r)], s);
        }
        _ => {
            // the frame's own shape, fitted in the picture
            let box_ = pic.shrink(10.0);
            let a = c.w.max(1) as f32 / c.h.max(1) as f32;
            let size = if a >= box_.width() / box_.height() { vec2(box_.width(), box_.width() / a) } else { vec2(box_.height() * a, box_.height()) };
            let frame = Rect::from_center_size(box_.center(), size);
            p.rect_stroke(frame, 3.0, Stroke::new(1.5, v.weak_text_color()), StrokeKind::Inside);
            if let Some(ch) = c.glyph.and_then(Glyph::icon) {
                let icons = crate::theme::icons();
                let family = match p.fonts(|f| f.definitions().families.contains_key(&icons)) {
                    true => icons,
                    false => egui::FontFamily::Proportional,
                };
                p.text(frame.center(), Align2::CENTER_CENTER, ch, FontId::new(22.0, family), ink);
            } else if let Some(g) = c.glyph {
                crate::ui::tools::draw_glyph(&p, Rect::from_center_size(frame.center(), vec2(24.0, 22.0)), g, ink);
            }
        }
    }
    let text_w = CARD_W - 16.0;
    let name = Pos2::new(rect.left() + 8.0, pic.bottom() + 8.0);
    let mut job = egui::text::LayoutJob::simple_singleline(c.name.clone(), FontId::proportional(13.0), v.strong_text_color());
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    p.galley(name, ui.painter().layout_job(job), ink);
    let small = match c.card {
        Card::Blank => "New project".to_string(),
        _ => format_text(c.w, c.h, c.fps),
    };
    p.text(name + vec2(0.0, 18.0), Align2::LEFT_TOP, small, FontId::proportional(11.0), v.weak_text_color());
    let mut tip = format!("{} · {}", c.name, format_text(c.w, c.h, c.fps));
    if guides::is_short_form(c.w, c.h) {
        tip.push_str(" · Max length 1:00");
    }
    resp.on_hover_text(tip)
}

/// A Projects row: [thumbnail][title + folder, fills and truncates][Rename / Remove][Open Folder][Open],
/// the buttons at the same x on every row.
fn row(ui: &mut egui::Ui, r: &RowView) -> Option<PageAction> {
    let mut out = None;
    let (rect, body) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let missing = r.info.is_some_and(|i| !i.exists);
    let v = ui.visuals().clone();
    let p = ui.painter_at(rect);
    if body.hovered() {
        p.rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
    }
    let thumb = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.center().y - THUMB.y / 2.0), THUMB);
    p.rect_filled(thumb, 3.0, v.extreme_bg_color);
    match (r.thumb, missing) {
        (Some((tex, [w, h])), false) if w > 0 && h > 0 => {
            let a = w as f32 / h as f32;
            let size = if a >= THUMB.x / THUMB.y { vec2(THUMB.x, THUMB.x / a) } else { vec2(THUMB.y * a, THUMB.y) };
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            p.image(tex, Rect::from_center_size(thumb.center(), size), uv, Color32::WHITE);
        }
        _ => {
            let g = if missing { Glyph::Warning } else { Glyph::Clapperboard };
            crate::ui::tools::draw_glyph(&p, Rect::from_center_size(thumb.center(), vec2(24.0, 22.0)), g, v.weak_text_color());
        }
    }
    // buttons, right-aligned at fixed widths
    let gap = 6.0;
    let mut x = rect.right() - 6.0;
    let mut slots = [Rect::NOTHING; 3];
    for (k, (_, bw)) in BUTTONS.iter().enumerate().rev() {
        slots[k] = Rect::from_min_size(Pos2::new(x - bw, rect.center().y - 12.0), vec2(*bw, 24.0));
        x -= bw + gap;
    }
    // title and folder: what is left, truncated
    let left = thumb.right() + 12.0;
    let text_w = (x - left - 8.0).max(20.0);
    let ink = if missing { v.weak_text_color() } else { v.strong_text_color() };
    let mut job = egui::text::LayoutJob::simple_singleline(title_of(r.path), FontId::proportional(15.0), ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    p.galley(Pos2::new(left, rect.top() + 12.0), ui.painter().layout_job(job), ink);
    let folder = Path::new(r.path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let sub = match r.info {
        Some(i) if !i.exists => "File not found".to_string(),
        Some(RecentInfo { size: Some((w, h, fps)), .. }) => format!("{}  ·  {folder}", format_text(*w, *h, *fps)),
        _ => folder.clone(),
    };
    let mut job = egui::text::LayoutJob::simple_singleline(sub, FontId::proportional(11.5), v.weak_text_color());
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    p.galley(Pos2::new(left, rect.top() + 36.0), ui.painter().layout_job(job), ink);
    let path = r.path.to_string();
    let first = if missing { "Remove" } else { BUTTONS[0].0 };
    let tips = [
        if missing { "Remove from the list (the file is gone)" } else { "Rename the project file" },
        "Show it in File Explorer",
        "Open the project (or double-click the row)",
    ];
    let folder_ok = Path::new(&folder).is_dir();
    for (k, slot) in slots.iter().enumerate() {
        let label = if k == 0 { first } else { BUTTONS[k].0 };
        let enabled = match k {
            1 => folder_ok,
            2 => !missing,
            _ => true,
        };
        // a child ui: `ui.put` would allocate the slot and pull the row cursor back up to the button's bottom
        let centred = egui::Layout::centered_and_justified(egui::Direction::LeftToRight);
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(*slot).layout(centred));
        let b = c.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tips[k]);
        let b = if enabled { b } else { b.on_disabled_hover_text("The file is missing") };
        if b.clicked() {
            out = Some(match (k, missing) {
                (0, true) => PageAction::Remove(path.clone()),
                (0, false) => PageAction::Rename(path.clone()),
                (1, _) => PageAction::OpenFolder(path.clone()),
                _ => PageAction::Open(path.clone()),
            });
        }
    }
    let body = body.on_hover_text(r.path);
    if body.double_clicked() && !missing {
        out = Some(PageAction::Open(path.clone()));
    }
    crate::ui::menu::context(&body, |ui| {
        if crate::ui::menu::row(ui, Some(Glyph::Cross), "Remove from List", "").clicked() {
            out = Some(PageAction::Remove(path.clone()));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn cards_list_blank_then_presets_then_saved_formats() {
        let mut s = Settings::default();
        s.project_templates.push(crate::settings::ProjectTemplate { name: "Mine".into(), width: 720, height: 1280, fps: 30.0 });
        let c = cards(&s);
        assert_eq!(c[0].card, Card::Blank);
        assert_eq!(c.len(), 1 + PRESETS.len() + 1);
        assert_eq!(c[1].card, Card::Preset(0));
        assert_eq!(c.last().unwrap().card, Card::Saved(0));
        assert_eq!(c.last().unwrap().name, "Mine");
        // a saved 9:16 format starts with the short-form Max length, like the built-in Shorts card
        let p = saved_project(&s, 0).unwrap();
        assert_eq!((p.width, p.height, p.fps, p.max_duration), (720, 1280, 30.0, Some(60.0)));
        assert!(saved_project(&s, 5).is_none());
        assert_eq!(format_text(1920, 1080, 60.0), "1920×1080 · 60 fps");
        assert_eq!(format_text(1920, 1080, 29.97), "1920×1080 · 29.97 fps");
    }

    #[test]
    fn scan_finds_the_size_and_the_first_picture() {
        let mut p = Project::new();
        (p.width, p.height) = (1080, 1920);
        let json = p.to_json();
        assert_eq!(scan_json(&json), (Some((1080, 1920, p.fps)), None), "an empty project: no thumbnail");
        let v = serde_json::json!({
            "width": 1920, "height": 1080, "fps": 30.0,
            "assets": [
                {"id": 1, "path": "C:/a.wav", "kind": "Audio", "duration": 9.0},
                {"id": 2, "path": "C:/late.mp4", "kind": "Video", "duration": 9.0},
                {"id": 3, "path": "C:/early.mp4", "kind": "Video", "duration": 9.0},
            ],
            "tracks": [
                {"kind": "Video", "clips": [{"kind": "Video", "asset": 2, "start": 5.0, "duration": 3.0, "src_in": 0.0}]},
                {"kind": "Video", "clips": [{"kind": "Video", "asset": 3, "start": 1.0, "duration": 4.0, "src_in": 2.0}]},
                {"kind": "Audio", "clips": [{"kind": "Audio", "asset": 1, "start": 0.0, "duration": 4.0}]},
            ],
        });
        assert_eq!(scan_json(&v.to_string()), (Some((1920, 1080, 30.0)), Some(("C:/early.mp4".into(), 3.0))));
        assert_eq!(scan_json("not json"), (None, None));
        let d = scratch("scan");
        assert!(!scan(&d.join("gone.sedit")).exists, "a missing file");
        let f = d.join("a.sedit");
        std::fs::write(&f, &json).unwrap();
        let i = scan(&f);
        assert!(i.exists && i.size.is_some() && i.thumb.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_windows_refuses_are_refused() {
        assert_eq!(check_name("  Trip  ").unwrap(), "Trip");
        assert_eq!(check_name("Trip.sedit").unwrap(), "Trip");
        for bad in ["", "  ", "a/b", "a:b", "what?", "CON", "com1", "LPT9.x"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rename_moves_the_file_and_refuses_collisions_missing_and_open_elsewhere() {
        let d = scratch("rename");
        let a = d.join("Alpha.sedit");
        let b = d.join("Beta.sedit");
        std::fs::write(&a, "{}").unwrap();
        std::fs::write(&b, "{\"keep\":1}").unwrap();
        std::fs::create_dir_all(d.join("Alpha LUTs")).unwrap();
        let me = std::process::id();
        // collision: Beta exists - nothing moves
        let e = rename_project(&a, "Beta", me).unwrap_err();
        assert!(e.contains("already exists"), "{e}");
        assert!(a.is_file() && std::fs::read_to_string(&b).unwrap().contains("keep"));
        // missing file
        assert!(rename_project(&d.join("Gone.sedit"), "X", me).unwrap_err().contains("missing"));
        // open in another Simple Editor window: a live simple-editor pid that isn't ours (the test
        // binary itself, with own_pid 0 standing in for "this window")
        std::fs::write(lock_path(&a), me.to_string()).unwrap();
        assert_eq!(open_elsewhere(&a, 0), Some(me));
        assert!(rename_project(&a, "Gamma", 0).unwrap_err().contains("another"));
        // our own lock (or a closed window's) is no one
        assert_eq!(open_elsewhere(&a, me), None);
        std::fs::write(lock_path(&a), "4294967290").unwrap();
        assert_eq!(open_elsewhere(&a, me), None, "a stale lock");
        // a real rename: the file, its LUTs folder, the lock dropped
        let g = rename_project(&a, "Gamma", me).unwrap();
        assert_eq!(g, d.join("Gamma.sedit"));
        assert!(g.is_file() && !a.exists() && !lock_path(&a).exists());
        assert!(d.join("Gamma LUTs").is_dir() && !d.join("Alpha LUTs").exists());
        // a change of case only is not a collision
        let g2 = rename_project(&g, "GAMMA", me).unwrap();
        assert!(g2.is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recents_rename_and_remove_keep_the_order() {
        let mut l: Vec<String> = vec!["C:/a.sedit".into(), "C:/b.sedit".into(), "C:/c.sedit".into()];
        rename_in_recents(&mut l, "C:/b.sedit", "C:/z.sedit");
        assert_eq!(l, ["C:/a.sedit", "C:/z.sedit", "C:/c.sedit"]);
        // renaming onto a path already listed leaves one entry, where the renamed one was
        rename_in_recents(&mut l, "C:/z.sedit", "C:/A.sedit");
        assert_eq!(l, ["C:/A.sedit", "C:/c.sedit"]);
        assert!(remove_from_recents(&mut l, "c:/C.sedit"));
        assert!(!remove_from_recents(&mut l, "C:/nope.sedit"));
        assert_eq!(l, ["C:/A.sedit"]);
    }

    /// ws:parity-io: the recents search matches words in the name or folder, any case, in any order.
    #[test]
    fn recents_search_matches_every_word() {
        let p = r"D:\Clients\Acme\Launch Video v2.sedit";
        assert!(matches_recent(p, ""));
        assert!(matches_recent(p, "launch"));
        assert!(matches_recent(p, "ACME v2"));
        assert!(matches_recent(p, "clients/acme"), "a folder typed with forward slashes");
        assert!(!matches_recent(p, "launch v3"));
    }

    /// The recents list survives a save + load of settings (with the page's own switch).
    #[test]
    fn recent_list_and_home_page_switch_round_trip() {
        let mut s = Settings::default();
        assert!(s.home_page, "on by default");
        s.recent_projects = vec!["C:/x/One.sedit".into(), "D:/Two.sedit".into()];
        s.home_page = false;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.recent_projects, s.recent_projects);
        assert!(!back.home_page);
        // a settings.json from before the Home page has it on
        let old: Settings = serde_json::from_str(r#"{"recent_projects": ["C:/a.sedit"]}"#).unwrap();
        assert!(old.home_page && old.recent_projects == ["C:/a.sedit"]);
    }

    fn run(ctx: &egui::Context, events: Vec<egui::Event>, t: f64, rows: &[RowView]) -> Option<PageAction> {
        let raw = egui::RawInput {
            events,
            time: Some(t),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
            ..Default::default()
        };
        let cards = cards(&Settings::default());
        let mut out = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| out = show(ui, &cards, rows));
        });
        out
    }

    fn click(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// A double-click on a row opens it; a single click doesn't; the buttons sit at the same x on rows
    /// with short and long titles; a missing project offers Remove and never opens.
    #[test]
    fn double_click_opens_and_buttons_line_up() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let here = RecentInfo { exists: true, ..Default::default() };
        let gone = RecentInfo { exists: false, ..Default::default() };
        let long = "C:/x/A very very very very long project title that goes on and on and on and on.sedit";
        let rows = [
            RowView { path: "C:/x/Short.sedit", info: Some(&here), thumb: None },
            RowView { path: long, info: Some(&here), thumb: None },
            RowView { path: "C:/x/Gone.sedit", info: Some(&gone), thumb: None },
        ];
        let _ = run(&ctx, vec![], 0.0, &rows);
        let mut t = 1.0;
        let mut at = |pos: Pos2, double: bool| {
            let mut got = None;
            for _ in 0..if double { 2 } else { 1 } {
                for e in [vec![egui::Event::PointerMoved(pos)], vec![click(pos, true)], vec![click(pos, false)]] {
                    t += 0.05;
                    got = got.or(run(&ctx, e, t, &rows));
                }
            }
            t += 2.0; // well past the double-click window before the next gesture
            got
        };
        // rows start below the cards: find them by double-clicking down the title column
        let x = 300.0;
        let mut row_y: Vec<(f32, String)> = Vec::new();
        let mut y = 0.0;
        while y < 900.0 && row_y.len() < 2 {
            if let Some(PageAction::Open(p)) = at(Pos2::new(x, y), true) {
                // step back to the row's top so every row is measured from the same place
                let mut top = y;
                while at(Pos2::new(x, top - 1.0), true).is_some_and(|a| a == PageAction::Open(p.clone())) {
                    top -= 1.0;
                }
                row_y.push((top, p));
                y = top + ROW_H + 12.0; // past the interact radius egui adds around a widget
                continue;
            }
            y += 8.0;
        }
        assert_eq!(row_y.len(), 2, "both existing rows open on a double-click: {row_y:?}");
        assert!(row_y[1].0 - row_y[0].0 >= ROW_H, "rows never overlap: {row_y:?}");
        assert_eq!(row_y[0].1, "C:/x/Short.sedit");
        assert_eq!(row_y[1].1, long);
        let gone_y = row_y[1].0 + ROW_H + (row_y[1].0 - row_y[0].0 - ROW_H) + ROW_H / 2.0;
        assert_eq!(at(Pos2::new(x, gone_y), true), None, "a missing project never opens");
        // a single click on a row opens nothing
        assert_eq!(at(Pos2::new(x, row_y[0].0), false), None);
        // the right-most x a button answers at, scanning in from the right edge
        let mut edge = |y: f32, want: &dyn Fn(&PageAction) -> bool| {
            let mut x = 1199.0;
            while x > 400.0 {
                if at(Pos2::new(x, y), false).is_some_and(|a| want(&a)) {
                    return x;
                }
                x -= 3.0;
            }
            f32::NAN
        };
        // the buttons sit on the row's middle
        let gap = row_y[1].0 - row_y[0].0 - ROW_H;
        let (b0, b1, gone_y) = (row_y[0].0 + ROW_H / 2.0, row_y[1].0 + ROW_H / 2.0, row_y[1].0 + ROW_H + gap + ROW_H / 2.0);
        let row_y = [(b0, row_y[0].1.clone()), (b1, row_y[1].1.clone())];
        let is_open = |a: &PageAction| matches!(a, PageAction::Open(_));
        let is_first = |a: &PageAction| matches!(a, PageAction::Rename(_) | PageAction::Remove(_));
        let (o0, o1) = (edge(row_y[0].0, &is_open), edge(row_y[1].0, &is_open));
        assert!(o0.is_finite() && o0 == o1, "Open sits at the same x on a short and a long title: {o0} {o1}");
        let (r0, r1, r2) = (edge(row_y[0].0, &is_first), edge(row_y[1].0, &is_first), edge(gone_y, &is_first));
        assert!(r0.is_finite() && r0 == r1 && r1 == r2, "Rename / Remove line up: {r0} {r1} {r2}");
        assert_eq!(at(Pos2::new(r1 - 4.0, row_y[1].0), false), Some(PageAction::Rename(long.into())));
        assert_eq!(at(Pos2::new(r2 - 4.0, gone_y), false), Some(PageAction::Remove("C:/x/Gone.sedit".into())));
        assert_eq!(at(Pos2::new(o0 - 4.0, gone_y), false), None, "Open is off for a missing file");
    }

    #[test]
    fn assert_no_idle_repaint_home_page() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let info = RecentInfo { exists: true, size: Some((1920, 1080, 60.0)), thumb: None };
        let rows = [RowView { path: "C:/x/a.sedit", info: Some(&info), thumb: None }, RowView { path: "C:/x/b.sedit", info: None, thumb: None }];
        for i in 0..30 {
            let _ = run(&ctx, vec![], i as f64 * 0.016, &rows);
        }
        assert!(!ctx.has_requested_repaint(), "idle Home page requested a repaint");
    }
}
