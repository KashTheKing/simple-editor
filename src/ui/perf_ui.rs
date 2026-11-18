//! ---- ws:perf-window ----
//! The Performance pane (Window ▸ Performance, the top bar's chart button, `perf.report`): a dark card
//! with a donut of where the frame time goes and a legend sorted by cost, a Live / Project toggle, and
//! live stat tiles under it.
//! Live = the span recorder's self time per category (`perf::self_totals`), sampled ~4 times a second by
//! `ui::app::perf_ctl::tick` - never per frame. Project = the heaviest sources and clips, each with a
//! Jump and, where there is one, a Fix. Drawing never asks for a repaint.

use crate::model::Id;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Segment colours, by rank (most expensive first).
pub const COLORS: [Color32; 8] = [
    Color32::from_rgb(0x33, 0x66, 0xFF),
    Color32::from_rgb(0xE0, 0x60, 0x2A),
    Color32::from_rgb(0x2B, 0xB3, 0xA3),
    Color32::from_rgb(0x9B, 0x5D, 0xE5),
    Color32::from_rgb(0xF2, 0xC1, 0x4E),
    Color32::from_rgb(0x4C, 0xAF, 0x50),
    Color32::from_rgb(0xE5, 0x48, 0x7A),
    Color32::from_rgb(0x6C, 0x8E, 0xBF),
];
const IDLE: Color32 = Color32::from_gray(78);
const CARD: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x23);
const TRACK: Color32 = Color32::from_rgb(0x2C, 0x2F, 0x37);
const TEXT: Color32 = Color32::from_gray(232);
const MUTED: Color32 = Color32::from_gray(140);
const GOOD: Color32 = Color32::from_rgb(0x4C, 0xC0, 0x6A);
const WARN: Color32 = Color32::from_rgb(0xF2, 0xC1, 0x4E);
const BAD: Color32 = Color32::from_rgb(0xEF, 0x55, 0x50);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tab {
    #[default]
    Live,
    Project,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    /// Asset id: `Asset.proxy = Some(true)`, built next.
    Proxy(Id),
    /// (from, to) seconds: the existing pre-render.
    Prerender(f64, f64),
    /// Run the disk-cache pruner now.
    PruneCache,
    /// Pause (true) / resume background jobs.
    PauseJobs(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Click {
    /// Select the clip and move the playhead to the time.
    Jump(Id, f64),
    Fix(Fix),
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub label: String,
    pub detail: String,
    /// Cost in ms per frame (the donut's measure).
    pub ms: f32,
    /// `ms` is an estimate, not a measurement.
    pub est: bool,
    /// The "Idle" slice: grey, whatever its rank.
    pub idle: bool,
    /// Shown instead of `ms` (a row outside the donut: disk cache, jobs).
    pub value: Option<String>,
    pub jump: Option<(Id, f64)>,
    pub fix: Option<(Fix, &'static str)>,
}

/// Sort `rows` by cost (most first) and return each one's share of the total (they sum to 1; all 0 when
/// nothing costs anything).
pub fn segments(rows: &mut [Row]) -> Vec<f32> {
    rows.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    let total: f32 = rows.iter().map(|r| r.ms.max(0.0)).sum();
    rows.iter().map(|r| if total > 0.0 { r.ms.max(0.0) / total } else { 0.0 }).collect()
}

fn seg_color(rows: &[Row], i: usize) -> Color32 {
    if rows[i].idle {
        return IDLE;
    }
    let rank = rows[..i].iter().filter(|r| !r.idle).count();
    COLORS[rank % COLORS.len()]
}

// ---- grading against plans/peak-performance budgets (0 drops, worst gap < 50 ms, UI p95 < 8 ms) ----
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grade {
    Good,
    Warn,
    Bad,
    Neutral,
}

impl Grade {
    pub fn color(self) -> Color32 {
        match self {
            Grade::Good => GOOD,
            Grade::Warn => WARN,
            Grade::Bad => BAD,
            Grade::Neutral => TEXT,
        }
    }
    fn under(v: f32, good: f32, warn: f32) -> Grade {
        if v < good {
            Grade::Good
        } else if v < warn {
            Grade::Warn
        } else {
            Grade::Bad
        }
    }
}

pub fn grade_fps(fps: f32, target: f32) -> Grade {
    if target <= 0.0 {
        return Grade::Neutral;
    }
    let r = fps / target;
    if r >= 0.98 {
        Grade::Good
    } else if r >= 0.9 {
        Grade::Warn
    } else {
        Grade::Bad
    }
}
/// Dropped frames in the last 10 s: the budget is none.
pub fn grade_drops(n: u64) -> Grade {
    Grade::under(n as f32, 1.0, 6.0)
}
pub fn grade_gap(ms: f32) -> Grade {
    Grade::under(ms, 50.0, 100.0)
}
pub fn grade_ui_p95(ms: f32) -> Grade {
    Grade::under(ms, 8.0, 16.0)
}
/// A decode is fine while it fits half a frame, a problem past a whole one.
pub fn grade_decode(ms: f32, frame_ms: f32) -> Grade {
    Grade::under(ms, frame_ms * 0.5, frame_ms)
}
pub fn grade_disk(used: f32, budget: f32) -> Grade {
    if budget <= 0.0 {
        return Grade::Neutral;
    }
    Grade::under(used / budget, 0.9, 1.0)
}

// ---- the live sampler ----
/// (label, `perf::NAMES` entries whose self time it is). `render_frame` is left out on purpose: its self
/// time is the render thread waiting for work.
const CATEGORIES: [(&str, &[&str]); 9] = [
    ("Decode", &["layer_decode"]),
    ("GPU composite", &["gpu_composite"]),
    ("Texture upload", &["gpu_upload"]),
    ("Nested sequences", &["nested_render"]),
    ("Effects", &["effects"]),
    ("Timeline draw", &["timeline_draw"]),
    ("Inspector", &["inspector_draw"]),
    ("Audio mix", &["audio_mix"]),
    ("Other UI", &["ui_frame", "other"]),
];
const PERIOD: Duration = Duration::from_millis(220);
const HIST: usize = 45; // ~10 s of samples

/// What the app hands the sampler each period.
#[derive(Clone, Copy, Default)]
pub struct Inputs {
    pub playing: bool,
    pub target_fps: f32,
    pub published: u64,
    pub dropped: u64,
    pub gap_ms: f32,
}

#[derive(Default)]
pub struct Live {
    at: Option<Instant>,
    totals: Option<[u64; crate::perf::NAMES.len()]>,
    published: u64,
    dropped0: Option<u64>,
    dropped_last: u64,
    ema: [f32; CATEGORIES.len()],
    /// (fps, drops, worst gap ms) per sample while playing, newest last.
    pub hist: VecDeque<(f32, u64, f32)>,
    pub playing: bool,
    pub fps: f32,
    pub target_fps: f32,
    pub dropped_session: u64,
    pub rows: Vec<Row>,
    // the HUD's and the app's numbers, copied in by perf_ctl at each sample
    pub ui_p95: f32,
    pub decode_avg: f32,
    pub decode_worst: f32,
    pub hit_rate: f32,
    pub ram_mb: f32,
    pub ram_budget_mb: f32,
    pub queued: usize,
    pub disk_gb: Option<f32>,
    pub disk_budget_gb: Option<f32>,
    pub jobs: usize,
    pub jobs_paused: bool,
}

impl Live {
    pub fn due(&self) -> bool {
        self.at.is_none_or(|a| a.elapsed() >= PERIOD)
    }
    pub fn dropped_10s(&self) -> u64 {
        self.hist.iter().map(|h| h.1).sum()
    }
    pub fn worst_gap(&self) -> f32 {
        self.hist.iter().map(|h| h.2).fold(0.0, f32::max)
    }
    pub fn frame_ms(&self) -> f32 {
        1000.0 / self.target_fps.max(1.0)
    }

    /// One sample: `totals` from `perf::self_totals`, `now` its time.
    pub fn sample(&mut self, totals: [u64; crate::perf::NAMES.len()], now: Instant, i: Inputs) {
        let dt = self.at.map(|a| now.saturating_duration_since(a).as_secs_f32());
        self.target_fps = i.target_fps;
        self.playing = i.playing;
        let d0 = *self.dropped0.get_or_insert(i.dropped);
        self.dropped_session = i.dropped.saturating_sub(d0);
        if let (Some(dt), Some(prev)) = (dt.filter(|&d| d > 0.0), self.totals) {
            let fps = self.target_fps.max(1.0);
            let per_frame = |us: u64| us as f32 / 1000.0 / dt / fps;
            for (k, (_, names)) in CATEGORIES.iter().enumerate() {
                let us: u64 = names
                    .iter()
                    .filter_map(|n| crate::perf::NAMES.iter().position(|x| x == n))
                    .map(|j| totals[j].saturating_sub(prev[j]))
                    .sum();
                self.ema[k] = 0.5 * self.ema[k] + 0.5 * per_frame(us);
            }
            if i.playing {
                self.fps = i.published.saturating_sub(self.published) as f32 / dt;
                if self.hist.len() == HIST {
                    self.hist.pop_front();
                }
                self.hist.push_back((self.fps, i.dropped.saturating_sub(self.dropped_last), i.gap_ms));
            }
        }
        (self.at, self.totals, self.published, self.dropped_last) = (Some(now), Some(totals), i.published, i.dropped);
        self.rows = self.category_rows();
    }

    fn category_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = CATEGORIES
            .iter()
            .zip(self.ema)
            .filter(|(_, ms)| *ms >= 0.005)
            .map(|((label, _), ms)| Row { label: label.to_string(), ms, ..Default::default() })
            .collect();
        let used: f32 = rows.iter().map(|r| r.ms).sum();
        rows.push(Row {
            label: "Idle".into(),
            ms: (self.frame_ms() - used).max(0.0),
            idle: true,
            ..Default::default()
        });
        rows
    }
}

/// The pane's state (`App::perf`).
#[derive(Default)]
pub struct PerfWin {
    /// The pane was on screen last frame (`perf_ctl::tick` sets it): live sampling runs only then.
    pub shown: bool,
    pub tab: Tab,
    pub live: Live,
    /// Project tab: donut rows, then rows outside the donut (disk cache, jobs).
    pub project: Vec<Row>,
    pub extra: Vec<Row>,
    pub project_at: Option<Instant>,
    /// `perf.report` keeps sampling for a minute without the pane on screen.
    pub report_until: Option<Instant>,
}

impl PerfWin {
    pub fn sampling(&self) -> bool {
        self.shown || self.report_until.is_some_and(|t| Instant::now() < t)
    }
}

// ---- drawing ----
/// Text with a 0.6 px double strike: the app has no bold face.
fn bold(p: &egui::Painter, at: Pos2, align: Align2, text: &str, size: f32, color: Color32) -> Rect {
    let font = FontId::proportional(size);
    p.text(at + vec2(0.6, 0.0), align, text, font.clone(), color);
    p.text(at, align, text, font, color)
}

/// The ring: each row a stroked arc, with a small gap between neighbours, clockwise from 12 o'clock.
fn donut(p: &egui::Painter, c: Pos2, r: f32, width: f32, rows: &[Row], fr: &[f32]) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    if fr.iter().all(|&f| f <= 0.0) {
        // a closed polyline, not `circle_stroke`: a 35 px wide circle stroke tessellates into an octagon
        let pts: Vec<Pos2> = (0..=160).map(|k| k as f32 / 160.0 * TAU).map(|t| c + r * vec2(t.cos(), t.sin())).collect();
        p.add(egui::Shape::line(pts, Stroke::new(width, TRACK)));
        return;
    }
    let shown = fr.iter().filter(|&&f| f > 0.0).count();
    let gap = if shown > 1 { 0.035 } else { 0.0 };
    let mut a = -FRAC_PI_2;
    for (i, &f) in fr.iter().enumerate() {
        let sweep = f * TAU;
        if f <= 0.0 {
            continue;
        }
        let (a0, a1) = if sweep > gap * 1.5 { (a + gap / 2.0, a + sweep - gap / 2.0) } else { (a, a + sweep) };
        let n = ((a1 - a0) / 0.04).ceil().max(2.0) as usize;
        let pts: Vec<Pos2> =
            (0..=n).map(|k| a0 + (a1 - a0) * k as f32 / n as f32).map(|t| c + r * vec2(t.cos(), t.sin())).collect();
        p.add(egui::Shape::line(pts, Stroke::new(width, seg_color(rows, i))));
        a += sweep;
    }
}

const ROW_H: f32 = 40.0;
const VALUE_W: f32 = 70.0;
const PCT_W: f32 = 42.0;
const CTRL_W: f32 = 190.0;

/// One legend row as a table row: [dot][label, truncated][value][percent][controls], the same x on
/// every row; a thin cost bar under the label.
/// ponytail: drawn here rather than with `ui::list_rows` - a legend is not reorderable, so no grip/arrow.
fn legend_row(ui: &mut Ui, row: &Row, frac: Option<f32>, color: Color32, ctrls: bool) -> Option<Click> {
    let w = ui.available_width();
    // a narrow pane puts the row's buttons on a line of their own under it
    let wrap = ctrls && w < CTRL_W + 200.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, if wrap { ROW_H + 24.0 } else { ROW_H }), Sense::hover());
    let p = ui.painter_at(rect);
    let ctrl_x = rect.right() - if ctrls && !wrap { CTRL_W } else { 0.0 };
    let pct_x = ctrl_x - 8.0;
    let val_x = pct_x - PCT_W;
    let y = rect.top() + 12.0;
    p.rect_filled(Rect::from_center_size(pos2(rect.left() + 6.0, y), vec2(10.0, 10.0)), 2.5, color);
    let label_w = (val_x - VALUE_W - rect.left() - 26.0).max(20.0);
    let galley = ui.painter().layout(row.label.clone(), FontId::proportional(13.0), TEXT, label_w);
    let label = if galley.rows.len() > 1 {
        let mut job = egui::text::LayoutJob::simple_singleline(row.label.clone(), FontId::proportional(13.0), TEXT);
        job.wrap = egui::text::TextWrapping::truncate_at_width(label_w);
        ui.painter().layout_job(job)
    } else {
        galley
    };
    p.galley(pos2(rect.left() + 18.0, y - label.size().y / 2.0), label, TEXT);
    let value = match (&row.value, row.est) {
        (Some(v), _) => v.clone(),
        (None, true) => format!("~{:.1} ms", row.ms),
        (None, false) => format!("{:.1} ms", row.ms),
    };
    bold(&p, pos2(val_x, y), Align2::RIGHT_CENTER, &value, 13.0, TEXT);
    if let Some(f) = frac {
        p.text(pos2(pct_x, y), Align2::RIGHT_CENTER, format!("{:.0}%", f * 100.0), FontId::proportional(12.0), MUTED);
        let bar = Rect::from_min_max(pos2(rect.left() + 18.0, rect.top() + 24.0), pos2(pct_x, rect.top() + 28.0));
        p.rect_filled(bar, 2.0, TRACK);
        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * f.clamp(0.0, 1.0), bar.height()));
        p.rect_filled(fill, 2.0, color);
    }
    if !row.detail.is_empty() && frac.is_none() {
        p.text(
            pos2(rect.left() + 18.0, rect.top() + 30.0),
            Align2::LEFT_CENTER,
            &row.detail,
            FontId::proportional(11.0),
            MUTED,
        );
    }
    let resp = if row.detail.is_empty() { resp } else { resp.on_hover_text(&row.detail) };
    let _ = resp;
    let mut out = None;
    if ctrls {
        let (x0, by) = if wrap { (rect.left() + 12.0, y + ROW_H) } else { (ctrl_x, y) };
        let b = |x: f32, w: f32| Rect::from_min_size(pos2(x - ctrl_x + x0, by - 10.0), vec2(w, 20.0));
        if let Some((id, t)) = row.jump {
            if ui.put(b(ctrl_x + 6.0, 64.0), egui::Button::new("Jump to")).clicked() {
                out = Some(Click::Jump(id, t));
            }
        }
        if let Some((fix, label)) = &row.fix {
            if ui.put(b(ctrl_x + 76.0, CTRL_W - 76.0), egui::Button::new(*label)).on_hover_text(&row.detail).clicked() {
                out = Some(Click::Fix(fix.clone()));
            }
        }
    }
    out
}

fn segmented(ui: &mut Ui, tab: &mut Tab) {
    egui::Frame::new().fill(TRACK).corner_radius(7.0).inner_margin(2.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for t in [Tab::Project, Tab::Live] {
            // right-to-left layout: Project is drawn first so Live sits on the left
            let on = *tab == t;
            let text = egui::RichText::new(if t == Tab::Live { "Live" } else { "Project" }).size(12.0).color(if on {
                Color32::WHITE
            } else {
                MUTED
            });
            let b = egui::Button::new(text).fill(if on { COLORS[0] } else { Color32::TRANSPARENT }).corner_radius(5.0);
            if ui.add(b).clicked() {
                *tab = t;
            }
        }
    });
}

/// The card: title, toggle, donut with the centre text, legend.
fn card(ui: &mut Ui, w: &mut PerfWin) -> Option<Click> {
    let mut out = None;
    egui::Frame::new().fill(CARD).corner_radius(12.0).inner_margin(16.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // narrow (the Export page's Jobs column): the tabs go under the title, the legend under the donut
        let narrow = ui.available_width() < STACK_BELOW;
        if narrow {
            ui.label(egui::RichText::new("Where the performance goes").size(15.0).strong().color(TEXT));
            ui.add_space(4.0);
            ui.horizontal(|ui| segmented(ui, &mut w.tab));
        } else {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Where the performance goes").size(15.0).strong().color(TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| segmented(ui, &mut w.tab));
            });
        }
        ui.add_space(10.0);
        let live = w.tab == Tab::Live;
        let mut rows = if live { w.live.rows.clone() } else { w.project.clone() };
        let fr = segments(&mut rows);
        let dir = if narrow { egui::Direction::TopDown } else { egui::Direction::LeftToRight };
        ui.with_layout(egui::Layout::from_main_dir_and_cross_align(dir, egui::Align::Min), |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(210.0, 210.0), Sense::hover());
            let p = ui.painter_at(rect);
            donut(&p, rect.center(), 84.0, 35.0, &rows, &fr);
            let (big, small, col) = if live {
                let l = &w.live;
                let fps = if l.playing { format!("{:.1}", l.fps) } else { "–".into() };
                let cap = if l.playing { format!("fps of {:.0}", l.target_fps) } else { "paused".into() };
                (fps, cap, if l.playing { grade_fps(l.fps, l.target_fps).color() } else { TEXT })
            } else {
                let total: f32 = rows.iter().map(|r| r.ms).sum::<f32>() + 0.0; // an empty sum is -0.0
                (format!("{total:.1}"), "ms per frame".into(), TEXT)
            };
            let col = if col == GOOD { TEXT } else { col }; // only a problem is coloured
            bold(&p, rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, &big, 30.0, col);
            p.text(rect.center() + vec2(0.0, 18.0), Align2::CENTER_CENTER, small, FontId::proportional(12.0), MUTED);
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if rows.is_empty() {
                    ui.colored_label(
                        MUTED,
                        if live { "Play the timeline to measure." } else { "Nothing heavy in this project." },
                    );
                }
                for (i, r) in rows.iter().enumerate() {
                    out = out.take().or(legend_row(ui, r, Some(fr[i]), seg_color(&rows, i), !live));
                }
                if !live {
                    for r in &w.extra {
                        out = out.take().or(legend_row(ui, r, None, IDLE, true));
                    }
                }
            });
        });
    });
    out
}

/// A stat tile: caption, big value in its grade's colour, a sub line, an optional sparkline.
fn tile(ui: &mut Ui, caption: &str, value: &str, sub: &str, g: Grade, spark: Option<(&[f32], f32)>) {
    egui::Frame::new().fill(CARD).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
        ui.set_width(124.0);
        ui.set_min_height(76.0);
        ui.vertical(|ui| tile_body(ui, caption, value, sub, g, spark));
    });
}

fn tile_body(ui: &mut Ui, caption: &str, value: &str, sub: &str, g: Grade, spark: Option<(&[f32], f32)>) {
    {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(egui::RichText::new(caption).size(11.0).color(MUTED));
        let (r, _) = ui.allocate_exact_size(vec2(124.0, 22.0), Sense::hover());
        bold(ui.painter(), r.left_center(), Align2::LEFT_CENTER, value, 18.0, g.color());
        ui.label(egui::RichText::new(sub).size(11.0).color(MUTED));
        if let Some((vals, max)) = spark {
            let (r, _) = ui.allocate_exact_size(vec2(124.0, 18.0), Sense::hover());
            if vals.len() > 1 {
                let n = (vals.len() - 1) as f32;
                let pts: Vec<Pos2> = vals
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        pos2(
                            r.left() + r.width() * i as f32 / n,
                            r.bottom() - r.height() * (v / max.max(1.0)).clamp(0.0, 1.0),
                        )
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, g.color())));
            }
        }
    }
}

/// One stat tile's content: caption, value, sub line, grade, sparkline.
type TileSpec = (&'static str, String, String, Grade, Option<(Vec<f32>, f32)>);

/// A tile's footprint: 124 content + 2 x 10 margin.
const TILE_W: f32 = 144.0;

fn tiles(ui: &mut Ui, l: &Live) {
    let fm = l.frame_ms();
    let hist: Vec<f32> = l.hist.iter().map(|h| h.0).collect();
    let fps = if l.playing { format!("{:.1}", l.fps) } else { "–".into() };
    let fps_g = if l.playing { grade_fps(l.fps, l.target_fps) } else { Grade::Neutral };
    let d10 = l.dropped_10s();
    let (disk, of, disk_g) = match (l.disk_gb, l.disk_budget_gb) {
        (Some(u), Some(b)) => (format!("{u:.1} GB"), format!("of {b:.0} GB"), grade_disk(u, b)),
        (Some(u), None) => (format!("{u:.1} GB"), "no budget".into(), Grade::Neutral),
        _ => ("…".into(), "measuring".into(), Grade::Neutral),
    };
    let jobs = if l.jobs_paused {
        "paused by you"
    } else if l.playing {
        "wait while playing"
    } else {
        "running"
    };
    let specs: [TileSpec; 10] = [
        ("FPS", fps, format!("of {:.0}", l.target_fps), fps_g, Some((hist, l.target_fps * 1.1))),
        (
            "Dropped frames",
            d10.to_string(),
            format!("last 10 s · {} session", l.dropped_session),
            grade_drops(d10),
            None,
        ),
        (
            "Worst frame gap",
            format!("{:.0} ms", l.worst_gap()),
            "last 10 s · budget 50".into(),
            grade_gap(l.worst_gap()),
            None,
        ),
        ("UI frame p95", format!("{:.1} ms", l.ui_p95), "budget 8 ms".into(), grade_ui_p95(l.ui_p95), None),
        (
            "Decode",
            format!("{:.1} ms", l.decode_avg),
            format!("avg · worst {:.1}", l.decode_worst),
            grade_decode(l.decode_avg, fm),
            None,
        ),
        ("Cache hits", format!("{:.0} %", l.hit_rate * 100.0), "frames from RAM".into(), Grade::Neutral, None),
        ("RAM cache", format!("{:.0} MB", l.ram_mb), format!("of {:.0} MB", l.ram_budget_mb), Grade::Neutral, None),
        ("Frames queued", l.queued.to_string(), "ready ahead of the clock".into(), Grade::Neutral, None),
        ("Disk cache", disk, of, disk_g, None),
        ("Background jobs", l.jobs.to_string(), jobs.into(), Grade::Neutral, None),
    ];
    // a grid, as many tiles a row as fit (five at most): a Frame inside `horizontal_wrapped` never wraps
    let cols = (((ui.available_width() + 8.0) / (TILE_W + 8.0)).floor() as usize).clamp(1, 5);
    egui::Grid::new("perf_tiles").spacing(vec2(8.0, 8.0)).show(ui, |ui| {
        for (i, (caption, value, sub, g, spark)) in specs.iter().enumerate() {
            tile(ui, caption, value, sub, *g, spark.as_ref().map(|(v, m)| (v.as_slice(), *m)));
            if (i + 1) % cols == 0 {
                ui.end_row();
            }
        }
    });
}

/// The widest the content gets: the card over two rows of five tiles, like the reference card.
pub const WIDTH: f32 = 5.0 * TILE_W + 4.0 * 8.0;
/// The narrowest: two tiles a row, the legend under the donut and each row's buttons under it (a narrower
/// pane scrolls sideways).
pub const MIN_WIDTH: f32 = 2.0 * TILE_W + 8.0;
/// Under this the card stacks: donut, then legend (the card's inner width, margins taken off).
const STACK_BELOW: f32 = 210.0 + 16.0 + 200.0;

/// The content width for the space on hand.
fn width(ui: &Ui) -> f32 {
    ui.available_width().clamp(MIN_WIDTH, WIDTH)
}

/// The pane body. Returns what the user clicked, for the app to apply.
pub fn body(ui: &mut Ui, w: &mut PerfWin) -> Option<Click> {
    ui.set_width(width(ui));
    let out = card(ui, w);
    if w.tab == Tab::Live {
        ui.add_space(8.0);
        tiles(ui, &w.live);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(label: &str, ms: f32) -> Row {
        Row { label: label.into(), ms, ..Default::default() }
    }

    #[test]
    fn donut_segments_sum_to_100_and_sort_by_cost() {
        let mut rows = vec![row("a", 1.0), row("b", 6.0), row("c", 0.0), row("d", 3.0)];
        let fr = segments(&mut rows);
        assert_eq!(rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["b", "d", "a", "c"]);
        assert!((fr.iter().sum::<f32>() - 1.0).abs() < 1e-5, "shares sum to 100%: {fr:?}");
        assert!(fr.windows(2).all(|w| w[0] >= w[1]));
        assert_eq!(fr[0], 0.6);
        let mut none = vec![row("x", 0.0)];
        assert_eq!(segments(&mut none), vec![0.0], "nothing measured: no slice, no NaN");
    }

    #[test]
    fn live_sample_splits_frame_time_and_fills_idle() {
        let mut l = Live::default();
        let t0 = Instant::now();
        let mut tot = [0u64; crate::perf::NAMES.len()];
        let i = Inputs { playing: true, target_fps: 50.0, published: 0, dropped: 0, gap_ms: 0.0 };
        l.sample(tot, t0, i);
        // one second later: 100 ms of decode over 50 frames = 2 ms a frame; EMA halves the first step
        tot[crate::perf::NAMES.iter().position(|n| *n == "layer_decode").unwrap()] = 100_000;
        tot[crate::perf::NAMES.iter().position(|n| *n == "render_frame").unwrap()] = 900_000;
        l.sample(tot, t0 + Duration::from_secs(1), Inputs { published: 48, dropped: 2, gap_ms: 70.0, ..i });
        let mut rows = l.rows.clone();
        let fr = segments(&mut rows);
        assert_eq!(rows[0].label, "Idle");
        assert!((rows.iter().map(|r| r.ms).sum::<f32>() - 20.0).abs() < 1e-3, "decode + idle fill the 20 ms frame");
        assert!(rows.iter().any(|r| r.label == "Decode" && (r.ms - 1.0).abs() < 1e-3));
        assert!(!rows.iter().any(|r| r.label.contains("render")), "the render thread's wait is not a cost");
        assert!((fr.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert_eq!((l.fps, l.dropped_10s(), l.dropped_session, l.worst_gap()), (48.0, 2, 2, 70.0));
    }

    #[test]
    fn colour_thresholds() {
        assert_eq!(grade_drops(0), Grade::Good);
        assert_eq!(grade_drops(1), Grade::Warn);
        assert_eq!(grade_drops(6), Grade::Bad);
        assert_eq!(grade_gap(49.9), Grade::Good);
        assert_eq!(grade_gap(50.0), Grade::Warn);
        assert_eq!(grade_gap(100.0), Grade::Bad);
        assert_eq!(grade_ui_p95(7.9), Grade::Good);
        assert_eq!(grade_ui_p95(8.0), Grade::Warn);
        assert_eq!(grade_ui_p95(16.0), Grade::Bad);
        assert_eq!(grade_fps(59.8, 60.0), Grade::Good);
        assert_eq!(grade_fps(55.0, 60.0), Grade::Warn);
        assert_eq!(grade_fps(40.0, 60.0), Grade::Bad);
        assert_eq!(grade_decode(5.0, 16.7), Grade::Good);
        assert_eq!(grade_decode(10.0, 16.7), Grade::Warn);
        assert_eq!(grade_decode(20.0, 16.7), Grade::Bad);
        assert_eq!(grade_disk(10.0, 30.0), Grade::Good);
        assert_eq!(grade_disk(28.0, 30.0), Grade::Warn);
        assert_eq!(grade_disk(31.0, 30.0), Grade::Bad);
        assert_eq!(grade_disk(5.0, 0.0), Grade::Neutral);
        assert_eq!((Grade::Good.color(), Grade::Warn.color(), Grade::Bad.color()), (GOOD, WARN, BAD));
    }

    fn filled() -> PerfWin {
        let mut w = PerfWin { shown: true, ..Default::default() };
        w.live.rows =
            vec![row("Decode", 4.0), Row { label: "Idle".into(), ms: 10.0, idle: true, ..Default::default() }];
        w.project = vec![Row {
            label: "clip.mp4".into(),
            ms: 3.0,
            jump: Some((1, 0.0)),
            fix: Some((Fix::Proxy(1), "Make proxy")),
            ..Default::default()
        }];
        w.extra = vec![Row {
            label: "Disk cache".into(),
            value: Some("1.0 GB".into()),
            fix: Some((Fix::PruneCache, "Free disk cache")),
            ..Default::default()
        }];
        w
    }

    /// Drawn as a docked pane (a panel the size of a tab, scrolling like `perf_ctl::draw`), both tabs,
    /// wide and narrow: no repaint asked.
    #[test]
    fn assert_no_idle_repaint_performance_pane() {
        let mut w = filled();
        for size in [vec2(1200.0, 800.0), vec2(480.0, 360.0)] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            for tab in [Tab::Live, Tab::Project] {
                w.tab = tab;
                for _ in 0..30 {
                    let input = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    };
                    let _ = ctx.run(input, |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| body(ui, &mut w));
                        });
                    });
                }
                assert!(!ctx.has_requested_repaint(), "the Performance pane repainted while idle ({tab:?}, {size:?})");
            }
        }
    }

    /// The pane fits the tab it is in: the full card at its widest, narrower in a narrow tab (the Export
    /// page's Jobs column, about 316 px), never under the minimum (the pane scrolls sideways instead).
    #[test]
    fn performance_pane_follows_the_tab_width() {
        let mut w = filled();
        for (tab, avail, want) in [
            (Tab::Live, 1200.0, WIDTH),
            (Tab::Live, 600.0, 600.0),
            (Tab::Live, 316.0, 316.0),
            (Tab::Project, 316.0, 316.0),
            (Tab::Live, 250.0, MIN_WIDTH),
        ] {
            w.tab = tab;
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            let mut got = 0.0;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(avail, 900.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    got = ui.scope(|ui| body(ui, &mut w)).response.rect.width();
                });
            });
            assert!((got - want).abs() < 1.0, "{tab:?} {avail} wide: the pane drew {got}, not {want}");
        }
    }
}
