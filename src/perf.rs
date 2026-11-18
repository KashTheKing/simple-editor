//! Zero-dependency span recorder for `playback.trace`. A fixed ring of (name, thread, start, duration)
//! behind an atomic write index; `span("decode")` returns a guard that records on drop. While not
//! recording, a span costs one relaxed atomic load. Names come from `NAMES` (a span with another
//! name is dropped), which keeps the ring free of raw pointers.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Instant;

/// The instrumented points. Add a name here before using it in `span`.
pub const NAMES: [&str; 11] = [
    "ui_frame",
    "timeline_draw",
    "inspector_draw",
    "render_frame",
    "layer_decode",
    "gpu_composite",
    "gpu_upload",
    "nested_render",
    "effects",
    "audio_mix",
    "other",
];

struct Slot {
    name: AtomicU64, // NAMES index + 1; 0 = empty (written last)
    thread: AtomicU64,
    start_us: AtomicU64,
    dur_us: AtomicU64,
}

impl Slot {
    const fn new() -> Self {
        Slot { name: AtomicU64::new(0), thread: AtomicU64::new(0), start_us: AtomicU64::new(0), dur_us: AtomicU64::new(0) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: &'static str,
    pub thread: u64,
    pub start_us: u64,
    pub dur_us: u64,
}

pub struct Ring<const N: usize> {
    on: AtomicBool,
    next: AtomicUsize,
    slots: [Slot; N],
}

impl<const N: usize> Ring<N> {
    pub const fn new() -> Self {
        Ring { on: AtomicBool::new(false), next: AtomicUsize::new(0), slots: [const { Slot::new() }; N] }
    }
    pub fn start(&self) {
        self.on.store(false, Relaxed);
        for s in &self.slots {
            s.name.store(0, Relaxed);
        }
        self.next.store(0, Relaxed);
        self.on.store(true, Relaxed);
    }
    pub fn stop(&self) {
        self.on.store(false, Relaxed);
    }
    pub fn is_on(&self) -> bool {
        self.on.load(Relaxed)
    }
    /// Does nothing unless recording; the oldest entries are overwritten once the ring wraps.
    pub fn record(&self, name: usize, thread: u64, start_us: u64, dur_us: u64) {
        if !self.on.load(Relaxed) {
            return;
        }
        let s = &self.slots[self.next.fetch_add(1, Relaxed) % N];
        s.name.store(0, Relaxed);
        s.thread.store(thread, Relaxed);
        s.start_us.store(start_us, Relaxed);
        s.dur_us.store(dur_us, Relaxed);
        s.name.store(name as u64 + 1, Relaxed);
    }
    /// Recorded events, oldest first by start time.
    pub fn snapshot(&self) -> Vec<Event> {
        let mut v: Vec<Event> = self
            .slots
            .iter()
            .filter_map(|s| {
                let n = s.name.load(Relaxed) as usize;
                (n > 0 && n <= NAMES.len()).then(|| Event {
                    name: NAMES[n - 1],
                    thread: s.thread.load(Relaxed),
                    start_us: s.start_us.load(Relaxed),
                    dur_us: s.dur_us.load(Relaxed),
                })
            })
            .collect();
        v.sort_by_key(|e| e.start_us);
        v
    }
}

/// Chrome trace (chrome://tracing, Perfetto) JSON for `events`.
pub fn chrome_json(events: &[Event]) -> String {
    let mut s = String::from("{\"traceEvents\":[");
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s += &format!(
            "{{\"name\":\"{}\",\"ph\":\"X\",\"ts\":{},\"dur\":{},\"pid\":1,\"tid\":{}}}",
            e.name, e.start_us, e.dur_us, e.thread
        );
    }
    s + "],\"displayTimeUnit\":\"ms\"}"
}

static RING: Ring<65536> = Ring::new();
static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
static ORIGIN: OnceLock<Instant> = OnceLock::new();

thread_local! { static TID: u64 = NEXT_THREAD.fetch_add(1, Relaxed); }

pub fn start() {
    ORIGIN.get_or_init(Instant::now);
    RING.start();
}
pub fn stop() {
    RING.stop();
}
pub fn trace_json() -> String {
    chrome_json(&RING.snapshot())
}

#[must_use = "bind the guard: `let _s = perf::span(..);`"]
pub struct Span {
    name: usize, // usize::MAX = not recording
    start_us: u64,
    acc: bool, // pushed on this thread's STACK
}

fn now_us() -> u64 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_micros() as u64
}

pub fn span(name: &'static str) -> Span {
    let acc = ACC_ON.load(Relaxed);
    if !RING.is_on() && !acc {
        return Span { name: usize::MAX, start_us: 0, acc: false };
    }
    let idx = NAMES.iter().position(|n| *n == name).unwrap_or(NAMES.len() - 1);
    if acc {
        STACK.with(|s| s.borrow_mut().push((idx, 0)));
    }
    Span { name: idx, start_us: now_us(), acc }
}

impl Drop for Span {
    fn drop(&mut self) {
        if self.name == usize::MAX {
            return;
        }
        let dur = now_us().saturating_sub(self.start_us);
        if RING.is_on() {
            RING.record(self.name, TID.with(|t| *t), self.start_us, dur);
        }
        if self.acc {
            STACK.with(|s| {
                let mut s = s.borrow_mut();
                let child = s.pop().map_or(0, |e| e.1);
                if let Some(parent) = s.last_mut() {
                    parent.1 += dur;
                }
                // everything inside a nested sequence is that sequence's cost
                let owner = if s.iter().any(|e| e.0 == NESTED) { NESTED } else { self.name };
                SELF_US[owner].fetch_add(dur.saturating_sub(child), Relaxed);
            });
        }
    }
}

// ---- ws:perf-window ----
// The Performance window's live breakdown: while `accumulate(true)`, every span adds its SELF time (its
// duration minus the spans inside it, on the same thread) to a per-name total, so the totals never count
// a microsecond twice. A span inside `nested_render` is credited to it (the nested sequence's whole cost).
const NESTED: usize = 7; // NAMES index of "nested_render"
static ACC_ON: AtomicBool = AtomicBool::new(false);
static SELF_US: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];
thread_local! { static STACK: std::cell::RefCell<Vec<(usize, u64)>> = const { std::cell::RefCell::new(Vec::new()) }; }

pub fn accumulate(on: bool) {
    ORIGIN.get_or_init(Instant::now);
    ACC_ON.store(on, Relaxed);
}
pub fn accumulating() -> bool {
    ACC_ON.load(Relaxed)
}
/// Cumulative self time (µs) per `NAMES` entry since the process started accumulating.
pub fn self_totals() -> [u64; NAMES.len()] {
    std::array::from_fn(|i| SELF_US[i].load(Relaxed))
}

/// Measured decode cost of one source while accumulating: (decodes, total µs, any frame came zero-copy).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceStat {
    pub n: u64,
    pub us: u64,
    pub hw: bool,
}
static SOURCES: std::sync::Mutex<Vec<(String, SourceStat)>> = std::sync::Mutex::new(Vec::new());

/// One video decode of `path` (the asset path, not its proxy). Free unless accumulating.
/// ponytail: a linear scan under one lock per decode; a map if a project ever has hundreds of live sources.
pub fn source_decode(path: &str, us: u64, hw: bool) {
    if !ACC_ON.load(Relaxed) {
        return;
    }
    let mut v = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    let i = match v.iter().position(|(p, _)| p == path) {
        Some(i) => i,
        None => {
            v.push((path.to_string(), SourceStat::default()));
            v.len() - 1
        }
    };
    let s = &mut v[i].1;
    (s.n, s.us, s.hw) = (s.n + 1, s.us + us, s.hw || hw);
}
pub fn source_stat(path: &str) -> Option<SourceStat> {
    let v = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    v.iter().find(|(p, _)| p == path).map(|(_, s)| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_records_nothing() {
        let r = Ring::<4>::new();
        r.record(0, 1, 5, 5);
        assert!(r.snapshot().is_empty());
    }

    #[test]
    fn wraps_keeping_the_newest() {
        let r = Ring::<4>::new();
        r.start();
        for i in 0..10u64 {
            r.record(1, 1, i, 1);
        }
        let v = r.snapshot();
        assert_eq!(v.len(), 4);
        assert_eq!(v.iter().map(|e| e.start_us).collect::<Vec<_>>(), vec![6, 7, 8, 9]);
        r.stop();
        r.record(1, 1, 99, 1);
        assert_eq!(r.snapshot().len(), 4);
    }

    #[test]
    fn self_time_is_exclusive_and_nested_owns_its_children() {
        accumulate(true);
        let before = self_totals();
        {
            let _outer = span("render_frame");
            {
                let _n = span("nested_render");
                let _c = span("gpu_composite");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _d = span("layer_decode");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let after = self_totals();
        let d = |n: &str| {
            let i = NAMES.iter().position(|x| *x == n).unwrap();
            after[i] - before[i]
        };
        assert!(d("nested_render") >= 5000, "the composite inside a nested sequence is the nested cost");
        assert!(d("layer_decode") >= 5000);
        assert!(d("render_frame") < 2000, "the outer span keeps only its own time");
        source_decode("a.mp4", 10, false);
        source_decode("a.mp4", 30, true);
        assert_eq!(source_stat("a.mp4"), Some(SourceStat { n: 2, us: 40, hw: true }));
        accumulate(false);
        source_decode("b.mp4", 10, false);
        assert_eq!(source_stat("b.mp4"), None, "nothing is measured while off");
    }

    #[test]
    fn chrome_json_shape() {
        let j = chrome_json(&[Event { name: "decode", thread: 2, start_us: 10, dur_us: 3 }]);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let e = &v["traceEvents"][0];
        assert_eq!((e["name"].as_str(), e["ph"].as_str(), e["ts"].as_u64(), e["dur"].as_u64(), e["tid"].as_u64()),
                   (Some("decode"), Some("X"), Some(10), Some(3), Some(2)));
    }
}

/// Time since this process was created (the OS's own creation time, so it includes loading the exe
/// and everything before `main`) - the player's and the editor's launch-to-first-frame figure.
pub fn since_process_start() -> std::time::Duration {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    unsafe {
        let (mut created, mut a, mut b, mut c) = Default::default();
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut a, &mut b, &mut c).is_err() {
            return std::time::Duration::ZERO;
        }
        let now = ft(GetSystemTimePreciseAsFileTime());
        std::time::Duration::from_nanos(now.saturating_sub(ft(created)) * 100)
    }
}

/// `SE_STARTUP_TRACE=1`: print `<label> <ms since process start>` at each startup milestone (stderr).
pub fn startup_mark(label: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("SE_STARTUP_TRACE").is_some()) {
        eprintln!("startup {label:<28} {:7.1} ms", since_process_start().as_secs_f64() * 1000.0);
    }
}
