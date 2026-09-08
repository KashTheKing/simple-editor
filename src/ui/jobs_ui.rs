//! ---- ws:jobs-panel ----
//! The Jobs pane widget: a pure, headless-testable list of every background task (running / queued /
//! recently finished) with progress, ETA, Cancel where the job honours it and reorder arrows for the
//! two queues that actually have an order. No `App` here — `ui::app::jobs_pane` builds the rows and
//! applies the response. Never requests a repaint itself (the App side owns the 150 ms cadence).

use crate::theme::Palette;
use eframe::egui;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Stable per-row id (`"<kind>:<index>"`, see `ui::app::jobs_pane::rows_from`) — what Cancel /
/// reorder / the `jobs.*` tools address a row by.
pub type JobId = String;

/// One variant per job holder on `App` (issue #64's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JobKind {
    Export,
    QueuedExport,
    Bake,
    Convert,
    Media,
    Mcp,
    Proxy,
    QueuedProxy,
    Download,
    Transcribe,
    Tracking,
    Tts,
    ModelDownload,
    ScreenRec,
    VoiceRec,
    Waveforms,
    Thumbnails,
    Prerender,
    Probes,
}

impl JobKind {
    pub fn name(self) -> &'static str {
        match self {
            JobKind::Export => "export",
            JobKind::QueuedExport => "queue",
            JobKind::Bake => "bake",
            JobKind::Convert => "convert",
            JobKind::Media => "media",
            JobKind::Mcp => "mcp",
            JobKind::Proxy => "proxy",
            JobKind::QueuedProxy => "proxy-queued",
            JobKind::Download => "download",
            JobKind::Transcribe => "transcribe",
            JobKind::Tracking => "tracking",
            JobKind::Tts => "tts",
            JobKind::ModelDownload => "model-download",
            JobKind::ScreenRec => "screen-rec",
            JobKind::VoiceRec => "voice-rec",
            JobKind::Waveforms => "waveforms",
            JobKind::Thumbnails => "thumbnails",
            JobKind::Prerender => "prerender",
            JobKind::Probes => "probes",
        }
    }
    /// Sets `Progress.cancel` (or its equivalent) and the job really stops.
    pub fn can_cancel(self) -> bool {
        !matches!(self, JobKind::Tts | JobKind::Tracking | JobKind::Probes | JobKind::QueuedProxy) && !self.is_cache()
    }
    /// Why Cancel is greyed out (shown as the disabled button's tooltip).
    pub fn cancel_hint(self) -> Option<&'static str> {
        match self {
            JobKind::Tts => Some("Speech finishes in under a second"),
            JobKind::Tracking => Some("Close the Tracking pane to stop"),
            JobKind::Probes => Some("Probing finishes on its own"),
            _ => None,
        }
    }
    /// Whole-queue caches: one aggregate row each, cleared as a whole, never logged as "finished".
    pub fn is_cache(self) -> bool {
        matches!(self, JobKind::Waveforms | JobKind::Thumbnails | JobKind::Prerender)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct JobRow {
    pub id: JobId,
    pub kind: JobKind,
    pub label: String,
    /// None = indeterminate (a count, not a fraction).
    pub fraction: Option<f32>,
    pub status: String,
    pub eta: Option<Duration>,
    pub elapsed: Duration,
    pub state: JobState,
    pub can_cancel: bool,
    pub can_reorder: bool,
}

impl JobRow {
    pub fn new(kind: JobKind, index: usize, label: impl Into<String>) -> Self {
        Self {
            id: format!("{}:{index}", kind.name()),
            kind,
            label: label.into(),
            fraction: None,
            status: String::new(),
            eta: None,
            elapsed: Duration::ZERO,
            state: JobState::Running,
            can_cancel: kind.can_cancel(),
            can_reorder: kind == JobKind::QueuedExport,
        }
    }
}

/// Finished rows are kept for this long so a fast job is not invisible.
pub const DONE_TTL: Duration = Duration::from_secs(60);
const DONE_CAP: usize = 20;

#[derive(Default)]
pub struct JobsState {
    pub show_done: bool,
    /// Last ~20 rows that left their holder, with when they left.
    pub done_log: VecDeque<(JobRow, Instant)>,
    /// The rows as of the last `note` — the pane, the menu-bar indicator and the `jobs.*` tools all
    /// read this one snapshot instead of walking the holders again.
    pub last: Vec<JobRow>,
    /// The pane or the indicator drew during the last frame (gates the repaint cadence).
    pub drew: bool,
    /// First `tick` ran (the SE_JOBS_PANE screenshot hook fires once).
    pub booted: bool,
    /// (next rescan, source paths): proxies still to build, refreshed every 2 s while the pane shows.
    pub proxy_queued: (Option<Instant>, Vec<String>),
}

impl JobsState {
    /// Take this frame's snapshot: rows that vanished since the last one go to `done_log` (as Done,
    /// or Cancelled when their cancel flag was already set), stale log entries are dropped. Returns
    /// the ids that are new this frame (auto-reveal keys off it).
    pub fn note(&mut self, rows: Vec<JobRow>, now: Instant) -> Vec<JobId> {
        let fresh: Vec<JobId> = rows
            .iter()
            .filter(|r| r.state == JobState::Running && !self.last.iter().any(|o| o.id == r.id))
            .map(|r| r.id.clone())
            .collect();
        for old in self.last.drain(..) {
            if old.kind.is_cache() || old.state == JobState::Queued || rows.iter().any(|r| r.id == old.id) {
                continue;
            }
            let mut fin = old;
            if fin.state == JobState::Running {
                fin.state = JobState::Done;
                fin.fraction = Some(1.0);
            }
            fin.eta = None;
            self.done_log.push_front((fin, now));
        }
        self.done_log.truncate(DONE_CAP);
        self.done_log.retain(|(_, at)| now.duration_since(*at) < DONE_TTL);
        self.last = rows;
        fresh
    }
    pub fn running(&self) -> usize {
        self.last.iter().filter(|r| r.state == JobState::Running).count()
    }
}

/// What the widget asked for this frame; `ui::app::jobs_pane::apply` honours it.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct JobsResponse {
    pub cancel: Vec<JobId>,
    /// (id, ±1): move a queued export up/down.
    pub reorder: Vec<(JobId, isize)>,
    /// A queued-proxy row's id: build that one next.
    pub proxy_next: Option<JobId>,
    /// Clear the waveform / thumbnail / pre-render caches.
    pub clear_caches: bool,
}

/// Running / Queued / Recent, in that order.
pub fn grouped<'a>(rows: &'a [JobRow], done_log: &'a VecDeque<(JobRow, Instant)>) -> [Vec<&'a JobRow>; 3] {
    let running = rows.iter().filter(|r| r.state == JobState::Running).collect();
    let queued = rows.iter().filter(|r| r.state == JobState::Queued).collect();
    let recent = done_log.iter().map(|(r, _)| r).collect();
    [running, queued, recent]
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn show(ui: &mut egui::Ui, st: &mut JobsState, rows: &[JobRow], pal: &Palette) -> JobsResponse {
    let mut out = JobsResponse::default();
    let [running, queued, recent] = grouped(rows, &st.done_log);
    let any_cache = rows.iter().any(|r| r.kind.is_cache());
    ui.horizontal(|ui| {
        ui.heading("Jobs");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut st.show_done, "Recent");
            if any_cache
                && ui
                    .button("Clear caches")
                    .on_hover_text("Drop queued waveform / thumbnail / pre-render work")
                    .clicked()
            {
                out.clear_caches = true;
            }
        });
    });
    if running.is_empty() && queued.is_empty() {
        ui.weak("No background jobs");
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (title, group) in [("Running", &running), ("Queued", &queued)] {
            if group.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("{title} ({})", group.len())).strong().color(pal.text_dim));
            for (i, row) in group.iter().enumerate() {
                row_ui(ui, row, i, group.len(), &mut out);
            }
        }
        if st.show_done && !recent.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Recent").strong().color(pal.text_dim));
            for row in recent {
                ui.horizontal(|ui| {
                    let (glyph, text) = match &row.state {
                        JobState::Failed(e) => ("✗", format!("{} — {e}", row.label)),
                        JobState::Cancelled => ("–", format!("{} — cancelled", row.label)),
                        _ => ("✓", format!("{} — {}", row.label, fmt_dur(row.elapsed))),
                    };
                    ui.label(glyph);
                    ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
                });
            }
        }
    });
    out
}

fn row_ui(ui: &mut egui::Ui, row: &JobRow, i: usize, n: usize, out: &mut JobsResponse) {
    ui.push_id(&row.id, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(&row.label).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if row.kind.is_cache() {
                    return;
                }
                let cancel = ui.add_enabled(row.can_cancel, egui::Button::new("Cancel").small());
                if cancel.clicked() {
                    out.cancel.push(row.id.clone());
                }
                if let Some(hint) = row.kind.cancel_hint() {
                    cancel.on_disabled_hover_text(hint);
                }
                if row.can_reorder {
                    if ui.add_enabled(i + 1 < n, egui::Button::new("▼").small()).clicked() {
                        out.reorder.push((row.id.clone(), 1));
                    }
                    if ui.add_enabled(i > 0, egui::Button::new("▲").small()).clicked() {
                        out.reorder.push((row.id.clone(), -1));
                    }
                }
                if row.kind == JobKind::QueuedProxy && ui.add(egui::Button::new("Build next").small()).clicked() {
                    out.proxy_next = Some(row.id.clone());
                }
            });
        });
        let mut text = row.status.clone();
        if let Some(eta) = row.eta {
            if !text.is_empty() {
                text.push_str(" · ");
            }
            text.push_str(&format!("{} left", fmt_dur(eta)));
        }
        match row.fraction {
            Some(f) if row.state == JobState::Running => {
                ui.add(egui::ProgressBar::new(f).show_percentage().text(egui::RichText::new(text).small()));
            }
            _ if !text.is_empty() => {
                ui.label(egui::RichText::new(text).small().weak());
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2};

    fn row(kind: JobKind, i: usize, state: JobState) -> JobRow {
        let mut r = JobRow::new(kind, i, format!("{}-{i}", kind.name()));
        r.state = state;
        r.fraction = Some(0.5);
        r
    }

    struct H {
        ctx: egui::Context,
        st: JobsState,
        rows: Vec<JobRow>,
        pal: Palette,
        shapes: Vec<egui::epaint::ClippedShape>,
    }
    impl H {
        fn new(rows: Vec<JobRow>) -> Self {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            Self { ctx, st: JobsState::default(), rows, pal: Palette::new(true, egui::Color32::WHITE), shapes: vec![] }
        }
        fn frame(&mut self, events: Vec<Event>) -> JobsResponse {
            let H { ctx, st, rows, pal, shapes } = self;
            let mut resp = JobsResponse::default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 600.0))),
                events,
                ..Default::default()
            };
            let full = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| resp = show(ui, st, rows, pal));
            });
            *shapes = full.shapes;
            resp
        }
        fn text_at(&self, label: &str, nth: usize) -> Pos2 {
            self.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
                    _ => None,
                })
                .nth(nth)
                .unwrap_or_else(|| panic!("no '{label}' #{nth} painted"))
        }
        fn click(&mut self, pos: Pos2) -> JobsResponse {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }])
        }
    }

    #[test]
    fn rows_group_running_queued_and_recent() {
        let mut st = JobsState::default();
        let t0 = Instant::now();
        let rows = vec![
            row(JobKind::QueuedExport, 0, JobState::Queued),
            row(JobKind::Convert, 0, JobState::Running),
            row(JobKind::Export, 0, JobState::Running),
        ];
        let fresh = st.note(rows.clone(), t0);
        assert_eq!(fresh, vec!["convert:0", "export:0"], "every Running row is new on the first snapshot");
        let [r, q, d] = grouped(&rows, &st.done_log);
        assert_eq!(r.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["convert:0", "export:0"]);
        assert_eq!(q.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["queue:0"]);
        assert!(d.is_empty());
        // the convert finishes (leaves its holder): it moves to Recent, the queued row never does
        let fresh = st.note(vec![rows[2].clone()], t0 + Duration::from_secs(1));
        assert!(fresh.is_empty(), "a row already seen last frame is not new");
        let [_, _, d] = grouped(&rows[2..], &st.done_log);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, "convert:0");
        assert_eq!(d[0].state, JobState::Done);
        // ... and is dropped once it is older than DONE_TTL
        st.note(vec![rows[2].clone()], t0 + DONE_TTL + Duration::from_secs(2));
        assert!(st.done_log.is_empty(), "a Done row older than 60 s leaves done_log");
        assert_eq!(st.running(), 1);
    }

    #[test]
    fn cancel_button_only_on_cancellable_rows() {
        let mut h = H::new(vec![row(JobKind::Tts, 0, JobState::Running), row(JobKind::Convert, 0, JobState::Running)]);
        h.frame(vec![]);
        assert!(!h.rows[0].can_cancel && h.rows[1].can_cancel);
        let tts = h.text_at("Cancel", 0);
        let resp = h.click(tts);
        assert!(resp.cancel.is_empty(), "a disabled Cancel returns no id");
        let conv = h.text_at("Cancel", 1);
        let resp = h.click(conv);
        assert_eq!(resp.cancel, vec!["convert:0".to_string()]);
    }

    #[test]
    fn reorder_arrows_only_on_reorderable_rows() {
        let mut h = H::new(vec![row(JobKind::Convert, 0, JobState::Running)]);
        h.frame(vec![]);
        assert!(!h.shapes.iter().any(|c| matches!(&c.shape, egui::epaint::Shape::Text(t) if t.galley.text() == "▲")));
        let mut h = H::new(vec![
            row(JobKind::QueuedExport, 0, JobState::Queued),
            row(JobKind::QueuedExport, 1, JobState::Queued),
        ]);
        h.frame(vec![]);
        let up = h.text_at("▲", 1);
        let resp = h.click(up);
        assert_eq!(resp.reorder, vec![("queue:1".to_string(), -1)]);
    }

    #[test]
    fn assert_no_idle_repaint_jobs_pane() {
        let mut h = H::new(vec![]);
        for _ in 0..30 {
            h.frame(vec![]);
        }
        assert!(!h.ctx.has_requested_repaint(), "an empty Jobs pane must request no repaint");
    }
}
