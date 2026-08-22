//! `simple-editor --selftest [dir]` — headless end-to-end check (debug builds print to the console).
//! Generates synthetic media with ffmpeg (solid colour segments + two sine audio streams), then verifies:
//! probe → project layout; video decode at known times (colour & seek accuracy) for both backends;
//! audio decode RMS; compositor (blend/opacity/text layer); mixer; waveform peaks; export (mp4) →
//! ffprobe duration; lossless cut; xmeml is well-formed. Prints PASS/FAIL lines, returns 0 on success.
//!
//! Round 3 adds headless checks that need no GL context: shape rasterising, the effect stack on the CPU,
//! the mixer's bus graph with a filter, an xmeml export imported back, the pre-render cache and
//! markers / labels / paste-attributes.
//!
//! Every step runs under `catch_unwind`, so a panic in one module is reported (FAIL — or SKIP when the
//! module is still an unimplemented stub) and the remaining steps still run.

use crate::engine::compose::Compositor;
use crate::engine::export::{self, ExportOptions, Progress};
use crate::engine::mixer::Mixer;
use crate::engine::text::TextRasterizer;
use crate::engine::xmeml;
use crate::media::waveform::WaveformCache;
use crate::media::{self, ffpipe, Backend, DecoderPool, Frame};
use crate::model::{BlendMode, Project, TextStyle, TrackKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type R = Result<(), String>;

/// Last panic message + location, recorded by the quiet panic hook and printed in the FAIL line.
static PANIC_AT: Mutex<String> = Mutex::new(String::new());

macro_rules! check {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err(format!($($arg)+));
        }
    };
}

pub fn run(args: &[String]) -> i32 {
    std::panic::set_hook(Box::new(|info| {
        let p = info.payload();
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        *PANIC_AT.lock().unwrap() = format!("{msg}{at}");
    }));

    let Some(ffmpeg) = ffpipe::ffmpeg_exe() else {
        println!("FAIL ffmpeg: ffmpeg.exe not found");
        println!("SELFTEST FAILED (1)");
        return 1;
    };
    let dir = args.first().map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("simple-editor-selftest"));
    let mp4 = dir.join("test.mp4").to_string_lossy().into_owned();
    let png = dir.join("logo.png").to_string_lossy().into_owned();
    println!("selftest dir: {}", dir.display());
    let mut fails = 0u32;

    // 1. media: red 0–2 s, green 2–4 s, 320x240 30 fps h264; sine 440 Hz (eng) + 880 Hz (Music), both AAC.
    step(&mut fails, "generate", || {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        #[rustfmt::skip]
        ff(&ffmpeg, &[
            "-f", "lavfi", "-i", "color=red:s=320x240:d=2",
            "-f", "lavfi", "-i", "color=lime:s=320x240:d=2",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=4,volume=4",
            "-f", "lavfi", "-i", "sine=frequency=880:duration=4,volume=4",
            "-filter_complex", "[0:v][1:v]concat=n=2:v=1[v]",
            "-map", "[v]", "-map", "2:a", "-map", "3:a",
            "-r", "30", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-c:a", "aac",
            "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Music",
            &mp4,
        ])?;
        #[rustfmt::skip]
        ff(&ffmpeg, &["-f", "lavfi", "-i", "color=blue:s=64x48:d=1", "-frames:v", "1", &png])?;
        Ok(())
    });
    if fails > 0 {
        println!("SELFTEST FAILED (1)");
        return 1;
    }

    // 2. + 3. probe / video / audio per backend.
    for b in [Backend::Ffmpeg, Backend::Mf] {
        let bn = bname(b);
        step(&mut fails, &format!("probe/{bn}"), || {
            let a = media::probe(&mp4, b)?;
            check!(near(a.duration, 4.0, 0.1), "duration {}", a.duration);
            check!(a.width == 320 && a.height == 240, "size {}x{}", a.width, a.height);
            check!(near(a.fps, 30.0, 0.5), "fps {}", a.fps);
            check!(a.audio_streams.len() == 2, "{} audio streams", a.audio_streams.len());
            Ok(())
        });
        step(&mut fails, &format!("video/{bn}"), || {
            let mut v = media::open_video(&mp4, b)?;
            let mut f = Frame::default();
            check!(v.frame_at(0.5, 320, 240, &mut f), "frame_at(0.5) returned false");
            check!(is_red(px(&f, 160, 120)), "t=0.5 centre {:?}, expected red", px(&f, 160, 120));
            check!(v.frame_at(2.5, 320, 240, &mut f), "frame_at(2.5) returned false");
            check!(is_green(px(&f, 160, 120)), "t=2.5 centre {:?}, expected green", px(&f, 160, 120));
            check!(v.frame_at(0.5, 320, 240, &mut f), "frame_at(0.5) again returned false");
            check!(is_red(px(&f, 160, 120)), "t=0.5 again centre {:?}, expected red", px(&f, 160, 120));
            check!(v.frame_at(1.0, 160, 120, &mut f), "frame_at(1.0, 160x120) returned false");
            check!(
                f.width == 160 && f.height == 120 && f.rgba.len() == 160 * 120 * 4,
                "scaled frame {}x{} ({} bytes)",
                f.width,
                f.height,
                f.rgba.len()
            );
            let t0 = Instant::now();
            let mut ok = 0;
            for i in 0..60 {
                ok += v.frame_at(i as f64 / 30.0, 320, 240, &mut f) as u32;
            }
            println!("  {bn}: 60 sequential frame_at calls in {} ms", t0.elapsed().as_millis());
            check!(ok == 60, "only {ok}/60 sequential frames decoded");
            Ok(())
        });
        step(&mut fails, &format!("audio/{bn}"), || {
            let mut buf = vec![0f32; 4800 * 2];
            let mut a = media::open_audio(&mp4, 0, b)?;
            a.read_at(1.0, &mut buf);
            let r = rms(&buf);
            check!((0.1..=1.0).contains(&r), "stream 0 rms at 1.0 = {r}");
            a.read_at(10.0, &mut buf);
            check!(buf.iter().all(|s| *s == 0.0), "stream 0 at 10.0 not silent (rms {})", rms(&buf));
            let mut a1 = media::open_audio(&mp4, 1, b)?;
            a1.read_at(1.0, &mut buf);
            let r = rms(&buf);
            check!((0.1..=1.0).contains(&r), "stream 1 rms at 1.0 = {r}");
            Ok(())
        });
    }

    // 4. project + compositor + mixer (shared state; later steps fail with a reason if earlier ones did).
    let probe = || media::probe(&mp4, Backend::Auto);
    let mut project = Project::new();
    let mut pool = DecoderPool::new(Backend::Auto);
    let mut comp = Compositor::new();
    let mut text = TextRasterizer::new();
    let mut frame = Frame::default();
    step(&mut fails, "project", || {
        project = Project::from_media(probe()?);
        let names: Vec<&str> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        check!(names == ["V1", "A1", "A2"], "tracks {names:?}");
        check!(near(project.duration(), 4.0, 0.1), "duration {}", project.duration());
        let n = project.split_at(2.0, None).len();
        check!(n == 3, "split_at(2.0) made {n} clips, expected 3");
        Ok(())
    });
    step(&mut fails, "compose", || {
        comp.render(&project, 1.0, 320, 240, &mut pool, &mut text, &mut frame);
        check!(frame.width == 320 && frame.height == 240, "canvas {}x{}", frame.width, frame.height);
        check!(is_red(px(&frame, 160, 120)), "t=1.0 centre {:?}, expected red", px(&frame, 160, 120));
        comp.render(&project, 3.0, 320, 240, &mut pool, &mut text, &mut frame);
        check!(is_green(px(&frame, 160, 120)), "t=3.0 centre {:?}, expected green", px(&frame, 160, 120));
        Ok(())
    });
    step(&mut fails, "compose/blend", || {
        let aid = project.add_asset(media::probe(&png, Backend::Auto)?);
        let ti = project.add_track(TrackKind::Video);
        let ids = project.insert_asset_clips(aid, 0.0, Some(ti));
        let c = ids.first().and_then(|&id| project.clip_mut(id)).ok_or("no image clip inserted")?;
        c.scale.value = 0.25;
        c.blend = BlendMode::Multiply;
        c.opacity.value = 1.0;
        comp.render(&project, 1.0, 320, 240, &mut pool, &mut text, &mut frame);
        let c = px(&frame, 160, 120);
        check!(c[0] < 60 && c[2] < 60, "centre {c:?}, expected red*blue (dark)");
        check!(is_red(px(&frame, 5, 5)), "corner {:?}, expected red", px(&frame, 5, 5));
        Ok(())
    });
    step(&mut fails, "compose/text", || {
        let id = project.add_text_clip(1.0, 2.0);
        let c = project.clip_mut(id).ok_or("no text clip")?;
        c.text = Some(TextStyle { text: "Hi".into(), size: 40.0, outline_width: 2.0, ..TextStyle::default() });
        comp.render(&project, 1.0, 320, 240, &mut pool, &mut text, &mut frame);
        let white = (60..180)
            .flat_map(|y| (100..220).map(move |x| (x, y)))
            .filter(|&(x, y)| px(&frame, x, y).iter().take(3).all(|&c| c > 200))
            .count();
        check!(white > 0, "no white pixels near the centre");
        Ok(())
    });
    step(&mut fails, "mixer", || {
        let mut mixer = Mixer::new();
        let mut buf = vec![0f32; 4800 * 2];
        mixer.mix(&project, 1.0, &mut pool, &mut buf);
        let r = rms(&buf);
        check!(r > 0.05, "rms {r} with A1+A2");
        for i in project.audio_tracks() {
            project.tracks[i].muted = true;
        }
        mixer.mix(&project, 1.0, &mut pool, &mut buf);
        check!(buf.iter().all(|s| *s == 0.0), "rms {} with all muted, expected 0", rms(&buf));
        let a2 = *project.audio_tracks().last().ok_or("no audio tracks")?;
        project.tracks[a2].solo = true;
        mixer.mix(&project, 1.0, &mut pool, &mut buf);
        let r = rms(&buf);
        check!(r > 0.05, "rms {r} with A2 solo");
        Ok(())
    });

    // 5. waveform peaks (background compute).
    step(&mut fails, "waveform", || {
        let mut wf = WaveformCache::new(eframe::egui::Context::default(), Backend::Auto);
        let t0 = Instant::now();
        let mut peaks = wf.get(&mp4, 0);
        check!(t0.elapsed() < Duration::from_millis(500), "first get blocked for {:?}", t0.elapsed());
        let peaks = loop {
            if let Some(p) = peaks {
                break p;
            }
            check!(t0.elapsed() < Duration::from_secs(5), "no peaks after 5 s");
            std::thread::sleep(Duration::from_millis(50));
            peaks = wf.get(&mp4, 0);
        };
        check!((380..=420).contains(&peaks.len()), "peaks len {}, expected ≈400", peaks.len());
        let mx = peaks.max.iter().cloned().fold(0.0f32, f32::max);
        check!(mx > 0.1, "peak max {mx}");
        Ok(())
    });

    // 6. export (re-encode) mp4 + wav of [0.5, 3.5).
    let opts = |out: &Path| ExportOptions {
        out_path: out.to_path_buf(),
        encoder: "auto".into(),
        crf: 23,
        preset: "ultrafast".into(),
        backend: Backend::Auto,
        out_size: None,
        scaler: "bicubic".into(),
        frames: crate::engine::export::FrameSource::Cpu,
        metadata: Vec::new(),
    };
    let trimmed = |a: f64, b: f64| -> Result<Project, String> {
        let mut p = Project::from_media(probe()?);
        p.trim_to_range(a, b);
        Ok(p)
    };
    step(&mut fails, "export/mp4", || {
        let out = dir.join("export.mp4");
        let _ = std::fs::remove_file(&out);
        let prog = export::start_export(trimmed(0.5, 3.5)?, opts(&out), Arc::new(Mutex::new(TextRasterizer::new())));
        wait_done(&prog, Duration::from_secs(120))?;
        check!(out.is_file(), "output missing");
        let out = out.to_string_lossy().into_owned();
        let d = ffprobe_duration(&out)?;
        check!(near(d, 3.0, 0.2), "duration {d}, expected 3.0");
        let mut v = media::open_video(&out, Backend::Auto)?;
        let mut f = Frame::default();
        check!(v.frame_at(0.5, 320, 240, &mut f), "frame_at(0.5) returned false");
        check!(is_red(px(&f, 160, 120)), "t=0.5 centre {:?}, expected red", px(&f, 160, 120));
        check!(v.frame_at(2.5, 320, 240, &mut f), "frame_at(2.5) returned false");
        check!(is_green(px(&f, 160, 120)), "t=2.5 centre {:?}, expected green", px(&f, 160, 120));
        Ok(())
    });
    step(&mut fails, "export/wav", || {
        let out = dir.join("export.wav");
        let _ = std::fs::remove_file(&out);
        let prog = export::start_export(trimmed(0.5, 3.5)?, opts(&out), Arc::new(Mutex::new(TextRasterizer::new())));
        wait_done(&prog, Duration::from_secs(120))?;
        let d = ffprobe_duration(&out.to_string_lossy())?;
        check!(near(d, 3.0, 0.2), "duration {d}, expected 3.0");
        Ok(())
    });

    // 7. lossless cut of [1.0, 3.0) — lands on keyframes, so only a loose duration check.
    step(&mut fails, "lossless", || {
        let p = trimmed(1.0, 3.0)?;
        let segs = export::lossless_segments(&p).ok_or("lossless_segments returned None for a plain cut")?;
        check!(
            segs.len() == 1 && near(segs[0].0, 1.0, 0.01) && near(segs[0].1, 2.0, 0.01),
            "segments {segs:?}, expected [(1.0, 2.0)]"
        );
        let out = dir.join("cut.mp4");
        let _ = std::fs::remove_file(&out);
        let prog = export::start_lossless_cut(p, out.clone());
        wait_done(&prog, Duration::from_secs(60))?;
        let d = ffprobe_duration(&out.to_string_lossy())?;
        check!((1.5..=3.5).contains(&d), "duration {d}, expected 1.5..3.5");
        Ok(())
    });

    // 8. xmeml.
    step(&mut fails, "xmeml", || {
        let xml = xmeml::export_xmeml(&Project::from_media(probe()?));
        check!(xml.contains("<xmeml"), "no <xmeml element");
        let p = mp4.replace('\\', "/");
        check!(xml.contains(&p) || xml.contains(&p.replace(' ', "%20")), "asset path {p} missing");
        Ok(())
    });

    // 9. round-3 engine pieces. None of these need a GL context; a module that is still a `todo!()`
    // stub is reported as SKIP (see `step`) so the rest of the check still runs.
    step(&mut fails, "shapes", || {
        use crate::engine::shapes::ShapeRasterizer;
        use crate::model::{ShapeKind, ShapeStyle};
        let mut style = ShapeStyle::new(ShapeKind::Rect);
        style.fill = [255, 0, 0, 255];
        style.stroke = [0, 0, 0, 0];
        style.w.value = 100.0;
        style.h.value = 60.0;
        // ShapeStyle.w/h are HALF sizes (see model::ShapeStyle), so the layer is twice that.
        let (w, h) = ShapeRasterizer::size(&style, 0.0);
        check!(near(w as f64, 200.0, 8.0) && near(h as f64, 120.0, 8.0), "size {w}x{h}, expected ≈200x120");
        let mut r = ShapeRasterizer::new();
        let f = r.render(&style, 1.0, 0.0);
        check!(!f.is_empty(), "empty raster");
        let c = px(&f, f.width / 2, f.height / 2);
        check!(is_red(c) && c[3] > 200, "centre {c:?}, expected opaque red");
        // a second call at the same size/time is served from the cache (same pixels)
        let f2 = r.render(&style, 1.0, 0.0);
        check!(f2.rgba == f.rgba, "cached raster differs");
        Ok(())
    });
    step(&mut fails, "effects/round3", || {
        use crate::engine::effects;
        use crate::model::{Effect, EffectKind};
        let mut base = Frame::new(64, 48);
        for (i, px) in base.rgba.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[(i % 251) as u8, (i % 97) as u8, 200, 255]);
        }
        let mut scratch = Frame::default();
        let mut changed = 0;
        for kind in EffectKind::ALL {
            let mut img = base.clone();
            effects::apply(&Effect::new(kind), 0.25, 1.0, &mut img, &mut scratch);
            check!(img.width == 64 && img.height == 48, "{} resized the layer", kind.name());
            check!(img.rgba.len() == 64 * 48 * 4, "{} broke the buffer", kind.name());
            check!(img.rgba.chunks_exact(4).all(|p| p[3] > 0), "{} zeroed the alpha", kind.name());
            changed += (img.rgba != base.rgba) as u32;
        }
        // geometric effects move the layer instead of touching pixels; GPU-only kinds are no-ops on the CPU
        check!(changed >= 5, "only {changed} of {} effects changed anything", EffectKind::ALL.len());
        println!("  {changed}/{} effect kinds change pixels on the CPU path", EffectKind::ALL.len());
        Ok(())
    });
    step(&mut fails, "mixer/bus", || {
        use crate::engine::mixer_fx::BusGraph;
        use crate::model::{AudioFilter, FilterKind};
        let mut p = Project::new();
        let main = p.main_bus();
        let bus = p.add_bus("Music");
        p.bus_mut(bus).ok_or("no bus")?.filters.push(AudioFilter::new(FilterKind::Gain));
        let mut g = BusGraph::new();
        g.sync(&p);
        let order = g.order();
        check!(order.last() == Some(&main), "evaluation order {order:?}, Main must be last");
        check!(order.contains(&bus), "the new bus is missing from {order:?}");
        let frames = 512;
        for (i, s) in g.buffer(bus, frames).iter_mut().enumerate() {
            *s = if (i / 2) % 2 == 0 { 0.5 } else { -0.5 };
        }
        let (music, main_bus) = (p.bus(bus).ok_or("no bus")?.clone(), p.bus(main).ok_or("no main")?.clone());
        let mut out = vec![0f32; frames * 2];
        g.flush(&music, 0.0, &mut out); // a sub-bus adds into its output bus (Main)
        g.flush(&main_bus, 0.0, &mut out);
        let r = rms(&out);
        check!(r > 0.05, "rms {r} out of the Main bus");
        let (l, _) = g.meter(bus);
        check!(l > 0.05, "bus meter {l}");
        Ok(())
    });
    step(&mut fails, "import/roundtrip", || {
        let path = dir.join("roundtrip.xml");
        let mut p = Project::from_media(probe()?);
        p.split_at(2.0, None);
        std::fs::write(&path, xmeml::export_xmeml(&p)).map_err(|e| e.to_string())?;
        let r = crate::engine::import::import_file(&path)?;
        check!(r.clips >= 2, "{} clips imported, expected the 2+ we exported", r.clips);
        check!(r.tracks >= 1, "{} tracks", r.tracks);
        check!(near(r.project.duration(), p.duration(), 0.2), "duration {} vs {}", r.project.duration(), p.duration());
        check!(r.to_markdown().contains('|'), "the report is not a markdown table");
        Ok(())
    });
    step(&mut fails, "prerender", || {
        use crate::engine::prerender::PreRender;
        let p = Project::from_media(probe()?);
        let mut pr = PreRender::new();
        pr.request(&p, 0.0, 1.0);
        let t0 = Instant::now();
        while pr.tick(&p, 8.0) {
            check!(t0.elapsed() < Duration::from_secs(30), "pre-render did not finish in 30 s");
        }
        check!(pr.progress() > 0.99, "progress {}", pr.progress());
        let f = pr.frame(&p, 0.5).ok_or("no cached frame at 0.5 s")?;
        check!(!f.is_empty(), "empty cached frame");
        check!(is_red(px(&f, f.width / 2, f.height / 2)), "cached frame at 0.5 s is not red");
        pr.clear();
        check!(pr.frame(&p, 0.5).is_none(), "clear() left the cache in place");
        Ok(())
    });
    step(&mut fails, "markers/labels/paste", || {
        use crate::model::{AttrSet, BlendMode as Bm, Effect, EffectKind};
        let mut p = Project::from_media(probe()?);
        // markers: project + clip, listed together in timeline order
        let clip = p.all_clips().next().ok_or("no clips")?.1.id;
        let m1 = p.add_marker(3.0, "three");
        let m0 = p.add_marker(1.0, "one");
        let mc = p.add_clip_marker(clip, 0.5, "on the clip").ok_or("no clip marker")?;
        let list = p.markers_in_timeline();
        let times: Vec<f64> = list.iter().map(|m| m.1).collect();
        check!(times.windows(2).all(|w| w[0] <= w[1]), "markers are not in timeline order: {times:?}");
        check!(list.len() == 3, "{} markers, expected 3", list.len());
        p.marker_mut(mc).ok_or("marker gone")?.label = 2;
        p.remove_marker(m0);
        check!(p.markers_in_timeline().len() == 2, "remove_marker did not remove one");
        check!(p.marker_mut(m1).is_some(), "the other project marker disappeared");
        // labels: add, use, remove — users of a removed label fall back to "none"
        let idx = p.add_label("Retake", [10, 20, 30]);
        check!(p.label_name(idx) == "Retake", "label name {}", p.label_name(idx));
        check!(p.label_color(idx) == Some([10, 20, 30]), "label colour");
        p.clip_mut(clip).ok_or("no clip")?.label = idx;
        p.remove_label(idx);
        check!(p.clip(clip).ok_or("no clip")?.label == 0, "a removed label must clear its users");
        check!(p.label_name(idx) == "None" || idx as usize <= p.labels.len(), "label list is inconsistent");
        // copy / paste attributes: only the ticked fields, never the timing
        let ids = p.split_at(2.0, None);
        let target = *ids.first().ok_or("split made no clip")?;
        {
            let c = p.clip_mut(clip).ok_or("no clip")?;
            c.opacity.value = 0.5;
            c.blend = Bm::Screen;
            c.effects.push(Effect::new(EffectKind::Blur));
        }
        let src = p.copy_attributes(clip).ok_or("copy_attributes returned None")?;
        let (start, dur) = {
            let c = p.clip(target).ok_or("no target")?;
            (c.start, c.duration)
        };
        let n = p.paste_attributes(&src, &[target], AttrSet { opacity: true, ..AttrSet::NONE });
        check!(n == 1, "paste_attributes changed {n} clips");
        let c = p.clip(target).ok_or("no target")?;
        check!(c.opacity.value == 0.5, "opacity {} was not pasted", c.opacity.value);
        check!(c.blend == Bm::Normal, "blend was pasted although it was not ticked");
        check!(c.effects.is_empty(), "effects were pasted although they were not ticked");
        check!(c.start == start && c.duration == dur, "timing changed ({start} {dur} -> {} {})", c.start, c.duration);
        Ok(())
    });

    if fails == 0 {
        println!("SELFTEST OK");
        0
    } else {
        println!("SELFTEST FAILED ({fails})");
        1
    }
}

/// Run one step. A `todo!()` in a module that is not written yet is reported as SKIP (it is a gap, not
/// a defect); everything else — a returned error or any other panic — is a FAIL.
fn step(fails: &mut u32, name: &str, f: impl FnOnce() -> R) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => println!("PASS {name}"),
        Ok(Err(e)) => {
            println!("FAIL {name}: {e}");
            *fails += 1;
        }
        Err(_) => {
            let at = PANIC_AT.lock().map(|s| s.clone()).unwrap_or_default();
            if at.starts_with("not yet implemented") {
                println!("SKIP {name}: {at}");
            } else {
                println!("FAIL {name}: panic: {at}");
                *fails += 1;
            }
        }
    }
}

fn bname(b: Backend) -> &'static str {
    match b {
        Backend::Mf => "mf",
        Backend::Ffmpeg => "ffmpeg",
        Backend::Auto => "auto",
    }
}

/// Run ffmpeg with `-y -v error` + args; Err = last stderr text.
fn ff(exe: &Path, args: &[&str]) -> R {
    let out = ffpipe::command(exe).args(["-y", "-v", "error"]).args(args).output().map_err(|e| e.to_string())?;
    check!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

fn ffprobe_duration(path: &str) -> Result<f64, String> {
    let exe = ffpipe::ffprobe_exe().ok_or("ffprobe.exe not found")?;
    let out = ffpipe::command(&exe)
        .args(["-v", "quiet", "-print_format", "json", "-show_format", path])
        .output()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe {path}: {e}"))?;
    v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("ffprobe: no duration for {path}"))
}

/// Poll until the worker finishes; Err on timeout, on a reported error, or when the worker thread
/// vanished without finishing (its Arc clone dropped → nothing can ever set done).
fn wait_done(p: &Arc<Progress>, timeout: Duration) -> R {
    let t0 = Instant::now();
    while !p.is_done() {
        check!(!(Arc::strong_count(p) == 1 && !p.is_done()), "worker ended without finishing ({})", p.status());
        check!(t0.elapsed() < timeout, "timeout after {:?} ({})", timeout, p.status());
        std::thread::sleep(Duration::from_millis(50));
    }
    p.error().map_or(Ok(()), Err)
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * f.width + x) * 4) as usize;
    f.rgba.get(i..i + 4).map(|p| [p[0], p[1], p[2], p[3]]).unwrap_or([0; 4])
}

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn is_green(p: [u8; 4]) -> bool {
    p[1] > 200 && p[0] < 60 && p[2] < 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        let mut f = Frame::new(2, 2);
        f.rgba[4..8].copy_from_slice(&[255, 0, 0, 255]);
        assert!(is_red(px(&f, 1, 0)));
        assert!(!is_red(px(&f, 0, 0)));
        assert_eq!(px(&f, 5, 5), [0; 4]); // out of range → zeros, no panic
        assert!((rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
        assert!(near(4.05, 4.0, 0.1) && !near(4.2, 4.0, 0.1));
    }

    #[test]
    fn step_reports_err_and_panic() {
        let mut fails = 0;
        step(&mut fails, "ok", || Ok(()));
        step(&mut fails, "err", || Err("nope".into()));
        step(&mut fails, "panic", || panic!("boom"));
        assert_eq!(fails, 2);
    }

    #[test]
    fn probe_duration_via_ffprobe() {
        let Some(ffmpeg) = ffpipe::ffmpeg_exe() else {
            return;
        };
        let dir = std::env::temp_dir().join("simple-editor-selftest-unit");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("one.wav").to_string_lossy().into_owned();
        ff(&ffmpeg, &["-f", "lavfi", "-i", "sine=frequency=440:duration=1", &p]).unwrap();
        let d = ffprobe_duration(&p).unwrap();
        assert!(near(d, 1.0, 0.05), "{d}");
        assert!(ffprobe_duration("C:/definitely/missing.mp4").is_err());
    }
}
