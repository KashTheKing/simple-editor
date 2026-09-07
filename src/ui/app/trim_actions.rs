//! ---- ws:trim-model ----
//! Keyboard-only dispatch for edit-point selection, trim ±1/±10 frames, extend/top/tail, slip, mark
//! clip, go to in/out, splice/overwrite/lift/extract at playhead, join/duplicate/unnest/replace,
//! select forward/backward/at-playhead, prev/next keyframe, and the placeholder track-flag toggles.
//! No mouse gestures, no new panes/glyphs — purely `App::act` arms reading existing
//! selection/playhead/`App::edit_point` state and calling the trim-model model ops directly.

use super::*;
use crate::model::ops::tracks::TrackFlag;
use crate::model::ops::trim::{EditPoint, Side};

/// Anything `commit` can read as "did the op actually do something" — a refused op (locked track,
/// no room, nothing selected) must push no undo entry.
trait Changed {
    fn changed(&self) -> bool;
}
impl Changed for bool {
    fn changed(&self) -> bool {
        *self
    }
}
impl<T> Changed for Vec<T> {
    fn changed(&self) -> bool {
        !self.is_empty()
    }
}
impl<T> Changed for Option<T> {
    fn changed(&self) -> bool {
        self.is_some()
    }
}

/// Snapshot before running `f`, and push one labelled undo entry (+ `after_edit`) iff it changed
/// anything — the shared shape every mutating arm below uses, mirroring `actions.rs`'s own
/// snapshot/push-if-changed idiom (see e.g. its `NudgeLeft | NudgeRight` arm) so History reads e.g.
/// "Ripple trim" instead of the generic "Project edited", and a refused op pushes nothing.
fn commit<T: Changed>(app: &mut App, label: &'static str, f: impl FnOnce(&mut Project) -> T) -> T {
    let before = app.project.to_json();
    let out = f(&mut app.project);
    if out.changed() {
        app.push_undo_labeled(before, label);
        app.after_edit();
    }
    out
}

/// `[` / `]` / `Ctrl+[` / `Ctrl+]`: nudge the current edit point by `frames` (signed) frame durations
/// — implemented as `extend_edit` to `edit_point.t + delta` rather than duplicating its Both(roll) vs
/// Left/Right(ripple_trim) dispatch, and follows the edit point to its new position on success so
/// repeated presses keep nudging the same cut. ponytail: with no edit point selected (no `U` yet) this
/// is a no-op rather than guessing which edge of the current selection to trim — press `U` first.
fn nudge_edit_point(app: &mut App, label: &'static str, frames: f64) -> bool {
    let Some(ep) = app.edit_point else { return false };
    let to = (ep.t + frames * app.project.frame_dur()).max(0.0);
    let changed = commit(app, label, |p| p.extend_edit(&ep, to));
    if changed {
        app.edit_point = Some(EditPoint { t: to, ..ep });
    }
    changed
}

/// `Q` / `W`: ripple-trim the start/end edge of the clip under the playhead (or first selected) to
/// the playhead, rippling iff its track is magnetic — matches the plain-edge-drag rule everywhere
/// else (only a magnetic track's plain edits ripple by default).
fn trim_to_playhead(app: &mut App, label: &'static str, start_edge: bool) -> bool {
    let Some(id) = app.selection.first().copied().or_else(|| app.project.clips_at(app.playhead).first().copied())
    else {
        return false;
    };
    let Some(ti) = app.project.track_of(id) else { return false };
    let ripple = app.project.tracks[ti].magnetic;
    let to = app.playhead;
    commit(app, label, |p| p.ripple_trim(id, start_edge, to, ripple))
}

/// `Alt+,` / `Alt+.`: slip the selection by one frame.
fn do_slip(app: &mut App, label: &'static str, left: bool) -> bool {
    let ids = app.project.expand_links(&app.selection);
    if ids.is_empty() {
        return false;
    }
    let d = if left { -app.project.frame_dur() } else { app.project.frame_dur() };
    commit(app, label, |p| p.slip(&ids, d))
}

/// `Alt+←` / `Alt+→`: seek to the previous/next keyframe of the first selected clip (any property).
fn keyframe_nav(app: &mut App, backward: bool) -> bool {
    let Some(&id) = app.selection.first() else { return false };
    let Some(c) = app.project.clip(id) else { return false };
    let lt = app.playhead - c.start;
    let start = c.start;
    let times = c.key_times();
    let target = if backward {
        times.iter().rev().find(|&&t| t < lt - 1e-6).copied()
    } else {
        times.iter().find(|&&t| t > lt + 1e-6).copied()
    };
    let Some(t) = target else { return false };
    app.seek(start + t);
    true
}

/// Placeholder target for the three unbound track-flag toggles until timeline-trim-gestures (wave 2)
/// gives them a real header-glyph target: the first selected clip's track.
fn toggle_track_flag(app: &mut App, flag: TrackFlag, label: &'static str) -> bool {
    let Some(&id) = app.selection.first() else { return false };
    let Some(ti) = app.project.track_of(id) else { return false };
    let cur = match flag {
        TrackFlag::Locked => app.project.tracks[ti].locked,
        TrackFlag::Ripple => app.project.tracks[ti].ripple.unwrap_or(false),
        TrackFlag::Magnetic => app.project.tracks[ti].magnetic,
    };
    commit(app, label, |p| p.set_track_flag(ti, flag, !cur))
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    use Action::*;
    match a {
        SelectEditPoint => {
            // ponytail: no mouse hover state exists headlessly — the first selected clip's track
            // stands in for "hovered track" (the timeline UI hands a real hovered track once
            // snap-engine/timeline-trim-gestures wire the gesture up).
            let track = app.selection.first().and_then(|&id| app.project.track_of(id));
            app.edit_point = app.project.nearest_edit_point(app.playhead, track);
            true
        }
        CycleEditSide => {
            if let Some(ep) = &mut app.edit_point {
                ep.side = match ep.side {
                    Side::Both => Side::Left,
                    Side::Left => Side::Right,
                    Side::Right => Side::Both,
                };
            }
            true
        }
        TrimLeft1 => {
            nudge_edit_point(app, "Trim edit -1 frame", -1.0);
            true
        }
        TrimRight1 => {
            nudge_edit_point(app, "Trim edit +1 frame", 1.0);
            true
        }
        TrimLeft10 => {
            nudge_edit_point(app, "Trim edit -10 frames", -10.0);
            true
        }
        TrimRight10 => {
            nudge_edit_point(app, "Trim edit +10 frames", 10.0);
            true
        }
        ExtendEdit => {
            if let Some(ep) = app.edit_point {
                let to = app.playhead;
                if commit(app, "Extend edit", |p| p.extend_edit(&ep, to)) {
                    app.edit_point = Some(EditPoint { t: to, ..ep });
                }
            }
            true
        }
        TrimTop => {
            trim_to_playhead(app, "Trim start", true);
            true
        }
        TrimTail => {
            trim_to_playhead(app, "Trim end", false);
            true
        }
        SlipLeft => {
            do_slip(app, "Slip", true);
            true
        }
        SlipRight => {
            do_slip(app, "Slip", false);
            true
        }
        MarkClip => {
            let clip = app.selection.first().copied();
            let ph = app.playhead;
            commit(app, "Mark clip", |p| p.mark_from_clip(clip, ph));
            true
        }
        GoToIn => {
            if let Some(t) = app.project.in_point {
                app.seek(t);
            }
            true
        }
        GoToOut => {
            if let Some(t) = app.project.out_point {
                app.seek(t);
            }
            true
        }
        JoinThroughEdit => {
            if let Some(id) =
                app.selection.first().copied().or_else(|| app.project.clips_at(app.playhead).first().copied())
            {
                commit(app, "Join through edit", |p| p.join_through(id));
            }
            true
        }
        DuplicateClips => {
            if !app.selection.is_empty() {
                let ids = app.selection.clone();
                let new_ids = commit(app, "Duplicate clips", |p| p.duplicate(&ids));
                if !new_ids.is_empty() {
                    app.selection = new_ids;
                }
            }
            true
        }
        SelectForward => {
            app.selection = app.project.clips_from(app.playhead, None, false);
            true
        }
        SelectBackward => {
            app.selection = app.project.clips_from(app.playhead, None, true);
            true
        }
        SelectAtPlayhead => {
            app.selection = app.project.clips_at(app.playhead);
            true
        }
        PrevKeyframe => {
            keyframe_nav(app, true);
            true
        }
        NextKeyframe => {
            keyframe_nav(app, false);
            true
        }
        SpliceInsert | OverwriteAtPlayhead => {
            // ponytail: uses the last-selected library asset, not real three-point source marks —
            // upgrade path: source-monitor (wave 2) supplies real src_in/src_out.
            let Some(asset) = app.library.selected else {
                app.toast("Select a library asset first");
                return true;
            };
            let at = app.playhead;
            let label = if a == SpliceInsert { "Splice" } else { "Overwrite" };
            let ids = if a == SpliceInsert {
                commit(app, label, |p| p.splice_in(asset, at, None, None))
            } else {
                commit(app, label, |p| p.overwrite_asset(asset, at, None, None))
            };
            if !ids.is_empty() {
                app.selection = ids;
            }
            true
        }
        LiftInOut | ExtractInOut => {
            let (Some(a0), Some(b0)) = (app.project.in_point, app.project.out_point) else {
                app.toast("Set In/Out first");
                return true;
            };
            if b0 > a0 {
                let label = if a == LiftInOut { "Lift" } else { "Extract" };
                if a == LiftInOut {
                    commit(app, label, |p| p.lift_range(a0, b0, None));
                } else {
                    commit(app, label, |p| p.extract_range(a0, b0, None));
                }
            }
            true
        }
        CloseGapAtPlayhead => {
            // ponytail: no gap-selection gesture exists yet — the last track the pointer pressed on
            // stands in until snap-engine/timeline-trim-gestures' GapSelect state calls
            // `close_gap_at` directly.
            if let Some(ti) = app.timeline.last_track {
                let t = app.playhead;
                commit(app, "Close gap", |p| p.close_gap_at(ti, t));
            }
            true
        }
        UnnestClip => {
            if let Some(&id) = app.selection.first() {
                let ids = commit(app, "Un-nest sequence", |p| p.unnest(id));
                if !ids.is_empty() {
                    app.selection = ids;
                }
            }
            true
        }
        ReplaceWithLibrarySelection => {
            if let (Some(&id), Some(asset)) = (app.selection.first(), app.library.selected) {
                commit(app, "Replace clip", |p| p.replace_clip(id, asset));
            }
            true
        }
        ToggleTrackLock => {
            toggle_track_flag(app, TrackFlag::Locked, "Lock track");
            true
        }
        ToggleTrackRipple => {
            toggle_track_flag(app, TrackFlag::Ripple, "Toggle track ripple");
            true
        }
        ToggleTrackMagnetic => {
            toggle_track_flag(app, TrackFlag::Magnetic, "Toggle track magnetic");
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `commit`'s undo-push decision reads `Changed`; pin every impl it relies on so a refused op
    /// (false / empty Vec / None) is guaranteed to push nothing, and a real one always does.
    #[test]
    fn changed_trait_matches_every_op_return_shape() {
        assert!(true.changed());
        assert!(!false.changed());
        assert!(!Vec::<u64>::new().changed());
        assert!(vec![1u64].changed());
        assert!(!Option::<u64>::None.changed());
        assert!(Some(1u64).changed());
    }

    /// `App::push_undo_labeled`/`after_edit` need a live `App` (see `tools_registry_tests.rs`'s doc
    /// comment on why one isn't buildable in a test), so this pins the structural property instead:
    /// every mutating arm above goes through the shared `commit` helper, never `push_undo_labeled`
    /// directly — the thing that actually makes "snapshot only if changed" hold for all 20+ arms
    /// instead of relying on each one to get its own snapshot/push pair right.
    #[test]
    fn every_mutation_goes_through_commit() {
        let src = include_str!("trim_actions.rs");
        let fn_start = src.find("pub(super) fn act(").expect("act() must exist");
        let commit_fn_start = src.find("fn commit<T: Changed>").expect("commit() must exist");
        // bounded to act()'s own body — the trailing #[cfg(test)] mod below (this very test file,
        // included via include_str!) mentions "push_undo_labeled" in doc comments/assertions, which
        // would otherwise false-positive this scan.
        let test_mod_start = src.find("#[cfg(test)]").expect("this test module must exist");
        let body = &src[fn_start..test_mod_start];
        // commit() itself is defined earlier in the file and legitimately calls push_undo_labeled —
        // only act()'s own arms (this slice) must never call it directly.
        assert!(fn_start > commit_fn_start, "commit() must be defined before act()");
        assert!(
            !body.contains("push_undo_labeled"),
            "act()'s own match arms must call the shared commit() helper, not push_undo_labeled directly"
        );
    }
}
