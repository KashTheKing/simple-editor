//! ---- ws:ui-kit ----
//! UI automation for verifying UI changes and scripting the docs site's screenshots:
//! - `ui.screenshot {path, pane?}`: the window (or one docked pane, tab bar included) to a PNG;
//! - `ui.input {events}`: synthetic clicks / keys / scrolls, played one step per frame through
//!   `eframe::App::raw_input_hook` (a click = move, press, release on three frames).
//!
//! Both reply through the `ToolOutcome::Job` path: the image arrives a frame after it is asked for, and
//! the input only lands once its last step has been drawn. `frame` (called once per `App::update`) also
//! publishes the `ui::menu` snapshot and drains the Actions its rows queued.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::menu;
use std::collections::VecDeque;

/// One frame of synthetic input: its events and the modifier state to report with them.
type Step = (Vec<egui::Event>, egui::Modifiers);

#[derive(Default)]
pub(super) struct UiKit {
    /// `ui.input` steps not played yet; a call's `Progress` rides its final (empty) step.
    steps: VecDeque<(Step, Option<Arc<Progress>>)>,
    /// `ui.screenshot` calls waiting for their image: (tag, path, crop in points, progress).
    shots: Vec<(u64, PathBuf, Option<egui::Rect>, Arc<Progress>)>,
    next_shot: u64,
    /// The root context, so `ui.screenshot` can ask for this very frame (its `run` gets no ctx).
    ctx: Option<egui::Context>,
}

impl UiKit {
    /// Is `p` one of our calls' progress? The Jobs pane skips those: automation isn't a user's job, and
    /// the running-jobs indicator would otherwise show up in every `ui.screenshot`.
    pub(super) fn owns(&self, p: &Arc<Progress>) -> bool {
        self.shots.iter().any(|s| Arc::ptr_eq(&s.3, p))
            || self.steps.iter().any(|(_, d)| d.as_ref().is_some_and(|d| Arc::ptr_eq(d, p)))
    }
}

/// Tags our screenshot requests - `screenshot_tick` (the `--screenshot` CLI path, which exits the app)
/// only takes untagged ones.
pub(super) struct UiShot(u64);

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "ui.screenshot",
        desc: "Save the editor window, or one docked pane with its tab bar, as a PNG. Replies once the file \
               is written (a frame later); status = '<w>x<h> px at <ppp> px/pt' (ui.input takes points).",
        args: &[
            "path:string:true:output .png path",
            "pane:string:false:a pane title (Timeline, Library, Preview, ...) to crop to; it must be on screen",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let crop = match arg_str(args, "pane") {
                None => None,
                Some(name) => {
                    let pane = Pane::ALL
                        .iter()
                        .find(|p| p.title().eq_ignore_ascii_case(name))
                        .ok_or_else(|| format!("unknown pane '{name}'"))?;
                    let rect = app.layout.rects.iter().find(|(p, _)| p == pane).map(|(_, r)| *r);
                    Some(rect.ok_or_else(|| format!("pane '{name}' is not on screen"))?)
                }
            };
            let ctx = app.uikit.ctx.clone().ok_or("no frame drawn yet")?;
            let prog = Progress::new();
            app.uikit.next_shot += 1;
            let tag = app.uikit.next_shot;
            // asked for now, while this frame's job snapshot doesn't hold this call yet - so the menu
            // bar's jobs indicator never shows up in the shot
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Screenshot(egui::UserData::new(UiShot(tag))),
            );
            app.uikit.shots.push((tag, path.clone(), crop, prog.clone()));
            Ok(ToolOutcome::Job(prog, path))
        },
    },
    ToolDef {
        name: "ui.input",
        desc: "Play synthetic input into the live UI, one step per frame; replies after the last step has been \
               drawn. events: [{type: move|click|rclick|dclick|key|text|scroll, x, y (points, see ui.screenshot), \
               key (egui name: A, Space, Enter, Escape, F1, ArrowLeft...), mods ('ctrl+shift+alt'), text, \
               delta (scroll [dx, dy] points or just dy; positive dy moves content down = scrolls up)}].",
        args: &["events:array:true:input events, played in order"],
        kind: ToolKind::Job,
        run: |app, args| {
            let events = args.get("events").and_then(Value::as_array).ok_or("events: array required")?;
            let steps = expand(events)?;
            let prog = Progress::new();
            app.uikit.steps.extend(steps.into_iter().map(|s| (s, None)));
            // one settle frame after the last real step: its effects (a context menu's sizing pass) are
            // drawn before the reply lets the caller screenshot them
            app.uikit.steps.push_back(((Vec::new(), egui::Modifiers::NONE), Some(prog.clone())));
            Ok(ToolOutcome::Job(prog, PathBuf::from("ui.input")))
        },
    },
];

/// `ui.input`'s events as per-frame steps.
fn expand(events: &[Value]) -> Result<Vec<Step>, String> {
    use egui::{Event, PointerButton};
    let mut out = Vec::new();
    for e in events {
        let kind = e.get("type").and_then(Value::as_str).ok_or("event without a type")?;
        let mods = parse_mods(e.get("mods").and_then(Value::as_str).unwrap_or(""))?;
        let pos = || -> Result<egui::Pos2, String> {
            match (arg_f64(e, "x"), arg_f64(e, "y")) {
                (Some(x), Some(y)) => Ok(egui::pos2(x as f32, y as f32)),
                _ => Err(format!("{kind}: x and y required")),
            }
        };
        let button = |button, pressed, pos| Event::PointerButton { pos, button, pressed, modifiers: mods };
        let mut steps: Vec<Vec<Event>> = Vec::new();
        match kind {
            "move" => steps.push(vec![Event::PointerMoved(pos()?)]),
            "click" | "rclick" | "dclick" => {
                let (p, b) = (pos()?, if kind == "rclick" { PointerButton::Secondary } else { PointerButton::Primary });
                steps.push(vec![Event::PointerMoved(p)]);
                for _ in 0..if kind == "dclick" { 2 } else { 1 } {
                    steps.push(vec![button(b, true, p)]);
                    steps.push(vec![button(b, false, p)]);
                }
            }
            "key" => {
                let name = e.get("key").and_then(Value::as_str).ok_or("key: key required")?;
                let key = egui::Key::from_name(name).ok_or_else(|| format!("unknown key '{name}'"))?;
                for pressed in [true, false] {
                    steps.push(vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods }]);
                }
            }
            "text" => steps
                .push(vec![Event::Text(e.get("text").and_then(Value::as_str).ok_or("text: text required")?.into())]),
            "scroll" => {
                let delta = match e.get("delta") {
                    Some(Value::Array(d)) if d.len() == 2 => egui::vec2(
                        d[0].as_f64().ok_or("delta: numbers")? as f32,
                        d[1].as_f64().ok_or("delta: numbers")? as f32,
                    ),
                    Some(d) => egui::vec2(0.0, d.as_f64().ok_or("delta: [dx, dy] or dy")? as f32),
                    None => return Err("scroll: delta required".into()),
                };
                let p = pos()?;
                steps.push(vec![Event::PointerMoved(p)]);
                steps.push(vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, modifiers: mods }]);
            }
            _ => return Err(format!("unknown event type '{kind}'")),
        }
        out.extend(steps.into_iter().map(|s| (s, mods)));
    }
    Ok(out)
}

/// "ctrl+shift" -> Modifiers. Ctrl also sets `command`, as egui-winit reports it on Windows.
fn parse_mods(s: &str) -> Result<egui::Modifiers, String> {
    let mut m = egui::Modifiers::NONE;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => m = m | egui::Modifiers::COMMAND | egui::Modifiers::CTRL,
            "shift" => m = m | egui::Modifiers::SHIFT,
            "alt" => m = m | egui::Modifiers::ALT,
            other => return Err(format!("unknown modifier '{other}'")),
        }
    }
    Ok(m)
}

/// `eframe::App::raw_input_hook`: play one queued `ui.input` step into this frame's input.
pub(super) fn input_hook(app: &mut App, ctx: &egui::Context, raw: &mut egui::RawInput) {
    app.uikit.ctx.get_or_insert_with(|| ctx.clone());
    let Some(((events, mods), done)) = app.uikit.steps.pop_front() else { return };
    raw.modifiers = mods;
    raw.events.extend(events);
    if let Some(p) = done {
        p.finish(None); // poll_mcp replies later this same frame
    }
    ctx.request_repaint(); // the next step, or the reply
}

/// Once per `App::update`, before anything draws: the `ui::menu` snapshot and last frame's menu clicks,
/// then any `ui.screenshot` image that came back.
pub(super) fn frame(app: &mut App, ctx: &egui::Context) {
    app.uikit.ctx.get_or_insert_with(|| ctx.clone());
    menu::publish(menu::MenuSnapshot::new(&app.hotkeys, &app.settings.icon_overrides, |a| app.enabled(a)));
    app.pending_actions.extend(menu::take_queued());
    let picks = menu::take_icon_picks();
    if !picks.is_empty() {
        for (key, pick) in picks {
            match pick {
                Some(name) => drop(app.settings.icon_overrides.insert(key, name)),
                None => drop(app.settings.icon_overrides.remove(&key)),
            }
        }
        app.settings.save();
    }
    if app.uikit.shots.is_empty() {
        return;
    }
    let images: Vec<(u64, Arc<egui::ColorImage>)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. } => {
                    user_data.data.as_ref()?.downcast_ref::<UiShot>().map(|t| (t.0, image.clone()))
                }
                _ => None,
            })
            .collect()
    });
    let ppp = ctx.pixels_per_point();
    for (tag, image) in images {
        let Some(i) = app.uikit.shots.iter().position(|s| s.0 == tag) else { continue };
        let (_, path, crop, prog) = app.uikit.shots.remove(i);
        let f = crop_frame(&image, crop.map(|r| r * ppp));
        prog.set(1.0, format!("{}x{} px at {ppp} px/pt", f.width, f.height));
        prog.finish(std::fs::write(&path, mcp::png_encode(&f)).err().map(|e| format!("{}: {e}", path.display())));
    }
    ctx.request_repaint(); // the image comes back on a later frame
}

/// The screenshot (opaque RGBA pixels), cropped to `px` (a pixel rect, clamped to the image) if given.
fn crop_frame(img: &egui::ColorImage, px: Option<egui::Rect>) -> Frame {
    let [w, h] = img.size;
    let (x0, y0, x1, y1) = match px {
        Some(r) => (
            (r.left().round().max(0.0) as usize).min(w),
            (r.top().round().max(0.0) as usize).min(h),
            (r.right().round().max(0.0) as usize).min(w),
            (r.bottom().round().max(0.0) as usize).min(h),
        ),
        None => (0, 0, w, h),
    };
    let rgba = (y0..y1).flat_map(|y| img.pixels[y * w + x0..y * w + x1].iter().flat_map(|c| c.to_array())).collect();
    Frame { width: x1.saturating_sub(x0) as u32, height: y1.saturating_sub(y0) as u32, pts: 0.0, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton};

    #[test]
    fn events_expand_to_per_frame_steps() {
        let ev = json!([
            {"type": "click", "x": 10, "y": 20},
            {"type": "rclick", "x": 1, "y": 2, "mods": "ctrl+shift"},
            {"type": "dclick", "x": 3, "y": 4},
            {"type": "key", "key": "Space"},
            {"type": "scroll", "x": 5, "y": 6, "delta": -40},
            {"type": "text", "text": "hi"},
        ]);
        let steps = expand(ev.as_array().unwrap()).unwrap();
        // click 3 + rclick 3 + dclick 5 + key 2 + scroll 2 + text 1
        assert_eq!(steps.len(), 16);
        assert!(matches!(steps[0].0[..], [Event::PointerMoved(p)] if p == egui::pos2(10.0, 20.0)));
        assert!(matches!(steps[1].0[..], [Event::PointerButton { button: PointerButton::Primary, pressed: true, .. }]));
        assert!(matches!(steps[2].0[..], [Event::PointerButton { pressed: false, .. }]));
        // modifiers ride every frame of their event - RawInput.modifiers, not only the event's own
        let ctrl_shift = egui::Modifiers::CTRL | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        assert!(steps[3..6].iter().all(|s| s.1 == ctrl_shift));
        assert!(
            matches!(steps[4].0[..], [Event::PointerButton { button: PointerButton::Secondary, modifiers, .. }] if modifiers == ctrl_shift)
        );
        assert_eq!(steps[6].1, egui::Modifiers::NONE);
        let presses =
            steps[6..11].iter().filter(|s| matches!(s.0[..], [Event::PointerButton { pressed: true, .. }])).count();
        assert_eq!(presses, 2, "a double-click is two press/release pairs");
        assert!(matches!(steps[11].0[..], [Event::Key { key: egui::Key::Space, pressed: true, .. }]));
        assert!(matches!(steps[14].0[..], [Event::MouseWheel { delta, .. }] if delta == egui::vec2(0.0, -40.0)));
        assert!(matches!(&steps[15].0[..], [Event::Text(t)] if t == "hi"));
        for bad in [json!([{"type": "click"}]), json!([{"type": "key", "key": "Nope"}]), json!([{"type": "wiggle"}])] {
            assert!(expand(bad.as_array().unwrap()).is_err(), "{bad}");
        }
        assert!(parse_mods("ctrl+hyper").is_err());
    }

    #[test]
    fn crop_takes_the_pane_rect_in_pixels() {
        let mut img = egui::ColorImage::filled([4, 3], egui::Color32::BLACK);
        img[(2, 1)] = egui::Color32::WHITE;
        let whole = crop_frame(&img, None);
        assert_eq!((whole.width, whole.height, whole.rgba.len()), (4, 3, 48));
        let f = crop_frame(&img, Some(egui::Rect::from_min_max(egui::pos2(2.0, 1.0), egui::pos2(9.0, 2.0))));
        assert_eq!((f.width, f.height), (2, 1), "clamped to the image");
        assert_eq!(&f.rgba[..4], &[255, 255, 255, 255]);
    }
}
