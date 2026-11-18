//! ---- ws:beat-aligner ----
//! The Beat Aligner's sheet: one song (or voiceover) with a tempo grid, the marks placed on it (tapped,
//! filled from the grid, or detected phrases) and named sections. Times are SOURCE seconds of the song,
//! so marks stay on the music when its clip moves; `BeatMap` turns them into timeline time.
//! Pure functions only - the edits that use the marks on the timeline are `model::ops::beats`.

use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkKind {
    /// Placed by hand (a tap, a click, an import).
    #[default]
    Tap,
    /// A grid beat (continuous mode / Fill grid).
    Beat,
    /// The first beat of a bar.
    Downbeat,
    /// A voiceover phrase starting.
    Phrase,
    /// A voiceover pause starting.
    Pause,
}

impl MarkKind {
    pub fn name(self) -> &'static str {
        match self {
            MarkKind::Tap => "Tap",
            MarkKind::Beat => "Beat",
            MarkKind::Downbeat => "Downbeat",
            MarkKind::Phrase => "Phrase",
            MarkKind::Pause => "Pause",
        }
    }
    pub fn parse(s: &str) -> MarkKind {
        [MarkKind::Beat, MarkKind::Downbeat, MarkKind::Phrase, MarkKind::Pause]
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(s.trim()))
            .unwrap_or(MarkKind::Tap)
    }
    /// The project label (1-based, `LABEL_COLORS` order) a timeline marker of this kind gets.
    pub fn label(self) -> u8 {
        match self {
            MarkKind::Tap => 3,      // yellow
            MarkKind::Beat => 8,     // gray
            MarkKind::Downbeat => 2, // orange
            MarkKind::Phrase => 4,   // green
            MarkKind::Pause => 8,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BeatMark {
    pub id: Id,
    /// Source seconds of the song.
    pub t: f64,
    pub kind: MarkKind,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: Id,
    /// Source seconds, a < b.
    pub a: f64,
    pub b: f64,
    pub name: String,
    /// Project label index (1-based) - its colour.
    pub label: u8,
}

/// The section names offered as one-click buttons, with their label (colour).
pub const SECTION_PRESETS: [(&str, u8); 8] = [
    ("Intro", 5),
    ("Verse", 6),
    ("Pre-chorus", 3),
    ("Chorus", 2),
    ("Drop", 1),
    ("Bridge", 7),
    ("Break", 4),
    ("Outro", 8),
];

/// The label a section called `name` gets: its preset's, else green.
pub fn section_label(name: &str) -> u8 {
    SECTION_PRESETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).map_or(4, |(_, l)| *l)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BeatSheet {
    /// The song's asset (0 = none picked).
    pub asset: Id,
    /// The timeline clip that plays it (0 = none: source time is timeline time).
    pub clip: Id,
    /// Voiceover: phrases and pauses instead of a beat grid.
    pub voice: bool,
    /// Tempo (0 = no grid yet).
    pub bpm: f64,
    /// Source second of a downbeat.
    pub offset: f64,
    /// Beats per bar.
    pub meter: u32,
    pub marks: Vec<BeatMark>,
    pub sections: Vec<Section>,
    pub(crate) next: Id,
}

impl Default for BeatSheet {
    fn default() -> Self {
        Self {
            asset: 0,
            clip: 0,
            voice: false,
            bpm: 0.0,
            offset: 0.0,
            meter: 4,
            marks: Vec::new(),
            sections: Vec::new(),
            next: 1,
        }
    }
}

/// Nothing on the sheet (not written to the project file).
pub fn is_empty(s: &BeatSheet) -> bool {
    *s == BeatSheet::default()
}

/// Source <-> timeline for the sheet's clip (identity without one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatMap {
    pub start: f64,
    pub src_in: f64,
    pub speed: f64,
}

impl Default for BeatMap {
    fn default() -> Self {
        Self { start: 0.0, src_in: 0.0, speed: 1.0 }
    }
}

impl BeatMap {
    pub fn of(c: &Clip) -> BeatMap {
        BeatMap { start: c.start, src_in: c.src_in, speed: if c.speed > 0.0 { c.speed } else { 1.0 } }
    }
    pub fn to_tl(&self, src: f64) -> f64 {
        self.start + (src - self.src_in) / self.speed
    }
    pub fn to_src(&self, tl: f64) -> f64 {
        self.src_in + (tl - self.start) * self.speed
    }
}

/// Marks closer than this are the same mark (a fill never doubles a tap).
const SAME: f64 = 0.02;

impl BeatSheet {
    fn id(&mut self) -> Id {
        self.next = self.next.max(1);
        let id = self.next;
        self.next += 1;
        id
    }

    pub fn period(&self) -> Option<f64> {
        (self.bpm > 0.0 && self.bpm.is_finite()).then(|| 60.0 / self.bpm)
    }

    /// Grid beats in [a, b] as (time, is a downbeat); `div` subdivides each beat (1, 2, 4).
    pub fn grid(&self, a: f64, b: f64, div: u32) -> Vec<(f64, bool)> {
        let Some(p) = self.period() else { return Vec::new() };
        let step = p / div.max(1) as f64;
        let bar = (self.meter.max(1) * div.max(1)) as i64;
        let k0 = ((a - self.offset) / step).ceil() as i64;
        let k1 = ((b - self.offset) / step).floor() as i64;
        if k1 < k0 || k1 - k0 > 200_000 {
            return Vec::new();
        }
        (k0..=k1).map(|k| (self.offset + k as f64 * step, k.rem_euclid(bar) == 0)).collect()
    }

    /// The grid line (beat / `div`) nearest `t`.
    pub fn nearest(&self, t: f64, div: u32) -> Option<f64> {
        let step = self.period()? / div.max(1) as f64;
        Some(self.offset + ((t - self.offset) / step).round() * step)
    }

    /// `t` pulled toward the nearest grid line by `strength` (0 = untouched, 1 = on the line).
    pub fn snap(&self, t: f64, strength: f64, div: u32) -> f64 {
        match self.nearest(t, div) {
            Some(b) => t + (b - t) * strength.clamp(0.0, 1.0),
            None => t,
        }
    }

    fn sort(&mut self) {
        self.marks.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.sections.sort_by(|a, b| a.a.total_cmp(&b.a));
    }

    pub fn add_mark(&mut self, t: f64, kind: MarkKind) -> Id {
        let id = self.id();
        self.marks.push(BeatMark { id, t: t.max(0.0), kind });
        self.sort();
        id
    }

    /// A tap heard at song time `heard` (the player clock at the key press): the input latency taken
    /// off, then pulled toward the grid when `snap` = Some((strength, div)).
    pub fn tap(&mut self, heard: f64, latency: f64, snap: Option<(f64, u32)>) -> Id {
        let mut t = heard - latency;
        if let Some((s, div)) = snap {
            t = self.snap(t, s, div);
        }
        self.add_mark(t, MarkKind::Tap)
    }

    /// Grid beats in (a, b] that have no mark yet become Beat / Downbeat marks (continuous mode adds
    /// the stretch played since last frame; Fill grid the whole song). Returns how many.
    pub fn fill(&mut self, a: f64, b: f64) -> usize {
        let mut n = 0;
        for (t, down) in self.grid(a.max(0.0), b, 1) {
            if t <= a || self.marks.iter().any(|m| (m.t - t).abs() < SAME) {
                continue;
            }
            let id = self.id();
            self.marks.push(BeatMark { id, t, kind: if down { MarkKind::Downbeat } else { MarkKind::Beat } });
            n += 1;
        }
        self.sort();
        n
    }

    /// Move marks by `dt`; with `div` each moved mark lands on its nearest grid line.
    pub fn move_marks(&mut self, ids: &[Id], dt: f64, div: Option<u32>) {
        let grid = self.clone();
        for m in self.marks.iter_mut().filter(|m| ids.contains(&m.id)) {
            let t = (m.t + dt).max(0.0);
            m.t = div.and_then(|d| grid.nearest(t, d)).unwrap_or(t).max(0.0);
        }
        self.sort();
    }

    pub fn delete_marks(&mut self, ids: &[Id]) -> usize {
        let n = self.marks.len();
        self.marks.retain(|m| !ids.contains(&m.id));
        n - self.marks.len()
    }

    pub fn add_section(&mut self, a: f64, b: f64, name: &str) -> Id {
        let id = self.id();
        let (a, b) = (a.min(b).max(0.0), a.max(b).max(0.0));
        self.sections.push(Section { id, a, b, name: name.trim().to_string(), label: section_label(name) });
        self.sort();
        id
    }

    pub fn section_mut(&mut self, id: Id) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.id == id)
    }

    pub fn delete_section(&mut self, id: Id) -> bool {
        let n = self.sections.len();
        self.sections.retain(|s| s.id != id);
        n != self.sections.len()
    }

    /// Halve / double the tempo, keeping the downbeat.
    pub fn scale_tempo(&mut self, by: f64) {
        if self.bpm > 0.0 {
            self.bpm = (self.bpm * by).clamp(20.0, 400.0);
        }
    }

    /// Put the downbeat on whichever beat of the bar is loudest across the song (`level` at a source
    /// second, `dur` the song's length).
    pub fn pick_downbeat(&mut self, level: impl Fn(f64) -> f32, dur: f64) {
        let Some(p) = self.period() else { return };
        let meter = self.meter.max(1) as usize;
        let mut sum = vec![0f32; meter];
        for (i, (t, _)) in self.grid(0.0, dur, 1).iter().enumerate() {
            sum[i % meter] += level(*t);
        }
        // the first grid beat at or after 0 is beat index 0 of `sum`
        let first = self.grid(0.0, dur, 1).first().map_or(self.offset, |g| g.0);
        if let Some((j, _)) = sum.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) {
            self.offset = (first + j as f64 * p) % (p * meter as f64);
        }
    }
}

/// Tempo and phase from onset times: the period whose comb lines up the most onsets (a phase-free
/// Fourier magnitude over 60-200 BPM, the slower of near-equal answers), then a least-squares fit of the
/// onsets near that grid for a precise tempo. Returns (bpm, offset of one beat).
pub fn fit_grid(onsets: &[f64]) -> Option<(f64, f64)> {
    if onsets.len() < 4 {
        return None;
    }
    let comb = |bpm: f64| {
        let w = std::f64::consts::TAU * bpm / 60.0;
        let (s, c) = onsets.iter().fold((0.0, 0.0), |(s, c), &t| (s + (w * t).sin(), c + (w * t).cos()));
        (s.hypot(c), s.atan2(c))
    };
    // ponytail: brute-force scan, 2800 candidates x onsets (~10 ms for a 4-minute song); a coarse-to-fine
    // search if songs ever get long enough to notice
    let mut best = (0.0, 0.0, 0.0); // (score, bpm, phase)
    let mut bpm = 60.0;
    while bpm <= 200.0 {
        let (score, ph) = comb(bpm);
        if score > best.0 * 1.03 {
            best = (score, bpm, ph);
        }
        bpm += 0.05;
    }
    if best.0 < onsets.len() as f64 * 0.2 {
        return None; // nothing periodic
    }
    let mut p = 60.0 / best.1;
    let mut off = (best.2 / std::f64::consts::TAU * p).rem_euclid(p);
    // least squares t = off + k p over the onsets within a quarter beat of the grid, twice
    for _ in 0..2 {
        let pts: Vec<(f64, f64)> = onsets
            .iter()
            .filter_map(|&t| {
                let k = ((t - off) / p).round();
                ((t - off - k * p).abs() < p / 4.0).then_some((k, t))
            })
            .collect();
        if pts.len() < 4 {
            break;
        }
        let n = pts.len() as f64;
        let (sk, st) = pts.iter().fold((0.0, 0.0), |a, &(k, t)| (a.0 + k, a.1 + t));
        let (mk, mt) = (sk / n, st / n);
        let (cov, var) = pts.iter().fold((0.0, 0.0), |a, &(k, t)| (a.0 + (k - mk) * (t - mt), a.1 + (k - mk).powi(2)));
        if var <= 0.0 {
            break;
        }
        p = cov / var;
        off = (mt - p * mk).rem_euclid(p);
    }
    (p > 0.0).then(|| (60.0 / p, off))
}

/// Tempo from tap times (the last 8): the mean interval.
pub fn tap_tempo(taps: &[f64]) -> Option<f64> {
    let t = &taps[taps.len().saturating_sub(8)..];
    if t.len() < 3 {
        return None;
    }
    let iv = (t[t.len() - 1] - t[0]) / (t.len() - 1) as f64;
    (iv > 0.15 && iv < 3.0).then(|| 60.0 / iv)
}

/// Input latency from calibration taps (song seconds as heard, before any correction) against the grid:
/// the median distance to the nearest beat. Positive = the taps land late.
pub fn calibrate(sheet: &BeatSheet, heard: &[f64]) -> Option<f64> {
    let mut d: Vec<f64> = heard.iter().filter_map(|&t| sheet.nearest(t, 1).map(|b| t - b)).collect();
    if d.len() < 4 {
        return None;
    }
    d.sort_by(f64::total_cmp);
    Some(d[d.len() / 2])
}

/// Voiceover phrases from the waveform: a Phrase mark where speech starts after a pause, a Pause mark
/// where it stops (source seconds in [a, b)). Reuses auto-cut's speech detection.
pub fn phrases(peaks: &crate::media::waveform::Peaks, a: f64, b: f64, min_pause: f64) -> Vec<(f64, MarkKind)> {
    let params = crate::engine::autocut::AutoCutParams { min_silence: min_pause, padding: 0.0, ..Default::default() };
    let mut out = Vec::new();
    for (s, e) in crate::engine::autocut::loud_segments(peaks, a, b - a, &params) {
        out.push((s, MarkKind::Phrase));
        if e < b - 0.05 {
            out.push((e, MarkKind::Pause));
        }
    }
    out
}

// ---------- marker files ----------

/// Formats the sheet's marks can be written to / read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeatFmt {
    /// `time,duration,name,note,label` - the Markers panel's own CSV (`markers.import` reads it too).
    Csv,
    /// A CMX 3600 EDL with one marker per event (`|C:` colour, `|M:` name, `|D:` frames), as DaVinci
    /// Resolve writes and reads timeline markers.
    Edl,
    /// `HH:MM:SS Name` per section (YouTube chapters; the marks when there are no sections).
    Chapters,
}

impl BeatFmt {
    pub fn parse(s: &str) -> Option<BeatFmt> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Some(BeatFmt::Csv),
            "edl" => Some(BeatFmt::Edl),
            "chapters" | "youtube" | "youtube_chapters" | "txt" => Some(BeatFmt::Chapters),
            _ => None,
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            BeatFmt::Csv => "csv",
            BeatFmt::Edl => "edl",
            BeatFmt::Chapters => "txt",
        }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_split(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                out.last_mut().unwrap().push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

const RESOLVE_COLORS: [&str; 8] = ["Red", "Yellow", "Yellow", "Green", "Cyan", "Blue", "Purple", "Sand"]; // by LABEL_COLORS index

/// Everything on the sheet as (timeline time, duration, name, label): marks then sections, by time.
fn rows(sheet: &BeatSheet, map: BeatMap) -> Vec<(f64, f64, String, u8)> {
    let mut v: Vec<(f64, f64, String, u8)> =
        sheet.marks.iter().map(|m| (map.to_tl(m.t), 0.0, m.kind.name().to_string(), m.kind.label())).collect();
    v.extend(sheet.sections.iter().map(|s| (map.to_tl(s.a), (s.b - s.a) / map.speed, s.name.clone(), s.label)));
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

/// The sheet as a marker file, in timeline time.
pub fn export(sheet: &BeatSheet, map: BeatMap, fps: f64, fmt: BeatFmt) -> String {
    let mut s = String::new();
    match fmt {
        BeatFmt::Csv => {
            s.push_str("time,duration,name,note,label\n");
            for (t, d, name, label) in rows(sheet, map) {
                s.push_str(&format!("{t:.3},{d:.3},{},,{label}\n", csv_field(&name)));
            }
        }
        BeatFmt::Edl => {
            let fps = fps.max(1.0);
            s.push_str("TITLE: Beat Aligner\nFCM: NON-DROP FRAME\n\n");
            for (i, (t, d, name, label)) in rows(sheet, map).into_iter().enumerate() {
                let f = (t.max(0.0) * fps).round() as i64;
                let (a, b) = (crate::engine::edl::tc(f, fps), crate::engine::edl::tc(f + 1, fps));
                let color = RESOLVE_COLORS.get((label as usize).wrapping_sub(1)).unwrap_or(&"Blue");
                let frames = ((d * fps).round() as i64).max(1);
                s.push_str(&format!("{:03}  001      V     C        {a} {b} {a} {b}  \n", i + 1));
                s.push_str(&format!(
                    " |C:ResolveColor{color} |M:{} |D:{frames}\n\n",
                    name.replace(['|', '\n', '\r'], " ")
                ));
            }
        }
        BeatFmt::Chapters => {
            let hms = |t: f64| {
                let x = t.max(0.0).round() as u64;
                format!("{:02}:{:02}:{:02}", x / 3600, (x / 60) % 60, x % 60)
            };
            let mut list: Vec<(f64, String)> = if sheet.sections.is_empty() {
                rows(sheet, map).into_iter().map(|r| (r.0, r.2)).collect()
            } else {
                sheet.sections.iter().map(|x| (map.to_tl(x.a), x.name.clone())).collect()
            };
            if list.first().is_none_or(|r| r.0 >= 1.0) {
                list.insert(0, (0.0, "Intro".into()));
            }
            for (t, name) in list {
                s.push_str(&format!("{} {}\n", hms(t), if name.is_empty() { "Chapter" } else { &name }));
            }
        }
    }
    s
}

/// `HH:MM:SS[.ms]`, `MM:SS` or `HH:MM:SS:FF` (with `fps`) -> seconds.
fn parse_clock(s: &str, fps: f64) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    let n: Vec<f64> = parts.iter().map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?;
    match n.as_slice() {
        [m, s] => Some(m * 60.0 + s),
        [h, m, s] => Some(h * 3600.0 + m * 60.0 + s),
        [h, m, s, f] => Some(h * 3600.0 + m * 60.0 + s + f / fps.max(1.0)),
        _ => None,
    }
}

/// Read a CSV / EDL / chapters file (whichever it looks like) onto the sheet: entries with a duration
/// become sections, the rest marks (kind from the name). Timeline time in the file. Returns how many.
pub fn import(sheet: &mut BeatSheet, map: BeatMap, fps: f64, text: &str) -> usize {
    let text = text.trim_start_matches('\u{feff}');
    let mut got: Vec<(f64, f64, String)> = Vec::new();
    if text.contains("|M:") || text.contains("* LOC:") {
        let mut rec_in: Option<f64> = None;
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() >= 8 && f[0].chars().all(|c| c.is_ascii_digit()) {
                rec_in = parse_clock(f[6], fps);
            } else if let Some(i) = line.find("|M:") {
                let rest = &line[i + 3..];
                let name = rest.split(" |").next().unwrap_or("").trim().to_string();
                let frames = line.split("|D:").nth(1).and_then(|d| d.trim().parse::<f64>().ok()).unwrap_or(1.0);
                if let Some(t) = rec_in.take() {
                    got.push((t, if frames > 1.0 { frames / fps.max(1.0) } else { 0.0 }, name));
                }
            } else if let Some(i) = line.find("* LOC:") {
                let f: Vec<&str> = line[i + 6..].split_whitespace().collect();
                if let Some(t) = f.first().and_then(|tc| parse_clock(tc, fps)) {
                    got.push((t, 0.0, f.get(2..).map(|r| r.join(" ")).unwrap_or_default()));
                }
            }
        }
    } else if text
        .lines()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| l.split_whitespace().next().is_some_and(|w| w.contains(':') && parse_clock(w, fps).is_some()))
    {
        // chapters: each line runs until the next one, so they come back as sections
        let mut list: Vec<(f64, String)> = text
            .lines()
            .filter_map(|l| {
                let (w, rest) = l.trim().split_once(char::is_whitespace).unwrap_or((l.trim(), ""));
                Some((parse_clock(w, fps)?, rest.trim().to_string()))
            })
            .collect();
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (i, (t, name)) in list.iter().enumerate() {
            let end = list.get(i + 1).map(|n| n.0);
            got.push((*t, end.map_or(0.0, |e| e - t), name.clone()));
        }
    } else {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let first = lines.next().unwrap_or("");
        let head: Vec<String> = csv_split(first).iter().map(|h| h.trim().to_ascii_lowercase()).collect();
        let has_head = head.first().is_some_and(|h| h.parse::<f64>().is_err());
        let col = |n: &str, d: usize| if has_head { head.iter().position(|h| h == n).unwrap_or(d) } else { d };
        let (ti, di, ni) =
            (col("time", 0), if has_head { col("duration", usize::MAX) } else { usize::MAX }, col("name", 1));
        let body: Vec<&str> = if has_head { lines.collect() } else { std::iter::once(first).chain(lines).collect() };
        for l in body {
            let f = csv_split(l);
            let Some(t) = f.get(ti).and_then(|s| s.trim().parse::<f64>().ok()) else { continue };
            let d = f.get(di).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
            got.push((t, d.max(0.0), f.get(ni).map(|s| s.trim().to_string()).unwrap_or_default()));
        }
    }
    for (t, d, name) in &got {
        let a = map.to_src(*t);
        if *d > 0.0 {
            sheet.add_section(a, a + d * map.speed, name);
        } else {
            sheet.add_mark(a, MarkKind::parse(name));
        }
    }
    got.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::AudioSource;

    /// A click track: a 5 ms burst of noise on every beat at `bpm`, silence between.
    struct Clicks {
        bpm: f64,
        dur: f64,
    }
    impl AudioSource for Clicks {
        fn duration(&self) -> f64 {
            self.dur
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            let sr = crate::media::SAMPLE_RATE as f64;
            let p = 60.0 / self.bpm;
            for (i, s) in out.chunks_exact_mut(2).enumerate() {
                let x = t + i as f64 / sr;
                let ph = x.rem_euclid(p);
                let v = if x < self.dur && ph < 0.005 {
                    if (i * 7919) % 3 == 0 {
                        0.9
                    } else {
                        -0.9
                    }
                } else {
                    0.0
                };
                s[0] = v;
                s[1] = v;
            }
        }
    }

    #[test]
    fn bpm_of_a_click_track_is_within_half_a_bpm() {
        for bpm in [92.0, 120.0, 128.0, 143.5, 174.0] {
            // the real pipeline: the whole-song analysis the visualizer caches, then the grid fit
            let a = crate::engine::visualizer::SongAnalysis::compute(&mut Clicks { bpm, dur: 30.0 });
            let (got, off) = fit_grid(&a.beats).unwrap_or_else(|| panic!("no tempo for {bpm}"));
            assert!((got - bpm).abs() <= 0.5, "{bpm} BPM detected as {got}");
            let p = 60.0 / bpm;
            let phase = off.rem_euclid(p).min(p - off.rem_euclid(p));
            assert!(phase < 0.02, "{bpm}: the grid is off the clicks by {phase} s");
        }
    }

    #[test]
    fn fit_prefers_the_beat_over_double_time() {
        let onsets: Vec<f64> = (0..40).map(|k| 0.3 + k as f64 * 0.75).collect(); // 80 BPM
        let (bpm, off) = fit_grid(&onsets).unwrap();
        assert!((bpm - 80.0).abs() < 0.05, "{bpm}");
        assert!((off - 0.3).abs() < 0.01, "{off}");
        assert!(fit_grid(&[1.0, 2.0]).is_none());
    }

    #[test]
    fn taps_take_the_latency_off_and_snap() {
        let mut s = BeatSheet { bpm: 120.0, offset: 0.0, ..Default::default() };
        // heard at 1.08 with 60 ms of latency = 1.02; no snap keeps it
        let id = s.tap(1.08, 0.06, None);
        assert!((s.marks.iter().find(|m| m.id == id).unwrap().t - 1.02).abs() < 1e-9);
        // full snap lands on the beat at 1.0; half strength halfway
        let id = s.tap(1.58, 0.06, Some((1.0, 1)));
        assert!((s.marks.iter().find(|m| m.id == id).unwrap().t - 1.5).abs() < 1e-9);
        let id = s.tap(2.14, 0.06, Some((0.5, 1)));
        assert!((s.marks.iter().find(|m| m.id == id).unwrap().t - 2.04).abs() < 1e-9);
        // eighth-note snapping
        assert!((s.snap(1.3, 1.0, 2) - 1.25).abs() < 1e-9);
        assert_eq!(s.snap(1.3, 1.0, 1), 1.5);
        let lat = calibrate(&s, &[0.53, 1.05, 1.56, 2.04, 2.55]).unwrap();
        assert!((lat - 0.05).abs() < 1e-9, "{lat}");
        assert!((tap_tempo(&[0.0, 0.5, 1.0, 1.5]).unwrap() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn fill_marks_downbeats_and_never_doubles() {
        let mut s = BeatSheet { bpm: 120.0, offset: 0.5, meter: 4, ..Default::default() };
        s.add_mark(1.005, MarkKind::Tap);
        let n = s.fill(-1.0, 4.0); // beats 0, .5, 1 ... 4 -> 9, one already tapped
        assert_eq!(n, 8);
        assert_eq!(s.marks.len(), 9);
        let downs: Vec<f64> = s.marks.iter().filter(|m| m.kind == MarkKind::Downbeat).map(|m| m.t).collect();
        assert_eq!(downs, vec![0.5, 2.5]);
        assert_eq!(s.fill(-1.0, 4.0), 0);
        // continuous: only what was played since last frame
        assert_eq!(s.fill(4.0, 5.0), 2);
    }

    #[test]
    fn marks_move_with_snapping_and_delete() {
        let mut s = BeatSheet { bpm: 120.0, ..Default::default() };
        let a = s.add_mark(1.0, MarkKind::Tap);
        let b = s.add_mark(2.0, MarkKind::Tap);
        s.move_marks(&[a, b], 0.2, Some(4)); // 1.2 -> 1.25, 2.2 -> 2.25
        assert_eq!(s.marks.iter().map(|m| m.t).collect::<Vec<_>>(), vec![1.25, 2.25]);
        s.move_marks(&[a], 0.03, None);
        assert!((s.marks[0].t - 1.28).abs() < 1e-9);
        assert_eq!(s.delete_marks(&[a]), 1);
        assert_eq!(s.marks.len(), 1);
    }

    #[test]
    fn downbeat_follows_the_loud_beat() {
        let mut s = BeatSheet { bpm: 120.0, offset: 0.0, meter: 4, ..Default::default() };
        // loud on beats 1, 5, 9 ... = 0.5 s, 2.5 s, ...
        s.pick_downbeat(|t| if ((t - 0.5) / 2.0).fract().abs() < 1e-6 { 1.0 } else { 0.2 }, 20.0);
        assert!((s.offset - 0.5).abs() < 1e-9, "{}", s.offset);
    }

    #[test]
    fn marker_files_round_trip() {
        let mut s = BeatSheet { bpm: 120.0, ..Default::default() };
        s.add_mark(1.0, MarkKind::Tap);
        s.add_mark(2.0, MarkKind::Downbeat);
        s.add_section(4.0, 12.0, "Chorus");
        s.add_section(12.0, 20.0, "My bit, with a comma");
        let map = BeatMap { start: 10.0, src_in: 2.0, speed: 1.0 };
        for fmt in [BeatFmt::Csv, BeatFmt::Edl] {
            let text = export(&s, map, 30.0, fmt);
            let mut back = BeatSheet::default();
            assert_eq!(import(&mut back, map, 30.0, &text), 4, "{fmt:?}:\n{text}");
            let marks: Vec<(i64, MarkKind)> =
                back.marks.iter().map(|m| ((m.t * 30.0).round() as i64, m.kind)).collect();
            assert_eq!(marks, vec![(30, MarkKind::Tap), (60, MarkKind::Downbeat)], "{fmt:?}");
            let secs: Vec<(i64, i64, &str, u8)> = back
                .sections
                .iter()
                .map(|x| ((x.a * 30.0).round() as i64, (x.b * 30.0).round() as i64, x.name.as_str(), x.label))
                .collect();
            assert_eq!(secs, vec![(120, 360, "Chorus", 2), (360, 600, "My bit, with a comma", 4)], "{fmt:?}");
        }
        // chapters: sections only, each up to the next
        let text = export(&s, map, 30.0, BeatFmt::Chapters);
        assert_eq!(text, "00:00:00 Intro\n00:00:12 Chorus\n00:00:20 My bit, with a comma\n");
        let mut back = BeatSheet::default();
        assert_eq!(import(&mut back, BeatMap::default(), 30.0, &text), 3);
        assert_eq!(back.sections.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["Intro", "Chorus"]);
        assert_eq!(back.marks.len(), 1, "the last chapter has no end: a mark");
        // the Markers panel's CSV reads the same file
        let mut p = Project::new();
        assert_eq!(crate::ui::markers_ui::import_markers_csv(&mut p, &export(&s, map, 30.0, BeatFmt::Csv)), 4);
    }

    #[test]
    fn sections_rename_and_delete() {
        let mut s = BeatSheet::default();
        let a = s.add_section(8.0, 4.0, "Drop");
        assert_eq!((s.sections[0].a, s.sections[0].b, s.sections[0].label), (4.0, 8.0, 1));
        s.section_mut(a).unwrap().name = "Big drop".into();
        assert_eq!(s.sections[0].name, "Big drop");
        assert!(s.delete_section(a) && s.sections.is_empty());
    }
}
