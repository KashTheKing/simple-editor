//! Tests of the fast renderer client against the real `se-html` helper. They need the helper exe: set
//! `SE_HTML_HELPER` to it (CI builds it first: `scripts/build-helper.ps1`), or build it in its default place
//! (`tools/se-html/target/{release,debug}/se-html.exe`). Without it every test here prints a note and
//! passes, so a plain `cargo test` works on a machine that never built the helper.

use super::blitz::{self, Engine, Helper};
use super::html::{self, View};
use crate::model::HtmlRenderer;
use std::path::PathBuf;

fn helper() -> Option<PathBuf> {
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            if let Some(p) = std::env::var_os("SE_HTML_HELPER").map(PathBuf::from) {
                return p.is_file().then_some(p);
            }
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools").join("se-html").join("target");
            let p = ["release", "debug"].iter().map(|d| root.join(d).join("se-html.exe")).find(|p| p.is_file())?;
            std::env::set_var("SE_HTML_HELPER", &p);
            Some(p)
        })
        .clone()
}

macro_rules! need_helper {
    () => {
        if helper().is_none() {
            eprintln!("se-html helper not built: test skipped (set SE_HTML_HELPER)");
            return;
        }
    };
}

const PAGE: &str = r#"<html><head><style>
html,body{margin:0;background:transparent}
.a{position:absolute;left:10px;top:10px;width:60px;height:60px;background:#e33;animation:slide 2s linear infinite alternate}
@keyframes slide{from{left:10px}to{left:150px}}
.h{position:absolute;left:10px;top:100px;width:60px;height:40px;background:#3a3;transition:background .5s linear}
.h:hover{background:#33e}.h:active{background:#ee3}
a{position:absolute;left:100px;top:100px;display:block;width:80px;height:30px;background:#888}
</style></head><body><div class="a"></div><div class="h"></div><a href="https://x.test/p">next page</a>
<button style="position:absolute;left:10px;top:150px">Buy</button></body></html>"#;

fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    rgba[((y * w + x) * 4) as usize..][..4].try_into().unwrap()
}

#[test]
fn protocol_messages_round_trip_and_reject_garbage() {
    use blitz::proto::*;
    let mut buf = Vec::new();
    write_msg(&mut buf, &serde_json::json!({"cmd": "frame", "t": 1.5}), &[1, 2, 3]).unwrap();
    write_msg(&mut buf, &serde_json::json!({"cmd": "ping"}), &[]).unwrap();
    let mut r = std::io::Cursor::new(buf);
    let (h, b) = read_msg(&mut r).unwrap().unwrap();
    assert_eq!((h["cmd"].as_str(), h["t"].as_f64(), b), (Some("frame"), Some(1.5), vec![1, 2, 3]));
    assert_eq!(read_msg(&mut r).unwrap().unwrap().0["cmd"], "ping");
    assert!(read_msg(&mut r).unwrap().is_none(), "clean end of input");
    // a length that is not a message
    assert!(read_msg(&mut std::io::Cursor::new(vec![2, 0, 0, 0, 9, 9])).is_err());
    assert!(read_msg(&mut std::io::Cursor::new(vec![255, 255, 255, 255])).is_err());
}

#[test]
fn the_helper_loads_draws_and_answers_in_order() {
    need_helper!();
    let mut h = Helper::spawn().expect("spawn");
    let info = h.load(PAGE, "", (200, 200), 1.0).expect("load");
    assert!(info.animated && info.ends.is_none(), "an infinite animation: {info:?}");
    let a = h.frame(info.id, 0.0, &[], None, false).expect("frame");
    assert_eq!((a.w, a.h, a.bytes.len()), (200, 200, 200 * 200 * 4));
    assert_eq!(px(&a.bytes, 200, 5, 5), [0, 0, 0, 0], "transparent where nothing is drawn");
    assert_eq!(px(&a.bytes, 200, 30, 30)[3], 255);
    let png = h.frame(info.id, 0.0, &[], None, true).expect("png");
    assert_eq!(&png.bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(html::decode_png(&png.bytes).is_ok(), "the app's own PNG reader takes it");
    h.free(info.id);
    assert!(h.frame(info.id, 0.0, &[], None, false).is_err(), "a freed page is gone");
    h.close();
}

#[test]
fn a_frame_at_t_does_not_depend_on_what_was_drawn_before() {
    need_helper!();
    let mut a = Helper::spawn().unwrap();
    let mut b = Helper::spawn().unwrap();
    let (ia, ib) = (a.load(PAGE, "", (200, 200), 1.0).unwrap().id, b.load(PAGE, "", (200, 200), 1.0).unwrap().id);
    // a: in order, every 1/30 s; b: straight to the moment, then back and forth
    let mut last = Vec::new();
    for i in 0..=100 {
        last = a.frame(ia, i as f64 / 30.0, &[], None, false).unwrap().bytes;
    }
    let direct = b.frame(ib, 100.0 / 30.0, &[], None, false).unwrap().bytes;
    assert!(last == direct, "t = 3.33 s: stepping and jumping give the same picture");
    let back = b.frame(ib, 0.5, &[], None, false).unwrap().bytes;
    let fresh = Helper::spawn().and_then(|mut c| {
        let id = c.load(PAGE, "", (200, 200), 1.0)?.id;
        c.frame(id, 0.5, &[], None, false).map(|f| f.bytes)
    });
    assert!(back == fresh.unwrap(), "going back in time is the same as a fresh page at that time");
    // and the batch call is the single calls
    let items: Vec<_> = [0.1, 0.9, 0.3].iter().map(|&t| (t, Vec::new(), None)).collect();
    let batch = a.frames(ia, &items, false).unwrap();
    for (i, t) in [0.1, 0.9, 0.3].iter().enumerate() {
        let one = a.frame(ia, *t, &[], None, false).unwrap().bytes;
        assert!(batch[i].bytes == one, "frames[{i}] == frame({t})");
    }
}

#[test]
fn hover_and_pressed_states_and_the_element_map() {
    need_helper!();
    let mut h = Helper::spawn().unwrap();
    let id = h.load(PAGE, "", (200, 200), 1.0).unwrap().id;
    let at = |h: &mut Helper, t: f64, p: Option<(f64, f64, bool)>| h.frame(id, t, &[], p, false).unwrap().bytes;
    assert_eq!(px(&at(&mut h, 0.0, None), 200, 30, 120)[..3], [0x33, 0xaa, 0x33]);
    // pointer arrives at t = 0.5; the 0.5 s transition is half way at 0.75 and done at 1.2
    let _ = at(&mut h, 0.5, Some((30.0, 120.0, false)));
    let mid = at(&mut h, 0.75, Some((30.0, 120.0, false)));
    assert!(mid[(120 * 200 + 30) * 4 + 2] > 0x33 + 20 && mid[(120 * 200 + 30) * 4 + 2] < 0xee - 20, "half way blue");
    assert_eq!(px(&at(&mut h, 1.2, Some((30.0, 120.0, false))), 200, 30, 120)[..3], [0x33, 0x33, 0xee]);
    // button down: :active
    let down = at(&mut h, 1.3, Some((30.0, 120.0, true)));
    let later = at(&mut h, 2.0, Some((30.0, 120.0, true)));
    let _ = down;
    assert_eq!(px(&later, 200, 30, 120)[..3], [0xee, 0xee, 0x33]);
    let els = h.elements(id).unwrap();
    let link = els.iter().find(|e| e["tag"] == "a").expect("the link");
    assert_eq!((link["text"].as_str(), link["href"].as_str()), (Some("next page"), Some("https://x.test/p")));
    assert_eq!((link["x"].as_f64(), link["y"].as_f64(), link["w"].as_f64()), (Some(100.0), Some(100.0), Some(80.0)));
    assert!(els.iter().any(|e| e["tag"] == "button" && e["text"] == "Buy"));
    // a part of the page at 4x, still sharp: 4x the pixels
    let png = h.region(id, [100.0, 100.0, 80.0, 30.0], 4.0, 0.0).unwrap();
    assert_eq!(html::png_size(&png), Some((320, 120)));
    h.close();
}

/// A region far from the page's corner shows what is THERE (the browser layer's zoom details ask for
/// parts thousands of px down the page): blitz-paint ADDS its x/y offsets, so the helper must scroll
/// instead (found in the integration smoke test: zooms showed the page's top, shifted right and down).
#[test]
fn a_region_away_from_the_corner_shows_that_part_of_the_page() {
    need_helper!();
    let page = r#"<html><body style="margin:0;background:#fff;height:3000px">
<div style="position:absolute;left:300px;top:1400px;width:40px;height:40px;background:#f00"></div></body></html>"#;
    for scale in [1.0, 2.5] {
        let png = blitz::region_png(page, "", 800, [290.0, 1390.0, 60.0, 60.0], scale, 0.0).expect("region");
        let f = html::decode_png(&png).unwrap();
        let px = |x: u32, y: u32| {
            let i = ((y * f.width + x) * 4) as usize;
            [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]]
        };
        let (w, h) = (f.width, f.height);
        assert_eq!(px(w / 2, h / 2), [255, 0, 0], "scale {scale}: the red box is in the middle of the region");
        assert_eq!(px(2, 2), [255, 255, 255], "scale {scale}: white around it");
    }
}

#[test]
fn a_panic_in_the_renderer_is_an_error_not_a_crash_and_the_helper_lives_on() {
    need_helper!();
    let bad = r#"<html><body><div style="width:50px;height:50px;background:red;filter:hue-rotate(90deg)"></div></body></html>"#;
    let mut h = Helper::spawn().unwrap();
    let good = h.load(PAGE, "", (200, 200), 1.0).unwrap().id;
    let b = h.load(bad, "", (100, 100), 1.0).unwrap().id;
    let e = h.frame(b, 0.0, &[], None, false).err().expect("vello_cpu cannot do colour filters: an error");
    assert!(e.starts_with("panic: "), "{e}");
    assert!(h.frame(good, 0.0, &[], None, false).is_ok(), "other pages and the process are fine");
    assert!(h.frame(b, 0.0, &[], None, false).is_err(), "the page it died on is dropped");
    h.close();
}

#[test]
fn a_dead_or_hung_helper_is_reported_not_waited_for() {
    need_helper!();
    let mut h = Helper::spawn().unwrap();
    h.kill();
    let e = h.load(PAGE, "", (10, 10), 1.0).err().unwrap();
    assert!(e.contains("stopped"), "{e}");
}

#[test]
fn auto_falls_back_to_edge_for_a_page_the_helper_cannot_draw() {
    need_helper!();
    if html::edge_exe().is_none() {
        eprintln!("no Edge: test skipped");
        return;
    }
    let bad = r#"<html><head><title>t</title><meta name="se:size" content="100x100"></head><body style="margin:0"><div style="width:50px;height:50px;background:#f00;filter:hue-rotate(90deg)"></div></body></html>"#;
    // forced to the fast renderer: it panics inside, the layer is drawn by Edge, and Auto remembers
    let v = View { renderer: HtmlRenderer::Blitz, ..Default::default() };
    let f = html::render_frame_in(bad, &[], 1.0, 0.0, &v).expect("drawn by Edge after the fast renderer failed");
    assert_eq!((f.width, f.height), (100, 100));
    assert!(blitz::failure_of(bad).is_some());
    blitz::forget_failure(bad);
}

#[test]
fn nothing_is_downloaded_or_started_by_choosing() {
    // the library points nowhere: a decision must not need it
    let before = blitz::available();
    let c = blitz::choose(HtmlRenderer::Auto, "<p>x</p>");
    assert_eq!(c.engine == Engine::Blitz, before, "Blitz exactly when the helper is already there");
}

/// Every library template drawn by both renderers must agree within `LIMIT` (mean absolute difference per
/// RGB channel over the picture, composited on mid grey), or be one the lint sends to Edge anyway.
#[test]
fn every_library_template_looks_the_same_in_both_renderers() {
    need_helper!();
    if html::edge_exe().is_none() {
        eprintln!("no Edge: golden test skipped");
        return;
    }
    const LIMIT: f64 = 7.0;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("library").join("html");
    let mut report = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let src = std::fs::read_to_string(e.path()).unwrap();
        let name = e.path().file_stem().unwrap().to_string_lossy().into_owned();
        let found = crate::engine::html_lint::scan(&src);
        let tpl = html::parse(&src);
        let t = tpl.thumb_at();
        let edge = html::render_frame_in(&src, &[], 0.5, t, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).unwrap();
        let fast = html::render_frame_in(&src, &[], 0.5, t, &View { renderer: HtmlRenderer::Blitz, ..Default::default() }).unwrap();
        assert_eq!((edge.width, edge.height), (fast.width, fast.height), "{name}: size");
        let mut sum = 0.0;
        for i in (0..edge.rgba.len()).step_by(4) {
            for c in 0..3 {
                let ea = edge.rgba[i + 3] as f64 / 255.0;
                let fa = fast.rgba[i + 3] as f64 / 255.0;
                sum += ((edge.rgba[i + c] as f64 * ea + 128.0 * (1.0 - ea)) - (fast.rgba[i + c] as f64 * fa + 128.0 * (1.0 - fa))).abs();
            }
        }
        let diff = sum / (edge.rgba.len() / 4 * 3) as f64;
        report.push(format!("{name:22} diff {diff:5.2}  {}", if found.blocks.is_empty() { "fast renderer".to_string() } else { format!("Edge (lint: {})", found.blocks.join(", ")) }));
        // known, explained exceptions: a larger limit, and the reason (also the lint's note)
        let limit = match name.as_str() {
            "game-chat-bubble" => 12.0, // text-wrap: balance
            _ => LIMIT,
        };
        assert!(diff <= limit || !found.blocks.is_empty(), "{name}: the renderers disagree ({diff:.2} > {limit}) and the lint did not send it to Edge\n{}", found.blocks.join(", "));
    }
    eprintln!("{}", report.join("\n"));
}

/// Benchmarks against the Edge path (`cargo test bench_fast_vs_edge -- --ignored --nocapture`, with
/// `SE_HTML_HELPER` = the RELEASE helper). A 60 s layer at 30 and 60 fps through `render_to_file`, and the two
/// captured Roblox pages (`SE_BENCH_PAGES` = a folder holding `roblox-charts/pg/index.html` and
/// `roblox-forsaken/pg/index.html`, see BLITZ_REPORT.md).
#[test]
#[ignore]
fn bench_fast_vs_edge() {
    need_helper!();
    use std::time::Instant;
    let lib = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("library").join("html");
    let stamp = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let run = |name: &str, src: &str, base: &str, fps: f64, secs: f64, who: HtmlRenderer, width: u32| {
        let page = format!("{src}\n<!-- bench {} {name} {fps} {who:?} -->", stamp());
        let spec = html::Spec {
            html: page,
            fields: Vec::new(),
            scale: 1.0,
            fps,
            duration: secs,
            base: base.to_string(),
            params: Vec::new(),
            width,
            mouse: None,
            focus: None,
            renderer: who,
        };
        let t0 = Instant::now();
        let r = html::render_to_file(&spec, &crate::engine::export::Progress::new());
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let frames = (secs * fps).round().max(1.0);
        println!("{name:34} {who:?}: {ms:8.0} ms{}", if r.is_ok() { String::new() } else { format!("  FAILED {:?}", r.err()) });
        (ms, frames)
    };
    let ticker = std::fs::read_to_string(lib.join("news-ticker.html")).unwrap();
    for fps in [30.0, 60.0] {
        let (e, n) = run("news-ticker 60 s", &ticker, "", fps, 60.0, HtmlRenderer::Edge, 0);
        let (b, _) = run("news-ticker 60 s", &ticker, "", fps, 60.0, HtmlRenderer::Blitz, 0);
        println!("  -> {n} frames at {fps} fps: Edge {:.1} ms/frame, fast {:.1} ms/frame, {:.1}x\n", e / n, b / n, e / b);
    }
    if let Some(dir) = std::env::var_os("SE_BENCH_PAGES").map(PathBuf::from) {
        for page in ["roblox-charts", "roblox-forsaken"] {
            let folder = dir.join(page).join("pg");
            let src = std::fs::read_to_string(folder.join("index.html")).unwrap();
            let base = folder.display().to_string();
            let (e, _) = run(page, &src, &base, 30.0, 1.0, HtmlRenderer::Edge, 1920);
            let (b, _) = run(page, &src, &base, 30.0, 1.0, HtmlRenderer::Blitz, 1920);
            println!("  -> {page}: Edge {e:.0} ms, fast {b:.0} ms, {:.1}x\n", e / b);
        }
    }
}

#[test]
fn a_fractional_scale_lays_the_page_out_like_scale_one() {
    need_helper!();
    // text that wraps: at a hinted fractional scale the lines would break elsewhere
    let page = r#"<html><body style="margin:8px;background:#fff;font:15px Arial;width:220px">The quick brown fox jumps over the lazy dog and keeps on running through the long grass of the meadow until evening falls over the hills.</body></html>"#;
    let mut h = Helper::spawn().unwrap();
    let one = h.load(page, "", (240, 160), 1.0).unwrap();
    let half = h.load(page, "", (240, 160), 0.5).unwrap();
    let a = h.frame(one.id, 0.0, &[], None, false).unwrap();
    let b = h.frame(half.id, 0.0, &[], None, false).unwrap();
    assert_eq!((a.w, a.h, b.w, b.h), (240, 160, 120, 80));
    let mut sum = 0.0;
    for y in 0..80usize {
        for x in 0..120usize {
            for c in 0..3 {
                let s: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| a.bytes[((y * 2 + dy) * 240 + x * 2 + dx) * 4 + c] as u32).sum();
                sum += (s as f64 / 4.0 - b.bytes[(y * 120 + x) * 4 + c] as f64).abs();
            }
        }
    }
    let mean = sum / (120.0 * 80.0 * 3.0);
    assert!(mean < 6.0, "scale 0.5 is the scale-1 layout, drawn smaller: mean diff {mean:.2}");
    h.close();
}

/// Each `html_lint` rule's sample drawn by the real helper and by Edge: prints how far the helper's
/// picture is from Edge's, next to how far Edge's own picture moves when the property is taken out. A rule
/// whose effect the helper draws (blitz ~ edge, far below the effect size) is over-conservative.
/// `SE_HTML_HELPER=... cargo test lint_rules_against_the_real_helper -- --ignored --nocapture`
#[test]
#[ignore]
fn lint_rules_against_the_real_helper() {
    need_helper!();
    let box_ = "<div class=b>Ab</div>";
    let base_css = "html,body{margin:0;background:transparent}body{width:240px;height:120px;position:relative;background:linear-gradient(90deg,#123 0 50%,#fc3 50%)}.b{position:absolute;left:40px;top:20px;width:150px;height:80px;background:#e33;color:#fff;font:bold 48px Arial}";
    let rules: &[(&str, &str, &str)] = &[
        ("filter hue-rotate", ".b{filter:hue-rotate(120deg)}", box_),
        ("filter saturate+brightness", ".b{filter:saturate(.2) brightness(1.6)}", box_),
        ("filter grayscale", ".b{filter:grayscale(1)}", box_),
        ("filter invert", ".b{filter:invert(1)}", box_),
        ("filter opacity()", ".b{filter:opacity(.3)}", box_),
        ("backdrop-filter", ".b{background:rgba(255,255,255,.2);backdrop-filter:blur(8px)}", box_),
        ("mix-blend-mode", ".b{mix-blend-mode:multiply}", box_),
        ("background-blend-mode", ".b{background:linear-gradient(#0f0,#00f),#e33;background-blend-mode:multiply}", box_),
        ("-webkit-text-stroke", ".b{background:none;color:#fff;-webkit-text-stroke:4px #e0f}", box_),
        ("paint-order", ".b{background:none;-webkit-text-stroke:4px #e0f;paint-order:stroke fill}", box_),
        ("position sticky", ".b{position:sticky;top:30px;margin-left:40px}", box_),
        ("3D transform", "body{perspective:300px}.b{transform:rotateY(55deg)}", box_),
        ("counter()", "body{counter-reset:n 6}.b::before{counter-increment:n;content:counter(n)}", "<div class=b></div>"),
        ("@property", "@property --w{syntax:'<length>';inherits:false;initial-value:60px}.b{width:var(--w)}", box_),
        ("writing-mode", ".b{writing-mode:vertical-rl;font-size:30px}", box_),
        ("svg css fill (class)", ".p{fill:#2d2}", "<svg width=240 height=120 viewBox=\"0 0 240 120\"><rect class=p x=40 y=20 width=150 height=80 fill=#e33 /></svg>"),
        ("svg css stroke (tag)", "svg rect{stroke:#2d2;stroke-width:12px}", "<svg width=240 height=120 viewBox=\"0 0 240 120\"><rect x=40 y=20 width=150 height=80 fill=#e33 /></svg>"),
        ("svg var() attribute", ":root{--c:#2d2}", "<svg width=240 height=120 viewBox=\"0 0 240 120\"><rect x=40 y=20 width=150 height=80 fill=\"var(--c)\" /></svg>"),
        ("text-shadow", ".b{background:none;text-shadow:6px 6px 0 #0f0}", box_),
        ("ellipsis (note)", ".b{white-space:nowrap;overflow:hidden;text-overflow:ellipsis;width:90px}", "<div class=b>Abcdefgh</div>"),
        ("text-wrap balance (note)", ".b{font-size:20px;width:200px;text-wrap:balance}", "<div class=b>one two three four five six seven</div>"),
        ("justify (note)", ".b{font-size:16px;width:200px;text-align:justify}", "<div class=b>one two three four five six seven eight nine ten</div>"),
    ];
    let page = |css: &str, body: &str| {
        format!("<html><head><meta name=\"se:size\" content=\"240x120\"><style>{base_css}{css}</style></head><body>{body}</body></html>")
    };
    // the helper itself (no fallback to Edge on a failure), and Edge
    let draw = |html: &str, r: HtmlRenderer| match r {
        HtmlRenderer::Blitz => blitz::png_still(html, "", (240, 120), 1.0, 0.0, &[], None, false)
            .and_then(|p| html::decode_png(&p))
            .map(|f| f.rgba),
        _ => html::render_frame_in(html, &[], 1.0, 0.0, &View { renderer: r, ..Default::default() }).map(|f| f.rgba),
    };
    let diff = |a: &[u8], b: &[u8]| -> f64 {
        if a.len() != b.len() {
            return 255.0;
        }
        a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>() / a.len() as f64
    };
    println!("{:<30} {:>8} {:>8} {:>8}  verdict", "rule", "fx size", "b vs e", "b vs e0");
    for (name, css, body) in rules {
        let with = page(css, body);
        let without = page("", body);
        let (Ok(e1), Ok(e0), Ok(b1)) =
            (draw(&with, HtmlRenderer::Edge), draw(&without, HtmlRenderer::Edge), draw(&with, HtmlRenderer::Blitz))
        else {
            println!("{name:<30} render failed: blitz {:?}", draw(&with, HtmlRenderer::Blitz).err());
            continue;
        };
        let (fx, be, b0) = (diff(&e1, &e0), diff(&b1, &e1), diff(&b1, &e0));
        let verdict = if be < fx * 0.35 {
            "DRAWS IT"
        } else if b0 < be {
            "ignores it"
        } else {
            "partly"
        };
        println!("{name:<30} {fx:>8.2} {be:>8.2} {b0:>8.2}  {verdict}");
        let blocks = crate::engine::html_lint::scan(&with);
        println!("{:<30} lint: blocks {:?} notes {:?}", "", blocks.blocks, blocks.notes);
    }
    html::shutdown();
}

/// Regression: the Sale Badge came out black with the fast renderer (its starburst is an inline SVG coloured
/// through the page's CSS variable), and the Rating Stars' fill must match Edge. Both library templates, drawn
/// by the real helper, against Edge.
#[test]
fn library_svg_templates_keep_their_colours() {
    need_helper!();
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join("html");
    let read = |n: &str| std::fs::read_to_string(lib.join(n)).unwrap().replace("\r\n", "\n");
    let badge = read("sale-badge.html");
    // the version installed by earlier libraries: the colour as var() in a presentation attribute
    let old_badge = badge.replace("fill=\"{{accent}}\"", "fill=\"var(--accent)\"");
    let stars = read("rating-stars.html");
    for src in [&badge, &old_badge, &stars] {
        assert_eq!(blitz::choose_with(HtmlRenderer::Auto, src, true).engine, Engine::Blitz, "Auto draws it fast");
    }
    let fast = |src: &str, t: f64| {
        let (tpl, page) = html::prepare_in(src, &[], "");
        html::decode_png(&blitz::png_still(&page, "", tpl.size, 1.0, t, &[], None, false).expect("drawn by the helper"))
            .unwrap()
    };
    let red = |f: &crate::media::Frame| {
        f.rgba.chunks(4).filter(|p| p[3] > 200 && p[0] > 200 && p[1] < 90 && p[2] < 90).count()
    };
    for (name, src) in [("current", &badge), ("old", &old_badge)] {
        let f = fast(src, 3.9);
        let n = red(&f);
        assert!(
            n * 10 > (f.width * f.height) as usize,
            "{name} Sale Badge: red on {n} of {} pixels",
            f.width * f.height
        );
    }
    if html::edge_exe().is_none() {
        eprintln!("no Edge: the comparison with Edge is skipped");
        return;
    }
    // the stars, filled, against Edge's picture: the accent pixels and their mean colour
    let edge =
        html::render_frame_in(&stars, &[], 1.0, 3.9, &View { renderer: HtmlRenderer::Edge, ..Default::default() })
            .unwrap();
    let blitz_f = fast(&stars, 3.9);
    let accent = |f: &crate::media::Frame| {
        let px: Vec<&[u8]> =
            f.rgba.chunks(4).filter(|p| p[3] > 240 && p[0] > 200 && p[1] > 140 && p[1] < 220 && p[2] < 80).collect();
        let mean = |i: usize| px.iter().map(|p| p[i] as f64).sum::<f64>() / px.len().max(1) as f64;
        (px.len(), [mean(0), mean(1), mean(2)])
    };
    let ((ne, ce), (nb, cb)) = (accent(&edge), accent(&blitz_f));
    assert!(ne > 1000, "Edge drew the stars ({ne})");
    assert!((nb as f64) > ne as f64 * 0.8 && (nb as f64) < ne as f64 * 1.25, "star area: fast {nb} vs Edge {ne}");
    for i in 0..3 {
        assert!((ce[i] - cb[i]).abs() < 6.0, "star colour: fast {cb:?} vs Edge {ce:?}");
    }
    html::shutdown();
}
