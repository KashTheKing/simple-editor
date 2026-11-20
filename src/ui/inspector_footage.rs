//! ---- ws:mixed-footage ----
//! Transform rows for camera footage whose rate or size differs from the project: a speed Preset
//! (Normal / Every frame / Custom), the Frame rate conversion for odd rates (Frame sampling, Frame blending,
//! Optical flow) and Zoom with a 1:1 mark ("Max sharp": past it the picture is upscaled), plus the quiet
//! "Upscaled" hint with an AI Upscale button. Verbs that need the App (an optical-flow bake, AI Upscale, the
//! timeline menu's every-frame slow motion) go through `request` and run in `footage_ctl`.

use crate::media::footage;
use crate::model::{snap_fps, Clip, ClipKind, Effect, EffectKind, Id, Project};
use crate::ui::Gesture;
use eframe::egui;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FootageReq {
    /// Set this clip (and what is linked to it) to this speed, one undo step.
    Speed(Id, f64),
    /// Optical-flow bake of this clip at its speed.
    Flow(Id),
    /// AI Upscale this clip by 2 or 4.
    Upscale(Id, u32),
}

static REQUESTS: Mutex<Vec<FootageReq>> = Mutex::new(Vec::new());

pub fn request(r: FootageReq) {
    REQUESTS.lock().unwrap_or_else(|e| e.into_inner()).push(r);
}

pub fn take_requests() -> Vec<FootageReq> {
    std::mem::take(&mut *REQUESTS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The video clip's source rate (snapped), if known.
fn src_fps(project: &Project, clip: &Clip) -> Option<f64> {
    (clip.kind == ClipKind::Video).then(|| project.asset(clip.asset)).flatten().map(|a| a.fps).filter(|f| *f > 1.0).map(snap_fps)
}

/// The speed that shows every source frame once (50p in a 25p project: 0.5), for a clip whose rate is a
/// whole multiple of the project's.
pub fn every_frame_for(project: &Project, clip: &Clip) -> Option<f64> {
    footage::every_frame_speed(src_fps(project, clip)?, project.fps)
}

/// The timeline badge for such a clip: its source rate ("50"), while it plays at normal speed.
pub fn badge(project: &Project, clip: &Clip) -> Option<String> {
    every_frame_for(project, clip)?;
    let src = src_fps(project, clip)?;
    (!clip.is_retimed()).then(|| crate::ui::inspector::fps_label(src))
}

/// The Zoom (clip scale) at which one source pixel is one project pixel. Scale 1 is Fit (the whole frame,
/// letterboxed), so 4K in a 1080p project is 2.0: up to 200 % nothing is upscaled.
pub fn native_zoom(project: &Project, clip: &Clip) -> Option<f64> {
    let (nw, nh) = project.clip_native_size(clip)?;
    Some((nw as f64 / project.width.max(1) as f64).max(nh as f64 / project.height.max(1) as f64))
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Sample,
    Blend,
    Flow,
}

fn blends(c: &Clip) -> bool {
    c.effects.iter().any(|e| e.kind == EffectKind::FrameBlend)
}

/// Frame blending on or off for `ids`: the existing Frame Blend effect (it mixes in the next decoded frame).
pub fn set_blend(project: &mut Project, ids: &[Id], on: bool) {
    for &id in ids {
        let Some(c) = project.clip_mut(id) else { continue };
        if on && !blends(c) {
            c.effects.push(Effect::new(EffectKind::FrameBlend));
        } else if !on {
            c.effects.retain(|e| e.kind != EffectKind::FrameBlend);
        }
    }
}

/// Set `ids` (and what is linked to them) to `speed`. The clip grows to play its whole source window when
/// there is room after it; otherwise it keeps its length and plays the start of the window (drag its end
/// for the rest). True when it grew.
pub fn set_speed_fit(project: &mut Project, ids: &[Id], speed: f64, reverse: bool) -> bool {
    if project.set_speed(ids, speed, reverse) {
        return true;
    }
    for id in project.expand_links(ids) {
        if let Some(c) = project.clip_mut(id) {
            c.speed = speed.clamp(0.01, 100.0);
            if !c.speed_curve.is_animated() {
                c.speed_curve.value = c.speed;
            }
            c.reverse = reverse;
        }
    }
    false
}

/// The rows, under Speed in Transform. True when the project changed.
pub(super) fn rows(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    playhead: f64,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(c) = ids.first().and_then(|&i| project.clip(i)).cloned() else { return false };
    if !matches!(c.kind, ClipKind::Video | ClipKind::Image) {
        return false;
    }
    let pf = project.fps;
    let mut speed: Option<f64> = None;
    let mut mode: Option<Mode> = None;
    if let Some(src) = src_fps(project, &c) {
        let every = footage::every_frame_speed(src, pf);
        let normal = (c.speed - 1.0).abs() < 1e-9 && !c.speed_curve.is_animated();
        let at_every = every.is_some_and(|s| (c.speed - s).abs() < 1e-9) && !c.speed_curve.is_animated();
        crate::ui::prop_field(ui, "Preset", |ui| {
            let tip = format!(
                "Real time. A {} fps clip in a {} fps project shows the frame nearest each project frame, never a blend",
                crate::ui::inspector::fps_label(src),
                crate::ui::inspector::fps_label(pf)
            );
            if ui.selectable_label(normal, "Normal").on_hover_text(tip).clicked() && !normal {
                speed = Some(1.0);
            }
            if let Some(s) = every {
                let tip = format!(
                    "Slow motion that shows each of the clip's {} fps frames once, one per project frame: {:.0} % speed, no blending",
                    crate::ui::inspector::fps_label(src),
                    s * 100.0
                );
                if ui.selectable_label(at_every, "Every frame").on_hover_text(tip).clicked() && !at_every {
                    speed = Some(s);
                }
            }
            let _ = ui.selectable_label(!normal && !at_every, "Custom").on_hover_text("Any other speed: set it in Speed");
        });
        if (src - pf).abs() > 1e-3 && footage::rate_multiple(src, pf).is_none() && footage::rate_multiple(pf, src).is_none() {
            // an odd mix (24 in 30): how the in-between frames are made
            let cur = if blends(&c) { Mode::Blend } else { Mode::Sample };
            crate::ui::prop_field(ui, "Frame rate", |ui| {
                let name = |m: Mode| match m {
                    Mode::Sample => "Frame sampling",
                    Mode::Blend => "Frame blending",
                    Mode::Flow => "Optical flow",
                };
                egui::ComboBox::from_id_salt("fps_mode").selected_text(name(cur)).width(crate::ui::PROP_VAL_W).show_ui(ui, |ui| {
                    for (m, tip) in [
                        (Mode::Sample, "Shows the nearest real frame: sharp, some frames repeat or drop"),
                        (Mode::Blend, "Mixes neighbouring frames: smoother motion, a little ghosting"),
                        (
                            Mode::Flow,
                            "Renders new in-between frames from the motion (offline, with ffmpeg) and swaps the clip onto the result. Playback never computes it",
                        ),
                    ] {
                        if ui.selectable_label(cur == m, name(m)).on_hover_text(tip).clicked() && cur != m {
                            mode = Some(m);
                        }
                    }
                })
                .response
                .on_hover_text(format!(
                    "This clip is {} fps, the project {} fps: they don't divide evenly",
                    crate::ui::inspector::fps_label(src),
                    crate::ui::inspector::fps_label(pf)
                ));
            });
        }
    }
    let mut changed = false;
    if let Some(s) = speed {
        undo(project);
        set_speed_fit(project, ids, s, c.reverse);
        changed = true;
    }
    match mode {
        Some(Mode::Flow) => request(FootageReq::Flow(c.id)),
        Some(m) => {
            undo(project);
            set_blend(project, ids, m == Mode::Blend);
            changed = true;
        }
        None => {}
    }
    changed | zoom_row(ui, project, &c, playhead, undo)
}

/// Zoom: 100 % = Fit (the whole frame). The tick is 1:1, one source pixel per project pixel.
fn zoom_row(ui: &mut egui::Ui, project: &mut Project, c: &Clip, playhead: f64, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(native) = native_zoom(project, c) else { return false };
    let lt = c.local(playhead);
    let mut pct = c.scale.at(lt) * 100.0;
    let top = (native * 125.0).max(400.0);
    let mut g = Gesture::default();
    let accent = ui.visuals().selection.bg_fill;
    crate::ui::prop_field(ui, "Zoom", |ui| {
        let r = ui.add_sized(
            [crate::ui::PROP_VAL_W, ui.spacing().interact_size.y],
            egui::Slider::new(&mut pct, 100.0..=top).show_value(false),
        );
        let r = r.on_hover_text(format!(
            "{pct:.0} %. 100 % fits the whole frame; the mark is 1:1 ({:.0} %), the sharpest punch-in",
            native * 100.0
        ));
        g.note(&r);
        if native > 1.0 {
            // the slider's handle travels the rail shrunk by its radius at each end (egui's Slider)
            let rail = r.rect.x_range().shrink(r.rect.height() / 2.5);
            let x = rail.min + ((native * 100.0 - 100.0) / (top - 100.0)) as f32 * rail.span();
            let y = r.rect.center().y;
            ui.painter().line_segment([egui::pos2(x, y - 7.0), egui::pos2(x, y + 7.0)], egui::Stroke::new(2.0_f32, accent));
        }
        ui.weak(format!("{pct:.0} %"));
    });
    let upscaled = pct > native * 100.0 + 0.5;
    if native > 1.0 {
        crate::ui::prop_field(ui, "Max sharp", |ui| {
            ui.weak(format!("{:.0} %", native * 100.0)).on_hover_text(
                "Up to this zoom every project pixel comes from a real camera pixel. Past it the picture is upscaled and softens",
            );
        });
    }
    if upscaled {
        crate::ui::prop_field(ui, "", |ui| {
            ui.weak("Upscaled").on_hover_text(
                "The clip has fewer pixels than it covers in the project frame, so it is enlarged and looks softer",
            );
            if crate::engine::enhance::available()
                && ui.small_button("AI Upscale").on_hover_text("Make a sharper 2× or 4× copy of the clip").clicked()
            {
                request(FootageReq::Upscale(c.id, if pct / 100.0 / native > 2.0 { 4 } else { 2 }));
            }
        });
    }
    if g.start {
        undo(project);
    }
    if !g.changed {
        return false;
    }
    let v = pct / 100.0;
    if let Some(cm) = project.clip_mut(c.id) {
        if cm.scale.is_animated() {
            cm.scale.set_at(lt, v);
        } else {
            cm.scale.value = v;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Asset;

    fn project_with(w: u32, h: u32, fps: f64) -> (Project, Id) {
        let mut p = Project::new();
        (p.width, p.height, p.fps) = (1920, 1080, 25.0);
        let mut a = crate::engine::import::placeholder("C:\\clips\\a.mp4");
        (a.kind, a.width, a.height, a.fps, a.duration) = (ClipKind::Video, w, h, fps, 10.0);
        let aid = p.add_asset(a);
        let mut c = Clip::new(p.new_id(), ClipKind::Video, "a", 0.0, 4.0);
        c.asset = aid;
        let id = c.id;
        p.tracks[0].clips.push(c);
        (p, id)
    }

    fn asset_of(p: &Project, id: Id) -> Asset {
        p.asset(p.clip(id).unwrap().asset).unwrap().clone()
    }

    /// "Every frame" on a 50p clip in a 25p project is exactly half speed, and each project frame then lands
    /// on the next source frame: all of them, in order, none twice, none skipped.
    #[test]
    fn every_frame_is_exact_half_speed_and_maps_every_source_frame() {
        let (mut p, id) = project_with(3840, 2160, 50.0);
        let s = every_frame_for(&p, p.clip(id).unwrap()).unwrap();
        assert_eq!(s, 0.5);
        assert!(p.set_speed(&[id], s, false));
        let c = p.clip(id).unwrap().clone();
        assert_eq!(c.duration, 8.0, "4 s of source at half speed");
        let src = asset_of(&p, id).fps;
        let frames: Vec<i64> = (0..(c.duration * p.fps) as i64)
            .map(|k| (c.src_time(k as f64 / p.fps) * src + 1e-6).floor() as i64)
            .collect();
        assert_eq!(frames, (0..200).collect::<Vec<_>>(), "every source frame once");
        assert!(!blends(&c), "conforming never adds blending");
        assert_eq!(badge(&p, &c), None, "the badge goes once the clip is retimed");
    }

    /// At normal speed a 50p clip in 25p samples every 2nd source frame (no blend effect, whole frames).
    #[test]
    fn normal_speed_samples_every_nth_frame_never_blends() {
        let (p, id) = project_with(1920, 1080, 50.0);
        let c = p.clip(id).unwrap();
        assert_eq!(badge(&p, c).as_deref(), Some("50"));
        let frames: Vec<i64> = (0..10).map(|k| (c.src_time(k as f64 / 25.0) * 50.0 + 1e-6).floor() as i64).collect();
        assert_eq!(frames, vec![0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);
        assert!(c.effects.is_empty(), "no Frame Blend unless the user picks Frame blending");
        let (p2, id2) = project_with(1920, 1080, 30.0);
        assert_eq!(every_frame_for(&p2, p2.clip(id2).unwrap()), None, "30 in 25 is no multiple");
    }

    /// No room after the clip: every-frame slow motion keeps the clip's length (the start of the window).
    #[test]
    fn slow_motion_without_room_keeps_the_length() {
        let (mut p, id) = project_with(1920, 1080, 50.0);
        let mut next = Clip::new(p.new_id(), ClipKind::Video, "b", 4.0, 2.0);
        next.asset = p.clip(id).unwrap().asset;
        p.tracks[0].clips.push(next);
        assert!(!set_speed_fit(&mut p, &[id], 0.5, false));
        let c = p.clip(id).unwrap();
        assert_eq!((c.duration, c.speed, c.src_len()), (4.0, 0.5, 2.0));
    }

    #[test]
    fn blending_is_the_frame_blend_effect_and_sampling_removes_it() {
        let (mut p, id) = project_with(1920, 1080, 24.0);
        set_blend(&mut p, &[id], true);
        set_blend(&mut p, &[id], true);
        assert_eq!(p.clip(id).unwrap().effects.len(), 1, "once");
        set_blend(&mut p, &[id], false);
        assert!(p.clip(id).unwrap().effects.is_empty());
    }

    /// 4K in a 1080p project fits the whole frame by default (scale 1 = contain) and 1:1 is 200 %.
    #[test]
    fn fit_is_the_default_and_one_to_one_is_200_percent_for_4k_in_1080() {
        let (p, id) = project_with(3840, 2160, 50.0);
        let c = p.clip(id).unwrap();
        let pl = crate::engine::compose::placement(&p, c, 0.0, (3840, 2160), 1920, 1080, true);
        assert_eq!((pl.w, pl.h), (1920.0, 1080.0), "Fit: the full frame, edge to edge");
        assert_eq!(native_zoom(&p, c), Some(2.0));
        let mut zoomed = c.clone();
        zoomed.scale.value = 2.0;
        let pl = crate::engine::compose::placement(&p, &zoomed, 0.0, (3840, 2160), 1920, 1080, true);
        assert_eq!((pl.w, pl.h), (3840.0, 2160.0), "at the 1:1 mark one source pixel = one project pixel");
        // a 720p clip is upscaled even at Fit
        let (p, id) = project_with(1280, 720, 25.0);
        assert!(native_zoom(&p, p.clip(id).unwrap()).unwrap() < 1.0);
    }
}
