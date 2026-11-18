use super::*;
use crate::model::Project;
use std::time::Instant;

const EXTS: &[&str] = &["mp4", "wav", "mp3", "png"];

/// A scratch folder that cleans up after itself (and clears a leftover from a killed run).
pub(crate) struct Tmp(pub PathBuf);
impl Tmp {
    pub fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("se-linked-bins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    pub fn file(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    }
    pub fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 1000 files across 3 levels of nesting: Assets/{A,B,C}/{x,y}/{deep}/... plus files at every level.
fn thousand(t: &Tmp) {
    let mut n = 0;
    for a in ["A", "B", "C"] {
        for b in ["x", "y"] {
            for i in 0..80 {
                t.file(&format!("{a}/{b}/deep/clip{i}.mp4"));
                n += 1;
            }
            for i in 0..60 {
                t.file(&format!("{a}/{b}/take{i}.wav"));
                n += 1;
            }
        }
        for i in 0..20 {
            t.file(&format!("{a}/still{i}.png"));
            n += 1;
        }
    }
    while n < 1000 {
        t.file(&format!("top{n}.mp3"));
        n += 1;
    }
}

fn ready(s: State) -> Arc<Listing> {
    match s {
        State::Ready(l) => l,
        other => panic!("not listed: {other:?}"),
    }
}

/// The task's bar: linking a 1000-file, 3-deep folder returns in well under 100 ms, adds no assets, and
/// the folder's sub-folders come back as nested bins three levels down when they are opened.
#[test]
fn linking_1000_files_3_deep_is_instant_and_makes_no_assets() {
    let t = Tmp::new("thousand");
    thousand(&t);
    let mut p = Project::new();
    let opts = crate::engine::folder_import::Options::default();
    assert!(opts.linked, "linking is the default way a folder comes in");
    let t0 = Instant::now();
    let bin = p.link_bin(&t.path(), opts.bin_name.as_deref());
    let took = t0.elapsed();
    assert!(took.as_millis() < 100, "link took {took:?}");
    assert!(p.assets.is_empty(), "no asset until a file is used");
    assert_eq!(p.linked_bins.len(), 1);

    let cache = Cache::new(EXTS, |_| None);
    let top = ready(cache.listing_now(&p.linked_bin_dir(&bin).unwrap()));
    assert_eq!(top.dirs, ["A", "B", "C"]);
    let level2 = ready(cache.listing_now(&p.linked_bin_dir(&format!("{bin}/B")).unwrap()));
    assert_eq!(level2.dirs, ["x", "y"]);
    assert_eq!(level2.files.len(), 20);
    let level3 = ready(cache.listing_now(&p.linked_bin_dir(&format!("{bin}/B/y")).unwrap()));
    assert_eq!(level3.dirs, ["deep"]);
    let deep = ready(cache.listing_now(&p.linked_bin_dir(&format!("{bin}/B/y/deep")).unwrap()));
    assert_eq!(deep.files.len(), 80);
    // natural order: clip2 before clip10
    let names: Vec<&str> = deep.files.iter().map(|f| file_name(f)).collect();
    assert_eq!(&names[..3], ["clip0.mp4", "clip1.mp4", "clip2.mp4"]);
    assert!(p.assets.is_empty(), "listing does not import");
}

#[test]
fn listing_skips_hidden_and_unsupported_files() {
    let t = Tmp::new("filter");
    t.file("a.mp4");
    t.file("notes.txt");
    t.file(".hidden.mp4");
    t.file("Sub/b.wav");
    let l = list_dir(&t.0, EXTS).unwrap();
    assert_eq!(l.dirs, ["Sub"]);
    assert_eq!(l.files.len(), 1);
    assert!(l.files[0].ends_with("a.mp4"));
    assert!(list_dir(&t.0.join("nope"), EXTS).is_none());
}

/// Opening a bin reads it on a worker: the first look answers Pending, a later one has the files.
#[test]
fn expanding_a_bin_lists_its_files_on_a_worker() {
    let t = Tmp::new("expand");
    t.file("one.mp4");
    t.file("two.wav");
    let cache = Cache::new(EXTS, |_| None);
    assert_eq!(cache.listing(&t.0), State::Pending);
    let t0 = Instant::now();
    let l = loop {
        if let State::Ready(l) = cache.listing(&t.0) {
            break l;
        }
        assert!(t0.elapsed() < Duration::from_secs(5), "the listing never landed");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(l.files.len(), 2);
}

/// A file added to a listed folder shows up: the watcher sees the folder's modified time move.
#[test]
fn the_folder_watch_picks_up_an_added_file() {
    let t = Tmp::new("watch");
    t.file("one.mp4");
    let cache = Cache::new(EXTS, |_| None);
    assert_eq!(ready(cache.listing_now(&t.0)).files.len(), 1);
    assert!(!cache.poll(), "nothing changed yet");
    t.file("two.mp4");
    // the watcher thread (started by the listing) re-reads it within a couple of passes
    let t0 = Instant::now();
    while ready(cache.listing(&t.0)).files.len() != 2 {
        assert!(t0.elapsed() < WATCH_EVERY * 4, "the added file never showed up");
        std::thread::sleep(Duration::from_millis(20));
    }
    // and a removal, through a direct pass
    std::fs::remove_file(t.0.join("one.mp4")).unwrap();
    let t0 = Instant::now();
    while ready(cache.listing(&t.0)).files.len() != 1 {
        cache.poll();
        assert!(t0.elapsed() < Duration::from_secs(5), "the removed file never went");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_missing_root_reads_offline_and_comes_back() {
    let t = Tmp::new("offline");
    let gone = t.0.join("unplugged");
    let cache = Cache::new(EXTS, |_| None);
    let (here, there) = (t.path(), gone.to_string_lossy().into_owned());
    cache.set_roots([here.as_str(), there.as_str()].into_iter());
    assert_eq!(cache.online(&there), None, "unknown until checked");
    assert!(cache.poll());
    assert_eq!(cache.online(&there), Some(false));
    assert_eq!(cache.online(&here), Some(true));
    assert_eq!(cache.listing_now(&gone), State::Missing);
    std::fs::create_dir_all(&gone).unwrap();
    std::fs::write(gone.join("a.mp4"), b"x").unwrap();
    assert!(cache.poll());
    assert_eq!(cache.online(&there), Some(true));
    assert_eq!(ready(cache.listing_now(&gone)).files.len(), 1, "read again once it is back");
}

/// Durations are probed off the UI thread, once per file, and only when asked for.
#[test]
fn durations_are_probed_on_demand_once() {
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn fake(_: &str) -> Option<f64> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        Some(2.5)
    }
    let cache = Cache::new(EXTS, fake);
    assert_eq!(cache.duration("C:/x/a.mp4"), None, "queued, not blocking");
    let t0 = Instant::now();
    while cache.duration("C:/x/a.mp4").is_none() {
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(cache.duration("C:/x/a.mp4"), Some(Some(2.5)));
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

/// The project keeps only the folder and the bin, and reads them back.
#[test]
fn the_project_round_trips_the_linked_bin() {
    let mut p = Project::new();
    let bin = p.link_bin(r"C:\Users\x\Videos\Video Assets", None);
    let back = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(back.linked_bins, p.linked_bins);
    assert_eq!(back.linked_bins[0].bin, bin);
    assert!(back.folders.contains(&bin));
    assert!(back.assets.is_empty());
    assert!(p.to_json().contains("Video Assets"));
    // an old project file without the field still opens
    let old = p.to_json().replace("\"linked_bins\"", "\"_gone\"");
    assert!(Project::from_json(&old).unwrap().linked_bins.is_empty());
}

/// Using a file from a linked bin makes exactly one asset, filed under the sub-bin it came from.
#[test]
fn using_a_linked_file_makes_exactly_one_asset() {
    let t = Tmp::new("use");
    let f = t.file("SFX/Hits/boom.wav").to_string_lossy().into_owned();
    let mut p = Project::new();
    let bin = p.link_bin(&t.path(), Some("Assets"));
    let a = p.add_asset(crate::engine::import::placeholder(&f));
    let b = p.add_asset(crate::engine::import::placeholder(&f));
    assert_eq!(a, b);
    assert_eq!(p.assets.len(), 1);
    assert_eq!(p.assets[0].folder, format!("{bin}/SFX/Hits"));
}
