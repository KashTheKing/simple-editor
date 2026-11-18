//! ---- ws:edit-templates ---- tests of the beat edits: grid, motion, song / sound slots, drop, retime.

use super::*;

/// The shipped beat edits (the phonk edit is yours only: `phonk_edit`).
const EDITS: [&str; 3] = ["Velocity Edit", "Hype Montage", "Meme Cut"];

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

/// The edit with its song and sound made slots again (the engine still fits a template to a song given in
/// a sound slot; the built-ins bring their own).
fn with_sound_slots(mut d: TemplateData) -> TemplateData {
    let song = d.sync.as_ref().unwrap().song;
    d.set_slot(song, "Song", SlotWant::Audio, SlotFit::Trim);
    let s = d.slots.pop().unwrap();
    d.slots.insert(0, s);
    if let Some(w) = d.clips.iter().find(|c| c.name == "Wasted sound").map(|c| c.id) {
        d.set_slot(w, "Wasted sound", SlotWant::Audio, SlotFit::Trim);
        let s = d.slots.last_mut().unwrap();
        (s.hint, s.hit) = ("wasted".into(), 0.6);
    }
    d
}

/// Seconds `t` is on the grid of `period` from `offset`.
fn on_grid(t: f64, offset: f64, period: f64) -> bool {
    let k = (t - offset) / period;
    (k - k.round()).abs() < 1e-6
}

#[test]
fn edit_templates_cut_on_the_beat_and_never_stand_still() {
    assert!(crate::engine::shaderlib::lint(ZOOM_MOMENTUM, true).is_empty(), "the zoom transition lints clean");
    for name in EDITS {
        assert!(BUILTIN_NAMES.contains(&name));
        let d = builtin(name, 1920, 1080).unwrap();
        let sy = d.sync.clone().unwrap_or_else(|| panic!("{name}: a beat edit"));
        assert_eq!(sy.bpm, BPM);
        let slots = d.slot_list();
        // only the video shots are slots: the song (and phonk's sound and WASTED shot) are the template's own
        assert!(slots.iter().all(|s| s.want == SlotWant::Video && d.clip(s.clip).unwrap().kind == ClipKind::Video), "{name}: a slot that is not a video shot");
        let song = d.clip(sy.song).unwrap();
        assert!(!song.container && song.kind == ClipKind::Audio, "{name}: the song is fixed");
        assert!((song.duration - d.duration()).abs() < 1e-9, "{name}: the song runs under all of it");
        assert!(on_grid(sy.drop, 0.0, BEAT), "{name}: the drop is on a beat");
        let shots: Vec<&Clip> = slots.iter().filter(|s| s.want != SlotWant::Audio).map(|s| d.clip(s.clip).unwrap()).collect();
        assert!(shots.len() >= 6, "{name}: {} shots", shots.len());
        for c in &shots {
            assert!(on_grid(c.start, 0.0, BEAT) && on_grid(c.end(), 0.0, BEAT), "{name}/{}: cut off the beat", c.name);
            // always moving: a motion block under every moment of the shot (bar a frame)
            let mut t = 0.0;
            while t < c.duration - 0.04 {
                let moving = c
                    .blocks
                    .iter()
                    .any(|b| b.props.iter().any(|p| matches!(p.as_str(), "scale" | "x" | "y")) && b.t <= t + 0.034 && b.end() >= t);
                assert!(moving, "{name}/{}: static at {t:.2} s", c.name);
                t += 1.0 / 30.0;
            }
        }
        // the cuts change picture with flashes and zooms (whips for some)
        let flash = d.transitions.iter().filter(|t| t.kind == TransitionKind::FadeToColor && t.color == [255, 255, 255, 255]).count();
        let zoom = d.transitions.iter().filter(|t| t.kind == TransitionKind::Shader && t.shader.contains("@name Zoom Momentum")).count();
        let whip = d.transitions.iter().filter(|t| t.kind == TransitionKind::Push).count();
        assert!(flash + zoom + whip >= 5, "{name}: {flash} flashes, {zoom} zooms, {whip} whips");
        assert!(d.thumb_t > 0.0 && d.thumb_t < d.duration(), "{name}: its Gallery picture moment");
    }
}

#[test]
fn the_phonk_edit_muffles_until_the_drop_then_wastes_the_shot() {
    let d = phonk_edit(1920, 1080);
    let sy = d.sync.clone().unwrap();
    let song = d.clip(sy.song).unwrap();
    let bus = d.buses.iter().find(|b| b.id == song.bus).expect("the song plays through its own bus");
    let lp = bus.filters.iter().find(|f| f.kind == FilterKind::LowPass).expect("a low-pass");
    assert!(lp.at(0, 1.0) < 600.0 && lp.at(0, sy.drop - 0.6) < 600.0, "muffled before the drop");
    assert!(lp.at(0, sy.drop + 0.01) >= 15000.0 && lp.at(0, d.duration() - 0.1) >= 15000.0, "open from the drop on");
    // the wasted shot: grey and slow from the hit, its card on top, the real sound's hit on the drop
    let w = d.clips.iter().find(|c| c.name == "Wasted shot").unwrap();
    assert!(!w.container && !d.slot_list().iter().any(|s| s.clip == w.id), "the WASTED shot is the template's own");
    let hit = sy.drop - w.start;
    assert!(w.start < sy.drop && w.end() > sy.drop + 1.0);
    assert!(w.speed_curve.at(hit + 0.3) < 0.7 && w.speed_curve.at(hit - 0.2) > 0.9, "a slight slow-mo from the drop");
    let grey = w.effects.iter().find(|e| e.kind == EffectKind::Color).unwrap();
    assert!((grey.start - hit).abs() < 1e-9 && grey.params[2].at(hit + 0.3) < 0.01 && grey.params[2].at(hit) > 0.99, "drained to grey on the drop");
    let word = d.clips.iter().find(|c| c.text.as_ref().is_some_and(|t| t.text == "wasted")).expect("the WASTED card is a text clip");
    assert!((word.start - sy.drop).abs() < 1e-9);
    assert!(!d.slot_list().iter().any(|s| d.clip(s.clip).is_some_and(|c| c.kind == ClipKind::Audio)), "no sound slot");
    let c = d.clips.iter().find(|c| c.name == "Wasted sound").unwrap();
    assert!(!c.container && c.kind == ClipKind::Audio, "the wasted sound is the template's own");
    // (its file is cut so the sting is 0.6 s in)
    assert!((c.start + 0.6 - sy.drop).abs() < 1e-9, "the sound's hit lands on the drop");
    // after the card: a flash into the next shot
    let next = d
        .clips
        .iter()
        .filter(|c| c.container && c.kind == ClipKind::Video && c.start >= w.end() - 1e-9)
        .min_by(|a, b| a.start.total_cmp(&b.start))
        .unwrap();
    let t = d.transitions.iter().find(|t| t.right == next.id).unwrap();
    assert_eq!((t.kind, t.color), (TransitionKind::FadeToColor, [255, 255, 255, 255]));
}

#[test]
fn a_song_retimes_the_edit_onto_its_grid_and_drop() {
    for (bpm, offset, drop) in [(140.0, 0.31, 30.0), (70.0, 0.31, 30.0), (95.0, 0.1, 48.2), (128.0, 0.0, 2.0)] {
        let mut d = phonk_edit(1920, 1080);
        let src_in = d.fit_to_song(&SongBeat { bpm, offset, drop, len: 180.0 }).unwrap();
        let sy = d.sync.clone().unwrap();
        // the tempo the edit cuts at is the song's, or its double / half
        let ratio = sy.bpm / bpm;
        assert!([0.5, 1.0, 2.0].iter().any(|r| (ratio - r).abs() < 1e-6), "{bpm}: cuts at {} BPM", sy.bpm);
        assert!(src_in >= 0.0, "{bpm}: the song starts inside it");
        if drop >= sy.drop {
            assert!((src_in + sy.drop - drop).abs() < 1e-6, "{bpm}: the drops meet");
        }
        let period = 60.0 / sy.bpm;
        for c in d.clips.iter().filter(|c| c.container && c.kind == ClipKind::Video) {
            assert!(on_grid(src_in + c.start - drop, 0.0, period), "{bpm}: {} starts off the song's beat", c.name);
        }
    }
    // the low-pass opens on the retimed drop
    let mut d = phonk_edit(1920, 1080);
    d.fit_to_song(&SongBeat { bpm: 150.0, offset: 0.0, drop: 40.0, len: 120.0 });
    let drop = d.sync.as_ref().unwrap().drop;
    let lp = &d.buses[0].filters[0];
    assert!(lp.at(0, drop - 0.5) < 600.0 && lp.at(0, drop + 0.01) > 15000.0);
}

#[test]
fn placing_a_beat_edit_fills_the_song_the_sound_and_every_shot() {
    let mut p = Project::new();
    let song = p.add_asset(asset("C:/phonk.mp3", ClipKind::Audio, 120.0, 1));
    let sfx = p.add_asset(asset("C:/wasted-green-screen.mp4", ClipKind::Video, 11.0, 1));
    let clips: Vec<Id> = (0..3).map(|i| p.add_asset(asset(&format!("C:/game{i}.mp4"), ClipKind::Video, 60.0, 1))).collect();
    let d = with_sound_slots(phonk_edit(p.width, p.height));
    let slots = d.slot_list();
    let wants: Vec<SlotWant> = slots.iter().map(|s| s.want).collect();
    let mut media: Vec<(Id, bool)> = clips.iter().map(|&c| (c, false)).collect();
    media.insert(1, (song, true));
    let got = assign(&wants, &media, true);
    assert_eq!(got[0], Some(song), "the song goes in the song slot");
    let shots = wants.iter().filter(|w| **w != SlotWant::Audio).count();
    assert_eq!(got.iter().zip(&wants).filter(|(g, w)| g.is_some() && **w != SlotWant::Audio).count(), shots, "three clips fill every shot, in turn");
    assert_eq!(assign(&wants, &media, false).iter().flatten().count(), 4, "without cycling: one each");
    let sfx_slot = slots.iter().position(|s| s.name == "Wasted sound").unwrap();
    let mut fills: Vec<Fill> = got.iter().enumerate().filter_map(|(i, g)| g.map(|asset| Fill { slot: i, asset, onset: 0.0 })).collect();
    fills.push(Fill { slot: sfx_slot, asset: sfx, onset: 2.46 });
    let at = 5.0;
    // a drop as find_drop reports one: on a bar line of the song's grid (bar 18)
    let song_drop = 0.2 + 72.0 * 60.0 / 140.0;
    let ids = apply_synced(&mut p, d, at, &fills, Some(SongBeat { bpm: 140.0, offset: 0.2, drop: song_drop, len: 120.0 }));
    let placed: Vec<&Clip> = ids.iter().filter_map(|&id| p.clip(id)).collect();
    // the song: from where its drop meets the edit's, through the muffle bus
    let s = placed.iter().find(|c| c.asset == song).unwrap();
    let period = 60.0 / 140.0;
    let drop_tl = at + song_drop - s.src_in;
    assert!(on_grid(s.src_in, 0.2, period), "the song starts on its beat");
    let bus = p.bus(s.bus).expect("a new bus for the muffle");
    assert!(bus.filters[0].at(0, drop_tl - 0.6) < 600.0 && bus.filters[0].at(0, drop_tl + 0.01) > 15000.0, "it opens on the drop");
    // the sound's hit (2.46 s into its file) is on the drop
    let w = placed.iter().find(|c| c.asset == sfx && c.kind == ClipKind::Audio).unwrap();
    assert!((w.start + 2.46 - w.src_in - drop_tl).abs() < 1e-6, "hit at {} vs drop {drop_tl}", w.start + 2.46 - w.src_in);
    // every shot filled, no placeholder card left, the same clip plays a different stretch each time
    assert!(placed.iter().all(|c| !c.is_empty_container()), "a slot stayed empty");
    // (the Wasted band is a real shape; a placeholder is the "<slot> card" under an empty slot)
    let cards: Vec<String> = slots.iter().map(|s| format!("{} card", s.name)).collect();
    assert!(placed.iter().all(|c| c.kind != ClipKind::Shape || !cards.contains(&c.name)), "a placeholder card stayed");
    for &a in &clips {
        let mut ins: Vec<f64> = placed.iter().filter(|c| c.asset == a && c.kind == ClipKind::Video).map(|c| c.src_in).collect();
        let n = ins.len();
        ins.sort_by(f64::total_cmp);
        ins.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
        assert!(n >= 4 && ins.len() == n, "{a}: {n} uses, {} different starts", ins.len());
    }
    // the shots cut on the song's beats, and the Beat Aligner knows the song and its drop
    for c in placed.iter().filter(|c| c.kind == ClipKind::Video) {
        assert!(on_grid(c.start - at + s.src_in, 0.2, period), "{} is off the beat", c.name);
    }
    assert_eq!((p.beats.asset, p.beats.bpm), (song, 140.0));
    // footage the shape of the frame is not scaled
    assert!(placed.iter().filter(|c| c.kind == ClipKind::Video).all(|c| c.scale.value < 1.2));
    assert!(p.beats.sections.iter().any(|x| x.name == "Drop" && (x.a - song_drop).abs() < 1e-9));
}

#[test]
fn tall_footage_covers_a_wide_edit() {
    let mut p = Project::new();
    let mut tall = asset("C:/short.mp4", ClipKind::Video, 30.0, 0);
    (tall.width, tall.height) = (1080, 1920);
    let tall = p.add_asset(tall);
    let d = builtin("Velocity Edit", p.width, p.height).unwrap();
    let base = d.clip(d.slot_list()[1].clip).unwrap().scale.clone();
    let ids = apply_synced(&mut p, d, 0.0, &[Fill { slot: 1, asset: tall, onset: 0.0 }], None);
    let c = ids.iter().filter_map(|&id| p.clip(id)).find(|c| c.asset == tall).unwrap();
    let k = (1920.0 / 1080.0) * (1920.0 / 1080.0);
    assert!((c.scale.value - base.value * k).abs() < 1e-6, "{} vs {}", c.scale.value, base.value * k);
    assert!(c.scale.keys.iter().zip(&base.keys).all(|(a, b)| (a.v - b.v * k).abs() < 1e-6), "the motion scales along");
}

#[test]
fn a_beat_edit_without_a_song_places_at_its_own_tempo() {
    for name in EDITS {
        let d = builtin(name, 1080, 1920).unwrap();
        let (n, dur) = (d.clips.len(), d.duration());
        let mut p = Project::new();
        let ids = apply_synced(&mut p, d, 1.0, &[], None);
        assert_eq!(ids.len(), n, "{name}");
        assert!((p.duration() - 1.0 - dur).abs() < 1e-6, "{name}");
        assert_eq!(p.beats.asset, 0, "{name}: no song, no Beat Aligner sheet");
    }
}

#[test]
fn the_drop_is_the_biggest_jump_in_low_end() {
    let hz = 100.0;
    // 120 BPM (2 s bars): a quiet intro with a small bump, the drop at 24 s, a breakdown at 40 s
    let lv: Vec<f32> = (0..6000)
        .map(|i| match i as f64 / hz {
            t if t < 10.0 => 0.1,
            t if t < 24.0 => 0.25,
            t if t < 40.0 => 0.9,
            _ => 0.3,
        })
        .collect();
    assert_eq!(find_drop(&lv, hz, 120.0, 0.0, 0.0), Some(24.0));
    let shifted: Vec<f32> = std::iter::repeat_n(0.1, 50).chain(lv.iter().copied()).collect();
    assert_eq!(find_drop(&shifted, hz, 120.0, 0.5, 0.0), Some(24.5), "on the song's own bar lines");
    assert_eq!(find_drop(&[0.5; 3000], hz, 120.0, 0.0, 0.0), None, "no jump, no drop");
    assert_eq!(find_drop(&lv, hz, 0.0, 0.0, 0.0), None);
    // a phonk shape: the intro lifts at 8 s (a big jump, but into a mid level), a breakdown, then the
    // drop at 42 s into the loudest part - the drop wins, and a build of 10 s must fit before it
    let phonk: Vec<f32> = (0..7000)
        .map(|i| match i as f64 / hz {
            t if t < 8.0 => 0.05,
            t if t < 30.0 => 0.5,
            t if t < 42.0 => 0.35,
            _ => 0.9,
        })
        .collect();
    assert_eq!(find_drop(&phonk, hz, 120.0, 0.0, 0.0), Some(42.0));
    assert_eq!(find_drop(&lv, hz, 120.0, 0.0, 100.0), Some(24.0), "nothing that late: the best anywhere");
}

#[test]
fn a_sound_is_found_by_its_hint_and_lined_up_on_its_hit() {
    let lv = [0.0, 0.3, 0.3, 0.2, 0.1, 0.1, 0.4, 0.9, 1.0, 0.6];
    assert!((loudest_onset(&lv, 10.0) - 0.7).abs() < 1e-9, "walks back to where it climbed past half");
    assert_eq!(loudest_onset(&[], 10.0), 0.0);
    // the GTA sound: a louder impact at 0.2 s, then the sting climbing from 2.45 s - the sting is the hit
    let gta: Vec<f32> = (0..400)
        .map(|i| match i {
            20..=60 => 0.5,
            245 => 0.25,
            246..=255 => 0.42,
            256..=300 => 0.15,
            _ => 0.02,
        })
        .collect();
    assert!((loudest_onset(&gta, 100.0) - 2.45).abs() < 1e-9, "{}", loudest_onset(&gta, 100.0));
    let dir = std::env::temp_dir().join(format!("se-tpl-sounds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("wasted")).unwrap();
    std::fs::write(dir.join("wasted").join("wasted-green-screen.mp4"), b"x").unwrap();
    std::fs::write(dir.join("boom.wav"), b"x").unwrap();
    std::fs::write(dir.join("notes wasted.txt"), b"x").unwrap();
    let roots = [dir.clone()];
    assert_eq!(find_sound("Wasted", &roots).unwrap().file_name().unwrap(), "wasted-green-screen.mp4");
    std::fs::write(dir.join("GTA Wasted.mp3"), b"x").unwrap();
    assert_eq!(find_sound("wasted", &roots).unwrap().file_name().unwrap(), "GTA Wasted.mp3", "a sound file before a video");
    assert!(find_sound("vine boom", &roots).is_none() && find_sound("", &roots).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_saved_template_keeps_its_bus_automation() {
    let mut p = Project::new();
    let a = p.add_asset(asset("C:/song.mp3", ClipKind::Audio, 20.0, 1));
    let id = p.insert_asset_clips(a, 2.0, None)[0];
    let bus = p.add_bus("Muffle");
    let mut f = AudioFilter::new(FilterKind::LowPass);
    f.params[0].keys = vec![Keyframe { t: 1.0, v: 400.0, ease: Ease::Hold }, Keyframe { t: 3.0, v: 20000.0, ease: Ease::Hold }];
    p.bus_mut(bus).unwrap().filters.push(f);
    p.clip_mut(id).unwrap().bus = bus;
    let d = capture(&p, &[id]);
    assert_eq!(d.buses.len(), 1);
    assert_eq!(d.buses[0].filters[0].params[0].keys[0].t, -1.0, "keys move with the clips to template time");
    let mut q = Project::new();
    let placed = apply(&mut q, d, 10.0, &[]);
    let b = q.bus(q.clip(placed[0]).unwrap().bus).unwrap();
    assert_eq!((b.name.as_str(), b.filters[0].params[0].keys[1].t), ("Muffle", 11.0));
}

#[test]
fn retiming_keeps_every_key_inside_its_clip() {
    for name in EDITS {
        let mut d = builtin(name, 1920, 1080).unwrap();
        let before = d.duration();
        d.retime(0.8);
        assert!((d.duration() - before * 0.8).abs() < 1e-9);
        for c in &d.clips {
            for a in c.all_animated() {
                assert!(a.keys.iter().all(|k| k.t >= -1e-9 && k.t <= c.duration + 1e-6), "{name}/{}", c.name);
            }
            assert!(c.blocks.iter().all(|b| b.end() <= c.duration + 1e-6));
            assert!(c.effects.iter().all(|e| e.start <= c.duration + 1e-9));
        }
    }
}

// ---- a template dropped on the timeline: one sequence clip of containers holding its own footage ----

/// The sample footage of `d` as project assets (sounds for .m4a, silent video otherwise), file -> asset.
fn sample_media(p: &mut Project, d: &TemplateData) -> Vec<(String, Id)> {
    let mut out: Vec<(String, Id)> = Vec::new();
    for s in &d.samples {
        if out.iter().any(|o| o.0 == s.file) {
            continue;
        }
        let path = format!("C:/lib/{}/{}", d.media, s.file);
        let a = if s.file.ends_with(".m4a") { asset(&path, ClipKind::Audio, 30.0, 1) } else { asset(&path, ClipKind::Video, 6.0, 0) };
        out.push((s.file.clone(), p.add_asset(a)));
    }
    out
}

#[test]
fn every_edit_template_brings_footage_for_every_slot() {
    for name in EDITS.into_iter().chain(["Wasted"]) {
        let d = builtin(name, 1920, 1080).unwrap();
        assert!(d.media.starts_with("templates/"), "{name}: {}", d.media);
        for s in d.slot_list() {
            assert!(d.samples.iter().any(|x| x.clip == s.clip), "{name}/{}: no sample footage", s.name);
        }
        // the fixed parts bring their media too: nothing to ask for
        for c in d.clips.iter().filter(|c| !c.container && matches!(c.kind, ClipKind::Video | ClipKind::Audio)) {
            assert!(d.samples.iter().any(|x| x.clip == c.id), "{name}/{}: a fixed part without its media", c.name);
        }
        // the shipped files are in the content library (not in the exe)
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join(&d.media);
        for x in d.samples.iter().filter(|x| !x.local) {
            assert!(dir.join(&x.file).is_file(), "{name}: {} is missing from {}", x.file, dir.display());
        }
    }
    // the phonk edit is not shipped: nothing of it is in the library
    assert!(!BUILTIN_NAMES.iter().any(|n| n.contains("Phonk")));
    let phonk = phonk_edit(1920, 1080);
    assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join(&phonk.media).exists());
    {
    }
}

#[test]
fn a_dropped_template_is_one_sequence_clip_of_filled_containers() {
    let mut p = Project::new();
    let d = phonk_edit(1920, 1080);
    let media = sample_media(&mut p, &d);
    let id = place_nested(&mut p, "Phonk Synced Edit", d.clone(), 3.0, None, &media).unwrap();
    // on the timeline: the sequence clip (its sound half linked to it) and nothing else
    let main: Vec<&Clip> = p.all_clips().map(|(_, c)| c).collect();
    assert!(main.iter().all(|c| c.kind == ClipKind::Sequence), "only the sequence clip lands on the timeline");
    assert_eq!(main.iter().filter(|c| p.track_of(c.id).is_some_and(|t| p.tracks[t].kind == TrackKind::Video)).count(), 1);
    let c = p.clip(id).unwrap();
    assert_eq!((c.start, c.kind), (3.0, ClipKind::Sequence));
    assert!((c.duration - d.duration()).abs() < 1e-6, "as long as the template");
    // inside: one container per slot, each with its footage; no placeholder cards are left
    let seq = c.sequence;
    assert!(p.open_sequence(seq));
    let slots = d.slot_list();
    let inside: Vec<&Clip> = p.all_clips().map(|(_, c)| c).filter(|c| c.container).collect();
    assert_eq!(inside.iter().filter(|c| c.kind != ClipKind::Audio).count(), slots.len());
    assert!(inside.iter().all(|c| c.asset != 0 && c.kind == ClipKind::Video), "every slot is a video shot holding footage");
    // the fixed parts are filled and locked: the WASTED shot, its sound, the song
    let fixed = |n: &str| p.all_clips().map(|(_, c)| c).find(|c| c.name == n).unwrap_or_else(|| panic!("{n}"));
    let path = |c: &Clip| p.asset(c.asset).map(|a| a.path.clone()).unwrap_or_default();
    let (wasted, sound, song) = (fixed("Wasted shot"), fixed("Wasted sound"), fixed("Song"));
    assert_eq!(path(wasted), format!("C:/lib/{}/hit-by-truck.mp4", d.media));
    assert_eq!(path(sound), format!("C:/lib/{}/wasted.m4a", d.media));
    assert_eq!(path(song), format!("C:/lib/{}/song.m4a", d.media));
    assert!([wasted, sound, song].iter().all(|c| !c.container && c.src_in == 0.0), "fixed, from the start of their trimmed file");
    assert!(!p.all_clips().any(|(_, c)| c.kind == ClipKind::Shape && c.name.ends_with(" card")), "no empty-slot cards");
    assert!(p.tracks.iter().all(|t| !t.clips.is_empty()), "no empty track where the cards were");
    // the media is a bin of its own
    assert!(media.iter().all(|m| p.asset(m.1).unwrap().folder == "Templates/Phonk Synced Edit"));
}

#[test]
fn reused_footage_is_linked_and_replaces_together() {
    let mut p = Project::new();
    let d = builtin("Meme Cut", 1920, 1080).unwrap();
    let media = sample_media(&mut p, &d);
    let id = place_nested(&mut p, "Meme Cut", d, 0.0, None, &media).unwrap();
    let seq = p.clip(id).unwrap().sequence;
    p.open_sequence(seq);
    let shots = |p: &Project| -> Vec<Clip> {
        let mut v: Vec<Clip> = p.all_clips().map(|(_, c)| c.clone()).filter(|c| c.container && c.kind != ClipKind::Audio).collect();
        v.sort_by(|a, b| a.start.total_cmp(&b.start));
        v
    };
    let before = shots(&p);
    // six shots from three files: two slots per file, sharing one media link, each a different stretch
    let first = before[0].clone();
    let twins: Vec<&Clip> = before.iter().filter(|c| c.asset == first.asset).collect();
    assert_eq!(twins.len(), 2);
    assert!(first.media_link != 0 && twins.iter().all(|c| c.media_link == first.media_link));
    assert!(before.iter().all(|c| c.asset == first.asset || c.media_link != first.media_link), "other footage is not in the link");
    assert!((twins[0].src_in - twins[1].src_in).abs() > 1e-6, "a reused shot plays a different stretch in each slot");
    // replacing it once changes both; each keeps its in point, speed, effects and timing
    let mine = p.add_asset(asset("C:/mine/long.mp4", ClipKind::Video, 60.0, 0));
    assert_eq!(p.replace_slot_media(first.id, mine, true), 2);
    let after = shots(&p);
    for (b, a) in before.iter().zip(&after) {
        if b.media_link == first.media_link {
            assert_eq!(a.asset, mine);
            assert_eq!((a.src_in, a.speed, a.start, a.duration, a.effects.len(), a.blocks.len()), (b.src_in, b.speed, b.start, b.duration, b.effects.len(), b.blocks.len()));
        } else {
            assert_eq!(a.asset, b.asset, "{}: not linked, untouched", b.name);
        }
    }
    // unlinked, a slot changes alone
    assert!(p.unlink_slot_media(first.id));
    let other = p.add_asset(asset("C:/mine/other.mp4", ClipKind::Video, 60.0, 0));
    assert_eq!(p.replace_slot_media(first.id, other, true), 1);
    let twin = twins.iter().find(|c| c.id != first.id).unwrap().id;
    assert_eq!(p.clip(twin).unwrap().asset, mine);
}

#[test]
fn a_second_drop_reuses_the_template_bin() {
    let mut p = Project::new();
    let d = builtin("Velocity Edit", 1920, 1080).unwrap();
    let media = sample_media(&mut p, &d);
    let a = place_nested(&mut p, "Velocity Edit", d.clone(), 0.0, None, &media).unwrap();
    let n = p.assets.len();
    // the app finds each file by its path the second time: the same assets
    let again = sample_media(&mut p, &d);
    assert_eq!(again, media);
    let b = place_nested(&mut p, "Velocity Edit", d, 20.0, None, &again).unwrap();
    assert_eq!(p.assets.len(), n, "no duplicate media");
    let (sa, sb) = (p.clip(a).unwrap().sequence, p.clip(b).unwrap().sequence);
    assert_ne!(sa, sb, "each drop is its own sequence");
    assert_eq!(p.sequence(sb).unwrap().name, "Velocity Edit 2");
    let bins: std::collections::HashSet<&str> = p.assets.iter().map(|a| a.folder.as_str()).collect();
    assert_eq!(bins.into_iter().collect::<Vec<_>>(), vec!["Templates/Velocity Edit"]);
}

#[test]
fn the_wasted_moment_looks_like_the_game() {
    let d = builtin("Wasted", 1920, 1080).unwrap();
    assert!(d.sync.is_none() && !d.clips.iter().any(|c| c.audio_role == AudioRole::Music), "no music: it ships");
    let slots = d.slot_list();
    assert_eq!(slots.len(), 1, "one slot: your shot");
    let shot = d.clip(slots[0].clip).unwrap();
    let hit = 1.5;
    // under the card: grey in a quarter second, a slight slow-mo, a slow zoom in
    let grey = shot.effects.iter().find(|e| e.kind == EffectKind::Color).unwrap();
    assert!(grey.params[2].at(hit - 0.1) > 0.99 && grey.params[2].at(hit + 0.25) < 0.01);
    let speed = shot.speed_curve.at(hit + 1.0);
    assert!(speed > 0.4 && speed < 0.7, "a slight slow-mo, not a freeze: {speed}");
    assert!(shot.scale.at(shot.duration - 0.05) > shot.scale.at(hit + 0.1) * 1.1, "it keeps zooming in");
    // the card: a dark band about a fifth of the frame, tilted a degree, the lowercase word in the game's
    // font (Pricedown when installed, else Impact), red with a dark outline, white later
    let band = d.clips.iter().find(|c| c.name == "Wasted band").unwrap();
    let sh = band.shape.as_ref().unwrap();
    assert!((sh.h.value * 2.0 / 1080.0 - 0.21).abs() < 0.01 && sh.w.value * 2.0 >= 1920.0 && sh.fill[3] > 180);
    assert!((band.start - hit).abs() < 1e-9 && (band.rotation.value + 1.0).abs() < 1e-9);
    let words: Vec<&Clip> = d.clips.iter().filter(|c| c.text.as_ref().is_some_and(|t| t.text == "wasted")).collect();
    assert_eq!(words.len(), 2);
    let (red, white) = (words[0].text.as_ref().unwrap(), words[1].text.as_ref().unwrap());
    assert!(red.font.starts_with("Pricedown") && red.font.contains("Impact"));
    assert_eq!((red.color, white.color), ([245, 0, 4, 255], [255, 255, 255, 255]));
    assert!(red.outline_color[..3] == [0, 0, 0] && red.outline_width.value > 0.0);
    assert!(words[1].start > words[0].start + 1.0, "it turns white later");
    // the sting is the user's own sound: never shipped, and left out without it
    let sting = d.samples.iter().find(|s| d.clip(s.clip).is_some_and(|c| c.name == "Wasted sound")).unwrap();
    assert!(sting.local);
    let mut p = Project::new();
    let a = p.add_asset(asset("C:/lib/templates/wasted/hit-by-truck.mp4", ClipKind::Video, 6.0, 0));
    let mut d2 = d.clone();
    fill_fixed(&mut d2, &p, &[("hit-by-truck.mp4".into(), a)]);
    assert!(!d2.clips.iter().any(|c| c.name == "Wasted sound"), "no sound file: no empty clip");
    assert_eq!(d2.layers.len(), d2.clips.len());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join(&d.media);
    assert!(!dir.join("wasted.m4a").exists(), "the game's sound is not in the library");
}

/// Save the phonk edit to a Gallery as one of your own templates (its media in your own folder, never
/// shipped): `SE_GALLERY=<gallery folder> cargo test save_local_phonk -- --ignored`.
#[test]
#[ignore]
fn save_local_phonk() {
    let Ok(root) = std::env::var("SE_GALLERY") else { return };
    let d = phonk_edit(1920, 1080);
    let item = gallery::save(std::path::Path::new(&root), gallery::Kind::Template, "Phonk Synced Edit", &["phonk".into(), "beat edit".into()], serde_json::to_value(&d).unwrap(), gallery::Collide::Replace).unwrap();
    println!("saved {}", item.path.display());
}
