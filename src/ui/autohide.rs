//! Overlay controls that get out of the way: the player-mode chrome (`crate::player`) and the
//! fullscreen Preview's transport. Pure state machine over explicit times, so it is testable and costs
//! nothing at rest:
//!
//! - the pointer leaves the video (or the window) -> the controls fade out at once (`FADE`)
//! - the pointer moves over the video -> they fade back in
//! - the pointer rests over the video for `IDLE` while playing -> they fade out, and the cursor hides
//! - `keep` (paused, or the pointer is over the controls) keeps them up while the pointer is inside;
//!   `hold` (a menu is open, the scrubber is being dragged) keeps them up even when it leaves
//!
//! `step` says when it next needs a frame (`repaint_in`); `None` means nothing changes until input
//! arrives - a paused player with hidden controls asks for no repaint at all.

/// Seconds the pointer may rest over playing video before the controls go.
pub const IDLE: f64 = 2.5;
/// Seconds a fade takes (in or out). Short: the controls are gone the moment the pointer is.
pub const FADE: f64 = 0.12;

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// Seconds on any monotonic clock.
    pub now: f64,
    /// The pointer is over the video area (and in the window).
    pub pointer_in: bool,
    /// The pointer moved (or a button went down) this frame.
    pub moved: bool,
    /// Keep the controls while the pointer is inside: paused, or hovering the controls themselves.
    pub keep: bool,
    /// Keep the controls even when the pointer is outside: an open menu, a drag in progress.
    pub hold: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    /// 0 = hidden, 1 = fully shown.
    pub opacity: f32,
    /// Seconds until the next frame this needs (a fade in progress, the idle timer); None = none.
    pub repaint_in: Option<f64>,
    /// Hide the mouse cursor (resting over playing video).
    pub hide_cursor: bool,
}

#[derive(Clone, Debug)]
pub struct AutoHide {
    shown: bool,
    last_move: f64,
    /// When the current fade started and the opacity it started from.
    fade_at: f64,
    fade_from: f32,
}

impl Default for AutoHide {
    /// Starts shown, as if the pointer just moved: a freshly opened player shows its controls once.
    fn default() -> Self {
        AutoHide { shown: true, last_move: f64::NEG_INFINITY, fade_at: f64::NEG_INFINITY, fade_from: 1.0 }
    }
}

impl AutoHide {
    /// Starts hidden (the editor's fullscreen Preview: the transport waits for a mouse move).
    pub fn hidden() -> Self {
        AutoHide { shown: false, last_move: f64::NEG_INFINITY, fade_at: f64::NEG_INFINITY, fade_from: 0.0 }
    }

    /// Show the controls now, as a mouse move would (a key press that changes something on them).
    pub fn poke(&mut self, now: f64) {
        self.last_move = now;
        self.set(true, now);
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    fn opacity(&self, now: f64) -> f32 {
        let k = ((now - self.fade_at) / FADE).clamp(0.0, 1.0) as f32;
        let to = if self.shown { 1.0 } else { 0.0 };
        self.fade_from + (to - self.fade_from) * k
    }

    fn set(&mut self, shown: bool, now: f64) {
        if shown != self.shown {
            self.fade_from = self.opacity(now);
            self.fade_at = now;
            self.shown = shown;
        }
    }

    pub fn step(&mut self, i: Input) -> Output {
        if i.pointer_in && i.moved {
            self.last_move = i.now;
        }
        if self.last_move == f64::NEG_INFINITY && self.shown {
            self.last_move = i.now; // the opening "just moved" of `default()`
        }
        let idle = i.now - self.last_move >= IDLE;
        let want = i.hold || (i.pointer_in && (i.keep || !idle));
        self.set(want, i.now);
        let opacity = self.opacity(i.now);
        let fading = i.now - self.fade_at < FADE;
        let repaint_in = if fading {
            Some(1.0 / 60.0)
        } else if want && !i.hold && i.pointer_in && !i.keep {
            Some((self.last_move + IDLE - i.now).max(0.0))
        } else {
            None
        };
        Output { opacity, repaint_in, hide_cursor: i.pointer_in && !want }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now: f64, pointer_in: bool, moved: bool) -> Input {
        Input { now, pointer_in, moved, ..Default::default() }
    }

    #[test]
    fn leaving_hides_at_once_and_moving_back_shows() {
        let mut h = AutoHide::default();
        assert_eq!(h.step(at(0.0, true, true)).opacity, 1.0);
        // the pointer leaves: hidden within one fade, no idle wait
        let o = h.step(at(0.5, false, false));
        assert!(!h.shown());
        assert!(o.opacity > 0.0 && o.repaint_in.is_some(), "fading out: {o:?}");
        let o = h.step(at(0.5 + FADE, false, false));
        assert_eq!(o.opacity, 0.0);
        assert_eq!(o.repaint_in, None, "hidden and outside: nothing to repaint");
        assert!(!o.hide_cursor, "the cursor is not ours to hide outside the video");
        // back in with a move: shown again
        h.step(at(3.0, true, true));
        assert!(h.shown());
        assert_eq!(h.step(at(3.0 + FADE, true, false)).opacity, 1.0);
    }

    #[test]
    fn idle_over_playing_video_hides_and_hides_the_cursor() {
        let mut h = AutoHide::default();
        let o = h.step(at(0.0, true, true));
        assert_eq!(o.repaint_in, Some(IDLE), "wakes exactly when the idle timer runs out");
        assert!(h.step(at(IDLE - 0.01, true, false)).opacity == 1.0);
        h.step(at(IDLE, true, false));
        let o = h.step(at(IDLE + FADE, true, false));
        assert_eq!(o.opacity, 0.0);
        assert!(o.hide_cursor);
        assert_eq!(o.repaint_in, None);
    }

    #[test]
    fn keep_and_hold() {
        // paused (keep): stays up over the video however long the pointer rests ...
        let mut h = AutoHide::default();
        let keep = |now, pointer_in| Input { now, pointer_in, keep: true, ..Default::default() };
        h.step(keep(0.0, true));
        let o = h.step(keep(60.0, true));
        assert_eq!((o.opacity, o.repaint_in), (1.0, None), "paused + shown: idle, no repaint");
        // ... but leaving still hides it at once
        h.step(keep(61.0, false));
        assert_eq!(h.step(keep(61.0 + FADE, false)).opacity, 0.0);
        // hold (an open menu) survives the pointer leaving
        let mut h = AutoHide::default();
        let o = h.step(Input { now: 0.0, pointer_in: false, hold: true, ..Default::default() });
        assert_eq!((o.opacity, o.repaint_in), (1.0, None));
    }

    #[test]
    fn hidden_start_waits_for_a_move_and_poke_shows() {
        let mut h = AutoHide::hidden();
        assert_eq!(h.step(at(0.0, true, false)).opacity, 0.0);
        h.poke(1.0);
        assert!(h.shown());
        assert_eq!(h.step(at(1.0 + FADE, true, false)).opacity, 1.0);
    }
}
