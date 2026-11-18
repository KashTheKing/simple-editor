//! Linked bins: a library bin that points at a folder on disk instead of holding one asset per file.
//!
//! Linking costs nothing up front (no walk, no probes, no proxies): `Project::link_bin` stores the folder
//! path and the bin name, and that is all a project file keeps. Everything else is read when it is looked at:
//! - `Cache::listing` reads ONE folder level on a worker the first time its bin is opened, so a bin of
//!   10 000 files never blocks a frame; sub-folders come back as nested bins.
//! - `Cache::duration` probes a file's length on one low-priority worker, newest request first, and is
//!   only asked for rows that are on screen (thumbnails already work that way - see `ui::library::row_art`).
//! - a watcher thread re-stats every listed folder (and every linked root) every `WATCH_EVERY`: a folder
//!   whose modified time moved is read again, a root that vanished turns the bin offline. It wakes the UI
//!   only when something changed, so an idle editor stays idle.
//!
//! A file becomes a real asset only when it is used (dropped on the timeline, opened in Source, added from
//! the menu): `Project::add_asset` files it under its linked bin. The UI half is `ui::library`'s tree.

use super::folder_import::natural_cmp;
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

/// How often the watcher re-stats what is listed.
pub const WATCH_EVERY: Duration = Duration::from_millis(1500);
/// Pending duration probes kept; older requests (rows long scrolled away) are dropped first.
const PROBE_QUEUE: usize = 64;

/// One folder level: sub-folder names and media files, both in natural order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Listing {
    pub dirs: Vec<String>,
    pub files: Vec<String>,
    /// The folder's modified time when it was read - the watcher reads it again once that moves.
    pub stamp: Option<SystemTime>,
}

/// What a bin shows while it is open.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Being read for the first time ("Listing…").
    Pending,
    Ready(Arc<Listing>),
    /// The folder is gone or unreadable.
    Missing,
}

/// Read one level of `dir`: hidden/system entries skipped, only files whose extension is in `exts`.
/// None = the folder can't be read (missing, offline drive, no access).
pub fn list_dir(dir: &Path, exts: &[&str]) -> Option<Listing> {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    let stamp = std::fs::metadata(dir).ok()?.modified().ok();
    let mut l = Listing { stamp, ..Default::default() };
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        if name.starts_with('.') || e.metadata().is_ok_and(|m| m.file_attributes() & HIDDEN_OR_SYSTEM != 0) {
            continue;
        }
        // a folder link is not followed (it could loop back up the tree); a file link is
        if t.is_dir() {
            l.dirs.push(name);
        } else {
            let ext = Path::new(&name).extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if exts.contains(&ext.as_str()) && (t.is_file() || std::fs::metadata(e.path()).is_ok_and(|m| m.is_file())) {
                l.files.push(e.path().to_string_lossy().into_owned());
            }
        }
    }
    l.dirs.sort_by(|a, b| natural_cmp(a, b));
    l.files.sort_by(|a, b| natural_cmp(file_name(a), file_name(b)));
    Some(l)
}

fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

fn key(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

#[derive(Default)]
struct Entry {
    state: Option<State>,
    /// A read is in flight (the first one, or a re-read after a change - the old listing stays up meanwhile).
    busy: bool,
}

type Prober = fn(&str) -> Option<f64>;

struct Inner {
    exts: Vec<String>,
    dirs: Mutex<HashMap<String, (PathBuf, Entry)>>,
    /// Linked roots -> online? (None until the watcher has looked).
    roots: Mutex<HashMap<String, (PathBuf, Option<bool>)>>,
    durations: Mutex<HashMap<String, Option<f64>>>,
    queue: Mutex<Vec<String>>,
    /// The file being probed right now (asking for it again must not queue a second probe).
    current: Mutex<Option<String>>,
    probing: AtomicBool,
    prober: Prober,
    wake: Mutex<Option<egui::Context>>,
    watching: AtomicBool,
}

/// The shared listing / metadata / watch state (cheap to clone; every clone is the same cache).
#[derive(Clone)]
pub struct Cache(Arc<Inner>);

impl Default for Cache {
    fn default() -> Self {
        Self::new(super::folder_import::MEDIA_EXTS, probe_duration)
    }
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("linked_bins::Cache")
    }
}

/// ffprobe/MF a file for its length (None = not media we can read; stills report 0).
fn probe_duration(path: &str) -> Option<f64> {
    crate::media::probe(path, crate::media::Backend::Auto).ok().map(|a| a.duration)
}

impl Cache {
    pub fn new(exts: &[&str], prober: Prober) -> Self {
        Self(Arc::new(Inner {
            exts: exts.iter().map(|e| e.to_string()).collect(),
            dirs: Mutex::default(),
            roots: Mutex::default(),
            durations: Mutex::default(),
            queue: Mutex::default(),
            current: Mutex::default(),
            probing: AtomicBool::new(false),
            prober,
            wake: Mutex::default(),
            watching: AtomicBool::new(false),
        }))
    }

    /// The context to wake when a read, a probe or the watcher has something new to show.
    pub fn set_ctx(&self, ctx: &egui::Context) {
        let mut w = lock(&self.0.wake);
        if w.is_none() {
            *w = Some(ctx.clone());
        }
    }

    /// `dir`'s listing; the first call starts the read on a worker and answers `Pending`.
    pub fn listing(&self, dir: &Path) -> State {
        self.start_watch();
        let k = key(dir);
        {
            let mut m = lock(&self.0.dirs);
            let (_, e) = m.entry(k.clone()).or_insert_with(|| (dir.to_path_buf(), Entry::default()));
            if let Some(s) = &e.state {
                return s.clone();
            }
            if e.busy {
                return State::Pending;
            }
            e.busy = true;
        }
        let me = Arc::downgrade(&self.0);
        let dir = dir.to_path_buf();
        std::thread::spawn(move || read_into(&me, &k, &dir));
        State::Pending
    }

    /// `listing` that blocks until the folder has been read (tests and the MCP tool's `list` mode).
    pub fn listing_now(&self, dir: &Path) -> State {
        let k = key(dir);
        let have = lock(&self.0.dirs).get(&k).and_then(|(_, e)| e.state.clone());
        if let Some(s) = have {
            return s;
        }
        lock(&self.0.dirs).entry(k.clone()).or_insert_with(|| (dir.to_path_buf(), Entry::default())).1.busy = true;
        read_into(&Arc::downgrade(&self.0), &k, dir);
        lock(&self.0.dirs).get(&k).and_then(|(_, e)| e.state.clone()).unwrap_or(State::Missing)
    }

    /// Every folder read so far (what a Library search looks through - it never reads a folder itself).
    pub fn listed(&self) -> Vec<(PathBuf, Arc<Listing>)> {
        lock(&self.0.dirs)
            .values()
            .filter_map(|(p, e)| match &e.state {
                Some(State::Ready(l)) => Some((p.clone(), l.clone())),
                _ => None,
            })
            .collect()
    }

    /// Forget `dir` and everything below it (Refresh): the next look reads them again.
    pub fn forget(&self, dir: &Path) {
        let k = key(dir);
        lock(&self.0.dirs).retain(|d, _| !(d == &k || d.starts_with(&format!("{k}\\"))));
    }

    /// The linked roots the project has now (called each frame the library draws; a cheap compare).
    pub fn set_roots<'a>(&self, roots: impl Iterator<Item = &'a str>) {
        let mut m = lock(&self.0.roots);
        let want: Vec<(String, &str)> = roots.map(|r| (key(Path::new(r)), r)).collect();
        if want.len() == m.len() && want.iter().all(|(k, _)| m.contains_key(k)) {
            return;
        }
        m.retain(|k, _| want.iter().any(|(w, _)| w == k));
        for (k, r) in want {
            m.entry(k).or_insert_with(|| (PathBuf::from(r), None));
        }
        drop(m);
        self.start_watch();
    }

    /// Is the linked root there? None = not checked yet (drawn as online until the watcher says otherwise).
    pub fn online(&self, root: &str) -> Option<bool> {
        lock(&self.0.roots).get(&key(Path::new(root))).and_then(|(_, o)| *o)
    }

    /// The file's length once probed - `Some(None)` when it could not be read; `None` = not yet (asking
    /// queues the probe: newest asks first, one low-priority worker).
    pub fn duration(&self, path: &str) -> Option<Option<f64>> {
        if let Some(d) = lock(&self.0.durations).get(path) {
            return Some(*d);
        }
        {
            // queue, then current, then durations: the worker's order, so a file is always in one of them
            let mut q = lock(&self.0.queue);
            if lock(&self.0.current).as_deref() == Some(path) {
                return None;
            }
            if let Some(d) = lock(&self.0.durations).get(path) {
                return Some(*d);
            }
            // asked again = on screen again: to the front of the line (the end - it pops from there)
            if let Some(i) = q.iter().position(|p| p == path) {
                if i + 1 == q.len() {
                    return None;
                }
                q.remove(i);
            }
            q.push(path.to_string());
            if q.len() > PROBE_QUEUE {
                q.remove(0);
            }
        }
        if !self.0.probing.swap(true, Ordering::SeqCst) {
            let me = Arc::downgrade(&self.0);
            std::thread::spawn(move || probe_loop(me));
        }
        None
    }

    /// One watcher pass, on the calling thread: re-read every listed folder whose modified time moved and
    /// re-check every root. True when anything changed. The watcher thread calls this every `WATCH_EVERY`.
    pub fn poll(&self) -> bool {
        poll(&self.0)
    }

    fn start_watch(&self) {
        if self.0.watching.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = Arc::downgrade(&self.0);
        std::thread::spawn(move || loop {
            {
                let Some(inner) = me.upgrade() else { return }; // the cache is gone: so is the watcher
                if poll(&inner) {
                    wake(&inner);
                }
            }
            std::thread::sleep(WATCH_EVERY);
        });
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn wake(inner: &Inner) {
    if let Some(c) = lock(&inner.wake).as_ref() {
        c.request_repaint();
    }
}

fn read_into(me: &Weak<Inner>, k: &str, dir: &Path) {
    let exts: Vec<String> = match me.upgrade() {
        Some(i) => i.exts.clone(),
        None => return,
    };
    let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
    let state = match list_dir(dir, &exts) {
        Some(l) => State::Ready(Arc::new(l)),
        None => State::Missing,
    };
    let Some(inner) = me.upgrade() else { return };
    if let Some((_, e)) = lock(&inner.dirs).get_mut(k) {
        e.state = Some(state);
        e.busy = false;
    }
    wake(&inner);
}

fn poll(inner: &Arc<Inner>) -> bool {
    let mut changed = false;
    let roots: Vec<(String, PathBuf, Option<bool>)> =
        lock(&inner.roots).iter().map(|(k, (p, o))| (k.clone(), p.clone(), *o)).collect();
    for (k, p, was) in roots {
        let now = Some(p.is_dir());
        if now != was {
            if let Some(e) = lock(&inner.roots).get_mut(&k) {
                e.1 = now;
            }
            changed = true;
            // back online: whatever was read as missing under it reads again
            if now == Some(true) {
                lock(&inner.dirs).retain(|d, (_, e)| !(d.starts_with(&k) && e.state == Some(State::Missing)));
            }
        }
    }
    let listed: Vec<(String, PathBuf, Option<SystemTime>, bool)> = lock(&inner.dirs)
        .iter()
        .filter(|(_, (_, e))| !e.busy)
        .filter_map(|(k, (p, e))| match &e.state {
            Some(State::Ready(l)) => Some((k.clone(), p.clone(), l.stamp, true)),
            Some(State::Missing) => Some((k.clone(), p.clone(), None, false)),
            _ => None,
        })
        .collect();
    for (k, p, stamp, was_there) in listed {
        let now = std::fs::metadata(&p).ok().map(|m| m.modified().ok());
        let moved = match now {
            Some(t) => !was_there || t != stamp,
            None => was_there,
        };
        if moved {
            if let Some((_, e)) = lock(&inner.dirs).get_mut(&k) {
                e.busy = true;
            }
            read_into(&Arc::downgrade(inner), &k, &p);
            changed = true;
        }
    }
    changed
}

fn probe_loop(me: Weak<Inner>) {
    lower_priority();
    loop {
        let Some(inner) = me.upgrade() else { return };
        let next = {
            // taken and marked current under one lock order (queue, then current), so `duration` never
            // sees the file in neither place
            let mut q = lock(&inner.queue);
            let next = q.pop(); // newest first: what is on screen now
            *lock(&inner.current) = next.clone();
            next
        };
        let Some(path) = next else {
            inner.probing.store(false, Ordering::SeqCst);
            // a request that raced the flag: take it rather than strand it
            if lock(&inner.queue).is_empty() || inner.probing.swap(true, Ordering::SeqCst) {
                return;
            }
            continue;
        };
        let prober = inner.prober;
        drop(inner); // a long probe must not keep a dropped cache alive
        let d = prober(&path);
        let Some(inner) = me.upgrade() else { return };
        {
            let _q = lock(&inner.queue);
            let mut cur = lock(&inner.current);
            lock(&inner.durations).insert(path, d);
            *cur = None;
        }
        wake(&inner);
    }
}

/// Probes are background work: below playback and the UI thread (not "lowest" - a busy machine starved it).
fn lower_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
    // SAFETY: the pseudo-handle of the calling thread, always valid
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

#[cfg(test)]
mod tests;
