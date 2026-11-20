//! ---- ws:gallery-stock ---- parsing (saved real responses in `fixtures/`; the Pexels / Pixabay / Freesound ones
//! are written from their API docs, since no key ships with the app), licence filtering, attribution, and the
//! download + import path. The tests that use the network are `#[ignore]`d: `cargo test stock -- --ignored`.

use super::sources::*;
use super::*;

fn fx(name: &str) -> &'static str {
    match name {
        "c_vid" => include_str!("fixtures/c_vid.json"),
        "c_img" => include_str!("fixtures/c_img.json"),
        "c_aud" => include_str!("fixtures/c_aud.json"),
        "ia_vid" => include_str!("fixtures/ia_vid.json"),
        "ia_aud" => include_str!("fixtures/ia_aud.json"),
        "ia_meta" => include_str!("fixtures/ia_meta.json"),
        "nasa" => include_str!("fixtures/nasa.json"),
        "nasa_img" => include_str!("fixtures/nasa_img.json"),
        "nasa_aud" => include_str!("fixtures/nasa_aud.json"),
        "nasa_assets" => include_str!("fixtures/nasa_assets.json"),
        "ov_img" => include_str!("fixtures/ov_img.json"),
        "ov_aud" => include_str!("fixtures/ov_aud.json"),
        "ov_mus" => include_str!("fixtures/ov_mus.json"),
        "pexels_videos" => include_str!("fixtures/pexels_videos.json"),
        "pexels_photos" => include_str!("fixtures/pexels_photos.json"),
        "pixabay_videos" => include_str!("fixtures/pixabay_videos.json"),
        "pixabay_images" => include_str!("fixtures/pixabay_images.json"),
        "freesound" => include_str!("fixtures/freesound.json"),
        _ => panic!("no fixture {name}"),
    }
}

fn hits(src: Source, kind: Kind, name: &str) -> Vec<Hit> {
    parse(src, kind, fx(name)).unwrap()
}

#[test]
fn stock_commons_parses_video_picture_and_audio() {
    let v = hits(Source::Commons, Kind::Video, "c_vid");
    assert_eq!(v.len(), 3);
    let h = &v[0];
    assert_eq!(h.id, "commons:81916645");
    assert_eq!(h.title, "Ocean in Sommaroy");
    assert_eq!(h.author, "User: Netha Hussain", "the Artist HTML becomes text");
    assert_eq!(h.licence(), Licence::Pd);
    assert_eq!(h.licence_name, "CC0");
    assert_eq!((h.width, h.height), (1920, 1080));
    assert!(h.duration > 15.0);
    assert!(h.file_url.ends_with("Ocean_in_Sommaroy.webm"), "{}", h.file_url);
    assert!(!h.file_url.contains("utm_") && !h.thumb_url.contains("utm_"));
    assert_eq!(h.page_url, "https://commons.wikimedia.org/wiki/File:Ocean_in_Sommaroy.webm");
    assert_eq!(v[1].licence(), Licence::By);
    assert_eq!(v[2].licence(), Licence::BySa);

    let i = hits(Source::Commons, Kind::Image, "c_img");
    assert_eq!(i.len(), 3);
    assert!(i.iter().all(|h| h.kind == Kind::Image && h.licence() == Licence::By && h.width > 4000));

    let a = hits(Source::Commons, Kind::Sfx, "c_aud");
    assert_eq!(a.len(), 3);
    assert!(a.iter().all(|h| h.kind == Kind::Sfx && h.thumb_url.is_empty()), "the file-type icon is not a thumbnail");
    assert!(a[1].title.contains("Québec"), "{}", a[1].title);
    // a picture search never returns the video results and back
    assert!(hits(Source::Commons, Kind::Image, "c_vid").is_empty());
    assert!(hits(Source::Commons, Kind::Video, "c_aud").is_empty());
}

#[test]
fn stock_archive_parses_search_and_picks_a_file() {
    let v = hits(Source::Archive, Kind::Video, "ia_vid");
    assert_eq!(v.len(), 6);
    assert_eq!(v[0].licence(), Licence::BySa);
    assert_eq!(v[1].licence(), Licence::Pd);
    assert_eq!(v[1].licence_name, "CC0 1.0");
    assert_eq!(v[1].thumb_url, "https://archive.org/services/img/gov.house.sgw.20080429");
    assert!(v.iter().all(|h| h.file_url.is_empty() && !h.detail.is_empty()), "the file is resolved on Add");
    let a = hits(Source::Archive, Kind::Music, "ia_aud");
    assert_eq!(a[0].licence_name, "Public Domain Mark 1.0");
    assert_eq!(a[3].licence(), Licence::By);
    // a Prelinger film with no licence URL is public domain
    let pre = r#"{"response":{"docs":[{"identifier":"200325_The_Open_Sea","title":"The Open Sea","creator":"Frith Films",
        "mediatype":"movies","collection":["prelinger","feature_films"]}]}}"#;
    let p = parse(Source::Archive, Kind::Video, pre).unwrap();
    assert_eq!(p[0].licence(), Licence::Pd);
    assert!(p[0].licence_name.contains("Prelinger"));
    // ... and anything else without one is never shown
    let other = pre.replace("\"prelinger\",", "");
    assert_eq!(parse(Source::Archive, Kind::Video, &other).unwrap()[0].licence(), Licence::Other);

    let (url, size) = archive_file("200325_The_Open_Sea", Kind::Video, fx("ia_meta")).unwrap();
    assert_eq!(url, "https://archive.org/download/200325_The_Open_Sea/200325_The_Open_Sea_master.intros.mp4");
    assert_eq!(size, 33291657, "the h.264 derivative, not the 900 MB .mov");
    let (mp3, _) = archive_file("200325_The_Open_Sea", Kind::Music, fx("ia_meta")).unwrap();
    assert!(mp3.ends_with(".mp3"));
}

#[test]
fn stock_nasa_parses_items_and_collection() {
    let v = hits(Source::Nasa, Kind::Video, "nasa");
    assert_eq!(v.len(), 3);
    let h = &v[0];
    assert_eq!(h.id, "nasa:NHQ_2019_0311_Go Forward to the Moon");
    assert_eq!(h.licence(), Licence::Pd);
    assert!(h.thumb_url.starts_with("https://") && !h.thumb_url.contains(' '), "{}", h.thumb_url);
    assert!(h.detail.ends_with("collection.json") && !h.detail.contains(' '));
    assert!(h.file_url.is_empty());
    assert_eq!(h.page_url, "https://images.nasa.gov/details/NHQ_2019_0311_Go%20Forward%20to%20the%20Moon");
    let i = hits(Source::Nasa, Kind::Image, "nasa_img");
    assert_eq!(i[0].file_url, "https://images-assets.nasa.gov/image/PIA01973/PIA01973~orig.jpg");
    assert_eq!((i[0].width, i[0].height), (425, 460));
    assert_eq!(i[0].author, "NASA/JPL");
    assert_eq!(hits(Source::Nasa, Kind::Sfx, "nasa_aud").len(), 3);

    let big = nasa_file(Kind::Video, fx("nasa_assets"), false).unwrap();
    assert!(big.ends_with("~large.mp4") && big.starts_with("https://") && !big.contains(' '), "{big}");
    let small = nasa_file(Kind::Video, fx("nasa_assets"), true).unwrap();
    assert!(small.ends_with("~preview.mp4"), "{small}");
}

#[test]
fn stock_openverse_parses_pictures_and_audio() {
    let i = hits(Source::Openverse, Kind::Image, "ov_img");
    assert_eq!(i.len(), 3);
    assert_eq!(i[0].title, "To the Ocean");
    assert_eq!(i[0].licence_name, "CC0 1.0");
    assert_eq!(i[0].page_url, "https://www.flickr.com/photos/24046097@N00/6051705623");
    assert_eq!((i[0].width, i[0].height), (1024, 683));
    let a = hits(Source::Openverse, Kind::Sfx, "ov_aud");
    assert!((a[0].duration - 240.079).abs() < 0.01, "milliseconds -> seconds");
    let m = hits(Source::Openverse, Kind::Music, "ov_mus");
    assert_eq!(m[0].licence(), Licence::BySa);
    assert_eq!(m[0].licence_name, "CC BY-SA 3.0");
}

#[test]
fn stock_keyed_sources_parse_their_documented_responses() {
    let v = hits(Source::Pexels, Kind::Video, "pexels_videos");
    assert_eq!(v[0].title, "Waves crashing on rocks");
    assert_eq!((v[0].width, v[0].height), (3840, 2160));
    assert!(v[0].preview_url.contains("sd_640"), "the smallest file previews");
    assert_eq!(v[0].licence(), Licence::Free);
    assert_eq!(v[1].height, 1920);
    let p = hits(Source::Pexels, Kind::Image, "pexels_photos");
    assert_eq!(p[0].title, "Brown Rocks During Golden Hour");
    assert_eq!(p[0].author, "Joey Farina");

    let pv = hits(Source::Pixabay, Kind::Video, "pixabay_videos");
    assert!(pv[0].file_url.ends_with("_medium.mp4"), "an empty large falls back to medium");
    assert_eq!(pv[0].title, "flowers, yellow, blossom");
    assert!(pv[0].preview_url.ends_with("_tiny.mp4"));
    assert!(pv[0].thumb_url.ends_with("_medium.jpg"));
    let pi = hits(Source::Pixabay, Kind::Image, "pixabay_images");
    assert_eq!((pi[0].width, pi[0].height), (1280, 720), "largeImageURL is 1280 px on the long side");

    let f = hits(Source::Freesound, Kind::Sfx, "freesound");
    assert_eq!(f[0].licence(), Licence::Pd);
    assert_eq!(f[0].licence_name, "CC0 1.0");
    assert!(f[0].file_url.ends_with("-hq.mp3"));
    assert_eq!(f[1].licence(), Licence::Other, "NC is never usable here");
}

#[test]
fn stock_bad_responses_are_errors_or_empty() {
    assert!(parse(Source::Commons, Kind::Video, "<html>").is_err());
    for s in Source::ALL {
        assert!(parse(s, Kind::Video, "{}").unwrap().is_empty(), "{s:?}");
    }
}

#[test]
fn stock_licences_read_from_names_and_urls() {
    for (s, l) in [
        ("CC0", Licence::Pd),
        ("Public domain", Licence::Pd),
        ("http://creativecommons.org/publicdomain/zero/1.0/", Licence::Pd),
        ("https://creativecommons.org/publicdomain/mark/1.0/", Licence::Pd),
        ("pdm", Licence::Pd),
        ("CC BY 4.0", Licence::By),
        ("https://creativecommons.org/licenses/by/3.0/", Licence::By),
        ("by", Licence::By),
        ("CC BY-SA 3.0", Licence::BySa),
        ("https://creativecommons.org/licenses/by-sa/2.0/", Licence::BySa),
        ("CC BY-NC 4.0", Licence::Other),
        ("https://creativecommons.org/licenses/by-nc-sa/3.0/", Licence::Other),
        ("https://creativecommons.org/licenses/by-nd/4.0/", Licence::Other),
        ("GFDL", Licence::Other),
        ("", Licence::Other),
    ] {
        assert_eq!(Licence::parse(s), l, "{s}");
    }
    assert_eq!(cc_name("https://creativecommons.org/licenses/by-sa/4.0/"), "CC BY-SA 4.0");
    assert_eq!(cc_name("http://creativecommons.org/publicdomain/zero/1.0/"), "CC0 1.0");
}

#[test]
fn stock_licence_filter_defaults_to_no_credit_needed() {
    let q = Query::default();
    assert_eq!(q.licence, LicenceLevel::Pd, "the default is the most permissive for the user");
    let h = |l: Licence| Hit { licence: Some(l), width: 1920, height: 1080, ..Default::default() };
    assert!(passes(&h(Licence::Pd), &q));
    assert!(passes(&h(Licence::Free), &q));
    assert!(!passes(&h(Licence::By), &q));
    assert!(!passes(&h(Licence::BySa), &q));
    assert!(!passes(&h(Licence::Other), &q));
    let by = Query { licence: LicenceLevel::By, ..Default::default() };
    assert!(passes(&h(Licence::By), &by) && !passes(&h(Licence::BySa), &by));
    let sa = Query { licence: LicenceLevel::BySa, ..Default::default() };
    assert!(passes(&h(Licence::BySa), &sa) && !passes(&h(Licence::Other), &sa));
    // over the real Commons sample: CC0 only by default, everything at BY-SA
    let v = hits(Source::Commons, Kind::Video, "c_vid");
    assert_eq!(v.iter().filter(|x| passes(x, &q)).count(), 1);
    assert_eq!(v.iter().filter(|x| passes(x, &sa)).count(), 3);
}

#[test]
fn stock_size_and_shape_filters() {
    let h = |w, hh| Hit { licence: Some(Licence::Pd), width: w, height: hh, ..Default::default() };
    let q = |o, r| Query { orientation: o, min_res: r, ..Default::default() };
    assert!(passes(&h(1920, 1080), &q(Orientation::Landscape, 1080)));
    assert!(!passes(&h(1280, 720), &q(Orientation::Any, 1080)));
    assert!(!passes(&h(1080, 1920), &q(Orientation::Landscape, 0)));
    assert!(passes(&h(1080, 1920), &q(Orientation::Portrait, 1080)));
    assert!(passes(&h(1000, 1050), &q(Orientation::Square, 0)));
    // unknown size: shown until a size or shape is asked for
    assert!(passes(&h(0, 0), &q(Orientation::Any, 0)));
    assert!(!passes(&h(0, 0), &q(Orientation::Any, 720)));
    assert!(!passes(&h(0, 0), &q(Orientation::Portrait, 0)));
    // sounds have no size
    let s = Hit { kind: Kind::Sfx, licence: Some(Licence::Pd), ..Default::default() };
    assert!(passes(&s, &q(Orientation::Portrait, 2160)));
}

#[test]
fn stock_attribution_text() {
    let v = hits(Source::Commons, Kind::Video, "c_vid");
    assert_eq!(
        attribution(&v[0]),
        "\"Ocean in Sommaroy\" by User: Netha Hussain, CC0, via Wikimedia Commons - https://commons.wikimedia.org/wiki/File:Ocean_in_Sommaroy.webm"
    );
    assert_eq!(
        attribution(&v[1]),
        "\"Chemical Pollution of the Ocean\" by Surfrider Foundation Europe, CC BY 3.0 (https://creativecommons.org/licenses/by/3.0), via Wikimedia Commons - https://commons.wikimedia.org/wiki/File:Chemical_Pollution_of_the_Ocean.webm"
    );
    let bare = Hit { licence: Some(Licence::Pd), ..Default::default() };
    assert_eq!(attribution(&bare), "\"Untitled\", Public domain");
    let n = &hits(Source::Nasa, Kind::Image, "nasa_img")[0];
    assert!(attribution(n).starts_with("\"Saturn Atmosphere\" by NASA/JPL, NASA media"), "{}", attribution(n));
}

#[test]
fn stock_credits_list_every_stock_file_once_credited_first() {
    let v = hits(Source::Commons, Kind::Video, "c_vid");
    let cs = vec![
        ("a.webm".to_string(), Credit::of(&v[0])),
        ("b.webm".to_string(), Credit::of(&v[1])),
        ("a-again.webm".to_string(), Credit::of(&v[0])),
    ];
    let t = credits_text(&cs, "My Video");
    assert!(t.starts_with("Credits - My Video\n"));
    let (pb, pa) = (t.find("Chemical Pollution").unwrap(), t.find("Ocean in Sommaroy").unwrap());
    assert!(pb < pa, "the one that must be credited comes first:\n{t}");
    assert_eq!(t.matches("Ocean in Sommaroy\"").count(), 1, "deduplicated by id");
    assert!(t.contains("(no credit required)"));
    assert!(credits_text(&[], "").contains("No stock media"));
}

#[test]
fn stock_requests_are_keyless_or_carry_only_the_users_key() {
    let q = Query { text: "ocean waves".into(), kind: Kind::Video, ..Default::default() };
    for s in [Source::Commons, Source::Archive, Source::Nasa] {
        let r = request(s, &q, None).unwrap();
        assert!(r.headers.is_empty() && r.url.starts_with("https://"), "{s:?}: {r:?}");
        assert!(r.url.contains("ocean%20waves"), "{}", r.url);
    }
    assert!(request(Source::Openverse, &q, None).is_none(), "Openverse has no video");
    let img = Query { kind: Kind::Image, orientation: Orientation::Portrait, ..q.clone() };
    let ov = request(Source::Openverse, &img, None).unwrap();
    assert!(ov.url.contains("license=cc0,pdm") && ov.url.contains("aspect_ratio=tall"), "{}", ov.url);
    // keyed sources: nothing without a key, and the key only where the site's docs put it
    assert!(request(Source::Pexels, &q, None).is_none());
    assert!(request(Source::Pexels, &q, Some("  ")).is_none());
    let p = request(Source::Pexels, &q, Some("K1")).unwrap();
    assert_eq!(p.headers, vec!["Authorization: K1".to_string()]);
    assert!(!p.url.contains("K1"));
    let px = request(Source::Pixabay, &q, Some("K2")).unwrap();
    assert!(px.url.contains("key=K2") && px.url.starts_with("https://pixabay.com/api/videos/"));
    let fs = request(Source::Freesound, &Query { kind: Kind::Sfx, ..q.clone() }, Some("K3")).unwrap();
    assert_eq!(fs.headers, vec!["Authorization: Token K3".to_string()]);
    // the licence level reaches the Archive's query
    let by = request(Source::Archive, &Query { licence: LicenceLevel::By, ..q.clone() }, None).unwrap();
    assert!(by.url.contains("by%5C%2F") && !request(Source::Archive, &q, None).unwrap().url.contains("by%5C%2F"));
    // and the user agent says nothing about the user
    let ua = user_agent();
    assert!(ua.starts_with("SimpleEditor/") && !ua.contains('@') && !ua.contains("Users"));
}

#[test]
fn stock_file_names_and_extensions() {
    let v = hits(Source::Commons, Kind::Video, "c_vid");
    assert_eq!(file_name(&v[0], "webm"), "commons-ocean-in-sommaroy-81916645.webm");
    assert_eq!(url_ext("https://x/a/b.MP4?x=1"), "mp4");
    assert_eq!(url_ext("https://archive.org/services/img/abc"), "");
    let mut h = v[0].clone();
    h.title = "???".into();
    assert_eq!(file_name(&h, ""), "commons-81916645.mp4", "no extension: the kind's default");
    assert_eq!(safe_url("http://a/b c~d.mp4"), "https://a/b%20c~d.mp4");
}

#[test]
fn stock_interleaves_sources() {
    let h = |id: &str| Hit { id: id.into(), ..Default::default() };
    let out = interleave(vec![vec![h("a1"), h("a2"), h("a3")], vec![h("b1")], vec![]]);
    let ids: Vec<&str> = out.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, ["a1", "b1", "a2", "a3"]);
}

// ------------------------------------------------------------------------------------- network (ignored)

fn live(src: Source, kind: Kind, text: &str) -> Vec<Hit> {
    let q = Query { text: text.into(), kind, licence: LicenceLevel::BySa, ..Default::default() };
    let req = request(src, &q, None).unwrap();
    let body = get(&req, 30).unwrap_or_else(|e| panic!("{src:?}: {e}"));
    parse(src, kind, &String::from_utf8_lossy(&body)).unwrap()
}

#[test]
#[ignore = "network"]
fn stock_live_keyless_sources_answer_without_a_key() {
    for (s, k, t) in [
        (Source::Commons, Kind::Video, "ocean"),
        (Source::Commons, Kind::Image, "mountain"),
        (Source::Archive, Kind::Video, "ocean"),
        (Source::Archive, Kind::Music, "piano"),
        (Source::Nasa, Kind::Video, "moon"),
        (Source::Nasa, Kind::Image, "saturn"),
        (Source::Openverse, Kind::Image, "ocean"),
        (Source::Openverse, Kind::Sfx, "rain"),
        (Source::Openverse, Kind::Music, "piano"),
    ] {
        let v = live(s, k, t);
        assert!(!v.is_empty(), "{s:?} {k:?} found nothing");
        assert!(v.iter().all(|h| h.source == Some(s) && !h.id.is_empty()));
    }
}

#[test]
#[ignore = "network"]
fn stock_live_download_and_import_into_a_scratch_project() {
    let dir = std::env::temp_dir().join(format!("se-stock-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // a small NASA picture, a public-domain Openverse sound, an Archive film's file list
    let img = live(Source::Nasa, Kind::Image, "saturn").into_iter().next().unwrap();
    let prog = crate::engine::export::Progress::new();
    let file = fetch(&img, &dir, None, &prog).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() > 1000);
    let c = read_credit(&file).unwrap();
    assert_eq!(c.id, img.id);
    assert!(c.attribution.contains("via NASA"));
    // imported like any file: the probe sees a picture
    let mut p = crate::model::Project::new();
    let id = p.add_asset(crate::engine::import::placeholder(&file.to_string_lossy()));
    assert!(p.asset(id).is_some());
    let a = crate::media::ffpipe::probe(&file.to_string_lossy()).unwrap();
    assert_eq!(a.kind, crate::model::ClipKind::Image);
    let film = live(Source::Archive, Kind::Video, "ocean").into_iter().next().unwrap();
    let (url, _) = resolve(&film).unwrap();
    assert!(url.starts_with("https://archive.org/download/"), "{url}");
    let _ = std::fs::remove_dir_all(&dir);
}
