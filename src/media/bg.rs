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

/// The playback threads (render, audio feeder) join the Multimedia Class Scheduler's "Playback" task
/// (as a media player's do; above normal when MMCSS is off). At normal priority they shared the CPU with
/// anything busy (parallel builds at 100 % CPU) and lost a whole scheduler quantum at a time: a lone
/// 1080x1920 clip dropped frames. MMCSS caps the boost (80 % by default), so a runaway loop cannot
/// freeze the PC.
pub fn raise_thread() {
    use windows::Win32::System::Threading::*;
    let mut task = 0u32;
    // SAFETY: plain Win32 calls on the calling thread; the MMCSS handle is kept for the thread's
    // lifetime (Windows releases it with the thread); a failure leaves the priority as it was.
    unsafe {
        if AvSetMmThreadCharacteristicsW(windows::core::w!("Playback"), &mut task).is_err() {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }
    }
}

/// The calling thread's priority, for the helper threads it starts to take on (`inherit`): a decode
/// helper of the render thread runs above normal, one of a background export below.
pub fn priority() -> i32 {
    use windows::Win32::System::Threading::*;
    // SAFETY: the pseudo handle of the calling thread
    unsafe { GetThreadPriority(GetCurrentThread()) }
}

/// Run the calling thread at the `priority()` of the thread that started it, kept between below and
/// above normal (an MMCSS thread reports its boosted level; its helpers stay ordinary threads).
pub fn inherit(p: i32) {
    use windows::Win32::System::Threading::*;
    let p = p.clamp(THREAD_PRIORITY_BELOW_NORMAL.0, THREAD_PRIORITY_ABOVE_NORMAL.0);
    // SAFETY: the pseudo handle of the calling thread; failure leaves the priority as it was
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(p));
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

    /// The playback threads run above normal (MMCSS "Playback", else ABOVE_NORMAL) and their decode
    /// helpers take that on, capped at above normal; a background worker's helpers stay below.
    #[test]
    fn playback_threads_outrank_a_busy_pc_and_helpers_inherit() {
        let (raised, helper) = std::thread::spawn(|| {
            raise_thread();
            let p = priority();
            (
                p,
                std::thread::spawn(move || {
                    inherit(p);
                    priority()
                })
                .join()
                .unwrap(),
            )
        })
        .join()
        .unwrap();
        assert!(raised > 0, "playback thread priority {raised}");
        assert_eq!(helper, 1, "a helper runs above normal, never higher");
        let low = std::thread::spawn(|| {
            lower_thread();
            let p = priority();
            std::thread::spawn(move || {
                inherit(p);
                priority()
            })
            .join()
            .unwrap()
        })
        .join()
        .unwrap();
        assert_eq!(low, -1, "a background worker's helper stays below normal");
    }
}
