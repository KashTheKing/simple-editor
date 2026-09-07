//! Frozen modifier table (`plans/ui-overhaul/README.md`, "Decided modifier table (timeline gestures)"):
//! a pure lookup from (press-time modifiers, hit zone, track flags, active tool) to what a drag (or a
//! zone-specific click) should become. Every table row is a unit test in this file's own `#[cfg(test)]`
//! module. Most `GestureKind` results are registry-protocol stubs this wave — tested and compiled, but
//! not yet wired into a live drag: timeline-trim-gestures (wave 2) wires Edge's Ctrl/Alt/Shift/Ctrl+Alt
//! rows and Drop's four rows; pro-timeline (wave 3) wires Seam's Shift (asymmetric multi-roller) row.
//! This workstream wires only Body's Shift-click selection fix (done directly in mod.rs's click-handling
//! block, not through `arm()` — that block edits *selection*, not a drag `GestureKind`), BodyBottom's
//! hairline/click-split, Seam's plain/Ctrl/Alt click → `EditPoint`, RulerInOut's drag, and Lane's
//! middle-mouse pan (also direct — Pan is button-driven, not modifier-driven, so it is not a real `arm()`
//! output despite being in the `GestureKind` enum for documentation completeness).
#![allow(dead_code)] // registry-protocol stub: most rows are tested here but consumed in wave 2/3
use crate::ui::tools::Tool;
use eframe::egui::Modifiers;

/// Where a press landed, before any modifier is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Body,
    BodyBottom,
    EdgeStart,
    EdgeEnd,
    Seam,
    Fade,
    VolumeLine,
    Key,
    Marker,
    TransitionEdge,
    Lane,
    RulerInOut,
    Drop,
    LegacyTool,
}

/// The three per-track flags the table branches on (`Track.locked`/`ripple`/`magnetic`, landed by
/// registries-schema-hooks). `arm.rs` has no `Project` dependency of its own — the caller reads the
/// track and builds this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackFlags {
    pub locked: bool,
    pub ripple: bool,
    pub magnetic: bool,
}

/// What a gesture (or zone-specific click) resolves to. Click-only outcomes (`SplitAt`, `SeamBoth`,
/// `SeamLeft`, `SeamRight`, `SeamAddToSet`, `GapSelect`) and drag outcomes share one enum because a zone
/// determines which kind of interaction it is (BodyBottom = click, Edge = drag, etc.) — `arm()` itself
/// stays a flat lookup, not a state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    MoveNoOverlap,
    MagneticMove,
    Slip,
    Slide,
    Segment,
    Trim,
    RippleTrim,
    Roll,
    RateStretch,
    MultiRippleTrim,
    SplitAt,
    SeamBoth,
    SeamLeft,
    SeamRight,
    SeamAddToSet,
    GapSelect,
    RubberBandAdd,
    Pan,
    RulerInOutDrag,
    DropSplice,
    DropOverwrite,
    DropPlaceOnTop,
    DropDefault,
    LegacyToolGesture,
}

/// Pure lookup over the frozen modifier table. `mods` is the press-time (or, for a zone-specific click,
/// click-time) `egui::Modifiers`. Returns `None` for a (zone, modifier) combination the table leaves
/// undefined — the caller keeps its own existing/unaffected behaviour in that case (see the Fade /
/// VolumeLine / Key / Marker / TransitionEdge zones below, whose row says "existing gestures unchanged").
pub fn arm(mods: Modifiers, zone: Zone, flags: TrackFlags, tool: Tool) -> Option<GestureKind> {
    // Any non-Select tool keeps its own legacy gesture regardless of zone/modifiers: no new tools ship
    // (Tool::Zoom stays deleted), Cut/Marker/Stretch/Spacer/Draw/Text/Shape/Mask are untouched.
    if !matches!(tool, Tool::Select) {
        return Some(GestureKind::LegacyToolGesture);
    }
    use GestureKind::*;
    let (ctrl, alt, shift) = (mods.ctrl, mods.alt, mods.shift);
    match zone {
        Zone::Body => match (ctrl, alt, shift) {
            (true, true, false) => Some(Slide),
            (true, false, true) => Some(Segment),
            (true, false, false) => Some(MoveNoOverlap),
            (false, true, false) => Some(Slip),
            (false, false, true) => Some(MoveNoOverlap),
            (false, false, false) => Some(if flags.magnetic { MagneticMove } else { MoveNoOverlap }),
            _ => None, // undefined combo (e.g. Ctrl+Alt+Shift)
        },
        Zone::BodyBottom => (!ctrl && !alt && !shift).then_some(SplitAt),
        Zone::EdgeStart | Zone::EdgeEnd => match (ctrl, alt, shift) {
            (false, false, false) => Some(if flags.magnetic { RippleTrim } else { Trim }),
            (true, false, false) => Some(RippleTrim),
            (false, true, false) => Some(Roll),
            (false, false, true) => Some(RateStretch),
            (true, true, false) => Some(MultiRippleTrim),
            _ => None,
        },
        Zone::Seam => match (ctrl, alt, shift) {
            (false, false, false) => Some(SeamBoth),
            (false, false, true) => Some(SeamAddToSet),
            (true, false, false) => Some(SeamLeft),
            (false, true, false) => Some(SeamRight),
            _ => None,
        },
        // "existing gestures unchanged" rows: arm() abstains, the caller keeps its own Fade/Volume/
        // Keys/Marker/TransDur gesture handling untouched.
        Zone::Fade | Zone::VolumeLine | Zone::Key | Zone::Marker | Zone::TransitionEdge => None,
        Zone::Lane => match (ctrl, alt, shift) {
            (false, false, false) => Some(GapSelect),
            (false, false, true) => Some(RubberBandAdd),
            _ => None,
        },
        Zone::RulerInOut => (!ctrl && !alt && !shift).then_some(RulerInOutDrag),
        Zone::Drop => match (ctrl, alt, shift) {
            (false, false, false) => Some(DropDefault),
            (true, false, false) => Some(DropSplice),
            (false, true, false) => Some(DropOverwrite),
            (false, false, true) => Some(DropPlaceOnTop),
            _ => None,
        },
        Zone::LegacyTool => Some(LegacyToolGesture),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const CTRL: Modifiers = Modifiers::CTRL;
    const ALT: Modifiers = Modifiers::ALT;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CTRL_ALT: Modifiers = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    const CTRL_SHIFT: Modifiers = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    const FLAT: TrackFlags = TrackFlags { locked: false, ripple: false, magnetic: false };
    const MAGNETIC: TrackFlags = TrackFlags { locked: false, ripple: false, magnetic: true };

    // ---- Body ----
    #[test]
    fn body_none_plain_track_moves() {
        assert_eq!(arm(NONE, Zone::Body, FLAT, Tool::Select), Some(GestureKind::MoveNoOverlap));
    }
    #[test]
    fn body_none_magnetic_track_moves_magnetically() {
        assert_eq!(arm(NONE, Zone::Body, MAGNETIC, Tool::Select), Some(GestureKind::MagneticMove));
    }
    #[test]
    fn body_ctrl_moves_toggled_selection() {
        assert_eq!(arm(CTRL, Zone::Body, FLAT, Tool::Select), Some(GestureKind::MoveNoOverlap));
    }
    #[test]
    fn body_alt_slips() {
        assert_eq!(arm(ALT, Zone::Body, FLAT, Tool::Select), Some(GestureKind::Slip));
    }
    #[test]
    fn body_shift_moves() {
        assert_eq!(arm(SHIFT, Zone::Body, FLAT, Tool::Select), Some(GestureKind::MoveNoOverlap));
    }
    #[test]
    fn body_ctrl_alt_slides() {
        assert_eq!(arm(CTRL_ALT, Zone::Body, FLAT, Tool::Select), Some(GestureKind::Slide));
    }
    #[test]
    fn body_ctrl_shift_segments() {
        assert_eq!(arm(CTRL_SHIFT, Zone::Body, FLAT, Tool::Select), Some(GestureKind::Segment));
    }

    // ---- BodyBottom ----
    #[test]
    fn body_bottom_none_splits() {
        assert_eq!(arm(NONE, Zone::BodyBottom, FLAT, Tool::Select), Some(GestureKind::SplitAt));
    }
    #[test]
    fn body_bottom_ctrl_is_undefined() {
        assert_eq!(arm(CTRL, Zone::BodyBottom, FLAT, Tool::Select), None);
    }

    // ---- Edge (start and end share the row) ----
    #[test]
    fn edge_none_plain_track_trims() {
        assert_eq!(arm(NONE, Zone::EdgeStart, FLAT, Tool::Select), Some(GestureKind::Trim));
        assert_eq!(arm(NONE, Zone::EdgeEnd, FLAT, Tool::Select), Some(GestureKind::Trim));
    }
    #[test]
    fn edge_none_magnetic_track_ripple_trims() {
        assert_eq!(arm(NONE, Zone::EdgeStart, MAGNETIC, Tool::Select), Some(GestureKind::RippleTrim));
    }
    #[test]
    fn edge_ctrl_ripple_trims() {
        assert_eq!(arm(CTRL, Zone::EdgeEnd, FLAT, Tool::Select), Some(GestureKind::RippleTrim));
    }
    #[test]
    fn edge_alt_rolls() {
        assert_eq!(arm(ALT, Zone::EdgeStart, FLAT, Tool::Select), Some(GestureKind::Roll));
    }
    #[test]
    fn edge_shift_rate_stretches() {
        assert_eq!(arm(SHIFT, Zone::EdgeEnd, FLAT, Tool::Select), Some(GestureKind::RateStretch));
    }
    #[test]
    fn edge_ctrl_alt_multi_ripple_trims() {
        assert_eq!(arm(CTRL_ALT, Zone::EdgeStart, FLAT, Tool::Select), Some(GestureKind::MultiRippleTrim));
    }

    // ---- Seam ----
    #[test]
    fn seam_none_selects_both() {
        assert_eq!(arm(NONE, Zone::Seam, FLAT, Tool::Select), Some(GestureKind::SeamBoth));
    }
    #[test]
    fn seam_shift_adds_to_set() {
        assert_eq!(arm(SHIFT, Zone::Seam, FLAT, Tool::Select), Some(GestureKind::SeamAddToSet));
    }
    #[test]
    fn seam_ctrl_selects_left() {
        assert_eq!(arm(CTRL, Zone::Seam, FLAT, Tool::Select), Some(GestureKind::SeamLeft));
    }
    #[test]
    fn seam_alt_selects_right() {
        assert_eq!(arm(ALT, Zone::Seam, FLAT, Tool::Select), Some(GestureKind::SeamRight));
    }

    // ---- "existing gestures unchanged" zones: arm() abstains ----
    #[test]
    fn fade_volume_key_marker_transition_zones_are_unclassified() {
        for z in [Zone::Fade, Zone::VolumeLine, Zone::Key, Zone::Marker, Zone::TransitionEdge] {
            assert_eq!(arm(NONE, z, FLAT, Tool::Select), None, "{z:?} must stay untouched by arm()");
            assert_eq!(arm(CTRL, z, FLAT, Tool::Select), None, "{z:?} must stay untouched by arm()");
        }
    }

    // ---- Lane ----
    #[test]
    fn lane_none_gap_selects() {
        assert_eq!(arm(NONE, Zone::Lane, FLAT, Tool::Select), Some(GestureKind::GapSelect));
    }
    #[test]
    fn lane_shift_adds_to_rubber_band() {
        assert_eq!(arm(SHIFT, Zone::Lane, FLAT, Tool::Select), Some(GestureKind::RubberBandAdd));
    }

    // ---- RulerInOut ----
    #[test]
    fn ruler_in_out_none_drags() {
        assert_eq!(arm(NONE, Zone::RulerInOut, FLAT, Tool::Select), Some(GestureKind::RulerInOutDrag));
    }

    // ---- Drop ----
    #[test]
    fn drop_none_places_default() {
        assert_eq!(arm(NONE, Zone::Drop, FLAT, Tool::Select), Some(GestureKind::DropDefault));
    }
    #[test]
    fn drop_ctrl_splices() {
        assert_eq!(arm(CTRL, Zone::Drop, FLAT, Tool::Select), Some(GestureKind::DropSplice));
    }
    #[test]
    fn drop_alt_overwrites() {
        assert_eq!(arm(ALT, Zone::Drop, FLAT, Tool::Select), Some(GestureKind::DropOverwrite));
    }
    #[test]
    fn drop_shift_places_on_top() {
        assert_eq!(arm(SHIFT, Zone::Drop, FLAT, Tool::Select), Some(GestureKind::DropPlaceOnTop));
    }

    // ---- legacy tools: any zone, any modifiers, unchanged ----
    #[test]
    fn non_select_tool_always_keeps_its_legacy_gesture() {
        for z in [Zone::Body, Zone::EdgeStart, Zone::Lane, Zone::Seam, Zone::Drop] {
            assert_eq!(arm(NONE, z, FLAT, Tool::Cut), Some(GestureKind::LegacyToolGesture));
            assert_eq!(arm(CTRL_ALT, z, FLAT, Tool::Spacer), Some(GestureKind::LegacyToolGesture));
        }
    }
}
