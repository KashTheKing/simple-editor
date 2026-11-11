use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------- transitions ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum TransitionKind {
    /// Dissolve A → B.
    CrossFade,
    /// A → colour → B (dip to black/white/…).
    FadeToColor,
    /// B pushes A out (direction).
    Push,
    /// B wipes over A (direction).
    Wipe,
    /// A GLSL transition (`Transition.shader`, the gl-transitions format in `library/README.md`).
    /// Not in `ALL`: it is meaningless without a source, so it comes from the library / a tool.
    Shader,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 4] =
        [TransitionKind::CrossFade, TransitionKind::FadeToColor, TransitionKind::Push, TransitionKind::Wipe];
    pub fn name(self) -> &'static str {
        match self {
            TransitionKind::CrossFade => "Cross Fade",
            TransitionKind::FadeToColor => "Fade to Color",
            TransitionKind::Push => "Push",
            TransitionKind::Wipe => "Wipe",
            TransitionKind::Shader => "Shader",
        }
    }
    pub fn has_direction(self) -> bool {
        matches!(self, TransitionKind::Push | TransitionKind::Wipe)
    }
}

/// Where a transition sits. `Cut` (the default) is centred on the cut between a clip and its left
/// neighbour; `In` / `Out` sit inside a single clip's first/last `duration` seconds and blend
/// from/to nothing (black), so they need no neighbour.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
pub enum TransitionEdge {
    #[default]
    Cut,
    In,
    Out,
}

/// A transition on a cut or a clip edge. For `Cut` it is centred on the cut between `right` and its
/// left neighbour, spanning [right.start - duration/2, right.start + duration/2); both clips are
/// extended virtually into that window (the engine clamps source times). For `In` / `Out`, `right`
/// is the single clip it belongs to and the window is its first/last `duration` seconds.
/// Audio tracks use CrossFade (a gain crossfade / fade).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transition {
    pub id: Id,
    /// The clip on the right side of the cut (`Cut`), or the clip the edge transition belongs to.
    pub right: Id,
    pub kind: TransitionKind,
    pub duration: f64,
    #[serde(default = "black")]
    pub color: [u8; 4],
    /// 0 = left, 1 = right, 2 = up, 3 = down (Push / Wipe).
    #[serde(default)]
    pub direction: u8,
    #[serde(default)]
    pub ease: Ease,
    #[serde(default)]
    pub edge: TransitionEdge,
    /// `TransitionKind::Shader` only: the GLSL source (its `// @name` names the transition).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shader: String,
    /// `TransitionKind::Shader` only: the `// @param` control values, in header order (header
    /// defaults for missing ones).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<f64>,
}

fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}

impl Transition {
    /// What the timeline, the Inspector and tools call it: a shader's `// @name`, else the kind's name.
    pub fn name(&self) -> String {
        if self.kind == TransitionKind::Shader {
            let n = crate::engine::shaderlib::parse_meta(&self.shader).name;
            if !n.is_empty() {
                return n;
            }
        }
        self.kind.name().to_string()
    }
    /// Make this a shader transition running `src`, keeping control values by index and growing /
    /// trimming them to the header's controls (new ones at their defaults).
    pub fn set_shader(&mut self, src: &str) {
        self.kind = TransitionKind::Shader;
        self.shader = src.to_string();
        let meta = crate::engine::shaderlib::parse_meta(src);
        self.params.truncate(meta.params.len());
        for p in meta.params.iter().skip(self.params.len()) {
            self.params.push(p.default);
        }
    }
    /// The shader's control values `p0..p7`: stored ones, header defaults for the rest.
    pub fn shader_params(&self) -> [f32; 8] {
        let meta = crate::engine::shaderlib::parse_meta(&self.shader);
        let mut out = [0.0f32; 8];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.params.get(i).copied().or_else(|| meta.params.get(i).map(|p| p.default)).unwrap_or(0.0) as f32;
        }
        out
    }
    /// Half the transition length, clamped to the clips it joins: an over-long transition must not
    /// reach past either neighbour (it would hide the clips beside them).
    pub fn half(&self, left: &Clip, right: &Clip) -> f64 {
        (self.duration / 2.0).min(left.duration).min(right.duration)
    }
    /// Eased progress 0..1 across a window of `cut ± half`.
    pub fn progress_at(&self, cut: f64, half: f64, t: f64) -> f64 {
        if half <= 0.0 {
            return 1.0;
        }
        self.ease.apply(((t - (cut - half)) / (2.0 * half)).clamp(0.0, 1.0))
    }
    /// (centre, half-width) of the clamped window for any placement; the window is `centre ± half`.
    /// `Cut` needs both clips; `In` / `Out` need only their own (the window stays inside the clip).
    pub fn cut_half(&self, left: Option<&Clip>, right: Option<&Clip>) -> Option<(f64, f64)> {
        match self.edge {
            TransitionEdge::Cut => {
                let (l, r) = (left?, right?);
                Some((r.start, self.half(l, r)))
            }
            TransitionEdge::In => {
                let c = right?;
                let h = (self.duration / 2.0).min(c.duration / 2.0);
                Some((c.start + h, h))
            }
            TransitionEdge::Out => {
                let c = left?;
                let h = (self.duration / 2.0).min(c.duration / 2.0);
                Some((c.end() - h, h))
            }
        }
    }
}

#[cfg(test)]
mod shader_tests {
    use super::*;

    const SRC: &str = "// @name Noise Dissolve\n// @param grain 60 5 400\n// @param softness 0.1 0 0.5\n\
                       vec4 transition(vec2 uv) { return mix(getFromColor(uv), getToColor(uv), progress); }\n";

    /// A transition saved before shader transitions existed loads unchanged, a built-in one never
    /// writes the new fields, and a shader one round-trips with its source and control values.
    #[test]
    fn shader_transitions_round_trip_and_old_files_still_load() {
        let old = r#"{"id":3,"right":7,"kind":"Wipe","duration":1.0,"direction":2}"#;
        let t: Transition = serde_json::from_str(old).unwrap();
        assert_eq!((t.kind, t.direction, t.shader.as_str(), t.params.len()), (TransitionKind::Wipe, 2, "", 0));
        let back = serde_json::to_string(&t).unwrap();
        assert!(!back.contains("shader") && !back.contains("params"), "{back}");

        let mut s = t.clone();
        s.set_shader(SRC);
        assert_eq!((s.kind, s.name().as_str()), (TransitionKind::Shader, "Noise Dissolve"));
        assert_eq!(s.params, [60.0, 0.1], "the header defaults");
        s.params[0] = 120.0;
        let json = serde_json::to_string(&s).unwrap();
        let again: Transition = serde_json::from_str(&json).unwrap();
        assert_eq!(again, s);
        assert_eq!(&again.shader_params()[..3], &[120.0, 0.1, 0.0]);
        // a new source keeps values by index and trims to its own controls
        s.set_shader("// @param grain 10 1 50\nvec4 transition(vec2 uv) { return getToColor(uv); }");
        assert_eq!(s.params, [120.0]);
        assert_eq!(s.name(), "Shader", "no @name: the kind's name");
        assert!(!TransitionKind::ALL.contains(&TransitionKind::Shader), "no bare Shader card without a source");
    }
}
