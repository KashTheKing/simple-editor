//! ---- ws:mixed-footage ----
//! What a source says about the camera behind it: make/model, a log or HLG gamma hint and 10-bit, read
//! from ffprobe's tags while the import worker probes the file (`ffpipe::probe` calls `remember`), plus
//! the frame-rate arithmetic for mixing 25p/50p-style footage. The cache is per path, like the transfer
//! cache next to it, so nothing here ever runs ffprobe on the UI thread: a file not probed this session
//! simply reads as unknown.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FootageMeta {
    /// "Sony ILCE-7M4", "Apple iPhone 15 Pro", "GoPro" (empty = unknown).
    pub camera: String,
    /// "S-Log3", "S-Log2", "HLG" or "" (no log / HDR gamma hint).
    pub gamma: &'static str,
    /// 10 bits (or more) per sample.
    pub ten_bit: bool,
}

static CACHE: Mutex<Option<HashMap<String, FootageMeta>>> = Mutex::new(None);

pub fn remember(path: &str, m: FootageMeta) {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(path.to_string(), m);
}

/// What the import probe found for `path` (None = not probed by ffprobe this session).
pub fn of(path: &str) -> Option<FootageMeta> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(path).cloned())
}

fn tag<'a>(tags: &'a Value, keys: &[&str]) -> &'a str {
    let Some(map) = tags.as_object() else { return "" };
    for k in keys {
        if let Some(v) = map.iter().find(|(name, _)| name.eq_ignore_ascii_case(k)).and_then(|(_, v)| v.as_str()) {
            if !v.trim().is_empty() {
                return v.trim();
            }
        }
    }
    ""
}

/// Read the meta from ffprobe's JSON: the first video `stream` and the `format` object, and the file `path`
/// (a "slog3" in the file name is a gamma hint too).
pub fn from_probe(stream: &Value, format: &Value, path: &str) -> FootageMeta {
    let (ft, st) = (format.get("tags").unwrap_or(&Value::Null), stream.get("tags").unwrap_or(&Value::Null));
    let make_keys = ["com.apple.quicktime.make", "make", "com.android.manufacturer"];
    let model_keys = ["com.apple.quicktime.model", "model", "com.android.model"];
    let make = Some(tag(ft, &make_keys)).filter(|s| !s.is_empty()).unwrap_or_else(|| tag(st, &make_keys));
    let model = Some(tag(ft, &model_keys)).filter(|s| !s.is_empty()).unwrap_or_else(|| tag(st, &model_keys));
    let camera = if !model.is_empty() {
        // "Sony" + "Sony ILCE-7M4" reads once
        if make.is_empty() || model.to_ascii_lowercase().starts_with(&make.to_ascii_lowercase()) {
            model.to_string()
        } else {
            format!("{make} {model}")
        }
    } else if !make.is_empty() {
        make.to_string()
    } else {
        // no make/model tags: the brand a camera's own muxer leaves behind
        let brand = tag(ft, &["major_brand"]).to_ascii_lowercase();
        let handler = format!("{} {}", tag(st, &["handler_name"]), tag(ft, &["encoder"])).to_ascii_lowercase();
        if brand.starts_with("xavc") {
            "Sony".into()
        } else if handler.contains("gopro") {
            "GoPro".into()
        } else if handler.contains("dji") {
            "DJI".into()
        } else {
            String::new()
        }
    };
    let mut hay = String::new();
    for t in [ft, st] {
        if let Some(m) = t.as_object() {
            for (k, v) in m {
                hay.push_str(&format!("{k}={} ", v.as_str().unwrap_or("")));
            }
        }
    }
    hay.push_str(&std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy());
    let hay = hay.to_ascii_lowercase().replace(['-', '_', ' '], "");
    let trc = stream.get("color_transfer").and_then(Value::as_str).unwrap_or("");
    let gamma = if hay.contains("slog3") {
        "S-Log3"
    } else if hay.contains("slog2") {
        "S-Log2"
    } else if trc == "arib-std-b67" || hay.contains("hlg") {
        "HLG"
    } else {
        ""
    };
    let pix = stream.get("pix_fmt").and_then(Value::as_str).unwrap_or("");
    let raw_bits: u32 = stream.get("bits_per_raw_sample").and_then(Value::as_str).and_then(|s| s.parse().ok()).unwrap_or(0);
    let ten_bit = raw_bits >= 10 || pix.contains("p10") || pix.contains("p12") || pix.contains("10le");
    FootageMeta { camera, gamma, ten_bit }
}

/// How many source frames make one project frame: Some(2) for 50p in a 25p project, 60p in 30p, 59.94 in
/// 29.97; None when the source is not a whole multiple (24p in 30p) or not faster at all.
pub fn rate_multiple(src_fps: f64, project_fps: f64) -> Option<u32> {
    if src_fps <= 0.0 || project_fps <= 0.0 {
        return None;
    }
    let r = src_fps / project_fps;
    let n = r.round();
    (n >= 2.0 && (r - n).abs() < 0.01).then_some(n as u32)
}

/// The speed that shows every source frame once, one per project frame (50p in 25p: exactly 0.5).
pub fn every_frame_speed(src_fps: f64, project_fps: f64) -> Option<f64> {
    rate_multiple(src_fps, project_fps).map(|n| 1.0 / n as f64)
}

/// The project rate for a batch of sources' (snapped) rates and whether every one fits it evenly: the
/// lowest rate that every other is a whole multiple of (25 for 25 + 50); with no such rate, the most
/// common one (ties: the first seen) and `false`.
pub fn batch_fps(rates: &[f64]) -> Option<(f64, bool)> {
    let rates: Vec<f64> = rates.iter().copied().filter(|r| *r > 0.0).collect();
    let mut distinct: Vec<(f64, usize)> = Vec::new();
    for &r in &rates {
        match distinct.iter_mut().find(|(d, _)| (d - r).abs() < 1e-3) {
            Some(d) => d.1 += 1,
            None => distinct.push((r, 1)),
        }
    }
    let base = distinct.iter().map(|d| d.0).fold(f64::INFINITY, f64::min);
    if !base.is_finite() {
        return None;
    }
    let even = distinct.iter().all(|&(r, _)| (r - base).abs() < 1e-3 || rate_multiple(r, base).is_some());
    if even {
        return Some((base, true));
    }
    let most = distinct.iter().fold(distinct[0], |best, d| if d.1 > best.1 { *d } else { best });
    Some((most.0, false))
}

/// The library id of the S-Log3 conversion LUT (`library::save(Kind::Lut, ..)` installs it on first use).
pub const SLOG3_LUT: &str = "slog3-to-rec709";

/// A 33³ `.cube` taking Sony S-Log3 / S-Gamut3.Cine (as decoded: legal range expanded to 0..1) to Rec.709:
/// Sony's published S-Log3 curve to scene linear, the S-Gamut3.Cine to Rec.709 primaries matrix, a soft
/// shoulder that keeps mid grey at about 42 % and rolls about +3.5 stops of highlights into white, then the
/// Rec.709 camera curve.
pub fn slog3_cube() -> String {
    const N: usize = 33;
    const M: [[f64; 3]; 3] = [
        [1.6269474097, -0.5401385389, -0.0868088709],
        [-0.1785155271, 1.4179409275, -0.2394254004],
        [0.0444361150, -0.1959199662, 1.2403560849],
    ];
    let to_lin = |x: f64| {
        let code = x * 876.0 + 64.0; // decoded full range back to 10-bit code values
        if code >= 171.2102946929 {
            10f64.powf((code - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
        } else {
            (code - 95.0) * 0.01125 / (171.2102946929 - 95.0)
        }
    };
    let shoulder = |x: f64| {
        let x = x.max(0.0);
        (x * (1.0 + x / 16.0) / (1.0 + x) * 1.25).min(1.0)
    };
    let oetf = |l: f64| if l < 0.018 { 4.5 * l } else { 1.099 * l.powf(0.45) - 0.099 };
    let mut s = format!("TITLE \"S-Log3 to Rec.709\"\nLUT_3D_SIZE {N}\n");
    for b in 0..N {
        for g in 0..N {
            for r in 0..N {
                let inp = [r, g, b].map(|i| to_lin(i as f64 / (N - 1) as f64));
                let out: Vec<f64> = M
                    .iter()
                    .map(|row| oetf(shoulder(row[0] * inp[0] + row[1] * inp[1] + row[2] * inp[2])).clamp(0.0, 1.0))
                    .collect();
                s.push_str(&format!("{:.6} {:.6} {:.6}\n", out[0], out[1], out[2]));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Real files: ffmpeg writes make/model tags the way a camera does, the import probe reads them back.
    #[test]
    fn camera_model_is_read_from_generated_files() {
        use super::super::ffpipe;
        let Some(ff) = ffpipe::ffmpeg_exe() else { return eprintln!("skipped: no ffmpeg") };
        if ffpipe::ffprobe_exe().is_none() {
            return eprintln!("skipped: no ffprobe");
        }
        let dir = std::env::temp_dir().join(format!("se-footage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            ("a7iv.mp4", vec!["-metadata", "make=Sony", "-metadata", "model=ILCE-7M4", "-metadata", "comment=Gamma S-Log3"], "Sony ILCE-7M4", "S-Log3"),
            ("iphone.mov", vec!["-metadata", "make=Apple", "-metadata", "model=iPhone 15 Pro"], "Apple iPhone 15 Pro", ""),
            ("plain.mp4", vec![], "", ""),
        ];
        for (name, tags, camera, gamma) in cases {
            let out = dir.join(name);
            let ok = ffpipe::command(&ff)
                .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=25", "-t", "0.2"])
                .args(&tags)
                .args(["-movflags", "use_metadata_tags", "-pix_fmt", "yuv420p"])
                .arg(&out)
                .status()
                .unwrap()
                .success();
            assert!(ok, "ffmpeg made {name}");
            let path = out.to_string_lossy().into_owned();
            ffpipe::probe(&path).expect("probe");
            let m = of(&path).expect("the probe remembered it");
            assert_eq!((m.camera.as_str(), m.gamma), (camera, gamma), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slog3_lut_parses_and_puts_mid_grey_mid_screen() {
        use crate::engine::lut::{parse_cube, sample_trilinear};
        let lut = parse_cube(&slog3_cube()).expect("a valid cube");
        // S-Log3 18 % grey is code 420 -> decoded (420 - 64) / 876
        let x = (420.0 - 64.0) / 876.0;
        let out = sample_trilinear(&lut, [x as f32, x as f32, x as f32]);
        assert!((0.38..0.46).contains(&out[0]) && (out[0] - out[1]).abs() < 0.01, "{out:?}");
        let black = sample_trilinear(&lut, [(95.0 - 64.0) / 876.0; 3]);
        assert!(black[1] < 0.02, "S-Log3 black is black: {black:?}");
    }

    #[test]
    fn batch_fps_picks_the_rate_the_others_divide_into() {
        assert_eq!(batch_fps(&[50.0, 25.0, 50.0]), Some((25.0, true)));
        assert_eq!(batch_fps(&[24.0, 48.0]), Some((24.0, true)));
        assert_eq!(batch_fps(&[60.0, 30.0]), Some((30.0, true)));
        assert_eq!(batch_fps(&[59.94, 29.97, 119.88]), Some((29.97, true)));
        assert_eq!(batch_fps(&[25.0, 25.0]), Some((25.0, true)));
        // no even divisor: the most common rate, flagged
        assert_eq!(batch_fps(&[24.0, 30.0, 30.0]), Some((30.0, false)));
        assert_eq!(batch_fps(&[25.0, 29.97]), Some((25.0, false)), "a tie keeps the first");
        assert_eq!(batch_fps(&[29.97, 25.0, 25.0]), Some((25.0, false)));
        assert_eq!(batch_fps(&[]), None);
    }

    #[test]
    fn multiples_and_every_frame_speed() {
        assert_eq!(rate_multiple(50.0, 25.0), Some(2));
        assert_eq!(rate_multiple(120.0, 30.0), Some(4));
        assert_eq!(rate_multiple(59.94, 29.97), Some(2));
        assert_eq!(rate_multiple(30.0, 24.0), None);
        assert_eq!(rate_multiple(25.0, 25.0), None, "the same rate is not slow motion");
        assert_eq!(rate_multiple(25.0, 50.0), None);
        assert_eq!(every_frame_speed(50.0, 25.0), Some(0.5));
        assert_eq!(every_frame_speed(100.0, 25.0), Some(0.25));
    }

    #[test]
    fn camera_and_gamma_from_tags() {
        let s = json!({"pix_fmt": "yuv420p10le", "color_transfer": "bt709"});
        let f = json!({"tags": {"make": "Sony", "model": "ILCE-7M4", "comment": "Gamma: S-Log3"}});
        let m = from_probe(&s, &f, "C:/x/C0001.MP4");
        assert_eq!((m.camera.as_str(), m.gamma, m.ten_bit), ("Sony ILCE-7M4", "S-Log3", true));
        let f = json!({"tags": {"com.apple.quicktime.make": "Apple", "com.apple.quicktime.model": "iPhone 15 Pro"}});
        let s = json!({"pix_fmt": "yuv420p", "color_transfer": "arib-std-b67"});
        let m = from_probe(&s, &f, "IMG_1.MOV");
        assert_eq!((m.camera.as_str(), m.gamma, m.ten_bit), ("Apple iPhone 15 Pro", "HLG", false));
        // only the brand a camera leaves: Sony's XAVC
        let m = from_probe(&json!({}), &json!({"tags": {"major_brand": "XAVC"}}), "a6400_slog2.mp4");
        assert_eq!((m.camera.as_str(), m.gamma), ("Sony", "S-Log2"));
        assert_eq!(from_probe(&json!({}), &json!({}), "a.mp4"), FootageMeta::default());
        // model already carries the make
        let m = from_probe(&json!({}), &json!({"tags": {"Make": "Canon", "Model": "Canon EOS R6"}}), "a.mp4");
        assert_eq!(m.camera, "Canon EOS R6");
    }
}
