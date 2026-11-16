//! Zero-dependency span recorder for `playback.trace`. A fixed ring of (name, thread, start, duration)
//! behind an atomic write index; `span("decode")` returns a guard that records on drop. While not
//! recording, a span costs one relaxed atomic load. Names come from `NAMES` (a span with another
//! name is dropped), which keeps the ring free of raw pointers.
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Instant;

/// The instrumented points. Add a name here before using it in `span`.
pub const NAMES: [&str; 8] =
    ["ui_frame", "timeline_draw", "inspector_draw", "render_frame", "layer_decode", "gpu_composite", "gpu_upload", "other"];

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
}

fn now_us() -> u64 {
    ORIGIN.get_or_init(Instant::now).elapsed().as_micros() as u64
}

pub fn span(name: &'static str) -> Span {
    if !RING.is_on() {
        return Span { name: usize::MAX, start_us: 0 };
    }
    let idx = NAMES.iter().position(|n| *n == name).unwrap_or(NAMES.len() - 1);
    Span { name: idx, start_us: now_us() }
}

impl Drop for Span {
    fn drop(&mut self) {
        if self.name != usize::MAX {
            RING.record(self.name, TID.with(|t| *t), self.start_us, now_us().saturating_sub(self.start_us));
        }
    }
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
    fn chrome_json_shape() {
        let j = chrome_json(&[Event { name: "decode", thread: 2, start_us: 10, dur_us: 3 }]);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let e = &v["traceEvents"][0];
        assert_eq!((e["name"].as_str(), e["ph"].as_str(), e["ts"].as_u64(), e["dur"].as_u64(), e["tid"].as_u64()),
                   (Some("decode"), Some("X"), Some(10), Some(3), Some(2)));
    }
}
