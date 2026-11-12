//! ---- ws:online-library ----
//! The repo's `library/` checked against its own rules. `scripts/library-index.ps1` runs these with
//! `SE_LIBRARY_WRITE=1` (rewrite `index.json`) and `SE_THUMBS=<folder>` (write LUT preview PNGs).

use super::*;

fn repo_library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("library")
}

/// Every file under `library/<kind dir>/`, as its catalogue entry; by kind (`Kind::ALL` order), then id.
fn build_index(lib: &Path) -> Catalogue {
    let mut items = Vec::new();
    for kind in Kind::ALL {
        for it in installed_in(lib, kind) {
            let bytes = std::fs::read(&it.path).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            items.push(entry_of(kind, &it.id, &text, bytes.len() as u64));
        }
    }
    Catalogue { version: 1, items }
}

/// What differs between two catalogues, one line per difference.
fn diff(want: &Catalogue, have: &Catalogue) -> Vec<String> {
    let mut out = Vec::new();
    for w in &want.items {
        match have.items.iter().find(|h| h.id == w.id) {
            None => out.push(format!("{}: missing from index.json", w.id)),
            Some(h) if h != w => out.push(format!("{}: index.json has {h:?}, the file gives {w:?}", w.id)),
            _ => {}
        }
    }
    for h in have.items.iter().filter(|h| !want.items.iter().any(|w| w.id == h.id)) {
        out.push(format!("{}: in index.json but there is no such file", h.id));
    }
    if out.is_empty() && want != have {
        out.push("same items, different order or version (rewrite it)".into());
    }
    out
}

#[test]
fn library_index_matches_files() {
    let lib = repo_library();
    let want = build_index(&lib);
    let path = lib.join("index.json");
    if std::env::var("SE_LIBRARY_WRITE").as_deref() == Ok("1") {
        let text = serde_json::to_string_pretty(&want).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
        eprintln!("wrote {} ({} items)", path.display(), want.items.len());
        return;
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing: run scripts/library-index.ps1", path.display()));
    let have: Catalogue = serde_json::from_str(&have).expect("index.json parses");
    let d = diff(&want, &have);
    assert!(d.is_empty(), "index.json is out of date (run scripts/library-index.ps1):\n{}", d.join("\n"));
    let empty: Vec<&str> = want.items.iter().filter(|e| e.name.is_empty()).map(|e| e.id.as_str()).collect();
    assert!(empty.is_empty(), "files with no name in their header: {empty:?}");
}

#[test]
fn library_luts_parse() {
    let mut bad = Vec::new();
    for it in installed_in(&repo_library(), Kind::Lut) {
        let text = std::fs::read_to_string(&it.path).unwrap_or_default();
        if let Err(e) = crate::engine::lut::parse_cube(&text) {
            bad.push(format!("{}: {e}", it.id));
        }
        for key in ["# @name ", "# @desc ", "# @tags "] {
            if !text.lines().any(|l| l.starts_with(key) && l.len() > key.len()) {
                bad.push(format!("{}: no `{}` line", it.id, key.trim()));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The LUT preview picture: a hue sweep (top half), a grey ramp, then skin / sky / foliage patches.
fn lut_test_picture(w: u32, h: u32) -> Vec<[f32; 3]> {
    const PATCHES: [[f32; 3]; 6] = [
        [0.87, 0.67, 0.55], // light skin
        [0.55, 0.38, 0.28], // dark skin
        [0.45, 0.65, 0.90], // sky
        [0.80, 0.88, 0.97], // pale sky
        [0.25, 0.45, 0.18], // foliage
        [0.55, 0.62, 0.25], // dry grass
    ];
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 / (w - 1).max(1) as f32;
            let fy = y as f32 / h as f32;
            px.push(if fy < 0.5 {
                let v = 1.0 - fy * 1.2; // brighter at the top
                                        // fully saturated hue fx: each channel is a clamped triangle wave
                let ch = |o: f32| ((((fx + o) * 6.0) % 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
                [ch(0.0) * v, ch(2.0 / 3.0) * v, ch(1.0 / 3.0) * v]
            } else if fy < 0.7 {
                [fx, fx, fx]
            } else {
                PATCHES[((fx * PATCHES.len() as f32) as usize).min(PATCHES.len() - 1)]
            });
        }
    }
    px
}

#[test]
#[ignore = "writes LUT previews; run by scripts/library-index.ps1 with SE_THUMBS set"]
fn library_lut_thumbs() {
    let Some(out) = std::env::var_os("SE_THUMBS").map(PathBuf::from) else {
        eprintln!("SE_THUMBS not set: nothing written");
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let (w, h) = (240u32, 135u32);
    let pic = lut_test_picture(w, h);
    for it in installed_in(&repo_library(), Kind::Lut) {
        let lut = crate::engine::lut::parse_cube(&std::fs::read_to_string(&it.path).unwrap()).unwrap();
        let mut f = crate::media::Frame::new(w, h);
        for (i, p) in pic.iter().enumerate() {
            let c = crate::engine::lut::sample_trilinear(&lut, *p);
            for k in 0..3 {
                f.rgba[i * 4 + k] = (c[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            f.rgba[i * 4 + 3] = 255;
        }
        std::fs::write(out.join(format!("luts-{}.png", it.id)), crate::mcp::png_encode(&f)).unwrap();
    }
}

#[test]
fn library_index_diff_names_each_difference() {
    let a = Catalogue { version: 1, items: vec![entry_of(Kind::Icon, "a", "<title>A</title>", 1)] };
    let mut b = a.clone();
    assert!(diff(&a, &b).is_empty());
    b.items[0].size = 2;
    b.items.push(entry_of(Kind::Icon, "b", "", 0));
    let d = diff(&a, &b);
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(d[0].starts_with("icons/a:") && d[1].starts_with("icons/b: in index.json"));
    assert_eq!(diff(&b, &a)[1], "icons/b: missing from index.json");
}
