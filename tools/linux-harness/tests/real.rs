use linux_harness::engine::{media_info, netstream};
use linux_harness::media::io_override::{MediaIo, Range};
use std::process::Command;

fn ff(args: &[&str]) {
    let s = Command::new("ffmpeg").args(["-y", "-hide_banner", "-loglevel", "error"]).args(args).status().unwrap();
    assert!(s.success(), "{args:?}");
}
fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join("parity_io_real");
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn probe_rotated_full_range_and_bitrate() {
    let d = dir();
    let src = d.join("src.mp4");
    let rot = d.join("rot.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc=s=320x180:r=25:d=3", "-f", "lavfi", "-i", "sine=d=3",
         "-vf", "format=yuvj420p", "-color_range", "pc", "-c:v", "libx264", "-g", "25", "-c:a", "aac", "-shortest", src.to_str().unwrap()]);
    ff(&["-display_rotation", "90", "-i", src.to_str().unwrap(), "-c", "copy", rot.to_str().unwrap()]);
    let i = media_info::properties(rot.to_str().unwrap()).unwrap();
    let v = i.streams.iter().find(|s| s.kind == "video").unwrap();
    println!("{:?} {:?} rot {} range {} fps {}", i.format_long, (v.width, v.height), v.rotation, v.color_range, v.fps);
    assert_eq!((v.width, v.height), (320, 180), "coded size");
    assert!(v.rotation == 90 || v.rotation == 270, "rotation {}", v.rotation);
    assert_eq!(v.color_range, "pc");
    assert!((v.fps - 25.0).abs() < 0.01);
    assert!(i.streams.iter().any(|s| s.kind == "audio" && s.sample_rate > 0));
    assert!(i.duration > 2.9 && i.bit_rate > 0);
    let b = media_info::bitrate(rot.to_str().unwrap(), "v:0", 1.0).unwrap();
    println!("kbps {:?} keys {:?}", b.kbps, b.keys);
    assert_eq!(b.kbps.len(), 3);
    assert_eq!(b.keys.len(), 3, "a keyframe a second (-g 25)");
    assert!(b.average > 0.0 && b.peak > 0.0);
}

/// The pipe's arguments with an override, as `VideoPipe::respawn` builds them: forced upright ignores the
/// matrix (320x180 out); a 90 turn ignores it and turns the coded picture (180x320).
#[test]
fn override_filters_decode_at_the_right_size() {
    let d = dir();
    let src = d.join("src2.mp4");
    let rot = d.join("rot2.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc=s=320x180:r=25:d=1", "-c:v", "libx264", src.to_str().unwrap()]);
    ff(&["-display_rotation", "90", "-i", src.to_str().unwrap(), "-c", "copy", rot.to_str().unwrap()]);
    for (io, want) in [
        (MediaIo { rotate: Some(0), range: Range::Full }, (320u32, 180u32)),
        (MediaIo { rotate: Some(90), range: Range::Auto }, (180, 320)),
        (MediaIo { rotate: Some(270), range: Range::Limited }, (180, 320)),
        (MediaIo::default(), (180, 320)), // the file's own turn (ffmpeg autorotates)
    ] {
        let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
        args.extend(io.input_args());
        args.extend(["-i".into(), rot.to_str().unwrap().into()]);
        if let Some(vf) = io.filters() {
            args.extend(["-vf".into(), vf]);
        }
        args.extend(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"].map(String::from));
        let out = Command::new("ffmpeg").args(&args).output().unwrap();
        assert!(out.status.success(), "{:?}: {}", io, String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.stdout.len(), (want.0 * want.1 * 4) as usize, "{io:?} decodes at {want:?}");
        assert_eq!(io.display_size((320, 180), 270), want, "display_size agrees for {io:?}");
    }
}

/// Range override: a full-range file read as Limited gets darker blacks clipped differently - compare the
/// mean of a black frame decoded each way.
#[test]
fn range_override_changes_levels() {
    let d = dir();
    let src = d.join("grey.mp4");
    // a dark grey, flagged full range: Y = 16 stored
    ff(&["-f", "lavfi", "-i", "color=c=0x101010:s=64x64:d=1", "-vf", "format=yuvj420p", "-color_range", "pc", "-c:v", "libx264", "-qp", "0", src.to_str().unwrap()]);
    let mean = |io: MediaIo| {
        let mut args: Vec<String> = vec!["-loglevel".into(), "error".into(), "-i".into(), src.to_str().unwrap().into()];
        if let Some(vf) = io.filters() {
            args.extend(["-vf".into(), vf]);
        }
        args.extend(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"].map(String::from));
        let o = Command::new("ffmpeg").args(&args).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o.stdout.chunks(4).map(|p| p[0] as f64).sum::<f64>() / (o.stdout.len() / 4) as f64
    };
    let (auto, full, limited) = (mean(MediaIo::default()), mean(MediaIo { range: Range::Full, ..Default::default() }), mean(MediaIo { range: Range::Limited, ..Default::default() }));
    println!("auto {auto:.1} full {full:.1} limited {limited:.1}");
    assert!((full - 16.0).abs() < 3.0, "read as full: 16 stays 16 ({full})");
    assert!(limited < 4.0, "read as limited: 16 is black ({limited})");
}

/// The export's full-range chain (`export::full_range` output for the RGBA path) yields a pc-tagged file.
#[test]
fn full_range_export_chain_tags_pc() {
    let d = dir();
    let out = d.join("fr.mp4");
    let vf = "scale=out_color_matrix=bt709:out_range=pc,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=pc";
    ff(&["-f", "lavfi", "-i", "testsrc=s=64x64:d=1,format=rgba", "-vf", vf, "-c:v", "libx264", "-pix_fmt", "yuv420p",
         "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "pc", out.to_str().unwrap()]);
    let i = media_info::properties(out.to_str().unwrap()).unwrap();
    assert_eq!(i.streams[0].color_range, "pc");
    // the NV12 path's expansion
    let out2 = d.join("fr2.mp4");
    ff(&["-f", "lavfi", "-i", "testsrc=s=64x64:d=1,format=nv12", "-vf", "scale=in_range=tv:out_range=pc,setparams=range=pc",
         "-c:v", "libx264", "-pix_fmt", "nv12", "-color_range", "pc", out2.to_str().unwrap()]);
    assert_eq!(media_info::properties(out2.to_str().unwrap()).unwrap().streams[0].color_range, "pc");
}

/// Record an HTTP stream with the exact record command (served by a local python http.server), with a limit.
#[test]
fn record_http_stream() {
    let d = dir().join("www");
    std::fs::create_dir_all(&d).unwrap();
    let src = d.join("live.ts");
    ff(&["-f", "lavfi", "-i", "testsrc=s=160x90:r=25:d=4", "-f", "lavfi", "-i", "sine=d=4", "-c:v", "libx264", "-c:a", "aac", "-f", "mpegts", src.to_str().unwrap()]);
    let mut server = Command::new("python3").args(["-m", "http.server", "18731", "--bind", "127.0.0.1"]).current_dir(&d)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    let url = netstream::validate("http://127.0.0.1:18731/live.ts").unwrap();
    let probe = media_info::properties(&url);
    let out = dir().join("rec.mkv");
    let _ = std::fs::remove_file(&out);
    let st = Command::new("ffmpeg").args(netstream::record_args(&url, &out, Some(2.0))).status().unwrap();
    let _ = server.kill();
    let probe = probe.unwrap();
    assert!(probe.streams.iter().any(|s| s.kind == "video"));
    assert!(st.success());
    let rec = media_info::properties(out.to_str().unwrap()).unwrap();
    println!("recorded {:.2}s {:?}", rec.duration, rec.streams.iter().map(|s| &s.codec).collect::<Vec<_>>());
    assert!(rec.duration > 1.5 && rec.duration < 2.6, "{}", rec.duration);
    assert_eq!(rec.streams.len(), 2, "video and audio copied");
}

/// The live preview pipe delivers 640-wide frames; a Recording stops cleanly on `q` and leaves a playable file.
#[test]
fn preview_and_recording_objects() {
    let d = dir().join("www2");
    std::fs::create_dir_all(&d).unwrap();
    let src = d.join("long.ts");
    ff(&["-f", "lavfi", "-i", "testsrc=s=320x240:r=25:d=20", "-c:v", "libx264", "-f", "mpegts", src.to_str().unwrap()]);
    let mut server = Command::new("python3").args(["-m", "http.server", "18732", "--bind", "127.0.0.1"]).current_dir(&d)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    let url = "http://127.0.0.1:18732/long.ts";
    let p = netstream::Preview::start(url, || {}).unwrap();
    let t0 = std::time::Instant::now();
    let mut got = None;
    while got.is_none() && t0.elapsed().as_secs() < 10 {
        got = p.take();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let f = got.expect("a preview frame");
    assert_eq!((f.width, f.height), (640, 360));
    drop(p);
    let _ = server.kill();
    // a live source: an ffmpeg pushing real-time MPEG-TS over UDP
    let mut sender = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-re", "-f", "lavfi", "-i", "testsrc=s=320x240:r=25:d=30",
        "-c:v", "libx264", "-g", "25", "-tune", "zerolatency", "-f", "mpegts", "udp://127.0.0.1:18733?pkt_size=1316"]).spawn().unwrap();
    let url = netstream::validate("udp://127.0.0.1:18733").unwrap();
    let out = dir().join("rec2.mkv");
    let _ = std::fs::remove_file(&out);
    let mut r = netstream::Recording::start(&url, out.clone(), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert!(r.finished().is_none(), "still running");
    // Stop while ffmpeg still probes the live input: no media, and it says so
    r.stop();
    let t0 = std::time::Instant::now();
    while r.finished().is_none() && t0.elapsed().as_secs() < 10 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(matches!(r.finished(), Some(Err(e)) if e.contains("nothing was received")), "{:?}", r.finished());
    let out = dir().join("rec3.mkv");
    let _ = std::fs::remove_file(&out);
    let mut r = netstream::Recording::start(&url, out.clone(), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(8000));
    assert!(r.finished().is_none(), "still running");
    r.stop();
    let t0 = std::time::Instant::now();
    while r.finished().is_none() && t0.elapsed().as_secs() < 10 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = sender.kill();
    assert_eq!(r.finished(), Some(Ok(())));
    let i = media_info::properties(out.to_str().unwrap()).unwrap();
    println!("stopped recording: {:.2}s, {} bytes", i.duration, r.bytes());
    assert!(i.duration > 0.5, "{}", i.duration);
}
