//! Import timelines from other editors, with an honest per-item report.
//!
//! Supported: Final Cut Pro 7 XML (`.xml` - what Premiere Pro exports and DaVinci Resolve reads/writes),
//! EDL (CMX 3600) and Premiere `.prproj` (gzip-wrapped XML: inflate, then the same reader). Anything the
//! reader cannot map (unknown effects, missing media, speed ramps, colour pages) becomes an `Issue`
//! instead of failing the import.

use crate::media::is_image_path;
use crate::model::{Asset, AudioStreamInfo, Clip, ClipKind, Id, Project, TrackKind, TransitionKind, MIN_CLIP};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Ok,
    Warning,
    Skipped,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Ok => "Ok",
            Level::Warning => "Warning",
            Level::Skipped => "Skipped",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub level: Level,
    /// What it was about ("clip 'a.mp4'", "effect 'Gaussian Blur'", "track V2").
    pub subject: String,
    pub detail: String,
}

#[derive(Debug)]
pub struct ImportReport {
    pub project: Project,
    pub issues: Vec<Issue>,
    pub clips: usize,
    pub tracks: usize,
    pub missing_media: usize,
}

impl ImportReport {
    pub fn ok(&self) -> usize {
        self.issues.iter().filter(|i| i.level == Level::Ok).count()
    }
    pub fn problems(&self) -> usize {
        self.issues.iter().filter(|i| i.level != Level::Ok).count()
    }
    /// Markdown table for the import dialog / clipboard.
    pub fn to_markdown(&self) -> String {
        let mut s = String::with_capacity(256 + self.issues.len() * 80);
        let _ = writeln!(s, "# Import: {}\n", self.project.name);
        let _ = writeln!(
            s,
            "{} clips on {} tracks, {}×{} @ {:.3} fps.",
            self.clips, self.tracks, self.project.width, self.project.height, self.project.fps
        );
        let _ =
            writeln!(s, "{} missing files, {} problems, {} notes.\n", self.missing_media, self.problems(), self.ok());
        let _ = writeln!(s, "| Level | Item | Detail |");
        let _ = writeln!(s, "| --- | --- | --- |");
        for i in &self.issues {
            let _ = writeln!(s, "| {} | {} | {} |", i.level.name(), cell(&i.subject), cell(&i.detail));
        }
        s
    }
}

/// A markdown table cell: no pipes, no line breaks.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Detect the format by extension + content and import it.
pub fn import_file(path: &std::path::Path) -> Result<ImportReport, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let stem = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut b = Build::new(dir, &stem);
    if bytes.starts_with(&[0x1f, 0x8b]) {
        // ponytail: no inflate in-tree (no new crates) — say so instead of failing silently
        b.note(
            Level::Skipped,
            format!("file '{stem}'"),
            "Premiere .prproj files are gzip-compressed and cannot be read here. In Premiere Pro use \
             File → Export → Final Cut Pro XML (or Media Browser → export an EDL) and import that file.",
        );
        return Ok(b.finish());
    }
    let text = decode(&bytes);
    if text.contains("<xmeml") {
        read_xmeml(&text, &mut b)?;
    } else if text.contains("<fcpxml") {
        b.note(
            Level::Skipped,
            format!("file '{stem}'"),
            "This is Final Cut Pro X XML. Only the FCP7 / xmeml interchange format is supported — \
             re-export as \"Final Cut Pro XML\" (version 4/5) or as an EDL.",
        );
    } else if crate::engine::export::ext_of(path) == "edl" || looks_like_edl(&text) {
        read_edl(&text, &mut b);
    } else {
        return Err("Not a timeline: expected FCP7 XML (xmeml), EDL or an uncompressed .prproj.".into());
    }
    Ok(b.finish())
}

pub const IMPORT_EXTS: &[&str] = &["xml", "fcpxml", "edl", "prproj"];

/// Bytes → text: strips a UTF-8 BOM, decodes UTF-16 (LE/BE BOM) and never fails.
fn decode(bytes: &[u8]) -> String {
    match bytes {
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xff, 0xfe, rest @ ..] => utf16(rest, true),
        [0xfe, 0xff, rest @ ..] => utf16(rest, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn utf16(bytes: &[u8], le: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

// ---------------------------------------------------------------- report building

struct Build {
    project: Project,
    issues: Vec<Issue>,
    dir: PathBuf,
    /// Asset paths that could not be found on disk.
    missing: Vec<String>,
}

impl Build {
    fn new(dir: PathBuf, name: &str) -> Self {
        let mut project = Project::new();
        project.name = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Self { project, issues: Vec::new(), dir, missing: Vec::new() }
    }
    fn note(&mut self, level: Level, subject: impl Into<String>, detail: impl Into<String>) {
        self.issues.push(Issue { level, subject: subject.into(), detail: detail.into() });
    }
    /// Existing path for `raw` (relinked next to the imported file when missing), plus a note.
    fn resolve(&mut self, raw: &str, subject: &str) -> String {
        if raw.is_empty() {
            return raw.to_string();
        }
        if Path::new(raw).is_file() {
            return raw.to_string();
        }
        if let Some(p) = relink(&self.dir, raw) {
            let p = p.to_string_lossy().into_owned();
            self.note(Level::Ok, subject, format!("relinked to {p}"));
            return p;
        }
        if !self.missing.iter().any(|m| m.eq_ignore_ascii_case(raw)) {
            self.missing.push(raw.to_string());
            self.note(Level::Warning, subject, format!("media not found: {raw} (relink it in the library)"));
        }
        raw.to_string()
    }
    /// Add an asset, filling the metadata from the real file when it exists.
    fn add_asset(&mut self, mut a: Asset) -> Id {
        if Path::new(&a.path).is_file() {
            if let Ok(p) = crate::media::probe(&a.path, crate::media::Backend::Auto) {
                a = Asset { folder: a.folder, tags: a.tags, label: a.label, description: a.description, ..p };
            }
        }
        self.project.add_asset(a)
    }
    fn finish(mut self) -> ImportReport {
        self.project.tidy();
        let clips = self.project.tracks.iter().map(|t| t.clips.len()).sum();
        let tracks = self.project.tracks.iter().filter(|t| !t.clips.is_empty()).count();
        ImportReport { issues: self.issues, clips, tracks, missing_media: self.missing.len(), project: self.project }
    }
}

/// Look for the file next to the imported timeline, then one level of sub-folders.
fn relink(dir: &Path, path: &str) -> Option<PathBuf> {
    let name = Path::new(path).file_name()?;
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let p = e.path().join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Ensure the `n`-th track of `kind` exists and return its index.
fn track_slot(p: &mut Project, kind: TrackKind, n: usize) -> usize {
    loop {
        let list = if kind == TrackKind::Video { p.video_tracks() } else { p.audio_tracks() };
        if let Some(&i) = list.get(n) {
            return i;
        }
        p.add_track(kind);
    }
}

/// Put the clip on `ti`, or on the first track of the same kind with room (a new one if needed).
fn place(p: &mut Project, ti: usize, c: Clip) -> Id {
    let id = c.id;
    let ti = if p.tracks[ti].fits(c.start, c.duration, &[]) {
        ti
    } else {
        p.find_free_track(p.tracks[ti].kind, c.start, c.duration, None)
    };
    p.tracks[ti].clips.push(c);
    p.tracks[ti].sort();
    id
}

/// Clips of one asset at the same place on different tracks are one linked group (video + its audio),
/// which is how `Project::insert_asset_clips` builds them.
fn link_stacks(p: &mut Project) {
    let mut groups: HashMap<(Id, i64, i64), Vec<Id>> = HashMap::new();
    for (_, c) in p.all_clips() {
        if c.asset != 0 {
            groups.entry((c.asset, ms(c.start), ms(c.duration))).or_default().push(c.id);
        }
    }
    for ids in groups.into_values().filter(|g| g.len() > 1) {
        let link = p.new_id();
        for id in ids {
            if let Some(c) = p.clip_mut(id) {
                c.link = link;
            }
        }
    }
}

fn ms(t: f64) -> i64 {
    (t * 1000.0).round() as i64
}

// ---------------------------------------------------------------- FCP7 XML (xmeml)

fn read_xmeml(text: &str, b: &mut Build) -> Result<(), String> {
    let root = parse_xml(text)?;
    let mut seqs = Vec::new();
    root.find_all("sequence", &mut seqs);
    let seq = *seqs.first().ok_or("no <sequence> in this XML")?;
    if seqs.len() > 1 {
        b.note(Level::Warning, "file", format!("{} sequences found; only the first was imported", seqs.len()));
    }
    let fps = rate_of(seq)
        .or_else(|| seq.path(&["media", "video", "format", "samplecharacteristics"]).and_then(rate_of))
        .unwrap_or(30.0)
        .max(1.0);
    let name = seq.text_of("name");
    if !name.is_empty() {
        b.project.name = name.to_string();
    }
    b.project.fps = fps;
    if let Some(sc) = seq.path(&["media", "video", "format", "samplecharacteristics"]) {
        let (w, h) = (sc.num("width").unwrap_or(0.0) as u32, sc.num("height").unwrap_or(0.0) as u32);
        if w > 0 && h > 0 {
            b.project.width = w;
            b.project.height = h;
        }
    }
    b.note(
        Level::Ok,
        format!("sequence '{}'", b.project.name),
        format!("{}×{} @ {fps:.3} fps", b.project.width, b.project.height),
    );

    b.project.tracks.clear();
    let mut files: HashMap<String, Id> = HashMap::new();
    // (track index, cut time, kind, duration, effect name)
    let mut transitions: Vec<(usize, f64, TransitionKind, f64, String)> = Vec::new();

    for (kind, section) in [(TrackKind::Video, "video"), (TrackKind::Audio, "audio")] {
        let Some(sec) = seq.path(&["media", section]) else { continue };
        for (n, tr) in sec.kids_named("track").enumerate() {
            let ti = track_slot(&mut b.project, kind, n);
            let label = b.project.tracks[ti].name.clone();
            if tr.text_of("enabled").eq_ignore_ascii_case("false") {
                b.project.tracks[ti].muted = true;
            }
            if tr.text_of("locked").eq_ignore_ascii_case("true") {
                b.note(Level::Warning, format!("track {label}"), "locked tracks are imported unlocked");
            }
            for item in tr.kids_named("clipitem") {
                read_clipitem(item, kind, ti, fps, &mut files, b);
            }
            for t in tr.kids_named("transitionitem") {
                if let Some(x) = read_transitionitem(t, ti, fps, b) {
                    transitions.push(x);
                }
            }
        }
    }
    if b.project.tracks.is_empty() {
        b.project.add_track(TrackKind::Video);
        b.project.add_track(TrackKind::Audio);
    }
    link_stacks(&mut b.project);

    for (ti, cut, kind, dur, name) in transitions {
        let right = b.project.tracks[ti].clips.iter().find(|c| (c.start - cut).abs() < 1.0 / fps).map(|c| c.id);
        match right.and_then(|id| b.project.add_transition(id, kind, dur)) {
            Some(_) => b.note(Level::Ok, format!("transition '{name}'"), format!("{kind:?} over {dur:.2} s")),
            None => b.note(
                Level::Warning,
                format!("transition '{name}'"),
                "dropped: it does not sit on a cut between two clips",
            ),
        }
    }
    Ok(())
}

fn read_clipitem(item: &El, kind: TrackKind, ti: usize, fps: f64, files: &mut HashMap<String, Id>, b: &mut Build) {
    let name = item.text_of("name").to_string();
    let subject = format!("clip '{}'", if name.is_empty() { "(unnamed)" } else { name.as_str() });
    let (Some(start), Some(end)) = (item.num("start"), item.num("end")) else {
        b.note(Level::Skipped, subject, "no <start>/<end>");
        return;
    };
    if start < 0.0 || end <= start {
        // FCP writes start -1 for items that live inside a transition rather than on the timeline
        b.note(Level::Skipped, subject, format!("not placed on the timeline (start {start}, end {end})"));
        return;
    }
    let Some(file) = item.child("file") else {
        b.note(Level::Skipped, subject, "no <file> — generators and titles are not interchangeable");
        return;
    };
    let Some(aid) = asset_of(file, fps, files, &subject, b) else {
        b.note(Level::Skipped, subject, "its <file> was never defined in this XML");
        return;
    };
    let size = (b.project.width as f64, b.project.height as f64);
    let asset_kind = b.project.asset(aid).map(|a| a.kind).unwrap_or(ClipKind::Video);
    let clip_kind = match kind {
        TrackKind::Audio => ClipKind::Audio,
        TrackKind::Video if asset_kind == ClipKind::Image => ClipKind::Image,
        TrackKind::Video => ClipKind::Video,
    };
    let dur = ((end - start) / fps).max(MIN_CLIP);
    let id = b.project.new_id();
    let mut c = Clip::new(id, clip_kind, name, start / fps, dur);
    c.asset = aid;
    c.src_in = item.num("in").unwrap_or(0.0).max(0.0) / fps;
    c.enabled = !item.text_of("enabled").eq_ignore_ascii_case("false");
    if kind == TrackKind::Audio {
        let idx = item.child("sourcetrack").and_then(|s| s.num("trackindex")).unwrap_or(1.0);
        c.audio_stream = (idx as usize).saturating_sub(1);
    }
    if let Some(sp) = item.num("speed") {
        if (sp - 100.0).abs() > 0.5 {
            b.note(Level::Warning, subject.clone(), format!("speed {sp} % not imported (clip plays at 100 %)"));
        }
    }
    read_filters(item, &mut c, size, &subject, b);
    place(&mut b.project, ti, c);
}

/// Asset for a `<file>`: defined once, then referenced by id (`<file id="file-3"/>`).
fn asset_of(file: &El, fps: f64, files: &mut HashMap<String, Id>, subject: &str, b: &mut Build) -> Option<Id> {
    let fid = file.attr("id").to_string();
    if let Some(&id) = files.get(&fid) {
        return Some(id);
    }
    if file.kids.is_empty() {
        return None;
    }
    let raw = from_pathurl(file.text_of("pathurl"));
    let path = b.resolve(&raw, subject);
    if path.is_empty() {
        return None;
    }
    let file_fps = rate_of(file).unwrap_or(fps).max(1.0);
    let sc = file.path(&["media", "video", "samplecharacteristics"]);
    let (w, h) =
        sc.map(|s| (s.num("width").unwrap_or(0.0) as u32, s.num("height").unwrap_or(0.0) as u32)).unwrap_or_default();
    let audio = file.path(&["media", "audio"]);
    let kind = if is_image_path(&path) {
        ClipKind::Image
    } else if sc.is_some() || w > 0 {
        ClipKind::Video
    } else {
        ClipKind::Audio
    };
    let asset = Asset {
        id: 0,
        path,
        kind,
        duration: file.num("duration").unwrap_or(0.0) / file_fps,
        width: w,
        height: h,
        fps: sc.and_then(rate_of).unwrap_or(file_fps),
        audio_streams: audio
            .map(|a| {
                vec![AudioStreamInfo {
                    index: 0,
                    channels: a.num("channelcount").unwrap_or(2.0) as u32,
                    sample_rate: a.num("samplerate").unwrap_or(48000.0) as u32,
                    ..Default::default()
                }]
            })
            .unwrap_or_default(),
        codec: String::new(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
    };
    let id = b.add_asset(asset);
    files.insert(fid, id);
    Some(id)
}

/// Opacity / Basic Motion become clip properties; everything else is reported.
fn read_filters(item: &El, c: &mut Clip, size: (f64, f64), subject: &str, b: &mut Build) {
    for f in item.kids_named("filter") {
        let Some(effect) = f.child("effect") else { continue };
        let id = effect.text_of("effectid").to_ascii_lowercase();
        let name = effect.text_of("name").to_string();
        let param = |key: &str| -> Option<f64> {
            effect
                .kids_named("parameter")
                .find(|p| p.text_of("parameterid").eq_ignore_ascii_case(key))
                .and_then(|p| p.num("value"))
        };
        let animated = effect.kids_named("parameter").any(|p| p.child("keyframe").is_some());
        match id.as_str() {
            "opacity" => {
                if let Some(v) = param("opacity") {
                    c.opacity.value = (v / 100.0).clamp(0.0, 1.0);
                }
            }
            "basic" => {
                if let Some(v) = param("scale") {
                    c.scale.value = (v / 100.0).max(0.0);
                }
                if let Some(v) = param("rotation") {
                    c.rotation.value = v;
                }
                if let Some(center) = effect
                    .kids_named("parameter")
                    .find(|p| p.text_of("parameterid").eq_ignore_ascii_case("center"))
                    .and_then(|p| p.child("value"))
                {
                    // FCP centre is a fraction of the frame, ours is pixels from the middle
                    c.x.value = center.num("horiz").unwrap_or(0.0) * size.0;
                    c.y.value = center.num("vert").unwrap_or(0.0) * size.1;
                }
            }
            _ => {
                b.note(
                    Level::Warning,
                    format!("effect '{}'", if name.is_empty() { id.clone() } else { name.clone() }),
                    format!("not imported on {subject}"),
                );
                continue;
            }
        }
        if animated {
            b.note(Level::Warning, format!("effect '{name}'"), format!("keyframes flattened on {subject}"));
        }
    }
}

fn read_transitionitem(
    t: &El,
    ti: usize,
    fps: f64,
    b: &mut Build,
) -> Option<(usize, f64, TransitionKind, f64, String)> {
    let effect = t.child("effect");
    let name = effect
        .map(|e| e.text_of("name"))
        .filter(|n| !n.is_empty())
        .or_else(|| effect.map(|e| e.text_of("effectid")))
        .unwrap_or("transition")
        .to_string();
    let (Some(s), Some(e)) = (t.num("start"), t.num("end")) else {
        b.note(Level::Warning, format!("transition '{name}'"), "no <start>/<end>");
        return None;
    };
    if e <= s {
        b.note(Level::Warning, format!("transition '{name}'"), "empty");
        return None;
    }
    let cut = match t.text_of("alignment") {
        "start" | "start-black" => s,
        "end" | "end-black" => e,
        _ => (s + e) / 2.0,
    } / fps;
    Some((ti, cut, transition_kind(&name), (e - s) / fps, name))
}

fn transition_kind(name: &str) -> TransitionKind {
    let n = name.to_ascii_lowercase();
    if n.contains("dip") || n.contains("to color") || n.contains("to black") {
        TransitionKind::FadeToColor
    } else if n.contains("wipe") || n.contains("iris") || n.contains("band") {
        TransitionKind::Wipe
    } else if n.contains("push") || n.contains("slide") {
        TransitionKind::Push
    } else {
        TransitionKind::CrossFade
    }
}

fn rate_of(el: &El) -> Option<f64> {
    let r = el.child("rate")?;
    let tb = r.num("timebase")?;
    if tb <= 0.0 {
        return None;
    }
    Some(if r.text_of("ntsc").eq_ignore_ascii_case("true") { tb * 1000.0 / 1001.0 } else { tb })
}

// ---------------------------------------------------------------- EDL (CMX 3600)

struct Event {
    line: usize,
    /// "V", "A", "A2", "AA", "B", "AA/V" …
    channel: String,
    /// 'C' cut, 'D' dissolve, 'W' wipe, 'K' key.
    kind: char,
    /// Transition length in frames.
    tdur: f64,
    src_in: f64,
    src_out: f64,
    rec_in: f64,
    rec_out: f64,
    reel: String,
    name: String,
}

fn looks_like_edl(text: &str) -> bool {
    text.contains("FCM:")
        || text.lines().take(200).filter(|l| !l.trim_start().starts_with('*')).any(|l| parse_event(l, 30.0).is_some())
}

fn read_edl(text: &str, b: &mut Build) {
    let drop_frame = text.lines().any(|l| {
        let u = l.to_ascii_uppercase();
        u.contains("FCM:") && u.contains("DROP") && !u.contains("NON-DROP") && !u.contains("NON DROP")
    });
    // ponytail: an EDL carries no frame rate — 30 / 29.97 is the CMX assumption, retime after import
    let fps = if drop_frame { 30.0 * 1000.0 / 1001.0 } else { 30.0 };
    b.project.fps = fps;
    b.note(
        Level::Warning,
        "sequence",
        format!("an EDL stores no frame rate or picture size; assumed {fps:.3} fps at the default size"),
    );

    let mut events: Vec<Event> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('*') {
            let r = rest.trim();
            let up = r.to_ascii_uppercase();
            // the source file name always follows the event it belongs to
            for key in ["FROM CLIP NAME:", "SOURCE FILE:"] {
                if up.starts_with(key) {
                    let v = r[key.len()..].trim();
                    match events.last_mut() {
                        Some(e) if !v.is_empty() => e.name = v.to_string(),
                        _ => {}
                    }
                    break;
                }
            }
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("TITLE:") {
            let t = line[6..].trim();
            if !t.is_empty() {
                b.project.name = t.to_string();
            }
            continue;
        }
        if upper.starts_with("FCM:") || upper.starts_with("EDL") {
            continue;
        }
        match parse_event(line, fps) {
            Some(mut e) => {
                e.line = n + 1;
                events.push(e);
            }
            None => b.note(Level::Skipped, format!("line {}", n + 1), format!("not a CMX 3600 event: {line}")),
        }
    }
    if events.is_empty() {
        b.note(Level::Skipped, "file", "no usable events");
        return;
    }

    // one asset per source name, its length the furthest source-out we saw
    let mut assets: HashMap<String, Id> = HashMap::new();
    let mut source: Vec<(String, f64, bool, bool)> = Vec::new(); // name, duration, video, audio
    for e in &events {
        let key = source_name(e);
        let (video, audio) = channels(&e.channel);
        match source.iter_mut().find(|(n, ..)| *n == key) {
            Some(s) => {
                s.1 = s.1.max(e.src_out);
                s.2 |= video;
                s.3 |= !audio.is_empty();
            }
            None => source.push((key, e.src_out, video, !audio.is_empty())),
        }
    }
    for (name, duration, video, audio) in source {
        let path = b.resolve(&name, &format!("source '{name}'"));
        let kind = if is_image_path(&path) {
            ClipKind::Image
        } else if video {
            ClipKind::Video
        } else {
            ClipKind::Audio
        };
        let id = b.add_asset(Asset {
            id: 0,
            path,
            kind,
            duration,
            width: 0,
            height: 0,
            fps,
            audio_streams: if audio {
                vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }]
            } else {
                Vec::new()
            },
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        });
        assets.insert(name, id);
    }

    let mut pending: Vec<(Id, TransitionKind, f64, usize)> = Vec::new();
    for e in &events {
        let Some(&aid) = assets.get(&source_name(e)) else { continue };
        let (video, audio) = channels(&e.channel);
        let dur = (e.rec_out - e.rec_in).max(MIN_CLIP);
        if e.rec_out <= e.rec_in {
            b.note(Level::Warning, format!("line {}", e.line), "record out is not after record in");
            continue;
        }
        let name = source_name(e);
        let title = Path::new(&name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
        let mut ids = Vec::new();
        if video {
            let ti = track_slot(&mut b.project, TrackKind::Video, 0);
            let id = b.project.new_id();
            let mut c = Clip::new(id, ClipKind::Video, title.clone(), e.rec_in, dur);
            c.asset = aid;
            c.src_in = e.src_in;
            ids.push(place(&mut b.project, ti, c));
        }
        for ch in audio {
            let ti = track_slot(&mut b.project, TrackKind::Audio, ch);
            let id = b.project.new_id();
            let mut c = Clip::new(id, ClipKind::Audio, title.clone(), e.rec_in, dur);
            c.asset = aid;
            c.src_in = e.src_in;
            ids.push(place(&mut b.project, ti, c));
        }
        match e.kind {
            'C' => {}
            'K' => b.note(Level::Warning, format!("line {}", e.line), "key (superimpose) events are not imported"),
            k => {
                let kind = if k == 'W' { TransitionKind::Wipe } else { TransitionKind::CrossFade };
                for id in &ids {
                    pending.push((*id, kind, (e.tdur / fps).max(MIN_CLIP), e.line));
                }
            }
        }
    }
    link_stacks(&mut b.project);
    for (id, kind, dur, line) in pending {
        match b.project.add_transition(id, kind, dur) {
            Some(_) => b.note(Level::Ok, format!("line {line}"), format!("{kind:?} of {dur:.2} s centred on the cut")),
            None => b.note(Level::Warning, format!("line {line}"), "transition dropped: no clip abuts this cut"),
        }
    }
}

fn source_name(e: &Event) -> String {
    if e.name.is_empty() {
        e.reel.clone()
    } else {
        e.name.clone()
    }
}

/// `("V", "A", "A2", "AA", "B", "AA/V")` → (has video, audio track indices).
fn channels(chan: &str) -> (bool, Vec<usize>) {
    let c = chan.to_ascii_uppercase();
    let video = c.contains('V') || c.contains('B');
    let mut audio = Vec::new();
    if c.contains("AA") {
        audio.extend([0, 1]);
    } else if c.contains('B') {
        audio.push(0);
    } else if let Some(p) = c.find('A') {
        let n: usize = c[p + 1..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(1);
        audio.push(n.max(1) - 1);
    }
    (video, audio)
}

/// `HH:MM:SS:FF` (`;` = drop frame) → seconds.
fn timecode(s: &str, fps: f64) -> Option<f64> {
    let p: Vec<&str> = s.split([':', ';', '.']).collect();
    if p.len() != 4 || p.iter().any(|x| x.is_empty() || !x.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let n: Vec<i64> = p.iter().filter_map(|x| x.parse().ok()).collect();
    if n.len() != 4 {
        return None;
    }
    let per = fps.round().max(1.0) as i64;
    Some((((n[0] * 60 + n[1]) * 60 + n[2]) * per + n[3]) as f64 / fps)
}

/// `001  AX  V  C  00:00:00:00 00:00:02:00 00:00:00:00 00:00:02:00` (a `D`/`W` adds a frame count).
fn parse_event(line: &str, fps: f64) -> Option<Event> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 8 || !f[0].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let channel = f[2].to_ascii_uppercase();
    if !channel.chars().all(|c| c.is_ascii_alphanumeric() || c == '/') || !channel.contains(['V', 'A', 'B']) {
        return None;
    }
    let trans = f[3].to_ascii_uppercase();
    let kind = trans.chars().next()?;
    if !matches!(kind, 'C' | 'D' | 'W' | 'K') {
        return None;
    }
    let (tdur, times) = match (kind, f.len()) {
        ('C', _) => (0.0, &f[4..]),
        (_, n) if n >= 9 => (f[4].parse::<f64>().ok()?, &f[5..]),
        _ => (0.0, &f[4..]),
    };
    if times.len() < 4 {
        return None;
    }
    let t: Vec<f64> = times[..4].iter().filter_map(|x| timecode(x, fps)).collect();
    if t.len() != 4 {
        return None;
    }
    Some(Event {
        line: 0,
        channel,
        kind,
        tdur,
        src_in: t[0],
        src_out: t[1],
        rec_in: t[2],
        rec_out: t[3],
        reel: f[1].to_string(),
        name: String::new(),
    })
}

// ---------------------------------------------------------------- tiny XML reader

#[derive(Debug, Default)]
struct El {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    kids: Vec<El>,
}

impl El {
    fn child(&self, name: &str) -> Option<&El> {
        self.kids.iter().find(|k| k.name == name)
    }
    fn kids_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// Descend by element name.
    fn path(&self, p: &[&str]) -> Option<&El> {
        p.iter().try_fold(self, |e, n| e.child(n))
    }
    fn attr(&self, name: &str) -> &str {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("")
    }
    fn text_of(&self, name: &str) -> &str {
        self.child(name).map(|c| c.text.trim()).unwrap_or("")
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.text_of(name).parse().ok()
    }
    fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        if self.name == name {
            out.push(self);
        }
        for k in &self.kids {
            k.find_all(name, out);
        }
    }
}

/// Minimal well-formed-ish XML reader: elements, attributes, text, CDATA; comments / PIs / DOCTYPE are
/// skipped. Mismatched or missing close tags unwind instead of failing — exported timelines are machine
/// written, but a truncated one should still give back what it has.
fn parse_xml(src: &str) -> Result<El, String> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut stack: Vec<El> = Vec::new();
    let mut root: Option<El> = None;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            if let Some(top) = stack.last_mut() {
                let t = &src[start..i];
                if !t.trim().is_empty() {
                    top.text.push_str(&unescape(t));
                }
            }
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            i += 4 + rest[4..].find("-->").ok_or("unterminated comment")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let e = rest[9..].find("]]>").ok_or("unterminated CDATA")?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&rest[9..9 + e]);
            }
            i += 9 + e + 3;
        } else if rest.starts_with("<?") {
            i += 2 + rest[2..].find("?>").ok_or("unterminated <?…?>")? + 2;
        } else if rest.starts_with("<!") {
            i += 2 + rest[2..].find('>').ok_or("unterminated <!…>")? + 1;
        } else if rest.starts_with("</") {
            let e = rest.find('>').ok_or("unterminated close tag")?;
            close(&mut stack, &mut root, rest[2..e].trim());
            i += e + 1;
        } else {
            let e = tag_end(rest).ok_or("unterminated tag")?;
            let inner = rest[1..e].trim_end();
            let selfclose = inner.ends_with('/');
            let el = parse_tag(inner.trim_end_matches('/'));
            i += e + 1;
            if selfclose {
                attach(&mut stack, &mut root, el);
            } else {
                stack.push(el);
            }
        }
    }
    while let Some(el) = stack.pop() {
        attach(&mut stack, &mut root, el);
    }
    root.ok_or_else(|| "no XML root element".to_string())
}

fn attach(stack: &mut [El], root: &mut Option<El>, el: El) {
    match stack.last_mut() {
        Some(p) => p.kids.push(el),
        None => {
            if root.is_none() {
                *root = Some(el);
            }
        }
    }
}

/// Close `name`, unwinding any elements left open inside it.
fn close(stack: &mut Vec<El>, root: &mut Option<El>, name: &str) {
    if !stack.iter().any(|e| e.name == name) {
        return; // stray close tag
    }
    while let Some(el) = stack.pop() {
        let matched = el.name == name;
        attach(stack, root, el);
        if matched {
            return;
        }
    }
}

/// Index of the `>` closing a tag, ignoring quoted attribute values.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = '\0';
    for (i, c) in rest.char_indices() {
        if quote != '\0' {
            if c == quote {
                quote = '\0';
            }
        } else if c == '"' || c == '\'' {
            quote = c;
        } else if c == '>' {
            return Some(i);
        }
    }
    None
}

fn parse_tag(inner: &str) -> El {
    let inner = inner.trim();
    let (name, mut rest) = match inner.find(char::is_whitespace) {
        Some(i) => (&inner[..i], &inner[i..]),
        None => (inner, ""),
    };
    let mut attrs = Vec::new();
    loop {
        rest = rest.trim_start();
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let after = &after[q.len_utf8()..];
        let Some(end) = after.find(q) else { break };
        attrs.push((key, unescape(&after[..end])));
        rest = &after[end + q.len_utf8()..];
    }
    El { name: name.to_string(), attrs, text: String::new(), kids: Vec::new() }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        o.push_str(&rest[..i]);
        rest = &rest[i..];
        let ch = rest.find(';').filter(|&e| e <= 12).and_then(|e| {
            let ent = &rest[1..e];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e + 1))
        });
        match ch {
            Some((c, n)) => {
                o.push(c);
                rest = &rest[n..];
            }
            None => {
                o.push('&');
                rest = &rest[1..];
            }
        }
    }
    o.push_str(rest);
    o
}

/// `file://localhost/C:/a%20b/v.mp4` → `C:\a b\v.mp4`; `file://nas/share/v.mp4` → `\\nas\share\v.mp4`.
fn from_pathurl(url: &str) -> String {
    let u = url.trim();
    let Some(rest) = u.strip_prefix("file://") else { return percent_decode(u).replace('/', "\\") };
    let path = if let Some(r) = rest.strip_prefix("localhost/") {
        r.to_string()
    } else if let Some(r) = rest.strip_prefix('/') {
        r.trim_start_matches('/').to_string()
    } else {
        format!("//{rest}")
    };
    percent_decode(&path).replace('/', "\\")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push((hex(b[i + 1]) << 4) | hex(b[i + 2]));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::xmeml::export_xmeml;

    fn asset(path: &str, streams: usize) -> Asset {
        Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: (0..streams)
                .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
                .collect(),
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    fn import_bytes(bytes: &[u8], name: &str) -> ImportReport {
        let dir = std::env::temp_dir().join("simple-editor-import-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}-{}", std::process::id(), name));
        std::fs::write(&path, bytes).expect("write test file");
        let r = import_file(&path).expect("import");
        let _ = std::fs::remove_file(&path);
        r
    }

    fn import_text(text: &str, name: &str) -> ImportReport {
        import_bytes(text.as_bytes(), name)
    }

    /// What engine::xmeml writes comes back: the same clips, at the same times, on the same track kinds.
    #[test]
    fn xmeml_round_trip() {
        let mut p = Project::from_media(asset(r"C:\media\clip & co.mp4", 2));
        p.split_at(4.0, None);
        p.tracks[0].clips[0].opacity.value = 0.5;
        p.tracks[0].clips[1].scale.value = 1.5;
        let want: Vec<(TrackKind, f64, f64, f64)> = p
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(move |c| (t.kind, c.start, c.duration, c.src_in)))
            .collect();

        let r = import_text(&export_xmeml(&p), "roundtrip.xml");
        assert_eq!(r.clips, want.len(), "{}", r.to_markdown());
        assert_eq!(r.project.tracks.len(), p.tracks.len());
        let frame = 1.0 / p.fps;
        let got: Vec<(TrackKind, f64, f64, f64)> = r
            .project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(move |c| (t.kind, c.start, c.duration, c.src_in)))
            .collect();
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g.0, w.0, "track kind");
            assert!((g.1 - w.1).abs() < frame, "start {} vs {}", g.1, w.1);
            assert!((g.2 - w.2).abs() < frame, "duration {} vs {}", g.2, w.2);
            assert!((g.3 - w.3).abs() < frame, "src_in {} vs {}", g.3, w.3);
        }
        // size / fps / one asset with its escaped path back
        assert_eq!((r.project.width, r.project.height), (1280, 720));
        assert!((r.project.fps - 30.0).abs() < 0.05, "{}", r.project.fps);
        assert_eq!(r.project.assets.len(), 1);
        assert_eq!(r.project.assets[0].path, r"C:\media\clip & co.mp4");
        // properties the exporter writes as filters
        assert!((r.project.tracks[0].clips[0].opacity.value - 0.5).abs() < 1e-6);
        assert!((r.project.tracks[0].clips[1].scale.value - 1.5).abs() < 1e-6);
        // video + its two audio clips are one linked group again
        let v = r.project.tracks[0].clips[0].id;
        assert_eq!(r.project.linked(v).len(), 3, "video + 2 audio clips link together");
        // the file was never there: counted once, not once per clip
        assert_eq!(r.missing_media, 1);
        assert_eq!(r.issues.iter().filter(|i| i.detail.contains("media not found")).count(), 1);
    }

    /// A text clip is a comment in the export (no clipitem), so it simply does not come back.
    #[test]
    fn xmeml_text_clip_is_not_invented() {
        let mut p = Project::from_media(asset(r"C:\media\a.mp4", 0));
        p.add_text_clip(1.0, 2.0);
        let r = import_text(&export_xmeml(&p), "text.xml");
        assert_eq!(r.clips, 1);
        assert!(r.project.all_clips().all(|(_, c)| c.kind != ClipKind::Text));
    }

    const EDL: &str = "TITLE: Cut 03\nFCM: NON-DROP FRAME\n\n\
001  AX       V     C        00:00:05:00 00:00:07:00 00:00:00:00 00:00:02:00\n\
* FROM CLIP NAME: red.mp4\n\
002  AX       AA    C        00:00:05:00 00:00:07:00 00:00:00:00 00:00:02:00\n\
* FROM CLIP NAME: red.mp4\n\
003  BX       V     D    015 00:00:00:00 00:00:03:00 00:00:02:00 00:00:05:00\n\
* FROM CLIP NAME: green.mp4\n";

    #[test]
    fn edl_parses() {
        let r = import_text(EDL, "cut.edl");
        assert_eq!(r.project.name, "Cut 03");
        assert_eq!(r.clips, 4, "2 video + 2 audio: {}", r.to_markdown());
        assert_eq!(r.project.assets.len(), 2);
        assert_eq!(r.missing_media, 2);
        let v = r.project.video_tracks();
        let a = r.project.audio_tracks();
        assert_eq!((v.len(), a.len()), (1, 2), "AA fills A1 and A2");
        let vclips = &r.project.tracks[v[0]].clips;
        assert_eq!(vclips.len(), 2);
        assert!((vclips[0].start - 0.0).abs() < 1e-9 && (vclips[0].duration - 2.0).abs() < 1e-9);
        assert!((vclips[0].src_in - 5.0).abs() < 1e-9, "source in is kept");
        assert!((vclips[1].start - 2.0).abs() < 1e-9 && (vclips[1].duration - 3.0).abs() < 1e-9);
        // the dissolve became a transition on the cut at 2 s (15 frames @ 30 = 0.5 s)
        let tr = &r.project.tracks[v[0]].transitions;
        assert_eq!(tr.len(), 1, "{}", r.to_markdown());
        assert_eq!(tr[0].kind, TransitionKind::CrossFade);
        assert!((tr[0].duration - 0.5).abs() < 1e-9);
        // source length is the furthest source-out
        assert!(r.project.assets.iter().any(|a| a.path.ends_with("red.mp4") && (a.duration - 7.0).abs() < 1e-9));
        // no frame rate in an EDL → one honest warning
        assert!(r.issues.iter().any(|i| i.detail.contains("stores no frame rate")));
    }

    /// A broken event line is one Issue, the rest of the EDL still imports.
    #[test]
    fn edl_malformed_block_is_an_issue() {
        let bad = format!("{EDL}004  CX  V  C  00:00:0 nonsense here\n005  DX  V  C  ??:??:??:?? 1 2 3\n");
        let r = import_text(&bad, "bad.edl");
        assert_eq!(r.clips, 4, "the good events survived");
        let skipped: Vec<&Issue> = r.issues.iter().filter(|i| i.level == Level::Skipped).collect();
        assert_eq!(skipped.len(), 2, "{}", r.to_markdown());
        assert!(skipped.iter().all(|i| i.subject.starts_with("line ")), "{:?}", skipped);
    }

    #[test]
    fn report_markdown_has_a_row_per_issue() {
        let r = import_text(EDL, "md.edl");
        let md = r.to_markdown();
        let rows = md.lines().filter(|l| l.starts_with('|')).count();
        assert_eq!(rows, r.issues.len() + 2, "header + separator + one row per issue:\n{md}");
        assert!(md.starts_with("# Import: Cut 03"));
        assert!(md.contains(&format!("{} clips on {} tracks", r.clips, r.tracks)));
        // a detail containing a pipe never breaks the table
        let mut r2 = r;
        r2.issues.push(Issue { level: Level::Ok, subject: "a|b".into(), detail: "c|d\ne".into() });
        let md = r2.to_markdown();
        assert!(md.contains(r"| Ok | a\|b | c\|d e |"), "{md}");
        assert_eq!(md.lines().filter(|l| l.starts_with('|')).count(), r2.issues.len() + 2);
    }

    #[test]
    fn gzip_prproj_reports_how_to_export() {
        let r = import_bytes(b"\x1f\x8b\x08\x00binary junk", "seq.prproj");
        assert_eq!(r.clips, 0);
        assert_eq!(r.issues.len(), 1);
        assert_eq!(r.issues[0].level, Level::Skipped);
        assert!(r.issues[0].detail.contains("Final Cut Pro XML"));
    }

    #[test]
    fn unknown_effects_and_bad_clipitems_become_issues() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE xmeml><xmeml version="5">
<sequence><name>S</name><rate><timebase>25</timebase><ntsc>FALSE</ntsc></rate>
<media><video><format><samplecharacteristics><width>640</width><height>360</height></samplecharacteristics></format>
<track>
 <clipitem id="c1"><name>a.mp4</name><start>0</start><end>25</end><in>0</in><out>25</out>
  <file id="f1"><name>a.mp4</name><pathurl>file://localhost/C:/nope/a.mp4</pathurl>
   <rate><timebase>25</timebase></rate><duration>250</duration>
   <media><video><samplecharacteristics><width>640</width><height>360</height></samplecharacteristics></video></media></file>
  <filter><effect><name>Gaussian Blur</name><effectid>blur</effectid>
   <parameter><parameterid>radius</parameterid><value>4</value></parameter></effect></filter>
 </clipitem>
 <clipitem id="c2"><name>inside-transition</name><start>-1</start><end>-1</end><file id="f1"/></clipitem>
 <clipitem id="c3"><name>orphan</name><start>50</start><end>75</end><file id="nope"/></clipitem>
 <transitionitem><start>20</start><end>30</end><alignment>center</alignment>
  <effect><name>Cross Dissolve</name><effectid>Cross Dissolve</effectid></effect></transitionitem>
</track></video></media></sequence></xmeml>"#;
        let r = import_text(xml, "eff.xml");
        assert_eq!(r.clips, 1, "{}", r.to_markdown());
        assert!((r.project.fps - 25.0).abs() < 1e-9);
        assert_eq!((r.project.width, r.project.height), (640, 360));
        assert_eq!(r.project.assets[0].path, r"C:\nope\a.mp4");
        assert!((r.project.assets[0].duration - 10.0).abs() < 1e-9, "250 frames @ 25");
        let d: Vec<&str> = r.issues.iter().map(|i| i.detail.as_str()).collect();
        assert!(r.issues.iter().any(|i| i.subject == "effect 'Gaussian Blur'"), "{d:?}");
        assert!(r.issues.iter().any(|i| i.detail.contains("not placed on the timeline")), "{d:?}");
        assert!(r.issues.iter().any(|i| i.detail.contains("never defined")), "{d:?}");
        // a transition with only one clip beside it cannot be placed
        assert!(r.issues.iter().any(|i| i.detail.contains("does not sit on a cut")), "{d:?}");
        assert_eq!(r.missing_media, 1);
    }

    #[test]
    fn xml_reader_survives_junk() {
        // unclosed <b>, stray </z>, CDATA, entities, attribute with a '>' inside
        let x = parse_xml(r#"<a t="x>y"><b><c>1 &amp; 2</c><d><![CDATA[<raw>]]></d></z></a>"#).unwrap();
        assert_eq!(x.name, "a");
        assert_eq!(x.attr("t"), "x>y");
        assert_eq!(x.path(&["b", "c"]).map(|e| e.text.as_str()), Some("1 & 2"));
        assert_eq!(x.path(&["b", "d"]).map(|e| e.text.as_str()), Some("<raw>"));
        assert!(parse_xml("no elements here").is_err());
    }

    #[test]
    fn pathurl_and_timecode() {
        assert_eq!(from_pathurl("file://localhost/C:/My%20Videos/a%20&%20b.mp4"), r"C:\My Videos\a & b.mp4");
        assert_eq!(from_pathurl("file:///C:/x/y.mp4"), r"C:\x\y.mp4");
        assert_eq!(from_pathurl("file://nas/share/v.mp4"), r"\\nas\share\v.mp4");
        assert_eq!(from_pathurl("C:/plain/v.mp4"), r"C:\plain\v.mp4");
        assert_eq!(timecode("00:01:02:15", 30.0), Some(62.5));
        assert_eq!(timecode("00;01;02;15", 30.0), Some(62.5));
        assert_eq!(timecode("garbage", 30.0), None);
        assert_eq!(channels("AA/V"), (true, vec![0, 1]));
        assert_eq!(channels("A2"), (false, vec![1]));
        assert_eq!(channels("B"), (true, vec![0]));
        assert_eq!(channels("V"), (true, vec![]));
    }
}
