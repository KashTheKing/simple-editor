//! ws:parity-io: the Media Properties window (format, streams, colour, the bitrate graph, rotation and range
//! overrides) and the Open Network Stream window (address, what's there, a live preview, Record to
//! Library). Plain drawing over plain data - `ui::app::io_ctl` owns the state, the threads and the edits -
//! so both are tested headless here.

use crate::engine::media_info::{bitrate_text, colour_text, size_text, Bitrate, MediaInfo, StreamInfo};
use crate::media::io_override::{MediaIo, Range};
use eframe::egui::{self, vec2, Color32, RichText, Sense, Stroke};

// ------------------------------------------------------------------------------------- properties

/// What the Properties window shows.
pub struct PropsView<'a> {
    pub title: &'a str,
    /// None = still reading.
    pub info: Option<&'a Result<MediaInfo, String>>,
    /// The graph's data: None = not asked for, Some(None) = running.
    pub bitrate: Option<Option<&'a Result<Bitrate, String>>>,
    /// The file's decode override (only offered for a project asset).
    pub io: Option<MediaIo>,
}

/// What a click asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum PropsAction {
    /// Draw the bitrate graph of this stream selector ("v:0", "a:0").
    Bitrate(String),
    SetIo(MediaIo),
}

/// Window state that lives between frames.
#[derive(Default)]
pub struct PropsUi {
    /// The graph's stream: 0 = video, 1 = audio.
    pub graph_audio: bool,
}

/// The window. `open` goes false when it is closed.
pub fn props_window(ctx: &egui::Context, open: &mut bool, st: &mut PropsUi, v: &PropsView) -> Option<PropsAction> {
    let mut out = None;
    egui::Window::new(format!("Properties - {}", v.title))
        .id(egui::Id::new("media_properties"))
        .open(open)
        .collapsible(false)
        .default_width(420.0)
        .default_pos(crate::ui::popup_open_pos(ctx))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(560.0).auto_shrink([false, true]).show(ui, |ui| {
                out = props_body(ui, st, v);
            });
        });
    out
}

fn props_body(ui: &mut egui::Ui, st: &mut PropsUi, v: &PropsView) -> Option<PropsAction> {
    let mut out = None;
    let info = match v.info {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Reading the file");
            });
            return None;
        }
        Some(Err(e)) => {
            ui.colored_label(ui.visuals().warn_fg_color, e);
            return None;
        }
        Some(Ok(i)) => i,
    };
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(&info.path).weak().small()).truncate()).on_hover_text(&info.path);
    });
    egui::Grid::new("props_file").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        row(ui, "Format", if info.format_long.is_empty() { &info.format } else { &info.format_long });
        if info.duration > 0.0 {
            row(ui, "Duration", &clock(info.duration));
        }
        if info.size > 0 {
            row(ui, "Size", &size_text(info.size));
        }
        row(ui, "Bitrate", &bitrate_text(info.bit_rate));
    });
    if !info.tags.is_empty() {
        egui::CollapsingHeader::new(format!("Tags ({})", info.tags.len())).id_salt("props_tags").show(ui, |ui| {
            egui::Grid::new("props_tag_grid").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                for (k, val) in &info.tags {
                    row(ui, k, val);
                }
            });
        });
    }
    for s in &info.streams {
        let head = format!("Stream {} · {} · {}", s.index, kind_name(&s.kind), s.codec.to_uppercase());
        egui::CollapsingHeader::new(head).id_salt(("props_stream", s.index)).default_open(s.kind != "data").show(ui, |ui| {
            stream_grid(ui, s);
        });
    }
    let video = info.streams.iter().find(|s| s.kind == "video");
    // decode overrides: a project asset's video only
    if let (Some(io), Some(vs)) = (v.io, video) {
        ui.add_space(6.0);
        ui.strong("Decode");
        egui::Grid::new("props_io").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label("Rotation").on_hover_text("Turn the picture, ignoring the rotation stored in the file");
            let file = format!("From file ({}°)", vs.rotation);
            let text = io.rotate.map_or(file.clone(), |r| format!("{r}°"));
            egui::ComboBox::from_id_salt("props_rotate").selected_text(text).show_ui(ui, |ui| {
                let mut pick = |r: Option<u16>, label: &str, ui: &mut egui::Ui| {
                    if ui.selectable_label(io.rotate == r, label).clicked() {
                        out = Some(PropsAction::SetIo(MediaIo { rotate: r, ..io }));
                    }
                };
                pick(None, &file, ui);
                for r in [0u16, 90, 180, 270] {
                    pick(Some(r), &format!("{r}°"), ui);
                }
            });
            ui.end_row();
            ui.label("Range").on_hover_text(
                "How to read the video's levels. Washed-out blacks: try Full. Crushed blacks and clipped whites: try Limited.",
            );
            let name = |r: Range| match r {
                Range::Auto => "As tagged",
                Range::Limited => "Limited (16-235)",
                Range::Full => "Full (0-255)",
            };
            egui::ComboBox::from_id_salt("props_range").selected_text(name(io.range)).show_ui(ui, |ui| {
                for r in [Range::Auto, Range::Limited, Range::Full] {
                    if ui.selectable_label(io.range == r, name(r)).clicked() {
                        out = Some(PropsAction::SetIo(MediaIo { range: r, ..io }));
                    }
                }
            });
            ui.end_row();
        });
    }
    // the bitrate graph
    let has_audio = info.streams.iter().any(|s| s.kind == "audio");
    if video.is_some() || has_audio {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Bitrate");
            if video.is_some() && has_audio {
                let before = st.graph_audio;
                ui.selectable_value(&mut st.graph_audio, false, "Video");
                ui.selectable_value(&mut st.graph_audio, true, "Audio");
                if before != st.graph_audio && v.bitrate.is_some() {
                    out = Some(PropsAction::Bitrate(sel(st.graph_audio).into()));
                }
            } else {
                st.graph_audio = video.is_none();
            }
            if v.bitrate.is_none()
                && ui.button("Show Graph").on_hover_text("Reads the whole file once to measure each second").clicked()
            {
                out = Some(PropsAction::Bitrate(sel(st.graph_audio).into()));
            }
        });
        match v.bitrate {
            None => {}
            Some(None) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Measuring");
                });
            }
            Some(Some(Err(e))) => {
                ui.colored_label(ui.visuals().warn_fg_color, e);
            }
            Some(Some(Ok(b))) => bitrate_graph(ui, b),
        }
    }
    if ui.button("Copy").on_hover_text("Copy these properties as text").clicked() {
        ui.ctx().copy_text(as_text(info));
    }
    out
}

/// "1:02:03" / "2:03" for a length in seconds.
pub fn clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", (s / 60) % 60, s % 60),
    }
}

fn sel(audio: bool) -> &'static str {
    if audio {
        "a:0"
    } else {
        "v:0"
    }
}

fn row(ui: &mut egui::Ui, k: &str, val: &str) {
    if val.is_empty() {
        return;
    }
    ui.label(k);
    ui.add(egui::Label::new(val).truncate()).on_hover_text(val);
    ui.end_row();
}

fn kind_name(k: &str) -> &str {
    match k {
        "video" => "Video",
        "audio" => "Audio",
        "subtitle" => "Subtitles",
        "data" => "Data",
        "attachment" => "Attachment",
        other => other,
    }
}

/// The rows of one stream, as (label, value) - shared by the grid and `as_text`.
pub fn stream_rows(s: &StreamInfo) -> Vec<(&'static str, String)> {
    let mut r: Vec<(&'static str, String)> = Vec::new();
    let codec = match (s.codec_long.is_empty(), s.profile.is_empty()) {
        (false, false) => format!("{} ({})", s.codec_long, s.profile),
        (false, true) => s.codec_long.clone(),
        _ => s.codec.clone(),
    };
    r.push(("Codec", codec));
    if s.kind == "video" {
        r.push(("Resolution", format!("{} × {}", s.width, s.height)));
        if s.fps > 0.0 {
            r.push(("Frame rate", format!("{:.3} fps", s.fps).replace(".000", "")));
        }
        r.push(("Pixels", s.pix_fmt.clone()));
        r.push(("Colour", colour_text(s)));
        if s.rotation != 0 {
            r.push(("Rotation", format!("{}°", s.rotation)));
        }
        if !matches!(s.field_order.as_str(), "" | "progressive" | "unknown") {
            r.push(("Fields", s.field_order.clone()));
        }
        if s.frames > 0 {
            r.push(("Frames", s.frames.to_string()));
        }
    }
    if s.kind == "audio" {
        r.push(("Sample rate", format!("{} Hz", s.sample_rate)));
        let layout = if s.channel_layout.is_empty() { String::new() } else { format!(" ({})", s.channel_layout) };
        r.push(("Channels", format!("{}{layout}", s.channels)));
        r.push(("Samples", s.sample_fmt.clone()));
    }
    r.push(("Bitrate", bitrate_text(s.bit_rate)));
    r.push(("Language", s.language.clone()));
    r.push(("Title", s.title.clone()));
    if s.default {
        r.push(("Default", "yes".into()));
    }
    r.retain(|(_, v)| !v.is_empty());
    r
}

fn stream_grid(ui: &mut egui::Ui, s: &StreamInfo) {
    egui::Grid::new(("props_stream_grid", s.index)).num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
        for (k, val) in stream_rows(s) {
            row(ui, k, &val);
        }
    });
}

/// The whole window as plain text (Copy).
pub fn as_text(info: &MediaInfo) -> String {
    let mut t = format!("{}\nFormat: {}\n", info.path, info.format_long);
    if info.duration > 0.0 {
        t.push_str(&format!("Duration: {:.3} s\n", info.duration));
    }
    if info.bit_rate > 0 {
        t.push_str(&format!("Bitrate: {}\n", bitrate_text(info.bit_rate)));
    }
    for s in &info.streams {
        t.push_str(&format!("\nStream {} ({})\n", s.index, kind_name(&s.kind)));
        for (k, v) in stream_rows(s) {
            t.push_str(&format!("  {k}: {v}\n"));
        }
    }
    t
}

/// Bars per bucket, an average line, keyframe ticks along the bottom, hover = time and rate.
fn bitrate_graph(ui: &mut egui::Ui, b: &Bitrate) {
    if b.kbps.is_empty() {
        ui.weak("No packets in that stream");
        return;
    }
    ui.weak(format!(
        "Average {} · peak {} · {} keyframes",
        bitrate_text((b.average * 1000.0) as u64),
        bitrate_text((b.peak * 1000.0) as u64),
        b.keys.len()
    ));
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 120.0), Sense::hover());
    let p = ui.painter_at(rect);
    let vis = ui.visuals();
    p.rect_filled(rect, 3.0, vis.extreme_bg_color);
    let top = b.kbps.iter().copied().fold(b.peak, f64::max).max(1.0);
    let n = b.kbps.len();
    let w = rect.width() / n as f32;
    let bar = vis.selection.bg_fill;
    for (i, k) in b.kbps.iter().enumerate() {
        let h = (k / top) as f32 * (rect.height() - 8.0);
        let x = rect.left() + i as f32 * w;
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(x, rect.bottom() - h), egui::pos2(x + w.max(1.0) - 0.5, rect.bottom())), 0.0, bar);
    }
    let avg_y = rect.bottom() - (b.average / top) as f32 * (rect.height() - 8.0);
    p.hline(rect.x_range(), avg_y, Stroke::new(1.0, vis.warn_fg_color));
    let span = n as f64 * b.step;
    for k in &b.keys {
        let x = rect.left() + (k / span) as f32 * rect.width();
        p.vline(x, (rect.bottom() - 4.0)..=rect.bottom(), Stroke::new(1.0, vis.text_color()));
    }
    if let Some(pos) = resp.hover_pos() {
        let i = (((pos.x - rect.left()) / w) as usize).min(n - 1);
        p.vline(pos.x, rect.y_range(), Stroke::new(1.0, Color32::from_white_alpha(80)));
        resp.on_hover_text(format!(
            "{} · {}",
            clock(i as f64 * b.step),
            bitrate_text((b.kbps[i] * 1000.0) as u64)
        ));
    }
}

// --------------------------------------------------------------------------------------- streams

/// What the Open Network Stream window shows.
pub struct StreamView<'a> {
    pub recent: &'a [String],
    /// None = not probed; Some(None) = probing.
    pub probe: Option<Option<&'a Result<MediaInfo, String>>>,
    /// The live preview's newest frame, and why it stopped.
    pub preview: Option<(egui::TextureId, egui::Vec2)>,
    pub previewing: bool,
    pub preview_error: Option<&'a str>,
    /// While recording: (seconds, bytes, stopping).
    pub recording: Option<(f64, u64, bool)>,
}

/// Window state that lives between frames.
pub struct StreamUi {
    pub url: String,
    pub limit_on: bool,
    pub limit_min: f64,
    /// Why the address is refused (from the last click).
    pub error: Option<String>,
}

impl Default for StreamUi {
    fn default() -> Self {
        StreamUi { url: String::new(), limit_on: false, limit_min: 10.0, error: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamAction {
    /// Look at the address (and start the preview).
    Open,
    StopPreview,
    /// Record (with the length limit in seconds when on).
    Record(Option<f64>),
    StopRecording,
}

pub fn stream_window(ctx: &egui::Context, open: &mut bool, st: &mut StreamUi, v: &StreamView) -> Option<StreamAction> {
    let mut out = None;
    egui::Window::new("Open Network Stream")
        .id(egui::Id::new("open_network_stream"))
        .open(open)
        .collapsible(false)
        .default_width(480.0)
        .default_pos(crate::ui::popup_open_pos(ctx))
        .show(ctx, |ui| out = stream_body(ui, st, v));
    out
}

fn stream_body(ui: &mut egui::Ui, st: &mut StreamUi, v: &StreamView) -> Option<StreamAction> {
    let mut out = None;
    let busy = v.recording.is_some();
    ui.horizontal(|ui| {
        let r = ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut st.url).hint_text("https://… .m3u8, rtsp://…, rtmp://…, srt://…, udp://…").desired_width(330.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        r.on_hover_text("A stream address: HTTP(S) / HLS / DASH, RTMP, RTSP, SRT, UDP or RTP");
        if !v.recent.is_empty() && !busy {
            egui::ComboBox::from_id_salt("stream_recent").selected_text("Recent").width(70.0).show_ui(ui, |ui| {
                for u in v.recent {
                    if ui.selectable_label(false, u).clicked() {
                        st.url = u.clone();
                        out = Some(StreamAction::Open);
                    }
                }
            });
        }
        if (ui.add_enabled(!busy && !st.url.trim().is_empty(), egui::Button::new("Open")).clicked() || enter) && !busy {
            out = Some(StreamAction::Open);
        }
    });
    if let Some(e) = &st.error {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    // what's there
    match v.probe {
        None => {}
        Some(None) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Connecting");
            });
        }
        Some(Some(Err(e))) => {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
        Some(Some(Ok(i))) => {
            ui.weak(stream_summary(i));
        }
    }
    // the picture
    if let Some((tex, size)) = v.preview {
        let w = ui.available_width().min(size.x);
        ui.image(egui::load::SizedTexture::new(tex, vec2(w, w * size.y / size.x.max(1.0))));
    } else if v.previewing {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(480.0), 270.0), Sense::hover());
        ui.painter().rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    }
    if let Some(e) = v.preview_error {
        ui.weak(format!("Preview: {e}"));
    }
    if v.previewing && ui.small_button("Stop Preview").clicked() {
        out = Some(StreamAction::StopPreview);
    }
    ui.add_space(6.0);
    ui.separator();
    match v.recording {
        None => {
            ui.horizontal(|ui| {
                let ok = matches!(v.probe, Some(Some(Ok(_))));
                if ui
                    .add_enabled(ok, egui::Button::new("Record to Library"))
                    .on_hover_text("Saves the stream as it comes (no re-encode) and adds the file to the Library")
                    .on_disabled_hover_text("Open a stream first")
                    .clicked()
                {
                    out = Some(StreamAction::Record(st.limit_on.then_some(st.limit_min * 60.0)));
                }
                ui.checkbox(&mut st.limit_on, "Stop after").on_hover_text("End the recording on its own after this long");
                ui.add_enabled(st.limit_on, egui::DragValue::new(&mut st.limit_min).range(0.1..=600.0).speed(0.5).suffix(" min"));
            });
        }
        Some((secs, bytes, stopping)) => {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(220, 60, 60), "●");
                ui.label(format!("{} · {}", clock(secs), size_text(bytes)));
                if stopping {
                    ui.spinner();
                    ui.weak("Finishing the file");
                } else if ui.button("Stop").clicked() {
                    out = Some(StreamAction::StopRecording);
                }
            });
        }
    }
    out
}

/// "Live · H.264 1920 × 1080 30 fps · AAC stereo" / "12:30 · ...".
pub fn stream_summary(i: &MediaInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(if crate::engine::netstream::is_live(i) { "Live".into() } else { clock(i.duration) });
    if let Some(v) = i.streams.iter().find(|s| s.kind == "video") {
        let fps = if v.fps > 0.0 { format!(" {:.0} fps", v.fps) } else { String::new() };
        parts.push(format!("{} {} × {}{fps}", v.codec.to_uppercase(), v.width, v.height));
    }
    if let Some(a) = i.streams.iter().find(|s| s.kind == "audio") {
        let ch = if a.channel_layout.is_empty() { format!("{} ch", a.channels) } else { a.channel_layout.clone() };
        parts.push(format!("{} {ch}", a.codec.to_uppercase()));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> MediaInfo {
        MediaInfo {
            path: "C:/clip.mp4".into(),
            format: "mov,mp4".into(),
            format_long: "QuickTime / MOV".into(),
            duration: 12.0,
            size: 4_000_000,
            bit_rate: 2_600_000,
            tags: vec![("encoder".into(), "Lavf".into())],
            streams: vec![
                StreamInfo {
                    index: 0,
                    kind: "video".into(),
                    codec: "h264".into(),
                    codec_long: "H.264 / AVC".into(),
                    profile: "High".into(),
                    width: 1920,
                    height: 1080,
                    fps: 30.0,
                    pix_fmt: "yuv420p".into(),
                    color_range: "tv".into(),
                    color_space: "bt709".into(),
                    rotation: 90,
                    ..Default::default()
                },
                StreamInfo { index: 1, kind: "audio".into(), codec: "aac".into(), sample_rate: 48000, channels: 2, channel_layout: "stereo".into(), ..Default::default() },
            ],
        }
    }

    fn run(ctx: &egui::Context, f: &mut dyn FnMut(&egui::Context)) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| f(ctx));
    }

    #[test]
    fn rows_and_text() {
        let i = info();
        let r = stream_rows(&i.streams[0]);
        assert!(r.contains(&("Codec", "H.264 / AVC (High)".into())));
        assert!(r.contains(&("Frame rate", "30 fps".into())));
        assert!(r.contains(&("Colour", "BT.709 · limited range".into())));
        assert!(r.contains(&("Rotation", "90°".into())));
        assert!(!r.iter().any(|(k, _)| *k == "Bitrate"), "an empty value is left out");
        let t = as_text(&i);
        assert!(t.contains("Stream 1 (Audio)") && t.contains("Channels: 2 (stereo)"), "{t}");
        assert_eq!(stream_summary(&i), "0:12 · H264 1920 × 1080 30 fps · AAC stereo");
        assert_eq!((clock(3723.4), clock(59.6)), ("1:02:03".to_string(), "1:00".to_string()));
        let live = MediaInfo { duration: 0.0, ..i };
        assert!(stream_summary(&live).starts_with("Live · "));
    }

    #[test]
    fn assert_no_idle_repaint_media_properties() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let ok: Result<MediaInfo, String> = Ok(info());
        let graph: Result<Bitrate, String> = Ok(crate::engine::media_info::buckets("0,0,1000,K\n1,1,2000,_\n2,2,500,_\n", 1.0));
        let mut st = PropsUi::default();
        let views = [
            PropsView { title: "clip.mp4", info: Some(&ok), bitrate: None, io: Some(MediaIo::default()) },
            PropsView { title: "clip.mp4", info: Some(&ok), bitrate: Some(Some(&graph)), io: None },
        ];
        for v in &views {
            let mut open = true;
            for _ in 0..20 {
                run(&ctx, &mut |ctx| assert_eq!(props_window(ctx, &mut open, &mut st, v), None));
            }
            assert!(!ctx.has_requested_repaint(), "an idle Properties window must not repaint");
        }
    }

    #[test]
    fn assert_no_idle_repaint_network_stream() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let ok: Result<MediaInfo, String> = Ok(MediaInfo { duration: 0.0, ..info() });
        let recent = vec!["rtsp://cam/1".to_string()];
        let mut st = StreamUi { url: "rtsp://cam/1".into(), ..Default::default() };
        let views = [
            StreamView { recent: &recent, probe: None, preview: None, previewing: false, preview_error: None, recording: None },
            StreamView { recent: &recent, probe: Some(Some(&ok)), preview: None, previewing: false, preview_error: Some("the stream ended"), recording: None },
            StreamView { recent: &[], probe: Some(Some(&ok)), preview: None, previewing: false, preview_error: None, recording: Some((12.0, 4_000_000, false)) },
        ];
        for v in &views {
            let mut open = true;
            for _ in 0..20 {
                run(&ctx, &mut |ctx| assert_eq!(stream_window(ctx, &mut open, &mut st, v), None));
            }
            assert!(!ctx.has_requested_repaint(), "an idle Network Stream window must not repaint");
        }
    }
}
