use super::*;

fn asset(path: &str, kind: ClipKind, dur: f64, streams: usize) -> Asset {
    Asset {
        id: 0,
        path: path.into(),
        kind,
        duration: dur,
        width: 1920,
        height: 1080,
        fps: 30.0,
        audio_streams: (0..streams)
            .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
            .collect(),
        codec: "h264".into(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
        rel_path: None,
        parent: None,
        range: None,
        effects: Vec::new(),
        html: None,
        proxy: None,
    }
}

fn block(p: &mut Project, id: Id, name: &str, t: f64, dur: f64) {
    let (w, h) = (p.width as f64, p.height as f64);
    let b = find_block(&[], name).unwrap();
    assert!(p.clip_mut(id).unwrap().place_block(&b, t, Some(dur), w, h).is_some(), "{name} did not land");
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("se-tpl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Position of the track a clip is on, among the video tracks (0 = lowest).
fn layer_of(p: &Project, id: Id) -> usize {
    let ti = p.track_of(id).unwrap();
    p.video_tracks().iter().position(|&t| t == ti).unwrap()
}

/// Two clips with a cross fade, animation blocks on both and a title over them.
fn animated_project() -> (Project, Vec<Id>) {
    let mut p = Project::new();
    let a = p.add_asset(asset("C:/tpl-a.mp4", ClipKind::Video, 4.0, 0));
    let b = p.add_asset(asset("C:/tpl-b.mp4", ClipKind::Video, 4.0, 0));
    let v1 = p.insert_asset_clips(a, 0.0, None)[0];
    let v2 = p.insert_asset_clips(b, 4.0, None)[0];
    assert_eq!(p.track_of(v1), p.track_of(v2));
    block(&mut p, v1, "Pop In", 0.0, 0.4);
    block(&mut p, v1, "Ken Burns", 0.5, 3.0);
    block(&mut p, v2, "Slide In Right", 0.0, 0.6);
    block(&mut p, v2, "Fade Out", 3.5, 0.5);
    assert!(p.add_transition(v2, TransitionKind::CrossFade, 0.5).is_some());
    let title = p.add_text_clip(1.0, 3.0);
    block(&mut p, title, "Drop In", 0.0, 0.6);
    assert!(layer_of(&p, title) > layer_of(&p, v1), "the title sits over the footage");
    (p, vec![v1, v2, title])
}

#[test]
fn a_template_round_trips_with_its_blocks_keys_layers_and_transitions() {
    let (p, ids) = animated_project();
    let d = capture(&p, &ids);
    assert_eq!((d.clips.len(), d.assets.len(), d.transitions.len(), d.layers.len()), (3, 2, 1, 3));
    // through the gallery file's JSON and back
    let d = TemplateData::decode(&serde_json::to_string(&d).unwrap()).unwrap();
    let mut q = Project::new();
    let first = q.add_text_clip(0.0, 30.0); // something already on V1 where it lands
    let new = apply(&mut q, d, 10.0, &[]);
    assert_eq!(new.len(), 3);
    for (old, new) in ids.iter().zip(&new) {
        let (o, n) = (p.clip(*old).unwrap(), q.clip(*new).unwrap());
        assert!((n.start - (o.start + 10.0)).abs() < 1e-9);
        // (a number may come back from JSON one bit off: compared to a billionth)
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert_eq!(n.blocks.len(), o.blocks.len());
        for (x, y) in n.blocks.iter().zip(&o.blocks) {
            assert!(x.name == y.name && x.props == y.props && near(x.t, y.t) && near(x.dur, y.dur), "{}: {}", o.name, y.name);
            let (cx, cy) = (x.ctl.as_ref().expect("the Animator controls survive"), y.ctl.as_ref().unwrap());
            assert!(cx.src.name == cy.src.name && cx.src.tracks.len() == cy.src.tracks.len() && cx.dir == cy.dir && near(cx.strength, cy.strength));
        }
        for prop in BLOCK_PROPS {
            let (a, b) = (&prop_ref(n, prop).unwrap().keys, &prop_ref(o, prop).unwrap().keys);
            assert_eq!(a.len(), b.len(), "{}: {prop} keys", o.name);
            assert!(a.iter().zip(b).all(|(x, y)| near(x.t, y.t) && near(x.v, y.v) && x.ease == y.ease), "{}: {prop} keys", o.name);
        }
    }
    // stacking is kept (and nothing lands on the clip that was already there)
    assert!(layer_of(&q, new[0]) > layer_of(&q, first));
    assert_eq!(layer_of(&q, new[0]), layer_of(&q, new[1]));
    assert!(layer_of(&q, new[2]) > layer_of(&q, new[0]));
    // the cross fade came along, on the new clip
    let t = &q.tracks[q.track_of(new[1]).unwrap()].transitions;
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].right, t[0].kind), (new[1], TransitionKind::CrossFade));
    assert_eq!(q.assets.len(), 2);
}

/// Slots A (0-3 s, trim) and B (3-6 s, stretch, fade in / fade out), a caption over B and a title after it.
fn slot_template() -> TemplateData {
    let mut p = Project::new();
    let a = p.add_asset(asset("C:/tpl-a.mp4", ClipKind::Video, 3.0, 1));
    let b = p.add_asset(asset("C:/tpl-b.mp4", ClipKind::Video, 3.0, 0));
    let va = p.insert_asset_clips(a, 0.0, None);
    assert_eq!(va.len(), 2, "picture + its sound");
    let vb = p.insert_asset_clips(b, 3.0, None)[0];
    block(&mut p, vb, "Fade In", 0.0, 0.5);
    block(&mut p, vb, "Fade Out", 2.5, 0.5);
    let caption = p.add_text_clip(3.0, 3.0);
    let after = p.add_text_clip(6.0, 2.0);
    p.clip_mut(caption).unwrap().name = "Caption".into();
    p.clip_mut(after).unwrap().name = "After".into();
    let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
    let parts = [
        Part { clip: va[0], slot: Some(("A".into(), SlotWant::Video, SlotFit::Trim)), include: false },
        Part { clip: vb, slot: Some(("B".into(), SlotWant::Any, SlotFit::Stretch)), include: false },
    ];
    build(&p, &ids, &parts, 1.0)
}

fn by_name<'a>(p: &'a Project, name: &str) -> &'a Clip {
    p.all_clips().map(|(_, c)| c).find(|c| c.name == name || c.container_label == name && c.kind != ClipKind::Audio).unwrap()
}

#[test]
fn building_marks_slots_placeholders_and_cards() {
    let d = slot_template();
    let slots = d.slot_list();
    assert_eq!(slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!((slots[0].want, slots[0].fit, slots[1].fit), (SlotWant::Video, SlotFit::Trim, SlotFit::Stretch));
    assert!(d.assets.is_empty(), "footage that was left out is not in the template");
    for s in &slots {
        let c = d.clip(s.clip).unwrap();
        assert!(c.is_empty_container() && c.container_label == s.name);
        let card = d.clip(s.card).expect("an empty slot has a card under it");
        assert_eq!(card.kind, ClipKind::Shape);
        assert_eq!((card.start, card.duration, &card.blocks), (c.start, c.duration, &c.blocks));
        let layer = |id: Id| d.layers[d.clips.iter().position(|c| c.id == id).unwrap()];
        assert_eq!(layer(s.card) + 1, layer(s.clip));
    }
    assert!((d.thumb_t - 1.0).abs() < 1e-9);
    // placed and built again: the cards on the timeline are the slots' cards, none is added
    let mut q = Project::new();
    let ids = apply(&mut q, d.clone(), 0.0, &[]);
    let again = build(&q, &ids, &[], 0.0);
    assert_eq!(again.clips.len(), d.clips.len());
    assert!(again.slot_list().iter().all(|s| again.clip(s.card).is_some_and(|c| c.kind == ClipKind::Shape)));
    // sound that is left out is dropped; footage that stays is kept, and a slot can keep its footage
    let (p, ids) = animated_project();
    let d = build(&p, &ids, &[Part { clip: ids[0], slot: Some((String::new(), SlotWant::Any, SlotFit::Trim)), include: true }], 0.0);
    assert_eq!(d.assets.len(), 2);
    let s = d.slot_list();
    assert_eq!(s.len(), 1);
    assert!(d.clip(s[0].clip).unwrap().asset != 0 && s[0].card == 0, "footage included: no card");
    assert_eq!(s[0].name, "tpl-a.mp4", "an unnamed slot takes the clip name");
}

#[test]
fn filling_a_slot_follows_its_timing_rule() {
    let fresh = || {
        let mut q = Project::new();
        let short = q.add_asset(asset("C:/short.mp4", ClipKind::Video, 2.0, 0));
        let long = q.add_asset(asset("C:/long.mp4", ClipKind::Video, 8.0, 1));
        let img = q.add_asset(asset("C:/photo.png", ClipKind::Image, 0.0, 0));
        (q, short, long, img)
    };
    // nothing filled: both slots stay placeholders with their cards
    let (mut q, short, long, img) = fresh();
    let ids = apply(&mut q, slot_template(), 5.0, &[]);
    assert_eq!(ids.len(), 7, "2 slots + A's sound + 2 cards + 2 texts");
    assert!(by_name(&q, "A").is_empty_container() && by_name(&q, "B").is_empty_container());

    // trim: footage shorter than the slot is slowed to fill it; the slot keeps its 3 s. Silent footage
    // removes the slot's sound half, and a filled slot loses its card
    let (mut q, ..) = fresh();
    let ids = apply(&mut q, slot_template(), 5.0, &[(0, short)]);
    let a = by_name(&q, "A");
    assert_eq!((a.asset, a.duration), (short, 3.0));
    assert!((a.speed - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(ids.len(), 5, "A's card and its empty sound half are gone");
    assert!(!q.all_clips().any(|(_, c)| c.kind == ClipKind::Audio));
    assert!(q.all_clips().any(|(_, c)| c.name == "B card") && !q.all_clips().any(|(_, c)| c.name == "A card"));

    // trim: longer footage is cut to the slot and its sound fills the slot's sound half
    let (mut q, ..) = fresh();
    apply(&mut q, slot_template(), 5.0, &[(0, long)]);
    let a = by_name(&q, "A");
    assert_eq!((a.asset, a.duration, a.speed), (long, 3.0, 1.0));
    let sound: Vec<&Clip> = q.all_clips().map(|(_, c)| c).filter(|c| c.kind == ClipKind::Audio).collect();
    assert_eq!(sound.len(), 1);
    assert_eq!((sound[0].asset, sound[0].link), (long, a.link));

    // stretch: the slot takes the footage's 8 s, what follows moves by 5 s, the caption over it grows,
    // and the slot's fade out stays at its end
    let (mut q, ..) = fresh();
    apply(&mut q, slot_template(), 5.0, &[(1, long)]);
    let b = by_name(&q, "B");
    assert_eq!((b.start, b.duration), (8.0, 8.0));
    let fade_out = b.blocks.iter().find(|x| x.name == "Fade Out").unwrap();
    assert!((fade_out.t - 7.5).abs() < 1e-9 && (fade_out.dur - 0.5).abs() < 1e-9);
    let fade_in = b.blocks.iter().find(|x| x.name == "Fade In").unwrap();
    assert!(fade_in.t.abs() < 1e-9 && (fade_in.dur - 0.5).abs() < 1e-9);
    let times: Vec<f64> = b.opacity.keys.iter().map(|k| k.t).collect();
    assert_eq!(times, [0.0, 0.5, 7.5, 8.0]);
    assert_eq!(by_name(&q, "After").start, 5.0 + 6.0 + 5.0);
    assert_eq!((by_name(&q, "Caption").start, by_name(&q, "Caption").duration), (8.0, 8.0));
    assert_eq!(by_name(&q, "A").start, 5.0, "what comes before stays");

    // stretch to shorter footage: everything closes up, nothing overlaps
    let (mut q, ..) = fresh();
    apply(&mut q, slot_template(), 0.0, &[(1, short)]);
    let b = by_name(&q, "B");
    assert_eq!((b.duration, by_name(&q, "After").start), (2.0, 5.0));
    assert!(b.opacity.keys.windows(2).all(|k| k[0].t < k[1].t) && b.opacity.keys.last().unwrap().t <= 2.0 + 1e-9);

    // an image has no length of its own: the slot keeps its timing either way
    let (mut q, ..) = fresh();
    apply(&mut q, slot_template(), 0.0, &[(0, img), (1, img)]);
    let b = by_name(&q, "B");
    assert_eq!((b.kind, b.duration, by_name(&q, "After").start), (ClipKind::Image, 3.0, 6.0));
    // one undo step is the caller's; here: no track was left behind just for cards
    assert!(q.tracks.iter().all(|t| !t.clips.is_empty() || t.id <= 2), "empty card track removed");
    // sound alone never fills a picture slot
    let (mut q, ..) = fresh();
    let music = q.add_asset(asset("C:/music.mp3", ClipKind::Audio, 9.0, 1));
    apply(&mut q, slot_template(), 0.0, &[(0, music)]);
    assert!(by_name(&q, "A").is_empty_container());
}

#[test]
fn the_built_in_templates_are_different_and_lean_on_blocks() {
    assert!(BUILTIN_NAMES.len() >= 8);
    let mut seen: Vec<String> = Vec::new();
    for name in BUILTIN_NAMES {
        for (w, h) in [(1920u32, 1080u32), (1080, 1920)] {
            let d = builtin(name, w, h).unwrap_or_else(|| panic!("{name} is missing"));
            let slots = d.slot_list();
            assert!(!slots.is_empty(), "{name}: footage goes somewhere");
            assert_eq!(d.layers.len(), d.clips.len());
            let blocks: Vec<&AppliedBlock> = d.clips.iter().flat_map(|c| &c.blocks).collect();
            assert!(blocks.len() >= 4, "{name}: {} blocks", blocks.len());
            assert!(blocks.iter().all(|b| b.ctl.is_some()), "{name}: every block keeps its Animator controls");
            for c in &d.clips {
                for b in &c.blocks {
                    assert!(b.t >= -1e-9 && b.end() <= c.duration + 1e-6, "{name}/{}: {} leaves the clip", c.name, b.name);
                }
            }
            for s in &slots {
                let c = d.clip(s.clip).unwrap();
                assert!(c.is_empty_container(), "{name}: a slot is a placeholder, not somebody's footage");
                assert_eq!(d.clip(s.card).map(|c| c.kind), Some(ClipKind::Shape), "{name}: a colour card under {}", s.name);
            }
            assert!(d.assets.is_empty(), "{name}: no media of its own");
            // it survives its JSON and places with every clip
            let n = d.clips.len();
            let back = TemplateData::decode(&serde_json::to_string(&d).unwrap()).unwrap();
            let mut p = Project::new();
            (p.width, p.height) = (w, h);
            assert_eq!(apply(&mut p, back, 2.0, &[]).len(), n, "{name}");
            if w > h {
                seen.push(format!("{} slots, {} clips, {:.1} s, {} transitions", slots.len(), n, d.duration(), d.transitions.len()));
            }
        }
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "two built-in templates have the same shape: {seen:?}");
    assert!(builtin("Photo Slideshow", 1920, 1080).unwrap().transitions.len() == 3);
    assert!(builtin("nope", 1920, 1080).is_none());
    // yours wins over a built-in one of the same name
    let mine = crate::settings::Template { name: "Quote Card".into(), json: r#"{"clips":[],"assets":[]}"#.into() };
    assert!(find(&[mine], "quote card", 1920, 1080).unwrap().clips.is_empty());
    assert!(!find(&[], "quote card", 1920, 1080).unwrap().clips.is_empty());
}

#[test]
fn old_templates_still_load_and_place() {
    // what a template saved before slots looked like: clips and assets, nothing else (a bare clip too)
    let json = r#"{"clips":[{"id":1,"kind":"Text","name":"T","start":0.0,"duration":2.0},
                            {"id":2,"kind":"Shape","name":"S","start":1.0,"duration":2.0}],"assets":[]}"#;
    let d = TemplateData::decode(json).unwrap();
    assert!(d.layers.is_empty() && d.slots.is_empty() && d.transitions.is_empty() && d.slot_list().is_empty());
    let (clips, assets) = crate::engine::presets::decode_template(&crate::settings::Template { name: "old".into(), json: json.into() }).unwrap();
    let mut a = Project::new();
    let mut b = Project::new();
    let old = a.place_clips(clips, assets, 4.0);
    let new = apply(&mut b, d, 4.0, &[]);
    assert_eq!(old.len(), 2);
    for (x, y) in old.iter().zip(&new) {
        assert_eq!((a.track_of(*x), a.clip(*x).unwrap().start), (b.track_of(*y), b.clip(*y).unwrap().start));
    }
    // an old template with container clips: each is a slot, and fills
    let mut p = Project::new();
    let (v, _) = p.add_container_clip(0.0, 4.0);
    p.clip_mut(v).unwrap().container_label = "Hero".into();
    let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
    let t = crate::engine::presets::capture_template("Old", &p, &ids);
    assert!(!t.json.contains("slots") && !t.json.contains("layers"), "the old writer's shape");
    let d = TemplateData::decode(&t.json).unwrap();
    let slots = d.slot_list();
    assert_eq!(slots.len(), 1);
    assert_eq!((slots[0].name.as_str(), slots[0].fit), ("Hero", SlotFit::Trim));
    let mut q = Project::new();
    let clip = q.add_asset(asset("C:/hero.mp4", ClipKind::Video, 9.0, 1));
    let ids = apply(&mut q, d, 0.0, &[(0, clip)]);
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| q.clip(*id).unwrap().asset == clip));
}

#[test]
fn a_template_exports_and_imports_with_and_without_footage() {
    let dir = tmp("share");
    let (src, root) = (dir.join("my footage"), dir.join("gallery"));
    std::fs::create_dir_all(&src).unwrap();
    let (fa, fb) = (src.join("beach.mp4"), src.join("logo.png"));
    std::fs::write(&fa, b"not really a video, 28 bytes").unwrap();
    std::fs::write(&fb, b"png!").unwrap();
    let mut p = Project::new();
    let a = p.add_asset(asset(&fa.to_string_lossy(), ClipKind::Video, 4.0, 1));
    let b = p.add_asset(asset(&fb.to_string_lossy(), ClipKind::Image, 0.0, 0));
    let va = p.insert_asset_clips(a, 0.0, None);
    let vb = p.insert_asset_clips(b, 4.0, None)[0];
    block(&mut p, va[0], "Pop In", 0.0, 0.4);
    let title = p.add_text_clip(0.5, 2.0);
    let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
    let d = build(&p, &ids, &[Part { clip: vb, slot: Some(("Logo".into(), SlotWant::Image, SlotFit::Trim)), include: true }], 0.0);
    let pops = p.clip(va[0]).unwrap().blocks.clone();
    let _ = title;

    // with footage: both files ride along, and nothing in the header says where they were
    let with = dir.join("with.setemplate");
    assert_eq!(export_package("Beach Intro", &d, true, None, &with).unwrap(), Exported { files: 2, empty: 0 });
    let bytes = std::fs::read(&with).unwrap();
    let head_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let head = String::from_utf8(bytes[20..20 + head_len].to_vec()).unwrap();
    assert!(!head.contains("my footage") && !head.contains("se-tpl-share") && !head.contains(":\\\\") && !head.contains(":/"), "{head}");
    assert!(head.contains("0-beach.mp4"));
    let item = import_package(&root, &with).unwrap();
    assert_eq!((item.name.as_str(), item.kind), ("Beach Intro", Kind::Template));
    let got: TemplateData = serde_json::from_value(item.data.clone()).unwrap();
    assert_eq!(got.assets.len(), 2);
    for a in &got.assets {
        let f = Path::new(&a.path);
        assert!(f.starts_with(root.join("templates").join("footage")) && f.is_file(), "{}", a.path);
    }
    let beach = got.assets.iter().find(|a| a.path.ends_with("beach.mp4")).unwrap();
    assert_eq!(std::fs::read(&beach.path).unwrap(), b"not really a video, 28 bytes");
    assert_eq!((beach.duration, beach.audio_streams.len()), (4.0, 1), "what was known about the footage is kept");
    let same = |a: &[AppliedBlock], b: &[AppliedBlock]| {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.name == y.name && (x.t - y.t).abs() < 1e-9 && x.ctl.is_some())
    };
    assert!(same(&got.clip(va[0]).unwrap().blocks, &pops), "the animation comes along");
    assert_eq!(got.slot_list().len(), 1);
    // it lists in the Gallery like any saved template, and a second import is numbered
    assert_eq!(gallery::list(&root, Kind::Template).len(), 1);
    assert_eq!(import_package(&root, &with).unwrap().name, "Beach Intro (2)");
    // the Gallery importer takes the file too
    let r = gallery::import(&root, &[with.clone()]);
    assert_eq!(r.added, [(Kind::Template, "Beach Intro (3)".to_string())]);

    // without footage: every footage clip is an empty slot, no file inside
    let without = dir.join("without.setemplate");
    assert_eq!(export_package("Beach Intro", &d, false, None, &without).unwrap(), Exported { files: 0, empty: 2 });
    assert!(std::fs::metadata(&without).unwrap().len() < std::fs::metadata(&with).unwrap().len());
    let root2 = dir.join("gallery2");
    let got: TemplateData = serde_json::from_value(import_package(&root2, &without).unwrap().data).unwrap();
    assert!(got.assets.is_empty());
    assert_eq!(got.slot_list().len(), 2, "the logo slot, and the beach clip that lost its footage");
    assert!(got.clips.iter().filter(|c| c.uses_asset()).all(|c| c.asset == 0 && c.container));
    assert!(!root2.join("templates").join("footage").exists());
    // ... and it still places, with the animation on the empty slot
    let mut q = Project::new();
    let n = got.clips.len();
    assert_eq!(apply(&mut q, got, 0.0, &[]).len(), n);
    assert!(q.all_clips().any(|(_, c)| c.is_empty_container() && same(&c.blocks, &pops)));

    // footage that is gone when exporting becomes an empty slot, the rest still rides along
    std::fs::remove_file(&fb).unwrap();
    let part = dir.join("part.setemplate");
    assert_eq!(export_package("Beach Intro", &d, true, None, &part).unwrap(), Exported { files: 1, empty: 1 });

    // a card picture rides along and lands beside the imported template
    let png = dir.join("card.png");
    std::fs::write(&png, b"PNGDATA").unwrap();
    let pic = dir.join("pic.setemplate");
    export_package("With Picture", &d, false, Some(&png), &pic).unwrap();
    let item = import_package(&root, &pic).unwrap();
    assert_eq!(std::fs::read(thumb_path(&item.path)).unwrap(), b"PNGDATA");
    let left: Vec<_> = std::fs::read_dir(root.join("templates").join("footage")).unwrap().flatten().collect();
    assert!(left.iter().all(|d| std::fs::read_dir(d.path()).unwrap().count() > 0), "no empty footage folder is left behind");

    // not a template, or cut short: an error, nothing saved
    let junk = dir.join("junk.setemplate");
    std::fs::write(&junk, b"hello there, this is not one").unwrap();
    assert!(import_package(&root, &junk).is_err());
    std::fs::write(&junk, &bytes[..bytes.len() - 5]).unwrap();
    let before = gallery::list(&root, Kind::Template).len();
    assert!(import_package(&root, &junk).is_err());
    assert_eq!(gallery::list(&root, Kind::Template).len(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_absolute_path_survives_in_a_shared_header() {
    let mut v = json!({"a": "C:\\Users\\me\\luts\\warm.cube", "b": ["D:/clips/x.mp4", "\\\\nas\\share\\y.mov", "C: is a drive", "/s"],
                       "c": {"file": "E:\\page.html", "n": 3}});
    scrub(&mut v);
    assert_eq!(v, json!({"a": "warm.cube", "b": ["x.mp4", "y.mov", "C: is a drive", "/s"], "c": {"file": "page.html", "n": 3}}));
}
