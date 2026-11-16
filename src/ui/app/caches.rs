//! ---- ws:forgiveness ----
//! Clear Caches: release decoder file handles, drop in-memory waveform/thumb caches, then delete the
//! on-disk cache dir on a spawned thread (deletion never blocks the frame).

use super::*;

/// Total bytes under `dir` (recursive). Free function, not tied to `Settings::cache_dir()`, so a test
/// can point it at a temp directory.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub(super) fn cache_bytes() -> u64 {
    dir_bytes(&Settings::cache_dir())
}

// ---- ws:cache-budget ----
use crate::engine::cache_lru::{self, Protect};

/// When the pruner runs next, the budget it last ran with, and whether the low-space toast was shown.
struct Sched {
    next: Instant,
    budget: u32,
    warned: bool,
}
static SCHED: Mutex<Option<Sched>> = Mutex::new(None);
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const FIRST: Duration = Duration::from_secs(45);
const EVERY: Duration = Duration::from_secs(3600);

/// What the open project is using: html outputs its assets point at (never pruned), the proxies its video
/// assets map to and the pre-render seconds rendered this session (pruned last).
fn protect_for(app: &App) -> Protect {
    let mut p = Protect::default();
    let cache = Settings::cache_dir();
    let h = app.settings.proxy_height.max(120);
    for a in &app.project.assets {
        if Path::new(&a.path).starts_with(&cache) {
            p.hard.insert(cache_lru::stem_of(Path::new(&a.path)));
        } else if a.kind == ClipKind::Video {
            let h = media::proxy::height_for(a, h);
            for stacked in [false, true] {
                p.soft.insert(cache_lru::stem_of(&media::proxy::stacked_or_plain(&a.path, h, stacked)));
            }
        }
    }
    p.soft.extend(app.prerender.done_stems());
    p
}

/// Prune on a low-priority thread (one at a time); the UI never waits for it. The HTML renders' cap (and
/// the one-time removal of the old PNG-in-MOV renders) always, the whole budget when there is one.
fn spawn_prune(budget: Option<u64>, protect: Protect) {
    use std::sync::atomic::Ordering;
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new().name("cache-prune".into()).spawn(move || {
        // SAFETY: lowers the priority of this thread only.
        unsafe {
            use windows::Win32::System::Threading::*;
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        }
        let (dir, now) = (Settings::cache_dir(), std::time::SystemTime::now());
        cache_lru::prune_html(&dir, cache_lru::HTML_CAP, &protect, now);
        if let Some(b) = budget {
            cache_lru::prune(&dir, b, &protect, now);
        }
        RUNNING.store(false, Ordering::Release);
    });
}

/// Run the pruner now (the `caches.prune` tool).
pub(super) fn prune_now(app: &mut App) {
    spawn_prune(cache_lru::budget_bytes(app.settings.cache_disk_gb), protect_for(app));
}

pub(super) fn size_report(s: &Settings) -> Value {
    let dir = Settings::cache_dir();
    json!({
        "bytes": cache_bytes(),
        "budgeted_bytes": cache_lru::budgeted_bytes(&dir),
        "budget_bytes": cache_lru::budget_bytes(s.cache_disk_gb),
        "free_bytes": cache_lru::free_bytes(&dir),
    })
}

/// Frame hook: prune 45 s after start (the first run also clears the old PNG-in-MOV HTML renders), then
/// hourly and whenever the budget setting changes; warn once when the cache drive has less than twice the
/// budget free.
pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    if cfg!(test) {
        return; // a test never touches the real cache
    }
    move_tick(app);
    let now = Instant::now();
    let gb = app.settings.cache_disk_gb;
    let mut g = SCHED.lock().unwrap_or_else(|e| e.into_inner());
    let st = g.get_or_insert(Sched { next: now + FIRST, budget: gb, warned: false });
    if st.budget != gb {
        st.budget = gb;
        st.next = now;
    }
    if now < st.next || app.player.is_playing() {
        return;
    }
    st.next = now + EVERY;
    let b = cache_lru::budget_bytes(gb);
    let warn = b.is_some()
        && !st.warned
        && cache_lru::free_bytes(&Settings::cache_dir()).is_some_and(|f| cache_lru::low_on_space(f, b));
    st.warned |= warn;
    drop(g);
    spawn_prune(b, protect_for(app));
    if warn {
        app.push_toast(feedback::Toast::new("Little free space on the cache drive: lower the cache budget in Settings ▸ General").kind(feedback::ToastKind::Warn));
    }
}

/// `Action::ClearCaches` / the Performance-tab button / the `caches.clear` MCP tool. Order matters:
/// release the decoders' file handles first (they hold the proxy/source files open on Windows),
/// THEN drop the in-memory caches, THEN delete on disk - anything still locked (a background worker
/// mid-write) is simply left for the next start's cleanup.
pub(super) fn clear(app: &mut App) {
    let freed = cache_bytes();
    app.player.release_files();
    app.waveforms.clear();
    app.thumbs.clear();
    let dir = Settings::cache_dir();
    std::thread::spawn(move || {
        let _ = std::fs::remove_dir_all(&dir);
    });
    app.toast(format!("Cleared caches - {:.1} MB freed", freed as f64 / 1e6));
}

// ---- ws:neutral-web ----
/// The folders the project's media lives in (a cache folder may not go inside one of them).
fn media_dirs(app: &App) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = app.project_path.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
    let cache = Settings::cache_dir();
    for a in &app.project.assets {
        let p = Path::new(&a.path);
        if let Some(d) = p.parent().filter(|_| !p.starts_with(&cache)) {
            if !v.iter().any(|x| x == d) {
                v.push(d.to_path_buf());
            }
        }
    }
    v
}

/// Settings ▸ General ▸ Cache folder: use `new` ("" = the default) from now on, moving what is in the
/// old folder there (`mv`) or starting fresh (and with `delete_old`, deleting the old folder). Takes
/// effect at once, the way Clear Caches does: open files are let go first, renders already in flight
/// finish in the old folder (the move skips them while they are being written) and are made again in
/// the new one when next needed. Nothing that was being written is ever copied half-done.
pub(super) fn change_folder(app: &mut App, new: &str, mv: bool, delete_old: bool, root_ok: bool) -> Result<String, String> {
    use crate::engine::cache_move;
    if cache_move::running().is_some() {
        return Err("A cache move is still running: wait for it (or cancel it in Jobs) first".into());
    }
    let old = Settings::cache_dir();
    let target = if new.trim().is_empty() { Settings::default_cache_dir() } else { PathBuf::from(new.trim()) };
    cache_move::validate(&target, &old, &media_dirs(app), root_ok)?;
    app.player.release_files();
    app.waveforms.clear();
    app.thumbs.clear();
    app.settings.cache_folder = if target == Settings::default_cache_dir() { String::new() } else { target.to_string_lossy().into_owned() };
    Settings::set_cache_folder(&app.settings.cache_folder);
    let msg = if mv {
        app.settings.cache_move_from = old.to_string_lossy().into_owned();
        cache_move::start(old, target.clone());
        format!("Cache folder is now {} - moving the old cache there in the background (Jobs)", target.display())
    } else {
        if delete_old {
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(&old);
            });
        }
        format!("Cache folder is now {}", target.display())
    };
    app.settings.save();
    Ok(msg)
}

/// A move that ended: say so and forget it. A move cut short by a quit or a crash starts again.
fn move_tick(app: &mut App) {
    use crate::engine::cache_move;
    if let Some(r) = cache_move::take_finished() {
        app.settings.cache_move_from.clear();
        app.settings.save();
        match r {
            Ok(m) => app.toast(m),
            Err(e) => app.push_toast(feedback::Toast::new(e).kind(feedback::ToastKind::Warn)),
        }
    } else if !app.settings.cache_move_from.is_empty() && cache_move::running().is_none() {
        let from = PathBuf::from(&app.settings.cache_move_from);
        if from.is_dir() && from != Settings::cache_dir() {
            cache_move::start(from, Settings::cache_dir());
        } else {
            app.settings.cache_move_from.clear();
            app.settings.save();
        }
    }
}

/// The cache move as a Jobs-pane row.
pub(super) fn move_jobs() -> Vec<jobs_pane::ProgJob> {
    crate::engine::cache_move::running()
        .map(|p| (crate::ui::jobs_ui::JobKind::Media, "Move cache folder".to_string(), p))
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_caches_empties_dir_and_reports_bytes() {
        let dir = std::env::temp_dir().join(format!("se-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 1024]).unwrap();
        std::fs::write(dir.join("sub").join("b.bin"), vec![0u8; 2048]).unwrap();
        assert_eq!(dir_bytes(&dir), 3072, "recurses into subdirectories");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(dir_bytes(&dir), 0, "a missing dir reports 0, not an error");
    }
}
