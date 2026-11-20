//! ---- forum-crash ----
//! Crash fingerprint: what the app was doing when it died.
//!
//! Risky work (opening or probing a media file, a drop onto the timeline, a newly added effect, an
//! export, the first frame of graphics, a hardware-encoder test) registers an in-flight entry with
//! `begin` (ends when its `Guard` drops), `linger` (ends after a few seconds) or `linger_while` (ends
//! when a check says the job is over). The table lives in memory; a writer thread mirrors it to
//! `inflight-<pid>.json` in the settings folder a moment later, and the panic hook / the Windows
//! unhandled-exception filter write it synchronously as the process dies. So nothing here touches the
//! disk on the caller's thread, and an entry shorter than the writer's coalescing delay never reaches
//! the disk at all.
//!
//! On the next start `take_leftovers` reads the files of processes that are no longer running: those
//! entries name the file or action the previous run died on (`recovery::boot` takes the file offline
//! and says which one; `main` offers compatibility graphics for a graphics death).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What was in flight. Serialized by name, so the file stays readable across versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Opening a project file.
    Open,
    /// Reading a media file's streams (import).
    Probe,
    /// Opening a decoder on a media file (preview, export, thumbnails).
    Decode,
    /// Files dropped on the window or dragged onto the timeline.
    Drag,
    /// An effect just added to a clip.
    Filter,
    /// An export or bake.
    Export,
    /// Creating the window and its OpenGL context, up to the first painted frame.
    Graphics,
    /// A hardware encoder's one-frame test.
    EncoderTest,
}

impl Kind {
    /// Plain verb for the recovery message: "while <verb> <file>".
    pub fn verb(self) -> &'static str {
        match self {
            Kind::Open => "opening",
            Kind::Probe => "reading",
            Kind::Decode => "playing",
            Kind::Drag => "adding",
            Kind::Filter => "applying an effect to",
            Kind::Export => "exporting",
            Kind::Graphics => "starting its graphics",
            Kind::EncoderTest => "testing the hardware encoder",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: Kind,
    /// The file involved ("" when none).
    #[serde(default)]
    pub path: String,
    /// Extra words: the effect name, the encoder, ...
    #[serde(default)]
    pub detail: String,
}

enum Until {
    Guard,
    Time(Instant),
    While(Box<dyn Fn() -> bool + Send>),
}

impl Until {
    fn live(&self, now: Instant) -> bool {
        match self {
            Until::Guard => true,
            Until::Time(t) => now < *t,
            Until::While(f) => f(),
        }
    }
}

struct Slot {
    id: u64,
    entry: Entry,
    until: Until,
}

#[derive(Default)]
struct Table {
    next: u64,
    slots: Vec<Slot>,
    /// Bumped on every change; the writer sleeps until it moves.
    gen: u64,
}

static TABLE: Mutex<Table> = Mutex::new(Table { next: 0, slots: Vec::new(), gen: 0 });
static WAKE: Condvar = Condvar::new();
/// Set once `init` ran: where the files go. Unset (tests, `--selftest`) = memory only.
static DIR: OnceLock<PathBuf> = OnceLock::new();
/// The process is dying: freeze the file as the crash path last wrote it (unwinding drops guards,
/// and those removals must not reach the disk).
static FROZEN: AtomicBool = AtomicBool::new(false);
/// Leftovers of dead processes, read once at start by `init`.
static LEFTOVERS: OnceLock<Leftover> = OnceLock::new();
/// The open project's file ("" = untitled), written with the entries so the next start can reopen it.
static PROJECT: Mutex<String> = Mutex::new(String::new());

/// What a dead run left: its open project and what was in flight.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Leftover {
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// The open project changed (`App::set_project`): cheap, no disk.
pub fn set_project(path: Option<&Path>) {
    let p = path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut g = PROJECT.lock().unwrap_or_else(|e| e.into_inner());
    if *g != p {
        *g = p;
        drop(g);
        let mut t = lock();
        t.gen += 1;
        WAKE.notify_all();
    }
}

fn project_now() -> String {
    PROJECT.try_lock().map(|g| g.clone()).unwrap_or_default()
}

/// How long the writer waits after a change before writing, so short opens never hit the disk.
const COALESCE: Duration = Duration::from_millis(150);

fn lock() -> std::sync::MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ends its entry when dropped.
#[must_use = "the entry ends when the guard drops"]
pub struct Guard(u64);

impl Drop for Guard {
    fn drop(&mut self) {
        end(self.0);
    }
}

fn push(entry: Entry, until: Until) -> u64 {
    let mut t = lock();
    t.next += 1;
    let id = t.next;
    t.slots.push(Slot { id, entry, until });
    t.gen += 1;
    WAKE.notify_all();
    id
}

fn end(id: u64) {
    let mut t = lock();
    let before = t.slots.len();
    t.slots.retain(|s| s.id != id);
    if t.slots.len() != before {
        t.gen += 1;
        WAKE.notify_all();
    }
}

fn entry(kind: Kind, path: &str, detail: &str) -> Entry {
    Entry { kind, path: path.to_string(), detail: detail.to_string() }
}

/// In flight until the returned guard drops.
pub fn begin(kind: Kind, path: &str, detail: &str) -> Guard {
    Guard(push(entry(kind, path, detail), Until::Guard))
}

/// In flight for `secs` from now: work whose risky part runs later on another thread (an effect's
/// first renders, a drop's first decodes).
pub fn linger(kind: Kind, path: &str, detail: &str, secs: f32) {
    push(entry(kind, path, detail), Until::Time(Instant::now() + Duration::from_secs_f32(secs)));
}

/// In flight while `running()` is true (checked by the writer about twice a second).
pub fn linger_while(kind: Kind, path: &str, detail: &str, running: impl Fn() -> bool + Send + 'static) {
    push(entry(kind, path, detail), Until::While(Box::new(running)));
}

/// Ends every entry of `kind` (the first painted frame ends `Graphics`).
pub fn end_kind(kind: Kind) {
    let mut t = lock();
    let before = t.slots.len();
    t.slots.retain(|s| s.entry.kind != kind);
    if t.slots.len() != before {
        t.gen += 1;
        WAKE.notify_all();
    }
}

/// The first frame is on screen: ends `Graphics` (once; later calls are one atomic load).
pub fn graphics_ready() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        end_kind(Kind::Graphics);
    }
}

/// The live entries right now (expired lingering ones dropped).
pub fn snapshot() -> Vec<Entry> {
    let now = Instant::now();
    let mut t = lock();
    t.slots.retain(|s| s.until.live(now));
    t.slots.iter().map(|s| s.entry.clone()).collect()
}

fn file_for(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("inflight-{pid}.json"))
}

/// Mirror the state to this process's file: written while anything is in flight, removed otherwise.
pub fn write_to(dir: &Path, pid: u32, rec: &Leftover) {
    let f = file_for(dir, pid);
    if rec.entries.is_empty() {
        let _ = std::fs::remove_file(f);
    } else if let Ok(json) = serde_json::to_string(rec) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(f, json);
    }
}

/// Every `inflight-<pid>.json` in `dir` except `me`'s, from processes `alive` says are gone: their
/// entries (oldest file first) and the newest one's project; the files are removed so the same death
/// is reported once.
pub fn leftovers_in(dir: &Path, me: u32, alive: impl Fn(u32) -> bool) -> Leftover {
    let mut out = Leftover::default();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix("inflight-").and_then(|r| r.strip_suffix(".json")) else { continue };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        if pid == me || alive(pid) {
            continue;
        }
        let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        files.push((t, e.path()));
    }
    files.sort();
    for (_, f) in files {
        if let Some(r) = std::fs::read_to_string(&f).ok().and_then(|j| serde_json::from_str::<Leftover>(&j).ok()) {
            out.entries.extend(r.entries);
            if !r.project.is_empty() {
                out.project = r.project;
            }
        }
        let _ = std::fs::remove_file(f);
    }
    out
}

/// Is process `pid` still running? (Another instance's file is not a crash.)
#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        ok && code == STILL_ACTIVE.0 as u32
    }
}

#[cfg(not(windows))]
pub fn pid_alive(_pid: u32) -> bool {
    false
}

/// Called once from `main`, before anything risky: reads the previous run's leftovers, then starts
/// the writer for this one and hooks the native crash path.
pub fn init(dir: PathBuf) {
    let me = std::process::id();
    let _ = LEFTOVERS.set(leftovers_in(&dir, me, pid_alive));
    if DIR.set(dir).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("crashguard".into()).spawn(writer);
    install_native_filter();
}

/// What the previous run died on (no entries after a clean exit). Read-only; `init` filled it.
pub fn leftovers() -> &'static Leftover {
    static NONE: Leftover = Leftover { project: String::new(), entries: Vec::new() };
    LEFTOVERS.get().unwrap_or(&NONE)
}

fn writer() {
    let Some(dir) = DIR.get() else { return };
    let me = std::process::id();
    let mut seen_gen = u64::MAX;
    let mut last = Leftover::default();
    loop {
        {
            let mut t = lock();
            // with lingering entries the table changes by the clock too: re-check twice a second
            let polling = t.slots.iter().any(|s| !matches!(s.until, Until::Guard));
            while t.gen == seen_gen && !polling {
                t = WAKE.wait(t).unwrap_or_else(|e| e.into_inner());
            }
            if polling && t.gen == seen_gen {
                t = WAKE.wait_timeout(t, Duration::from_millis(500)).unwrap_or_else(|e| e.into_inner()).0;
            }
            seen_gen = t.gen;
        }
        std::thread::sleep(COALESCE);
        if FROZEN.load(Ordering::SeqCst) {
            return;
        }
        let now = Leftover { project: project_now(), entries: snapshot() };
        // a project change alone, with nothing in flight, has nothing to write
        if now != last && !(now.entries.is_empty() && last.entries.is_empty()) {
            write_to(dir, me, &now);
        }
        last = now;
    }
}

/// The process is going down (panic on the UI thread, access violation): write the table as it is
/// now and freeze it. Never blocks on the table lock (the dying thread may hold it).
pub fn flush_for_crash() {
    if FROZEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(dir) = DIR.get() else { return };
    let Ok(t) = TABLE.try_lock() else { return };
    let now = Instant::now();
    let entries: Vec<Entry> = t.slots.iter().filter(|s| s.until.live(now)).map(|s| s.entry.clone()).collect();
    drop(t);
    write_to(dir, std::process::id(), &Leftover { project: project_now(), entries });
}

/// A clean exit: nothing was in flight, whatever the table says now.
pub fn clean_exit() {
    FROZEN.store(true, Ordering::SeqCst);
    if let Some(dir) = DIR.get() {
        write_to(dir, std::process::id(), &Leftover::default());
    }
}

#[cfg(windows)]
fn install_native_filter() {
    use windows::Win32::System::Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS};
    unsafe extern "system" fn on_crash(_info: *const EXCEPTION_POINTERS) -> i32 {
        flush_for_crash();
        0 // EXCEPTION_CONTINUE_SEARCH: Windows Error Reporting still runs
    }
    unsafe {
        SetUnhandledExceptionFilter(Some(on_crash));
    }
}

#[cfg(not(windows))]
fn install_native_filter() {}

/// File name only, for messages ("clip.mp4", not the whole path).
pub fn short(path: &str) -> String {
    // both separators, whatever the host: a record can carry either
    path.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or(path).to_string()
}

/// One plain sentence naming what the previous run died on, or None when nothing was in flight.
/// The most specific entry wins: a decode or probe names the file, before a broader export or open.
pub fn describe(entries: &[Entry]) -> Option<String> {
    let rank = |k: Kind| match k {
        Kind::Decode | Kind::Probe => 0,
        Kind::Filter => 1,
        Kind::Drag => 2,
        Kind::EncoderTest => 3,
        Kind::Export => 4,
        Kind::Open => 5,
        Kind::Graphics => 6,
    };
    let e = entries.iter().min_by_key(|e| rank(e.kind))?;
    let what = match (e.kind, e.path.is_empty(), e.detail.is_empty()) {
        (Kind::Filter, false, false) => format!("{} {} ({})", e.kind.verb(), short(&e.path), e.detail),
        (Kind::EncoderTest, _, false) => format!("{} {}", e.kind.verb(), e.detail),
        (_, false, _) => format!("{} {}", e.kind.verb(), short(&e.path)),
        (_, true, false) => format!("{} {}", e.kind.verb(), e.detail),
        _ => e.kind.verb().to_string(),
    };
    Some(format!("Simple Editor closed unexpectedly while {what}."))
}

/// The media files to take offline after this death (deduplicated, in order).
pub fn culprit_files(entries: &[Entry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // a decode or probe is the direct culprit; a drop or filter only when nothing narrower ran
    let direct = entries.iter().any(|e| matches!(e.kind, Kind::Decode | Kind::Probe));
    for e in entries {
        let take = match e.kind {
            Kind::Decode | Kind::Probe => true,
            Kind::Drag | Kind::Filter => !direct,
            _ => false,
        };
        if take && !e.path.is_empty() && !out.contains(&e.path) {
            out.push(e.path.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-crashguard-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// One injected crash per kind of action the forum reports name: the previous run's file is
    /// found, the culprit named, and the media file listed for taking offline.
    #[test]
    fn injected_crash_names_the_culprit_for_drag_open_filter_export() {
        let cases: [(Entry, &str, Option<&str>); 4] = [
            (entry(Kind::Drag, r"C:\clips\broken.mp4", ""), "while adding broken.mp4", Some(r"C:\clips\broken.mp4")),
            (entry(Kind::Open, r"C:\p\film.sedit", ""), "while opening film.sedit", None),
            (
                entry(Kind::Filter, r"C:\clips\a.mov", "Glow"),
                "while applying an effect to a.mov (Glow)",
                Some(r"C:\clips\a.mov"),
            ),
            (entry(Kind::Export, r"C:\out\final.mp4", ""), "while exporting final.mp4", None),
        ];
        for (i, (e, words, offline)) in cases.into_iter().enumerate() {
            let dir = tmp(&format!("kind{i}"));
            let dead = 4_000_000 + i as u32; // the crashed run
            let rec = Leftover { project: r"C:\p\film.sedit".into(), entries: vec![e.clone()] };
            write_to(&dir, dead, &rec);
            // next start (pid 1): the dead run's entries and project come back once, then the file is gone
            let back = leftovers_in(&dir, 1, |_| false);
            assert_eq!(back, rec);
            assert!(leftovers_in(&dir, 1, |_| false).entries.is_empty(), "reported once");
            let got = back.entries;
            let msg = describe(&got).unwrap();
            assert!(msg.contains(words), "{msg:?} should say {words:?}");
            assert_eq!(culprit_files(&got), offline.map(String::from).into_iter().collect::<Vec<_>>());
            let _ = std::fs::remove_dir_all(&dir);
        }
        // "at most 30 s of lost work": the default autosave interval
        assert!(crate::settings::Settings::default().autosave_secs <= 30);
    }

    #[test]
    fn a_running_instance_and_this_process_are_not_crashes() {
        let dir = tmp("alive");
        let rec = |e: Entry| Leftover { project: String::new(), entries: vec![e] };
        write_to(&dir, 77, &rec(entry(Kind::Decode, "x.mp4", "")));
        write_to(&dir, 5, &rec(entry(Kind::Decode, "y.mp4", "")));
        assert!(leftovers_in(&dir, 5, |pid| pid == 77).entries.is_empty(), "77 still runs, 5 is me");
        assert_eq!(leftovers_in(&dir, 5, |_| false).entries.len(), 1, "77 has died since");
        // an empty table removes the file: a clean exit leaves nothing behind
        write_to(&dir, 9, &rec(entry(Kind::Open, "p.sedit", "")));
        write_to(&dir, 9, &Leftover::default());
        assert!(leftovers_in(&dir, 5, |_| false).entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decode_beats_the_broader_actions_and_lingers_expire() {
        let es = vec![
            entry(Kind::Export, "out.mp4", ""),
            entry(Kind::Drag, "dropped.mp4", ""),
            entry(Kind::Decode, r"D:\v\bad.mkv", ""),
        ];
        assert!(describe(&es).unwrap().contains("while playing bad.mkv"));
        assert_eq!(culprit_files(&es), vec![r"D:\v\bad.mkv".to_string()], "the drop is not blamed when a decode ran");
        assert_eq!(describe(&[]), None);
        assert!(describe(&[entry(Kind::EncoderTest, "", "h264_nvenc")]).unwrap().contains("hardware encoder h264_nvenc"));

        // the in-memory table: a guard ends on drop, a linger by the clock, linger_while by its check
        let flag = std::sync::Arc::new(AtomicBool::new(true));
        let f2 = flag.clone();
        {
            let _g = begin(Kind::Probe, "guard-test.mp4", "");
            linger(Kind::Filter, "linger-test.mp4", "", 0.0);
            linger_while(Kind::Export, "while-test.mp4", "", move || f2.load(Ordering::SeqCst));
            let now = snapshot();
            assert!(now.iter().any(|e| e.path == "guard-test.mp4"));
            assert!(!now.iter().any(|e| e.path == "linger-test.mp4"), "a 0 s linger is already over");
            assert!(now.iter().any(|e| e.path == "while-test.mp4"));
        }
        flag.store(false, Ordering::SeqCst);
        let now = snapshot();
        assert!(!now.iter().any(|e| e.path == "guard-test.mp4" || e.path == "while-test.mp4"));
    }
}
