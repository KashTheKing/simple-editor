//! Simple Editor — a tiny, fast video trimmer/editor for Windows.
//! `simple-editor [file]`   open a video/project
//! `simple-editor --selftest [dir]`   headless engine check
//! `simple-editor [file] --screenshot out.ppm`   render the UI once and save it (for visual checks)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod contextmenu;
mod engine;
mod hotkeys;
mod mcp;
mod media;
mod model;
mod playback;
mod selftest;
mod settings;
mod theme;
mod ui;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--selftest") {
        std::process::exit(selftest::run(&args[1..]));
    }
    let mut screenshot: Option<PathBuf> = None;
    let mut open: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" => {
                screenshot = args.get(i + 1).map(PathBuf::from);
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

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Simple Editor")
            .with_app_id("SimpleEditor")
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([900.0, 560.0]),
        persist_window: true,
        ..Default::default()
    };
    if let Err(e) = eframe::run_native(
        "Simple Editor",
        options,
        Box::new(move |cc| Ok(Box::new(ui::app::App::new(cc, open, screenshot)))),
    ) {
        eprintln!("failed to start: {e}");
        std::process::exit(1);
    }
}
