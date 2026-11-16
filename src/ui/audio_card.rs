//! ---- ws:audio-visualizer ----
//! The "now playing" card a monitor shows for sound with no picture and no visualizer: the file's cover
//! art (its embedded picture, from the thumbnail cache) or the app's music-note icon, and the song's
//! title / artist from its tags. Tags are read once per file by a background `ffprobe` and cached; the
//! card itself is a still (no animation, no repaint requests).

use crate::model::{ClipKind, Id, Project, TrackKind};
use crate::ui::tools::Glyph;
use eframe::egui;
use std::collections::HashMap;
use std::sync::Mutex;

/// What a file's metadata holds: "Title - Artist" and its embedded picture (RGBA, size).
#[derive(Clone, Default)]
struct Meta {
    title: Option<String>,
    cover: Option<std::sync::Arc<crate::media::Frame>>,
}

/// path -> its metadata (None: still reading).
static TAGS: Mutex<Option<HashMap<String, Option<Meta>>>> = Mutex::new(None);

/// The file's metadata if read, else start reading it (one background thread per file) and answer None.
fn meta(ctx: &egui::Context, path: &str) -> Option<Meta> {
    let mut g = TAGS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(m) = map.get(path) {
        return m.clone();
    }
    map.insert(path.to_string(), None);
    let (path, ctx) = (path.to_string(), ctx.clone());
    std::thread::spawn(move || {
        let m = Meta { title: read_tags(&path), cover: read_cover(&path).map(std::sync::Arc::new) };
        if let Some(map) = TAGS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            map.insert(path, Some(m));
        }
        ctx.request_repaint();
    });
    None
}

/// The song's display name: its title (and artist) tags when the file has them, else the file name.
/// The first call for a path starts one background `ffprobe` and answers with the file name; `ctx`
/// repaints once when the tags land.
pub fn title(ctx: &egui::Context, path: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned());
    meta(ctx, path).and_then(|m| m.title).unwrap_or(stem)
}

/// The file's embedded cover picture (`attached_pic`) as a texture, uploaded once per file and kept in
/// egui's memory. None while reading or when the file has none.
pub fn cover(ctx: &egui::Context, path: &str) -> Option<(egui::TextureId, [u32; 2])> {
    let id = egui::Id::new(("audio_cover", path));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some((t.id(), [t.size()[0] as u32, t.size()[1] as u32]));
    }
    let f = meta(ctx, path)?.cover?;
    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
    let t = ctx.load_texture(format!("audio-cover {path}"), img, egui::TextureOptions::LINEAR);
    let out = (t.id(), [f.width, f.height]);
    ctx.data_mut(|d| d.insert_temp(id, t));
    Some(out)
}

/// The first attached picture, at most 512 px wide, decoded from ffmpeg's PNG output.
fn read_cover(path: &str) -> Option<crate::media::Frame> {
    let exe = crate::media::ffpipe::ffmpeg_exe()?;
    let out = crate::media::ffpipe::command(&exe)
        .args(["-v", "error", "-i", path, "-map", "0:v:0", "-frames:v", "1", "-vf", "scale='min(512,iw)':-2"])
        .args(["-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if out.stdout.is_empty() {
        return None;
    }
    crate::engine::html::decode_png(&out.stdout).ok()
}

/// "Title - Artist" / "Title" from the container's tags (`ffprobe -show_entries format_tags`).
fn read_tags(path: &str) -> Option<String> {
    let exe = crate::media::ffpipe::ffprobe_exe()?;
    let out = crate::media::ffpipe::command(&exe)
        .args(["-v", "error", "-print_format", "json", "-show_entries", "format_tags=title,artist", path])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    format_tags(v.pointer("/format/tags")?)
}

/// Tag names vary in case between containers (`TITLE` in Ogg/FLAC).
fn format_tags(tags: &serde_json::Value) -> Option<String> {
    let get = |k: &str| {
        tags.as_object()?
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .and_then(|(_, v)| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    match (get("title"), get("artist")) {
        (Some(t), Some(a)) => Some(format!("{t} - {a}")),
        (Some(t), None) => Some(t),
        _ => None,
    }
}

/// The audio clip heard at `t` when nothing visual is on screen then (the timeline's audio-only
/// stretches): None as soon as any enabled clip on a video track covers `t`.
pub fn audio_only_at(p: &Project, t: f64) -> Option<Id> {
    let covers = |c: &crate::model::Clip| c.enabled && t >= c.start && t < c.start + c.duration;
    let tracks = || p.tracks.iter();
    if tracks().filter(|tr| tr.kind == TrackKind::Video).flat_map(|tr| &tr.clips).any(covers) {
        return None;
    }
    tracks()
        .filter(|tr| tr.kind == TrackKind::Audio && !tr.muted)
        .flat_map(|tr| &tr.clips)
        .find(|c| covers(c) && c.kind == ClipKind::Audio)
        .map(|c| c.id)
}

/// Paint the card centred in `rect`: the cover (letterboxed into a square) or the music-note icon, the
/// title under it.
pub fn paint(ui: &egui::Ui, rect: egui::Rect, cover: Option<(egui::TextureId, [u32; 2])>, title: &str) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, egui::Color32::BLACK);
    let side = (rect.height() * 0.5).min(rect.width() * 0.6).max(8.0);
    let art = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, rect.height() * 0.06), egui::vec2(side, side));
    match cover {
        Some((tex, size)) => {
            let aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
            let fit = crate::ui::preview::letterbox(art, aspect, ui.pixels_per_point());
            p.image(tex, fit, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        None => {
            p.rect_filled(art, side * 0.08, egui::Color32::from_gray(28));
            let icons = crate::theme::icons();
            let family = match p.fonts(|f| f.definitions().families.contains_key(&icons)) {
                true => icons,
                false => egui::FontFamily::Proportional,
            };
            let ch = Glyph::MusicNote.icon().unwrap_or('?');
            p.text(art.center(), egui::Align2::CENTER_CENTER, ch, egui::FontId::new(side * 0.4, family), egui::Color32::from_gray(200));
        }
    }
    let font = egui::FontId::proportional((rect.height() * 0.045).clamp(11.0, 28.0));
    let galley = p.layout(title.to_string(), font, egui::Color32::WHITE, rect.width() * 0.9);
    let at = egui::pos2(rect.center().x - galley.size().x * 0.5, art.bottom() + rect.height() * 0.04);
    p.galley(at, galley, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_read_title_and_artist_in_any_case() {
        let t = serde_json::json!({"TITLE": "Song", "artist": " Band "});
        assert_eq!(format_tags(&t).as_deref(), Some("Song - Band"));
        assert_eq!(format_tags(&serde_json::json!({"title": "Only"})).as_deref(), Some("Only"));
        assert_eq!(format_tags(&serde_json::json!({"artist": "No title"})), None);
    }

    #[test]
    fn audio_only_stretches_are_found() {
        let mut p = Project::new();
        let (vt, at) = (p.video_tracks()[0], p.audio_tracks()[0]);
        p.tracks[at].clips.push(crate::model::Clip::new(70, ClipKind::Audio, "song", 0.0, 10.0));
        p.tracks[vt].clips.push(crate::model::Clip::new(71, ClipKind::Video, "shot", 2.0, 3.0));
        assert_eq!(audio_only_at(&p, 1.0), Some(70));
        assert_eq!(audio_only_at(&p, 3.0), None, "a picture is on screen");
        assert_eq!(audio_only_at(&p, 6.0), Some(70));
        assert_eq!(audio_only_at(&p, 11.0), None, "silence");
        p.tracks[vt].clips[0].enabled = false;
        assert_eq!(audio_only_at(&p, 3.0), Some(70), "a disabled clip shows nothing");
    }

    /// The card is a still: painting it (title cached, no cover) asks for no repaint.
    #[test]
    fn assert_no_idle_repaint_audio_card() {
        let m = Meta { title: Some("Song".into()), cover: None };
        TAGS.lock().unwrap().get_or_insert_with(HashMap::new).insert("C:/x/song.mp3".into(), Some(m));
        let ctx = egui::Context::default();
        for _ in 0..30 {
            ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let t = title(ctx, "C:/x/song.mp3");
                    assert_eq!(t, "Song");
                    assert!(cover(ctx, "C:/x/song.mp3").is_none());
                    paint(ui, ui.max_rect(), None, &t);
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "the idle card requested a repaint");
    }
}
