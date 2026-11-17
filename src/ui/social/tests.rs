use super::device::{self, Hit, Sheet};
use super::paint::{self, GuideView};
use super::profile::{self, SocialProfile};
use super::scene::{self, Op};
use super::*;
use crate::settings::Settings;
use crate::media::Frame;
use eframe::egui::Color32;

fn settings_with(guide: &str, mode: &str, device: &str) -> Settings {
    Settings { guide: Some(guide.into()), guide_mode: mode.into(), guide_device: device.into(), ..Settings::default() }
}

#[test]
fn embedded_guides_parse_and_validate() {
    let d = parse(EMBEDDED).expect("embedded guides.json");
    assert!(d.platforms.len() >= 8 && d.devices.len() >= 5);
    for p in &d.platforms {
        assert!(!p.src.is_empty() && p.ver.len() == 10, "{} says where and when it was measured", p.id);
        assert!(matches!(p.pg.as_str(), "full" | "feed" | "watch"), "{} page shape", p.id);
        assert!(["youtube", "tiktok", "instagram", "facebook", "snapchat", "x"].contains(&p.net.as_str()));
        let phone = p.lay.get("phone").unwrap_or_else(|| panic!("{} has a phone layout", p.id));
        assert!(phone.zones.iter().any(|z| z.k == "s"), "{} has a safe zone", p.id);
        assert!(phone.safe.is_some());
    }
    for id in ["yt_shorts", "yt_long", "tiktok", "ig_reels", "ig_feed", "fb_reels", "snap_spotlight", "x_video"] {
        assert!(d.platform(id).is_some(), "{id} is in the data");
    }
}

/// Every rectangle is a valid fraction of its reference frame (the ones that sit outside the video, like a
/// desktop action column, may leave it on purpose).
#[test]
fn rect_fractions_stay_inside_the_frame() {
    let d = data();
    for p in &d.platforms {
        for (name, l) in &p.lay {
            let f = |r: &[f32; 4]| [r[0] / l.refsz[0], r[1] / l.refsz[1], r[2] / l.refsz[0], r[3] / l.refsz[1]];
            for z in &l.zones {
                let r = f(&z.r);
                assert!(r.iter().all(|v| (-0.001..=1.001).contains(v)), "{}/{name} zone {} = {r:?}", p.id, z.l);
                assert!(r[0] < r[2] && r[1] < r[3], "{}/{name} zone {} is inverted", p.id, z.l);
            }
            for e in l.ui.iter().filter(|e| e.o == 0) {
                let r = f(&e.r);
                assert!(r.iter().all(|v| (-0.001..=1.001).contains(v)), "{}/{name} ui {} {} = {r:?}", p.id, e.t, e.i);
            }
            // painter's order: the safe outline is drawn after the danger fills it sits over
            let first_safe = l.zones.iter().position(|z| z.k == "s");
            if let Some(i) = first_safe {
                assert!(l.zones[i..].iter().all(|z| z.k == "s"), "{}/{name}: safe zones come last", p.id);
            }
            let s = f(&l.safe.unwrap_or([0.0, 0.0, l.refsz[0], l.refsz[1]]));
            assert!(s[0] >= 0.0 && s[1] >= 0.0 && s[2] <= 1.0 && s[3] <= 1.0, "{}/{name} safe rect", p.id);
        }
    }
}

#[test]
fn a_bad_user_file_is_ignored_and_a_good_one_replaces_by_id() {
    assert!(parse("{").is_err());
    let bad = EMBEDDED.replacen("\"r\":[", "\"r\":[9999,", 1);
    assert!(parse(&bad).is_err() || merged(Some(&bad)).platforms.len() == merged(None).platforms.len());
    let base = merged(None);
    let mut user: serde_json::Value = serde_json::from_str(EMBEDDED).unwrap();
    user["verified"] = "2030-01-01".into();
    user["platforms"].as_array_mut().unwrap().retain(|p| p["id"] == "tiktok");
    user["platforms"][0]["n"] = "TikTok (updated)".into();
    let m = merged(Some(&user.to_string()));
    assert_eq!(m.platforms.len(), base.platforms.len(), "same ids are replaced, none dropped");
    assert_eq!(m.platform("tiktok").unwrap().n, "TikTok (updated)");
    assert_eq!(m.verified, "2030-01-01");
    assert_eq!(merged(Some("not json")).platforms.len(), base.platforms.len(), "garbage keeps the built-ins");
}

#[test]
fn map_sticks_edges_and_keeps_centres() {
    let m = Map::new([360.0, 640.0], vec2(393.0, 852.0), pos2(0.0, 0.0), 1.0);
    // right-edge column: 32 dp wide, 16 from the right edge on the 360 wide reference
    let col = m.dp([312.0, 389.0, 344.0, 449.0]);
    assert_eq!(col[0], 345.0, "stays 16 dp from the right edge: 393 - 48");
    assert_eq!(col[3] - col[1], 60.0, "keeps its height");
    assert_eq!(col[1], 389.0 + 212.0, "bottom half sticks to the bottom edge");
    // a top bar stays on top
    assert_eq!(m.dp([0.0, 0.0, 360.0, 48.0])[1], 0.0);
    assert_eq!(m.dp([0.0, 0.0, 360.0, 48.0])[2], 393.0, "full width stays full width");
    // a centred element stays centred
    let t = m.dp([140.0, 20.0, 220.0, 40.0]);
    assert_eq!((t[0] + t[2]) / 2.0, 393.0 / 2.0);
    // identity on the reference size
    let id = Map::fill([360.0, 640.0], Rect::from_min_size(pos2(10.0, 20.0), vec2(180.0, 320.0)));
    let r = id.rect([0.0, 0.0, 360.0, 640.0]);
    assert_eq!(r, Rect::from_min_size(pos2(10.0, 20.0), vec2(180.0, 320.0)));
}

#[test]
fn caption_wraps_and_truncates_with_more() {
    let text = "I tried the hardest challenge on the internet and you will not believe how it ended #gaming #challenge #fyp";
    let (s, w) = (14.0, 200.0);
    let lines = scene::wrap(text, s, false, w);
    assert!(lines.len() >= 3, "{lines:?}");
    assert!(lines.iter().all(|l| scene::text_w(l, s, false) <= w + 0.01), "every line fits: {lines:?}");
    assert_eq!(lines.join(" "), text, "wrapping loses no words");
    let tail = scene::text_w("… more", s, false);
    let (cut, was) = scene::collapse(lines.clone(), 2, s, false, w, tail);
    assert!(was && cut.len() == 2);
    assert!(scene::text_w(&cut[1], s, false) + tail <= w + 0.01, "room left for '… more'");
    let (same, was) = scene::collapse(lines.clone(), 9, s, false, w, tail);
    assert!(!was && same == lines);
    // one unbreakable word is cut, never overflows
    let long = scene::wrap("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", s, false, 80.0);
    assert!(long.len() > 1 && long.iter().all(|l| scene::text_w(l, s, false) <= 80.01));
    assert_eq!(scene::ellipsize("short", s, false, 200.0), "short");
    let e = scene::ellipsize("a very long channel name that cannot fit", s, false, 90.0);
    assert!(e.ends_with('…') && scene::text_w(&e, s, false) <= 90.0);
}

#[test]
fn profile_round_trips_and_old_settings_load() {
    let mut s = Settings::default();
    let mut p = SocialProfile::sample("tiktok");
    p.id = "p1".into();
    p.handle = "kashtheking".into();
    p.avatar = "iVBORw0KGgo=".into();
    p.comment_list.push(SampleComment2::new());
    s.social_profiles.push(p.clone());
    s.social_active.insert("tiktok".into(), "p1".into());
    let json = serde_json::to_string(&s).unwrap();
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.social_profiles, vec![p]);
    assert_eq!(back.active_profile("tiktok").unwrap().handle, "kashtheking");
    assert!(back.active_profile("youtube").is_none());
    // settings.json from before this feature: no social keys, and the old enum names for `guide`
    let old: Settings = serde_json::from_str(r#"{"guide":"IgReels","guides_keep_aspect":false}"#).unwrap();
    assert_eq!(old.guide.as_deref(), Some("ig_reels"));
    assert!(old.social_profiles.is_empty() && old.guide_mode == "safe" && !old.guide_render);
    for (a, b) in [("TikTok", "tiktok"), ("YtShorts", "yt_shorts"), ("IgPost", "ig_feed"), ("YouTube", "yt_long")] {
        let s: Settings = serde_json::from_str(&format!(r#"{{"guide":"{a}"}}"#)).unwrap();
        assert_eq!(s.guide.as_deref(), Some(b));
        assert!(data().platform(b).is_some());
    }
    let none: Settings = serde_json::from_str(r#"{"guide":null}"#).unwrap();
    assert!(none.guide.is_none());
}

struct SampleComment2;
impl SampleComment2 {
    fn new() -> profile::SampleComment {
        profile::SampleComment { name: "x".into(), text: "hello".into(), likes: "1".into(), when: "1m".into() }
    }
}

#[test]
fn fetch_input_and_meta_parsing() {
    assert_eq!(profile::profile_url("youtube", "@YouTube").unwrap(), "https://www.youtube.com/@YouTube");
    assert_eq!(profile::profile_url("instagram", "nasa").unwrap(), "https://www.instagram.com/nasa/");
    assert_eq!(profile::profile_url("tiktok", "https://www.tiktok.com/@tiktok").unwrap(), "https://www.tiktok.com/@tiktok");
    assert!(profile::profile_url("youtube", "file:///c:/secret").is_err());
    assert!(profile::profile_url("youtube", "javascript:alert(1)").is_err());
    assert!(profile::profile_url("youtube", "a b").is_err());
    assert!(profile::profile_url("youtube", "  ").is_err());
    let html = r#"<html><head><title>Fallback</title><meta property="og:title" content="NASA (@nasa) &bull; Instagram photos &amp; videos">
        <META name='description' content='97M Followers, 100 Following'><meta property="og:image" content="https://cdn.example/a.jpg?x=1&amp;y=2"></head></html>"#;
    let (t, d, i) = profile::parse_meta(html);
    assert!(t.starts_with("NASA (@nasa)"), "{t}");
    assert_eq!(d, "97M Followers, 100 Following");
    assert_eq!(i, "https://cdn.example/a.jpg?x=1&y=2");
    assert_eq!(profile::display_from_title("NASA (@nasa) • Instagram photos and videos"), "NASA");
    assert_eq!(profile::display_from_title("YouTube"), "YouTube");
    assert_eq!(profile::display_from_title("TikTok (@tiktok) Official | TikTok"), "TikTok");
    assert_eq!(profile::followers_from("@tiktok on TikTok | 1.2B Likes. 80.5M Followers. Hello"), "80.5M");
    assert_eq!(profile::followers_from("12.4K subscribers"), "12.4K");
    assert_eq!(profile::followers_from("no counts here"), "");
    let (t, _, _) = profile::parse_meta("<title> Only title </title>");
    assert_eq!(t, "Only title");
}

/// Nothing in this feature touches the network until a fetch is asked for: drawing every platform in
/// every mode with a profile whose link is set, and the Settings tab, start no curl.
#[test]
fn no_network_until_a_fetch_is_asked_for() {
    let before = profile::NET_CALLS.load(std::sync::atomic::Ordering::SeqCst);
    let d = data();
    let mut s = Settings::default();
    let mut p = SocialProfile::sample("youtube");
    p.link = "https://www.youtube.com/@YouTube".into();
    s.social_profiles.push(p);
    for plat in &d.platforms {
        for mode in ["safe", "ui", "device"] {
            let st = settings_with(&plat.id, mode, "iphone");
            let g = GuideView::new(&st, "t", 9.0 / 16.0, 1.0, 10.0).unwrap();
            let _ = g.overlay_ops(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 533.0)));
            let _ = device::build(&g.device_view(Some(Sheet::Comments), 1.0), Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 900.0)));
        }
    }
    assert_eq!(profile::NET_CALLS.load(std::sync::atomic::Ordering::SeqCst), before, "no request was started");
    assert!(profile::fetch("youtube", "file:///etc/passwd").is_err(), "refused before any request");
    assert_eq!(profile::NET_CALLS.load(std::sync::atomic::Ordering::SeqCst), before, "a refused input starts no curl either");
}

#[test]
fn every_platform_builds_in_every_mode_and_device() {
    let d = data();
    for plat in &d.platforms {
        for dev in &d.devices {
            for mode in ["safe", "ui", "device"] {
                let st = settings_with(&plat.id, mode, &dev.id);
                let g = GuideView::new(&st, "My video", 9.0 / 16.0, 3.0, 10.0).unwrap();
                let ops = g.overlay_ops(Rect::from_min_size(pos2(5.0, 5.0), vec2(400.0, 711.0)));
                assert!(!ops.is_empty());
                for open in [None, Some(Sheet::Comments), Some(Sheet::Desc), Some(Sheet::Share), Some(Sheet::More), Some(Sheet::Profile), Some(Sheet::Sound)] {
                    let sc = device::build(&g.device_view(open, 1.0), Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 900.0)));
                    assert!(sc.video.width() > 10.0 && sc.video.height() > 10.0, "{} on {} has a video", plat.id, dev.id);
                    assert!(sc.video.is_finite() && sc.uv.is_finite());
                    assert!(!sc.over.is_empty() && !sc.under.is_empty());
                }
            }
        }
    }
}

/// The device preview's state machine: a tap on Comments opens it, another tap on it (or the X, the
/// handle, the dimmed video) closes it, a different silhouette swaps the sheet.
#[test]
fn sheet_taps_open_and_close() {
    let st = settings_with("tiktok", "device", "iphone");
    let g = GuideView::new(&st, "t", 9.0 / 16.0, 1.0, 10.0).unwrap();
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 900.0));
    let mut s = paint::State::default();
    let closed = device::build(&g.device_view(None, 0.0), area);
    let tap = |sc: &device::Scene, which: Sheet| sc.hits.iter().rev().find(|(_, h)| *h == Hit::Open(which)).map(|(r, _)| r.center()).expect("tap target");
    s.click(&closed, tap(&closed, Sheet::Comments), ViewMode::Default);
    assert_eq!(s.want, Some(Sheet::Comments));
    let open = device::build(&g.device_view(Some(Sheet::Comments), 1.0), area);
    let x = open.hits.iter().rev().find(|(_, h)| *h == Hit::Close).expect("close target").0.center();
    s.click(&open, x, ViewMode::Default);
    assert_eq!(s.want, None, "the X closes it");
    s.want = Some(Sheet::Comments);
    s.click(&closed, tap(&closed, Sheet::Share), ViewMode::Default);
    assert_eq!(s.want, Some(Sheet::Share), "another silhouette swaps the sheet");
    s.click(&closed, pos2(-50.0, -50.0), ViewMode::Default);
    assert_eq!(s.want, Some(Sheet::Share), "a tap on nothing changes nothing");
}

/// Shorts-style apps shrink the video into the part above the sheet, "cover" apps leave it and dim it.
#[test]
fn comments_move_the_video_the_way_each_platform_does() {
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 900.0));
    let rects = |id: &str| {
        let st = settings_with(id, "device", "iphone");
        let g = GuideView::new(&st, "t", 9.0 / 16.0, 1.0, 10.0).unwrap();
        let a = device::build(&g.device_view(None, 0.0), area);
        let b = device::build(&g.device_view(Some(Sheet::Comments), 1.0), area);
        let mid = device::build(&g.device_view(Some(Sheet::Comments), 0.5), area);
        let sheet = data().platform(id).unwrap().st.comments.sh;
        (a, b, mid, sheet)
    };
    let (a, b, mid, sh) = rects("tiktok");
    assert!(b.video.height() < a.video.height() * 0.6, "TikTok shrinks the video: {:?} -> {:?}", a.video, b.video);
    assert!(b.video.max.y <= a.base.min.y + a.base.height() * sh + 1.0, "...into the part above the sheet");
    assert!(mid.video.height() < a.video.height() && mid.video.height() > b.video.height(), "and slides there");
    assert_eq!(a.base, b.base, "the render size never changes while it animates");
    let (a, b, _, _) = rects("ig_feed");
    assert_eq!(a.video, b.video, "a feed post stays put");
    let (a, b, _, _) = rects("yt_long");
    assert_eq!(a.video, b.video, "the long-form player stays put and the page scrolls under it");
}

#[test]
fn cover_platforms_crop_and_contain_platforms_bar() {
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 900.0));
    let go = |id: &str, dev: &str| {
        let st = settings_with(id, "device", dev);
        let g = GuideView::new(&st, "t", 9.0 / 16.0, 1.0, 10.0).unwrap();
        device::build(&g.device_view(None, 0.0), area)
    };
    let tt = go("tiktok", "iphone");
    assert!(tt.uv.width() < 1.0 && tt.uv.height() == 1.0, "9:16 on a 19.5:9 screen fills it and crops the sides: {:?}", tt.uv);
    let yt = go("yt_shorts", "iphone");
    assert_eq!(yt.uv, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    assert!(yt.video.width() < yt.video.height() * 0.6, "contain: bars above and below");
    let small = go("tiktok", "small");
    assert!(small.uv.width() > 0.99, "a 16:9 phone is exactly 9:16: no crop");
    // a landscape project in a vertical app is letterboxed, never cropped
    let st = settings_with("tiktok", "device", "iphone");
    let g = GuideView::new(&st, "t", 16.0 / 9.0, 1.0, 10.0).unwrap();
    let land = device::build(&g.device_view(None, 0.0), area);
    assert!(land.video.width() > land.video.height() && land.uv.width() == 1.0);
}

fn ops_key(ops: &[Op]) -> String {
    format!("{ops:?}")
}

#[test]
fn scenes_and_bitmaps_are_deterministic() {
    let st = settings_with("yt_shorts", "device", "android");
    let g = GuideView::new(&st, "t", 9.0 / 16.0, 2.0, 8.0).unwrap();
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 900.0));
    let a = device::build(&g.device_view(Some(Sheet::Desc), 1.0), area);
    let b = device::build(&g.device_view(Some(Sheet::Desc), 1.0), area);
    assert_eq!(ops_key(&a.over), ops_key(&b.over));
    assert_eq!(ops_key(&a.under), ops_key(&b.under));
    let mut vid = Frame::new(108, 192);
    for (i, px) in vid.rgba.chunks_exact_mut(4).enumerate() {
        px.copy_from_slice(&[(i % 108) as u8 * 2, (i / 108) as u8, 90, 255]);
    }
    let r1 = raster::render_device(Some(&vid), &g, Some(Sheet::Comments), 360);
    let r2 = raster::render_device(Some(&vid), &g, Some(Sheet::Comments), 360);
    assert_eq!((r1.width, r1.height), (r2.width, r2.height));
    assert!(r1.height > r1.width, "a phone is portrait");
    assert_eq!(r1.rgba, r2.rgba, "same input, same bytes");
    let closed = raster::render_device(Some(&vid), &g, None, 360);
    assert_ne!(closed.rgba, r1.rgba, "an open sheet changes the picture");
    let o1 = raster::render_overlay(vid.clone(), &settings_view("ig_reels", "ui"));
    let o2 = raster::render_overlay(vid.clone(), &settings_view("ig_reels", "ui"));
    assert_eq!(o1.rgba, o2.rgba);
    assert_ne!(o1.rgba, vid.rgba, "the overlay drew something");
}

fn settings_view(id: &str, mode: &str) -> GuideView<'static> {
    let st: &'static Settings = Box::leak(Box::new(settings_with(id, mode, "iphone")));
    GuideView::new(st, "t", 9.0 / 16.0, 1.0, 10.0).unwrap()
}

#[test]
fn worst_case_is_inside_every_platform_of_the_same_shape() {
    let d = data();
    let plat = d.platform("tiktok").unwrap();
    let (w, names) = scene::worst_safe(&d, plat);
    assert!(names.len() >= 4, "{names:?}");
    for p in d.platforms.iter().filter(|p| p.a == plat.a) {
        let l = &p.lay["phone"];
        let s = l.safe.unwrap();
        assert!(w[0] >= s[0] / l.refsz[0] - 1e-4 && w[1] >= s[1] / l.refsz[1] - 1e-4 && w[2] <= s[2] / l.refsz[0] + 1e-4 && w[3] <= s[3] / l.refsz[1] + 1e-4, "{} contains the worst case", p.id);
    }
    assert!(w[0] < w[2] && w[1] < w[3], "a worst case still has room: {w:?}");
}

#[test]
fn the_playhead_fills_the_progress_bar() {
    let at = |t: f64| {
        let st = settings_with("yt_shorts", "ui", "iphone");
        let g = GuideView::new(&st, "t", 9.0 / 16.0, t, 10.0).unwrap();
        let ops = g.overlay_ops(Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 640.0)));
        // the progress fill is the last red-ish rect of the progress bar
        ops.iter().filter_map(|o| if let Op::Rect { r, fill, .. } = o { (*fill == scene::RED).then_some(r.width()) } else { None }).next().unwrap_or(0.0)
    };
    assert!(at(0.0) < 1.0 && (at(5.0) - 180.0).abs() < 1.0 && (at(10.0) - 360.0).abs() < 1.0);
}

#[test]
fn clock_text() {
    assert_eq!(paint::clock(11.0, 538.0), "0:11 / 8:58");
    assert_eq!(paint::clock(-3.0, 0.0), "0:00 / 0:00");
    assert_eq!(paint::clock(42.9, 231.0), "0:42 / 3:51", "rounds down like the real players");
    assert_eq!(paint::clock(90.0 / 30.0, 10.0), "0:03 / 0:10", "frame 90 at 30 fps is 0:03, not 0:02");
    assert_eq!(paint::clock(0.1 * 3.0 * 10.0, 10.0), "0:03 / 0:10", "2.9999... still reads 0:03");
    assert_eq!(paint::clock(65.0, 3723.0), "0:01:05 / 1:02:03", "hours once the video is an hour long");
}

#[test]
fn guide_time_and_progress_follow_the_playhead() {
    let st = settings_with("yt_long", "ui", "iphone");
    for (t, dur, p, c) in [(0.0, 231.0, 0.0, "0:00 / 3:51"), (42.0, 231.0, 42.0 / 231.0, "0:42 / 3:51"), (231.0, 231.0, 1.0, "3:51 / 3:51"), (300.0, 231.0, 1.0, "5:00 / 3:51"), (1.0, 0.0, 0.0, "0:01 / 0:00")] {
        let g = GuideView::new(&st, "t", 16.0 / 9.0, t, dur).unwrap();
        assert!((g.progress - p as f32).abs() < 1e-6, "progress at {t}/{dur}: {}", g.progress);
        assert_eq!(g.clock, c);
        assert!(!g.playing, "a new view is paused until the player says otherwise");
        let ops = g.overlay_ops(Rect::from_min_size(pos2(0.0, 0.0), vec2(390.0, 693.0)));
        assert!(ops.iter().any(|o| matches!(o, Op::Text { t, .. } if t == c)), "the time readout shows {c}");
    }
}

/// The player's play button: the play triangle while paused, two pause bars while playing.
#[test]
fn guide_play_button_shows_the_play_state() {
    let triangles = |playing: bool| {
        let st = settings_with("yt_long", "ui", "iphone");
        let mut g = GuideView::new(&st, "t", 16.0 / 9.0, 3.0, 10.0).unwrap();
        g.playing = playing;
        let ops = g.overlay_ops(Rect::from_min_size(pos2(0.0, 0.0), vec2(390.0, 693.0)));
        ops.iter().filter(|o| matches!(o, Op::Poly { p, .. } if p.len() == 3)).count()
    };
    assert_eq!(triangles(false), triangles(true) + 1, "playing swaps the one play triangle for pause bars");
    // the device preview's player too (desktop long form: the control bar's button)
    let st = settings_with("yt_long", "device", "desktop");
    let mut g = GuideView::new(&st, "t", 16.0 / 9.0, 3.0, 10.0).unwrap();
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 760.0));
    let paused = ops_key(&device::build(&g.device_view(None, 0.0), area).over);
    g.playing = true;
    assert_ne!(paused, ops_key(&device::build(&g.device_view(None, 0.0), area).over));
}

/// An exported (rasterised) frame: the Shorts progress bar is red up to the frame's own time.
#[test]
fn exported_frame_shows_the_bar_at_its_time() {
    let st = settings_with("yt_shorts", "ui", "iphone");
    let fps = 30.0;
    for (frame, want) in [(75u32, 90.0), (150, 180.0), (270, 324.0)] {
        let t = frame as f64 / fps; // the exported frame's time, 10 s timeline
        let mut g = GuideView::new(&st, "t", 9.0 / 16.0, t, 10.0).unwrap();
        g.playing = true;
        let out = raster::render_overlay(Frame::new(360, 640), &g);
        let red = (0..360).filter(|&x| {
            let i = ((638 * 360 + x) * 4) as usize;
            let px = &out.rgba[i..i + 4];
            px[0] > 150 && px[1] < 90
        });
        let end = red.max().map_or(0.0, |x| x as f32 + 1.0);
        assert!((end - want).abs() <= 2.0, "frame {frame}: bar ends at {end}, want {want}");
    }
}

/// A synthetic "video": diagonal gradient with a bright centre box, so crops and bars are visible.
fn test_video(w: u32, h: u32) -> Frame {
    let mut f = Frame::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let center = (fx - 0.5).abs() < 0.2 && (fy - 0.5).abs() < 0.12;
            let c = if center { [250, 210, 70] } else { [(40.0 + 90.0 * fx) as u8, (60.0 + 80.0 * fy) as u8, (120.0 + 80.0 * (1.0 - fx)) as u8] };
            f.rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    f
}

/// Pictures for the eye: `SE_GUIDE_DUMP=<dir> cargo test dump_pictures -- --ignored`.
#[test]
#[ignore]
fn dump_pictures() {
    let Some(dir) = std::env::var_os("SE_GUIDE_DUMP").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    // a made-up profile with a drawn avatar
    let av = test_video(64, 64);
    let tmp = dir.join("avatar.png");
    std::fs::write(&tmp, crate::mcp::png_encode(&av)).unwrap();
    let b64 = profile::avatar_from_file(&tmp).unwrap_or_default();
    let d = data();
    let mut s = Settings::default();
    for (net, _) in profile::NETS {
        let mut p = SocialProfile::sample(net);
        p.id = format!("t-{net}");
        p.avatar = b64.clone();
        p.display = "Kash Plays".into();
        p.handle = "kashplays".into();
        p.verified = true;
        s.social_profiles.push(p);
    }
    let write = |name: String, f: &Frame| std::fs::write(dir.join(name), crate::mcp::png_encode(f)).unwrap();
    for plat in &d.platforms {
        let (vw, vh) = if plat.aspect() > 1.0 { (1920, 1080) } else if plat.id == "ig_feed" { (1080, 1350) } else { (1080, 1920) };
        let vid = test_video(vw, vh);
        for mode in ["safe", "ui"] {
            s.guide = Some(plat.id.clone());
            s.guide_mode = mode.into();
            s.guide_device = "iphone".into();
            let g = GuideView::new(&s, "How to build a planted aquarium", vw as f32 / vh as f32, 5.0, 20.0).unwrap();
            let f = raster::render_overlay(vid.clone(), &g);
            write(format!("overlay-{}-{mode}.png", plat.id), &f);
        }
        for dev in &d.devices {
            s.guide = Some(plat.id.clone());
            s.guide_mode = "device".into();
            s.guide_device = dev.id.clone();
            let g = GuideView::new(&s, "How to build a planted aquarium", vw as f32 / vh as f32, 5.0, 20.0).unwrap();
            for (name, open) in [("closed", None), ("comments", Some(Sheet::Comments)), ("desc", Some(Sheet::Desc))] {
                let f = raster::render_device(Some(&vid), &g, open, if dev.k == "desktop" { 900 } else { 420 });
                write(format!("device-{}-{}-{name}.png", plat.id, dev.id), &f);
            }
        }
    }
    // the rest of the sheets, once
    s.guide = Some("tiktok".into());
    s.guide_device = "iphone".into();
    let g = GuideView::new(&s, "t", 9.0 / 16.0, 5.0, 20.0).unwrap();
    let vid = test_video(1080, 1920);
    for sh in Sheet::ALL {
        write(format!("sheet-tiktok-{}.png", sh.id()), &raster::render_device(Some(&vid), &g, Some(sh), 420));
    }
}

/// Two tap targets never sit on top of each other (the click would go to the wrong sheet).
#[test]
fn tap_targets_do_not_overlap() {
    for p in &data().platforms {
        for (name, l) in &p.lay {
            let taps: Vec<(Rect, &str)> = l.ui.iter().filter(|e| !matches!(e.tap.as_str(), "" | "none")).map(|e| (Rect::from_min_max(pos2(e.r[0], e.r[1]), pos2(e.r[2], e.r[3])), e.i.as_str())).collect();
            for (i, (a, ai)) in taps.iter().enumerate() {
                for (b, bi) in &taps[i + 1..] {
                    let x = a.intersect(*b);
                    let overlap = if x.is_positive() { x.area() } else { 0.0 };
                    assert!(overlap <= 0.2 * a.area().min(b.area()), "{}/{name}: '{ai}' and '{bi}' overlap", p.id);
                }
            }
        }
    }
}

/// A button that straddles the middle of the reference screen keeps its size on a taller phone, and a
/// notch pushes the top-anchored UI down and the bottom-anchored UI up.
#[test]
fn elements_keep_their_size_on_taller_phones_and_clear_the_notch() {
    let m = Map::new([360.0, 640.0], vec2(393.0, 852.0), pos2(0.0, 0.0), 1.0);
    let like = m.rect_el([312.0, 310.0, 344.0, 361.0]); // straddles y = 320
    assert!((like.height() - 51.0).abs() < 0.01, "kept its height: {}", like.height());
    let topbar = m.rect_el([300.0, 0.0, 340.0, 48.0]);
    assert_eq!(topbar.min.y, 0.0);
    let notched = m.insets(30.0, 34.0, 0.0);
    assert_eq!(notched.rect_el([300.0, 0.0, 340.0, 48.0]).min.y, 30.0, "below the island");
    let caption = notched.rect_el([16.0, 590.0, 300.0, 620.0]);
    assert!((caption.max.y - (620.0 + 212.0 - 34.0)).abs() < 0.01, "above the home indicator: {}", caption.max.y);
}


use super::device::op_bbox as bbox;

/// Nothing is drawn outside the device's clip: every shape between the body and the corner mask lies inside
/// the rounded screen (the browser window on desktop), the screen sits inside the bezel by exactly the bezel
/// width, and the whole device has an even margin in the viewer, for every device and viewer shape.
#[test]
fn device_geometry_stays_inside_the_clip_for_every_preset() {
    let d = data();
    for plat in &d.platforms {
        for dev in &d.devices {
            for area in [vec2(600.0, 900.0), vec2(1400.0, 500.0), vec2(300.0, 300.0), vec2(1000.0, 1000.0)] {
                let area = Rect::from_min_size(pos2(7.0, 11.0), area);
                for (open, vm) in [(None, 0.0), (Some(Sheet::Comments), 0.0), (Some(Sheet::Share), 0.0), (None, 1.0), (None, 2.0), (None, 0.4)] {
                    let st = settings_with(&plat.id, "device", &dev.id);
                    let g = GuideView::new(&st, "t", 9.0 / 16.0, 3.0, 10.0).unwrap();
                    let mut v = g.device_view(open, if open.is_some() { 1.0 } else { 0.0 });
                    v.vm = vm;
                    let sc = device::build(&v, area);
                    let ctx = format!("{} on {} in {area:?} open {open:?} vm {vm}", plat.id, dev.id);
                    assert!(area.contains_rect(sc.outer), "device inside the viewer: {ctx}");
                    let m = area.width().min(area.height()) * 0.04;
                    let (l, r) = (sc.outer.min.x - area.min.x, area.max.x - sc.outer.max.x);
                    let (t, b) = (sc.outer.min.y - area.min.y, area.max.y - sc.outer.max.y);
                    assert!((l - r).abs() < 1.0 && (t - b).abs() < 1.0, "centred, even margins {l} {r} {t} {b}: {ctx}");
                    assert!(l.min(t) >= m.max(10.0).min(area.width().min(area.height()) * 0.2) - 0.5, "margin kept: {ctx}");
                    if dev.k == "desktop" {
                        assert_eq!(sc.bezel, 0.0);
                        assert!(sc.outer.contains_rect(sc.screen));
                    } else {
                        let inset = sc.screen.min.x - sc.outer.min.x;
                        assert!((inset - sc.bezel).abs() < 0.01 && (sc.outer.max.y - sc.screen.max.y - sc.bezel).abs() < 0.01, "screen inside the bezel by its width: {ctx}");
                    }
                    let clip = if dev.k == "desktop" { sc.outer } else { sc.screen }.expand(0.75);
                    for (i, op) in sc.under.iter().enumerate().skip(sc.chrome_under) {
                        if let Some(bb) = bbox(op) {
                            assert!(clip.contains_rect(bb), "under op {i} {op:?} leaves the screen {clip:?}: {ctx}");
                        }
                    }
                    let end = sc.over.len() - sc.mask_over;
                    assert!(sc.mask_over >= 1, "the corner mask is the last thing drawn: {ctx}");
                    for (i, op) in sc.over[..end].iter().enumerate() {
                        if let Some(bb) = bbox(op) {
                            assert!(clip.contains_rect(bb), "over op {i} {op:?} leaves the screen {clip:?}: {ctx}");
                        }
                    }
                    assert!(clip.contains_rect(sc.video.expand(-0.01)) || vm > 0.0, "video inside the screen: {ctx}");
                }
            }
        }
    }
}

/// The side action column keeps a margin from the screen edge and the bottom bar clears the home indicator.
#[test]
fn phone_ui_keeps_off_the_screen_edge() {
    let st = settings_with("tiktok", "device", "iphone");
    let g = GuideView::new(&st, "t", 9.0 / 16.0, 3.0, 10.0).unwrap();
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 900.0));
    let sc = device::build(&g.device_view(None, 0.0), area);
    let k = sc.screen.width() / 393.0;
    let mut right_most = f32::MIN;
    for op in &sc.over[..sc.over.len() - sc.mask_over] {
        if let Op::Circle { c, r, .. } | Op::Avatar { c, r } = op {
            if c.y > sc.screen.min.y + 200.0 * k && c.y < sc.screen.max.y - 120.0 * k {
                right_most = right_most.max(c.x + r);
            }
        }
    }
    assert!(sc.screen.max.x - right_most >= 8.0 * k, "the action column is {} pt from the edge", sc.screen.max.x - right_most);
}

/// Desktop long form: default, theatre and fullscreen. Sizes follow the measured table, the video scales
/// between them, the control bar's theatre / fullscreen buttons ask for the view, and fullscreen drops the browser.
#[test]
fn desktop_player_views() {
    let st = settings_with("yt_long", "device", "desktop");
    let g = GuideView::new(&st, "t", 16.0 / 9.0, 3.0, 10.0).unwrap();
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 700.0));
    let at = |vm: f32| {
        let mut v = g.device_view(None, 0.0);
        v.vm = vm;
        device::build(&v, area)
    };
    let (d, t, f) = (at(0.0), at(1.0), at(2.0));
    let page = d.screen.width();
    assert!((d.video.width() / page - 1344.0 / 1920.0).abs() < 0.01, "default player is 70% of the page: {}", d.video.width() / page);
    assert!((t.video.width() / page - 1620.0 / 1920.0).abs() < 0.01, "theatre video is 84% of the page, pillarboxed in a full-width box: {}", t.video.width() / page);
    assert!(t.video.width() > d.video.width());
    assert!((f.video.width() - f.outer.width()).abs() < 1.0, "fullscreen fills the window's width");
    assert!(f.under.len() < d.under.len() + 10 && f.under.iter().skip(f.chrome_under).any(|o| matches!(o, Op::Rect { fill, .. } if *fill == Color32::BLACK)));
    let mid = at(0.5);
    assert!(mid.video.width() > d.video.width() && mid.video.width() < t.video.width(), "scales between the views");
    assert_eq!(d.base, d.video, "the render size follows the default player");
    // the control bar's buttons
    let tap = |sc: &device::Scene, v: ViewMode| sc.hits.iter().rev().find(|(_, h)| *h == Hit::View(v)).map(|(r, _)| r.center()).expect("view button");
    let mut s = paint::State::default();
    assert_eq!(s.click(&d, tap(&d, ViewMode::Theatre), ViewMode::Default), Some(Set::View(ViewMode::Theatre)));
    assert_eq!(s.click(&t, tap(&t, ViewMode::Theatre), ViewMode::Theatre), Some(Set::View(ViewMode::Default)), "the same button again goes back");
    assert_eq!(s.click(&d, tap(&d, ViewMode::Full), ViewMode::Default), Some(Set::View(ViewMode::Full)));
    assert_eq!(s.click(&f, tap(&f, ViewMode::Full), ViewMode::Full), Some(Set::View(ViewMode::Default)));
    // ids and settings
    assert_eq!(ViewMode::from_id("fullscreen"), Some(ViewMode::Full));
    assert_eq!(ViewMode::from_id("theatre"), Some(ViewMode::Theatre));
    assert_eq!(ViewMode::from_id("nope"), None);
    let mut set = Settings::default();
    Set::View(ViewMode::Theatre).apply(&mut set);
    assert_eq!(set.guide_view, "theatre");
    // a phone has no such buttons
    let st = settings_with("yt_long", "device", "iphone");
    let g = GuideView::new(&st, "t", 16.0 / 9.0, 3.0, 10.0).unwrap();
    assert!(!device::build(&g.device_view(None, 0.0), area).hits.iter().any(|(_, h)| matches!(h, Hit::View(_))));
}

/// The real Shorts app has a thumbs-down; the guide does not draw one (removed at the user's request).
#[test]
fn youtube_guides_have_no_dislike() {
    for id in ["yt_shorts", "yt_long"] {
        let p = data().platform(id).unwrap().clone();
        for (name, l) in &p.lay {
            for e in &l.ui {
                assert!(!e.i.contains("dislike") && !e.i.contains("thumb_down") && e.n != "dislike", "{id}/{name} draws {} {}", e.t, e.i);
                assert!(!e.lbl.to_lowercase().contains("dislike"), "{id}/{name}");
            }
            for z in &l.zones {
                assert!(!z.l.to_lowercase().contains("dislike"), "{id}/{name}: {}", z.l);
            }
        }
    }
    // the YouTube like is a thumbs-up, the others' a heart; no glyph is named after the old shape
    let d = data();
    let like_of = |id: &str| d.platform(id).unwrap().lay["phone"].ui.iter().find(|e| e.n == "likes").map(|e| e.i.clone());
    assert_eq!(like_of("yt_shorts").as_deref(), Some("thumb"));
    assert_eq!(like_of("tiktok").as_deref(), Some("like"));
    assert!(!EMBEDDED.contains("dislike"));
}

/// Every icon the data names has a drawing (an unknown name falls back to a plain ring, which is a bug).
#[test]
fn every_icon_name_has_a_glyph() {
    let mut b = scene::B::new();
    for p in &data().platforms {
        for l in p.lay.values() {
            for e in l.ui.iter().filter(|e| e.t == "ico") {
                let before = b.ops.len();
                scene::glyph(&mut b, &e.i, pos2(10.0, 10.0), 5.0, Color32::WHITE);
                let ops = &b.ops[before..];
                assert!(!(ops.len() == 1 && matches!(ops[0], Op::Circle { fill, .. } if fill == Color32::TRANSPARENT)), "{}: icon '{}' has no drawing", p.id, e.i);
            }
        }
    }
}

/// The red bar's right end on row `y` of an RGBA frame `w` wide (0 = no red).
fn red_end(rgba: &[u8], w: usize, y: usize) -> f32 {
    (0..w).filter(|&x| {
        let px = &rgba[(y * w + x) * 4..][..4];
        px[0] > 150 && px[1] < 90
    })
    .max()
    .map_or(0.0, |x| x as f32 + 1.0)
}

/// Export ▸ Burn in social guide: each frame's bar is that frame's share of the exported video (the
/// timeline, or the In/Out range when that is what is exported).
#[test]
fn burnt_in_guide_follows_the_exported_frame() {
    let st = settings_with("yt_shorts", "device", "iphone"); // a device preview burns as its UI overlay
    let mut p = crate::model::Project::default();
    (p.width, p.height, p.fps) = (360, 640, 30.0);
    p.add_text_clip(0.0, 4.0);
    let o = raster::export_overlay(&st, &p, None).expect("a guide is chosen");
    for (t, want) in [(1.0, 90.0), (2.0, 180.0), (3.0, 270.0)] {
        let f = (o.0)(Frame::new(360, 640), t);
        assert!((red_end(&f.rgba, 360, 638) - want).abs() <= 2.0, "t {t}");
    }
    let o = raster::export_overlay(&st, &p, Some((1.0, 3.0))).unwrap();
    assert!((red_end(&(o.0)(Frame::new(360, 640), 2.0).rgba, 360, 638) - 180.0).abs() <= 2.0, "half way through the range");
    assert!(raster::export_overlay(&Settings::default(), &p, None).is_none(), "no guide, nothing burnt in");
    // none of the editor's own marks (the guide's name, "Measured ...") are burnt in
    let mut g = GuideView::new(&st, "t", 9.0 / 16.0, 1.0, 4.0).unwrap();
    let says = |g: &GuideView, w: &str| g.overlay_ops(Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 640.0))).iter().any(|o| matches!(o, Op::Text { t, .. } if t.contains(w)));
    assert!(says(&g, "Measured"), "the viewer says where the guide comes from");
    g.export = true;
    assert!(!says(&g, "Measured") && !says(&g, "UI preview"), "an export does not");

    // a real export (skipped without ffmpeg): frame 60 of 120 has the bar half way
    let Some(exe) = crate::media::ffpipe::ffmpeg_exe() else { return };
    let dir = std::env::temp_dir().join(format!("se-guide-burn-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join("guide.mp4");
    use crate::engine::export::{self, ExportOptions};
    let opts = ExportOptions {
        out_path: out.clone(),
        encoder: "libx264".into(),
        crf: 12,
        preset: "ultrafast".into(),
        backend: crate::media::Backend::Auto,
        out_size: None,
        scaler: "bicubic".into(),
        frames: export::FrameSource::Cpu,
        metadata: Vec::new(),
        range: None,
        loudnorm: false,
        letterbox: false,
        audio: Default::default(),
        stem_by: Default::default(),
        overlay: raster::export_overlay(&st, &p, None),
    };
    let prog = export::start_export(p, opts, std::sync::Arc::new(std::sync::Mutex::new(crate::engine::text::TextRasterizer::new())));
    for _ in 0..1200 {
        if prog.is_done() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(prog.error(), None);
    let raw = crate::media::ffpipe::command(&exe)
        .args(["-v", "error", "-i"])
        .arg(&out)
        .args(["-vf", r"select=eq(n\,60)", "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(raw.len(), 360 * 640 * 4);
    let end = red_end(&raw, 360, 638);
    assert!((end - 180.0).abs() <= 3.0, "exported frame 60 of 120: bar ends at {end}");
    let _ = std::fs::remove_dir_all(&dir);
}
