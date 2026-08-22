//! Node editor: the selected clip's effect chain as a graph you can wire up.
//!
//! Canvas with pan (drag empty space / middle-drag) and zoom (Ctrl+Scroll). Each node is a rounded box:
//! title (`NodeKind::title`), an enable checkbox, input ports on the left, one output port on the right,
//! and its parameters inline (collapsed to the first two when the node is small). Drag from a port to
//! another to connect (`NodeGraph::connect` refuses cycles); drag a wire off a port to disconnect;
//! right-click a node for Delete / Duplicate / Add mask; right-click the canvas for the "Add node" menu
//! (every `EffectKind` grouped by `category()`, Blend, Combine, Merge, Matte, Mask, Color, Clip, Text,
//! Input, then one entry per project asset — an Asset node needs a real id, so it is picked there).
//! Selecting a node shows its full parameters in the Inspector and lets the Curves pane keyframe them.
//! `Project::ensure_graph` converts a clip's linear stack the first time this pane opens for it — an
//! undoable edit, since the renderer evaluates the graph from then on and ignores `clip.effects`.
//! Every mutation calls `undo` once per gesture.

use crate::model::{Animated, BlendMode, Effect, EffectKind, Id, Mask, NodeKind, Project};
use crate::theme::Palette;
use eframe::egui;
use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, TextStyle, UiBuilder,
};

/// Node box width in graph units.
const NODE_W: f32 = 150.0;
const HEADER_H: f32 = 22.0;
const ROW_H: f32 = 18.0;
const THUMB_H: f32 = 56.0;
/// A wire snaps to a port within this many screen px.
const SNAP: f32 = 12.0;
const PORT_R: f32 = 4.0;
/// At most this many parameters are shown on the box itself (the Inspector has the rest).
const INLINE_PARAMS: usize = 2;

#[derive(Default)]
pub struct NodesState {
    /// Pan/zoom of the canvas.
    pub offset: egui::Vec2,
    pub zoom: f32,
    pub selected: Option<Id>,
    /// A connection being dragged: (node, port, from_output).
    pub linking: Option<(Id, usize, bool)>,
    /// The node currently being moved (one undo per gesture).
    pub dragging: Option<Id>,
    /// Input port a wire is being pulled off, until the pointer actually leaves it.
    pub detach: Option<(Id, usize)>,
    /// Where the canvas was right-clicked, in graph units: new nodes land there.
    pub menu_at: Option<(f32, f32)>,
    /// Live previews, keyed by node. Empty unless a renderer hands textures over, and the panel is
    /// fully usable without them (there is no GL context in tests or headless runs).
    pub thumbs: std::collections::HashMap<Id, egui::TextureId>,
}

#[derive(Default)]
pub struct NodesResponse {
    pub edited: bool,
    /// The node whose parameters the inspector should show.
    pub selected: Option<Id>,
}

/// One buffered mutation. The UI pass only reads the graph, so the undo snapshot can be taken
/// between collecting these and applying them.
#[derive(Debug)]
enum Act {
    Add(NodeKind, f32, f32),
    Delete(Id),
    Move(Id, f32, f32),
    SetParam(Id, usize, f64),
    SetText(Id, String),
    SetEnabled(Id, bool),
    Connect(Id, Id, usize),
    Disconnect(Id, usize),
    AddMask(Id),
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut NodesState,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> NodesResponse {
    let mut out = NodesResponse::default();
    if !(state.zoom > 0.05) {
        state.zoom = 1.0; // a default-constructed state has zoom 0
    }
    let Some(&clip_id) = selection.first() else {
        hint(ui, palette, "Select a clip to edit its node graph");
        return out;
    };
    if project.clip(clip_id).is_none() {
        hint(ui, palette, "Select a clip to edit its node graph");
        return out;
    }
    // Converting the linear stack is a real edit, not a side effect of drawing: from here on the
    // renderer evaluates the graph and ignores clip.effects, so it gets its own undo entry and tells
    // the app the project changed.
    if project.clip(clip_id).is_some_and(|c| c.graph.is_none()) {
        undo(project);
        project.ensure_graph(clip_id);
        out.edited = true;
    }
    // the graph is small; a copy keeps the draw pass read-only so `undo` can snapshot mid-frame
    let Some(graph) = project.clip(clip_id).and_then(|c| c.graph.clone()) else {
        return out;
    };
    let lt = project.clip(clip_id).map(|c| c.local(playhead)).unwrap_or(0.0);
    if state.selected.is_some_and(|s| graph.node(s).is_none()) {
        state.selected = None;
    }

    let rect = ui.available_rect_before_wrap();
    let base = ui.id();
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0, palette.bg);

    // ---- pan / zoom ----
    let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|q| rect.contains(*q));
    if pointer.is_some() {
        // egui folds Ctrl+Scroll (and pinch) into zoom_delta and keeps it out of the scroll delta
        let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.raw_scroll_delta));
        if (zoom_delta - 1.0).abs() > 1e-4 {
            let anchor = pointer.unwrap_or(rect.center());
            let g = (anchor - rect.min - state.offset) / state.zoom;
            state.zoom = (state.zoom * zoom_delta).clamp(0.3, 3.0);
            state.offset = anchor - rect.min - g * state.zoom;
        } else if scroll != egui::Vec2::ZERO {
            state.offset += scroll;
        }
    }
    let z = state.zoom;
    let org = rect.min + state.offset;
    let to_screen = |x: f32, y: f32| org + vec2(x * z, y * z);
    let to_graph = |q: Pos2| ((q - org) / z).to_pos2();

    grid(&p, rect, org, z, palette);

    // ---- geometry ----
    let mut boxes: Vec<(usize, Rect)> = Vec::with_capacity(graph.nodes.len());
    for (i, n) in graph.nodes.iter().enumerate() {
        let h = node_height(&n.kind, state.thumbs.contains_key(&n.id));
        boxes.push((i, Rect::from_min_size(to_screen(n.x, n.y), vec2(NODE_W * z, h * z))));
    }
    let rect_of = |id: Id| boxes.iter().find(|(i, _)| graph.nodes[*i].id == id).map(|(_, r)| *r);

    // ---- wires (behind the boxes) ----
    let wire = Stroke::new((1.5 * z).max(1.0), palette.text_dim);
    for e in &graph.edges {
        let (Some(a), Some(b)) = (rect_of(e.from), rect_of(e.to)) else { continue };
        bezier(&p, out_port(a, z), in_port(b, e.port, z), wire);
    }

    // ---- ports (screen space) ----
    let mut ports: Vec<(Id, usize, bool, Pos2)> = Vec::new();
    for (i, r) in &boxes {
        let n = &graph.nodes[*i];
        for k in 0..n.kind.inputs() {
            ports.push((n.id, k, false, in_port(*r, k, z)));
        }
        if n.kind != NodeKind::Output {
            ports.push((n.id, 0, true, out_port(*r, z)));
        }
    }
    let nearest = |q: Pos2, want_output: Option<bool>| {
        ports
            .iter()
            .filter(|(_, _, o, _)| want_output.is_none_or(|w| *o == w))
            .map(|prt| (prt, prt.3.distance(q)))
            .filter(|(_, d)| *d <= SNAP)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(prt, _)| *prt)
    };

    let mut acts: Vec<Act> = Vec::new();
    let mut needs_undo = false;

    // ---- wire dragging ----
    let (pressed, released) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released()));
    if pressed && state.linking.is_none() {
        if let Some(q) = pointer {
            if let Some((node, port, is_out, _)) = nearest(q, None) {
                if !is_out {
                    // pulling a wire off an input detaches it and keeps dragging the far end
                    match graph.input_of(node, port) {
                        // the wire is only detached once the pointer leaves the port (below): a plain
                        // click used to disconnect and reconnect, two undo entries for no net change
                        Some(up) => {
                            state.linking = Some((up, 0, true));
                            state.detach = Some((node, port));
                        }
                        None => state.linking = Some((node, port, false)),
                    }
                } else {
                    state.linking = Some((node, port, is_out));
                }
            }
        }
    }
    if let Some((node, port, is_out)) = state.linking {
        if let (Some(r), Some(q)) = (rect_of(node), pointer) {
            let anchor = if is_out { out_port(r, z) } else { in_port(r, port, z) };
            let (a, b) = if is_out { (anchor, q) } else { (q, anchor) };
            bezier(&p, a, b, Stroke::new((1.5 * z).max(1.0), palette.accent));
            // highlight the ports this wire could land on
            for (_, _, _, pp) in ports.iter().filter(|(_, _, o, _)| *o != is_out) {
                if pp.distance(q) <= SNAP {
                    p.circle_filled(*pp, PORT_R * z + 2.0, palette.accent);
                }
            }
        }
        // committed detach: the pointer left the port it was pulled from
        if let Some((dn, dp)) = state.detach {
            if let (Some(dr), Some(q)) = (rect_of(dn), pointer) {
                if in_port(dr, dp, z).distance(q) > SNAP {
                    acts.push(Act::Disconnect(dn, dp));
                    needs_undo = true;
                    state.detach = None;
                }
            }
        }
        if released {
            // still attached = the gesture was a click on the port: leave the graph alone
            let clicked_in_place = state.detach.take().is_some();
            if let (false, Some(q)) = (clicked_in_place, pointer) {
                if let Some((tn, tp, ..)) = nearest(q, Some(!is_out)) {
                    let (from, to, prt) = if is_out { (node, tn, tp) } else { (tn, node, port) };
                    // dry-run on the frame's copy: a refused (cycle-forming) wire must not push an
                    // undo entry that changes nothing
                    if graph.clone().connect(from, to, prt) {
                        acts.push(Act::Connect(from, to, prt));
                        needs_undo = true;
                    }
                }
            }
            state.linking = None;
        }
    }

    // ---- node boxes ----
    let linking = state.linking.is_some();
    for &(i, r) in &boxes {
        let n = &graph.nodes[i];
        let nr = ui.interact(r, base.with(("node", n.id)), Sense::click_and_drag());
        if nr.clicked() || nr.drag_started() {
            state.selected = Some(n.id);
        }
        if nr.drag_started_by(egui::PointerButton::Primary) && !linking {
            state.dragging = Some(n.id);
            needs_undo = true; // exactly one snapshot for the whole move
        }
        if state.dragging == Some(n.id) {
            if nr.dragged() {
                let d = nr.drag_delta() / z;
                if d != egui::Vec2::ZERO {
                    acts.push(Act::Move(n.id, n.x + d.x, n.y + d.y));
                }
            }
            if nr.drag_stopped() {
                state.dragging = None;
            }
        }
        let selected = state.selected == Some(n.id);
        paint_node(&p, ui, r, n, selected, palette, z, state.thumbs.get(&n.id).copied());

        // enable toggle
        let cb = Rect::from_min_size(r.min + vec2(4.0 * z, 6.0 * z), vec2(10.0 * z, 10.0 * z));
        let cbr = ui.interact(cb, base.with(("en", n.id)), Sense::click());
        p.rect_stroke(cb, CornerRadius::same(1), Stroke::new(1.0, palette.border), StrokeKind::Inside);
        if n.enabled {
            p.rect_filled(cb.shrink(2.0 * z), CornerRadius::ZERO, palette.accent);
        }
        if cbr.clicked() {
            acts.push(Act::SetEnabled(n.id, !n.enabled));
            needs_undo = true;
        }

        // inline parameters (real widgets only when the box is close to full size)
        if z >= 0.6 {
            if let NodeKind::Text(style) = &n.kind {
                let row = Rect::from_min_size(
                    r.min + vec2(6.0 * z, HEADER_H * z + 2.0),
                    vec2((NODE_W - 12.0) * z, ROW_H * z),
                );
                let mut txt = style.text.clone();
                let mut sub = ui.new_child(UiBuilder::new().max_rect(row).id_salt(("txt", n.id)));
                sub.set_clip_rect(row.intersect(rect));
                let te = sub
                    .add(egui::TextEdit::singleline(&mut txt).desired_width(f32::INFINITY))
                    .on_hover_text("{frame} timeline frame · {time} clock · {n} frames into the clip");
                if te.changed() {
                    acts.push(Act::SetText(n.id, txt));
                }
                if te.gained_focus() {
                    needs_undo = true;
                }
            }
            if let NodeKind::Effect(e) = &n.kind {
                for (pi, spec) in e.specs().iter().take(INLINE_PARAMS).enumerate() {
                    let row = Rect::from_min_size(
                        r.min + vec2(6.0 * z, (HEADER_H + ROW_H * pi as f32) * z + 2.0),
                        vec2((NODE_W - 12.0) * z, ROW_H * z),
                    );
                    let mut v = e.at(pi, lt);
                    let mut sub = ui.new_child(UiBuilder::new().max_rect(row).id_salt(("p", n.id, pi)));
                    sub.set_clip_rect(row.intersect(rect));
                    let changed = sub
                        .horizontal(|ui| {
                            ui.style_mut().spacing.interact_size.y = ROW_H;
                            ui.label(egui::RichText::new(spec.name).small().color(palette.text_dim));
                            if e.kind.is_bool_param(pi) {
                                let mut b = v >= 0.5;
                                let r = ui.checkbox(&mut b, "");
                                v = if b { 1.0 } else { 0.0 };
                                r
                            } else {
                                ui.add(
                                    egui::DragValue::new(&mut v)
                                        .speed((spec.max - spec.min).abs().max(1.0) / 200.0)
                                        .range(spec.min..=spec.max),
                                )
                            }
                        })
                        .inner;
                    if changed.changed() {
                        acts.push(Act::SetParam(n.id, pi, v));
                    }
                    if changed.drag_started() || changed.gained_focus() {
                        needs_undo = true;
                    }
                }
            }
        }

        // per-node menu
        let is_output = n.kind == NodeKind::Output;
        let (id, kind) = (n.id, n.kind.clone());
        let (nx, ny) = (n.x, n.y);
        nr.context_menu(|ui| {
            state.selected = Some(id);
            if !is_output && menu_entry(ui, base.with(("del", id)), "Delete").clicked() {
                acts.push(Act::Delete(id));
                needs_undo = true;
                ui.close();
            }
            if menu_entry(ui, base.with(("dup", id)), "Duplicate").clicked() {
                acts.push(Act::Add(kind.clone(), nx + 24.0, ny + 24.0));
                needs_undo = true;
                ui.close();
            }
            if matches!(&kind, NodeKind::Effect(_)) && menu_entry(ui, base.with(("mask", id)), "Add mask").clicked() {
                acts.push(Act::AddMask(id));
                needs_undo = true;
                ui.close();
            }
        });
    }

    // ---- canvas ----
    if resp.dragged_by(egui::PointerButton::Middle)
        || (resp.dragged_by(egui::PointerButton::Primary) && !linking && state.dragging.is_none())
    {
        state.offset += resp.drag_delta();
    }
    if resp.clicked() {
        state.selected = None;
    }
    if resp.secondary_clicked() {
        let g = to_graph(pointer.unwrap_or(rect.center()));
        state.menu_at = Some((g.x, g.y));
    }
    let at = state.menu_at.unwrap_or((40.0, 40.0));
    let assets: Vec<(Id, String)> = project.assets.iter().map(|a| (a.id, a.name())).collect();
    resp.context_menu(|ui| add_menu(ui, base, at, &assets, &mut acts, &mut needs_undo));

    // Delete / Ctrl+D on the selected node (the same actions as the menu, reachable from the keyboard)
    if let Some(sel) = state.selected {
        if !ui.ctx().wants_keyboard_input() && pointer.is_some() {
            let (del, dup) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                    i.consume_key(egui::Modifiers::CTRL, egui::Key::D),
                )
            });
            if del && graph.node(sel).is_some_and(|n| n.kind != NodeKind::Output) {
                acts.push(Act::Delete(sel));
                needs_undo = true;
            }
            if dup {
                if let Some(n) = graph.node(sel) {
                    acts.push(Act::Add(n.kind.clone(), n.x + 24.0, n.y + 24.0));
                    needs_undo = true;
                }
            }
        }
    }

    // ---- apply ----
    // not gated on `acts`: a gesture's first frame (drag start / entering a text field) usually has no
    // act yet, and that is exactly the state the snapshot has to capture
    if needs_undo {
        undo(project);
    }
    for a in acts {
        out.edited |= apply(project, clip_id, a, &mut state.selected);
    }
    out.selected = state.selected;
    out
}

/// Apply one buffered action; returns true when the project actually changed.
fn apply(project: &mut Project, clip: Id, act: Act, selected: &mut Option<Id>) -> bool {
    if let Act::Add(kind, x, y) = act {
        let id = project.add_node(clip, kind, x, y);
        if let Some(id) = id {
            *selected = Some(id);
        }
        return id.is_some();
    }
    let Some(g) = project.clip_mut(clip).and_then(|c| c.graph.as_mut()) else { return false };
    match act {
        Act::Add(..) => false,
        Act::Delete(id) => {
            if g.node(id).is_none_or(|n| n.kind == NodeKind::Output) {
                return false;
            }
            g.remove_node(id);
            if *selected == Some(id) {
                *selected = None;
            }
            true
        }
        Act::Move(id, x, y) => match g.node_mut(id) {
            Some(n) => {
                n.x = x;
                n.y = y;
                true
            }
            None => false,
        },
        Act::SetText(id, s) => match g.node_mut(id) {
            Some(n) => match &mut n.kind {
                NodeKind::Text(style) => {
                    style.text = s;
                    true
                }
                _ => false,
            },
            None => false,
        },
        Act::SetParam(id, i, v) => match g.node_mut(id) {
            Some(n) => match &mut n.kind {
                NodeKind::Effect(e) => match e.params.get_mut(i) {
                    Some(a) => {
                        a.value = v;
                        true
                    }
                    None => false,
                },
                _ => false,
            },
            None => false,
        },
        Act::SetEnabled(id, on) => match g.node_mut(id) {
            Some(n) => {
                n.enabled = on;
                if let NodeKind::Effect(e) = &mut n.kind {
                    e.enabled = on;
                }
                true
            }
            None => false,
        },
        // refused when it would close a loop — the graph is left exactly as it was
        Act::Connect(from, to, port) => g.connect(from, to, port),
        Act::Disconnect(to, port) => {
            let had = g.input_of(to, port).is_some();
            g.disconnect(to, port);
            had
        }
        Act::AddMask(id) => match g.node_mut(id) {
            Some(n) => match &mut n.kind {
                NodeKind::Effect(e) if e.mask.is_none() => {
                    e.mask = Some(Mask::default());
                    true
                }
                _ => false,
            },
            None => false,
        },
    }
}

/// "Add node": every `EffectKind` under its `category()`, then the non-effect nodes and the project's
/// assets (an Asset node needs a real id, so it is picked here instead of on the node box).
fn add_menu(
    ui: &mut egui::Ui,
    base: egui::Id,
    at: (f32, f32),
    assets: &[(Id, String)],
    acts: &mut Vec<Act>,
    needs_undo: &mut bool,
) {
    let mut push = |ui: &egui::Ui, kind: NodeKind| {
        acts.push(Act::Add(kind, at.0, at.1));
        *needs_undo = true;
        ui.close();
    };
    let mut cats: Vec<&'static str> = Vec::new();
    for k in EffectKind::ALL {
        if !cats.contains(&k.category()) {
            cats.push(k.category());
        }
    }
    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
        for c in cats {
            ui.label(egui::RichText::new(c).small().weak());
            for k in EffectKind::ALL.iter().filter(|k| k.category() == c) {
                if menu_entry(ui, base.with(("add", k.name())), k.name()).clicked() {
                    push(ui, NodeKind::Effect(Effect::new(*k)));
                }
            }
            ui.separator();
        }
        ui.label(egui::RichText::new("Graph").small().weak());
        for (name, kind) in [
            ("Blend", NodeKind::Blend { mode: BlendMode::Normal, opacity: Animated::new(1.0) }),
            ("Combine", NodeKind::Combine { mode: BlendMode::Normal, factor: Animated::new(0.5) }),
            ("Merge", NodeKind::Merge),
            ("Matte", NodeKind::Matte { invert: false, use_alpha: false }),
            ("Mask", NodeKind::Mask(Mask::default())),
            ("Color", NodeKind::Color([0, 0, 0, 255])),
            // ponytail: a fresh Clip node points at nothing — the inspector picks the source clip
            ("Clip", NodeKind::Clip(0)),
            // a fresh Text node is the clock; a plain string is one edit away
            ("Text", NodeKind::Text(crate::model::TextStyle { text: "{time}".into(), ..Default::default() })),
            ("Input", NodeKind::Input),
        ] {
            if menu_entry(ui, base.with(("add", name)), name).clicked() {
                push(ui, kind);
            }
        }
        if !assets.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("Asset").small().weak());
            for (id, name) in assets {
                if menu_entry(ui, base.with(("asset", *id)), name).clicked() {
                    push(ui, NodeKind::Asset(*id));
                }
            }
        }
    });
}

/// Menu row with a caller-chosen id (so the headless tests can find it).
fn menu_entry(ui: &mut egui::Ui, id: egui::Id, label: &str) -> egui::Response {
    let w = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(2), ui.visuals().widgets.hovered.bg_fill);
    }
    ui.painter().text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        TextStyle::Button.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    r
}

// ---------- painting ----------

fn hint(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(palette.text_dim));
    });
}

fn node_height(kind: &NodeKind, thumb: bool) -> f32 {
    let params = match kind {
        NodeKind::Effect(e) => e.specs().len().min(INLINE_PARAMS),
        NodeKind::Text(_) => 1, // the format field
        _ => 0,
    };
    let rows = kind.inputs().max(params).max(1);
    HEADER_H + rows as f32 * ROW_H + 6.0 + if thumb { THUMB_H } else { 0.0 }
}

fn in_port(r: Rect, i: usize, z: f32) -> Pos2 {
    pos2(r.left(), r.top() + (HEADER_H + ROW_H * (i as f32 + 0.5)) * z)
}

fn out_port(r: Rect, z: f32) -> Pos2 {
    pos2(r.right(), r.top() + (HEADER_H + ROW_H * 0.5) * z)
}

fn bezier(p: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let d = ((b.x - a.x).abs() * 0.5).clamp(24.0, 140.0);
    p.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [a, pos2(a.x + d, a.y), pos2(b.x - d, b.y), b],
        false,
        Color32::TRANSPARENT,
        stroke,
    ));
}

fn grid(p: &egui::Painter, rect: Rect, org: Pos2, z: f32, palette: &Palette) {
    let step = 40.0 * z;
    if step < 8.0 {
        return;
    }
    let s = Stroke::new(1.0, palette.border.gamma_multiply(0.4));
    let mut x = rect.left() + (org.x - rect.left()).rem_euclid(step);
    while x < rect.right() {
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], s);
        x += step;
    }
    let mut y = rect.top() + (org.y - rect.top()).rem_euclid(step);
    while y < rect.bottom() {
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], s);
        y += step;
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_node(
    p: &egui::Painter,
    ui: &egui::Ui,
    r: Rect,
    n: &crate::model::Node,
    selected: bool,
    palette: &Palette,
    z: f32,
    thumb: Option<egui::TextureId>,
) {
    let cr = CornerRadius::same((3.0 * z) as u8);
    p.rect_filled(r, cr, palette.panel);
    let head = Rect::from_min_size(r.min, vec2(r.width(), HEADER_H * z));
    p.rect_filled(head, cr, if n.enabled { palette.header } else { palette.bg });
    let border = if selected { Stroke::new(2.0, palette.accent) } else { Stroke::new(1.0, palette.border) };
    p.rect_stroke(r, cr, border, StrokeKind::Inside);
    let title_color = if n.enabled { palette.text } else { palette.text_dim };
    p.text(
        pos2(r.left() + 18.0 * z, head.center().y),
        Align2::LEFT_CENTER,
        n.kind.title(),
        FontId::proportional((11.0 * z).clamp(7.0, 16.0)),
        title_color,
    );
    if let Some(tex) = thumb {
        let t = Rect::from_min_size(
            pos2(r.left() + 4.0 * z, r.bottom() - (THUMB_H - 2.0) * z),
            vec2(r.width() - 8.0 * z, (THUMB_H - 6.0) * z),
        );
        p.image(tex, t, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    // ports
    for i in 0..n.kind.inputs() {
        p.circle_filled(in_port(r, i, z), PORT_R * z, palette.text_dim);
    }
    if n.kind != NodeKind::Output {
        p.circle_filled(out_port(r, z), PORT_R * z, palette.text_dim);
    }
    // non-effect nodes have no inline widgets; show a one-line summary instead
    let summary = match &n.kind {
        NodeKind::Blend { mode, opacity } => Some(format!("{:?}  {:.0}%", mode, opacity.value * 100.0)),
        NodeKind::Combine { mode, factor } => Some(format!("{:?}  {:.0}%", mode, factor.value * 100.0)),
        NodeKind::Merge => Some("b over a".into()),
        NodeKind::Asset(id) => Some(format!("#{id}")),
        NodeKind::Matte { invert, use_alpha } => {
            Some(format!("{}{}", if *use_alpha { "alpha" } else { "luma" }, if *invert { " inv" } else { "" }))
        }
        NodeKind::Color(c) => Some(format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])),
        NodeKind::Clip(id) => Some(if *id == 0 { "(pick a clip)".into() } else { format!("#{id}") }),
        _ => None,
    };
    if let Some(s) = summary {
        p.text(
            pos2(r.left() + 8.0 * z, r.top() + (HEADER_H + ROW_H * 0.5) * z),
            Align2::LEFT_CENTER,
            s,
            TextStyle::Small.resolve(ui.style()),
            palette.text_dim,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, ClipKind};
    use egui::{Event, Modifiers, PointerButton, RawInput, Vec2};

    struct Harness {
        ctx: egui::Context,
        state: NodesState,
        project: Project,
        selection: Vec<Id>,
        base: egui::Id,
        undos: usize,
        time: f64,
        out: NodesResponse,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            let aid = project.add_asset(Asset {
                id: 0,
                path: "C:/x.mp4".into(),
                kind: ClipKind::Video,
                duration: 10.0,
                width: 1280,
                height: 720,
                fps: 30.0,
                audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
                codec: "h264".into(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
            });
            project.insert_asset_clips(aid, 0.0, None);
            let clip = project.tracks[0].clips[0].id;
            let mut h = Self {
                ctx: egui::Context::default(),
                state: NodesState::default(),
                project,
                selection: vec![clip],
                base: egui::Id::NULL,
                undos: 0,
                time: 0.0,
                out: NodesResponse::default(),
            };
            h.frame(vec![]);
            h
        }
        fn clip(&self) -> Id {
            self.selection[0]
        }
        fn graph(&self) -> &crate::model::NodeGraph {
            self.project.clip(self.clip()).and_then(|c| c.graph.as_ref()).expect("graph")
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
            let Harness { ctx, state, project, selection, base, undos, out, .. } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    *base = ui.id();
                    let mut undo = |_: &Project| *undos += 1;
                    *out = show(ui, state, project, selection, 0.0, &pal, &mut undo);
                });
            });
        }
        fn node_rect(&self, id: Id) -> Rect {
            self.ctx
                .read_response(self.base.with(("node", id)))
                .unwrap_or_else(|| panic!("node {id} not laid out"))
                .rect
        }
        fn press(&mut self, pos: Pos2, button: PointerButton) {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Event::PointerButton { pos, button, pressed: true, modifiers: Modifiers::NONE }]);
        }
        fn release(&mut self, pos: Pos2, button: PointerButton) {
            self.frame(vec![Event::PointerButton { pos, button, pressed: false, modifiers: Modifiers::NONE }]);
            self.frame(vec![]);
        }
        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.press(from, PointerButton::Primary);
            for i in 1..=4 {
                self.frame(vec![Event::PointerMoved(from + (to - from) * (i as f32 / 4.0))]);
            }
            self.release(to, PointerButton::Primary);
        }
        /// Right-click at `pos`, then click the menu row with the given salt. The menu Area needs a
        /// couple of frames to settle into its final position before its rows can be hit.
        fn menu_click<S: std::hash::Hash + std::fmt::Debug>(&mut self, pos: Pos2, salt: S) {
            self.press(pos, PointerButton::Secondary);
            self.release(pos, PointerButton::Secondary);
            let id = self.base.with(&salt);
            let mut r = Pos2::ZERO;
            for _ in 0..4 {
                self.frame(vec![]);
                r = self.ctx.read_response(id).unwrap_or_else(|| panic!("menu row {salt:?} missing")).rect.center();
                self.frame(vec![Event::PointerMoved(r)]);
            }
            self.press(r, PointerButton::Primary);
            self.release(r, PointerButton::Primary);
        }
        fn ids(&self) -> (Id, Id) {
            let g = self.graph();
            (g.nodes[0].id, g.output().expect("output"))
        }
    }

    #[test]
    fn opening_the_pane_converts_the_effect_stack() {
        let mut h = Harness::new();
        assert!(h.project.clip(h.clip()).unwrap().graph.is_some(), "ensure_graph ran");
        let (input, output) = h.ids();
        assert_eq!(h.graph().nodes.len(), 2);
        assert_eq!(h.graph().eval_order(), vec![input, output]);
        h.frame(vec![]);
        assert_eq!(h.graph().nodes.len(), 2, "idempotent");
    }

    #[test]
    fn no_selection_shows_a_hint_instead_of_panicking() {
        let mut h = Harness::new();
        h.selection.clear();
        h.frame(vec![]);
        assert_eq!(h.out.selected, None);
        assert!(!h.out.edited);
    }

    #[test]
    fn adding_an_effect_node_inserts_it() {
        let mut h = Harness::new();
        h.menu_click(pos2(600.0, 140.0), ("add", "Blur"));
        let g = h.graph();
        assert_eq!(g.nodes.len(), 3, "{:?}", g.nodes.iter().map(|n| n.kind.title()).collect::<Vec<_>>());
        let added = g.nodes.iter().find(|n| n.kind.title() == "Blur").expect("blur node");
        assert!(matches!(&added.kind, NodeKind::Effect(e) if e.kind == EffectKind::Blur));
        assert_eq!(h.out.selected, Some(added.id), "a new node becomes the selection");
        assert!(h.undos >= 1, "adding a node is undoable");
    }

    #[test]
    fn input_effect_output_evaluates_in_order() {
        let mut h = Harness::new();
        let (input, output) = h.ids();
        h.menu_click(pos2(560.0, 200.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        // wire Input -> Blur -> Output by dragging between the ports
        let (ir, fr, orr) = (h.node_rect(input), h.node_rect(fx), h.node_rect(output));
        let z = h.state.zoom;
        h.drag(out_port(ir, z), in_port(fr, 0, z));
        let (fr, orr) = (h.node_rect(fx), h.node_rect(output));
        h.drag(out_port(fr, z), in_port(orr, 0, z));
        let g = h.graph();
        assert_eq!(g.input_of(fx, 0), Some(input), "edges: {:?}", g.edges);
        assert_eq!(g.input_of(output, 0), Some(fx), "edges: {:?}", g.edges);
        assert_eq!(g.eval_order(), vec![input, fx, output]);
    }

    #[test]
    fn a_cycle_is_refused() {
        let mut h = Harness::new();
        let (input, _) = h.ids();
        h.menu_click(pos2(420.0, 120.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        h.menu_click(pos2(660.0, 260.0), ("add", "Sharpen"));
        let fx2 = h.out.selected.expect("selected");
        let z = h.state.zoom;
        let (ir, fr) = (h.node_rect(input), h.node_rect(fx));
        h.drag(out_port(ir, z), in_port(fr, 0, z)); // Input -> Blur
        let (fr, fr2) = (h.node_rect(fx), h.node_rect(fx2));
        h.drag(out_port(fr, z), in_port(fr2, 0, z)); // Blur -> Sharpen
        assert_eq!(h.graph().input_of(fx2, 0), Some(fx), "edges: {:?}", h.graph().edges);
        let mut edges = h.graph().edges.clone();
        let undos = h.undos;
        let (fr, fr2) = (h.node_rect(fx), h.node_rect(fx2));
        h.drag(out_port(fr2, z), in_port(fr, 0, z)); // Sharpen -> Blur would loop
                                                     // `connect` restores the previous edge, so compare as a set
        let mut after = h.graph().edges.clone();
        let key = |e: &crate::model::Edge| (e.from, e.to, e.port);
        edges.sort_by_key(key);
        after.sort_by_key(key);
        assert_eq!(after, edges, "the cycle must be refused");
        assert!(!h.graph().has_cycle());
        assert_eq!(undos, h.undos, "a refused wire pushes no undo entry");
    }

    #[test]
    fn deleting_a_node_relinks_its_neighbour() {
        let mut h = Harness::new();
        let (input, output) = h.ids();
        h.menu_click(pos2(560.0, 200.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        let z = h.state.zoom;
        let (ir, fr) = (h.node_rect(input), h.node_rect(fx));
        h.drag(out_port(ir, z), in_port(fr, 0, z));
        let (fr, orr) = (h.node_rect(fx), h.node_rect(output));
        h.drag(out_port(fr, z), in_port(orr, 0, z));
        assert_eq!(h.graph().input_of(output, 0), Some(fx));
        let fr = h.node_rect(fx);
        h.menu_click(fr.center(), ("del", fx));
        let g = h.graph();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.input_of(output, 0), Some(input), "the neighbour was re-linked");
    }

    #[test]
    fn the_output_node_cannot_be_deleted() {
        let mut h = Harness::new();
        let (_, output) = h.ids();
        h.state.selected = Some(output);
        h.frame(vec![]);
        // no Delete row in the Output menu
        let orr = h.node_rect(output);
        h.press(orr.center(), PointerButton::Secondary);
        h.release(orr.center(), PointerButton::Secondary);
        assert!(h.ctx.read_response(h.base.with(("del", output))).is_none(), "Output offers no Delete");
        // and the keyboard shortcut is refused too
        h.frame(vec![Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(h.graph().output().is_some(), "Output survives");
        assert_eq!(h.graph().nodes.len(), 2);
    }

    #[test]
    fn dragging_a_node_moves_it_with_one_undo() {
        let mut h = Harness::new();
        let (input, _) = h.ids();
        let before = {
            let n = h.graph().node(input).unwrap();
            (n.x, n.y)
        };
        let start = h.node_rect(input).center();
        h.undos = 0;
        h.drag(start, start + vec2(60.0, 40.0));
        let after = {
            let n = h.graph().node(input).unwrap();
            (n.x, n.y)
        };
        assert!((after.0 - before.0 - 60.0).abs() < 1.0, "x {before:?} -> {after:?}");
        assert!((after.1 - before.1 - 40.0).abs() < 1.0, "y {before:?} -> {after:?}");
        assert_eq!(h.undos, 1, "exactly one undo snapshot per drag gesture");
        assert!(h.out.selected == Some(input));
    }

    #[test]
    fn pulling_a_wire_off_an_input_disconnects_it() {
        let mut h = Harness::new();
        let (input, output) = h.ids();
        assert_eq!(h.graph().input_of(output, 0), Some(input));
        let z = h.state.zoom;
        let port = in_port(h.node_rect(output), 0, z);
        h.drag(port, port + vec2(0.0, 160.0)); // drop on empty canvas
        assert_eq!(h.graph().input_of(output, 0), None, "the wire was pulled off");
    }

    #[test]
    fn converting_the_effect_stack_is_undoable() {
        // Harness::new draws exactly one frame, and that frame converts the clip
        let h = Harness::new();
        assert!(h.project.clip(h.clip()).unwrap().graph.is_some());
        assert_eq!(h.undos, 1, "the conversion is a snapshotted edit, not a silent side effect");
        assert!(h.out.edited, "and the app is told the project changed");
    }

    #[test]
    fn clicking_a_connected_input_port_changes_nothing() {
        let mut h = Harness::new();
        let (input, output) = h.ids();
        let z = h.state.zoom;
        let port = in_port(h.node_rect(output), 0, z);
        h.undos = 0;
        h.press(port, PointerButton::Primary);
        h.release(port, PointerButton::Primary);
        assert_eq!(h.graph().input_of(output, 0), Some(input), "the wire survives a plain click");
        assert_eq!(h.undos, 0, "detach + re-attach must not push two no-op undo entries");
    }

    #[test]
    fn typing_an_inline_parameter_snapshots_undo_first() {
        let mut h = Harness::new();
        h.menu_click(pos2(560.0, 200.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        let value = |h: &Harness| match &h.graph().node(fx).unwrap().kind {
            NodeKind::Effect(e) => e.at(0, 0.0),
            _ => panic!("not an effect"),
        };
        let before = value(&h);
        let r = h.node_rect(fx);
        let z = h.state.zoom;
        // the first inline parameter row: its DragValue fills the right-hand side of the row
        let start = pos2(r.left() + 62.0 * z, r.top() + (HEADER_H + ROW_H * 0.5) * z + 2.0);
        h.undos = 0;
        // click the DragValue to type into it (the frame that gains focus has no change to apply yet)
        h.press(start, PointerButton::Primary);
        h.release(start, PointerButton::Primary);
        h.frame(vec![Event::Text("9".into())]);
        h.frame(vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![]);
        assert!((value(&h) - before).abs() > 1e-9, "the typed parameter took: {before} -> {}", value(&h));
        assert_eq!(h.undos, 1, "the snapshot lands on the gesture's first frame, before the value changes");
    }

    #[test]
    fn ctrl_scroll_zooms_around_the_pointer() {
        let mut h = Harness::new();
        let anchor = pos2(400.0, 300.0);
        h.frame(vec![Event::PointerMoved(anchor)]);
        let before = h.state.zoom;
        h.frame(vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 100.0),
            modifiers: Modifiers::CTRL,
        }]);
        assert!(h.state.zoom > before, "zoom {} -> {}", before, h.state.zoom);
        assert!(h.state.zoom <= 3.0);
    }

    #[test]
    fn ctrl_d_duplicates_the_selected_node() {
        let mut h = Harness::new();
        h.menu_click(pos2(560.0, 200.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        let (x, y) = {
            let n = h.graph().node(fx).unwrap();
            (n.x, n.y)
        };
        h.frame(vec![Event::PointerMoved(pos2(800.0, 500.0))]);
        h.frame(vec![Event::Key {
            key: egui::Key::D,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }]);
        let g = h.graph();
        assert_eq!(g.nodes.len(), 4, "the duplicate was added");
        let copies: Vec<_> = g.nodes.iter().filter(|n| n.kind.title() == "Blur").collect();
        assert_eq!(copies.len(), 2);
        let dup = copies.iter().find(|n| n.id != fx).unwrap();
        assert!((dup.x - x - 24.0).abs() < 0.01 && (dup.y - y - 24.0).abs() < 0.01, "offset from the original");
    }

    #[test]
    fn the_header_checkbox_disables_a_node() {
        let mut h = Harness::new();
        h.menu_click(pos2(560.0, 200.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        assert!(h.graph().node(fx).unwrap().enabled);
        let cb = h.node_rect(fx).min + vec2(9.0, 11.0);
        h.undos = 0;
        h.press(cb, PointerButton::Primary);
        h.release(cb, PointerButton::Primary);
        let n = h.graph().node(fx).unwrap();
        assert!(!n.enabled, "the checkbox toggles the node off");
        assert!(matches!(&n.kind, NodeKind::Effect(e) if !e.enabled), "and the effect with it");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn apply_edits_parameters_and_masks() {
        let mut h = Harness::new();
        h.menu_click(pos2(560.0, 200.0), ("add", "Blur"));
        let fx = h.out.selected.expect("selected");
        let mut sel = Some(fx);
        let clip = h.clip();
        assert!(apply(&mut h.project, clip, Act::SetParam(fx, 0, 7.5), &mut sel));
        assert!(apply(&mut h.project, clip, Act::AddMask(fx), &mut sel));
        assert!(!apply(&mut h.project, clip, Act::AddMask(fx), &mut sel), "a second mask is a no-op");
        let n = h.graph().node(fx).unwrap();
        let NodeKind::Effect(e) = &n.kind else { panic!("not an effect") };
        assert_eq!(e.at(0, 0.0), 7.5);
        assert!(e.mask.is_some());
    }

    #[test]
    fn node_height_grows_with_a_thumbnail() {
        let plain = node_height(&NodeKind::Effect(Effect::new(EffectKind::Blur)), false);
        let with = node_height(&NodeKind::Effect(Effect::new(EffectKind::Blur)), true);
        assert!(with > plain + 40.0, "{plain} vs {with}");
        assert!(node_height(&NodeKind::Output, false) >= HEADER_H + ROW_H);
    }

    #[test]
    fn the_add_menu_lists_every_effect_kind_once() {
        let mut seen = Vec::new();
        let mut cats = Vec::new();
        for k in EffectKind::ALL {
            assert!(!seen.contains(&k.name()), "{} listed twice", k.name());
            seen.push(k.name());
            if !cats.contains(&k.category()) {
                cats.push(k.category());
            }
        }
        assert_eq!(seen.len(), EffectKind::ALL.len());
        assert!(cats.len() > 1, "the menu is grouped: {cats:?}");
    }
}
