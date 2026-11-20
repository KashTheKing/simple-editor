//! ws:parity-io: detailed media properties (container, every stream, bitrate, colour) and the per-second
//! bitrate graph behind the Media Properties window (`ui::app::io_ctl`) and `media.properties` /
//! `media.bitrate`.
//!
//! Both run ffprobe on a worker thread: `properties` reads the headers (instant), `bitrate` demuxes the
//! whole file once (seconds for a long file), so the window only asks for it when the graph is opened.
//! The parsing (`parse`, `buckets`) is plain and tested on its own.

use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;

/// One stream of a file.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct StreamInfo {
    pub index: u32,
    /// "video" | "audio" | "subtitle" | "data" | "attachment".
    pub kind: String,
    pub codec: String,
    /// The long codec name ("H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10").
    pub codec_long: String,
    pub profile: String,
    /// Bits per second; 0 = not stored in the file.
    pub bit_rate: u64,
    pub language: String,
    pub title: String,
    pub default: bool,
    // video
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub pix_fmt: String,
    /// "tv" (limited) | "pc" (full) | "" (untagged).
    pub color_range: String,
    pub color_primaries: String,
    pub color_transfer: String,
    pub color_space: String,
    pub field_order: String,
    /// Display-matrix rotation in degrees (0, 90, 180, 270), normalised to clockwise.
    pub rotation: i32,
    pub frames: u64,
    // audio
    pub sample_rate: u32,
    pub channels: u32,
    pub channel_layout: String,
    pub sample_fmt: String,
}

/// A whole file.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MediaInfo {
    pub path: String,
    /// "mov,mp4,m4a,3gp,3g2,mj2" -> the long name "QuickTime / MOV".
    pub format: String,
    pub format_long: String,
    pub duration: f64,
    pub size: u64,
    /// Overall bits per second (0 = unknown).
    pub bit_rate: u64,
    /// Container tags (title, encoder, creation_time, ...), as stored.
    pub tags: Vec<(String, String)>,
    pub streams: Vec<StreamInfo>,
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn n(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))).unwrap_or(0.0)
}
fn rate(s: &str) -> f64 {
    let (a, b) = s.split_once('/').unwrap_or((s, "1"));
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(a), Ok(b)) if b > 0.0 => a / b,
        _ => 0.0,
    }
}

/// Clockwise degrees from ffprobe's display matrix (which reports counter-clockwise, e.g. -90 for a phone
/// held upright) or an old `rotate` tag (clockwise).
fn rotation_of(st: &Value) -> i32 {
    let side = st.get("side_data_list").and_then(Value::as_array).into_iter().flatten();
    let ccw = side.map(|d| n(d, "rotation")).find(|r| *r != 0.0);
    let cw = match ccw {
        Some(r) => -r,
        None => n(st.get("tags").unwrap_or(&Value::Null), "rotate"),
    };
    (cw.round() as i32).rem_euclid(360)
}

/// `ffprobe -show_format -show_streams -print_format json` output -> MediaInfo.
pub fn parse(path: &str, json: &Value) -> MediaInfo {
    let f = json.get("format").unwrap_or(&Value::Null);
    let mut tags: Vec<(String, String)> = f
        .get("tags")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default();
    tags.sort();
    let empty = Vec::new();
    let streams = json.get("streams").and_then(Value::as_array).unwrap_or(&empty);
    MediaInfo {
        path: path.to_string(),
        format: s(f, "format_name"),
        format_long: s(f, "format_long_name"),
        duration: n(f, "duration"),
        size: n(f, "size") as u64,
        bit_rate: n(f, "bit_rate") as u64,
        tags,
        streams: streams.iter().map(stream).collect(),
    }
}

fn stream(st: &Value) -> StreamInfo {
    let tags = st.get("tags").unwrap_or(&Value::Null);
    let mut fps = rate(&s(st, "avg_frame_rate"));
    if fps <= 0.0 {
        fps = rate(&s(st, "r_frame_rate"));
    }
    StreamInfo {
        index: n(st, "index") as u32,
        kind: s(st, "codec_type"),
        codec: s(st, "codec_name"),
        codec_long: s(st, "codec_long_name"),
        profile: s(st, "profile"),
        bit_rate: n(st, "bit_rate").max(n(tags, "BPS")) as u64,
        language: s(tags, "language"),
        title: s(tags, "title"),
        default: st.pointer("/disposition/default").and_then(Value::as_i64).unwrap_or(0) != 0,
        width: n(st, "width") as u32,
        height: n(st, "height") as u32,
        fps,
        pix_fmt: s(st, "pix_fmt"),
        color_range: s(st, "color_range"),
        color_primaries: s(st, "color_primaries"),
        color_transfer: s(st, "color_transfer"),
        color_space: s(st, "color_space"),
        field_order: s(st, "field_order"),
        rotation: rotation_of(st),
        frames: n(st, "nb_frames") as u64,
        sample_rate: n(st, "sample_rate") as u32,
        channels: n(st, "channels") as u32,
        channel_layout: s(st, "channel_layout"),
        sample_fmt: s(st, "sample_fmt"),
    }
}

/// "12.4 Mb/s" / "320 kb/s" / "" for 0.
pub fn bitrate_text(bps: u64) -> String {
    match bps {
        0 => String::new(),
        b if b >= 1_000_000 => format!("{:.1} Mb/s", b as f64 / 1e6),
        b => format!("{} kb/s", (b as f64 / 1e3).round()),
    }
}

/// "1.2 GB" / "340 MB" / "12 KB".
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

/// The colour line of a video stream: "BT.709 · limited range · 8-bit 4:2:0" style, from ffprobe names.
pub fn colour_text(st: &StreamInfo) -> String {
    let range = match st.color_range.as_str() {
        "tv" | "mpeg" => "limited range",
        "pc" | "jpeg" => "full range",
        _ => "range not tagged",
    };
    let space = [&st.color_space, &st.color_primaries, &st.color_transfer]
        .into_iter()
        .find(|x| !x.is_empty() && *x != "unknown")
        .map(|x| x.replace("bt", "BT.").replace("BT.2020nc", "BT.2020").to_string())
        .unwrap_or_else(|| "untagged".into());
    let trc = match st.color_transfer.as_str() {
        "smpte2084" => " · PQ (HDR10)",
        "arib-std-b67" => " · HLG",
        _ => "",
    };
    format!("{space}{trc} · {range}")
}

/// Run ffprobe for the headers. Blocking (tens of ms): call it on a worker thread.
pub fn properties(path: &str) -> Result<MediaInfo, String> {
    let exe =
        crate::media::ffpipe::ffprobe_exe().ok_or("ffprobe.exe not found (Settings ▸ General ▸ FFmpeg folder)")?;
    let out = crate::media::ffpipe::command(&exe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe json: {e}"))?;
    Ok(parse(path, &v))
}

/// Per-second bitrate of one stream.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Bitrate {
    /// Seconds per bucket.
    pub step: f64,
    /// Bits per second in each bucket, from t = 0.
    pub kbps: Vec<f64>,
    pub average: f64,
    pub peak: f64,
    /// Keyframe times (video), for the graph's ticks.
    pub keys: Vec<f64>,
}

/// ffprobe `packet=pts_time,dts_time,size,flags` csv lines -> buckets of `step` seconds. Packets with no time
/// are skipped; the graph starts at the first packet's time.
pub fn buckets(csv: &str, step: f64) -> Bitrate {
    let step = step.max(0.01);
    let mut rows: Vec<(f64, u64, bool)> = Vec::new();
    for line in csv.lines() {
        let mut it = line.split(',').map(str::trim);
        let pts = it.next().and_then(|x| x.parse::<f64>().ok());
        let dts = it.next().and_then(|x| x.parse::<f64>().ok());
        let size = it.next().and_then(|x| x.parse::<u64>().ok());
        let key = it.next().is_some_and(|f| f.starts_with('K'));
        if let (Some(t), Some(size)) = (pts.or(dts), size) {
            rows.push((t, size, key));
        }
    }
    let Some(t0) = rows.iter().map(|r| r.0).reduce(f64::min) else {
        return Bitrate { step, ..Default::default() };
    };
    let mut bits: Vec<f64> = Vec::new();
    let mut keys = Vec::new();
    for (t, size, key) in rows {
        let i = ((t - t0) / step).floor().max(0.0) as usize;
        if bits.len() <= i {
            bits.resize(i + 1, 0.0);
        }
        bits[i] += size as f64 * 8.0;
        if key {
            keys.push(t - t0);
        }
    }
    let kbps: Vec<f64> = bits.iter().map(|b| b / step / 1000.0).collect();
    // the last bucket is usually partial: leave it out of the peak so a short tail never reads as a dip/spike
    let full = &kbps[..kbps.len().saturating_sub(1).max(1).min(kbps.len())];
    let peak = full.iter().copied().fold(0.0, f64::max);
    let average = kbps.iter().sum::<f64>() / kbps.len().max(1) as f64;
    keys.sort_by(f64::total_cmp);
    Bitrate { step, kbps, average, peak, keys }
}

/// Demux `sel` ("v:0", "a:0") and bucket it. Blocking (reads the whole file): a worker thread only.
pub fn bitrate(path: &str, sel: &str, step: f64) -> Result<Bitrate, String> {
    let exe = crate::media::ffpipe::ffprobe_exe().ok_or("ffprobe.exe not found")?;
    let out = crate::media::ffpipe::command(&exe)
        .args(["-v", "error", "-select_streams", sel, "-show_entries", "packet=pts_time,dts_time,size,flags"])
        .args(["-of", "csv=p=0", path])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(buckets(&String::from_utf8_lossy(&out.stdout), step))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn phone() -> Value {
        json!({
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "format_long_name": "QuickTime / MOV",
                       "duration": "12.5", "size": "31250000", "bit_rate": "20000000",
                       "tags": {"major_brand": "isom", "creation_time": "2026-01-02T03:04:05Z"}},
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": "hevc", "codec_long_name": "H.265 / HEVC",
                 "profile": "Main 10", "width": 3840, "height": 2160, "avg_frame_rate": "0/0",
                 "r_frame_rate": "60000/1001", "pix_fmt": "yuv420p10le", "color_range": "tv",
                 "color_space": "bt2020nc", "color_primaries": "bt2020", "color_transfer": "arib-std-b67",
                 "bit_rate": "19500000", "nb_frames": "749", "disposition": {"default": 1},
                 "side_data_list": [{"side_data_type": "Display Matrix", "rotation": -90}]},
                {"index": 1, "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 2,
                 "channel_layout": "stereo", "sample_fmt": "fltp",
                 "tags": {"language": "eng", "BPS": "256000"}},
                {"index": 2, "codec_type": "subtitle", "codec_name": "mov_text", "tags": {"title": "English"}}
            ]
        })
    }

    #[test]
    fn parses_format_streams_and_colour() {
        let m = parse("C:/a.mov", &phone());
        assert_eq!(
            (m.format_long.as_str(), m.duration, m.size, m.bit_rate),
            ("QuickTime / MOV", 12.5, 31_250_000, 20_000_000)
        );
        assert_eq!(m.tags[0], ("creation_time".into(), "2026-01-02T03:04:05Z".into()), "sorted");
        let v = &m.streams[0];
        assert_eq!((v.width, v.height, v.rotation, v.frames), (3840, 2160, 90, 749));
        assert!((v.fps - 59.94).abs() < 0.01, "falls back to r_frame_rate");
        assert!(v.default);
        assert_eq!(colour_text(v), "BT.2020 · HLG · limited range");
        let a = &m.streams[1];
        assert_eq!((a.sample_rate, a.channels, a.bit_rate, a.language.as_str()), (48000, 2, 256_000, "eng"));
        assert_eq!(m.streams[2].title, "English");
    }

    #[test]
    fn rotation_reads_both_conventions() {
        assert_eq!(rotation_of(&json!({"side_data_list": [{"rotation": 90}]})), 270);
        assert_eq!(rotation_of(&json!({"side_data_list": [{"rotation": -180}]})), 180);
        assert_eq!(rotation_of(&json!({"tags": {"rotate": "90"}})), 90);
        assert_eq!(rotation_of(&json!({})), 0);
    }

    #[test]
    fn text_helpers() {
        assert_eq!(bitrate_text(0), "");
        assert_eq!(bitrate_text(320_000), "320 kb/s");
        assert_eq!(bitrate_text(12_400_000), "12.4 Mb/s");
        assert_eq!(size_text(31_250_000), "31.2 MB");
        let full = StreamInfo { color_range: "pc".into(), ..Default::default() };
        assert_eq!(colour_text(&full), "untagged · full range");
    }

    #[test]
    fn buckets_sum_bits_per_second_and_find_keys() {
        // 1 s buckets from t0 = 10: 2 x 125000 B in second 0 (2 Mb/s), 1 x 62500 B in second 1, tail in 2
        let csv = "10.0,9.9,125000,K__\n10.5,10.4,125000,___\n11.2,11.1,62500,K_\nN/A,N/A,999,K\n12.0,,1000,___\n";
        let b = buckets(csv, 1.0);
        assert_eq!(b.kbps.len(), 3);
        assert_eq!((b.kbps[0], b.kbps[1], b.kbps[2]), (2000.0, 500.0, 8.0));
        assert_eq!(b.peak, 2000.0);
        assert!((b.average - 836.0).abs() < 1e-9);
        assert_eq!(b.keys.len(), 2);
        assert!(b.keys[0] == 0.0 && (b.keys[1] - 1.2).abs() < 1e-9);
        assert_eq!(buckets("", 1.0).kbps.len(), 0, "empty input");
        // the partial last bucket is not the peak
        assert_eq!(buckets("0,0,100,K\n1,1,10000,_\n", 1.0).peak, 0.8);
    }
}
