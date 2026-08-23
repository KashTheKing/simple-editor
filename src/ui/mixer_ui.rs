//! Mixer pane: the routing list (which track feeds which bus) on top, one channel strip per bus below
//! (Main last). Everything in a strip scrolls vertically, so a long filter chain stays reachable.
//!
//! Strip: name (editable), what feeds it, a stereo peak meter fed by `BusGraph::meter`, a gain fader
//! (dB), pan knob, Mute (speaker glyph) / Solo / Mono, the output-bus combo, and the filter chain —
//! each filter as a header row (enable, name, up/down, X) with its parameters below (DragValue per
//! `FilterKind::params()` + a diamond keyframe toggle); EQ / pass filters also draw a response plot over a
//! labelled log-frequency grid with one draggable handle per band. "+ Filter" adds from
//! `FilterKind::ALL`, "+ Bus" creates a bus, X deletes one (never Main; its users fall back to Main).
//! Routing: every audio track is a row — click it to send it to the selected bus, or pick from its own
//! combo (`Track.bus`); the selected clip gets an override combo (`Clip.bus`, "(track)" = inherit).
//! Undo once per gesture; returns true when the project changed.
//!
//! Every parameter (bus gain/pan and each filter param) is an `Animated` read and written at the
//! playhead: `set_at` moves the constant while the parameter is unkeyed and upserts a keyframe once it
//! is animated, exactly like a clip property.
//!
//! Edits go into a per-frame clone of `Project.buses` and are written back at the end of the frame, so
//! `undo` can snapshot the project before the first change of a gesture (the effects/inspector pattern).

use crate::engine::mixer_fx::{db_to_lin, filter_bands, filter_response_db, lin_to_db, BusGraph, EQ_BANDS};
use crate::model::{Animated, AudioFilter, Bus, FilterKind, Id, Project, TrackKind};
use crate::theme::Palette;
use crate::ui::tools::{glyph_text_button, icon_button, Dir, Glyph};
use crate::ui::Gesture;
use eframe::egui::{
    self, pos2, vec2, Align2, Button, ComboBox, DragValue, FontId, Grid, Rect, Sense, Stroke, TextEdit,
};

/// Channel-strip width in points.
const STRIP_W: f32 = 210.0;
/// Response-curve plot height.
const CURVE_H: f32 = 96.0;
/// Full-scale range of the EQ plot, in dB.
const CURVE_DB: f32 = 24.0;
/// Grid lines of the EQ plot: a third of a decade apart, labelled.
const GRID_HZ: [(f32, &str); 10] = [
    (31.0, "31"),
    (62.0, "62"),
    (125.0, "125"),
    (250.0, "250"),
    (500.0, "500"),
    (1000.0, "1k"),
    (2000.0, "2k"),
    (4000.0, "4k"),
    (8000.0, "8k"),
    (16000.0, "16k"),
];

#[derive(Default)]
pub struct MixerState {
    pub selected_bus: Option<Id>,
    /// Show the clip/track routing section.
    pub show_routing: bool,
    /// Band being dragged on a response plot, as (bus, filter index, band index). Latched for the whole
    /// drag: with five bands the nearest one changes under the pointer and the grab would hop.
    drag: Option<(Id, usize, usize)>,
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

/// Structural edits that need `Project`'s own helpers, applied after `undo` has snapshotted.
#[derive(Default)]
struct Edits {
    add_bus: bool,
    remove: Option<Id>,
    track_bus: Option<(usize, Id)>,
    clip_bus: Option<(Id, Id)>,
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut MixerState,
    project: &mut Project,
    selection: &[Id],
    buses: &BusGraph,
    time: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    #[cfg(test)]
    test_rects::clear();
    let mut g = Gesture::default();
    let mut ed = Edits::default();
    // ponytail: clone the whole bus list every frame — a handful of buses with a few filters each is
    // nothing next to a repaint. Upgrade path if it ever shows up: edit in place and snapshot lazily.
    // Main is the one bus that always exists: without it the pane is empty on a fresh project and there
    // is nothing to route to. Creating it is not a user edit, so it takes no undo entry.
    project.main_bus();
    let mut list = project.buses.clone();
    let names: Vec<(Id, String)> = list.iter().map(|b| (b.id, b.name.clone())).collect();
    let main = list.first().map(|b| b.id).unwrap_or(0);
    let feeds = feeds(project, &list, main);

    ui.horizontal(|ui| {
        let r = crate::ui::tools::glyph_text_button(ui, Glyph::Headphone, "+ Bus");
        #[cfg(test)]
        test_rects::push("add_bus".into(), r.rect);
        if r.clicked() {
            ed.add_bus = true;
            g.click();
        }
        ui.checkbox(&mut state.show_routing, "Routing");
    });
    // routing above the strips: the strips claim the rest of the pane for their own scrollbars
    if state.show_routing {
        ui.separator();
        routing(ui, project, selection, &names, main, state, &mut g, &mut ed);
    }
    ui.separator();

    // Main first: it is the master, and every other bus is read as feeding into it.
    let order: Vec<usize> = (0..list.len()).collect();
    egui::ScrollArea::horizontal().id_salt("mixer_strips").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for (n, &i) in order.iter().enumerate() {
                let is_main = list[i].id == main;
                let id = list[i].id;
                let meter = buses.meter(id);
                let sel = state.selected_bus == Some(id);
                let feed = feeds.iter().find(|(b, _)| *b == id).map(|(_, s)| s.clone()).unwrap_or_default();
                ui.push_id(("strip", id), |ui| {
                    let size = vec2(STRIP_W, ui.available_height());
                    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(STRIP_W);
                        let stroke = if sel { Stroke::new(1.0, palette.accent) } else { Stroke::NONE };
                        egui::Frame::new().stroke(stroke).inner_margin(2.0).show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt(("strip_scroll", id))
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    let s = Strip { is_main, meter, feed: &feed, time };
                                    strip(ui, &mut list[i], s, &names, palette, state, &mut g, &mut ed, n);
                                });
                        });
                    });
                });
                ui.separator();
            }
        });
    });

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

/// What one strip needs to know about its bus beyond the `Bus` itself.
struct Strip<'a> {
    is_main: bool,
    meter: (f32, f32),
    /// Tracks and buses that sum into this one, comma-joined.
    feed: &'a str,
    /// Playhead, in timeline seconds: every parameter is read and written there.
    time: f64,
}

/// Who sums into each bus: the audio tracks routed to it and the buses that send here. Mirrors
/// `mixer_fx::resolve` — a dangling send lands in Main.
/// ponytail: track-level routing only. A single clip's `Clip.bus` override is visible in the routing
/// list instead; walk `Project::bus_of` over every clip here if that turns out to be confusing.
fn feeds(project: &Project, list: &[Bus], main: Id) -> Vec<(Id, String)> {
    let live = |id: Id| if id != 0 && list.iter().any(|o| o.id == id) { id } else { main };
    list.iter()
        .map(|b| {
            let mut v: Vec<String> = project
                .audio_tracks()
                .into_iter()
                .filter(|&ti| live(project.tracks[ti].bus) == b.id)
                .map(|ti| project.tracks[ti].name.clone())
                .collect();
            let sends = list.iter().filter(|o| o.id != main && o.id != b.id && live(o.output) == b.id);
            v.extend(sends.map(|o| o.name.clone()));
            (b.id, v.join(", "))
        })
        .collect()
}

/// Diamond keyframe toggle for one parameter at the playhead; right-click clears every key. `at` only has
/// to be unique inside the strip — the mixer edits a fresh clone of the bus list every frame, so an
/// id derived from the `Animated`'s address (`crate::ui::key_buttons`) would not survive a click.
fn key_button(ui: &mut egui::Ui, a: &mut Animated, t: f64, palette: &Palette, g: &mut Gesture, at: (Id, usize, usize)) {
    let tip = if a.is_animated() {
        format!("{} keyframes — right-click to clear", a.keys.len())
    } else {
        "Toggle keyframe at playhead".to_string()
    };
    let r = icon_button(ui, palette, ui.id().with(("kf", at)), Glyph::Diamond, &tip, a.has_key_at(t));
    #[cfg(test)]
    test_rects::push(format!("kf{}_{}_{}", at.0, at.1, at.2), r.rect);
    if r.clicked() {
        a.toggle_key(t);
        g.click();
    }
    if a.is_animated() {
        r.context_menu(|ui| {
            if ui.button("Remove all keyframes").clicked() {
                a.clear_keys(t);
                g.click();
                ui.close();
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn strip(
    ui: &mut egui::Ui,
    bus: &mut Bus,
    s: Strip,
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
        let del = ui.add_enabled_ui(!s.is_main, crate::ui::markers_ui::x_button).inner;
        #[cfg(test)]
        test_rects::push(format!("del_bus{n}"), del.rect);
        if del.clicked() {
            ed.remove = Some(bus.id);
            g.click();
        }
    });

    let feed = if s.feed.is_empty() { "nothing routed here" } else { s.feed };
    let r = ui
        .add(
            egui::Label::new(egui::RichText::new(feed).small().color(palette.text_dim))
                .truncate()
                .sense(Sense::click()),
        )
        .on_hover_text(if s.feed.is_empty() { "No track or bus feeds this bus" } else { s.feed });
    #[cfg(test)]
    test_rects::push(format!("feed{n}"), r.rect);
    if r.clicked() {
        state.selected_bus = Some(bus.id);
    }

    meter(ui, s.meter, palette);

    ui.horizontal(|ui| {
        let mut db = lin_to_db(bus.gain.at(s.time) as f32).max(-60.0);
        let r = ui.add(DragValue::new(&mut db).range(-60.0..=12.0).speed(0.2).suffix(" dB").fixed_decimals(1));
        #[cfg(test)]
        test_rects::push(format!("gain{n}"), r.rect);
        if r.changed() {
            bus.gain.set_at(s.time, db_to_lin(db) as f64);
        }
        g.note(&r);
        key_button(ui, &mut bus.gain, s.time, palette, g, (bus.id, usize::MAX, 0));
        let mut pan = bus.pan.at(s.time);
        let r = ui.add(DragValue::new(&mut pan).range(-1.0..=1.0).speed(0.01).prefix("pan ").fixed_decimals(2));
        if r.changed() {
            bus.pan.set_at(s.time, pan);
        }
        g.note(&r);
        key_button(ui, &mut bus.pan, s.time, palette, g, (bus.id, usize::MAX, 1));
    });

    ui.horizontal(|ui| {
        let icon = if bus.muted { Glyph::SpeakerOff } else { Glyph::SpeakerOn };
        let r = icon_button(ui, palette, ui.id().with(("mute", bus.id)), icon, "Mute", bus.muted);
        #[cfg(test)]
        test_rects::push(format!("m{n}"), r.rect);
        if r.clicked() {
            bus.muted = !bus.muted;
            g.click();
        }
        for (label, flag, hint) in [("S", &mut bus.solo, "Solo"), ("Mono", &mut bus.mono, "Fold to mono")] {
            let r = ui.add(Button::new(label).small().selected(*flag)).on_hover_text(hint);
            #[cfg(test)]
            test_rects::push(format!("{}{n}", label.to_lowercase()), r.rect);
            if r.clicked() {
                *flag = !*flag;
                g.click();
            }
        }
    });

    if !s.is_main {
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

    filters(ui, bus, s.time, palette, state, g, n);
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

#[allow(clippy::too_many_arguments)]
fn filters(
    ui: &mut egui::Ui,
    bus: &mut Bus,
    time: f64,
    palette: &Palette,
    state: &mut MixerState,
    g: &mut Gesture,
    n: usize,
) {
    let _ = n;
    let bus_id = bus.id;
    let count = bus.filters.len();
    let mut remove: Option<usize> = None;
    let mut swap: Option<(usize, usize)> = None;
    for (i, f) in bus.filters.iter_mut().enumerate() {
        // a project saved before the kind grew a parameter is short: top it up from the spec table
        f.fill_params();
        ui.horizontal(|ui| {
            g.note(&ui.checkbox(&mut f.enabled, ""));
            ui.label(f.kind.name());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let del = crate::ui::markers_ui::x_button(ui).on_hover_text("Remove this filter");
                #[cfg(test)]
                test_rects::push(format!("delfx{n}_{i}"), del.rect);
                if del.clicked() {
                    remove = Some(i);
                }
                let down = |ui: &mut egui::Ui| glyph_text_button(ui, Glyph::Tri(Dir::Down), "");
                if ui.add_enabled_ui(i + 1 < count, down).inner.on_hover_text("Move down").clicked() {
                    swap = Some((i, i + 1));
                }
                let up = |ui: &mut egui::Ui| glyph_text_button(ui, Glyph::Tri(Dir::Up), "");
                if ui.add_enabled_ui(i > 0, up).inner.on_hover_text("Move up").clicked() {
                    swap = Some((i, i - 1));
                }
            });
        });
        if curve(ui, f, (bus_id, i), time, palette, state, g, n) {
            g.changed = true;
        }
        let specs = f.kind.params();
        // the EQ's parameters are stored in the order the old 3-band EQ used; read them by band
        let order: Vec<usize> = if f.kind == FilterKind::Eq {
            EQ_BANDS.iter().flat_map(|&(_, gi, fi, qi)| [fi, gi, qi]).collect()
        } else {
            (0..specs.len()).collect()
        };
        Grid::new(("fx", bus_id, i)).num_columns(3).spacing(vec2(4.0, 2.0)).show(ui, |ui| {
            for j in order {
                let (Some(spec), Some(a)) = (specs.get(j), f.params.get_mut(j)) else { continue };
                ui.label(spec.name);
                let mut v = a.at(time);
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
                        let mut r = ComboBox::from_id_salt(("ty", bus_id, i))
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
                    a.set_at(time, v);
                }
                g.note(&r);
                key_button(ui, a, time, palette, g, (bus_id, i, j));
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

/// EQ / pass-filter response plot: a labelled log-frequency + dB grid, the summed curve on top, and one
/// draggable handle per band (X = frequency, Y = gain). The grabbed band is latched for the whole drag.
/// Returns true when the drag changed a parameter.
#[allow(clippy::too_many_arguments)]
fn curve(
    ui: &mut egui::Ui,
    f: &mut AudioFilter,
    key: (Id, usize),
    time: f64,
    palette: &Palette,
    state: &mut MixerState,
    g: &mut Gesture,
    n: usize,
) -> bool {
    let _ = n;
    // (gain param, freq param) per band, in the order `filter_bands` reports them
    let map: Vec<(Option<usize>, usize)> = match f.kind {
        FilterKind::Eq => EQ_BANDS.iter().map(|&(_, gi, fi, _)| (Some(gi), fi)).collect(),
        FilterKind::HighPass | FilterKind::LowPass => vec![(None, 0)],
        _ => return false,
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), CURVE_H), Sense::click_and_drag());
    #[cfg(test)]
    test_rects::push(format!("curve{n}_{}", key.1), rect);
    // the frequency labels live in a strip along the bottom; the response only uses the rest
    let label_h = 11.0;
    let plot = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - label_h));
    let y_of = |db: f32| plot.center().y - db.clamp(-CURVE_DB, CURVE_DB) / CURVE_DB * plot.height() * 0.5;
    let x_of = |hz: f32| rect.left() + (hz.max(20.0) / 20.0).log10() / 3.0 * rect.width();
    let hz_of = |x: f32| 20.0 * 10f32.powf(((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0) * 3.0);
    let bands = filter_bands(f, time);
    let handle = |i: usize| pos2(x_of(bands[i].1), y_of(bands[i].3));

    let p = ui.painter();
    p.rect_filled(rect, 0.0, palette.bg);
    let faint = palette.border.gamma_multiply(0.8);
    let font = FontId::proportional(8.0);
    for db in [-18.0, -12.0, -6.0, 6.0, 12.0, 18.0] {
        p.line_segment([pos2(plot.left(), y_of(db)), pos2(plot.right(), y_of(db))], Stroke::new(1.0, faint));
    }
    for (hz, name) in GRID_HZ {
        let x = x_of(hz);
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, faint));
        // keep the outermost labels inside the plot instead of half-clipped
        let tx = x.clamp(rect.left() + 9.0, rect.right() - 9.0);
        p.text(pos2(tx, rect.bottom() - label_h), Align2::CENTER_TOP, name, font.clone(), palette.text_dim);
    }
    let zero = Stroke::new(1.0, palette.text_dim.gamma_multiply(0.6));
    p.line_segment([pos2(plot.left(), y_of(0.0)), pos2(plot.right(), y_of(0.0))], zero);
    for db in [12.0f32, -12.0] {
        let t = if db > 0.0 { "+12" } else { "-12" };
        p.text(pos2(plot.left() + 2.0, y_of(db)), Align2::LEFT_CENTER, t, font.clone(), palette.text_dim);
    }
    let steps = (rect.width() as usize / 3).clamp(16, 128);
    let pts: Vec<_> = (0..=steps)
        .map(|i| {
            let x = rect.left() + rect.width() * i as f32 / steps as f32;
            pos2(x, y_of(filter_response_db(f, time, hz_of(x))))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5, palette.accent)));
    let hot = state.drag.filter(|d| (d.0, d.1) == key).map(|d| d.2);
    for i in 0..bands.len() {
        let c = handle(i);
        let on = hot == Some(i) || resp.hover_pos().is_some_and(|q| (q - c).length() < 8.0);
        p.circle_filled(c, if on { 4.5 } else { 3.0 }, palette.keyframe);
        if on {
            p.circle_stroke(c, 6.5, Stroke::new(1.0, palette.accent));
        }
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);

    let mut changed = false;
    if resp.drag_started() {
        g.start = true;
        // grab the handle nearest the pointer and keep it for the rest of the drag
        if let Some(pos) = resp.interact_pointer_pos() {
            let pick =
                (0..bands.len()).min_by(|&a, &b| (handle(a) - pos).length().total_cmp(&(handle(b) - pos).length()));
            state.drag = pick.map(|i| (key.0, key.1, i));
        }
    }
    if resp.dragged() {
        if let (Some(bi), Some(pos)) =
            (state.drag.filter(|d| (d.0, d.1) == key).map(|d| d.2), resp.interact_pointer_pos())
        {
            if let Some(&(gi, fi)) = map.get(bi) {
                let specs = f.kind.params();
                if let (Some(spec), Some(a)) = (specs.get(fi), f.params.get_mut(fi)) {
                    a.set_at(time, (hz_of(pos.x) as f64).clamp(spec.min, spec.max));
                    changed = true;
                }
                if let Some(gi) = gi {
                    let db = (plot.center().y - pos.y) / (plot.height() * 0.5) * CURVE_DB;
                    if let (Some(spec), Some(a)) = (specs.get(gi), f.params.get_mut(gi)) {
                        a.set_at(time, (db as f64).clamp(spec.min, spec.max));
                        changed = true;
                    }
                }
            }
        }
    }
    if resp.drag_stopped() {
        state.drag = None;
    }
    changed
}

/// One row per audio track — click a track to send it to the selected bus, or pick from its own combo —
/// plus the `Clip.bus` override of the selected clip.
#[allow(clippy::too_many_arguments)]
fn routing(
    ui: &mut egui::Ui,
    project: &Project,
    selection: &[Id],
    names: &[(Id, String)],
    main: Id,
    state: &mut MixerState,
    g: &mut Gesture,
    ed: &mut Edits,
) {
    if names.is_empty() {
        ui.label("Add a bus first");
        return;
    }
    let label = |id: Id| {
        names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.as_str()).unwrap_or(if id == 0 { "Main" } else { "?" })
    };
    let target = state.selected_bus.filter(|id| names.iter().any(|(i, _)| i == id));
    let clip = selection.iter().copied().find(|&id| project.clip(id).is_some());

    ui.label(match target {
        Some(id) => format!("Click a track to send it to “{}”", label(id)),
        None => "Click a bus strip's name to pick where tracks go".to_string(),
    });
    egui::ScrollArea::vertical().id_salt("mixer_routing_scroll").max_height(96.0).show(ui, |ui| {
        Grid::new("mixer_routing").num_columns(2).show(ui, |ui| {
            for (row, ti) in project.audio_tracks().into_iter().enumerate() {
                let Some(track) = project.tracks.get(ti) else { continue };
                let cur = if track.bus == 0 { main } else { track.bus };
                let hit = target.is_some_and(|id| id != cur);
                let r = ui
                    .add(Button::new(&track.name).selected(clip.is_some_and(|c| project.track_of(c) == Some(ti))))
                    .on_hover_text(match target {
                        Some(id) if id != cur => format!("Send this track to “{}”", label(id)),
                        Some(_) => "Already on the selected bus".to_string(),
                        None => "Select a bus first (click its name in a strip)".to_string(),
                    });
                #[cfg(test)]
                test_rects::push(format!("track{row}"), r.rect);
                let _ = row;
                if r.clicked() && hit {
                    ed.track_bus = Some((ti, target.unwrap_or(main)));
                    g.click();
                }
                ComboBox::from_id_salt(("track_bus", ti)).selected_text(label(cur)).width(140.0).show_ui(ui, |ui| {
                    for (id, name) in names {
                        if ui.selectable_label(cur == *id, name).clicked() && cur != *id {
                            ed.track_bus = Some((ti, *id));
                            g.click();
                        }
                    }
                });
                ui.end_row();
            }

            if let Some(c) = clip.and_then(|id| project.clip(id)) {
                let audio = project
                    .track_of(c.id)
                    .and_then(|ti| project.tracks.get(ti))
                    .is_some_and(|t| t.kind == TrackKind::Audio);
                if audio || c.kind == crate::model::ClipKind::Sequence {
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
        /// Timeline playhead handed to `show` — every parameter is read and written there.
        playhead: f64,
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
                playhead: 0.0,
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
            let Harness { ctx, project, state, graph, selection, undos, playhead, .. } = self;
            let playhead = *playhead;
            let mut edited = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    edited |= show(ui, state, project, selection, graph, playhead, &pal, &mut undo);
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
        // both strips are there (Main draws first, so it is index 0)
        assert!(test_rects::get("m0").is_some() && test_rects::get("m1").is_some());
    }

    #[test]
    fn add_and_remove_a_bus() {
        let mut h = Harness::new();
        assert!(h.click_named("add_bus"));
        assert_eq!(h.project.buses.len(), 3);
        assert_eq!(h.undos, 1);
        // strip 1 is the first non-Main bus; deleting it drops back to two
        assert!(h.click_named("del_bus1"));
        assert_eq!(h.project.buses.len(), 2);
    }

    #[test]
    fn mute_solo_mono_toggle() {
        let mut h = Harness::new();
        assert!(h.click_named("m1"));
        assert!(h.project.buses[1].muted, "strip 1 is the Music bus");
        assert!(h.click_named("s1"));
        assert!(h.project.buses[1].solo);
        assert!(h.click_named("mono1"));
        assert!(h.project.buses[1].mono);
        assert_eq!(h.undos, 3);
        // Main can never be deleted
        let del_main = test_rects::get("del_bus0").expect("Main delete button");
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
        let rect = test_rects::get("p1_0_0").expect("gain param");
        let changed = h.drag(rect.center(), vec2(40.0, 0.0));
        assert!(changed, "dragging the DragValue edits the project");
        assert!(h.project.buses[1].filters[0].params[0].value > 0.0);
        assert!(h.undos >= 1);
        // and it can be removed again
        assert!(h.click_named("delfx1_0"));
        assert!(h.project.buses[1].filters.is_empty());
    }

    #[test]
    fn eq_curve_drag_moves_a_band() {
        let mut h = Harness::new();
        let music = h.project.buses[1].id;
        h.project.bus_mut(music).unwrap().filters.push(AudioFilter::new(FilterKind::Eq));
        h.frame(vec![]);
        // grab the low shelf's handle: x is its frequency on the log axis, y its 0 dB gain
        let plot = test_rects::get("curve1_0").expect("response plot");
        let x = plot.left() + (120.0f32 / 20.0).log10() / 3.0 * plot.width();
        let handle = pos2(x, plot.center().y);
        assert!(h.drag(handle, vec2(0.0, -20.0)), "dragging a handle edits its band");
        let f = &h.project.buses[1].filters[0];
        assert!(f.params[0].value > 3.0, "the low shelf was boosted: {}", f.params[0].value);
        assert!((f.params[1].value - 120.0).abs() < 20.0, "its frequency stayed put: {}", f.params[1].value);
        assert_eq!(f.params[9].value, 0.0, "the neighbouring band is untouched");
        assert!(h.undos >= 1);
    }

    #[test]
    fn filter_params_are_keyframed_at_the_playhead() {
        let mut h = Harness::new();
        let music = h.project.buses[1].id;
        h.project.bus_mut(music).unwrap().filters.push(AudioFilter::new(FilterKind::Gain));
        h.frame(vec![]);
        assert!(h.click_named(&format!("kf{music}_0_0")), "◆ keyframes the parameter");
        assert_eq!(h.project.buses[1].filters[0].params[0].keys.len(), 1);
        // at another playhead the drag upserts a key instead of moving the constant
        h.playhead = 2.0;
        let rect = test_rects::get("p1_0_0").expect("gain param");
        assert!(h.drag(rect.center(), vec2(40.0, 0.0)));
        let a = &h.project.buses[1].filters[0].params[0];
        assert_eq!(a.keys.len(), 2, "{:?}", a.keys);
        assert!(a.at(2.0) > a.at(0.0) + 1.0, "the 2 s key holds the dragged value: {:?}", a.keys);
    }

    #[test]
    fn clicking_a_track_row_sends_it_to_the_selected_bus() {
        let mut h = Harness::new();
        let main = h.project.buses[0].id;
        let ai = h.project.audio_tracks()[0];
        h.state.selected_bus = Some(main);
        assert!(h.click_named("track0"), "the track row reroutes to the selected bus");
        assert_eq!(h.project.tracks[ai].bus, main);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn feeds_names_what_sums_into_each_bus() {
        let h = Harness::new();
        let (main, music) = (h.project.buses[0].id, h.project.buses[1].id);
        let f = feeds(&h.project, &h.project.buses, main);
        let of = |id| f.iter().find(|(b, _)| *b == id).map(|(_, s)| s.clone()).unwrap();
        let track = h.project.tracks[h.project.audio_tracks()[0]].name.clone();
        assert_eq!(of(music), track, "the track routed to Music");
        assert_eq!(of(main), "Music", "and Music itself sends into Main");
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
