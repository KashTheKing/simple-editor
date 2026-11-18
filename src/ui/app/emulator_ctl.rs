//! ---- ws:device-emulator ----
//! The Device Emulator pane (`ui::emulator`) in the app: its per-project choice, the picture it shows (the
//! preview's own texture, or its own upload when the Preview pane is hidden), what its taps do (play, seek,
//! the project's subtitles), the Screenshot button, and the `emulator.open` / `emulator.screenshot` tools.

use super::thumbs::base64;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::emulator::{self, Asked, EmuCtx, Prefs};
use crate::ui::social::device::{Player, Sheet};
use crate::ui::social::paint::GuideView;
use crate::ui::social::{self, raster, ViewMode};

/// The settings key of the open project ("" while it is unsaved).
fn key(app: &App) -> String {
    app.project_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
}

/// The open project's emulator choice.
pub(super) fn prefs(app: &App) -> Prefs {
    app.settings.emulator.get(&key(app)).cloned().unwrap_or_default()
}

fn save_prefs(app: &mut App, p: Prefs) {
    let k = key(app);
    app.settings.emulator.insert(k, p);
    app.settings.save();
}

/// Show the pane and bring it to the front.
pub(super) fn open(app: &mut App) {
    app.layout.reveal(Pane::Emulator);
    app.layout_dirty = true;
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Emulator {
        return false;
    }
    // the picture: the GPU renderer's texture, the preview's upload, or (Preview hidden) our own upload of the
    // frame nobody else takes - nothing is rendered for the emulator
    let preview_shown = app.pane_drawn(Pane::Preview);
    if !preview_shown {
        if let Some(f) = app.pending_frame.take() {
            let (w, h) = (f.width as usize, f.height as usize);
            if w > 0 && h > 0 && f.rgba.len() == w * h * 4 {
                let img = egui::ColorImage::from_rgba_premultiplied([w, h], &f.rgba);
                match &mut app.emulator_tex {
                    Some(t) if t.size() == [w, h] => t.set_partial([0, 0], img, egui::TextureOptions::LINEAR),
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => app.emulator_tex = Some(ui.ctx().load_texture("emulator", img, egui::TextureOptions::LINEAR)),
                }
            }
        }
        // the player renders for someone: give it this pane's size when the Preview is not there to
        let s = ui.available_size() * ui.pixels_per_point();
        let (cw, ch) = (s.x.max(16.0) as u32, s.y.max(16.0) as u32);
        if app.canvas.0 == 0 {
            app.player.set_canvas(cw, ch, app.settings.preview_max_width);
            app.canvas = (cw, ch);
        }
    }
    let video = app
        .gpu_tex
        .map(|t| t.0)
        .or_else(|| if preview_shown { app.preview.texture.as_ref().map(|t| t.id()) } else { None })
        .or_else(|| app.emulator_tex.as_ref().map(|t| t.id()));
    let mut prefs = prefs(app);
    let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
    let asked = {
        let App { emulator: st, settings, project, playhead, player, play_gate, .. } = app;
        emulator::show(
            ui,
            st,
            EmuCtx {
                settings,
                prefs: &mut prefs,
                title: &project.name,
                aspect,
                playhead: *playhead,
                duration: project.duration(),
                playing: player.is_playing() || play_gate.buffering(),
                buffering: player.is_buffering() || play_gate.buffering(),
                cc: project.show_subtitles,
                video,
            },
        )
    };
    for a in asked {
        match a {
            Asked::Play => app.pending_actions.push(Action::PlayPause),
            Asked::Seek(t) => app.seek(t),
            Asked::Captions => toggle_captions(app),
            Asked::Saved => save_prefs(app, prefs.clone()),
            Asked::Screenshot => screenshot_to_file(app, &prefs),
        }
    }
    true
}

/// The player's CC button: show / hide the project's subtitles, one undo step.
pub(super) fn toggle_captions(app: &mut App) {
    let before = app.project.to_json();
    app.project.show_subtitles = !app.project.show_subtitles;
    app.push_undo_labeled(before, if app.project.show_subtitles { "Show subtitles" } else { "Hide subtitles" });
    app.after_edit();
}

/// A picture of `p`'s device and platform at the playhead (the frame rendered now), `width` px wide.
fn picture(app: &mut App, p: &Prefs, sheet: Option<Sheet>, view: ViewMode, pl: Player, width: u32) -> Result<Frame, String> {
    let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
    let t = app.playhead;
    let frame = app.render_frame_now(t, width.min(app.project.width.max(16)));
    let title = app.project.name.clone();
    let mut g = GuideView::with(&app.settings, &p.platform, &p.device, &title, aspect, t, app.project.duration()).ok_or_else(|| format!("unknown platform '{}'", p.platform))?;
    g.playing = app.player.is_playing();
    g.view = view;
    Ok(raster::render_device_with(frame.as_deref(), &g, sheet, width, p.land, pl))
}

/// The Screenshot button: the large view as a PNG in Pictures\Simple Editor.
fn screenshot_to_file(app: &mut App, p: &Prefs) {
    let pl = app.emulator.social.player(app.project.show_subtitles, false, 0.0);
    let (sheet, view) = (app.emulator.social.open, app.emulator.view);
    let out = match picture(app, p, sheet, view, pl, 1400) {
        Ok(f) => f,
        Err(e) => return app.toast(e),
    };
    let dir = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_default().join("Pictures").join("Simple Editor");
    let file = dir.join(format!("{} - {} {} {}.png", app.project.name, p.platform, p.device, if p.land { "landscape" } else { "portrait" }));
    let r = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| std::fs::write(&file, crate::mcp::png_encode(&out)).map_err(|e| e.to_string()));
    match r {
        Ok(()) => app.toast_with_folder(format!("Saved {}", file.display()), file),
        Err(e) => app.toast(format!("Screenshot not saved: {e}")),
    }
}

/// `device`, `platform`, `orientation` (+ `gallery`) from tool arguments over `p`.
fn apply_args(p: &mut Prefs, args: &Value) -> Result<(), String> {
    let d = social::data();
    if let Some(v) = arg_str(args, "device") {
        if !d.devices.iter().any(|x| x.id == v) {
            return Err(format!("device: one of {}", d.devices.iter().map(|x| x.id.as_str()).collect::<Vec<_>>().join(", ")));
        }
        p.device = v.into();
    }
    if let Some(v) = arg_str(args, "platform") {
        if d.platform(v).is_none() {
            return Err(format!("platform: one of {}", d.platforms.iter().map(|x| x.id.as_str()).collect::<Vec<_>>().join(", ")));
        }
        p.platform = v.into();
    }
    match arg_str(args, "orientation") {
        None => {}
        Some("portrait") => p.land = false,
        Some("landscape") => p.land = true,
        Some(o) => return Err(format!("orientation: portrait or landscape (got '{o}')")),
    }
    if let Some(g) = arg_bool(args, "gallery") {
        p.gallery = g;
    }
    Ok(())
}

fn sheet_arg(args: &Value) -> Result<Option<Sheet>, String> {
    match arg_str(args, "sheet") {
        None | Some("") | Some("none") => Ok(None),
        Some(s) => Sheet::from_id(s).map(Some).ok_or_else(|| format!("sheet: none, comments, desc, share, more, profile or sound (got '{s}')")),
    }
}

fn view_arg(args: &Value) -> Result<Option<ViewMode>, String> {
    match arg_str(args, "view") {
        None => Ok(None),
        Some(v) => ViewMode::from_id(v).map(Some).ok_or_else(|| format!("view: default, theatre or fullscreen (got '{v}')")),
    }
}

fn state_json(app: &App) -> Value {
    let p = prefs(app);
    json!({"device": p.device, "platform": p.platform, "orientation": if p.land { "landscape" } else { "portrait" }, "gallery": p.gallery,
           "sheet": app.emulator.social.want.map(|s| s.id()), "view": app.emulator.view.id(), "captions": app.project.show_subtitles})
}

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "emulator.open",
        desc: "Show the Device Emulator pane on a device and platform: the project's video inside a real phone / tablet / browser / TV with the platform's UI. Any argument left out keeps the current choice (kept per project). Returns the choice and the ids to pick from.",
        args: &[
            "device:string:false:iphone|iphone_notch|android|galaxy|small|tablet|desktop|tv",
            "platform:string:false:a guides.list platform id (yt_long, yt_shorts, tiktok, ig_reels, ...)",
            "orientation:string:false:portrait|landscape",
            "gallery:boolean:false:true = every device and platform at once",
            "sheet:string:false:none|comments|desc|share|more|profile|sound",
            "view:string:false:default|theatre|fullscreen (YouTube on desktop)",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let mut p = prefs(app);
            apply_args(&mut p, args)?;
            let sheet = sheet_arg(args)?;
            if let Some(v) = view_arg(args)? {
                app.emulator.view = v;
            }
            if args.get("sheet").is_some() {
                app.emulator.social.want = sheet;
                if sheet.is_some() {
                    p.gallery = false;
                }
            }
            save_prefs(app, p);
            open(app);
            let d = social::data();
            Ok(ToolOutcome::Done(json!({"state": state_json(app),
                "devices": d.devices.iter().map(|v| json!({"id": v.id, "name": v.n, "kind": v.k, "os": v.os})).collect::<Vec<_>>(),
                "platforms": d.platforms.iter().map(|p| json!({"id": p.id, "name": p.n})).collect::<Vec<_>>()})))
        },
    },
    ToolDef {
        name: "emulator.screenshot",
        desc: "A picture of the Device Emulator: the frame at the playhead inside the device with the platform's UI. Defaults to the pane's own choice; device / platform / orientation / sheet / view override it for this picture only. menu = the player's settings menu open, buffering = the platform's buffering spinner. Captions follow the project's subtitles switch.",
        args: &[
            "device:string:false:default = the pane's device",
            "platform:string:false:default = the pane's platform",
            "orientation:string:false:portrait|landscape",
            "sheet:string:false:none|comments|desc|share|more|profile|sound",
            "view:string:false:default|theatre|fullscreen",
            "menu:boolean:false:the settings menu open",
            "buffering:boolean:false:show the buffering spinner",
            "width:integer:false:picture width in px (default 900)",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let mut p = prefs(app);
            apply_args(&mut p, args)?;
            let sheet = match args.get("sheet") {
                Some(_) => sheet_arg(args)?,
                None => app.emulator.social.open,
            };
            let view = view_arg(args)?.unwrap_or(app.emulator.view);
            let mut pl = app.emulator.social.player(app.project.show_subtitles, arg_bool(args, "buffering").unwrap_or(false), 0.6);
            if let Some(m) = arg_bool(args, "menu") {
                pl.menu = m;
            }
            let width = arg_u64(args, "width").map(|w| w as u32).unwrap_or(900).clamp(160, 2400);
            let out = picture(app, &p, sheet, view, pl, width)?;
            let meta = json!({"device": p.device, "platform": p.platform, "orientation": if p.land { "landscape" } else { "portrait" },
                              "sheet": sheet.map(|s| s.id()), "view": view.id(), "captions": pl.cc, "menu": pl.menu, "buffering": pl.buffering,
                              "t": app.playhead, "width": out.width, "height": out.height});
            Ok(ToolOutcome::Image { png_b64: base64(&crate::mcp::png_encode(&out)), meta })
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every argument of both tools is checked: unknown ids, orientations, sheets and views are refused.
    #[test]
    fn emulator_args_are_checked() {
        let mut p = Prefs::default();
        assert!(apply_args(&mut p, &json!({"device": "galaxy", "platform": "yt_long", "orientation": "landscape"})).is_ok());
        assert_eq!((p.device.as_str(), p.platform.as_str(), p.land), ("galaxy", "yt_long", true));
        assert!(apply_args(&mut p, &json!({"device": "nokia"})).is_err());
        assert!(apply_args(&mut p, &json!({"platform": "myspace"})).is_err());
        assert!(apply_args(&mut p, &json!({"orientation": "sideways"})).is_err());
        assert!(sheet_arg(&json!({"sheet": "comments"})).unwrap() == Some(Sheet::Comments));
        assert!(sheet_arg(&json!({"sheet": "nope"})).is_err());
        assert!(view_arg(&json!({"view": "theatre"})).unwrap() == Some(ViewMode::Theatre));
    }
}
