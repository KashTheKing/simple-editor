//! Node editor: the selected clip's effect chain as a graph you can wire up.
//!
//! Canvas with pan (middle-drag / scroll) and zoom (Ctrl+Scroll). Each node is a rounded box:
//! title (`NodeKind::title`), an enable checkbox, input ports on the left, one output port on the right,
//! and its parameters inline (collapsed to the first two when the node is small). Drag from a port to
//! another to connect (`NodeGraph::connect` refuses cycles); drag a wire off a port to disconnect;
//! right-click a node for Delete / Duplicate / Add mask; right-click the canvas for the "Add node" menu
//! (every `EffectKind` grouped by `category()`, Blend, Combine, Merge, Matte, Mask, Color, Clip, Text,
//! Input, then one entry per project asset — an Asset node needs a real id, so it is picked there).
//! Selecting a node shows its full parameters in the Inspector and lets the Curves pane keyframe them.
//! Ctrl+click adds/removes a node, a primary drag over empty canvas rubber-bands a group (Shift adds),
//! dragging any picked node moves the whole selection, Delete removes it and Ctrl+C / Ctrl+V duplicates
//! it (edges *between* the copied nodes come along; wires to the outside are dropped). Effects and
//! transitions dragged out of their panels drop in as fresh, unwired nodes.
//! A clip without a graph gets a "Build a node graph from it" button instead of one: `ensure_graph`
//! converts the linear stack, and from then on the renderer evaluates the graph and ignores
//! `clip.effects` — far too much to do to a clip just because this pane is docked somewhere.
//! Every mutation calls `undo` once per gesture.

use crate::model::{Animated, BlendMode, Edge, Effect, EffectKind, Id, Mask, Node, NodeKind, Project, TransitionKind};
use crate::theme::Palette;
use crate::ui::DragPayload;
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
/// Duplicates and pastes land this far from the original, so they are visibly separate.
const OFFSET: f32 = 24.0;

#[derive(Default)]
pub struct NodesState {
    /// Pan/zoom of the canvas.
    pub offset: egui::Vec2,
    pub zoom: f32,
    /// The picked nodes; the last one is what the Inspector follows (`primary`).
    pub selection: Vec<Id>,
    /// A connection being dragged: (node, port, from_output).
    pub linking: Option<(Id, usize, bool)>,
    /// The node under the pointer while moving (one undo per gesture; the selection travels with it).
    pub dragging: Option<Id>,
    /// Input port a wire is being pulled off, until the pointer actually leaves it.
    pub detach: Option<(Id, usize)>,
    /// Rubber band in progress: (press origin in screen space, add instead of replace).
    pub band: Option<(Pos2, bool)>,
    /// Ctrl+C copy: the nodes and the edges between them, pasted with fresh ids.
    pub clipboard: (Vec<Node>, Vec<Edge>),
    /// Where the canvas was right-clicked, in graph units: new nodes land there.
    pub menu_at: Option<(f32, f32)>,
    /// Live previews, keyed by node. Empty unless a renderer hands textures over, and the panel is
    /// fully usable without them (there is no GL context in tests or headless runs).
    pub thumbs: std::collections::HashMap<Id, egui::TextureId>,
}

impl NodesState {
    /// The node the Inspector shows: the most recently picked one.
    pub fn primary(&self) -> Option<Id> {
        self.selection.last().copied()
    }
    /// Pick a node; `add` (Ctrl) toggles it in the selection instead of replacing it.
    fn pick(&mut self, id: Id, add: bool) {
        match (add, self.selection.iter().position(|&s| s == id)) {
            (true, Some(i)) => {
                self.selection.remove(i);
            }
            (true, None) => self.selection.push(id),
            (false, _) => self.selection = vec![id],
        }
    }
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
    /// Drive a mask node's centre along a saved project path (node, path).
    SetPath(Id, Id),
    /// Re-create copied nodes with fresh ids, offset, plus the edges between them.
    Paste(Vec<Node>, Vec<Edge>),
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
    // renderer evaluates the graph and ignores clip.effects. So the user has to ask for it — merely
    // selecting a clip while this pane happens to be docked must not take its effect list away.
    if project.clip(clip_id).is_some_and(|c| c.graph.is_none()) {
        let mut make = false;
        let base_id = ui.id();
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.label(egui::RichText::new("This clip renders from its effect list.").color(palette.text_dim));
            let b = ui.button("Build a node graph from it");
            // click-sensing interact on top of the button: a stable id the headless test can find
            make = ui.interact(b.rect, base_id.with("build_graph"), Sense::click()).clicked();
        });
        if !make {
            return out;
        }
        undo(project);
        project.ensure_graph(clip_id);
        out.edited = true;
    }
    // the graph is small; a copy keeps the draw pass read-only so `undo` can snapshot mid-frame
    let Some(graph) = project.clip(clip_id).and_then(|c| c.graph.clone()) else {
        return out;
    };
    let lt = project.clip(clip_id).map(|c| c.local(playhead)).unwrap_or(0.0);
    state.selection.retain(|s| graph.node(*s).is_some());

    let rect = ui.available_rect_before_wrap();
    let base = ui.id();
    let resp = ui.allocate_rect(rect, Sense::click_and_drag());
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0, palette.bg);

    // ---- pan / zoom ----
    let mods = ui.input(|i| i.modifiers);
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
    // saved drawings / polygons: a mask node's centre can travel along one (the graph's position input)
    let paths: Vec<(Id, String)> = project.paths.iter().map(|p| (p.id, p.name.clone())).collect();

    // rubber band: hit-tested while the boxes are drawn, applied on release (mirrors the timeline)
    let band_rect = state.band.map(|(o, _)| Rect::from_two_pos(o, pointer.unwrap_or(o)).intersect(rect));
    let mut band_ids: Vec<Id> = Vec::new();

    // ---- wire dragging ----
    let (pressed, released, down) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down()));
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
        if band_rect.is_some_and(|br| br.intersects(r)) {
            band_ids.push(n.id);
        }
        let nr = ui.interact(r, base.with(("node", n.id)), Sense::click_and_drag());
        // grabbing one of several picked nodes keeps the group; anything else picks (Ctrl toggles)
        if nr.clicked() || (nr.drag_started() && !state.selection.contains(&n.id)) {
            state.pick(n.id, mods.ctrl);
        }
        if nr.drag_started_by(egui::PointerButton::Primary) && !linking {
            state.dragging = Some(n.id);
            needs_undo = true; // exactly one snapshot for the whole move
        }
        if state.dragging == Some(n.id) {
            if nr.dragged() {
                let d = nr.drag_delta() / z;
                if d != egui::Vec2::ZERO {
                    for m in graph.nodes.iter().filter(|m| m.id == n.id || state.selection.contains(&m.id)) {
                        acts.push(Act::Move(m.id, m.x + d.x, m.y + d.y));
                    }
                }
            }
            if nr.drag_stopped() {
                state.dragging = None;
            }
        }
        let selected = state.selection.contains(&n.id);
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
            if !state.selection.contains(&id) {
                state.selection = vec![id];
            }
            if !is_output && menu_entry(ui, base.with(("del", id)), "Delete").clicked() {
                acts.push(Act::Delete(id));
                needs_undo = true;
                ui.close();
            }
            if menu_entry(ui, base.with(("dup", id)), "Duplicate").clicked() {
                acts.push(Act::Add(kind.clone(), nx + OFFSET, ny + OFFSET));
                needs_undo = true;
                ui.close();
            }
            if matches!(&kind, NodeKind::Effect(_)) && menu_entry(ui, base.with(("mask", id)), "Add mask").clicked() {
                acts.push(Act::AddMask(id));
                needs_undo = true;
                ui.close();
            }
            if matches!(&kind, NodeKind::Mask(_)) {
                for (pid, name) in &paths {
                    if menu_entry(ui, base.with(("path", id, pid)), &format!("Move along: {name}")).clicked() {
                        acts.push(Act::SetPath(id, *pid));
                        needs_undo = true;
                        ui.close();
                    }
                }
            }
        });
    }

    // ---- canvas ----
    if resp.dragged_by(egui::PointerButton::Middle) {
        state.offset += resp.drag_delta();
    }
    // a primary drag that missed every node rubber-bands a group instead of panning (Shift adds)
    if state.band.is_none()
        && !linking
        && state.dragging.is_none()
        && resp.drag_started_by(egui::PointerButton::Primary)
    {
        if let Some(o) = resp.interact_pointer_pos().or(pointer) {
            state.band = Some((o, mods.shift));
        }
    }
    if let (Some(br), Some((_, add))) = (band_rect, state.band) {
        p.rect_filled(br, 0, palette.accent.gamma_multiply(0.15));
        p.rect_stroke(br, 0, Stroke::new(1.0, palette.accent), StrokeKind::Inside);
        if !down {
            if !add {
                state.selection.clear();
            }
            for id in band_ids {
                if !state.selection.contains(&id) {
                    state.selection.push(id);
                }
            }
            state.band = None;
        }
    }
    // an effect or transition dragged out of its panel lands here as a fresh, unwired node
    if let Some(payload) = resp.dnd_release_payload::<DragPayload>() {
        let g = to_graph(pointer.unwrap_or(rect.center()));
        let kind = match &*payload {
            DragPayload::Effect(k) => Some(NodeKind::Effect(Effect::new(*k))),
            DragPayload::Transition(k) => Some(transition_node(*k)),
            _ => None,
        };
        if let Some(kind) = kind {
            acts.push(Act::Add(kind, g.x, g.y));
            needs_undo = true;
        }
    }
    if resp.clicked() {
        state.selection.clear();
    }
    if resp.secondary_clicked() {
        let g = to_graph(pointer.unwrap_or(rect.center()));
        state.menu_at = Some((g.x, g.y));
    }
    let at = state.menu_at.unwrap_or((40.0, 40.0));
    let assets: Vec<(Id, String)> = project.assets.iter().map(|a| (a.id, a.name())).collect();
    resp.context_menu(|ui| add_menu(ui, base, at, &assets, &mut acts, &mut needs_undo));

    // Delete / Ctrl+D / Ctrl+C / Ctrl+V on the selection (the menu's actions, from the keyboard)
    if !ui.ctx().wants_keyboard_input() && pointer.is_some() {
        let (del, dup, copy, paste) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::D),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::C),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::V),
            )
        });
        // the Output is part of every graph: it is never deleted, duplicated or copied
        let sel: Vec<&Node> =
            state.selection.iter().filter_map(|s| graph.node(*s)).filter(|n| n.kind != NodeKind::Output).collect();
        for n in &sel {
            if del {
                acts.push(Act::Delete(n.id));
                needs_undo = true;
            }
            if dup {
                acts.push(Act::Add(n.kind.clone(), n.x + OFFSET, n.y + OFFSET));
                needs_undo = true;
            }
        }
        if copy && !sel.is_empty() {
            let ids: Vec<Id> = sel.iter().map(|n| n.id).collect();
            state.clipboard = (
                sel.iter().map(|n| (*n).clone()).collect(),
                graph.edges.iter().filter(|e| ids.contains(&e.from) && ids.contains(&e.to)).copied().collect(),
            );
        }
        if paste && !state.clipboard.0.is_empty() {
            acts.push(Act::Paste(state.clipboard.0.clone(), state.clipboard.1.clone()));
            needs_undo = true;
        }
    }

    // ---- apply ----
    // not gated on `acts`: a gesture's first frame (drag start / entering a text field) usually has no
    // act yet, and that is exactly the state the snapshot has to capture
    if needs_undo {
        undo(project);
    }
    for a in acts {
        out.edited |= apply(project, clip_id, a, &mut state.selection);
    }
    out.selected = state.primary();
    out
}

/// The nearest node to a transition: a cross fade / push / wipe is a two-input mix at any one instant,
/// and a dip to colour is that colour.
// ponytail: Combine holds the mix but not a wipe's geometry — give the graph a real Transition node if
// anyone needs to animate one there.
fn transition_node(kind: TransitionKind) -> NodeKind {
    match kind {
        TransitionKind::FadeToColor => NodeKind::Color([0, 0, 0, 255]),
        _ => NodeKind::Combine { mode: BlendMode::Normal, factor: Animated::new(0.5) },
    }
}

/// Apply one buffered action; returns true when the project actually changed.
fn apply(project: &mut Project, clip: Id, act: Act, selected: &mut Vec<Id>) -> bool {
    // reads the project before the graph is borrowed mutably
    if let Act::SetPath(node, path) = act {
        let Some(pts) = project.path(path).map(|p| p.points.clone()) else { return false };
        let dur = project.clip(clip).map_or(0.0, |c| c.duration);
        let (cx, cy) = crate::model::path_to_keys(&pts, dur);
        if cx.keys.len() < 2 {
            return false;
        }
        let Some(n) = project.clip_mut(clip).and_then(|c| c.graph.as_mut()).and_then(|g| g.node_mut(node)) else {
            return false;
        };
        let NodeKind::Mask(m) = &mut n.kind else { return false };
        m.cx = cx;
        m.cy = cy;
        return true;
    }
    if let Act::Add(kind, x, y) = act {
        let id = project.add_node(clip, kind, x, y);
        if let Some(id) = id {
            *selected = vec![id];
        }
        return id.is_some();
    }
    // fresh ids for the copies, then the edges that had both ends inside the copied set
    if let Act::Paste(nodes, edges) = act {
        let mut map: Vec<(Id, Id)> = Vec::with_capacity(nodes.len());
        for n in &nodes {
            if let Some(id) = project.add_node(clip, n.kind.clone(), n.x + OFFSET, n.y + OFFSET) {
                map.push((n.id, id));
            }
        }
        let Some(g) = project.clip_mut(clip).and_then(|c| c.graph.as_mut()) else { return false };
        let new = |old: Id| map.iter().find(|(o, _)| *o == old).map(|(_, n)| *n);
        for e in &edges {
            if let (Some(from), Some(to)) = (new(e.from), new(e.to)) {
                g.connect(from, to, e.port);
            }
        }
        *selected = map.iter().map(|(_, n)| *n).collect();
        return !map.is_empty();
    }
    let Some(g) = project.clip_mut(clip).and_then(|c| c.graph.as_mut()) else { return false };
    match act {
        Act::Add(..) | Act::SetPath(..) | Act::Paste(..) => false,
        Act::Delete(id) => {
            if g.node(id).is_none_or(|n| n.kind == NodeKind::Output) {
                return false;
            }
            g.remove_node(id);
            selected.retain(|s| *s != id);
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
            project.ensure_graph(clip); // the pane no longer builds one just by being open
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
            self.frame_m(events, Modifiers::NONE);
        }
        fn frame_m(&mut self, events: Vec<Event>, modifiers: Modifiers) {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
                time: Some(self.time),
                events,
                modifiers,
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
            self.press_m(pos, button, Modifiers::NONE);
        }
        fn press_m(&mut self, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
            self.frame_m(vec![Event::PointerMoved(pos)], modifiers);
            self.frame_m(vec![Event::PointerButton { pos, button, pressed: true, modifiers }], modifiers);
        }
        fn release(&mut self, pos: Pos2, button: PointerButton) {
            self.release_m(pos, button, Modifiers::NONE);
        }
        fn release_m(&mut self, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
            self.frame_m(vec![Event::PointerButton { pos, button, pressed: false, modifiers }], modifiers);
            self.frame_m(vec![], modifiers);
        }
        /// Click a node's title bar (its middle is an inline parameter widget) — Ctrl to add it to /
        /// drop it from the selection.
        fn click_node(&mut self, id: Id, modifiers: Modifiers) {
            let r = self.node_rect(id);
            let c = pos2(r.center().x, r.top() + HEADER_H * 0.5 * self.state.zoom);
            self.press_m(c, PointerButton::Primary, modifiers);
            self.release_m(c, PointerButton::Primary, modifiers);
        }
        fn key(&mut self, key: egui::Key, modifiers: Modifiers) {
            self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
        }
        fn pos_of(&self, id: Id) -> (f32, f32) {
            let n = self.graph().node(id).expect("node");
            (n.x, n.y)
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

    /// The pane is opt-in: pointing it at a clip that has no graph offers a button and leaves the clip's
    /// effect list alone until it is pressed. Otherwise a Nodes pane sitting in the layout silently took
    /// over every clip the user selected, and effects could never be put on footage directly again.
    #[test]
    fn a_clip_without_a_graph_is_left_alone_until_asked() {
        let mut h = Harness::new();
        let id = h.clip();
        let c = h.project.clip_mut(id).unwrap();
        c.graph = None;
        c.effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.project.clip(id).unwrap().graph.is_none(), "drawing the pane is not an edit");
        assert_eq!(h.undos, 0);
        assert!(!h.out.edited);
        assert_eq!(h.project.clip(id).unwrap().effects.len(), 1, "and the effect list is untouched");
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
        h.state.selection = vec![output];
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
        let mut h = Harness::new();
        let id = h.clip();
        h.project.clip_mut(id).unwrap().graph = None;
        h.frame(vec![]);
        let pos = h.ctx.read_response(h.base.with("build_graph")).expect("build button").rect.center();
        h.press(pos, PointerButton::Primary);
        // the release frame is the one that reports the edit; release() would draw another and clear it
        h.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(h.project.clip(id).unwrap().graph.is_some());
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
        let mut sel = vec![fx];
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

    /// Two effect nodes, wired Input -> a -> b, neither selected afterwards.
    fn two_nodes(h: &mut Harness) -> (Id, Id) {
        h.menu_click(pos2(420.0, 160.0), ("add", "Blur"));
        let a = h.out.selected.expect("selected");
        h.menu_click(pos2(660.0, 320.0), ("add", "Sharpen"));
        let b = h.out.selected.expect("selected");
        let (input, _) = h.ids();
        let z = h.state.zoom;
        let (ir, ar) = (h.node_rect(input), h.node_rect(a));
        h.drag(out_port(ir, z), in_port(ar, 0, z));
        let (ar, br) = (h.node_rect(a), h.node_rect(b));
        h.drag(out_port(ar, z), in_port(br, 0, z));
        (a, b)
    }

    #[test]
    fn ctrl_click_picks_several_nodes_and_they_move_and_delete_together() {
        let mut h = Harness::new();
        let (a, b) = two_nodes(&mut h);
        h.click_node(a, Modifiers::NONE);
        h.click_node(b, Modifiers::CTRL);
        assert_eq!(h.state.selection, vec![a, b], "Ctrl+click added the second node");
        let (before_a, before_b) = (h.pos_of(a), h.pos_of(b));
        h.undos = 0;
        let start = h.node_rect(a).center();
        h.drag(start, start + vec2(50.0, 30.0));
        let (after_a, after_b) = (h.pos_of(a), h.pos_of(b));
        for (before, after) in [(before_a, after_a), (before_b, after_b)] {
            assert!((after.0 - before.0 - 50.0).abs() < 1.0, "x {before:?} -> {after:?}");
            assert!((after.1 - before.1 - 30.0).abs() < 1.0, "y {before:?} -> {after:?}");
        }
        assert_eq!(h.undos, 1, "one snapshot for the whole group move");
        // Ctrl+click again drops a node from the selection, then Delete takes the rest
        h.click_node(a, Modifiers::CTRL);
        assert_eq!(h.state.selection, vec![b]);
        h.click_node(a, Modifiers::CTRL);
        h.frame(vec![Event::PointerMoved(pos2(800.0, 520.0))]);
        h.key(egui::Key::Delete, Modifiers::NONE);
        assert_eq!(h.graph().nodes.len(), 2, "both picked nodes were deleted");
        assert!(h.state.selection.is_empty());
    }

    #[test]
    fn a_rubber_band_picks_every_node_it_covers() {
        let mut h = Harness::new();
        let (input, output) = h.ids();
        // press on empty canvas below the two nodes and drag up across both
        h.drag(pos2(700.0, 500.0), pos2(10.0, 12.0));
        assert_eq!(h.state.selection, vec![input, output]);
        assert!(h.state.band.is_none(), "band cleared on release");
        // a band over empty space replaces the selection with nothing
        h.drag(pos2(700.0, 500.0), pos2(500.0, 300.0));
        assert!(h.state.selection.is_empty(), "{:?}", h.state.selection);
    }

    #[test]
    fn copy_paste_clones_the_selection_with_the_edges_inside_it() {
        let mut h = Harness::new();
        let (a, b) = two_nodes(&mut h);
        let (input, _) = h.ids();
        h.click_node(a, Modifiers::NONE);
        h.click_node(b, Modifiers::CTRL);
        h.frame(vec![Event::PointerMoved(pos2(800.0, 520.0))]);
        h.key(egui::Key::C, Modifiers::CTRL);
        h.undos = 0;
        h.key(egui::Key::V, Modifiers::CTRL);
        assert_eq!(h.graph().nodes.len(), 6, "two copies joined the four nodes");
        assert_eq!(h.undos, 1, "one snapshot for the paste");
        let copies = h.state.selection.clone();
        assert_eq!(copies.len(), 2, "the copies are what stays selected");
        assert!(!copies.contains(&a) && !copies.contains(&b), "fresh ids: {copies:?}");
        let g = h.graph();
        assert_eq!(g.input_of(copies[1], 0), Some(copies[0]), "the edge inside the copy came along");
        assert_eq!(g.input_of(copies[0], 0), None, "the edge from Input did not");
        assert_eq!(g.input_of(b, 0), Some(a), "the originals are untouched");
        assert_eq!(g.input_of(a, 0), Some(input));
        let (orig, copy) = (h.pos_of(a), h.pos_of(copies[0]));
        assert!((copy.0 - orig.0 - OFFSET).abs() < 0.01 && (copy.1 - orig.1 - OFFSET).abs() < 0.01, "offset");
    }

    #[test]
    fn dropping_an_effect_or_a_transition_makes_a_node_where_it_landed() {
        let mut h = Harness::new();
        // the drag starts outside the canvas (in the effects panel), so only the release lands here
        let drop = |h: &mut Harness, payload: DragPayload, at: Pos2| {
            h.press(pos2(2.0, 2.0), PointerButton::Primary);
            egui::DragAndDrop::set_payload(&h.ctx, payload);
            h.frame(vec![Event::PointerMoved(at)]);
            h.release(at, PointerButton::Primary);
        };
        let at = pos2(600.0, 400.0);
        drop(&mut h, DragPayload::Effect(EffectKind::Pixelate), at);
        let g = h.graph();
        assert_eq!(g.nodes.len(), 3, "{:?}", g.nodes.iter().map(|n| n.kind.title()).collect::<Vec<_>>());
        let n = g.nodes.iter().find(|n| n.kind.title() == "Pixelate").expect("effect node");
        assert!(g.edges.iter().all(|e| e.from != n.id && e.to != n.id), "wired to nothing");
        // dropped at the pointer: graph units = (screen - origin) / zoom, and the canvas starts unpanned
        let origin = h.ctx.read_response(h.base.with(("node", g.nodes[0].id))).expect("input node").rect.min;
        assert!((n.x - (at.x - origin.x)).abs() < 1.0 && (n.y - (at.y - origin.y)).abs() < 1.0, "at {},{}", n.x, n.y);
        assert!(h.undos >= 1, "a dropped node is undoable");
        drop(&mut h, DragPayload::Transition(TransitionKind::CrossFade), pos2(300.0, 450.0));
        assert!(h.graph().nodes.iter().any(|n| n.kind.title() == "Combine"), "a cross fade drops in as a mix");
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
