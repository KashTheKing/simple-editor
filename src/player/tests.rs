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
