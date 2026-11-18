//! Simple Editor - a tiny, fast video trimmer/editor for Windows.
//! `simple-editor [file]`   play a video / sound file (player mode, `player`), or open a project
//! `simple-editor --selftest [dir]`   headless engine check
//! `simple-editor [file] --screenshot out.ppm`   render the UI once and save it (for visual checks)
//! `--size 1600x900`   start with this window inner size in points (reproducible screenshots)
//! `simple-editor --dump-hotkeys out.md`   write the shortcut list (F1) as Markdown for the docs site
//! `SE_BACKGROUND=1`   (env) open at the saved window rect without taking focus - for automation
//! `--mcp-full-access` / `SE_MCP_FULL_ACCESS=1`   every MCP call runs, nothing asks (default on for debug and worktree builds; =0 turns it off)
//! `--quiet` / `SE_QUIET=1`   no What's New, welcome wizard or recover-autosave prompt (implied by SE_BACKGROUND)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod contextmenu;
mod engine;
mod hotkeys;
mod keymaps;
mod mcp;
mod media;
mod model;
mod perf;
mod playback;
mod player;
mod scripting;
mod selftest;
mod settings;
mod theme;
mod theme_import;
mod ui;
mod winpos;

use std::path::PathBuf;

fn main() {
    perf::startup_mark("main");
    ui::app::recovery::install_panic_hook(); // ws:forgiveness: crash.log + a snapshot of the latest autosave
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--quiet") {
        std::env::set_var("SE_QUIET", "1"); // before any thread starts; read by winpos::quiet
    }
    if args.iter().any(|a| a == "--mcp-full-access") {
        std::env::set_var("SE_MCP_FULL_ACCESS", "1"); // read by mcp::perms::full_access
    }
    if args.first().map(|s| s.as_str()) == Some("--selftest") {
        std::process::exit(selftest::run(&args[1..]));
    }
    if let (Some("--dump-hotkeys"), Some(out)) = (args.first().map(|s| s.as_str()), args.get(1)) {
        let md = ui::cheatsheet::markdown(&hotkeys::Hotkeys::defaults());
        if let Err(e) = std::fs::write(out, md) {
            eprintln!("{out}: {e}");
            std::process::exit(1);
        }
        return;
    }
    let mut screenshot: Option<PathBuf> = None;
    let mut open: Option<PathBuf> = None;
    let mut size: Option<[f32; 2]> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" => {
                screenshot = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--size" => {
                size = args.get(i + 1).and_then(|s| parse_size(s));
                i += 1;
            }
            // absolute: the path is stored in the project/recents and must survive a different cwd
            a if !a.starts_with("--") && open.is_none() => {
                open = Some(std::path::absolute(a).unwrap_or_else(|_| PathBuf::from(a)))
            }
            _ => {}
        }
        i += 1;
    }

    // a media file from Explorer / a file argument: the player, with no editor behind it until asked
    if let Some(path) = open.as_ref().filter(|p| screenshot.is_none() && p.is_file() && player::folder::is_playable(p)) {
        run_player(path.clone(), size);
        return;
    }

    // read once to seed the viewport; App::new does its own (cheap) re-read of the same file - not
    // worth threading a loaded Settings through eframe's boxed FnOnce for this one field.
    let seed = settings::Settings::load();
    let window_rect = seed.window_rect;
    let mut viewport = winpos::apply_rect(
        eframe::egui::ViewportBuilder::default()
            .with_title("Simple Editor")
            .with_app_id("SimpleEditor")
            .with_inner_size([1400.0, 860.0])
            // hidden until the first frame is painted (App::update shows it) - otherwise the OS
            // flashes a blank white window at the restored position before we move/paint it
            .with_visible(false)
            .with_min_inner_size([900.0, 560.0]),
        window_rect,
    );
    if let Some(s) = size {
        viewport = viewport.with_inner_size(s);
    } else if seed.start_maximized && !winpos::background() && screenshot.is_none() {
        // (automation - SE_BACKGROUND, --size, --screenshot - keeps the exact rect it asked for)
        viewport = winpos::maximized_on_cursor_monitor(viewport);
    }
    if winpos::background() {
        viewport = viewport.with_active(false); // shown with SW_SHOWNOACTIVATE
    }
    let options = eframe::NativeOptions {
        viewport,
        // inert either way now that eframe's "persistence" feature is gone - false for clarity, so
        // this field doesn't read as a live knob it no longer is.
        persist_window: false,
        ..Default::default()
    };
    // the UI thread shows every frame the player publishes: above normal, so a busy PC (parallel builds)
    // does not hold the picture back while the render thread keeps up (`bg::raise_thread`)
    media::bg::inherit(i32::MAX);
    perf::startup_mark("run_native");
    if let Err(e) = eframe::run_native(
        "Simple Editor",
        options,
        Box::new(move |cc| {
            perf::startup_mark("window + GL ready");
            Ok(Box::new(ui::app::App::new(cc, open, screenshot)))
        }),
    ) {
        eprintln!("failed to start: {e}");
        std::process::exit(1);
    }
}

/// Player mode (`player`): the decoders start here, before the window, so opening the file and
/// decoding its first frame overlap window and GL creation. No settings, no editor.
fn run_player(path: PathBuf, size: Option<[f32; 2]>) {
    perf::startup_mark("player: start");
    let engine = player::engine::Engine::start(&path, 0.0, false, true); // plays once the window shows
    let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id("SimpleEditor")
        .with_inner_size([1280.0, 720.0])
        .with_min_inner_size([320.0, 180.0])
        // the player draws its own title strip (`player::chrome`), over the picture
        .with_decorations(false)
        .with_visible(false);
    if let Some(s) = size {
        viewport = viewport.with_inner_size(s);
    } else if winpos::background() {
        // automation: where its saved rect puts it (a second screen), never stealing focus
        viewport = winpos::apply_rect(viewport, settings::Settings::load().window_rect);
    } else {
        viewport = winpos::maximized_on_cursor_monitor(viewport);
    }
    if winpos::background() {
        viewport = viewport.with_active(false);
    }
    let options = eframe::NativeOptions { viewport, persist_window: false, ..Default::default() };
    media::bg::inherit(i32::MAX);
    if let Err(e) = eframe::run_native(
        "Simple Editor",
        options,
        Box::new(move |cc| Ok(Box::new(player::Root::Player(Box::new(player::PlayerApp::new(&cc.egui_ctx, engine)))))),
    ) {
        eprintln!("failed to start: {e}");
        std::process::exit(1);
    }
}

/// `--size WxH` ("1600x900") -> inner size in points.
fn parse_size(s: &str) -> Option<[f32; 2]> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
}

#[cfg(test)]
mod tests {
    #[test]
    fn size_flag_parses() {
        assert_eq!(super::parse_size("1600x900"), Some([1600.0, 900.0]));
        assert_eq!(super::parse_size("800X600"), Some([800.0, 600.0]));
        assert_eq!(super::parse_size("big"), None);
    }
}
