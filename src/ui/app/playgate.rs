//! ---- ws:play-gate ----
//! Buffer before playing. Pressing Play when the frames just ahead of the playhead are not decoded
//! yet used to start the clock anyway, so the preview stuttered and showed stale frames while the audio
//! ran ahead. Now Play asks one question first, `ready?`, built from real signals:
//!
//!   * video - the render thread's cache holds the next `window` seconds of whole frames (a frame that
//!     went without a layer whose source was still opening does not count): decode, proxies, stills,
//!     nested sequences, transitions all end up as "is the frame in the cache";
//!   * audio - the ring holds its first buffers for the playhead, device still stopped, so the sound
//!     starts with the first frame instead of lagging it by the first (cold) mix;
//!   * shaders - the programs the clips in the window use are compiled (`GpuRenderer::warm`, a few ms a
//!     frame, so the UI stays alive), instead of compiling on the first frame that shows the effect;
//!   * anything else slow: add a `fn(&mut App, &Query) -> Report` to `SOURCES` (the HTML renderer's
//!     "this layer is still rendering and has no still yet" goes there, one line).
//!
//! Ready at the press: the clock starts in the same call, exactly as before (the check reads what the
//! render thread already published, it waits for nothing). Not ready: the player fills the window
//! (`Player::preroll`) with the clock frozen and the audio stopped, the Monitor says "Buffering", and the
//! clock starts the moment it is ready, or after `CAP` regardless (then "Still loading" is shown for a
//! moment instead of hanging). Space, Stop, a step or any `pause()` cancels; a seek keeps buffering at
//! the new place; J/L and `playback.rate` change the rate it will start at. Everything here is the
//! timeline player; export, scripts, the Source monitor, recording takes (`roll`) and the mid-play
//! stall (`buffer_stall`, which already pauses and resumes) do not go through it.

use super::*;
use std::time::{Duration, Instant};

/// Seconds of frames (and audio) that must be ready ahead of the playhead, before it is scaled by how
/// heavy a frame is (`window_secs`). The render thread's read-ahead is 1.5 s, so the window never
/// asks for more than the cache is sized to hold.
pub const WINDOW_SECS: f64 = 1.0;
const WINDOW_MIN: f64 = 0.5;
const WINDOW_MAX: f64 = 1.5;
/// Start anyway after this long, ready or not.
pub const CAP: Duration = Duration::from_secs(3);
/// How long "Still loading" stays up after a capped start.
pub const LATE_NOTE: Duration = Duration::from_secs(2);
/// UI poll while buffering (the render thread also wakes the UI the moment its window is full).
pub const POLL: Duration = Duration::from_millis(50);
/// Shader compile time spent per UI frame while buffering.
const WARM_BUDGET: Duration = Duration::from_millis(6);

/// The window for the current content: a frame that decodes in a fraction of its duration needs a
/// short lead, one that takes as long as it lasts (4K HEVC, stacked layers) needs the longest the cache
/// can hold. `cost` is the render thread's recent seconds per uncached frame; unknown = the default.
pub fn window_secs(cost: Option<f64>, fps: f64) -> f64 {
    cost.map_or(WINDOW_SECS, |c| (0.5 + c * fps.max(1.0)).clamp(WINDOW_MIN, WINDOW_MAX))
}

/// What a source is asked: is the stretch `[at, at + window]` ready? `work` false = just answer (the
/// press-time check must not cost anything); true = a source that has something to do (compile,
/// request) may do a small slice of it now.
#[derive(Clone, Copy, Debug)]
pub struct Query {
    pub at: f64,
    pub window: f64,
    pub work: bool,
}

/// One source's answer. `frac` is how far along it is when it knows (None = no number to show).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    pub name: &'static str,
    pub ready: bool,
    pub frac: Option<f32>,
}

impl Report {
    pub fn ready(name: &'static str) -> Self {
        Self { name, ready: true, frac: Some(1.0) }
    }
    pub fn waiting(name: &'static str, frac: Option<f32>) -> Self {
        Self { name, ready: false, frac }
    }
}

/// Every source that is not the player's own (video, audio). The registration point: a later change
/// adds its `fn` here and playback needs nothing else.
pub type Source = fn(&mut App, &Query) -> Report;
pub const SOURCES: &[Source] = &[shaders_ready];

/// All sources folded into one answer: ready when every one is; the progress is the slowest known
/// one; `waiting_on` names the first that is not ready.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub ready: bool,
    pub frac: Option<f32>,
    pub waiting_on: Option<&'static str>,
}

pub fn aggregate(reports: &[Report]) -> Verdict {
    Verdict {
        ready: reports.iter().all(|r| r.ready),
        frac: reports.iter().filter_map(|r| r.frac).reduce(f32::min),
        waiting_on: reports.iter().find(|r| !r.ready).map(|r| r.name),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Idle,
    /// Play was asked for, the clock is frozen, the window is filling.
    Buffering,
    /// The cap started playback before everything was ready: "Still loading" until `late_until`.
    Late,
}

#[derive(Debug, PartialEq)]
pub enum Start {
    Now,
    Wait,
}

#[derive(Debug, PartialEq)]
pub enum Poll {
    Idle,
    Wait,
    Go { rate: f64, capped: bool },
}

/// The pure state machine. Time is passed in, the readiness comes in as a `Verdict`, and what it
/// decides comes out as `Start`/`Poll`: nothing here touches a player or a clock.
#[derive(Debug)]
pub struct Gate {
    phase: Phase,
    since: Instant,
    late_until: Instant,
    rate: f64,
    window: f64,
    /// `Player::pauses()` when buffering began: it moving means someone paused, which cancels.
    pauses: u64,
    verdict: Verdict,
    /// How the last start went: (time from the press to the clock starting, started by the cap).
    pub last: Option<(Duration, bool)>,
}

impl Default for Gate {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            phase: Phase::Idle,
            since: now,
            late_until: now,
            rate: 1.0,
            window: WINDOW_SECS,
            pauses: 0,
            verdict: Verdict { ready: false, frac: None, waiting_on: None },
            last: None,
        }
    }
}

impl Gate {
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn buffering(&self) -> bool {
        self.phase == Phase::Buffering
    }
    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn window(&self) -> f64 {
        self.window
    }
    /// Play was pressed. `ready` is the press-time answer; true (or `enabled` false) starts the clock
    /// at once, nothing is held.
    pub fn request(&mut self, now: Instant, enabled: bool, ready: bool, window: f64, rate: f64, pauses: u64) -> Start {
        self.rate = rate;
        if !enabled || ready {
            (self.phase, self.last) = (Phase::Idle, Some((Duration::ZERO, false)));
            return Start::Now;
        }
        self.phase = Phase::Buffering;
        self.since = now;
        self.window = window;
        self.pauses = pauses;
        self.verdict = Verdict { ready: false, frac: Some(0.0), waiting_on: None };
        Start::Wait
    }
    /// One look while buffering (every UI frame): go when ready or when the cap is up.
    pub fn poll(&mut self, now: Instant, v: Verdict, pauses: u64) -> Poll {
        match self.phase {
            Phase::Idle => Poll::Idle,
            Phase::Late => {
                if pauses != self.pauses || now >= self.late_until {
                    self.phase = Phase::Idle;
                }
                Poll::Idle
            }
            Phase::Buffering => {
                if pauses != self.pauses {
                    self.phase = Phase::Idle; // someone paused: the request is cancelled
                    return Poll::Idle;
                }
                let waited = now.saturating_duration_since(self.since);
                let (ready, capped) = (v.ready, !v.ready && waited >= CAP);
                self.verdict = v;
                if !ready && !capped {
                    return Poll::Wait;
                }
                self.last = Some((waited, capped));
                (self.phase, self.late_until) =
                    if capped { (Phase::Late, now + LATE_NOTE) } else { (Phase::Idle, now) };
                Poll::Go { rate: self.rate, capped }
            }
        }
    }
    /// A shuttle or `playback.rate` while buffering changes the rate the clock will start at.
    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
    }
    /// A seek: still buffering (now at the new place; the window is re-checked from the next poll on),
    /// and the "Still loading" note of the old place goes.
    pub fn on_seek(&mut self) {
        match self.phase {
            Phase::Buffering => self.verdict.frac = Some(0.0),
            Phase::Late => self.phase = Phase::Idle,
            Phase::Idle => {}
        }
    }
    pub fn cancel(&mut self) {
        self.phase = Phase::Idle;
    }
    /// The text over the Monitor: "Buffering 40 %" / "Buffering - Waiting for X" / "Still loading".
    pub fn note(&self) -> Option<String> {
        match self.phase {
            Phase::Idle => None,
            Phase::Late => Some("Still loading".into()),
            // (0 % is no news: the first frame is still waiting on a source that is opening)
            Phase::Buffering => Some(match (self.verdict.frac.filter(|f| *f >= 0.005), self.verdict.waiting_on) {
                (Some(f), _) => format!("Buffering {} %", (f * 100.0).round() as u32),
                (None, Some(w)) => format!("Buffering ({w})"),
                (None, None) => "Buffering".into(),
            }),
        }
    }
    /// Progress for status reports (0..1), None when no source can say.
    pub fn frac(&self) -> Option<f32> {
        self.verdict.frac
    }
    pub fn waiting_on(&self) -> Option<&'static str> {
        self.verdict.waiting_on
    }
}

// ---------------- sources ----------------

/// The shader programs the clips in the window use are built (`GpuRenderer::warm`).
fn shaders_ready(app: &mut App, q: &Query) -> Report {
    let App { gpu, project, .. } = app;
    let Some(g) = gpu.as_mut() else { return Report::ready("shaders") };
    let budget = q.work.then_some(WARM_BUDGET);
    match guarded(|| g.warm(project, q.at, q.at + q.window, budget)) {
        Some(0) | None => Report::ready("shaders"),
        Some(_) => Report::waiting("shaders", None),
    }
}

// ---------------- glue ----------------

impl App {
    /// The timeline is playing or about to be (Buffering): what the transport button and the Space
    /// toggle go by.
    pub(super) fn transport_playing(&self) -> bool {
        self.player.is_playing() || self.play_gate.buffering()
    }

    /// The rate in effect or about to be.
    pub(super) fn transport_rate(&self) -> f64 {
        if self.play_gate.buffering() {
            self.play_gate.rate()
        } else {
            self.player.rate()
        }
    }

    /// `rate` for a clock that is running, or for the one that will start when buffering ends.
    pub(super) fn set_rate_gated(&mut self, rate: f64) {
        if self.play_gate.buffering() {
            self.play_gate.set_rate(rate);
        } else {
            self.player.set_rate(rate);
        }
    }

    /// Every way to start the timeline player goes through here: Space, J/L, Play In-Out / Around /
    /// To Out, Fast Review, `playback.play` / `playback.rate`. Already playing (or buffering): just the rate.
    pub(super) fn play_gated(&mut self, rate: f64) {
        if self.transport_playing() {
            return self.set_rate_gated(rate);
        }
        let end = self.project.duration();
        // not worth holding for: movie mode has its own hold, an empty timeline plays nothing, and a
        // reverse shuttle reads what is behind the playhead (the protected trail, mostly proxies)
        let enabled = self.settings.buffer_before_play && !self.settings.movie_mode && end > 0.0 && rate > 0.0;
        if enabled && self.player.time() >= end - 1e-6 {
            self.seek(0.0); // play() starts over from the end: buffer there, not at the end
        }
        let (at, window) = (self.player.time(), window_secs(self.player.frame_cost(), self.project.fps));
        let ready = enabled && {
            let q = Query { at, window, work: false };
            self.player.video_report(window).0 && self.source_reports(&q).iter().all(|r| r.ready)
        };
        let pauses = self.player.pauses();
        match self.play_gate.request(Instant::now(), enabled, ready, window, rate, pauses) {
            Start::Now => self.start_clock(rate),
            Start::Wait => self.player.preroll(window),
        }
    }

    /// The transport as `playback.play` / `playback.status` report it. `buffering` covers both kinds
    /// (Play waiting for its window, and the player refilling after a stall); `settled` is the field to
    /// poll for.
    pub(super) fn transport_json(&self) -> serde_json::Value {
        let (start, stall) = (self.play_gate.buffering(), self.player.is_buffering() || self.buffer_stall);
        let state = if start || stall {
            "buffering"
        } else if self.player.is_playing() {
            "playing"
        } else {
            "paused"
        };
        let (waited, capped) = self.play_gate.last.map_or((None, None), |(d, c)| (Some(d.as_millis() as u64), Some(c)));
        serde_json::json!({
            "state": state,
            "buffering": start || stall,
            "buffering_for": if start { Some("start") } else if stall { Some("stall") } else { None },
            "buffer_fraction": if start { self.play_gate.frac() } else { None },
            "waiting_on": if start { self.play_gate.waiting_on() } else { None },
            "settled": !(start || stall),
            "still_loading": self.play_gate.phase() == Phase::Late,
            // what Play would find right now if pressed while paused: the window and how much of it is cached
            "window_secs": window_secs(self.player.frame_cost(), self.project.fps),
            "cached_ahead": self.player.video_report(window_secs(self.player.frame_cost(), self.project.fps)).1,
            "start_wait_ms": waited,
            "start_capped": capped,
            "audio_start_lag_ms": self.player.audio_start_lag_ms(),
        })
    }

    fn start_clock(&mut self, rate: f64) {
        self.player.play();
        if (rate - 1.0).abs() > 1e-9 {
            self.player.set_rate(rate);
        }
    }

    fn source_reports(&mut self, q: &Query) -> Vec<Report> {
        SOURCES.iter().map(|s| s(self, q)).collect()
    }
}

/// FRAME_HOOKS-side tick (called from `playback_ctl::tick`): while buffering, look at every source and
/// start the clock when ready. Costs nothing, and requests no repaint, when idle.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if app.play_gate.phase() == Phase::Idle {
        return;
    }
    let (window, now) = (app.play_gate.window(), Instant::now());
    let mut reports = Vec::new();
    if app.play_gate.buffering() {
        let (ready, frac) = app.player.video_report(window);
        reports.push(Report { name: "video", ready, frac: Some(frac) });
        reports.push(Report { name: "audio", ready: app.player.audio_ready(), frac: None });
        let q = Query { at: app.player.time(), window, work: true };
        reports.extend(app.source_reports(&q));
    }
    match app.play_gate.poll(now, aggregate(&reports), app.player.pauses()) {
        Poll::Go { rate, .. } => app.start_clock(rate),
        Poll::Wait => ctx.request_repaint_after(POLL),
        Poll::Idle => {}
    }
    if app.play_gate.phase() == Phase::Late {
        ctx.request_repaint_after(POLL); // the note comes down by the clock, with no other event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake readiness source: not ready until `k` polls have gone by (None = never), then ready.
    struct Fake {
        ready_after: Option<u32>,
        polls: u32,
    }
    impl Fake {
        fn verdict(&mut self) -> Verdict {
            self.polls += 1;
            let ready = self.ready_after.is_some_and(|k| self.polls > k);
            aggregate(&[Report { name: "video", ready, frac: Some(self.polls as f32 / 10.0) }])
        }
    }

    /// Drive the gate on a fake 50 ms clock the way `tick` does; (what each poll returned, when).
    fn run(g: &mut Gate, f: &mut Fake, t0: Instant, ticks: u32) -> Vec<(u32, Poll)> {
        let mut out = Vec::new();
        for n in 1..=ticks {
            let p = g.poll(t0 + POLL * n, f.verdict(), 0);
            let done = matches!(p, Poll::Go { .. });
            out.push((n, p));
            if done {
                break;
            }
        }
        out
    }

    #[test]
    fn ready_at_the_press_starts_at_once_and_holds_nothing() {
        let mut g = Gate::default();
        let now = Instant::now();
        assert_eq!(g.request(now, true, true, 1.0, 1.0, 0), Start::Now);
        assert_eq!(g.phase(), Phase::Idle, "no buffering state, no indicator, nothing to poll");
        assert_eq!(g.note(), None);
        assert_eq!(g.last, Some((Duration::ZERO, false)));
        // the setting off behaves the same whatever the sources say
        assert_eq!(g.request(now, false, false, 1.0, 1.0, 0), Start::Now);
        assert_eq!(g.phase(), Phase::Idle);
    }

    #[test]
    fn becomes_ready_after_k_ticks_then_goes_exactly_once() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        assert_eq!(g.request(t0, true, false, 1.0, 1.0, 0), Start::Wait);
        assert!(g.buffering());
        let mut f = Fake { ready_after: Some(3), polls: 0 };
        let polls = run(&mut g, &mut f, t0, 20);
        assert_eq!(polls.len(), 4, "three waits, then the go");
        assert!(polls[..3].iter().all(|(_, p)| *p == Poll::Wait));
        assert_eq!(polls[3].1, Poll::Go { rate: 1.0, capped: false });
        assert_eq!(g.phase(), Phase::Idle);
        assert_eq!(g.last.map(|l| l.1), Some(false));
        assert_eq!(g.poll(t0 + POLL * 9, f.verdict(), 0), Poll::Idle, "it does not go twice");
    }

    #[test]
    fn progress_shows_while_waiting() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        g.request(t0, true, false, 1.0, 1.0, 0);
        assert_eq!(g.note().as_deref(), Some("Buffering"), "0 % says nothing yet");
        g.poll(t0 + POLL, aggregate(&[Report::waiting("video", Some(0.4))]), 0);
        assert_eq!(g.note().as_deref(), Some("Buffering 40 %"));
        g.poll(t0 + POLL * 2, aggregate(&[Report::waiting("html", None)]), 0);
        assert_eq!(g.note().as_deref(), Some("Buffering (html)"), "no number: say what it waits on");
    }

    #[test]
    fn never_ready_the_cap_starts_playback_and_keeps_a_still_loading_note() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        g.request(t0, true, false, 1.0, 1.0, 0);
        let mut f = Fake { ready_after: None, polls: 0 };
        // just under the cap: still waiting
        assert_eq!(g.poll(t0 + CAP - Duration::from_millis(1), f.verdict(), 0), Poll::Wait);
        let go = g.poll(t0 + CAP, f.verdict(), 0);
        assert_eq!(go, Poll::Go { rate: 1.0, capped: true });
        assert_eq!(g.phase(), Phase::Late);
        assert_eq!(g.note().as_deref(), Some("Still loading"));
        assert_eq!(g.last.map(|l| l.1), Some(true));
        // the note comes down by the clock, not by anything the source says
        assert_eq!(g.poll(t0 + CAP + LATE_NOTE - Duration::from_millis(1), f.verdict(), 0), Poll::Idle);
        assert_eq!(g.phase(), Phase::Late);
        g.poll(t0 + CAP + LATE_NOTE, f.verdict(), 0);
        assert_eq!(g.phase(), Phase::Idle);
        assert_eq!(g.note(), None);
    }

    #[test]
    fn a_pause_cancels_while_buffering_and_while_late() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        g.request(t0, true, false, 1.0, 1.0, 7);
        // the player's pause counter moved: Space / Stop / a step
        assert_eq!(g.poll(t0 + POLL, Fake { ready_after: Some(0), polls: 5 }.verdict(), 8), Poll::Idle);
        assert_eq!(g.phase(), Phase::Idle, "cancelled: no go even though it would have been ready");
        // and a second press right after starts a fresh request
        assert_eq!(g.request(t0 + POLL * 2, true, false, 1.0, 1.0, 8), Start::Wait);
        g.poll(t0 + POLL * 2 + CAP, aggregate(&[Report::waiting("video", None)]), 8);
        assert_eq!(g.phase(), Phase::Late);
        g.poll(t0 + POLL * 3 + CAP, aggregate(&[]), 9);
        assert_eq!(g.phase(), Phase::Idle, "pausing the late start clears the note");
        g.request(t0, true, false, 1.0, 1.0, 9);
        g.cancel();
        assert!(!g.buffering());
    }

    #[test]
    fn a_seek_keeps_buffering_at_the_new_place_and_the_cap_does_not_restart() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        g.request(t0, true, false, 1.0, 1.0, 0);
        g.poll(t0 + POLL, aggregate(&[Report::waiting("video", Some(0.8))]), 0);
        g.on_seek();
        assert!(g.buffering(), "a seek is not a cancel");
        assert_eq!(g.frac(), Some(0.0), "what was cached was for the old place");
        // the cap counts from the press: scrubbing about cannot hold playback off for ever
        let late = g.poll(t0 + CAP, aggregate(&[Report::waiting("video", Some(0.1))]), 0);
        assert_eq!(late, Poll::Go { rate: 1.0, capped: true });
        g.on_seek();
        assert_eq!(g.phase(), Phase::Idle, "a seek during the late note ends it");
    }

    #[test]
    fn a_rate_change_while_buffering_is_the_rate_it_starts_at() {
        let (mut g, t0) = (Gate::default(), Instant::now());
        g.request(t0, true, false, 1.0, 2.0, 0);
        g.set_rate(4.0);
        let go = g.poll(t0 + POLL, aggregate(&[Report::ready("video")]), 0);
        assert_eq!(go, Poll::Go { rate: 4.0, capped: false });
        // and a press that is ready at once carries its rate straight through
        assert_eq!(g.request(t0, true, true, 1.0, -2.0, 0), Start::Now);
        assert_eq!(g.rate(), -2.0);
    }

    #[test]
    fn a_loop_start_is_the_same_request_from_the_loop_in() {
        // Loop playback seeks to the loop in (`set_loop`) and is then an ordinary press; the wrap
        // while playing is the render thread's own cold-cache stall (`playback.rs`), which pauses
        // and resumes the clock outside the gate. The gate therefore only has to hold from the in point.
        let (mut g, t0) = (Gate::default(), Instant::now());
        g.on_seek(); // set_loop's seek to the in point, before the press: nothing to cancel
        assert_eq!(g.phase(), Phase::Idle);
        assert_eq!(g.request(t0, true, false, 1.0, 1.0, 0), Start::Wait);
        assert_eq!(g.poll(t0 + POLL, aggregate(&[Report::ready("video")]), 0), Poll::Go { rate: 1.0, capped: false });
    }

    #[test]
    fn aggregation_is_ready_only_when_every_source_is() {
        let v = aggregate(&[Report::ready("video"), Report::ready("audio"), Report::ready("shaders")]);
        assert_eq!(v, Verdict { ready: true, frac: Some(1.0), waiting_on: None });
        let v = aggregate(&[Report::ready("video"), Report::waiting("html", None), Report::waiting("audio", None)]);
        assert!(!v.ready);
        assert_eq!(v.waiting_on, Some("html"), "the first that is not ready is named");
        assert_eq!(v.frac, Some(1.0), "only sources that know their progress count towards it");
        let v = aggregate(&[Report::waiting("video", Some(0.25)), Report::waiting("shaders", Some(0.75))]);
        assert_eq!(v.frac, Some(0.25), "the slowest one sets the bar");
        let v = aggregate(&[Report::waiting("html", None)]);
        assert_eq!(v.frac, None);
        assert!(aggregate(&[]).ready, "no sources, nothing to wait for");
    }

    #[test]
    fn the_window_grows_with_the_cost_of_a_frame() {
        assert_eq!(window_secs(None, 30.0), WINDOW_SECS, "unknown: the default");
        assert!(window_secs(Some(0.001), 30.0) < 0.6, "cheap frames: a short lead is enough");
        assert_eq!(window_secs(Some(0.0), 30.0), WINDOW_MIN);
        let mid = window_secs(Some(1.0 / 30.0), 30.0);
        assert!((mid - 1.5).abs() < 1e-9, "a frame that takes its whole duration: the longest lead");
        assert_eq!(window_secs(Some(1.0), 30.0), WINDOW_MAX, "never more than the cache's read-ahead");
        assert!(window_secs(Some(0.01), 30.0) < window_secs(Some(0.02), 30.0));
    }

    /// Idle CPU rule: an idle gate asks for no repaint, however many frames go by.
    #[test]
    fn assert_no_idle_repaint_play_gate() {
        let ctx = egui::Context::default();
        let g = Gate::default();
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |_ctx| {
                // what `tick` does first: nothing at all while idle
                assert_eq!(g.phase(), Phase::Idle);
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle play gate must not ask for repaints");
    }
}
