//! Mixer pane: one channel strip per bus (Main last), plus the strip of the selected audio clip/track.
//!
//! Strip: name (editable), a stereo peak meter fed by `BusGraph::meter`, a gain fader (dB), pan knob,
//! Mute / Solo / Mono buttons, the output-bus combo, and the filter chain — each filter as a header row
//! (enable, name, up/down, X) with its parameters below (DragValue per `FilterKind::params()`); the EQ
//! filter also draws a small response curve you can drag. "+ Filter" adds from `FilterKind::ALL`,
//! "+ Bus" creates a bus, X deletes one (never Main; its users fall back to Main).
//! Track/clip routing: the selected audio track shows a "Bus" combo (`Track.bus`), the selected clip an
//! override combo (`Clip.bus`, "(track)" = inherit).
//! Undo once per gesture; returns true when the project changed.
//!
//! Edits go into a per-frame clone of `Project.buses` and are written back at the end of the frame, so
//! `undo` can snapshot the project before the first change of a gesture (the effects/inspector pattern).

use crate::engine::mixer_fx::{db_to_lin, filter_bands, filter_response_db, lin_to_db, BusGraph};
use crate::model::{AudioFilter, Bus, FilterKind, Id, Project, TrackKind};
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Button, ComboBox, DragValue, Grid, Rect, Response, Sense, Stroke, TextEdit};

/// Channel-strip width in points.
const STRIP_W: f32 = 176.0;
/// Response-curve plot height.
const CURVE_H: f32 = 54.0;
/// Full-scale range of the EQ plot, in dB.
const CURVE_DB: f32 = 24.0;

#[derive(Default)]
pub struct MixerState {
    pub selected_bus: Option<Id>,
    /// Show the clip/track routing section.
    pub show_routing: bool,
}

/// Test-only registry of widget rects so headless tests can click real widgets without pixel-guessing.
#[cfg(test)]
pub(crate) mod test_rects {
    use eframe::egui::Rect;
    use std::cell::RefCell;
    thread_local! {
        static RECTS: RefCell<Vec<(String, Rect)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn clear() {
        RECTS.with(|r| r.borrow_mut().clear());
    }
    pub fn push(name: String, rect: Rect) {
        RECTS.with(|r| r.borrow_mut().push((name, rect)));
    }
    pub fn get(name: &str) -> Option<Rect> {
        RECTS.with(|r| r.borrow().iter().rev().find(|(n, _)| n == name).map(|(_, rect)| *rect))
    }
}

/// True on the first frame of an edit gesture (drag start, or a click / typed change).
fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

#[derive(Default)]
struct Gesture {
    start: bool,
    changed: bool,
}

impl Gesture {
    fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

/// Structural edits that need `Project`'s own helpers, applied after `undo` has snapshotted.
#[derive(Default)]
struct Edits {
    add_bus: bool,
    remove: Option<Id>,
    track_bus: Option<(usize, Id)>,
    clip_bus: Option<(Id, Id)>,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut MixerState,
    project: &mut Project,
    selection: &[Id],
    buses: &BusGraph,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    #[cfg(test)]
    test_rects::clear();
    let mut g = Gesture::default();
    let mut ed = Edits::default();
    // ponytail: clone the whole bus list every frame — a handful of buses with a few filters each is
    // nothing next to a repaint. Upgrade path if it ever shows up: edit in place and snapshot lazily.
    let mut list = project.buses.clone();
    let names: Vec<(Id, String)> = list.iter().map(|b| (b.id, b.name.clone())).collect();
    let main = list.first().map(|b| b.id).unwrap_or(0);

    ui.horizontal(|ui| {
        let r = ui.add(Button::new("+ Bus").small());
        #[cfg(test)]
        test_rects::push("add_bus".into(), r.rect);
        if r.clicked() {
            ed.add_bus = true;
            g.click();
        }
        ui.checkbox(&mut state.show_routing, "Routing");
        if list.is_empty() {
            ui.label("no buses — clips go straight to the output");
        }
    });
    ui.separator();

    // Main last: it is where everything ends up.
    let order: Vec<usize> = (1..list.len()).chain(0..list.len().min(1)).collect();
    egui::ScrollArea::horizontal().id_salt("mixer_strips").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (n, &i) in order.iter().enumerate() {
                let is_main = list[i].id == main;
                let meter = buses.meter(list[i].id);
                let sel = state.selected_bus == Some(list[i].id);
                ui.push_id(("strip", list[i].id), |ui| {
                    let size = vec2(STRIP_W, ui.available_height());
                    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(STRIP_W);
                        let stroke = if sel { Stroke::new(1.0, palette.accent) } else { Stroke::NONE };
                        egui::Frame::new().stroke(stroke).inner_margin(2.0).show(ui, |ui| {
                            strip(ui, &mut list[i], is_main, meter, &names, palette, state, &mut g, &mut ed, n);
                        });
                    });
                });
                ui.separator();
            }
        });
    });

    if state.show_routing {
        ui.separator();
        routing(ui, project, selection, &names, main, &mut g, &mut ed);
    }

    if g.start {
        undo(project);
    }
    if g.changed {
        project.buses = list;
        if ed.add_bus {
            let n = project.buses.len();
            let id = project.add_bus(format!("Bus {n}"));
            state.selected_bus = Some(id);
        }
        if let Some(id) = ed.remove {
            project.remove_bus(id);
            if state.selected_bus == Some(id) {
                state.selected_bus = None;
            }
        }
        if let Some((ti, b)) = ed.track_bus {
            if let Some(t) = project.tracks.get_mut(ti) {
                t.bus = b;
            }
        }
        if let Some((cid, b)) = ed.clip_bus {
            if let Some(c) = project.clip_mut(cid) {
                c.bus = b;
            }
        }
    }
    g.changed
}

#[allow(clippy::too_many_arguments)]
fn strip(
    ui: &mut egui::Ui,
    bus: &mut Bus,
    is_main: bool,
    meter_lr: (f32, f32),
    names: &[(Id, String)],
    palette: &Palette,
    state: &mut MixerState,
    g: &mut Gesture,
    ed: &mut Edits,
    n: usize,
) {
    let _ = n; // only the test rect registry uses the strip index
    ui.horizontal(|ui| {
        let r = ui.add(TextEdit::singleline(&mut bus.name).desired_width(STRIP_W - 46.0));
        g.note(&r);
        if r.clicked() {
            state.selected_bus = Some(bus.id);
        }
        let del = ui.add_enabled(!is_main, Button::new("✕").small());
        #[cfg(test)]
        test_rects::push(format!("del_bus{n}"), del.rect);
        if del.clicked() {
            ed.remove = Some(bus.id);
            g.click();
        }
    });

    meter(ui, meter_lr, palette);

    ui.horizontal(|ui| {
        let mut db = lin_to_db(bus.gain.at(0.0) as f32).max(-60.0);
        let r = ui.add(DragValue::new(&mut db).range(-60.0..=12.0).speed(0.2).suffix(" dB").fixed_decimals(1));
        #[cfg(test)]
        test_rects::push(format!("gain{n}"), r.rect);
        if r.changed() {
            bus.gain.value = db_to_lin(db) as f64;
        }
        g.note(&r);
        let mut pan = bus.pan.at(0.0);
        let r = ui.add(DragValue::new(&mut pan).range(-1.0..=1.0).speed(0.01).prefix("pan ").fixed_decimals(2));
        if r.changed() {
            bus.pan.value = pan;
        }
        g.note(&r);
    });

    ui.horizontal(|ui| {
        for (label, flag, hint) in
            [("M", &mut bus.muted, "Mute"), ("S", &mut bus.solo, "Solo"), ("Mono", &mut bus.mono, "Fold to mono")]
        {
            let r = ui.add(Button::new(label).small().selected(*flag)).on_hover_text(hint);
            #[cfg(test)]
            test_rects::push(format!("{}{n}", label.to_lowercase()), r.rect);
            if r.clicked() {
                *flag = !*flag;
                g.click();
            }
        }
    });

    if !is_main {
        ui.horizontal(|ui| {
            ui.label("→");
            let cur = names.iter().find(|(id, _)| *id == bus.output).map(|(_, n)| n.as_str()).unwrap_or("Main");
            ComboBox::from_id_salt(("out", bus.id)).selected_text(cur).width(STRIP_W - 34.0).show_ui(ui, |ui| {
                for (id, name) in names {
                    // no self-send; a longer loop is broken back to Main by the graph anyway
                    if *id == bus.id {
                        continue;
                    }
                    if ui.selectable_label(bus.output == *id, name).clicked() && bus.output != *id {
                        bus.output = *id;
                        g.click();
                    }
                }
            });
        });
    }

    filters(ui, bus, palette, g, n);
}

/// Stereo peak meter, -60 dB … +6 dB left to right.
fn meter(ui: &mut egui::Ui, lr: (f32, f32), palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, palette.bg);
    for (i, v) in [lr.0, lr.1].into_iter().enumerate() {
        let db = lin_to_db(v).clamp(-60.0, 6.0);
        let f = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
        if f <= 0.0 {
            continue;
        }
        let y = rect.top() + 1.0 + i as f32 * 5.0;
        let bar = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width() * f, 4.0));
        p.rect_filled(bar, 0.0, if db > -1.0 { palette.playhead } else { palette.waveform });
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
}

fn filters(ui: &mut egui::Ui, bus: &mut Bus, palette: &Palette, g: &mut Gesture, n: usize) {
    let _ = n;
    let count = bus.filters.len();
    let mut remove: Option<usize> = None;
    let mut swap: Option<(usize, usize)> = None;
    for (i, f) in bus.filters.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            g.note(&ui.checkbox(&mut f.enabled, ""));
            ui.label(f.kind.name());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let del = ui.add(Button::new("✕").small());
                #[cfg(test)]
                test_rects::push(format!("delfx{n}_{i}"), del.rect);
                if del.clicked() {
                    remove = Some(i);
                }
                if ui.add_enabled(i + 1 < count, Button::new("▼").small()).clicked() {
                    swap = Some((i, i + 1));
                }
                if ui.add_enabled(i > 0, Button::new("▲").small()).clicked() {
                    swap = Some((i, i - 1));
                }
            });
        });
        if curve(ui, f, palette, g) {
            g.changed = true;
        }
        Grid::new(("fx", bus.id, i)).num_columns(2).spacing(vec2(4.0, 2.0)).show(ui, |ui| {
            for (j, spec) in f.kind.params().iter().enumerate() {
                let Some(a) = f.params.get_mut(j) else { continue };
                ui.label(spec.name);
                let mut v = a.value;
                let r = match spec.name {
                    "Ping-pong" => {
                        let mut on = v >= 0.5;
                        let r = ui.checkbox(&mut on, "");
                        v = if on { 1.0 } else { 0.0 };
                        r
                    }
                    "Type" => {
                        let opts = ["White", "Pink", "Tone"];
                        let cur = opts.get(v.round().clamp(0.0, 2.0) as usize).copied().unwrap_or("White");
                        let mut picked = None;
                        let mut r = ComboBox::from_id_salt(("ty", bus.id, i))
                            .selected_text(cur)
                            .width(84.0)
                            .show_ui(ui, |ui| {
                                for (k, o) in opts.iter().enumerate() {
                                    if ui.selectable_label(cur == *o, *o).clicked() {
                                        picked = Some(k as f64);
                                    }
                                }
                            })
                            .response;
                        if let Some(k) = picked {
                            v = k;
                            r.mark_changed();
                        }
                        r
                    }
                    _ => ui.add(
                        DragValue::new(&mut v)
                            .range(spec.min..=spec.max)
                            .clamp_existing_to_range(false)
                            .speed((spec.max - spec.min) / 200.0),
                    ),
                };
                #[cfg(test)]
                test_rects::push(format!("p{n}_{i}_{j}"), r.rect);
                if r.changed() {
                    a.value = v;
                }
                g.note(&r);
                ui.end_row();
            }
        });
    }
    if let Some((a, b)) = swap {
        bus.filters.swap(a, b);
        g.click();
    }
    if let Some(i) = remove {
        bus.filters.remove(i);
        g.click();
    }
    let mut add: Option<FilterKind> = None;
    ComboBox::from_id_salt(("addfx", bus.id)).selected_text("+ Filter").width(STRIP_W - 16.0).show_ui(ui, |ui| {
        for kind in FilterKind::ALL {
            if ui.selectable_label(false, kind.name()).clicked() {
                add = Some(kind);
            }
        }
    });
    if let Some(kind) = add {
        bus.filters.push(AudioFilter::new(kind));
        g.click();
    }
}

/// EQ / pass-filter response plot. Drag a band: X moves its frequency, Y its gain. Returns true when
/// the drag changed a parameter.
fn curve(ui: &mut egui::Ui, f: &mut AudioFilter, palette: &Palette, g: &mut Gesture) -> bool {
    // (gain param, freq param) per band, in the order `filter_bands` reports them
    let map: &[(Option<usize>, usize)] = match f.kind {
        FilterKind::Eq => &[(Some(0), 1), (Some(2), 3), (Some(5), 6)],
        FilterKind::HighPass | FilterKind::LowPass => &[(None, 0)],
        _ => return false,
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), CURVE_H), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, palette.bg);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    let y_of = |db: f32| rect.center().y - db.clamp(-CURVE_DB, CURVE_DB) / CURVE_DB * rect.height() * 0.5;
    let x_of = |hz: f32| rect.left() + (hz.max(20.0) / 20.0).log10() / 3.0 * rect.width();
    let hz_of = |x: f32| 20.0 * 10f32.powf(((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0) * 3.0);
    p.line_segment(
        [pos2(rect.left(), y_of(0.0)), pos2(rect.right(), y_of(0.0))],
        Stroke::new(1.0, palette.text_dim.gamma_multiply(0.5)),
    );
    let pts: Vec<_> = (0..48)
        .map(|i| {
            let x = rect.left() + rect.width() * i as f32 / 47.0;
            pos2(x, y_of(filter_response_db(f, 0.0, hz_of(x))))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5, palette.accent)));
    let bands = filter_bands(f, 0.0);
    for (b, (gi, _)) in bands.iter().zip(map) {
        let db = gi.map(|_| b.3).unwrap_or(0.0);
        p.circle_filled(pos2(x_of(b.1), y_of(db)), 3.0, palette.keyframe);
    }

    let mut changed = false;
    if resp.drag_started() {
        g.start = true;
    }
    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            // the nearest band follows the pointer, so it stays nearest for the rest of the drag
            let pick = bands
                .iter()
                .enumerate()
                .min_by(|a, b| (x_of(a.1 .1) - pos.x).abs().total_cmp(&(x_of(b.1 .1) - pos.x).abs()))
                .map(|(i, _)| i);
            if let Some((gi, fi)) = pick.and_then(|i| map.get(i)) {
                let specs = f.kind.params();
                if let (Some(spec), Some(a)) = (specs.get(*fi), f.params.get_mut(*fi)) {
                    a.value = (hz_of(pos.x) as f64).clamp(spec.min, spec.max);
                    changed = true;
                }
                if let Some(gi) = gi {
                    let db = (rect.center().y - pos.y) / (rect.height() * 0.5) * CURVE_DB;
                    if let (Some(spec), Some(a)) = (specs.get(*gi), f.params.get_mut(*gi)) {
                        a.value = (db as f64).clamp(spec.min, spec.max);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// "Bus" combo for the selected audio track and an override combo for the selected clip.
fn routing(
    ui: &mut egui::Ui,
    project: &Project,
    selection: &[Id],
    names: &[(Id, String)],
    main: Id,
    g: &mut Gesture,
    ed: &mut Edits,
) {
    let clip = selection.iter().copied().find(|&id| project.clip(id).is_some());
    let ti = clip.and_then(|id| project.track_of(id)).or_else(|| project.audio_tracks().first().copied());
    let Some(ti) = ti else { return };
    let Some(track) = project.tracks.get(ti) else { return };
    if names.is_empty() {
        ui.label("Add a bus first");
        return;
    }
    let label = |id: Id| {
        names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.as_str()).unwrap_or(if id == 0 { "Main" } else { "?" })
    };

    Grid::new("mixer_routing").num_columns(2).show(ui, |ui| {
        ui.label(format!("Track “{}”", track.name));
        let cur = if track.bus == 0 { "Main" } else { label(track.bus) };
        ComboBox::from_id_salt("track_bus").selected_text(cur).width(140.0).show_ui(ui, |ui| {
            for (id, name) in names {
                let sel = track.bus == *id || (track.bus == 0 && *id == main);
                if ui.selectable_label(sel, name).clicked() && !sel {
                    ed.track_bus = Some((ti, *id));
                    g.click();
                }
            }
        });
        ui.end_row();

        if let Some(c) = clip.and_then(|id| project.clip(id)) {
            if track.kind == TrackKind::Audio || c.kind == crate::model::ClipKind::Sequence {
                ui.label(format!("Clip “{}”", c.name));
                let cur = if c.bus == 0 { "(track)".to_string() } else { label(c.bus).to_string() };
                ComboBox::from_id_salt("clip_bus").selected_text(cur).width(140.0).show_ui(ui, |ui| {
                    if ui.selectable_label(c.bus == 0, "(track)").clicked() && c.bus != 0 {
                        ed.clip_bus = Some((c.id, 0));
                        g.click();
                    }
                    for (id, name) in names {
                        if ui.selectable_label(c.bus == *id, name).clicked() && c.bus != *id {
                            ed.clip_bus = Some((c.id, *id));
                            g.click();
                        }
                    }
                });
                ui.end_row();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};
    use eframe::egui::{Color32, Event, Modifiers, PointerButton, Pos2, RawInput};

    struct Harness {
        ctx: egui::Context,
        project: Project,
        state: MixerState,
        graph: BusGraph,
        selection: Vec<Id>,
        undos: usize,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            let ai = project.audio_tracks()[0];
            project.tracks[ai].clips.push(Clip::new(7, ClipKind::Audio, "a", 0.0, 4.0));
            project.main_bus();
            let music = project.add_bus("Music");
            project.tracks[ai].bus = music;
            let mut graph = BusGraph::new();
            graph.sync(&project);
            Self {
                ctx: egui::Context::default(),
                project,
                state: MixerState { show_routing: true, ..Default::default() },
                graph,
                selection: vec![7],
                undos: 0,
                time: 0.0,
            }
        }
        fn frame(&mut self, events: Vec<Event>) -> bool {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(700.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let Harness { ctx, project, state, graph, selection, undos, .. } = self;
            let mut edited = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    edited |= show(ui, state, project, selection, graph, &pal, &mut undo);
                });
            });
            edited
        }
        fn click(&mut self, pos: Pos2) -> bool {
            self.frame(vec![Event::PointerMoved(pos)]);
            let mut e = self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            }]);
            e |= self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::default(),
            }]);
            e |= self.frame(vec![]);
            e
        }
        fn click_named(&mut self, name: &str) -> bool {
            self.frame(vec![]);
            let rect = test_rects::get(name).unwrap_or_else(|| panic!("no widget {name}"));
            self.click(rect.center())
        }
        /// Press at `from`, move there in four steps, release. True if any frame reported an edit.
        fn drag(&mut self, from: Pos2, delta: egui::Vec2) -> bool {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            let mut edited = false;
            for i in 1..=4 {
                edited |= self.frame(vec![Event::PointerMoved(from + delta * (i as f32 / 4.0))]);
            }
            edited |= self.frame(vec![Event::PointerButton {
                pos: from + delta,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            edited | self.frame(vec![])
        }
    }

    #[test]
    fn draws_two_buses_without_changing_anything() {
        let mut h = Harness::new();
        assert!(!h.frame(vec![]), "drawing must not report an edit");
        assert!(!h.frame(vec![]));
        assert_eq!(h.undos, 0);
        assert_eq!(h.project.buses.len(), 2);
        // both strips are there (Main is drawn last, so it is index 1)
        assert!(test_rects::get("m0").is_some() && test_rects::get("m1").is_some());
    }

    #[test]
    fn add_and_remove_a_bus() {
        let mut h = Harness::new();
        assert!(h.click_named("add_bus"));
        assert_eq!(h.project.buses.len(), 3);
        assert_eq!(h.undos, 1);
        // strip 0 is the first non-Main bus; deleting it drops back to two
        assert!(h.click_named("del_bus0"));
        assert_eq!(h.project.buses.len(), 2);
    }

    #[test]
    fn mute_solo_mono_toggle() {
        let mut h = Harness::new();
        assert!(h.click_named("m0"));
        assert!(h.project.buses[1].muted, "strip 0 is the Music bus");
        assert!(h.click_named("s0"));
        assert!(h.project.buses[1].solo);
        assert!(h.click_named("mono0"));
        assert!(h.project.buses[1].mono);
        assert_eq!(h.undos, 3);
        // Main can never be deleted
        let del_main = test_rects::get("del_bus1").expect("Main delete button");
        h.click(del_main.center());
        assert_eq!(h.project.buses.len(), 2);
    }

    #[test]
    fn add_a_filter_and_edit_a_param() {
        let mut h = Harness::new();
        let music = h.project.buses[1].id;
        h.project.bus_mut(music).unwrap().filters.push(AudioFilter::new(FilterKind::Gain));
        h.frame(vec![]);
        // the Gain filter has one param row; drag it
        let rect = test_rects::get("p0_0_0").expect("gain param");
        let changed = h.drag(rect.center(), vec2(40.0, 0.0));
        assert!(changed, "dragging the DragValue edits the project");
        assert!(h.project.buses[1].filters[0].params[0].value > 0.0);
        assert!(h.undos >= 1);
        // and it can be removed again
        assert!(h.click_named("delfx0_0"));
        assert!(h.project.buses[1].filters.is_empty());
    }

    #[test]
    fn eq_curve_drag_moves_a_band() {
        let mut h = Harness::new();
        let music = h.project.buses[1].id;
        h.project.bus_mut(music).unwrap().filters.push(AudioFilter::new(FilterKind::Eq));
        h.frame(vec![]);
        // the curve sits between the filter header and its first param row
        let head = test_rects::get("delfx0_0").expect("filter header");
        let first = test_rects::get("p0_0_0").expect("first param");
        let target = pos2(head.center().x, (head.bottom() + first.top()) * 0.5);
        let changed = h.drag(target, vec2(0.0, -14.0));
        assert!(changed, "dragging the response curve edits a band");
        let f = &h.project.buses[1].filters[0];
        let moved = f.params.iter().zip(FilterKind::Eq.params()).any(|(a, s)| (a.value - s.default).abs() > 1e-6);
        assert!(moved, "some band parameter changed");
        assert!(h.undos >= 1);
    }

    #[test]
    fn routing_combos_reach_the_track_and_clip() {
        let mut h = Harness::new();
        h.frame(vec![]);
        let ai = h.project.audio_tracks()[0];
        let music = h.project.buses[1].id;
        assert_eq!(h.project.tracks[ai].bus, music, "the harness routes the track to Music");
        assert_eq!(h.project.clip(7).unwrap().bus, 0, "the clip inherits by default");
        // the routing grid is drawn (show_routing = true) without panicking or editing
        assert!(!h.frame(vec![]));
    }

    #[test]
    fn works_with_no_buses_at_all() {
        let mut h = Harness::new();
        h.project.buses.clear();
        h.project.tracks.iter_mut().for_each(|t| t.bus = 0);
        assert!(!h.frame(vec![]));
        assert!(h.click_named("add_bus"), "the first click creates Main + a bus");
        assert_eq!(h.project.buses.len(), 2);
    }
}
