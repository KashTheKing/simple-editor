//! ---- ws:beat-aligner ----
//! The Beat Aligner pane: a song's waveform under a beat ruler and its sections, with the marks
//! placed on it. Tapping, continuous fill and the player live in `app::beats_ctl`; this draws and edits
//! the sheet (`model::beats`) and asks the App for the rest through `Resp`.

use crate::model::beats::{self, BeatFmt, SECTION_PRESETS};
use crate::model::*;
use crate::theme::Palette;
use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};

/// What the pane remembers between frames (not saved: the sheet itself is in the project).
pub struct AlignState {
    /// Space / B / M drop a mark while the song plays.
    pub armed: bool,
    /// Grid beats are marked as the playhead passes them.
    pub continuous: bool,
    pub snap: bool,
    pub strength: f32,
    /// Grid subdivision for snapping (1 = beats, 2 = eighths, 4 = sixteenths).
    pub div: u32,
    pub sel: Vec<Id>,
    pub sel_section: Option<Id>,
    /// Song seconds shown (None = all of it).
    pub view: Option<(f64, f64)>,
    /// A dragged range (song seconds), for a new section.
    pub range: Option<(f64, f64)>,
    drag: Option<Drag>,
    pub section_name: String,
    /// Calibration taps (song seconds as heard) while calibrating.
    pub cal: Option<Vec<f64>>,
    /// Tap-tempo presses (seconds on the UI clock).
    pub tempo_taps: Vec<f64>,
    pub every: usize,
    pub cut_all_video: bool,
    pub sfx: Id,
    pub sfx_lead: f64,
    pub block: String,
    /// Waiting for the song analysis to land before detecting.
    pub detect_pending: bool,
    /// Continuous mode: song time filled up to, and whether this play has its undo step yet.
    pub filled_to: Option<f64>,
    /// Where the waveform was drawn last frame (tests drive the pointer there).
    pub canvas: Rect,
    /// The (asset, voice) this state belongs to: another song (from a tool, undo, a pick) resets it.
    seen: (Id, bool),
}

impl Default for AlignState {
    fn default() -> Self {
        Self {
            armed: false,
            continuous: false,
            snap: false,
            strength: 1.0,
            div: 1,
            sel: Vec::new(),
            sel_section: None,
            view: None,
            range: None,
            drag: None,
            section_name: String::new(),
            cal: None,
            tempo_taps: Vec::new(),
            every: 1,
            cut_all_video: true,
            sfx: 0,
            sfx_lead: 0.03,
            block: "Zoom Punch".into(),
            detect_pending: false,
            filled_to: None,
            canvas: Rect::NOTHING,
            seen: (0, false),
        }
    }
}

impl AlignState {
    /// A new song: forget the selection, zoom, range and anything mid-way (mark ids start again per sheet).
    fn new_song(&mut self, seen: (Id, bool)) {
        (self.sel, self.sel_section, self.view, self.range, self.drag) = (Vec::new(), None, None, None, None);
        (self.cal, self.filled_to) = (None, None);
        self.seen = seen;
    }
}

enum Drag {
    /// Moving the selected marks: where the press was (song s) and the marks as they were.
    Marks { from: f64, before: Vec<BeatMark> },
    /// Sweeping a range from this song second.
    Range { from: f64 },
}

/// What the App supplies for one frame.
pub struct View<'a> {
    pub peaks: Option<&'a crate::media::waveform::Peaks>,
    /// Song length, seconds.
    pub dur: f64,
    /// The song's library assets / audio clips to pick from: (clip id or 0, asset id, label).
    pub songs: &'a [(Id, Id, String)],
    /// Sound assets for "Add SFX": (asset id, name).
    pub sounds: &'a [(Id, String)],
    pub blocks: &'a [String],
    /// Player position, song seconds (None when the song is not on the timeline).
    pub now: Option<f64>,
    pub playing: bool,
    pub latency_ms: f64,
}

#[derive(Default)]
pub struct Resp {
    pub edited: bool,
    /// Seek the player to this timeline second.
    pub seek: Option<f64>,
    pub play: bool,
    pub tap: bool,
    pub detect: bool,
    pub calibrate: bool,
    pub latency_ms: Option<f64>,
    pub add_to_timeline: bool,
    pub to_markers: bool,
    pub cut: bool,
    pub sfx: bool,
    pub snap_clips: bool,
    pub effect: bool,
    pub export: Option<BeatFmt>,
    pub import: bool,
    pub toast: Option<String>,
}

const RULER_H: f32 = 22.0;
const BAND_H: f32 = 18.0;
const WAVE_H: f32 = 150.0;
const HIT_PX: f32 = 5.0;

fn label_color(p: &Project, label: u8) -> Color32 {
    let [r, g, b] = p.label_color(label).unwrap_or([150, 150, 150]);
    Color32::from_rgb(r, g, b)
}

pub fn show(
    ui: &mut egui::Ui,
    st: &mut AlignState,
    project: &mut Project,
    view: &View,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> Resp {
    let mut out = Resp::default();
    if st.seen != (project.beats.asset, project.beats.voice) {
        st.new_song((project.beats.asset, project.beats.voice));
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        source_row(ui, st, project, view, &mut out, undo);
        if project.beats.asset == 0 {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Pick a song or voiceover above, or select an audio clip on the timeline.")
                    .color(pal.text_dim),
            );
            return;
        }
        tap_row(ui, st, project, view, &mut out, undo);
        canvas(ui, st, project, view, pal, &mut out, undo);
        ui.add_space(4.0);
        sections(ui, st, project, &mut out, undo);
        ui.separator();
        use_row(ui, st, project, view, &mut out);
    });
    out
}

fn source_row(
    ui: &mut egui::Ui,
    st: &mut AlignState,
    p: &mut Project,
    view: &View,
    out: &mut Resp,
    undo: &mut dyn FnMut(&Project),
) {
    ui.horizontal_wrapped(|ui| {
        let cur = view
            .songs
            .iter()
            .find(|(c, a, _)| *a == p.beats.asset && (*c == p.beats.clip || p.beats.clip == 0))
            .map_or("Choose a song", |s| s.2.as_str())
            .to_string();
        let mut pick = None;
        egui::ComboBox::from_id_salt("beat_song").selected_text(cur).width(200.0).show_ui(ui, |ui| {
            for (c, a, name) in view.songs {
                if ui.selectable_label(*a == p.beats.asset && *c == p.beats.clip, name).clicked() {
                    pick = Some((*c, *a));
                }
            }
        });
        let mut voice = p.beats.voice;
        ui.selectable_value(&mut voice, false, "Music").on_hover_text("Beats, bars and tempo");
        ui.selectable_value(&mut voice, true, "Voice").on_hover_text("Phrases and pauses instead of beats");
        if let Some((c, a)) = pick.or((voice != p.beats.voice).then_some((p.beats.clip, p.beats.asset))) {
            undo(p);
            let clip = (c != 0).then_some(c);
            if p.set_beat_source(clip, Some(a), voice) {
                st.new_song((p.beats.asset, p.beats.voice));
                out.detect = p.beats.marks.is_empty() && p.beats.bpm == 0.0;
                out.edited = true;
            }
        }
        if p.beats.asset == 0 {
            return;
        }
        if ui
            .button(if st.detect_pending { "Detecting…" } else { "Detect" })
            .on_hover_text(if p.beats.voice {
                "Mark where phrases start and pauses begin"
            } else {
                "Find the tempo, the beat grid and the downbeats"
            })
            .clicked()
        {
            out.detect = true;
        }
        if p.beats.voice {
            return;
        }
        ui.separator();
        ui.label("BPM");
        let mut bpm = p.beats.bpm;
        let r = ui.add(egui::DragValue::new(&mut bpm).range(0.0..=400.0).speed(0.05).fixed_decimals(2));
        if r.drag_started() || (r.changed() && !r.dragged()) {
            undo(p);
        }
        if r.changed() {
            p.beats.bpm = bpm;
            out.edited = true;
        }
        for (txt, by, tip) in [("½", 0.5, "Half time"), ("×2", 2.0, "Double time")] {
            if ui.button(txt).on_hover_text(tip).clicked() && p.beats.bpm > 0.0 {
                undo(p);
                p.beats.scale_tempo(by);
                out.edited = true;
            }
        }
        if ui.button("Tap tempo").on_hover_text("Click on the beat a few times; the last click is a downbeat").clicked()
        {
            let now = ui.input(|i| i.time);
            if st.tempo_taps.last().is_some_and(|t| now - t > 2.5) {
                st.tempo_taps.clear();
            }
            st.tempo_taps.push(now);
            if let Some(b) = beats::tap_tempo(&st.tempo_taps) {
                undo(p);
                p.beats.bpm = (b * 10.0).round() / 10.0;
                if let Some(t) = view.now {
                    p.beats.offset = t;
                }
                out.edited = true;
            }
        }
        ui.label("Offset");
        let mut off = p.beats.offset;
        let r = ui.add(
            egui::DragValue::new(&mut off).range(0.0..=view.dur.max(0.0)).speed(0.002).fixed_decimals(3).suffix(" s"),
        );
        if r.drag_started() || (r.changed() && !r.dragged()) {
            undo(p);
        }
        if r.changed() {
            p.beats.offset = off;
            out.edited = true;
        }
        ui.label("Beats per bar");
        let mut m = p.beats.meter;
        if ui.add(egui::DragValue::new(&mut m).range(1..=12)).changed() {
            undo(p);
            p.beats.meter = m;
            out.edited = true;
        }
    });
}

fn tap_row(
    ui: &mut egui::Ui,
    st: &mut AlignState,
    p: &mut Project,
    view: &View,
    out: &mut Resp,
    undo: &mut dyn FnMut(&Project),
) {
    ui.horizontal_wrapped(|ui| {
        if view.now.is_none() {
            if ui
                .button("Add to timeline")
                .on_hover_text("Put the song on an audio track so it can play while you tap")
                .clicked()
            {
                out.add_to_timeline = true;
            }
        } else if ui.button(if view.playing { "⏸ Pause" } else { "▶ Play" }).clicked() {
            out.play = true;
        }
        ui.toggle_value(&mut st.armed, "Tap keys").on_hover_text("While it plays, Space, B or M drops a mark");
        if ui.button("Tap").on_hover_text("Drop a mark at the player position").clicked() {
            out.tap = true;
        }
        if !p.beats.voice {
            ui.toggle_value(&mut st.continuous, "Continuous").on_hover_text("Mark every beat of the grid as it plays");
            if ui.button("Fill grid").on_hover_text("Mark every beat of the grid at once").clicked()
                && p.beats.bpm > 0.0
            {
                undo(p);
                let n = p.beats.fill(-1.0, view.dur);
                out.toast = Some(format!("{n} beats marked"));
                out.edited = true;
            }
            ui.separator();
            ui.checkbox(&mut st.snap, "Snap")
                .on_hover_text("Taps and dragged marks move toward the nearest beat (hold Alt to drag freely)");
            ui.add_enabled(st.snap, egui::Slider::new(&mut st.strength, 0.0..=1.0).show_value(false))
                .on_hover_text("Snap strength");
            egui::ComboBox::from_id_salt("beat_div").width(56.0).selected_text(div_name(st.div)).show_ui(ui, |ui| {
                for d in [1, 2, 4] {
                    ui.selectable_value(&mut st.div, d, div_name(d));
                }
            });
        }
        ui.separator();
        ui.label("Latency");
        let mut ms = view.latency_ms;
        if ui.add(egui::DragValue::new(&mut ms).range(-300.0..=500.0).speed(1.0).suffix(" ms")).changed() {
            out.latency_ms = Some(ms);
        }
        let cal = st.cal.as_ref().map(|c| c.len());
        let txt = cal.map_or("Calibrate".to_string(), |n| format!("Calibrating {n}/8"));
        if ui
            .button(txt)
            .on_hover_text("Play the song and tap 8 times on the beat: the latency becomes how late your taps land")
            .clicked()
        {
            out.calibrate = true;
        }
    });
}

fn div_name(d: u32) -> &'static str {
    match d {
        2 => "1/8",
        4 => "1/16",
        _ => "1/4",
    }
}

#[allow(clippy::too_many_arguments)]
fn canvas(
    ui: &mut egui::Ui,
    st: &mut AlignState,
    p: &mut Project,
    view: &View,
    pal: &Palette,
    out: &mut Resp,
    undo: &mut dyn FnMut(&Project),
) {
    let dur = view.dur.max(0.1);
    let (mut v0, mut v1) = st.view.unwrap_or((0.0, dur));
    let w = ui.available_width().max(100.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, RULER_H + BAND_H + WAVE_H), Sense::click_and_drag());
    st.canvas = rect;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, pal.bg);
    let x_of = |t: f64, v0: f64, v1: f64| rect.left() + ((t - v0) / (v1 - v0)) as f32 * rect.width();
    let t_of = |x: f32, v0: f64, v1: f64| v0 + ((x - rect.left()) / rect.width()) as f64 * (v1 - v0);

    // zoom (wheel) and scroll (Shift+wheel) around the pointer
    if resp.hovered() {
        let (scroll, zoom, shift, ptr) =
            ui.input(|i| (i.raw_scroll_delta, i.zoom_delta(), i.modifiers.shift, i.pointer.hover_pos()));
        let at = ptr.map_or((v0 + v1) / 2.0, |p| t_of(p.x, v0, v1));
        let mut f = if zoom != 1.0 { 1.0 / zoom as f64 } else { 1.0 };
        if !shift && scroll.y != 0.0 {
            f = (-scroll.y as f64 / 400.0).exp();
        }
        if f != 1.0 {
            let span = ((v1 - v0) * f).clamp(0.5, dur);
            let k = (at - v0) / (v1 - v0);
            v0 = at - k * span;
            v1 = v0 + span;
        }
        let pan = if shift { scroll.y + scroll.x } else { scroll.x };
        if pan != 0.0 {
            let d = -pan as f64 / rect.width() as f64 * (v1 - v0);
            v0 += d;
            v1 += d;
        }
    }
    // follow the playhead while playing
    if let (Some(now), true) = (view.now, view.playing) {
        if now > v1 || now < v0 {
            let span = v1 - v0;
            v0 = now - span * 0.1;
            v1 = v0 + span;
        }
    }
    let span = (v1 - v0).clamp(0.5, dur);
    v0 = v0.clamp(0.0, (dur - span).max(0.0));
    v1 = v0 + span;
    st.view = (span < dur - 1e-6).then_some((v0, v1));

    let ruler = Rect::from_min_size(rect.min, vec2(w, RULER_H));
    let band = Rect::from_min_size(pos2(rect.left(), ruler.bottom()), vec2(w, BAND_H));
    let wave = Rect::from_min_max(pos2(rect.left(), band.bottom()), rect.max);
    painter.rect_filled(ruler, 0.0, pal.header);

    // waveform: one min/max line per pixel column
    if let Some(pk) = view.peaks {
        let per = crate::media::waveform::PEAKS_PER_SEC as f64;
        let mid = wave.center().y;
        let h = wave.height() * 0.46;
        let mut x = wave.left();
        while x < wave.right() {
            let (a, b) = (t_of(x, v0, v1), t_of(x + 1.0, v0, v1));
            let (i0, i1) = ((a * per) as usize, ((b * per).ceil() as usize).max((a * per) as usize + 1).min(pk.len()));
            if i0 < i1 {
                let lo = pk.min[i0..i1].iter().fold(0f32, |m, &v| m.min(v));
                let hi = pk.max[i0..i1].iter().fold(0f32, |m, &v| m.max(v));
                painter
                    .line_segment([pos2(x, mid - hi * h), pos2(x, mid - lo * h + 1.0)], Stroke::new(1.0, pal.waveform));
            }
            x += 1.0;
        }
    } else {
        painter.text(
            wave.center(),
            Align2::CENTER_CENTER,
            "Reading the waveform…",
            FontId::proportional(12.0),
            pal.text_dim,
        );
    }

    // beat ruler: bar numbers on downbeats, ticks on beats (and sub-beats when zoomed in)
    if let Some(per) = p.beats.period().filter(|_| !p.beats.voice) {
        let px_per_beat = rect.width() as f64 / ((v1 - v0) / per);
        let div = if px_per_beat > 60.0 {
            4
        } else if px_per_beat > 24.0 {
            2
        } else {
            1
        };
        let meter = p.beats.meter.max(1) as f64;
        let show_beats = px_per_beat > 5.0;
        let bar_px = px_per_beat * meter;
        let stride = [1i64, 2, 4, 8, 16, 32, 64, 128].into_iter().find(|s| bar_px * *s as f64 >= 30.0).unwrap_or(256);
        for (t, down) in p.beats.grid(v0, v1, div) {
            let x = x_of(t, v0, v1);
            let beat_idx = ((t - p.beats.offset) / per).round();
            let on_beat = ((t - p.beats.offset) / per - beat_idx).abs() < 1e-6;
            if down {
                let bar = (beat_idx / meter).floor() as i64 + 1;
                // zoomed out, only every `stride`th bar gets its number and line
                let lead = (bar - 1).rem_euclid(stride) == 0;
                if lead || bar_px >= 8.0 {
                    painter.line_segment(
                        [pos2(x, ruler.top() + if lead { 0.0 } else { 10.0 }), pos2(x, ruler.bottom())],
                        Stroke::new(if lead { 1.5 } else { 1.0 }, pal.text_dim),
                    );
                }
                if lead {
                    painter.text(
                        pos2(x + 3.0, ruler.top() + 1.0),
                        Align2::LEFT_TOP,
                        bar.to_string(),
                        FontId::proportional(11.0),
                        pal.text,
                    );
                    painter.line_segment(
                        [pos2(x, wave.top()), pos2(x, wave.bottom())],
                        Stroke::new(1.0, pal.text_dim.gamma_multiply(0.45)),
                    );
                }
            } else if on_beat && show_beats {
                painter.line_segment(
                    [pos2(x, ruler.bottom() - 9.0), pos2(x, ruler.bottom())],
                    Stroke::new(1.0, pal.text_dim),
                );
                painter.line_segment(
                    [pos2(x, wave.top()), pos2(x, wave.bottom())],
                    Stroke::new(1.0, pal.text_dim.gamma_multiply(0.18)),
                );
            } else if show_beats {
                painter.line_segment(
                    [pos2(x, ruler.bottom() - 4.0), pos2(x, ruler.bottom())],
                    Stroke::new(1.0, pal.text_dim.gamma_multiply(0.6)),
                );
            }
        }
    } else {
        // no grid: seconds
        let step = nice_step((v1 - v0) / (rect.width() as f64 / 70.0));
        let mut t = (v0 / step).ceil() * step;
        while t <= v1 {
            let x = x_of(t, v0, v1);
            painter
                .line_segment([pos2(x, ruler.bottom() - 6.0), pos2(x, ruler.bottom())], Stroke::new(1.0, pal.text_dim));
            painter.text(
                pos2(x + 3.0, ruler.top() + 1.0),
                Align2::LEFT_TOP,
                clock(t),
                FontId::proportional(11.0),
                pal.text_dim,
            );
            t += step;
        }
    }

    // sections
    for s in &p.beats.sections {
        let (a, b) = (x_of(s.a, v0, v1).max(band.left()), x_of(s.b, v0, v1).min(band.right()));
        if b <= a {
            continue;
        }
        let c = label_color(p, s.label);
        let r = Rect::from_min_max(pos2(a, band.top() + 1.0), pos2(b, band.bottom() - 1.0));
        painter.rect_filled(r, 3.0, c.gamma_multiply(if st.sel_section == Some(s.id) { 0.95 } else { 0.6 }));
        painter.rect_filled(
            Rect::from_min_max(pos2(a, wave.top()), pos2(b, wave.bottom())),
            0.0,
            c.gamma_multiply(0.07),
        );
        let galley = painter.layout_no_wrap(s.name.clone(), FontId::proportional(11.0), Color32::WHITE);
        if galley.size().x < r.width() - 4.0 {
            painter.galley(pos2(r.left() + 4.0, r.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
        }
    }
    // the swept range
    if let Some((a, b)) = st.range {
        let r = Rect::from_min_max(pos2(x_of(a, v0, v1), band.top()), pos2(x_of(b, v0, v1), wave.bottom()));
        painter.rect_filled(r.intersect(rect), 0.0, pal.selection.gamma_multiply(0.25));
    }
    // marks
    for m in &p.beats.marks {
        if m.t < v0 || m.t > v1 {
            continue;
        }
        let x = x_of(m.t, v0, v1);
        let sel = st.sel.contains(&m.id);
        let c = if sel { pal.selection } else { label_color(p, m.kind.label()) };
        let wdt = if sel || m.kind == MarkKind::Tap { 2.0 } else { 1.0 };
        painter.line_segment([pos2(x, wave.top()), pos2(x, wave.bottom())], Stroke::new(wdt, c));
        let tri = vec![pos2(x - 4.0, wave.top()), pos2(x + 4.0, wave.top()), pos2(x, wave.top() + 6.0)];
        painter.add(egui::Shape::convex_polygon(tri, c, Stroke::NONE));
    }
    // playhead
    if let Some(now) = view.now.filter(|t| *t >= v0 && *t <= v1) {
        let x = x_of(now, v0, v1);
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, pal.playhead));
    }
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);

    // ---- pointer ----
    let alt = ui.input(|i| i.modifiers.alt);
    let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
    let hit_mark = |marks: &[BeatMark], x: f32| {
        marks
            .iter()
            .filter(|m| (x_of(m.t, v0, v1) - x).abs() <= HIT_PX)
            .min_by(|a, b| (x_of(a.t, v0, v1) - x).abs().total_cmp(&(x_of(b.t, v0, v1) - x).abs()))
            .map(|m| m.id)
    };
    if resp.drag_started() {
        // where the button went down: by now the pointer has moved past the drag threshold
        if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
            let t = t_of(pos.x, v0, v1);
            match hit_mark(&p.beats.marks, pos.x).filter(|_| pos.y > band.bottom()) {
                Some(id) => {
                    if !st.sel.contains(&id) {
                        st.sel = vec![id];
                    }
                    undo(p);
                    st.drag = Some(Drag::Marks { from: t, before: p.beats.marks.clone() });
                }
                None => {
                    st.drag = Some(Drag::Range { from: t.clamp(0.0, dur) });
                    st.range = None;
                }
            }
        }
    }
    if let (Some(drag), Some(pos)) = (&st.drag, resp.interact_pointer_pos()) {
        let t = t_of(pos.x, v0, v1).clamp(0.0, dur);
        match drag {
            Drag::Marks { from, before } => {
                p.beats.marks = before.clone();
                let snap = (st.snap && !alt && p.beats.bpm > 0.0).then_some(st.div);
                p.beats.move_marks(&st.sel, t - from, snap);
                out.edited = true;
            }
            Drag::Range { from } => {
                st.range = Some((from.min(t), from.max(t)));
                let (a, b) = (from.min(t), from.max(t));
                st.sel = p.beats.marks.iter().filter(|m| m.t >= a && m.t <= b).map(|m| m.id).collect();
            }
        }
    }
    if resp.drag_stopped() {
        st.drag = None;
    }
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if band.contains(pos) {
                let t = t_of(pos.x, v0, v1);
                st.sel_section = p.beats.sections.iter().find(|s| t >= s.a && t <= s.b).map(|s| s.id);
                if let Some(s) = p.beats.sections.iter().find(|s| Some(s.id) == st.sel_section) {
                    st.section_name = s.name.clone();
                }
            } else if let Some(id) = hit_mark(&p.beats.marks, pos.x) {
                if ctrl || shift {
                    if let Some(i) = st.sel.iter().position(|x| *x == id) {
                        st.sel.remove(i);
                    } else {
                        st.sel.push(id);
                    }
                } else {
                    st.sel = vec![id];
                }
            } else {
                st.sel.clear();
                st.range = None;
                let t = t_of(pos.x, v0, v1);
                out.seek = Some(p.beat_map().to_tl(t));
            }
        }
    }
    menu_on(&resp, st, p, out, undo);
    // Delete / Backspace remove the selected marks; Ctrl+A selects them all
    if resp.hovered() || resp.has_focus() {
        let (del, all) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.modifiers.command && i.key_pressed(egui::Key::A),
            )
        });
        if del && !st.sel.is_empty() && !ui.ctx().wants_keyboard_input() {
            undo(p);
            p.beats.delete_marks(&st.sel);
            st.sel.clear();
            out.edited = true;
        }
        if all {
            st.sel = p.beats.marks.iter().map(|m| m.id).collect();
        }
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} marks · {} selected{}",
                p.beats.marks.len(),
                st.sel.len(),
                if p.beats.bpm > 0.0 && !p.beats.voice { format!(" · {:.2} BPM", p.beats.bpm) } else { String::new() }
            ))
            .color(pal.text_dim)
            .small(),
        );
    });
}

fn menu_on(
    resp: &egui::Response,
    st: &mut AlignState,
    p: &mut Project,
    out: &mut Resp,
    undo: &mut dyn FnMut(&Project),
) {
    crate::ui::menu::context(resp, |ui| {
        if ui.add_enabled(!st.sel.is_empty(), egui::Button::new("Delete selected marks")).clicked() {
            undo(p);
            p.beats.delete_marks(&st.sel);
            st.sel.clear();
            out.edited = true;
            ui.close();
        }
        if ui.add_enabled(st.range.is_some(), egui::Button::new("Make a section of the range")).clicked() {
            if let Some((a, b)) = st.range.take() {
                undo(p);
                let name =
                    if st.section_name.trim().is_empty() { "Section".to_string() } else { st.section_name.clone() };
                st.sel_section = Some(p.beats.add_section(a, b, &name));
                out.edited = true;
            }
            ui.close();
        }
        if ui.add_enabled(!p.beats.marks.is_empty(), egui::Button::new("Clear all marks")).clicked() {
            undo(p);
            p.beats.marks.clear();
            st.sel.clear();
            out.edited = true;
            ui.close();
        }
    });
}

fn sections(ui: &mut egui::Ui, st: &mut AlignState, p: &mut Project, out: &mut Resp, undo: &mut dyn FnMut(&Project)) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Section");
        let range = st.range.or_else(|| {
            let ts: Vec<f64> = p.beats.marks.iter().filter(|m| st.sel.contains(&m.id)).map(|m| m.t).collect();
            (ts.len() >= 2).then(|| (ts[0], ts[ts.len() - 1]))
        });
        let tip = "Drag across the waveform (or select two marks), then pick a name";
        for (name, label) in SECTION_PRESETS {
            let [r, g, b] = p.label_color(label).unwrap_or([150, 150, 150]);
            let txt = egui::RichText::new(name).color(Color32::from_rgb(r, g, b));
            if ui
                .add_enabled(range.is_some(), egui::Button::new(txt))
                .on_hover_text(tip)
                .on_disabled_hover_text(tip)
                .clicked()
            {
                if let Some((a, b)) = range {
                    undo(p);
                    st.sel_section = Some(p.beats.add_section(a, b, name));
                    st.range = None;
                    out.edited = true;
                }
            }
        }
        ui.add(egui::TextEdit::singleline(&mut st.section_name).hint_text("Any name").desired_width(110.0));
        if ui.add_enabled(range.is_some() && !st.section_name.trim().is_empty(), egui::Button::new("Add")).clicked() {
            if let Some((a, b)) = range {
                undo(p);
                st.sel_section = Some(p.beats.add_section(a, b, &st.section_name));
                st.range = None;
                out.edited = true;
            }
        }
    });
    if p.beats.sections.is_empty() {
        return;
    }
    // the list: one aligned row per section (they keep time order, so a drag does not reorder them)
    const CTRLS: &[crate::ui::list_rows::Ctrl] =
        &[crate::ui::list_rows::Ctrl { glyph: "✕", hint: "Delete the section" }];
    let mut rows = crate::ui::list_rows::Rows::new(ui, "beat_sections", CTRLS, &[96.0], false);
    let list = p.beats.sections.clone();
    for (i, s) in list.iter().enumerate() {
        let range = format!("{} – {}", clock(s.a), clock(s.b));
        let label = if s.name.is_empty() { "Section" } else { s.name.as_str() };
        let r = rows.row(
            ui,
            i,
            &crate::ui::list_rows::RowSpec { key: s.id, label, depth: 0, open: None, dim: false, values: &[&range] },
        );
        let [cr, cg, cb] = p.label_color(s.label).unwrap_or([150, 150, 150]);
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(r.label.left() - 6.0, r.label.top() + 5.0), vec2(3.0, 12.0)),
            1.0,
            Color32::from_rgb(cr, cg, cb),
        );
        if r.ctrl == Some(0) {
            undo(p);
            p.beats.delete_section(s.id);
            out.edited = true;
        }
        if ui.rect_contains_pointer(r.label) && ui.input(|i| i.pointer.primary_clicked()) {
            st.sel_section = Some(s.id);
            st.section_name = s.name.clone();
        }
    }
    let _ = rows.finish(ui); // time order is the order
                             // the selected section's name and colour (two aligned columns)
    let Some(id) = st.sel_section.filter(|id| p.beats.sections.iter().any(|s| s.id == *id)) else { return };
    let labels: Vec<(String, [u8; 3])> = p.labels.iter().map(|l| (l.name.clone(), l.color)).collect();
    egui::Grid::new("beat_section_form").num_columns(2).show(ui, |ui| {
        let s = p.beats.sections.iter().find(|s| s.id == id).cloned().unwrap_or_default();
        ui.label("Name");
        let mut name = s.name.clone();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(180.0));
        if r.gained_focus() {
            undo(p);
        }
        if r.changed() {
            if let Some(x) = p.beats.section_mut(id) {
                x.name = name;
            }
            out.edited = true;
        }
        ui.end_row();
        ui.label("Colour");
        let cur = labels.get((s.label as usize).wrapping_sub(1)).map_or("None".to_string(), |l| l.0.clone());
        egui::ComboBox::from_id_salt("beat_section_label").selected_text(cur).show_ui(ui, |ui| {
            for (i, (n, [r, g, b])) in labels.iter().enumerate() {
                if ui
                    .selectable_label(
                        s.label as usize == i + 1,
                        egui::RichText::new(n).color(Color32::from_rgb(*r, *g, *b)),
                    )
                    .clicked()
                {
                    undo(p);
                    if let Some(x) = p.beats.section_mut(id) {
                        x.label = i as u8 + 1;
                    }
                    out.edited = true;
                }
            }
        });
        ui.end_row();
    });
}

fn use_row(ui: &mut egui::Ui, st: &mut AlignState, p: &mut Project, view: &View, out: &mut Resp) {
    let has = !p.beats.marks.is_empty();
    egui::Grid::new("beat_use").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Use every");
        ui.horizontal(|ui| {
            let unit = if st.every == 1 { " mark" } else { " marks" };
            ui.add(egui::DragValue::new(&mut st.every).range(1..=64).suffix(unit));
            if ui.add_enabled(has || !p.beats.sections.is_empty(), egui::Button::new("To timeline markers")).clicked() {
                out.to_markers = true;
            }
        });
        ui.end_row();
        ui.label("Cut");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut st.cut_all_video, true, "All video");
            ui.selectable_value(&mut st.cut_all_video, false, "Selected track");
            if ui.add_enabled(has, egui::Button::new("Cut at marks")).clicked() {
                out.cut = true;
            }
        });
        ui.end_row();
        ui.label("Sound");
        ui.horizontal(|ui| {
            let cur = view.sounds.iter().find(|s| s.0 == st.sfx).map_or("Choose a sound", |s| s.1.as_str()).to_string();
            egui::ComboBox::from_id_salt("beat_sfx").selected_text(cur).width(170.0).show_ui(ui, |ui| {
                for (id, name) in view.sounds {
                    ui.selectable_value(&mut st.sfx, *id, name);
                }
            });
            if ui.add_enabled(has && st.sfx != 0, egui::Button::new("Add at marks")).clicked() {
                out.sfx = true;
            }
        });
        ui.end_row();
        ui.label("Effect");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("beat_block").selected_text(st.block.clone()).width(170.0).show_ui(ui, |ui| {
                for b in view.blocks {
                    ui.selectable_value(&mut st.block, b.clone(), b);
                }
            });
            if ui
                .add_enabled(has, egui::Button::new("Apply on marks"))
                .on_hover_text("On the selected clips, or every video clip under a mark")
                .clicked()
            {
                out.effect = true;
            }
        });
        ui.end_row();
        ui.label("Clips");
        if ui.add_enabled(has, egui::Button::new("Snap selected clips to marks")).clicked() {
            out.snap_clips = true;
        }
        ui.end_row();
        ui.label("Markers file");
        ui.horizontal(|ui| {
            for (txt, f) in
                [("Export CSV", BeatFmt::Csv), ("Export EDL", BeatFmt::Edl), ("Export chapters", BeatFmt::Chapters)]
            {
                if ui.button(txt).clicked() {
                    out.export = Some(f);
                }
            }
            if ui.button("Import…").clicked() {
                out.import = true;
            }
        });
        ui.end_row();
    });
}

fn nice_step(raw: f64) -> f64 {
    [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0]
        .into_iter()
        .find(|s| *s >= raw)
        .unwrap_or(600.0)
}

fn clock(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0).floor() as u64, t % 60.0)
}
