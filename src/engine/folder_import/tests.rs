use super::*;

const EXTS: &[&str] = &["mp4", "wav", "mp3", "png", "jpg"];

/// A scratch folder that cleans up after itself (and clears a leftover from a killed run).
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("se-folder-import-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn file(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(b: &Bin) -> Vec<String> {
    b.files.iter().map(|f| name_of(f)).collect()
}

fn bin<'a>(s: &'a Scan, name: &str) -> &'a Bin {
    s.bins.iter().find(|b| b.name == name).unwrap_or_else(|| panic!("no bin {name}: {:?}", s.bins))
}

#[test]
fn nested_folders_become_nested_bins_named_after_the_folder() {
    let t = Tmp::new("nested");
    t.file("SFX/hit.wav");
    t.file("SFX/Impacts/boom.wav");
    t.file("SFX/Impacts/Big/crash.mp3");
    t.file("SFX/Whooshes/zip.wav");
    let s = scan(&t.0.join("SFX"), &Options::default(), EXTS);
    let order: Vec<&str> = s.bins.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(order, ["SFX", "SFX/Impacts", "SFX/Impacts/Big", "SFX/Whooshes"], "parents first, siblings in name order");
    assert_eq!(names(bin(&s, "SFX/Impacts/Big")), ["crash.mp3"]);
    assert_eq!(s.file_count(), 4);
    assert_eq!(s.top(), "SFX");
    assert_eq!(s.skipped, 0);
}

#[test]
fn bin_name_and_recursive_and_extensions_options() {
    let t = Tmp::new("opts");
    t.file("in/a.wav");
    t.file("in/b.mp3");
    t.file("in/deep/c.wav");
    let o = Options::from_args(&serde_json::json!({"bin_name": " My / Pack ", "recursive": false, "extensions": [".WAV", "*mp4"]}));
    assert!(!o.recursive);
    assert_eq!(o.extensions, ["wav", "mp4"]);
    let s = scan(&t.0.join("in"), &o, EXTS);
    assert_eq!(s.bins.len(), 1, "non-recursive: no sub-bins");
    assert_eq!(s.bins[0].name, "My - Pack", "separators cannot nest a name");
    assert_eq!(names(&s.bins[0]), ["a.wav"], "the filter drops b.mp3");
    assert_eq!(s.skipped, 1, "and counts it");
}

#[test]
fn unsupported_hidden_and_empty_are_skipped() {
    let t = Tmp::new("skip");
    t.file("lib/a.wav");
    t.file("lib/notes.txt");
    t.file("lib/cover.psd");
    t.file("lib/.hidden.wav");
    t.file("lib/.git/x.wav");
    t.file("lib/Empty/readme.txt");
    t.file("lib/Sys/b.wav");
    // a Windows-hidden folder and a hidden-attribute file (no dot in their names)
    t.file("lib/Secret/c.wav");
    t.file("lib/h.wav");
    for p in ["lib/Secret", "lib/h.wav"] {
        let st = std::process::Command::new("attrib").arg("+h").arg(t.0.join(p)).status().unwrap();
        assert!(st.success());
    }
    let s = scan(&t.0.join("lib"), &Options::default(), EXTS);
    let order: Vec<&str> = s.bins.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(order, ["lib", "lib/Sys"], "Empty has nothing importable, Secret is hidden");
    assert_eq!(names(&s.bins[0]), ["a.wav"]);
    // notes.txt, cover.psd and Empty's readme.txt; hidden things are not counted
    assert_eq!(s.skipped, 3);
}

#[test]
fn files_sort_naturally() {
    let t = Tmp::new("sort");
    for n in ["take10.wav", "Take2.wav", "take1.wav", "take02.wav", "b.wav", "a10b.wav", "a2b.wav"] {
        t.file(&format!("s/{n}"));
    }
    t.file("s/10 x/a.wav");
    t.file("s/9 x/a.wav");
    let s = scan(&t.0.join("s"), &Options::default(), EXTS);
    assert_eq!(names(&s.bins[0]), ["a2b.wav", "a10b.wav", "b.wav", "take1.wav", "Take2.wav", "take02.wav", "take10.wav"]);
    let order: Vec<&str> = s.bins.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(order, ["s", "s/9 x", "s/10 x"], "folders sort the same way");
    assert_eq!(natural_cmp("a", "A").reverse(), natural_cmp("A", "a"));
    assert_eq!(natural_cmp("x007", "x7"), "x007".cmp("x7"), "numerically equal falls back to plain order");
}

#[test]
fn depth_is_capped() {
    let t = Tmp::new("depth");
    let mut rel = String::from("top");
    t.file("top/a.wav");
    for i in 1..=MAX_DEPTH + 2 {
        rel = format!("{rel}/d{i}");
        t.file(&format!("{rel}/f{i}.wav"));
    }
    let s = scan(&t.0.join("top"), &Options::default(), EXTS);
    assert_eq!(s.bins.len(), 1 + MAX_DEPTH, "the top bin and MAX_DEPTH levels below it");
    assert_eq!(s.skipped, 1, "the folder past the cap is counted once");
    assert!(s.bins.iter().all(|b| !b.name.contains(&format!("d{}", MAX_DEPTH + 1))));
}

#[test]
fn non_ascii_names_and_long_paths_work() {
    let t = Tmp::new("uni");
    t.file("Sonidos/Ünïcødé/猫.wav");
    t.file("Sonidos/日本語 フォルダ/ça va.mp3");
    // past MAX_PATH (260): std switches to the \\?\ form by itself
    let mut long = String::from("Sonidos");
    for i in 0..5 {
        long.push_str(&format!("/segmento_{i}_muy_largo_de_ruta_para_pasar_de_doscientos_sesenta_caracteres"));
    }
    t.file(&format!("{long}/final.wav"));
    let s = scan(&t.0.join("Sonidos"), &Options::default(), EXTS);
    assert_eq!(s.file_count(), 3, "{:?}", s.bins);
    assert_eq!(names(bin(&s, "Sonidos/Ünïcødé")), ["猫.wav"]);
    assert_eq!(names(bin(&s, "Sonidos/日本語 フォルダ")), ["ça va.mp3"]);
    let deepest = s.bins.iter().find(|b| b.files.iter().any(|f| name_of(f) == "final.wav")).unwrap();
    assert!(deepest.files[0].to_string_lossy().len() > 260);
}

#[test]
fn apply_files_assets_under_their_bins_and_reuses_duplicates() {
    let t = Tmp::new("apply");
    t.file("SFX/a.wav");
    t.file("SFX/Sub/b.wav");
    let s = scan(&t.0.join("SFX"), &Options::default(), EXTS);
    let mut p = Project::new();
    // one of the files is already in the library, under another bin and with a different case
    let known = placeholder(&t.0.join("SFX").join("Sub").join("B.WAV").to_string_lossy());
    let known = p.add_asset(known);
    p.asset_mut(known).unwrap().folder = "Mine".into();

    let done = apply(&mut p, &s);
    assert_eq!(done.fresh.len(), 1);
    assert_eq!(done.reused, 1);
    assert_eq!(p.assets.len(), 2, "no second copy of the known file");
    assert_eq!(p.asset(known).unwrap().folder, "Mine", "a reused asset stays where the user put it");
    let a = p.asset(done.fresh[0].0).unwrap();
    assert_eq!((a.name().as_str(), a.folder.as_str()), ("a.wav", "SFX"));
    assert_eq!(p.folder_names(), ["Mine", "SFX", "SFX/Sub"]);

    // importing the same folder again adds nothing
    let again = apply(&mut p, &s);
    assert_eq!((again.fresh.len(), again.reused, p.assets.len()), (0, 2, 2));
}

#[test]
fn one_snapshot_undoes_the_whole_import() {
    let t = Tmp::new("undo");
    t.file("Pack/a.wav");
    t.file("Pack/Sub/b.wav");
    let mut p = Project::new();
    let before = p.to_json();
    apply(&mut p, &scan(&t.0.join("Pack"), &Options::default(), EXTS));
    assert_ne!(p.to_json(), before);
    // the app pushes ONE labelled undo snapshot taken before `apply` - restoring it undoes the lot
    let back = Project::from_json(&before).unwrap();
    assert!(back.assets.is_empty() && back.folder_names().is_empty());
    // and the app code really does push exactly once, with the snapshot taken before the mutation
    let src = include_str!("../../ui/app/folder_import.rs");
    let body = &src[src.find("fn apply_scan").expect("apply_scan exists")..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let (snap, applied, push) =
        (body.find("to_json()").unwrap(), body.find("fi::apply(").unwrap(), body.find("push_undo_labeled").unwrap());
    assert!(snap < applied && applied < push, "snapshot, then apply, then the one undo entry");
    assert_eq!(body.matches("push_undo").count(), 1);
}

#[test]
fn numbered_still_runs_leave_the_file_list_and_fold_back_without_ffmpeg() {
    let t = Tmp::new("runs");
    for i in 1..=4 {
        t.file(&format!("Shots/frame_{i:04}.png"));
    }
    t.file("Shots/logo.png");
    t.file("Shots/a.jpg");
    let mut s = scan(&t.0.join("Shots"), &Options::default(), EXTS);
    assert_eq!(s.runs.len(), 1);
    assert_eq!((s.runs[0].0.as_str(), s.runs[0].1.len()), ("Shots", 4));
    assert_eq!(names(&s.bins[0]), ["a.jpg", "logo.png"], "the four frames are one clip, not four stills");
    assert_eq!(s.file_count(), 6);
    s.fold_runs();
    assert!(s.runs.is_empty());
    assert_eq!(s.bins[0].files.len(), 6);
    assert_eq!(names(&s.bins[0])[1], "frame_0001.png", "a.jpg, then the frames in order, then logo.png");
}

#[test]
fn a_folder_with_nothing_importable_scans_to_nothing() {
    let t = Tmp::new("none");
    t.file("Docs/readme.txt");
    let s = scan(&t.0.join("Docs"), &Options::default(), EXTS);
    assert!(s.bins.is_empty() && s.file_count() == 0);
    assert_eq!(s.skipped, 1);
    // an unreadable path is one skipped folder, not a panic
    let s = scan(&t.0.join("missing"), &Options::default(), EXTS);
    assert!(s.bins.is_empty() && s.skipped == 1);
    assert_eq!(Options::from_args(&serde_json::json!({})).bin_name, None);
    assert!(Options::from_args(&serde_json::json!({})).linked, "a folder links by default");
    assert!(!Options::from_args(&serde_json::json!({"linked": false})).linked);
}
