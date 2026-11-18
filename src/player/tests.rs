//! Player mode end to end on a real file (made with ffmpeg; skipped without it).

use super::*;
use crate::player::tracks::tests::two_audio_one_sub;
use eframe::egui::{Event, Key, Modifiers, RawInput};
use std::sync::Mutex;

/// The tests that count editors built / build one run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 540.0))),
        events,
        ..Default::default()
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// A headless player on the test file, paused at `at`, CPU frames.
fn player(at: f64) -> Option<(egui::Context, PlayerApp)> {
    let f = two_audio_one_sub()?;
    let ctx = egui::Context::default();
    let engine = Engine::start(&f, at, false, false);
    let app = PlayerApp::new(&ctx, engine);
    Some((ctx, app))
}

fn run(ctx: &egui::Context, app: &mut PlayerApp, events: Vec<Event>) {
    let _ = ctx.run(input(events), |ctx| app.frame(ctx, None));
}

/// Frames until the first picture is up (decoding is on its own thread).
fn until_picture(ctx: &egui::Context, app: &mut PlayerApp) {
    let end = Instant::now() + Duration::from_secs(10);
    while app.video.shown.is_none() && Instant::now() < end {
        run(ctx, app, vec![]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.video.shown.is_some(), "no picture: {:?}", app.error);
}

#[test]
fn player_mode_starts_with_no_editor_project_mcp_or_jobs() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let built = crate::ui::app::EDITORS_BUILT.load(std::sync::atomic::Ordering::Relaxed);
    let Some((ctx, mut app)) = player(0.0) else { return };
    until_picture(&ctx, &mut app);
    for _ in 0..5 {
        run(&ctx, &mut app, vec![Event::PointerMoved(egui::pos2(300.0, 200.0))]);
    }
    // the project, MCP server, jobs (proxies, thumbnails, waveforms, HTML), autosave and the Library
    // all live in `App`: player mode never builds one
    assert_eq!(crate::ui::app::EDITORS_BUILT.load(std::sync::atomic::Ordering::Relaxed), built);
    let info = app.info.clone().expect("file info");
    assert!(info.has_video && info.duration > 3.5, "{info:?}");
    assert_eq!(info.audio_streams.len(), 2);
    assert!(app.tracks.is_none(), "tracks are listed only when the selector opens");
}

#[test]
fn assert_no_idle_repaint_player_paused_with_controls_hidden() {
    let Some((ctx, mut app)) = player(1.0) else { return };
    until_picture(&ctx, &mut app);
    run(&ctx, &mut app, vec![Event::PointerMoved(egui::pos2(300.0, 200.0))]);
    // the pointer leaves the window: the controls fade out ...
    run(&ctx, &mut app, vec![Event::PointerGone]);
    std::thread::sleep(Duration::from_millis((autohide::FADE * 1000.0) as u64 + 60));
    for _ in 0..3 {
        run(&ctx, &mut app, vec![]);
    }
    assert!(!app.hide.shown());
    // ... and then nothing at all happens until input arrives
    for i in 0..10 {
        run(&ctx, &mut app, vec![]);
        assert!(!ctx.has_requested_repaint(), "idle frame {i}: paused, controls hidden, yet a repaint was asked for");
    }
}

/// The reported bug: moving the mouse over a paused picture stepped it on a frame per move (every UI
/// update took the next queued frame), so the footage crept forward and glitched. Pointer moves must
/// leave the frame, its place and the clock exactly where they were.
#[test]
fn pointer_moves_over_a_paused_picture_never_move_it() {
    let Some((ctx, mut app)) = player(1.0) else { return };
    until_picture(&ctx, &mut app);
    // let the decoder fill its queue ahead, so there are frames a buggy update could take
    let end = Instant::now() + Duration::from_secs(3);
    while app.engine.next_due().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    run(&ctx, &mut app, vec![]);
    let (pts, shown, t, next) = (app.video.pts, app.video.shown, app.engine.time(), app.engine.next_due());
    assert!(next.is_some(), "frames queued ahead");
    let placed = |app: &PlayerApp| app.zoom.place(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 540.0)), app.info.as_ref().unwrap().size, 1.0);
    let rect = placed(&app);
    for i in 0..40 {
        let p = egui::pos2(100.0 + i as f32 * 19.0, 120.0 + (i % 7) as f32 * 50.0);
        run(&ctx, &mut app, vec![Event::PointerMoved(p)]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.hide.shown(), "the moves bring the controls up");
    assert_eq!(app.video.pts, pts, "the picture stepped on while paused");
    assert_eq!(app.video.shown, shown, "the picture's texture / size / uv changed");
    assert_eq!(placed(&app), rect, "the picture moved");
    assert_eq!(app.engine.time(), t);
    assert_eq!(app.engine.next_due(), next, "queued frames were taken");
    assert!(!app.engine.is_playing());
}

/// Run frames for `ms` milliseconds, the way the window would.
fn run_for(ctx: &egui::Context, app: &mut PlayerApp, ms: u64, mut each: impl FnMut(&PlayerApp)) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        run(ctx, app, vec![]);
        each(app);
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// Reverse plays backwards from the playhead - the clock runs down and the picture follows it,
/// never stepping forwards - and toggled off, forwards again.
#[test]
fn reverse_runs_the_clock_and_the_picture_backwards() {
    let Some((ctx, mut app)) = player(3.0) else { return };
    until_picture(&ctx, &mut app);
    app.toggle_reverse();
    assert!(app.engine.is_reverse() && app.engine.is_playing());
    let mut shown = Vec::new();
    run_for(&ctx, &mut app, 1500, |a| shown.push(a.video.pts));
    let t = app.engine.time();
    assert!((1.2..2.0).contains(&t), "3 s, then 1.5 s backwards: at {t}");
    let pts = app.video.pts;
    assert!((t - 0.15..t + 0.25).contains(&pts), "the picture ({pts}) follows the clock ({t})");
    assert!(shown.windows(2).all(|w| w[1] <= w[0] + 1e-9), "stepped forwards while reversing: {shown:?}");
    assert!(app.engine.dropped() <= 3, "{} frames dropped reversing", app.engine.dropped());
    app.toggle_reverse();
    assert!(!app.engine.is_reverse());
    let t0 = app.engine.time();
    run_for(&ctx, &mut app, 500, |_| {});
    assert!(app.engine.time() > t0 + 0.2, "forwards again: {t0} -> {}", app.engine.time());
}

/// Loop on: the end of the file goes round to the start and keeps playing.
#[test]
fn loop_wraps_at_the_end_and_keeps_playing() {
    let Some(f) = two_audio_one_sub() else { return };
    let ctx = egui::Context::default();
    let mut app = PlayerApp::new(&ctx, Engine::start(&f, 3.2, true, false));
    app.set_loop(true);
    assert!(app.looping());
    let (mut late, mut wrapped) = (false, false);
    let end = Instant::now() + Duration::from_secs(6);
    while !wrapped && Instant::now() < end {
        run(&ctx, &mut app, vec![]);
        let t = app.engine.time();
        late |= t > 3.5;
        wrapped = late && t < 1.0;
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(wrapped, "never went round: at {}", app.engine.time());
    assert!(app.engine.is_playing() && !app.ended);
}

/// The speed scales the clock: 2x covers twice the file time, 0.5x half; 4x (too fast to hear, so on
/// the wall clock) four times.
#[test]
fn speed_scales_the_clock() {
    let Some((ctx, mut app)) = player(0.2) else { return };
    until_picture(&ctx, &mut app);
    for (rate, ms, lo, hi) in [(2.0, 600, 0.85, 1.45), (0.5, 600, 0.18, 0.42), (4.0, 300, 0.9, 1.5)] {
        app.engine.set_rate(rate);
        app.engine.set_playing(true);
        // the clock waits for the sound to start (picture follows sound): measure once it runs
        let start = app.engine.time();
        let end = Instant::now() + Duration::from_secs(3);
        while app.engine.time() <= start + 0.02 && Instant::now() < end {
            run(&ctx, &mut app, vec![]);
            std::thread::sleep(Duration::from_millis(2));
        }
        let t0 = app.engine.time();
        run_for(&ctx, &mut app, ms, |_| {});
        let dt = app.engine.time() - t0;
        app.engine.set_playing(false);
        assert!((lo..hi).contains(&dt), "{rate}x for {ms} ms moved the clock {dt} s");
        app.seek(0.2);
    }
}

#[test]
fn esc_builds_the_editor_with_the_file_on_the_timeline_at_the_current_time() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some((ctx, mut app)) = player(2.0) else { return };
    until_picture(&ctx, &mut app);
    assert!(app.take_handoff().is_none());
    run(&ctx, &mut app, vec![key(Key::Escape, Modifiers::NONE)]);
    let (path, t) = app.take_handoff().expect("Esc asks for the editor");
    assert_eq!(path, two_audio_one_sub().unwrap());
    assert!((t - 2.0).abs() < 0.05, "handed over at {t}");
    assert!(!app.engine.is_playing(), "the player pauses as it hands over");

    // the Editor button takes the same road
    let (_, mut b) = player(0.5).unwrap();
    b.open_editor();
    assert!(b.take_handoff().is_some());

    // the editor, built now, in a scratch profile
    let scratch = std::env::temp_dir().join(format!("se-player-handoff-{}", std::process::id()));
    *crate::settings::TEST_DIR.lock().unwrap() = Some(scratch.clone());
    let built = crate::ui::app::EDITORS_BUILT.load(std::sync::atomic::Ordering::Relaxed);
    let editor = crate::ui::app::App::from_player(&ctx, None, &path, t);
    *crate::settings::TEST_DIR.lock().unwrap() = None;
    assert_eq!(crate::ui::app::EDITORS_BUILT.load(std::sync::atomic::Ordering::Relaxed), built + 1);
    let (project, playhead) = editor.project_and_playhead();
    let p = path.to_string_lossy();
    let asset = project.assets.iter().find(|a| a.path == p).expect("the file is in the project");
    let on_timeline = project.tracks.iter().flat_map(|t| &t.clips).any(|c| c.asset == asset.id);
    assert!(on_timeline, "the file is on the timeline");
    assert!((playhead - t).abs() < 1e-6, "playhead {playhead} vs {t}");
    drop(editor);
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn skip_back_10_and_forward_30_move_the_clock_and_the_picture_follows() {
    let Some((ctx, mut app)) = player(2.0) else { return };
    until_picture(&ctx, &mut app);
    let dur = app.engine.duration();
    app.skip(-SKIP_BACK);
    assert_eq!(app.engine.time(), 0.0, "back 10 s from 2 s: the start");
    app.skip(SKIP_FORWARD);
    assert!((app.engine.time() - (dur - 0.001)).abs() < 1e-9, "forward 30 s in a 4 s file: its last frame");
    // the decoder seeks there and the next picture is from the end of the file
    let end = Instant::now() + Duration::from_secs(5);
    while app.engine.next_due().is_none_or(|p| p < dur - 0.2) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.engine.next_due().is_some_and(|p| p >= dur - 0.2), "next frame at {:?}", app.engine.next_due());
    // the keys do the same: Left = back 10, Right = forward 30
    app.seek(1.0);
    run(&ctx, &mut app, vec![key(Key::ArrowRight, Modifiers::NONE)]);
    assert!(app.engine.time() > 3.9);
    run(&ctx, &mut app, vec![key(Key::ArrowLeft, Modifiers::NONE)]);
    assert_eq!(app.engine.time(), 0.0);
}

#[test]
fn playing_reaches_the_end_and_stops_on_the_last_frame() {
    let Some(f) = two_audio_one_sub() else { return };
    let ctx = egui::Context::default();
    let mut app = PlayerApp::new(&ctx, Engine::start(&f, 3.0, true, false));
    app.playlist.repeat = Repeat::Off;
    app.playlist.shuffle = false;
    let end = Instant::now() + Duration::from_secs(6);
    while !app.ended && Instant::now() < end {
        run(&ctx, &mut app, vec![]);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.ended, "played to the end at t={}", app.engine.time());
    assert!(!app.engine.is_playing());
    assert!(app.video.shown.is_some(), "the last frame stays up");
}

/// Not a check: `cargo test bench_player_launch -- --ignored --nocapture` launches the built exe on a
/// 1080p60 test clip (scratch profile, a background window) and prints process start -> first frame
/// on screen (median of `SE_BENCH_RUNS`, default 7), and the editor's start -> first painted frame
/// without a file. Windows Media Player is timed from outside by `scripts/launch-bench.ps1`.
#[test]
#[ignore]
fn bench_player_launch() {
    let exe = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("simple-editor.exe");
    assert!(exe.is_file(), "build the exe first: {exe:?}");
    let ff = crate::media::ffpipe::ffmpeg_exe().expect("ffmpeg");
    let dir = std::env::temp_dir().join("se-player-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("bench-1080p60.mp4");
    if !clip.is_file() {
        let ok = crate::media::ffpipe::command(&ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=60:duration=20"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=20", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&clip)
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }
    let runs: usize = std::env::var("SE_BENCH_RUNS").ok().and_then(|s| s.parse().ok()).unwrap_or(7);
    let time = |file: Option<&Path>| -> f64 {
        let out = dir.join("first-frame.txt");
        let _ = std::fs::remove_file(&out);
        let mut c = std::process::Command::new(&exe);
        if let Some(f) = file {
            c.arg(f);
        }
        let status = c
            .env("APPDATA", dir.join("appdata"))
            .env("LOCALAPPDATA", dir.join("localappdata"))
            .env("SE_BACKGROUND", "1")
            .env("SE_BENCH_FIRST_FRAME", &out)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::read_to_string(&out).unwrap().trim().parse().unwrap()
    };
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    time(Some(&clip)); // warm the file cache
    let player: Vec<f64> = (0..runs).map(|_| time(Some(&clip))).collect();
    time(None);
    let editor: Vec<f64> = (0..runs).map(|_| time(None)).collect();
    println!("player: launch -> first frame on screen, median {:.0} ms of {player:.0?}", median(player.clone()));
    println!("editor: launch -> first painted frame, median {:.0} ms of {editor:.0?}", median(editor.clone()));
}

/// The visualizer pick goes into the editor's settings.json under `visualizer_style`, every other key
/// kept as it was (and read back by the editor's `Settings`).
#[test]
fn the_visualizer_style_pick_is_saved_in_the_editors_settings() {
    let p = std::env::temp_dir().join(format!("se-player-settings-{}.json", std::process::id()));
    std::fs::write(&p, r#"{"theme":"Light","crf":18}"#).unwrap();
    save_setting(&p, "visualizer_style", serde_json::Value::String("Spectrum Bars".into()));
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    assert_eq!(j["visualizer_style"], "Spectrum Bars");
    assert_eq!((j["theme"].as_str(), j["crf"].as_u64()), (Some("Light"), Some(18)), "the rest kept");
    let s: crate::settings::Settings = serde_json::from_value(j).unwrap();
    assert_eq!(s.visualizer_style, "Spectrum Bars");
    let look: Look = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    assert_eq!(look.visualizer_style, "Spectrum Bars", "the player reads it back");
    let _ = std::fs::remove_file(&p);
}

/// `SE_STRIP_FILE=<video> cargo test thumbnails_while_playing -- --ignored --nocapture` (a scratch
/// LOCALAPPDATA, so the strip is not in the cache yet): play 20 s with the strip filling in the
/// background, then 20 s with no strip, and compare dropped frames.
#[test]
#[ignore]
fn thumbnails_while_playing_cost_no_frames() {
    let f = std::path::PathBuf::from(std::env::var("SE_STRIP_FILE").expect("SE_STRIP_FILE"));
    let ctx = egui::Context::default();
    let mut drops = Vec::new();
    for with_strip in [true, false] {
        let mut app = PlayerApp::new(&ctx, Engine::start(&f, 5.0, false, true));
        until_picture(&ctx, &mut app);
        if !with_strip {
            app.strip = None;
        }
        app.engine.set_playing(true);
        run_for(&ctx, &mut app, 20_000, |_| {});
        let thumbs = app.strip.as_ref().map_or(0, |s| s.len());
        println!("strip {with_strip}: {} frames dropped in 20 s, {thumbs} thumbnails made meanwhile", app.engine.dropped());
        drops.push(app.engine.dropped());
    }
    assert!(drops[0] <= drops[1] + 3, "the strip cost frames: {drops:?}");
}
