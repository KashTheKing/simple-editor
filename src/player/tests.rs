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
    assert!(app.engine.dropped() <= 3, "{} frames dropped reversing: {shown:?}", app.engine.dropped());
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
fn skip_back_10_and_forward_10_move_the_clock_and_the_picture_follows() {
    let Some((ctx, mut app)) = player(2.0) else { return };
    until_picture(&ctx, &mut app);
    let dur = app.engine.duration();
    assert_eq!((SKIP_BACK, SKIP_FORWARD), (10.0, 10.0), "both skips are 10 s");
    app.skip(-SKIP_BACK);
    assert_eq!(app.engine.time(), 0.0, "back 10 s from 2 s: the start");
    app.skip(SKIP_FORWARD);
    assert!((app.engine.time() - (dur - 0.001)).abs() < 1e-9, "forward 10 s in a 4 s file: its last frame");
    // the decoder seeks there and the next picture is from the end of the file
    let end = Instant::now() + Duration::from_secs(5);
    while app.engine.next_due().is_none_or(|p| p < dur - 0.2) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.engine.next_due().is_some_and(|p| p >= dur - 0.2), "next frame at {:?}", app.engine.next_due());
    // the keys do the same: Left = back 10, Right = forward 10
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

// ---------------------------------------------------------------- the player batch

/// A file made with ffmpeg in a fresh temp folder (None without ffmpeg).
fn made(name: &str, args: &[&str]) -> Option<std::path::PathBuf> {
    let ff = crate::media::ffpipe::ffmpeg_exe()?;
    let dir = std::env::temp_dir().join(format!("se-player-batch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join(name);
    let ok = crate::media::ffpipe::command(&ff).args(["-y", "-v", "error"]).args(args).arg(&out).status().ok()?.success();
    ok.then_some(out)
}

fn until(ctx: &egui::Context, app: &mut PlayerApp, secs: u64, mut done: impl FnMut(&PlayerApp) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while !done(app) {
        if Instant::now() > end {
            return false;
        }
        run(ctx, app, vec![]);
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

#[test]
fn a_picked_resolution_caps_the_decode_size() {
    assert_eq!(capped((1920, 1080), Some(720)), (1280, 720));
    assert_eq!(capped((3840, 2160), Some(1080)), (1920, 1080));
    assert_eq!(capped((1920, 1080), Some(2160)), (1920, 1080), "never above the source");
    assert_eq!(capped((1920, 1080), None), (1920, 1080), "Auto");
    let Some((ctx, mut app)) = player(1.0) else { return };
    until_picture(&ctx, &mut app);
    assert!(app.res_options().iter().all(|&h| h < 180), "only below the 180-line source: {:?}", app.res_options());
    app.set_resolution(Some(120));
    assert_eq!(app.prefs.res.get(&file_key(app.path())), Some(&120), "remembered per file");
    let path = app.path().to_path_buf();
    app.open_at(&ctx, &path, 0.5, false);
    assert_eq!(app.res, Some(120), "and picked again when the file opens");
    app.set_resolution(None);
    assert!(app.prefs.res.is_empty());
}

/// Right-clicking the picture opens the menu, and the controls stay up while it is open.
#[test]
fn right_click_opens_the_menu_and_holds_the_controls() {
    let Some((ctx, mut app)) = player(1.0) else { return };
    until_picture(&ctx, &mut app);
    let at = egui::pos2(300.0, 200.0);
    let click = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    run(&ctx, &mut app, vec![Event::PointerMoved(at), click(true)]);
    run(&ctx, &mut app, vec![click(false)]);
    run(&ctx, &mut app, vec![]);
    assert!(egui::Popup::is_any_open(&ctx), "the right-click menu is open");
    app.engine.set_playing(true);
    std::thread::sleep(Duration::from_millis(((autohide::IDLE + 0.3) * 1000.0) as u64));
    run(&ctx, &mut app, vec![]);
    assert!(app.hide.shown(), "an open menu keeps the controls (and the cursor) up");
}

/// A song shows its cover (a placeholder while it loads), the visualizer only without one or when
/// asked for.
#[test]
fn songs_show_their_cover_and_the_visualizer_only_without_one() {
    let Some(pic) = made("cover.png", &["-f", "lavfi", "-i", "color=red:s=64x64:d=1", "-frames:v", "1"]) else { return };
    let src = ["-f", "lavfi", "-i", "sine=f=440:d=3"];
    let pic_s = pic.to_string_lossy().into_owned();
    let mut with: Vec<&str> = src.to_vec();
    with.extend(["-i", &pic_s, "-map", "0:a", "-map", "1:v", "-c:v", "png", "-disposition:v", "attached_pic"]);
    let Some(song) = made("song.mp3", &with) else { return };
    let Some(bare) = made("bare.mp3", &src) else { return };
    let ctx = egui::Context::default();
    let mut app = PlayerApp::new(&ctx, Engine::start(&song, 0.0, false, false));
    assert!(until(&ctx, &mut app, 10, |a| a.cover_rx.is_some() || a.cover.is_some()), "the cover starts loading");
    assert!(!app.song_shows_visualizer(), "loading: the placeholder, not a visualizer");
    assert!(until(&ctx, &mut app, 10, |a| a.cover.is_some()), "the cover loads");
    assert!(!app.song_shows_visualizer() && app.cover_bg.is_some(), "the cover, with its blurred backdrop");
    app.set_song_visualizer(true);
    assert!(app.song_shows_visualizer(), "Visualizer picked");
    app.set_song_visualizer(false);
    app.open(&ctx, &bare);
    assert!(until(&ctx, &mut app, 10, |a| a.info.is_some() && a.cover_rx.is_none()), "read");
    assert!(app.cover.is_none() && app.song_shows_visualizer(), "no cover: the visualizer");
}

/// Properties / Metadata writes the edit into the file (let go of first, temp file, atomic swap) and
/// opens it again where it was; Explorer's tags go in too.
#[test]
fn the_metadata_panel_saves_into_the_file_and_reopens_it() {
    let Some(f) = made("clip.m4a", &["-f", "lavfi", "-i", "sine=f=440:d=4", "-c:a", "aac", "-metadata", "title=Old"]) else { return };
    let ctx = egui::Context::default();
    let mut app = PlayerApp::new(&ctx, Engine::start(&f, 1.5, false, false));
    assert!(until(&ctx, &mut app, 10, |a| a.info.is_some()));
    app.open_metadata(&ctx);
    assert!(until(&ctx, &mut app, 10, |a| a.meta.as_ref().is_some_and(|m| m.loaded.is_some())), "tags read");
    let m = app.meta.as_mut().unwrap();
    assert_eq!(m.draft["title"], "Old");
    m.draft.insert("title".into(), "New".into());
    m.draft.insert("copyright".into(), "Me".into());
    m.keywords = "one; two".into();
    let (edit, kw) = m.edit().unwrap();
    app.save_meta(&ctx, edit, kw);
    assert!(until(&ctx, &mut app, 20, |a| a.meta_save.is_none()), "saved");
    assert_eq!(app.meta.as_ref().unwrap().status, "Saved");
    let t = crate::engine::tags::read(&f).unwrap();
    assert_eq!((t.get("title"), t.get("copyright")), ("New", "Me"));
    assert_eq!(crate::engine::tags::read_keywords(&f), ["one", "two"]);
    assert_eq!(app.path(), f.as_path(), "the same file open again");
    assert!((app.engine.time() - 1.5).abs() < 0.2, "where it was: {}", app.engine.time());
    assert!(!f.with_extension("m4a.bak").exists(), "no backup left behind");
}

/// Upscale… without the AI enhancer: the job says so (the window offers the install).
#[test]
fn an_upscale_without_the_enhancer_says_so() {
    if crate::engine::enhance::available() {
        return; // a real upscale takes minutes: covered by the editor's enhance tests
    }
    let Some((ctx, mut app)) = player(0.0) else { return };
    until_picture(&ctx, &mut app);
    app.start_upscale(Default::default());
    assert!(until(&ctx, &mut app, 10, |a| a.upscale_job.is_none()));
    let msg = app.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(msg.contains("Upscale failed") && msg.contains("not installed"), "{msg}");
}

/// Screenshot frame: a PNG of the moment beside the file, never over one.
#[test]
fn screenshot_frame_saves_a_png_beside_the_file() {
    let Some(f) = made("shot.mp4", &["-f", "lavfi", "-i", "testsrc2=s=160x90:r=30:d=3", "-pix_fmt", "yuv420p"]) else { return };
    let ctx = egui::Context::default();
    let mut app = PlayerApp::new(&ctx, Engine::start(&f, 1.0, false, false));
    until_picture(&ctx, &mut app);
    for _ in 0..2 {
        app.screenshot();
        assert!(until(&ctx, &mut app, 10, |a| a.shot_rx.is_none()));
    }
    let pngs: Vec<_> = std::fs::read_dir(f.parent().unwrap()).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "png")).collect();
    assert_eq!(pngs.len(), 2, "two shots, two files");
    assert!(app.toast.as_ref().is_some_and(|t| t.0.starts_with("Saved shot 0-00-01")), "{:?}", app.toast);
}
