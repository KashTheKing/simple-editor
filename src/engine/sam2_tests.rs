//! sam2: manifest, mask-format IO, the playback-safe lookup, coverage maths, the not-installed path,
//! and (`--ignored`) a real run on a generated clip with a moving box, checked by IoU.

use super::*;
use crate::model::{Animated, Clip, ClipKind, Mask, MaskShape};

fn tmp(name: &str) -> PathBuf {
    let base = std::env::var_os("SE_TEST_TMP").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("se-sam2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_gray(path: &Path, w: u32, h: u32, px: &[u8]) {
    let f = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Grayscale);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(px).unwrap();
}

fn req(dir: &Path) -> TrackReq {
    TrackReq {
        video: "C:/x/clip.mp4".into(),
        start: 2.0,
        fps: 10.0,
        count: 5,
        seed: 2,
        points: vec![Point { x: 0.5, y: 0.25, add: true }, Point { x: 0.1, y: 0.9, add: false }],
        dir: dir.to_path_buf(),
        size: (8, 4),
    }
}

/// A 5-frame sequence: frame i covers column i of an 8×4 frame.
fn sequence(name: &str) -> (PathBuf, TrackReq) {
    let d = tmp(name);
    let r = req(&d);
    write_meta(&r, true).unwrap();
    for i in 0..5 {
        let px: Vec<u8> = (0..32).map(|k| if k % 8 == i { 255 } else { 0 }).collect();
        write_gray(&frame_path(&d, i), 8, 4, &px);
    }
    (d, r)
}

const A64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[test]
fn manifest_must_be_complete_and_stay_inside_the_folder() {
    let good = json!({"version": "1", "files": [
        {"url": "https://x/a.onnx", "path": "models/a.onnx", "sha256": A64, "size": 10},
        {"url": "bin/ort.zip", "sha256": "b".repeat(64), "size": 5,
         "take": {"runtimes/win-x64/native/onnxruntime.dll": "onnxruntime.dll"}},
    ]})
    .to_string();
    let m = parse_manifest(&good).unwrap();
    assert_eq!(m.parts.len(), 2);
    assert_eq!(m.total(), 15);
    assert_eq!(m.parts[0].sha256, "a".repeat(64), "checksums compare lower-case");
    assert_eq!(m.parts[1].take[0].1, "onnxruntime.dll");
    let one = |path: &str, sha: &str, size: u64| json!({"files": [{"url": "x", "path": path, "sha256": sha, "size": size}]}).to_string();
    for bad in [
        json!({"files": []}).to_string(),
        one("a", "short", 1),
        one("../evil.exe", A64, 1),
        one("C:/evil.exe", A64, 1),
        one("/evil.exe", A64, 1),
        one("a", A64, 0),
    ] {
        assert!(parse_manifest(&bad).is_err(), "{bad}");
    }
}

#[test]
fn the_shipped_manifest_parses() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/library/bin/sam2.json")).unwrap();
    let m = parse_manifest(&text).unwrap();
    for f in MODEL_FILES {
        assert!(m.parts.iter().any(|p| p.path == format!("models/{f}")), "{f} is downloaded");
    }
    assert!(m.parts.iter().any(|p| p.path == EXE), "the helper");
    assert!(m.parts.iter().any(|p| p.take.iter().any(|t| t.1 == "onnxruntime.dll")), "the runtime");
    assert!(m.parts.iter().any(|p| p.take.iter().any(|t| t.1 == "DirectML.dll")), "DirectML");
}

#[test]
fn mask_format_round_trips() {
    let (d, _) = sequence("format");
    let m = read_meta(&d).unwrap();
    assert_eq!(m, Meta { width: 8, height: 4, fps: 10.0, start: 2.0, count: 5, done: true });
    let v: Value = serde_json::from_str(&std::fs::read_to_string(d.join("mask.json")).unwrap()).unwrap();
    assert_eq!(v["format"], "se-matte");
    assert_eq!(v["version"], 1);
    assert_eq!(v["points"][1][2], 0);
    assert_eq!(v["seed"], 2);
    // frame i shows source second start + i/fps; outside the range clamps
    assert_eq!(m.index(2.0), 0);
    assert_eq!(m.index(2.25), 2);
    assert_eq!(m.index(1.0), 0);
    assert_eq!(m.index(99.0), 4);
    let f = decode_png(&frame_path(&d, 3), 3).unwrap();
    assert_eq!((f.w, f.h, f.idx), (8, 4, 3));
    assert_eq!(f.px[3], 255);
    assert_eq!(f.px[4], 0);
    // a folder that is not a mask sequence
    std::fs::write(d.join("mask.json"), "{\"format\":\"other\"}").unwrap();
    assert!(read_meta(&d).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn points_parse_and_reject_out_of_frame() {
    let p = parse_points(&json!([[0.2, 0.3, 1], [0.9, 0.1, 0], [0.5, 0.5]])).unwrap();
    assert_eq!(p.len(), 3);
    assert!(p[0].add && !p[1].add && p[2].add);
    assert!(parse_points(&json!([[1.5, 0.3, 1]])).is_err());
    assert!(parse_points(&json!("x")).is_err());
    assert_eq!(points_json(&p)[1][2], 0);
}

#[test]
fn matte_sizes_keep_the_aspect_and_cap_the_long_side() {
    assert_eq!(matte_size(1920, 1080), (1024, 576));
    assert_eq!(matte_size(1080, 1920), (576, 1024));
    assert_eq!(matte_size(640, 360), (640, 360));
    assert_eq!(matte_size(0, 0), (1, 1));
}

#[test]
fn a_different_prompt_gets_a_different_cache_folder() {
    let d = PathBuf::from("x");
    let a = req(&d);
    let mut b = a.clone();
    assert_eq!(matte_dir(&a), matte_dir(&b), "stable");
    b.points[0].x = 0.6;
    assert_ne!(matte_dir(&a), matte_dir(&b));
    let mut c = a.clone();
    c.start = 2.5;
    assert_ne!(matte_dir(&a), matte_dir(&c));
}

#[test]
fn playback_never_decodes_and_falls_back_to_the_last_good_frame() {
    let (d, _) = sequence("fallback");
    forget(&d);
    // playing with nothing decoded: nothing to show yet (the mask lets the layer through)
    assert!(frame(&d, 2.0, false).is_none(), "the playback path never decodes in place");
    // off playback the exact frame is decoded in place
    forget(&d);
    let f1 = frame(&d, 2.1, true).unwrap();
    assert_eq!(f1.idx, 1);
    // playing at frame 3 before it is decoded: the last good one comes back at once
    forget(&d);
    let _ = frame(&d, 2.1, true);
    let t = Instant::now();
    let f = frame(&d, 2.3, false).unwrap();
    assert!(f.idx == 1 || f.idx == 3, "the decoded neighbour, or 3 if the worker already got to it");
    assert!(t.elapsed() < Duration::from_millis(50));
    // the worker decodes ahead; soon the exact frame is there
    let t = Instant::now();
    while !frame(&d, 2.3, false).is_some_and(|f| f.idx == 3) {
        assert!(t.elapsed() < Duration::from_secs(5), "the decode worker never delivered frame 3");
        std::thread::sleep(Duration::from_millis(10));
    }
    // a frame a running track has not written yet: the nearest earlier one stands in
    std::fs::remove_file(frame_path(&d, 4)).unwrap();
    forget(&d);
    let _ = frame(&d, 2.2, true);
    assert_eq!(frame(&d, 2.4, true).unwrap().idx, 2, "frame 4 is missing: frame 2 stands in");
    let _ = std::fs::remove_dir_all(&d);
}

fn object_clip(dir: &Path) -> (Clip, Mask) {
    let mut c = Clip::new(1, ClipKind::Video, "x", 10.0, 1.0);
    c.src_in = 2.0; // timeline 10.0 shows source 2.0 = frame 0
    let mut m = Mask::new(MaskShape::Object);
    m.matte = dir.to_string_lossy().into_owned();
    (c, m)
}

#[test]
fn layer_coverage_follows_the_frame_and_inverts() {
    let (d, _) = sequence("layer");
    let (c, mut m) = object_clip(&d);
    let mut out = Vec::new();
    // timeline 10.2 = source 2.2 = frame 2: column 2 of 8 is inside
    assert!(coverage_layer(&m, &c, 10.2, 8, 4, &mut out));
    assert_eq!(out[2], 255);
    assert_eq!(out[5], 0);
    m.invert = true;
    assert!(coverage_layer(&m, &c, 10.2, 8, 4, &mut out));
    assert_eq!((out[2], out[5]), (0, 255));
    m.invert = false;
    m.opacity = Animated::new(0.5);
    assert!(coverage_layer(&m, &c, 10.2, 8, 4, &mut out));
    assert_eq!(out[2], 128);
    // no folder: nothing to apply
    let mut r = m.clone();
    r.matte.clear();
    assert!(!coverage_layer(&r, &c, 10.2, 8, 4, &mut out));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn canvas_coverage_maps_through_the_placement() {
    let (d, _) = sequence("canvas");
    let (c, m) = object_clip(&d);
    let mut out = Vec::new();
    // an 8×4 layer drawn 2× at the centre of a 32×16 canvas: frame 0 (column 0) lands at x 8..10
    assert!(coverage_canvas(&m, &c, 10.0, 32, 16, (16.0, 8.0, 16.0, 8.0, 0.0), &mut out));
    let row = &out[8 * 32..9 * 32];
    assert_eq!(row[8], 255);
    assert_eq!(row[12], 0);
    assert_eq!(row[2], 0, "outside the layer");
    // rotated 180°: column 0 ends up on the right
    assert!(coverage_canvas(&m, &c, 10.0, 32, 16, (16.0, 8.0, 16.0, 8.0, 180.0), &mut out));
    let row = &out[8 * 32..9 * 32];
    assert_eq!(row[23], 255);
    assert_eq!(row[9], 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn iou_counts_half_coverage() {
    assert_eq!(iou(&[255, 0, 200, 0], &[255, 0, 0, 0]), 0.5);
    assert_eq!(iou(&[0, 0], &[0, 0]), 1.0);
}

#[test]
fn not_installed_fails_fast_without_spawning_anything() {
    // only an exe, no runtime or models beside it: not an install
    let d = tmp("noinstall");
    std::fs::write(d.join(EXE), b"not a helper").unwrap();
    assert!(!complete(&d));
    let _ = std::fs::remove_dir_all(&d);
    if available() {
        eprintln!("an add-on is installed for this profile: skipping the not-installed half");
        return;
    }
    assert!(exe().is_none());
    let t = Instant::now();
    let e = segment("C:/x/a.mp4", 0.0, &[Point { x: 0.5, y: 0.5, add: true }], (8, 8), &|| false).unwrap_err();
    assert!(e.contains("not installed") || e.contains("ffmpeg"), "{e}");
    let d = tmp("noinstall-track");
    let r = req(&d);
    let e = track(&r, &Progress::new()).unwrap_err();
    assert!(e.contains("not installed") || e.contains("ffmpeg"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(2), "it failed fast");
    assert!(device().is_none(), "no helper was started");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn install_from_a_folder_library_checks_every_part() {
    let lib = tmp("lib");
    let bin = tmp("bin");
    std::fs::create_dir_all(lib.join("bin")).unwrap();
    let mut files = Vec::new();
    for (name, body) in [("se-sam2.exe", "x"), ("onnxruntime.dll", "y")] {
        let f = lib.join("bin").join(name);
        std::fs::write(&f, body).unwrap();
        let sha = blitz::sha256_of(&f).unwrap();
        files.push(json!({"url": format!("bin/{name}"), "path": name, "sha256": sha, "size": body.len()}));
    }
    // a wrong checksum is refused, and nothing is swapped in
    let mut bad = files.clone();
    bad[1]["sha256"] = json!("0".repeat(64));
    std::fs::write(lib.join(MANIFEST), json!({"version": "t", "files": bad}).to_string()).unwrap();
    let src = crate::engine::library::Source::Dir(lib.clone());
    let e = install_from(&src, &bin, &Progress::new()).unwrap_err();
    assert!(e.contains("checksum"), "{e}");
    assert!(!bin.join("sam2").exists());
    // a download without the models is refused before it is swapped in; verified parts are kept
    std::fs::write(lib.join(MANIFEST), json!({"version": "t", "files": files}).to_string()).unwrap();
    let e = install_from(&src, &bin, &Progress::new()).unwrap_err();
    assert!(e.contains("missing"), "{e}");
    assert!(!bin.join("sam2").exists());
    assert!(bin.join("sam2.dl").is_dir(), "verified parts stay for the next try");
    let _ = std::fs::remove_dir_all(&lib);
    let _ = std::fs::remove_dir_all(&bin);
}

/// The real thing: SAM 2 on a generated 3 s clip (20 fps) of an orange box moving over noise, clicked
/// on the middle frame, IoU against the known box on every frame. Needs the add-on (installed, or
/// `SE_SAM2` = a se-sam2.exe with its runtime and models beside it) and ffmpeg.
/// `cargo test sam2_real -- --ignored --nocapture` (`SE_SAM2_CPU=1` for the CPU).
#[test]
#[ignore]
fn sam2_real_run_tracks_a_moving_box() {
    let ffmpeg = ffpipe::ffmpeg_exe().expect("ffmpeg");
    assert!(available(), "install the add-on or set SE_SAM2");
    let d = tmp("real");
    let video = d.join("moving.mp4");
    let st = std::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=0x305070:s=640x360:d=3:r=20", "-f", "lavfi", "-i"])
        .arg("color=c=0xF08020:s=120x100:d=3:r=20")
        .arg("-filter_complex")
        .arg("[0]noise=alls=12:allf=t[bg];[bg][1]overlay=x='40+150*t':y='120+20*sin(2*t)':shortest=1")
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "20"])
        .arg(&video)
        .status()
        .unwrap();
    assert!(st.success());
    let gt = |n: usize| {
        let t = n as f64 / 20.0;
        let (x, y) = ((40.0 + 150.0 * t).round() as usize, (120.0 + 20.0 * (2.0 * t).sin()).round() as usize);
        let mut v = vec![0u8; 640 * 360];
        for yy in y..(y + 100).min(360) {
            for xx in x..(x + 120).min(640) {
                v[yy * 640 + xx] = 255;
            }
        }
        v
    };
    let seed = 30;
    let (bx, by) = (40.0 + 150.0 * 1.5 + 60.0, 120.0 + 20.0 * 3f64.sin() + 50.0);
    let pts = vec![Point { x: (bx / 640.0) as f32, y: (by / 360.0) as f32, add: true }];
    // the click: one frame
    let t = Instant::now();
    let (m, reply) = segment(&video.to_string_lossy(), seed as f64 / 20.0, &pts, (640, 360), &|| false).unwrap();
    let click = iou(&m.px, &gt(seed));
    eprintln!("click: IoU {click:.3}, {:.1} s with the model load, helper {reply}", t.elapsed().as_secs_f64());
    assert!(click > 0.9, "click IoU {click}");
    let t = Instant::now();
    let pts2 = vec![pts[0], Point { x: 0.05, y: 0.05, add: false }];
    let (_, reply) = segment(&video.to_string_lossy(), seed as f64 / 20.0, &pts2, (640, 360), &|| false).unwrap();
    eprintln!("second click on the same frame: {:.2} s, helper {reply}", t.elapsed().as_secs_f64());
    // the track, both ways from the seed
    let r = TrackReq {
        video: video.to_string_lossy().into_owned(),
        start: 0.0,
        fps: 20.0,
        count: 60,
        seed,
        points: pts,
        dir: d.join("masks"),
        size: (640, 360),
    };
    let t = Instant::now();
    let reply = track(&r, &Progress::new()).unwrap();
    let secs = t.elapsed().as_secs_f64();
    assert!(read_meta(&r.dir).unwrap().done);
    let mut worst = 1.0f64;
    for n in 0..60 {
        let f = decode_png(&frame_path(&r.dir, n), n).unwrap();
        worst = worst.min(iou(&f.px, &gt(n)));
    }
    eprintln!(
        "track: 60 frames in {secs:.1} s ({:.2} s/frame) on {}, worst IoU {worst:.3}; per frame {}",
        secs / 60.0,
        device().unwrap_or_default(),
        reply["per_frame_ms"]
    );
    assert!(worst > 0.85, "worst IoU {worst}");
    drop_helper();
    let _ = std::fs::remove_dir_all(&d);
}
