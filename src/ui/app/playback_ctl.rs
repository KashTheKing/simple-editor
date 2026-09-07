//! ---- ws:player-rate-loop ----
//! JKL shuttle ladder, Loop In->Out, Play In->Out/Around/To-Out, Fast Review and Step+-10 (ACT_HANDLER:
//! `act`), plus the play-range auto-stop and audio-scrub-on-paused-playhead-change hook (FRAME_HOOK:
//! `tick`, filling the pre-placed `playback_tick` hook). All nine actions land on `Player`'s new
//! rate/loop/step/scrub API (`src/playback.rs`) — nothing here touches the render/audio threads
//! directly.

use super::*;

/// Pure JKL ladder step: 0 (stopped) or a direction switch -> unit rate; same-direction repeat doubles
/// the magnitude, capped at 8. `rate` is the CURRENT rate (0.0 when not playing — `act` passes that).
pub(super) fn shuttle_rate(rate: f64, back: bool) -> f64 {
    let same_dir = (rate < 0.0) == back && rate != 0.0;
    let mag = if same_dir { (rate.abs() * 2.0).min(8.0) } else { 1.0 };
    if back {
        -mag
    } else {
        mag
    }
}

/// Start time and auto-stop target for Play In->Out / Play Around Playhead / Play to Out. In/out
/// default to 0/duration when unset; Play Around uses one `preroll` seconds symmetrically.
pub(super) fn play_range_target(
    mode: &str,
    in_point: Option<f64>,
    out_point: Option<f64>,
    playhead: f64,
    duration: f64,
    preroll: f64,
) -> (f64, f64) {
    match mode {
        "in_out" => (in_point.unwrap_or(0.0), out_point.unwrap_or(duration)),
        "around" => ((playhead - preroll).max(0.0), (playhead + preroll).min(duration)),
        _ /* "to_out" */ => (playhead, out_point.unwrap_or(duration)),
    }
}

/// Has playback reached (or passed) the play-range auto-stop target?
fn stop_reached(t: f64, target: f64) -> bool {
    t >= target
}

/// Should `tick` emit a paused-playhead-change scrub this frame?
fn should_scrub(enabled: bool, playing: bool, last_t: f64, cur_t: f64) -> bool {
    enabled && !playing && (cur_t - last_t).abs() > 1e-9
}

/// Seek to the range's start, force normal forward playback, and arm `app.play_stop_at` — shared by
/// the three Play* actions below and the `playback.play_range` MCP tool.
pub(super) fn start_play_range(app: &mut App, mode: &str) {
    let (start, stop) = play_range_target(
        mode,
        app.project.in_point,
        app.project.out_point,
        app.playhead,
        app.project.duration(),
        app.settings.preroll_secs as f64,
    );
    app.seek(start);
    if app.player.is_playing() {
        app.player.set_rate(1.0);
    } else {
        app.player.play();
    }
    app.play_stop_at = Some(stop);
}

fn step_frames(app: &mut App, frames: i64) {
    app.player.pause();
    let t = app.project.snap_frame(app.playhead + frames as f64 * app.project.frame_dur());
    app.seek(t);
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ShuttleBack | Action::ShuttleFwd => {
            let back = a == Action::ShuttleBack;
            let cur = if app.player.is_playing() { app.player.rate() } else { 0.0 };
            let new_rate = shuttle_rate(cur, back);
            if !app.player.is_playing() {
                app.player.play();
            }
            app.player.set_rate(new_rate);
            true
        }
        Action::LoopInOut => {
            if app.player.loop_range().is_some() {
                app.player.set_loop(None);
            } else {
                match (app.project.in_point, app.project.out_point) {
                    (Some(a), Some(b)) if b > a => app.player.set_loop(Some((a, b))),
                    _ => app.toast("Mark In and Out first (I / O)"),
                }
            }
            true
        }
        Action::PlayInOut => {
            start_play_range(app, "in_out");
            true
        }
        Action::PlayAround => {
            start_play_range(app, "around");
            true
        }
        Action::PlayToOut => {
            start_play_range(app, "to_out");
            true
        }
        Action::StepBack10 => {
            step_frames(app, -10);
            true
        }
        Action::StepFwd10 => {
            step_frames(app, 10);
            true
        }
        Action::FastReview => {
            // ponytail: flat 4x forward shuttle to the end, no per-cut pause-and-resume. Upgrade path:
            // reuse Project::cut_points() (already used by PrevCut/NextCut) to schedule holds here.
            if !app.player.is_playing() {
                app.player.play();
            }
            app.player.set_rate(4.0);
            true
        }
        _ => false,
    }
}

pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    if let Some(stop_at) = app.play_stop_at {
        if !app.player.is_playing() {
            app.play_stop_at = None; // stopped some other way (K, buffering, end of timeline)
        } else if stop_reached(app.player.time(), stop_at) {
            app.player.pause();
            app.seek(stop_at.clamp(0.0, app.project.duration()));
            app.play_stop_at = None;
        }
    }
    if should_scrub(app.settings.audio_scrub, app.player.is_playing(), app.scrub_last_t, app.playhead) {
        app.player.scrub(app.playhead);
    }
    app.scrub_last_t = app.playhead;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuttle_rate_ladder() {
        assert_eq!(shuttle_rate(0.0, true), -1.0);
        assert_eq!(shuttle_rate(-1.0, true), -2.0);
        assert_eq!(shuttle_rate(-2.0, true), -4.0);
        assert_eq!(shuttle_rate(-4.0, true), -8.0);
        assert_eq!(shuttle_rate(-8.0, true), -8.0, "clamps at -8");
        assert_eq!(shuttle_rate(2.0, true), -1.0, "direction switch resets to unit");
        // mirrored for forward (back=false)
        assert_eq!(shuttle_rate(0.0, false), 1.0);
        assert_eq!(shuttle_rate(1.0, false), 2.0);
        assert_eq!(shuttle_rate(2.0, false), 4.0);
        assert_eq!(shuttle_rate(4.0, false), 8.0);
        assert_eq!(shuttle_rate(8.0, false), 8.0, "clamps at 8");
        assert_eq!(shuttle_rate(-2.0, false), 1.0, "direction switch resets to unit");
    }

    /// `App::new` needs a live `eframe::CreationContext` (see tests.rs's doc comment / the identical
    /// deviation `tools_registry_tests.rs` already documents) — there is no headless `App` to build
    /// `act`/`tick` against, so this exercises the pure logic they're built from directly instead.
    #[test]
    fn play_range_auto_stops() {
        // in_out: starts at in_point (default 0), stops at out_point (default duration)
        assert_eq!(play_range_target("in_out", Some(2.0), Some(6.0), 3.0, 10.0, 2.0), (2.0, 6.0));
        assert_eq!(play_range_target("in_out", None, None, 3.0, 10.0, 2.0), (0.0, 10.0));
        // around: symmetric preroll around the playhead, clamped to [0, duration]
        assert_eq!(play_range_target("around", None, None, 5.0, 10.0, 2.0), (3.0, 7.0));
        assert_eq!(play_range_target("around", None, None, 0.5, 10.0, 2.0), (0.0, 2.5), "clamped at 0");
        assert_eq!(play_range_target("around", None, None, 9.5, 10.0, 2.0), (7.5, 10.0), "clamped at duration");
        // to_out: starts at the current playhead, stops at out_point (default duration)
        assert_eq!(play_range_target("to_out", None, Some(8.0), 3.0, 10.0, 2.0), (3.0, 8.0));
        assert_eq!(play_range_target("to_out", None, None, 3.0, 10.0, 2.0), (3.0, 10.0));
        // the auto-stop predicate: fires at or past the target, not before
        assert!(!stop_reached(5.9, 6.0));
        assert!(stop_reached(6.0, 6.0));
        assert!(stop_reached(6.1, 6.0));
    }

    /// `Settings::audio_scrub` gates the paused-playhead-change scrub; it must never fire while playing.
    #[test]
    fn audio_scrub_toggle_respected() {
        assert!(!should_scrub(false, false, 1.0, 2.0), "disabled: never scrubs");
        assert!(!should_scrub(true, true, 1.0, 2.0), "playing: never scrubs");
        assert!(!should_scrub(true, false, 2.0, 2.0), "no change: no scrub");
        assert!(should_scrub(true, false, 1.0, 2.0), "enabled, paused, changed: scrubs once");
    }
}
