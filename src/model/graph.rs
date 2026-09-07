use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------- node graph ----------

/// Arithmetic for a `Math` node.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
pub enum MathOp {
    #[default]
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Pow,
    Mod,
}

impl MathOp {
    pub const ALL: [MathOp; 8] =
        [MathOp::Add, MathOp::Sub, MathOp::Mul, MathOp::Div, MathOp::Min, MathOp::Max, MathOp::Pow, MathOp::Mod];
    pub fn name(self) -> &'static str {
        match self {
            MathOp::Add => "Add",
            MathOp::Sub => "Subtract",
            MathOp::Mul => "Multiply",
            MathOp::Div => "Divide",
            MathOp::Min => "Min",
            MathOp::Max => "Max",
            MathOp::Pow => "Power",
            MathOp::Mod => "Modulo",
        }
    }
}

/// Comparison for a `Compare` node.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
pub enum CmpOp {
    #[default]
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl CmpOp {
    pub const ALL: [CmpOp; 6] = [CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge, CmpOp::Eq, CmpOp::Ne];
    pub fn name(self) -> &'static str {
        match self {
            CmpOp::Lt => "Less",
            CmpOp::Le => "Less or equal",
            CmpOp::Gt => "Greater",
            CmpOp::Ge => "Greater or equal",
            CmpOp::Eq => "Equal",
            CmpOp::Ne => "Not equal",
        }
    }
}

/// Boolean logic for a `Logic` node.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
pub enum LogicOp {
    #[default]
    And,
    Or,
    Not,
    Xor,
}

impl LogicOp {
    pub const ALL: [LogicOp; 4] = [LogicOp::And, LogicOp::Or, LogicOp::Not, LogicOp::Xor];
    pub fn name(self) -> &'static str {
        match self {
            LogicOp::And => "And",
            LogicOp::Or => "Or",
            LogicOp::Not => "Not",
            LogicOp::Xor => "Xor",
        }
    }
}

/// What a node does. `Input` is the clip's own decoded layer; `Output` is what the compositor draws.
///
/// Two kinds of wire run through a graph: pictures (one texture per node, evaluated on the GPU) and
/// **values** (one number per node, `NodeGraph::eval_values`). Value ports on a picture node - an
/// effect's parameter ports, a blend's amount - are what let the logic nodes drive the image.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeKind {
    /// The clip's decoded layer (or, in an adjustment clip, everything below it).
    Input,
    /// A solid colour (RGBA) filling the canvas.
    Color([u8; 4]),
    /// Another clip's layer at the same time (compositing across tracks).
    Clip(Id),
    /// A project asset sampled as a texture - footage that need not be on the timeline at all.
    Asset(Id),
    Effect(Effect),
    /// Combines two inputs (`a` under `b`) with a blend mode and opacity.
    Blend {
        mode: BlendMode,
        opacity: Animated,
    },
    /// Same two inputs, mixed in by `factor` (0..1) - "how much of `b`", not its opacity.
    Combine {
        mode: BlendMode,
        factor: Animated,
    },
    /// `b` composited straight over `a` (alpha over, no knobs).
    Merge,
    /// Uses `b`'s luminance (or alpha) as a matte for `a`.
    Matte {
        invert: bool,
        use_alpha: bool,
    },
    /// A standalone mask (matte generator).
    Mask(Mask),
    /// A string rasterised into the graph. `{frame}`, `{time}` and `{n}` expand at evaluation time,
    /// which is all a frame counter or a running clock needs (see `expand_text`). Called `Text` in
    /// projects written before it was renamed - the alias keeps those loading.
    #[serde(alias = "Text")]
    String(TextStyle),
    /// A keyframeable constant: the plain number input, and a grey card at its value as a picture.
    Number(Animated),
    /// A constant flag - 1.0 or 0.0 downstream.
    Bool(bool),
    /// Deterministic noise in `min..max`, hashed from (`seed`, frame): the same project always
    /// renders the same numbers, and every frame gets a different one. A constant is a `Number`.
    Random {
        seed: u32,
        min: f64,
        max: f64,
    },
    /// Arithmetic on two value inputs.
    Math(MathOp),
    /// Compares two value inputs: 1.0 when it holds, 0.0 when it does not.
    Compare(CmpOp),
    /// Boolean logic on two value inputs (`Not` reads only `a`); anything >= 0.5 counts as true.
    Logic(LogicOp),
    /// `cond ? a : b` - the switch. Works on values *and* on pictures.
    Select,
    Output,
}

impl NodeKind {
    /// How many inputs the node consumes. An effect takes its picture on port 0 and one optional
    /// value per parameter after it, so any number in the graph can drive any knob.
    pub fn inputs(&self) -> usize {
        match self {
            NodeKind::Input
            | NodeKind::Color(_)
            | NodeKind::Clip(_)
            | NodeKind::Asset(_)
            | NodeKind::Mask(_)
            | NodeKind::String(_)
            | NodeKind::Number(_)
            | NodeKind::Bool(_)
            | NodeKind::Random { .. } => 0,
            NodeKind::Output => 1,
            NodeKind::Effect(e) => 1 + e.specs().len(),
            NodeKind::Merge | NodeKind::Matte { .. } | NodeKind::Math(_) | NodeKind::Compare(_) => 2,
            NodeKind::Logic(op) => {
                if *op == LogicOp::Not {
                    1
                } else {
                    2
                }
            }
            NodeKind::Blend { .. } | NodeKind::Combine { .. } | NodeKind::Select => 3,
        }
    }
    /// What port `i` takes, for the node box and the tooltips. Empty when it has no name.
    pub fn port_label(&self, i: usize) -> &str {
        match self {
            NodeKind::Effect(e) => {
                if i == 0 {
                    "in"
                } else {
                    e.specs().get(i - 1).map(|s| s.name).unwrap_or("")
                }
            }
            NodeKind::Blend { .. } => ["a", "b", "opacity"][i.min(2)],
            NodeKind::Combine { .. } => ["a", "b", "factor"][i.min(2)],
            NodeKind::Merge => ["a", "b"][i.min(1)],
            NodeKind::Matte { .. } => ["a", "matte"][i.min(1)],
            NodeKind::Math(_) | NodeKind::Compare(_) | NodeKind::Logic(_) => ["a", "b"][i.min(1)],
            NodeKind::Select => ["cond", "a", "b"][i.min(2)],
            _ => "",
        }
    }
    /// Nodes whose output is a number rather than a picture (they still paint as a grey card, so a
    /// value can be used as a matte without a conversion node).
    pub fn is_value(&self) -> bool {
        matches!(
            self,
            NodeKind::Number(_)
                | NodeKind::Bool(_)
                | NodeKind::Random { .. }
                | NodeKind::Math(_)
                | NodeKind::Compare(_)
                | NodeKind::Logic(_)
        )
    }
    pub fn title(&self) -> String {
        match self {
            NodeKind::Input => "Input".into(),
            NodeKind::Color(_) => "Color".into(),
            NodeKind::Clip(_) => "Clip".into(),
            NodeKind::Asset(_) => "Asset".into(),
            NodeKind::Effect(e) => e.kind.name().into(),
            NodeKind::Blend { .. } => "Blend".into(),
            NodeKind::Combine { .. } => "Combine".into(),
            NodeKind::Merge => "Merge".into(),
            NodeKind::Matte { .. } => "Matte".into(),
            NodeKind::Mask(m) => format!("Mask ({})", m.shape.name()),
            NodeKind::String(_) => "String".into(),
            NodeKind::Number(_) => "Number".into(),
            NodeKind::Bool(_) => "Boolean".into(),
            NodeKind::Random { .. } => "Random".into(),
            NodeKind::Math(op) => op.name().into(),
            NodeKind::Compare(op) => op.name().into(),
            NodeKind::Logic(op) => op.name().into(),
            NodeKind::Select => "Select".into(),
            NodeKind::Output => "Output".into(),
        }
    }
}

/// splitmix64 folded to 0..1 - the `Random` node's whole implementation.
fn hash01(x: u64) -> f64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Expand a text node's format at time `t` (timeline seconds, `lt` clip-local): `{frame}` is the
/// timeline frame number, `{time}` the timeline clock, `{n}` the frames since the clip started - so a
/// counter is `{n}` and a clock is `{time}`. Anything else is left alone.
pub fn expand_text(fmt: &str, t: f64, lt: f64, fps: f64) -> String {
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    let count = |s: f64| (s.max(0.0) * fps).floor() as i64;
    let mut out = fmt.to_string();
    if out.contains("{frame}") {
        out = out.replace("{frame}", &count(t).to_string());
    }
    if out.contains("{n}") {
        out = out.replace("{n}", &count(lt).to_string());
    }
    if out.contains("{time}") {
        let s = t.max(0.0);
        let (h, m, sec, cs) = (s as i64 / 3600, (s as i64 / 60) % 60, s as i64 % 60, (s.fract() * 100.0) as i64);
        let clock = if h > 0 { format!("{h}:{m:02}:{sec:02}.{cs:02}") } else { format!("{m:02}:{sec:02}.{cs:02}") };
        out = out.replace("{time}", &clock);
    }
    out
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: Id,
    pub kind: NodeKind,
    /// Editor position (graph units).
    pub x: f32,
    pub y: f32,
    #[serde(default = "crate::model::tru")]
    pub enabled: bool,
}

/// `to`'s input `port` is fed by `from`'s output.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from: Id,
    pub to: Id,
    pub port: usize,
}

/// A clip's effect chain as a DAG. When present it replaces `Clip.effects` (kept as the simple linear
/// stack for clips that never opened the node editor). Always contains exactly one `Output`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct NodeGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl NodeGraph {
    /// Input -> Output, ready to have effects dropped in between.
    pub fn new(next_id: &mut impl FnMut() -> Id) -> Self {
        let (i, o) = (next_id(), next_id());
        Self {
            nodes: vec![
                Node { id: i, kind: NodeKind::Input, x: 0.0, y: 0.0, enabled: true },
                Node { id: o, kind: NodeKind::Output, x: 320.0, y: 0.0, enabled: true },
            ],
            edges: vec![Edge { from: i, to: o, port: 0 }],
        }
    }
    /// A graph equivalent to a linear effect stack (used when a clip's stack is converted).
    pub fn from_effects(effects: &[Effect], next_id: &mut impl FnMut() -> Id) -> Self {
        let mut g = Self::new(next_id);
        let out = g.output().unwrap();
        let mut prev = g.nodes[0].id;
        g.edges.clear();
        for (i, e) in effects.iter().enumerate() {
            let id = next_id();
            g.nodes.push(Node {
                id,
                kind: NodeKind::Effect(e.clone()),
                x: 160.0 * (i + 1) as f32,
                y: 0.0,
                enabled: e.enabled,
            });
            g.edges.push(Edge { from: prev, to: id, port: 0 });
            prev = id;
        }
        if let Some(o) = g.nodes.iter_mut().find(|n| n.id == out) {
            o.x = 160.0 * (effects.len() + 1) as f32;
        }
        g.edges.push(Edge { from: prev, to: out, port: 0 });
        g
    }
    /// The inverse of `from_effects`: the port-0 chain from Output back to Input as a linear stack.
    /// Err (with a reason for the toast) when the graph is not that shape - anything the output does
    /// not read is not part of the picture and is dropped without complaint.
    pub fn to_effects(&self) -> Result<Vec<Effect>, String> {
        let out = self.output().ok_or("it has no Output node")?;
        let mut chain = Vec::new();
        let mut spine = vec![out];
        let mut cur = self.input_of(out, 0);
        while let Some(id) = cur {
            let n = self.node(id).ok_or("a wire points at a missing node")?;
            spine.push(id);
            match &n.kind {
                NodeKind::Input => break,
                NodeKind::Effect(e) => {
                    let mut e = e.clone();
                    e.enabled &= n.enabled;
                    chain.push(e);
                    cur = self.input_of(id, 0);
                }
                k => return Err(format!("a {} node has no effect-stack equivalent", k.title())),
            }
        }
        if cur.is_none() {
            return Err("the chain does not start at the Input node".into());
        }
        if let Some(extra) = self.eval_order().into_iter().find(|id| !spine.contains(id)) {
            let name = self.node(extra).map(|n| n.kind.title()).unwrap_or_default();
            return Err(format!("the {name} node feeds a branch a flat effect list can't hold"));
        }
        chain.reverse();
        Ok(chain)
    }
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: Id) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    pub fn output(&self) -> Option<Id> {
        self.nodes.iter().find(|n| n.kind == NodeKind::Output).map(|n| n.id)
    }
    /// The node feeding `to`'s input `port`.
    pub fn input_of(&self, to: Id, port: usize) -> Option<Id> {
        self.edges.iter().find(|e| e.to == to && e.port == port).map(|e| e.from)
    }
    /// Connect (replacing whatever fed that port). Refused when it would create a cycle.
    pub fn connect(&mut self, from: Id, to: Id, port: usize) -> bool {
        if from == to || self.node(from).is_none() || self.node(to).is_none() {
            return false;
        }
        let prev: Vec<Edge> = self.edges.iter().filter(|e| e.to == to && e.port == port).copied().collect();
        self.edges.retain(|e| !(e.to == to && e.port == port));
        self.edges.push(Edge { from, to, port });
        if self.has_cycle() {
            self.edges.pop();
            self.edges.extend(prev);
            return false;
        }
        true
    }
    pub fn disconnect(&mut self, to: Id, port: usize) {
        self.edges.retain(|e| !(e.to == to && e.port == port));
    }
    /// Remove a node (the Output can never be removed) and re-link its first input to its consumers.
    pub fn remove_node(&mut self, id: Id) {
        if self.node(id).map(|n| n.kind == NodeKind::Output).unwrap_or(true) {
            return;
        }
        let up = self.input_of(id, 0);
        let down: Vec<(Id, usize)> = self.edges.iter().filter(|e| e.from == id).map(|e| (e.to, e.port)).collect();
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
        if let Some(up) = up {
            for (to, port) in down {
                self.connect(up, to, port);
            }
        }
    }
    /// Depth-first cycle check (the editor refuses connections that would loop).
    pub fn has_cycle(&self) -> bool {
        fn visit(g: &NodeGraph, id: Id, state: &mut std::collections::HashMap<Id, u8>) -> bool {
            match state.get(&id) {
                Some(1) => return true,
                Some(2) => return false,
                _ => {}
            }
            state.insert(id, 1);
            let ins: Vec<Id> = g.edges.iter().filter(|e| e.to == id).map(|e| e.from).collect();
            for i in ins {
                if visit(g, i, state) {
                    return true;
                }
            }
            state.insert(id, 2);
            false
        }
        let mut state = std::collections::HashMap::new();
        self.nodes.iter().any(|n| visit(self, n.id, &mut state))
    }
    /// Nodes reachable from the output, inputs before consumers.
    pub fn eval_order(&self) -> Vec<Id> {
        let Some(out) = self.output() else { return Vec::new() };
        let mut order = Vec::new();
        let mut seen = std::collections::HashSet::new();
        fn walk(g: &NodeGraph, id: Id, seen: &mut std::collections::HashSet<Id>, order: &mut Vec<Id>) {
            if !seen.insert(id) {
                return;
            }
            let mut ins: Vec<(usize, Id)> = g.edges.iter().filter(|e| e.to == id).map(|e| (e.port, e.from)).collect();
            ins.sort();
            for (_, from) in ins {
                walk(g, from, seen, order);
            }
            order.push(id);
        }
        walk(self, out, &mut seen, &mut order);
        order
    }
    /// The scalar half of the graph at clip-local time `lt`: one number per node, in evaluation
    /// order, so a `Math`/`Compare`/`Select` chain resolves in a single pass. Picture-only nodes
    /// evaluate to 0. GL-free on purpose - the renderer calls it once per frame and it is what the
    /// tests exercise.
    pub fn eval_values(&self, lt: f64, fps: f64) -> std::collections::HashMap<Id, f64> {
        let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
        let frame = (lt.max(0.0) * fps).floor() as u64;
        let mut vals: std::collections::HashMap<Id, f64> = std::collections::HashMap::new();
        for id in self.eval_order() {
            let Some(n) = self.node(id) else { continue };
            let port = |p: usize| self.input_of(id, p).and_then(|f| vals.get(&f).copied()).unwrap_or(0.0);
            let (a, b) = (port(0), port(1));
            // a disabled node passes its first input through, exactly like the picture side
            let v = if !n.enabled {
                a
            } else {
                match &n.kind {
                    NodeKind::Number(x) => x.at(lt),
                    NodeKind::Bool(x) => *x as u8 as f64,
                    NodeKind::Random { seed, min, max } => min + (max - min) * hash01((*seed as u64) << 32 ^ frame),
                    NodeKind::Math(op) => match op {
                        MathOp::Add => a + b,
                        MathOp::Sub => a - b,
                        MathOp::Mul => a * b,
                        MathOp::Div => {
                            if b == 0.0 {
                                0.0
                            } else {
                                a / b
                            }
                        }
                        MathOp::Min => a.min(b),
                        MathOp::Max => a.max(b),
                        MathOp::Pow => a.powf(b),
                        MathOp::Mod => {
                            if b == 0.0 {
                                0.0
                            } else {
                                a.rem_euclid(b)
                            }
                        }
                    },
                    NodeKind::Compare(op) => {
                        let t = match op {
                            CmpOp::Lt => a < b,
                            CmpOp::Le => a <= b,
                            CmpOp::Gt => a > b,
                            CmpOp::Ge => a >= b,
                            CmpOp::Eq => (a - b).abs() < 1e-9,
                            CmpOp::Ne => (a - b).abs() >= 1e-9,
                        };
                        t as u8 as f64
                    }
                    NodeKind::Logic(op) => {
                        let (x, y) = (a >= 0.5, b >= 0.5);
                        let t = match op {
                            LogicOp::And => x && y,
                            LogicOp::Or => x || y,
                            LogicOp::Not => !x,
                            LogicOp::Xor => x != y,
                        };
                        t as u8 as f64
                    }
                    NodeKind::Select => {
                        if a >= 0.5 {
                            b
                        } else {
                            port(2)
                        }
                    }
                    NodeKind::Blend { opacity, .. } => opacity.at(lt),
                    NodeKind::Combine { factor, .. } => factor.at(lt),
                    _ => 0.0,
                }
            };
            vals.insert(id, if v.is_finite() { v } else { 0.0 });
        }
        vals
    }
    /// Every animated property in the graph (effect params, blend opacity, numbers, mask properties).
    pub fn animated_mut(&mut self) -> Vec<&mut Animated> {
        let mut v = Vec::new();
        for n in &mut self.nodes {
            match &mut n.kind {
                NodeKind::Effect(e) => {
                    v.extend(e.params.iter_mut());
                    if let Some(m) = &mut e.mask {
                        v.extend(m.animated_mut());
                    }
                }
                NodeKind::Blend { opacity, .. } => v.push(opacity),
                NodeKind::Combine { factor, .. } => v.push(factor),
                NodeKind::Number(a) => v.push(a),
                NodeKind::Mask(m) => v.extend(m.animated_mut()),
                _ => {}
            }
        }
        v
    }
    pub fn animated(&self) -> Vec<&Animated> {
        let mut v = Vec::new();
        for n in &self.nodes {
            match &n.kind {
                NodeKind::Effect(e) => {
                    v.extend(e.params.iter());
                    if let Some(m) = &e.mask {
                        v.extend(m.animated());
                    }
                }
                NodeKind::Blend { opacity, .. } => v.push(opacity),
                NodeKind::Combine { factor, .. } => v.push(factor),
                NodeKind::Number(a) => v.push(a),
                NodeKind::Mask(m) => v.extend(m.animated()),
                _ => {}
            }
        }
        v
    }
}
