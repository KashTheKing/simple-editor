//! parity-effects: the picture behind a custom gradient wipe (`Transition.image`), decoded once per
//! path off the UI thread and shared. The preview asks without waiting (`get(path, false)`: the first
//! frames use the shader's built-in pattern while it loads); export asks with `block = true` so every
//! exported frame uses the real image.

use crate::media::{Backend, Frame};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Gradient images are smooth ramps: this size keeps a 1080p wipe soft and costs ~0.9 MB.
pub const SIZE: (u32, u32) = (640, 360);

#[derive(Clone)]
enum Slot {
    Loading,
    Ready(Arc<Frame>),
    Failed,
}

fn cache() -> &'static (Mutex<HashMap<String, Slot>>, Condvar) {
    static C: OnceLock<(Mutex<HashMap<String, Slot>>, Condvar)> = OnceLock::new();
    C.get_or_init(|| (Mutex::new(HashMap::new()), Condvar::new()))
}

/// Decode `path` at `SIZE` into straight RGBA. Runs on a worker thread.
fn decode(path: &str) -> Option<Frame> {
    let mut src = crate::media::open_video(path, Backend::Auto).ok()?;
    let mut f = Frame::default();
    if !src.frame_at(0.0, SIZE.0, SIZE.1, &mut f) || f.is_empty() {
        return None;
    }
    f.make_rgba();
    Some(f)
}

/// The decoded image for `path`, starting its decode on first use. `block` waits for it (export).
/// None while loading (non-blocking), or when the file can't be read.
pub fn get(path: &str, block: bool) -> Option<Arc<Frame>> {
    if path.is_empty() {
        return None;
    }
    let (lock, cv) = cache();
    let mut map = lock.lock().unwrap_or_else(|e| e.into_inner());
    if !map.contains_key(path) {
        map.insert(path.to_string(), Slot::Loading);
        let p = path.to_string();
        std::thread::spawn(move || {
            let slot = decode(&p).map(|f| Slot::Ready(Arc::new(f))).unwrap_or(Slot::Failed);
            let (lock, cv) = cache();
            lock.lock().unwrap_or_else(|e| e.into_inner()).insert(p, slot);
            cv.notify_all();
        });
    }
    loop {
        match map.get(path).cloned() {
            Some(Slot::Ready(f)) => return Some(f),
            Some(Slot::Failed) | None => return None,
            Some(Slot::Loading) if !block => return None,
            Some(Slot::Loading) => {
                let (m, timeout) =
                    cv.wait_timeout(map, std::time::Duration::from_secs(10)).unwrap_or_else(|e| e.into_inner());
                map = m;
                if timeout.timed_out() {
                    return None;
                }
            }
        }
    }
}

/// Start decoding `path` now (the user just picked it), so the preview has it by the next frame.
pub fn warm(path: &str) {
    let _ = get(path, false);
}

/// Forget `path` (the file changed on disk, or a test).
pub fn forget(path: &str) {
    cache().0.lock().unwrap_or_else(|e| e.into_inner()).remove(path);
}

/// Seed the cache with an already-decoded picture (tests, and callers that have the pixels).
pub fn insert(path: &str, frame: Frame) {
    let (lock, cv) = cache();
    lock.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string(), Slot::Ready(Arc::new(frame)));
    cv.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_images_are_served_without_waiting() {
        let mut f = Frame::new(4, 4);
        f.fill([200, 200, 200, 255]);
        insert("test://wipe-a", f);
        assert_eq!(get("test://wipe-a", false).map(|f| f.width), Some(4));
        assert!(get("", true).is_none());
        forget("test://wipe-a");
    }
}
