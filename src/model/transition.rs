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
}

fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}

impl Transition {
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
