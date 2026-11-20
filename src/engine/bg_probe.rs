//! ---- ws:page-switch-freeze ----
//! A value worked out once, off the UI thread: the list of microphones (`ffmpeg -list_devices`, 5-16 s on
//! some machines), the encoder list (`ffmpeg -encoders`, 2-3 s). `get` answers the value when it is ready,
//! else starts the job on a background thread (once) and answers `None`; `wake` runs when it lands (the UI
//! repaints). A pane drawing for the first time therefore never waits on a child process.
use std::sync::Mutex;

enum State<T> {
    Idle,
    Running,
    Done(T),
}

pub struct BgProbe<T> {
    state: Mutex<(u64, State<T>)>, // (generation, state): a `reset` drops the answer of a run already going
}

impl<T: Clone + Send + 'static> BgProbe<T> {
    pub const fn new() -> Self {
        BgProbe { state: Mutex::new((0, State::Idle)) }
    }

    /// The value, or `None` while it is being worked out (the first call starts `job`).
    pub fn get(&'static self, job: fn() -> T, wake: fn()) -> Option<T> {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match &g.1 {
            State::Done(v) => return Some(v.clone()),
            State::Running => return None,
            State::Idle => {}
        }
        g.1 = State::Running;
        let gen = g.0;
        drop(g);
        let spawned = std::thread::Builder::new().name("bg-probe".into()).spawn(move || {
            let v = job();
            let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if g.0 == gen {
                g.1 = State::Done(v);
            }
            drop(g);
            wake();
        });
        if spawned.is_err() {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = State::Done(job());
        }
        None
    }

    /// Forget the value (its inputs changed): the next `get` works it out again.
    pub fn reset(&self) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        g.0 += 1;
        g.1 = State::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    static SLOW: BgProbe<u32> = BgProbe::new();

    #[test]
    fn get_never_waits_on_the_job() {
        fn job() -> u32 {
            std::thread::sleep(Duration::from_millis(300));
            7
        }
        let t = Instant::now();
        assert_eq!(SLOW.get(job, || {}), None);
        assert_eq!(SLOW.get(job, || {}), None, "a second call while running starts nothing");
        assert!(t.elapsed() < Duration::from_millis(50), "get blocked for {:?}", t.elapsed());
        let deadline = Instant::now() + Duration::from_secs(5);
        while SLOW.get(job, || {}).is_none() {
            assert!(Instant::now() < deadline, "the job never landed");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(SLOW.get(job, || {}), Some(7));
        SLOW.reset();
        assert_eq!(SLOW.get(job, || {}), None, "reset works it out again");
    }
}
