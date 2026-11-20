//! How a running job tells the window what it is doing, and hears Cancel.

use std::sync::atomic::{AtomicBool, Ordering};

pub struct Report<'a> {
    say: &'a (dyn Fn(f32, &str) + Sync),
    cancel: &'a AtomicBool,
}

impl<'a> Report<'a> {
    pub fn new(say: &'a (dyn Fn(f32, &str) + Sync), cancel: &'a AtomicBool) -> Self {
        Report { say, cancel }
    }
    /// `frac` is the whole job's progress, 0..=1.
    pub fn say(&self, frac: f32, text: &str) {
        (self.say)(frac.clamp(0.0, 1.0), text)
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Err("Cancelled") once Cancel was pressed.
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
}

pub const CANCELLED: &str = "Cancelled";

/// A report that goes nowhere (tests, silent runs).
pub fn quiet<R>(f: impl FnOnce(&Report) -> R) -> R {
    let cancel = AtomicBool::new(false);
    let say = |_: f32, _: &str| {};
    f(&Report::new(&say, &cancel))
}

/// "12.3 MB"
pub fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}
