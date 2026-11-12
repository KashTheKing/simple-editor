//! ---- ws:assistant-pane ----
//! The Assistant's data, no egui and no `App`: conversations (chat lines + the task queue) and their
//! per-project store under `%APPDATA%\SimpleEditor\assistant`, pinned context chips, image
//! attachments, the task state machine, the "Add to Assistant" pin queue the right-click menus feed,
//! and the agent / model options the header offers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Conversations kept per project; the oldest (by last use) are dropped past this.
pub const MAX_CONVERSATIONS: usize = 50;
/// Attachment thumbnails are stored as raw RGBA of exactly this size (letterboxed).
pub const THUMB: [u32; 2] = [96, 54];
/// Longest edge of the PNG an agent gets for an attachment.
pub const ATTACH_MAX: u32 = 1024;

/// The agents the header / Settings offer: (stored `Settings.assistant_backend` value, label). The
/// labels are the one place a vendor name appears (an option the user picks).
pub const AGENTS: &[(&str, &str)] =
    &[("claude", "Claude Code"), ("codex", "Codex"), ("custom", "Custom command"), ("external", "External agent")];
/// Models the "claude" agent takes for `--model`: (id passed to the CLI, label). "" = the CLI's own
/// default. The first four are aliases that follow the newest release of a family; the rest are exact.
/// ponytail: a fixed list - a model released after this build is reachable through "Other…"; make the
/// list a setting if retyping an id there gets old.
pub const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("fable", "Fable (latest)"),
    ("opus", "Opus (latest)"),
    ("sonnet", "Sonnet (latest)"),
    ("haiku", "Haiku (latest)"),
    ("claude-fable-5-2", "Fable 5.2"),
    ("claude-fable-5-1", "Fable 5.1"),
    ("claude-fable-5", "Fable 5"),
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-opus-5", "Opus 5"),
    ("claude-opus-4-8", "Opus 4.8"),
    ("claude-opus-4-7", "Opus 4.7"),
    ("claude-opus-4-6", "Opus 4.6"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-sonnet-5", "Sonnet 5"),
    ("claude-sonnet-4-6", "Sonnet 4.6"),
    ("claude-haiku-5-5", "Haiku 5.5"),
    ("claude-haiku-4-5", "Haiku 4.5"),
];
/// How many of `CLAUDE_MODELS` are the "latest" aliases (a separator follows them in the menu).
pub const CLAUDE_ALIASES: usize = 4;
/// Effort levels the "claude" agent takes for `--effort`: (value, label). "" = the model's default.
pub const EFFORTS: &[(&str, &str)] =
    &[("low", "Low"), ("medium", "Medium"), ("high", "High"), ("xhigh", "Extra high"), ("max", "Max")];

/// The label of a listed model, else the id itself.
pub fn model_label(model: &str) -> &str {
    CLAUDE_MODELS.iter().find(|(id, _)| *id == model).map_or(model, |(_, l)| l)
}

/// "3 min ago" for a `now_secs`-style timestamp.
pub fn ago(then: f64, now: f64) -> String {
    let s = (now - then).max(0.0);
    match s {
        s if s < 60.0 => "just now".into(),
        s if s < 3600.0 => format!("{} min ago", (s / 60.0) as u64),
        s if s < 86_400.0 => format!("{} h ago", (s / 3600.0) as u64),
        s => format!("{} d ago", (s / 86_400.0) as u64),
    }
}

pub fn agent_label(backend: &str) -> &'static str {
    AGENTS.iter().find(|(v, _)| *v == backend).map_or("Agent", |(_, l)| l)
}

/// One row of a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Line {
    User(String),
    /// What went with the user message above it: the chips' labels and the images.
    Attached { chips: Vec<String>, images: Vec<Attachment> },
    Text(String),
    /// A tool call: "name {args}".
    Tool(String),
    Error(String),
    Info(String),
    /// A request finished. `undo` = (undo-stack length right after its entry was pushed, that entry's
    /// label) when it changed anything.
    Done { undo: Option<(usize, String)> },
}

/// An image sent with a request: a PNG for the agent plus a thumbnail, both files in the store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub png: PathBuf,
    /// Raw RGBA, `THUMB` sized.
    pub thumb: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// A pinned piece of context: a snapshot taken when the user added it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    pub label: String,
    pub data: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Writing,
    Ready,
    Running,
    Done,
    Failed,
}

impl TaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Writing => "Writing…",
            TaskStatus::Ready => "Ready",
            TaskStatus::Running => "Running",
            TaskStatus::Done => "Done",
            TaskStatus::Failed => "Failed",
        }
    }
}

/// A queued script the agent wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub code: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub error: String,
}

impl Task {
    pub fn runnable(&self) -> bool {
        !self.code.trim().is_empty() && !matches!(self.status, TaskStatus::Writing | TaskStatus::Running)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conversation {
    pub id: u64,
    pub title: String,
    pub created: f64,
    pub updated: f64,
    /// The agent's session id, resumed by the next message.
    pub session: Option<String>,
    pub lines: Vec<Line>,
    pub tasks: Vec<Task>,
}

impl Conversation {
    pub fn new(id: u64) -> Self {
        let now = now_secs();
        Self { id, created: now, updated: now, ..Default::default() }
    }
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.tasks.is_empty()
    }
    pub fn title(&self) -> &str {
        if self.title.is_empty() {
            "New chat"
        } else {
            &self.title
        }
    }
    pub fn attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.lines.iter().flat_map(|l| match l {
            Line::Attached { images, .. } => images.as_slice(),
            _ => &[],
        })
    }
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `s` on one line, cut to `n` characters with an ellipsis.
pub fn short(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

// ---------------------------------------------------------------------------------------------------
// tasks

fn next_task_id(tasks: &[Task]) -> u64 {
    tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1
}

/// A placeholder the running request fills (`hand_over`): shown as "Writing…".
pub fn task_writing(tasks: &mut Vec<Task>, title: &str) -> u64 {
    let id = next_task_id(tasks);
    tasks.push(Task {
        id,
        title: short(title, 60),
        note: String::new(),
        code: String::new(),
        status: TaskStatus::Writing,
        error: String::new(),
    });
    id
}

/// `tasks.add`: fill `slot` while it is still a placeholder, else queue a new Ready task.
pub fn task_hand_over(tasks: &mut Vec<Task>, slot: Option<u64>, title: &str, code: &str, note: &str) -> u64 {
    if let Some(t) = slot.and_then(|id| tasks.iter_mut().find(|t| t.id == id && t.status == TaskStatus::Writing)) {
        (t.title, t.code, t.note, t.status) = (short(title, 60), code.to_string(), short(note, 120), TaskStatus::Ready);
        return t.id;
    }
    let id = next_task_id(tasks);
    tasks.push(Task {
        id,
        title: short(title, 60),
        note: short(note, 120),
        code: code.to_string(),
        status: TaskStatus::Ready,
        error: String::new(),
    });
    id
}

/// New code / title (`tasks.set`, the card's Edit ▸ Save). New code makes it Ready again.
pub fn task_set(tasks: &mut [Task], id: u64, title: Option<&str>, code: Option<&str>) -> Result<(), String> {
    let t = tasks.iter_mut().find(|t| t.id == id).ok_or(format!("no task {id}"))?;
    if t.status == TaskStatus::Running {
        return Err("that task is running".into());
    }
    if let Some(title) = title.filter(|s| !s.trim().is_empty()) {
        t.title = short(title, 60);
    }
    if let Some(code) = code {
        t.code = code.to_string();
        t.status = TaskStatus::Ready;
        t.error.clear();
    }
    Ok(())
}

pub fn task_remove(tasks: &mut Vec<Task>, id: u64) -> bool {
    let n = tasks.len();
    tasks.retain(|t| t.id != id);
    tasks.len() != n
}

/// One step up (earlier in the queue) or down.
pub fn task_move(tasks: &mut [Task], id: u64, up: bool) {
    let Some(i) = tasks.iter().position(|t| t.id == id) else { return };
    let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < tasks.len()) };
    if let Some(j) = j {
        tasks.swap(i, j);
    }
}

/// Run `only` (any runnable task), or every Ready task in queue order (`None`), through `run`; a failure
/// stops the queue. Statuses and errors are set here. Returns how many ran and the first error.
pub fn run_queue(
    tasks: &mut [Task],
    only: Option<u64>,
    run: &mut dyn FnMut(&Task) -> Result<(), String>,
) -> (usize, Option<String>) {
    let ids: Vec<u64> = match only {
        Some(id) => tasks.iter().filter(|t| t.id == id && t.runnable()).map(|t| t.id).collect(),
        None => tasks.iter().filter(|t| t.status == TaskStatus::Ready && t.runnable()).map(|t| t.id).collect(),
    };
    let mut ran = 0;
    for id in ids {
        let Some(t) = tasks.iter_mut().find(|t| t.id == id) else { continue };
        t.status = TaskStatus::Running;
        let r = run(t);
        ran += 1;
        match r {
            Ok(()) => (t.status, t.error) = (TaskStatus::Done, String::new()),
            Err(e) => {
                (t.status, t.error) = (TaskStatus::Failed, e.clone());
                return (ran, Some(e));
            }
        }
    }
    (ran, None)
}

/// The Luau file "Save to Scripts" writes: a header the Scripts menu reads, then the code.
pub fn task_script(t: &Task) -> String {
    let mut s = format!("-- @name {}\n", short(&t.title, 60));
    if !t.note.is_empty() {
        s.push_str(&format!("-- @desc {}\n", short(&t.note, 120)));
    }
    s.push('\n');
    s.push_str(&t.code);
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// A file name in `dir` for `title`: safe characters only, `-2`, `-3`… when taken.
pub fn script_path(dir: &Path, title: &str) -> PathBuf {
    let stem: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let stem = if stem.is_empty() { "task".to_string() } else { stem.chars().take(40).collect() };
    let mut p = dir.join(format!("{stem}.luau"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}-{n}.luau"));
        n += 1;
    }
    p
}

// ---------------------------------------------------------------------------------------------------
// pinned chips

/// What "Add to Assistant" snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    /// The selected clips, else transcript words, else a marker, else In/Out.
    Smart,
    Clips,
    Range,
    Marker,
    Words,
}

impl Pin {
    pub fn parse(s: &str) -> Option<Pin> {
        Some(match s {
            "" | "selection" => Pin::Smart,
            "clips" => Pin::Clips,
            "range" => Pin::Range,
            "marker" => Pin::Marker,
            "words" => Pin::Words,
            _ => return None,
        })
    }
}

/// A chip for `pin` from an `editor.context` JSON; None when there is nothing of that kind.
pub fn pin_chip(c: &Value, pin: Pin) -> Option<Chip> {
    match pin {
        Pin::Smart => [Pin::Clips, Pin::Words, Pin::Marker, Pin::Range].into_iter().find_map(|p| pin_chip(c, p)),
        Pin::Clips => {
            let clips = c["selected_clips"].as_array().filter(|a| !a.is_empty())?;
            let label = match clips.as_slice() {
                [one] => format!("Clip: {}", short(one["name"].as_str().unwrap_or("clip"), 28)),
                many => format!("{} clips", many.len()),
            };
            Some(Chip { label, data: json!({"kind": "clips", "clips": clips}) })
        }
        Pin::Range => {
            let (a, b) = (c["in"].as_f64()?, c["out"].as_f64()?);
            Some(Chip { label: format!("In/Out {a:.1}s–{b:.1}s"), data: json!({"kind": "range", "in": a, "out": b}) })
        }
        Pin::Marker => {
            let m = c["selected_markers"].as_array()?.first()?;
            let what = [m["note"].as_str(), m["name"].as_str()].into_iter().flatten().find(|s| !s.trim().is_empty());
            let label = format!("Marker: {}", short(what.unwrap_or("marker"), 28));
            let mut data = m.clone();
            data["kind"] = json!("marker");
            Some(Chip { label, data })
        }
        Pin::Words => {
            let w = c.get("selected_words").filter(|w| w.is_object())?;
            let text = w["text"].as_str().filter(|t| !t.is_empty())?;
            let mut data = w.clone();
            data["kind"] = json!("words");
            Some(Chip { label: format!("Words: \u{201c}{}\u{201d}", short(text, 24)), data })
        }
    }
}

/// Add `chip` unless the same snapshot is already pinned. True when added.
pub fn pin_add(pinned: &mut Vec<Chip>, chip: Chip) -> bool {
    if pinned.iter().any(|c| c.data == chip.data) {
        return false;
    }
    pinned.push(chip);
    true
}

thread_local! {
    /// "Add to Assistant" clicks from right-click menus, drained once per frame by the Assistant's tick
    /// (the same shape as `ui::menu`'s action queue).
    static PINS: RefCell<Vec<Pin>> = const { RefCell::new(Vec::new()) };
}

pub fn request_pin(ctx: &eframe::egui::Context, pin: Pin) {
    PINS.with(|q| q.borrow_mut().push(pin));
    ctx.request_repaint();
}

pub fn take_pins() -> Vec<Pin> {
    PINS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

// ---------------------------------------------------------------------------------------------------
// attachments

/// Write an attachment for top-down RGBA `rgba` (`w` x `h`): a PNG at most `ATTACH_MAX` on its long
/// edge for the agent, and a `THUMB` thumbnail, both under `dir`.
pub fn write_attachment(dir: &Path, name: &str, rgba: &[u8], w: u32, h: u32) -> Result<Attachment, String> {
    if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
        return Err("empty image".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let scale = (ATTACH_MAX as f32 / w.max(h) as f32).min(1.0);
    let (pw, ph) = (((w as f32 * scale) as u32).max(1), ((h as f32 * scale) as u32).max(1));
    let mut frame = crate::media::Frame::new(pw, ph);
    resample(rgba, w, h, &mut frame.rgba, pw, ph, (0, 0, pw, ph));
    let mut thumb = vec![0u8; (THUMB[0] * THUMB[1] * 4) as usize];
    for px in thumb.chunks_exact_mut(4) {
        px[3] = 255;
    }
    let s = (THUMB[0] as f32 / w as f32).min(THUMB[1] as f32 / h as f32);
    let (tw, th) = (((w as f32 * s) as u32).clamp(1, THUMB[0]), ((h as f32 * s) as u32).clamp(1, THUMB[1]));
    resample(rgba, w, h, &mut thumb, THUMB[0], THUMB[1], ((THUMB[0] - tw) / 2, (THUMB[1] - th) / 2, tw, th));
    let stem = format!("{:x}", unique_id());
    let png = dir.join(format!("{stem}.png"));
    let thumb_path = dir.join(format!("{stem}.thumb"));
    std::fs::write(&png, crate::mcp::png_encode(&frame)).map_err(|e| e.to_string())?;
    std::fs::write(&thumb_path, &thumb).map_err(|e| e.to_string())?;
    Ok(Attachment { name: name.to_string(), png, thumb: thumb_path, width: pw, height: ph })
}

/// Nearest-neighbour copy of `src` (sw x sh) into the `(x, y, w, h)` box of `dst` (dw wide).
fn resample(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, _dh: u32, (bx, by, bw, bh): (u32, u32, u32, u32)) {
    for y in 0..bh {
        let sy = (y as u64 * sh as u64 / bh as u64) as usize;
        for x in 0..bw {
            let sx = (x as u64 * sw as u64 / bw as u64) as usize;
            let s = (sy * sw as usize + sx) * 4;
            let d = (((by + y) * dw + bx + x) * 4) as usize;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

/// A thumbnail file back as RGBA (None when missing or the wrong size).
pub fn read_thumb(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok().filter(|b| b.len() == (THUMB[0] * THUMB[1] * 4) as usize)
}

/// Millisecond clock, never the same value twice in a process (ids for conversations and files).
pub fn unique_id() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = (now_secs() * 1000.0) as u64;
    let mut prev = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(prev + 1);
        match LAST.compare_exchange(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(p) => prev = p,
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// store

/// `%APPDATA%\SimpleEditor\assistant`.
pub fn store_dir() -> PathBuf {
    crate::settings::Settings::dir().join("assistant")
}

/// The store bucket of a project: a stable hash of its path, or "untitled" for an unsaved project.
pub fn bucket(project: Option<&Path>) -> String {
    match project {
        // FNV-1a: std's hasher is not promised to stay the same across Rust releases
        Some(p) => {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in p.to_string_lossy().to_lowercase().bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
            format!("{h:016x}")
        }
        None => "untitled".into(),
    }
}

/// A bucket's conversations (oldest first); empty when there are none or the file is unreadable.
pub fn load(dir: &Path, bucket: &str) -> Vec<Conversation> {
    std::fs::read_to_string(dir.join(format!("{bucket}.json")))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Keep the `MAX_CONVERSATIONS` most recently used (empty ones are never kept); returns the dropped
/// ones so their attachment files can go too.
pub fn cap(convs: &[Conversation]) -> (Vec<Conversation>, Vec<Conversation>) {
    let mut keep: Vec<Conversation> = convs.iter().filter(|c| !c.is_empty()).cloned().collect();
    keep.sort_by(|a, b| a.updated.total_cmp(&b.updated));
    let cut = keep.len().saturating_sub(MAX_CONVERSATIONS);
    let dropped = keep.drain(..cut).collect();
    (keep, dropped)
}

/// The JSON `save` writes, plus the files of the conversations it drops.
pub fn snapshot(convs: &[Conversation]) -> (String, Vec<PathBuf>) {
    let (keep, dropped) = cap(convs);
    let files = dropped.iter().flat_map(|c| c.attachments().flat_map(|a| [a.png.clone(), a.thumb.clone()])).collect();
    (serde_json::to_string(&keep).unwrap_or_else(|_| "[]".into()), files)
}

/// Write a bucket off the UI thread (temp file + rename). Writes are serialised and an older write that
/// lost the race to a newer one is skipped.
pub fn save_async(dir: PathBuf, bucket: String, json: String, remove: Vec<PathBuf>) {
    static GEN: AtomicU64 = AtomicU64::new(0);
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = std::thread::Builder::new().name("assistant-store".into()).spawn(move || {
        let _g = LOCK.lock();
        if GEN.load(Ordering::SeqCst) != gen {
            return; // a newer snapshot is queued behind us
        }
        let _ = save(&dir, &bucket, &json);
        for f in remove {
            let _ = std::fs::remove_file(f);
        }
    });
}

pub fn save(dir: &Path, bucket: &str, json: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{bucket}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(id: u64, updated: f64, text: &str) -> Conversation {
        Conversation {
            id,
            title: text.into(),
            created: updated,
            updated,
            session: Some(format!("s{id}")),
            lines: vec![Line::User(text.into()), Line::Done { undo: None }],
            tasks: Vec::new(),
        }
    }

    /// A bucket round-trips through its JSON file; past the cap the least recently used conversations go
    /// (with their attachment files listed for removal), empty ones are never written.
    #[test]
    fn conversation_store_round_trip_and_cap() {
        let dir = std::env::temp_dir().join(format!("se-assistant-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut convs: Vec<Conversation> = (0..MAX_CONVERSATIONS as u64 + 3).map(|i| conv(i, i as f64, "hi")).collect();
        convs[0].lines.push(Line::Attached {
            chips: vec!["2 clips".into()],
            images: vec![Attachment {
                name: "frame".into(),
                png: dir.join("a.png"),
                thumb: dir.join("a.thumb"),
                width: 4,
                height: 2,
            }],
        });
        convs[1].tasks.push(Task {
            id: 1,
            title: "t".into(),
            note: String::new(),
            code: "x".into(),
            status: TaskStatus::Ready,
            error: String::new(),
        });
        convs.push(Conversation::new(999)); // empty: dropped silently
        let (json, files) = snapshot(&convs);
        assert_eq!(files, vec![dir.join("a.png"), dir.join("a.thumb")], "the oldest conversation's files go");
        save(&dir, "b", &json).unwrap();
        let back = load(&dir, "b");
        assert_eq!(back.len(), MAX_CONVERSATIONS);
        assert_eq!(back[0].id, 3, "the three least recently used are dropped");
        assert_eq!(back.last().unwrap(), &convs[MAX_CONVERSATIONS + 2], "lines, session and title survive");
        assert!(load(&dir, "missing").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        // buckets: stable per path (case-insensitive like Windows), "untitled" without one
        let a = bucket(Some(Path::new("C:\\Videos\\Trip.sedit")));
        assert_eq!(a, bucket(Some(Path::new("c:\\videos\\trip.sedit"))));
        assert_ne!(a, bucket(Some(Path::new("C:\\Videos\\Other.sedit"))));
        assert_eq!(bucket(None), "untitled");
    }

    /// add (Writing) → handed over (Ready) → run ok (Done) / run fail (Failed + error); Run all runs the
    /// Ready ones in queue order and a failure stops it.
    #[test]
    fn task_state_machine() {
        let mut t = Vec::new();
        let a = task_writing(&mut t, "Cut the ums");
        assert_eq!(t[0].status, TaskStatus::Writing);
        assert!(!t[0].runnable(), "nothing to run while it is being written");
        assert_eq!(task_hand_over(&mut t, Some(a), "Cut ums", "editor.log('a')", "cuts them"), a);
        assert_eq!((t[0].status, t[0].title.as_str()), (TaskStatus::Ready, "Cut ums"));
        // the slot is filled: a second hand-over queues a new task
        let b = task_hand_over(&mut t, Some(a), "Fail", "error('x')", "");
        let c = task_hand_over(&mut t, None, "Never", "editor.log('c')", "");
        assert_eq!((t.len(), b, c), (3, a + 1, a + 2));
        let mut ran = Vec::new();
        let (n, err) = run_queue(&mut t, None, &mut |task| {
            ran.push(task.id);
            if task.code.contains("error") {
                Err("boom".into())
            } else {
                Ok(())
            }
        });
        assert_eq!((n, err.as_deref()), (2, Some("boom")));
        assert_eq!(ran, vec![a, b], "the failure stopped the queue before c");
        assert_eq!(t[0].status, TaskStatus::Done);
        assert_eq!((t[1].status, t[1].error.as_str()), (TaskStatus::Failed, "boom"));
        assert_eq!(t[2].status, TaskStatus::Ready);
        // fixing the code makes it Ready again; a single Run reruns a Done task too
        task_set(&mut t, b, None, Some("editor.log('ok')")).unwrap();
        assert_eq!((t[1].status, t[1].error.as_str()), (TaskStatus::Ready, ""));
        assert_eq!(run_queue(&mut t, Some(a), &mut |_| Ok(())), (1, None));
        // order and removal
        task_move(&mut t, c, true);
        assert_eq!(t.iter().map(|t| t.id).collect::<Vec<_>>(), vec![a, c, b]);
        task_move(&mut t, a, true); // already first
        assert_eq!(t[0].id, a);
        assert!(task_remove(&mut t, c) && !task_remove(&mut t, c));
        assert!(task_set(&mut t, 77, None, None).is_err());
        // "Save to Scripts": a header the Scripts menu reads, and a free file name
        let s = task_script(&t[0]);
        assert!(s.starts_with("-- @name Cut ums\n-- @desc cuts them\n\n") && s.ends_with("editor.log('a')\n"), "{s}");
        let dir = std::env::temp_dir().join(format!("se-assistant-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = script_path(&dir, "Cut ums!");
        assert_eq!(p.file_name().unwrap(), "cut-ums.luau");
        std::fs::write(&p, "").unwrap();
        assert_eq!(script_path(&dir, "Cut ums!").file_name().unwrap(), "cut-ums-2.luau");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pinned chips snapshot the context by kind; the smart pick prefers clips, then words, marker, range.
    #[test]
    fn pinned_chips() {
        let c = json!({
            "in": 1.0, "out": 4.5,
            "selected_clips": [{"id": 3, "name": "intro.mp4"}, {"id": 4, "name": "b.mp4"}],
            "selected_markers": [{"id": 9, "name": "Comment", "note": "too loud", "t": 2.0}],
            "selected_words": {"clip_id": 3, "text": "um so", "start": 1.0, "end": 1.4},
        });
        assert_eq!(pin_chip(&c, Pin::Smart).unwrap().label, "2 clips");
        assert_eq!(pin_chip(&c, Pin::Range).unwrap().data, json!({"kind": "range", "in": 1.0, "out": 4.5}));
        assert_eq!(pin_chip(&c, Pin::Marker).unwrap().label, "Marker: too loud");
        assert_eq!(pin_chip(&c, Pin::Words).unwrap().data["kind"], "words");
        let bare = json!({"in": null, "out": null, "selected_clips": [], "selected_markers": [], "selected_words": null});
        assert!(pin_chip(&bare, Pin::Smart).is_none());
        let mut pinned = Vec::new();
        assert!(pin_add(&mut pinned, pin_chip(&c, Pin::Range).unwrap()));
        assert!(!pin_add(&mut pinned, pin_chip(&c, Pin::Range).unwrap()), "the same snapshot twice is one chip");
        assert!(pin_add(&mut pinned, pin_chip(&c, Pin::Clips).unwrap()));
        assert_eq!(pinned.len(), 2);
        assert_eq!(Pin::parse("range"), Some(Pin::Range));
        assert_eq!(Pin::parse("nope"), None);
        // the menu queue drains once
        let ctx = eframe::egui::Context::default();
        request_pin(&ctx, Pin::Marker);
        assert_eq!(take_pins(), vec![Pin::Marker]);
        assert!(take_pins().is_empty());
    }

    /// An attachment is a capped PNG the agent can read plus a letterboxed thumbnail.
    #[test]
    fn attachments_are_written_and_listed() {
        let dir = std::env::temp_dir().join(format!("se-assistant-att-{}", std::process::id()));
        let (w, h) = (2048, 512);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 7, 9, 255]).collect();
        let a = write_attachment(&dir, "frame", &rgba, w, h).unwrap();
        assert_eq!((a.width, a.height), (ATTACH_MAX, 256), "the long edge is capped, the aspect kept");
        let png = std::fs::read(&a.png).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(read_thumb(&a.thumb).is_some());
        assert_ne!(a.png, write_attachment(&dir, "frame", &rgba, w, h).unwrap().png, "every attachment gets its own file");
        assert!(write_attachment(&dir, "x", &[], 0, 0).is_err());
        let c = Conversation {
            lines: vec![Line::Attached { chips: vec![], images: vec![a.clone()] }, Line::Text("ok".into())],
            ..Default::default()
        };
        assert_eq!(c.attachments().collect::<Vec<_>>(), vec![&a]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
