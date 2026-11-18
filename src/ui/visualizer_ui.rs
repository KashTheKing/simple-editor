//! ---- ws:audio-visualizer ----
//! The Inspector's "Visualizer" section for a clip carrying a visualizer effect (`engine::visualizer`):
//! pick the style (built-ins + the user's audio-reading library effects), bake the audio under the clip
//! again after moving or trimming it, and the band mixer (which bands drive it and how). Commits
//! straight to the project, with one undo per click or drag; a mix change re-bakes from the song's
//! cached analysis (`visualizer::analysis`), so it shows at once and costs playback nothing.

use crate::engine::visualizer;
use crate::media::Backend;
use crate::model::{Animated, AudioMix, Id, Project, VizCamera};
use crate::theme::Palette;
use eframe::egui;

/// The clip's current style name, if it is a visualizer clip.
pub fn style(project: &Project, id: Id) -> Option<String> {
    project.clip(id)?.effects.iter().find_map(visualizer::style_of)
}

/// Draw the section; true when the project changed.
/// `last` is `Settings::visualizer_style`: a pick becomes the style the next visualizer gets.
pub fn section(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: Id,
    backend: Backend,
    last: &mut String,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(current) = style(project, id) else { return false };
    let err_id = egui::Id::new(("visualizer_err", id));
    let mut pick: Option<visualizer::Style> = None;
    let mut rebake = false;
    let mut to_mix = false;
    let source = visualizer::source_of(project, id);
    egui::Grid::new("inspector_visualizer").num_columns(2).show(ui, |ui| {
        ui.label("Type");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("visualizer_style").selected_text(&current).show_ui(ui, |ui| {
                // ponytail: lists the library folder each frame the popup is open (a few small files)
                for s in visualizer::styles() {
                    let label = if s.custom { format!("{} (custom)", s.name) } else { s.name.clone() };
                    if ui.selectable_label(s.name == current, label).clicked() && s.name != current {
                        pick = Some(s);
                    }
                }
            });
            // ws:preview-audio-only: a GLSL visualizer of your own, checked, into the library, then used
            let r = ui.small_button("Import shader…");
            if r.on_hover_text("Load a GLSL visualizer file: it joins the types in your library").clicked() {
                if let Some(path) =
                    rfd::FileDialog::new().add_filter("GLSL", &["glsl", "frag", "fs", "txt"]).pick_file()
                {
                    match import_shader(&path) {
                        Ok(s) => pick = Some(s),
                        Err(e) => ui.ctx().data_mut(|d| d.insert_temp(err_id, e)),
                    }
                }
            }
        });
        ui.end_row();
        ui.label("Source");
        ui.horizontal(|ui| {
            let song = source.and_then(|a| project.clip(a)).map(|a| a.name.clone());
            let shown = song.clone().unwrap_or_else(|| MIX_SOURCE.to_string());
            egui::ComboBox::from_id_salt(("visualizer_source", id))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if let Some(s) = &song {
                        let _ = ui.selectable_label(true, s.as_str());
                    }
                    if ui.selectable_label(song.is_none(), MIX_SOURCE).clicked() && song.is_some() {
                        to_mix = true;
                    }
                })
                .response
                .on_hover_text("What it reacts to: its linked song, or everything the timeline plays at that moment");
            if ui.small_button("Re-bake").on_hover_text("Read the audio again (after moving or trimming it)").clicked()
            {
                rebake = true;
            }
        });
        ui.end_row();
    });
    if to_mix {
        undo(project);
        visualizer::follow_mix(project, id);
        let r = visualizer::rebake_any(project, id, backend).map(|_| ());
        if let Err(e) = r {
            ui.ctx().data_mut(|d| d.insert_temp(err_id, e));
        }
        return true;
    }
    // ---- the band mixer
    let mix_now = project
        .clip(id)
        .and_then(|c| c.effects.iter().find(|e| visualizer::style_of(e).is_some()))
        .and_then(|e| e.audio_mix.clone())
        .unwrap_or_default();
    let spectrum = source.and_then(|a| {
        let (c, v) = (project.clip(a)?, project.clip(id)?);
        let path = project.asset(c.asset)?.path.clone();
        let ctx = ui.ctx().clone();
        let song = visualizer::analysis_async(&path, c.audio_stream, backend, move || ctx.request_repaint())?;
        let (s0, s1) = (c.src_time(v.start), c.src_time(v.start + v.duration));
        Some(song.spectrum(s0.min(s1), s0.max(s1)))
    });
    // a Timeline-mix visualizer: the mix's spectrum under it (once the background analysis has landed)
    let spectrum = spectrum.or_else(|| {
        let v = project.clip(id).filter(|_| source.is_none())?;
        visualizer::mix_cached().map(|(_, m)| m.spectrum(v.start, v.start + v.duration))
    });
    let (mix, mix_started) = mixer(ui, id, &mix_now, spectrum);
    if let Some(e) = ui.ctx().data(|d| d.get_temp::<String>(err_id)) {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    if mix != mix_now {
        if mix_started {
            undo(project);
        }
        if let Some(e) = project
            .clip_mut(id)
            .and_then(|c| c.effects.iter_mut().find(|e| visualizer::style_of(e).is_some()))
        {
            e.audio_mix = (mix != AudioMix::default()).then_some(mix);
        }
        if pick.is_none() {
            rebake = true;
        }
    }
    let mix_changed = project
        .clip(id)
        .and_then(|c| c.effects.iter().find(|e| visualizer::style_of(e).is_some()))
        .map(|e| e.audio_mix.clone().unwrap_or_default())
        != Some(mix_now);
    let r = match (pick, rebake) {
        (Some(s), _) => {
            undo(project);
            *last = s.name.clone();
            Some(visualizer::restyle(project, id, &s, backend))
        }
        (None, true) => {
            if !mix_changed {
                undo(project); // the Re-bake button (a mix edit pushed its own undo above)
            }
            Some(visualizer::rebake_any(project, id, backend).map(|_| ()))
        }
        _ => None,
    };
    match r {
        Some(Ok(())) => {
            ui.ctx().data_mut(|d| d.remove::<String>(err_id));
            true
        }
        Some(Err(e)) => {
            ui.ctx().data_mut(|d| d.insert_temp(err_id, e));
            true
        }
        None => mix_changed,
    }
}

/// The Source choice for a visualizer that is not linked to a song.
pub const MIX_SOURCE: &str = "Timeline mix";

/// Read, check (`visualizer::check_import`) and save a GLSL visualizer to the library's effects: a new type.
fn import_shader(path: &std::path::Path) -> Result<visualizer::Style, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = visualizer::check_import(&src, &stem)?;
    let item = crate::engine::library::save(crate::engine::library::Kind::Effect, &name, &src)?;
    Ok(visualizer::styles().into_iter().find(|s| s.src == src).unwrap_or(visualizer::Style {
        name: item.id,
        src,
        custom: true,
    }))
}

/// The band response curve: one point per band (x = the band, low to high frequency; y = its weight,
/// 0 at the bottom to `MAX_GAIN` at the top). Drag a point up or down; double-click puts it back to 1.
/// The song's own spectrum is drawn behind it, dim. Returns true while a point moved this frame.
fn curve_editor(ui: &mut egui::Ui, m: &mut AudioMix, spectrum: Option<[f32; 6]>) -> egui::Response {
    let (rect, r) =
        ui.allocate_exact_size(egui::vec2(ui.available_width().min(300.0), 96.0), egui::Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let plot = rect.shrink2(egui::vec2(10.0, 8.0)).with_max_y(rect.bottom() - 14.0);
    let x_of = |b: usize| plot.left() + plot.width() * b as f32 / 5.0;
    let y_of = |g: f32| plot.bottom() - plot.height() * (g / MAX_GAIN).clamp(0.0, 1.0);
    // the 1.0 line, the song's spectrum
    p.hline(plot.x_range(), y_of(1.0), egui::Stroke::new(1.0, ui.visuals().weak_text_color().gamma_multiply(0.4)));
    if let Some(s) = spectrum {
        for b in 0..6 {
            let h = plot.height() * s[b].sqrt().clamp(0.0, 1.0);
            let x = x_of(b);
            p.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x - 6.0, plot.bottom() - h), egui::pos2(x + 6.0, plot.bottom())),
                1.0,
                ui.visuals().weak_text_color().gamma_multiply(0.3),
            );
        }
    }
    // drag the nearest point
    let mut moved = false;
    if let Some(pos) = r.interact_pointer_pos() {
        let b = (((pos.x - plot.left()) / plot.width() * 5.0).round().clamp(0.0, 5.0)) as usize;
        if r.double_clicked() {
            m.gains[b] = 1.0;
            moved = true;
        } else if r.dragged() || r.clicked() {
            m.gains[b] = gain_at(plot, pos.y);
            moved = true;
        }
    }
    // the curve (Catmull-Rom through the points), then the points
    let pts: Vec<egui::Pos2> = (0..6).map(|b| egui::pos2(x_of(b), y_of(m.gains[b]))).collect();
    let mut line = Vec::new();
    for i in 0..5usize {
        let (p0, p1, p2, p3) = (pts[i.saturating_sub(1)], pts[i], pts[i + 1], pts[(i + 2).min(5)]);
        for k in 0..12 {
            let t = k as f32 / 12.0;
            let (t2, t3) = (t * t, t * t * t);
            let c = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            line.push(egui::pos2(
                c(p0.x, p1.x, p2.x, p3.x),
                c(p0.y, p1.y, p2.y, p3.y).clamp(plot.top(), plot.bottom()),
            ));
        }
    }
    line.push(pts[5]);
    let accent = ui.visuals().selection.bg_fill;
    p.add(egui::Shape::line(line, egui::Stroke::new(2.0, accent)));
    for (b, q) in pts.iter().enumerate() {
        p.circle_filled(*q, 4.5, accent);
        p.text(
            egui::pos2(q.x, rect.bottom() - 1.0),
            egui::Align2::CENTER_BOTTOM,
            AudioMix::BANDS[b].0,
            egui::FontId::proportional(9.0),
            ui.visuals().text_color(),
        );
    }
    let mut r = r.on_hover_text("Frequency left to right, weight bottom to top: drag a point; double-click resets it");
    if moved {
        r.mark_changed();
    }
    r
}

/// A band's weight top-to-bottom of the curve editor's plot: `MAX_GAIN` at the top, 0 at the bottom.
fn gain_at(plot: egui::Rect, y: f32) -> f32 {
    ((plot.bottom() - y) / plot.height().max(1.0) * MAX_GAIN).clamp(0.0, MAX_GAIN)
}

/// The top of a band's weight (the Intensity sliders' range).
const MAX_GAIN: f32 = 3.0;

/// The effect index of `id`'s 3D visualizer (a `// @camera` shader), if it has one.
fn camera_effect(project: &Project, id: Id) -> Option<usize> {
    project.clip(id)?.effects.iter().position(|e| visualizer::camera_at(e, 0.0).is_some())
}

/// The Inspector's Camera section for a 3D visualizer: auto-orbit + speed, then yaw, pitch, distance,
/// field of view, roll and the target, each a value with its ◆ keyframe button (clip-local `lt`), and
/// Reset camera. One undo per edit (the Inspector's `Gesture`). True when the project changed.
pub fn camera_section(ui: &mut egui::Ui, project: &mut Project, id: Id, lt: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(ei) = camera_effect(project, id) else { return false };
    let Some(e) = project.clip(id).map(|c| &c.effects[ei]) else { return false };
    let had = e.camera.is_some();
    let mut cam = e.camera.clone().unwrap_or_default();
    let mut g = crate::ui::Gesture::default();
    let mut reset = false;
    egui::Grid::new(("visualizer_camera", id)).num_columns(3).show(ui, |ui| {
        ui.label("Auto-orbit");
        g.note(&ui.checkbox(&mut cam.auto_orbit, ""));
        let r = ui.add_enabled(cam.auto_orbit, egui::DragValue::new(&mut cam.orbit_speed).speed(0.1).suffix(" °/s").range(-90.0..=90.0));
        g.note(&r);
        ui.end_row();
        let rows: [(&str, &mut Animated, f64, &str, (f64, f64)); 8] = [
            ("Yaw", &mut cam.yaw, 0.5, "°", (-100_000.0, 100_000.0)),
            ("Pitch", &mut cam.pitch, 0.5, "°", (-89.0, 89.0)),
            ("Distance", &mut cam.distance, 0.1, "", (0.5, 500.0)),
            ("Field of view", &mut cam.fov, 0.2, "°", (5.0, 150.0)),
            ("Roll", &mut cam.roll, 0.5, "°", (-180.0, 180.0)),
            ("Target X", &mut cam.target_x, 0.05, "", (-1000.0, 1000.0)),
            ("Target Y", &mut cam.target_y, 0.05, "", (-1000.0, 1000.0)),
            ("Target Z", &mut cam.target_z, 0.05, "", (-1000.0, 1000.0)),
        ];
        for (name, a, speed, suffix, (lo, hi)) in rows {
            ui.label(name);
            let mut v = a.at(lt);
            let r = ui.add(egui::DragValue::new(&mut v).speed(speed).suffix(suffix).range(lo..=hi).max_decimals(2));
            g.note(&r);
            if r.changed() {
                a.set_at(lt, v);
            }
            crate::ui::key_buttons(ui, a, lt, palette, &mut g, &format!("camera {name}"), &[]);
            ui.end_row();
        }
    });
    if ui.add_enabled(had, egui::Button::new("Reset camera").small()).on_hover_text("Back to the default slow orbit").clicked() {
        reset = true;
        g.click();
    }
    if !(g.changed || reset) {
        return false;
    }
    if g.start {
        undo(project);
    }
    if let Some(e) = project.clip_mut(id).map(|c| &mut c.effects[ei]) {
        e.camera = if reset { None } else { Some(cam) };
    }
    true
}

/// The Preview's camera gestures on a selected 3D visualizer under the playhead: drag orbits (yaw /
/// pitch), Shift+drag pans the target across the view, the scroll wheel dollies. None when `id` is not
/// one (the Preview's own move / handles apply); Some(edited) when it took the gesture. Values are set
/// at the clip-local playhead, so a keyframed camera gets a key there (the Inspector's `set_at`). Only
/// the camera uniforms change: no re-render cost beyond the frame itself.
pub fn camera_gesture(
    ui: &egui::Ui,
    resp: &egui::Response,
    project: &mut Project,
    id: Id,
    playhead: f64,
    undo: &mut dyn FnMut(&Project),
) -> Option<bool> {
    let cl = project.clip(id)?;
    if playhead < cl.start || playhead >= cl.start + cl.duration {
        return None;
    }
    let lt = cl.local(playhead);
    let ei = camera_effect(project, id)?;
    let (shift, ctrl, scroll) = ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.raw_scroll_delta.y));
    let scroll = if resp.hovered() && !ctrl { scroll } else { 0.0 };
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
    }
    let dragging = resp.dragged_by(egui::PointerButton::Primary);
    if !dragging && scroll == 0.0 {
        return Some(false);
    }
    // one undo per drag, and per burst of wheel turns (a pause of half a second starts a new one)
    let now = ui.input(|i| i.time);
    let last_id = egui::Id::new(("visualizer_camera_scroll", id));
    let last_scroll = ui.ctx().data(|d| d.get_temp::<f64>(last_id)).unwrap_or(f64::NEG_INFINITY);
    if resp.drag_started_by(egui::PointerButton::Primary) || (!dragging && now - last_scroll > 0.5) {
        undo(project);
    }
    if scroll != 0.0 {
        ui.ctx().data_mut(|d| d.insert_temp(last_id, now));
    }
    let basis = visualizer::camera_at(&project.clip(id)?.effects[ei], lt)?;
    let e = &mut project.clip_mut(id)?.effects[ei];
    let cam = e.camera.get_or_insert_with(VizCamera::default);
    let d = resp.drag_delta();
    if dragging && shift {
        // pan: move the target along the view's right / up axes, scaled to the distance
        let (p, t) = (basis.pos.map(f64::from), basis.target.map(f64::from));
        let fw = [t[0] - p[0], t[1] - p[1], t[2] - p[2]];
        let len = (fw[0] * fw[0] + fw[1] * fw[1] + fw[2] * fw[2]).sqrt().max(1e-6);
        let fw = fw.map(|v| v / len);
        let rt = [-fw[2], 0.0, fw[0]];
        let rl = (rt[0] * rt[0] + rt[2] * rt[2]).sqrt().max(1e-6);
        let rt = rt.map(|v| v / rl);
        let up = [rt[1] * fw[2] - rt[2] * fw[1], rt[2] * fw[0] - rt[0] * fw[2], rt[0] * fw[1] - rt[1] * fw[0]];
        let k = len * 0.002;
        let (dx, dy) = (d.x as f64 * k, d.y as f64 * k);
        for (a, i) in [(&mut cam.target_x, 0), (&mut cam.target_y, 1), (&mut cam.target_z, 2)] {
            let v = a.at(lt) - rt[i] * dx + up[i] * dy;
            a.set_at(lt, v);
        }
    } else if dragging {
        let (yaw, pitch) = (cam.yaw.at(lt) + d.x as f64 * 0.3, (cam.pitch.at(lt) + d.y as f64 * 0.3).clamp(-89.0, 89.0));
        cam.yaw.set_at(lt, yaw);
        cam.pitch.set_at(lt, pitch);
    }
    if scroll != 0.0 {
        let dist = (cam.distance.at(lt) * (-(scroll as f64) * 0.002).exp()).clamp(0.5, 500.0);
        cam.distance.set_at(lt, dist);
    }
    Some(true)
}

/// The band mixer's widgets: a preset, the spectrum graph, per band a gain and a beat-source tick, then
/// sensitivity and smoothing. Returns the edited mix and whether this frame STARTED an interaction
/// (a click, a typed value, a drag's first move) - the caller's one undo per edit.
/// The band mixer's title in the Inspector.
pub const BANDS_TITLE: &str = "Frequency bands";

fn mixer(ui: &mut egui::Ui, id: Id, cur: &AudioMix, spectrum: Option<[f32; 6]>) -> (AudioMix, bool) {
    let mut m = cur.clone();
    let mut started = false;
    ui.add_space(4.0);
    // ws:preview-audio-only: titled, so the band controls are found
    ui.separator();
    ui.label(egui::RichText::new(BANDS_TITLE).strong())
        .on_hover_text("Which frequencies drive the visualizer, how hard, and which ones its beat follows");
    ui.horizontal(|ui| {
        ui.label("Preset");
        let shown = if m.preset.is_empty() { "Custom".to_string() } else { m.preset.clone() };
        egui::ComboBox::from_id_salt(("visualizer_preset", id)).selected_text(shown).show_ui(ui, |ui| {
            for p in AudioMix::PRESETS {
                if ui.selectable_label(m.preset == p, p).clicked() {
                    if let Some(next) = AudioMix::preset(p) {
                        m = next;
                        started = true;
                    }
                }
            }
        });
        if ui.small_button("Reset").on_hover_text("Every band equal, the default smoothing").clicked() {
            m = AudioMix::default();
            started = true;
        }
    });
    // ws:preview-audio-only: the band response curve (the sound's spectrum behind it)
    let before = m.clone();
    let curve = curve_editor(ui, &mut m, spectrum);
    if spectrum.is_none() {
        ui.weak("reading the sound…");
    }
    // a drag owes ONE undo, paid by its first frame that actually changes a value
    let owed_id = egui::Id::new(("visualizer_mix_owed", id));
    let mut owed = ui.ctx().data(|d| d.get_temp::<bool>(owed_id)).unwrap_or(false);
    let mut note = |r: &egui::Response| {
        if r.drag_started() {
            owed = true;
        }
        if r.changed() && (owed || !r.dragged()) {
            started = true;
            owed = false;
        }
    };
    note(&curve);
    egui::Grid::new(("visualizer_bands", id)).num_columns(3).show(ui, |ui| {
        ui.weak("Band");
        ui.weak("Intensity");
        ui.weak("Beat source").on_hover_text("Which bands the beat detection listens to (none = the whole sound)");
        ui.end_row();
        for (b, (name, lo, hi)) in AudioMix::BANDS.iter().enumerate() {
            ui.label(*name).on_hover_text(format!("{lo:.0} - {hi:.0} Hz"));
            note(&ui.add(egui::Slider::new(&mut m.gains[b], 0.0..=3.0).fixed_decimals(2)));
            note(&ui.checkbox(&mut m.beat_bands[b], ""));
            ui.end_row();
        }
        ui.label("Sensitivity");
        note(&ui.add(egui::Slider::new(&mut m.sensitivity, 0.25..=4.0).logarithmic(true).fixed_decimals(2)));
        ui.end_row();
        ui.label("Attack");
        note(&ui.add(egui::Slider::new(&mut m.attack, 0.0..=0.5).suffix(" s").fixed_decimals(2)));
        ui.end_row();
        ui.label("Release");
        note(&ui.add(egui::Slider::new(&mut m.release, 0.02..=2.0).logarithmic(true).suffix(" s").fixed_decimals(2)));
        ui.end_row();
    });
    ui.ctx().data_mut(|d| d.insert_temp(owed_id, owed));
    if m != before {
        m.preset.clear(); // edited by hand: no longer that preset
    }
    (m.clamped(), started)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(p: &mut Project, id: Id) -> (bool, usize, String) {
        let ctx = egui::Context::default();
        let (mut changed, mut undos) = (false, 0);
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed = section(ui, p, id, Backend::Auto, &mut String::new(), &mut |_| undos += 1);
            });
        });
        let text: String = out.shapes.iter().map(|s| format!("{:?}", s.shape)).collect();
        (changed, undos, text)
    }

    /// The curve editor's height is the band weight: top = the maximum, bottom = 0, clamped outside.
    #[test]
    fn the_curve_maps_height_to_weight() {
        let plot = egui::Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(100.0, 70.0));
        assert_eq!(gain_at(plot, 10.0), MAX_GAIN);
        assert_eq!(gain_at(plot, 70.0), 0.0);
        assert!((gain_at(plot, 40.0) - MAX_GAIN / 2.0).abs() < 1e-4);
        assert_eq!(gain_at(plot, -50.0), MAX_GAIN);
        assert_eq!(gain_at(plot, 500.0), 0.0);
    }

    /// A visualizer clip gets the Style picker showing its style; a plain shape gets nothing; drawing
    /// alone never edits or pushes an undo.
    #[test]
    fn section_shows_the_style_and_is_idle() {
        let mut p = Project::new();
        let plain = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 1.0);
        let vis = p.add_shape_clip(crate::model::ShapeKind::Rect, 2.0, 1.0);
        p.clip_mut(vis).unwrap().effects.push(crate::model::Effect::from_shader(visualizer::BUILTIN[1].1));
        assert_eq!(style(&p, vis).as_deref(), Some("Spectrum Bars"));
        assert_eq!(style(&p, plain), None);
        let before = p.to_json();
        let (changed, undos, text) = draw(&mut p, vis);
        assert!(!changed && undos == 0 && p.to_json() == before);
        assert!(text.contains("Spectrum Bars") && text.contains("Timeline mix"), "{text}");
        for w in [
            BANDS_TITLE,
            "Preset",
            "Custom",
            "Sub",
            "High-mid",
            "Air",
            "Intensity",
            "Beat source",
            "Sensitivity",
            "Release",
        ] {
            assert!(text.contains(w), "the band mixer shows {w}");
        }
        assert!(!draw(&mut p, plain).0);
    }
}
