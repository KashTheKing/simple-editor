//! Background work never competes with playback: worker threads run below normal priority with
//! background I/O, child processes (ffmpeg, whisper, headless Edge) start below normal, and while a
//! player is running every worker stops between units of work (`pause_point`).
//! ponytail: a unit already running (one ffmpeg proxy, one HTML piece) finishes at low priority rather
//! than being suspended; suspend the child process if a single unit ever proves too long.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static PLAYING: AtomicBool = AtomicBool::new(false);

/// The UI sets this once a frame: the timeline or the Source monitor is playing.
pub fn set_playing(on: bool) {
    PLAYING.store(on, Ordering::Relaxed);
}

pub fn playing() -> bool {
    PLAYING.load(Ordering::Relaxed) || PAUSED.load(Ordering::Relaxed)
}

// ---- ws:perf-window ----
static PAUSED: AtomicBool = AtomicBool::new(false);
/// The Performance window's "Pause background jobs": workers wait as if something were playing.
pub fn set_paused(on: bool) {
    PAUSED.store(on, Ordering::Relaxed);
}
pub fn paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}

/// Call at the top of every background worker thread.
pub fn lower_thread() {
    use windows::Win32::System::Threading::*;
    // SAFETY: the pseudo handle of the calling thread; failure just leaves the priority as it was.
    unsafe {
        let t = GetCurrentThread();
        let _ = SetThreadPriority(t, THREAD_PRIORITY_BELOW_NORMAL);
        let _ = SetThreadPriority(t, THREAD_MODE_BACKGROUND_BEGIN); // low I/O + memory priority
    }
}

/// Between units of work: block while something is playing, until it stops or `cancelled`.
pub fn pause_point(cancelled: impl Fn() -> bool) {
    while playing() && !cancelled() {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `ffpipe::command` for a background child: below-normal priority class.
pub fn command(exe: &std::path::Path) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = crate::media::ffpipe::command(exe);
    c.creation_flags(0x0800_0000 | 0x0000_4000); // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_point_waits_for_playback_and_honours_cancel() {
        set_playing(false);
        pause_point(|| false); // returns at once
        set_playing(true);
        let t = std::time::Instant::now();
        let h = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(250));
            set_playing(false);
        });
        pause_point(|| false);
        assert!(t.elapsed() >= Duration::from_millis(200));
        h.join().unwrap();
        set_playing(true);
        pause_point(|| true); // cancelled: does not block
        set_playing(false);
    }

    #[test]
    fn lower_thread_does_not_panic() {
        std::thread::spawn(lower_thread).join().unwrap();
    }
}
