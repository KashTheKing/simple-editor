//! ---- feat-retro-2000s-pack ----
//! Make Slideshow, the Classic 2000s way: the photos and clips you picked, in order, on one video track;
//! each photo drifts in a slow pan-and-zoom (alternately in and out), a classic transition on every cut
//! (one you chose, or a random one each time), a blue title card first and a credits card last, and
//! optional background music under it all - with the photos' length fitted to the song if asked.
//!
//! Everything it makes is ordinary clips, keyframe blocks and transitions the user can still edit.
//! `make` only edits the project; the caller wraps it in one undo step (`slideshow.make` is a Mutate tool).

use super::retro;
use crate::model::*;

/// The shortest a photo may get when fitting to the music.
pub const MIN_PHOTO: f64 = 1.0;

/// What to build.
#[derive(Clone, Debug)]
pub struct Opts {
    /// Photos and clips (asset ids), in order.
    pub items: Vec<Id>,
    /// Seconds per photo (ignored when fitting to the music).
    pub photo_secs: f64,
    /// A pack transition id (`classic-heart`), "random" (a different classic one on each cut) or "none".
    pub transition: String,
    pub transition_secs: f64,
    /// The title card's text; None = no title card.
    pub title: Option<String>,
    pub title_secs: f64,
    /// The credits card's text; None = no credits.
    pub credits: Option<String>,
    pub credits_secs: f64,
    /// Background music (an asset with sound).
    pub music: Option<Id>,
    /// Stretch or squeeze the photos so the slideshow ends with the music.
    pub fit_to_music: bool,
    /// The slow pan-and-zoom on photos.
    pub ken_burns: bool,
    /// Timeline seconds the slideshow starts at.
    pub at: f64,
    /// Seed of the random transition choice (the same seed makes the same slideshow).
    pub seed: u64,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            photo_secs: 5.0,
            transition: "random".into(),
            transition_secs: 1.0,
            title: Some("My Slideshow".into()),
            title_secs: 4.0,
            credits: Some(retro::CREDITS.into()),
            credits_secs: 8.0,
            music: None,
            fit_to_music: false,
            ken_burns: true,
            at: 0.0,
            seed: 1,
        }
    }
}

/// The lengths `make` lays out: the title, each item, the credits.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub title: f64,
    pub items: Vec<f64>,
    pub credits: f64,
    /// Seconds per photo actually used.
    pub photo: f64,
    /// False when the music is too short for every photo to get `MIN_PHOTO` (or there is no photo to
    /// stretch): the slideshow then runs past the song's end.
    pub fits: bool,
}

impl Timing {
    pub fn total(&self) -> f64 {
        self.title + self.items.iter().sum::<f64>() + self.credits
    }
}

/// Lay out the lengths. `items`: None for a photo, Some(seconds) for a clip (it keeps its own length).
/// Clips abut and each transition is centred on its cut, so the total is just the sum. With `music`
/// (its length, when fitting), the photos share what the title, credits and clips leave of it.
pub fn timing(items: &[Option<f64>], photo: f64, title: f64, credits: f64, music: Option<f64>) -> Timing {
    let photos = items.iter().filter(|i| i.is_none()).count();
    let fixed: f64 = title + credits + items.iter().flatten().sum::<f64>();
    let mut photo = photo.max(MIN_PHOTO);
    let mut fits = true;
    if let Some(m) = music {
        if photos == 0 {
            fits = (fixed - m).abs() < 1e-6;
        } else {
            let each = (m - fixed) / photos as f64;
            fits = each >= MIN_PHOTO - 1e-9;
            photo = each.max(MIN_PHOTO);
        }
    }
    Timing { title, items: items.iter().map(|i| i.unwrap_or(photo)).collect(), credits, photo, fits }
}

/// What `make` did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Every clip it added (cards, texts, items, music).
    pub clips: Vec<Id>,
    /// The transition names on the cuts, in order.
    pub transitions: Vec<String>,
    pub duration: f64,
    pub photo_secs: f64,
    pub fits: bool,
}

/// A tiny deterministic random sequence (xorshift), so a seed makes the same choices every time.
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: usize) -> usize {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n.max(1) as u64) as usize
    }
}

/// The pan-and-zoom on photo `i`: in and to the upper left, then out and to the right, alternately.
fn ken_burns(i: usize) -> KeyBlock {
    let k = |f: f64, v: BlockVal| BlockKey { f, v, ease: Ease::Linear };
    let tr = |prop: &str, keys: Vec<BlockKey>| BlockTrack { prop: prop.into(), keys };
    let (s0, s1, dx, dy) = if i % 2 == 0 { (1.0, 1.15, -0.03, -0.02) } else { (1.15, 1.0, 0.03, 0.01) };
    KeyBlock {
        name: "Ken Burns".into(),
        dur: 5.0,
        tracks: vec![
            tr("scale", vec![k(0.0, BlockVal::Mul(s0)), k(1.0, BlockVal::Mul(s1))]),
            tr("x", vec![k(0.0, BlockVal::Add(0.0)), k(1.0, BlockVal::Add(dx))]),
            tr("y", vec![k(0.0, BlockVal::Add(0.0)), k(1.0, BlockVal::Add(dy))]),
        ],
        gen: None,
        proc: None,
    }
}

/// The lowest video track above `below` with [start, start + dur) free, or a new one on top.
fn track_above(p: &mut Project, below: usize, start: f64, dur: f64) -> usize {
    let free = p.video_tracks().into_iter().find(|&i| i > below && p.tracks[i].fits(start, dur, &[]));
    free.unwrap_or_else(|| p.add_track(TrackKind::Video))
}

/// The pack transition for the next cut: the chosen one, or a random classic (never the same twice
/// in a row).
fn pick(o: &Opts, rng: &mut Rng, last: &mut usize) -> Option<(&'static str, &'static str)> {
    match o.transition.as_str() {
        "none" | "" => None,
        "random" => {
            let n = retro::TRANSITIONS.len();
            let mut i = rng.next(n);
            if i == *last {
                i = (i + 1) % n;
            }
            *last = i;
            Some(retro::TRANSITIONS[i])
        }
        id => retro::TRANSITIONS.iter().find(|(i, _)| *i == id).copied(),
    }
}

/// Build the slideshow into `p`. Errors (nothing changed) when there is nothing to show, an item is
/// not a picture or video in the project, or the transition / music is unknown.
pub fn make(p: &mut Project, o: &Opts) -> Result<Report, String> {
    if o.items.is_empty() {
        return Err("pick some photos or clips for the slideshow first".into());
    }
    let mut kinds = Vec::new();
    for &id in &o.items {
        let a = p.asset(id).ok_or_else(|| format!("no media with id {id} in the project"))?;
        match a.kind {
            ClipKind::Image => kinds.push(None),
            ClipKind::Video => kinds.push(Some(a.range.map(|(s, e)| e - s).unwrap_or(a.duration).max(MIN_CLIP))),
            _ => return Err(format!("\"{}\" is not a photo or a video clip", a.name())),
        }
    }
    if !matches!(o.transition.as_str(), "random" | "none" | "") && retro::transition(&o.transition).is_none() {
        let ids: Vec<&str> = retro::TRANSITIONS.iter().map(|t| t.0).collect();
        return Err(format!("unknown transition '{}': use random, none or one of {}", o.transition, ids.join(", ")));
    }
    let music = match o.music {
        Some(id) => {
            let a = p.asset(id).ok_or_else(|| format!("no music with id {id} in the project"))?;
            if a.audio_streams.is_empty() {
                return Err(format!("\"{}\" has no sound", a.name()));
            }
            Some((id, a.duration))
        }
        None => None,
    };
    let title = if o.title.is_some() { o.title_secs.max(MIN_CLIP) } else { 0.0 };
    let credits = if o.credits.is_some() { o.credits_secs.max(MIN_CLIP) } else { 0.0 };
    let fit = music.filter(|_| o.fit_to_music).map(|(_, d)| d);
    let tm = timing(&kinds, o.photo_secs, title, credits, fit);
    let total = tm.total();
    let (w, h) = (p.width as f64, p.height as f64);
    let at = o.at.max(0.0);
    let main = p.find_free_track(TrackKind::Video, at, total, None);
    let mut rep = Report { duration: total, photo_secs: tm.photo, fits: tm.fits, ..Default::default() };
    // the run of pictures on the main track, left to right (each right one gets a transition)
    let mut run: Vec<(Id, f64)> = Vec::new();
    let mut t = at;
    let card = |p: &mut Project, text: Clip, dur: f64, t: f64, rep: &mut Report| -> Id {
        let mut bg = retro::blue_card(dur, w, h);
        bg.id = p.new_id();
        bg.start = t;
        let bid = bg.id;
        p.tracks[main].clips.push(bg);
        p.tracks[main].sort();
        let up = track_above(p, main, t, dur);
        let mut text = text;
        text.id = p.new_id();
        text.start = t;
        rep.clips.extend([bid, text.id]);
        p.tracks[up].clips.push(text);
        p.tracks[up].sort();
        bid
    };
    if let Some(text) = &o.title {
        let mut c = retro::text_clip("Title", text, 110.0, title);
        if let Some(b) = find_block(&[], "Fade In") {
            c.place_block(&b, 0.0, Some(0.6_f64.min(title / 3.0)), w, h);
        }
        if let Some(b) = find_block(&[], "Fade Out") {
            let d = 0.6_f64.min(title / 3.0);
            c.place_block(&b, title - d, Some(d), w, h);
        }
        let id = card(p, c, title, t, &mut rep);
        run.push((id, title));
        t += title;
    }
    let mut n_photo = 0;
    for (i, &aid) in o.items.iter().enumerate() {
        let dur = tm.items[i];
        let src = p.asset(aid).and_then(|a| a.range).map(|r| r.0).unwrap_or(0.0);
        let ids = p.insert_asset_clips_ranged(aid, t, Some(main), None, Some((src, src + dur)));
        let Some(&vid) = ids.iter().find(|&&id| p.clip(id).is_some_and(|c| c.kind != ClipKind::Audio)) else {
            continue;
        };
        if let Some(c) = p.clip_mut(vid) {
            c.duration = dur;
        }
        if kinds[i].is_none() && o.ken_burns {
            p.place_key_block(vid, &ken_burns(n_photo), 0.0, Some(dur));
            n_photo += 1;
        }
        rep.clips.extend(ids);
        run.push((vid, dur));
        t += dur;
    }
    if let Some(text) = &o.credits {
        let c = retro::credits_roll(text, credits, h);
        let id = card(p, c, credits, t, &mut rep);
        run.push((id, credits));
        t += credits;
        p.add_edge_transition(id, TransitionKind::CrossFade, 1.0_f64.min(credits / 4.0), true);
    }
    // a classic transition on every cut, never longer than half of the shorter neighbour
    let mut rng = Rng(o.seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut last = usize::MAX;
    for pair in run.windows(2) {
        let ((_, ld), (right, rd)) = (pair[0], pair[1]);
        let Some((_, src)) = pick(o, &mut rng, &mut last) else { break };
        let d = o.transition_secs.max(MIN_CLIP).min(0.5 * ld.min(rd));
        if let Some(tid) = p.add_transition(right, TransitionKind::Shader, d) {
            if let Some(tr) = p.transition_mut(tid) {
                tr.shader = src.to_string();
            }
            rep.transitions.push(crate::engine::shaderlib::parse_meta(src).name);
        }
    }
    // the music under it all, from the first frame, cut at the end with a fade
    if let Some((mid, mdur)) = music {
        let len = mdur.min(total).max(MIN_CLIP);
        let ids = p.insert_asset_clips_ranged(mid, at, None, None, Some((0.0, len)));
        let sound: Vec<Id> = ids.iter().copied().filter(|&id| p.clip(id).is_some_and(|c| c.kind == ClipKind::Audio)).collect();
        for id in sound {
            p.add_edge_transition(id, TransitionKind::CrossFade, 2.0_f64.min(len / 4.0), true);
        }
        rep.clips.extend(ids);
    }
    debug_assert!((t - at - total).abs() < 1e-6);
    p.tidy();
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test asset (from JSON, so it keeps up with new optional fields); `n` makes the path unique.
    fn asset(n: u32, kind: ClipKind, dur: f64, sound: bool) -> Asset {
        let streams = if sound { serde_json::json!([{"index": 0, "channels": 2}]) } else { serde_json::json!([]) };
        serde_json::from_value(serde_json::json!({
            "id": 0, "path": format!("C:/media/{n}.{}", if kind == ClipKind::Image { "jpg" } else { "mp4" }),
            "kind": kind, "duration": dur, "width": 1920, "height": 1080, "fps": 30.0, "audio_streams": streams,
        }))
        .unwrap()
    }

    /// A blank 1080p project with three photos, a 6 s video with sound and a 40 s song.
    fn project() -> (Project, Vec<Id>, Id) {
        let mut p = Project::new();
        p.width = 1920;
        p.height = 1080;
        let a = p.add_asset(asset(1, ClipKind::Image, 0.0, false));
        let b = p.add_asset(asset(2, ClipKind::Image, 0.0, false));
        let v = p.add_asset(asset(3, ClipKind::Video, 6.0, true));
        let c = p.add_asset(asset(4, ClipKind::Image, 0.0, false));
        let song = p.add_asset(asset(5, ClipKind::Audio, 40.0, true));
        (p, vec![a, b, v, c], song)
    }

    #[test]
    fn timing_without_music_uses_the_photo_length() {
        let t = timing(&[None, Some(6.0), None], 5.0, 4.0, 8.0, None);
        assert_eq!(t.items, vec![5.0, 6.0, 5.0]);
        assert_eq!(t.total(), 4.0 + 16.0 + 8.0);
        assert!(t.fits);
    }

    #[test]
    fn timing_fits_the_photos_to_the_music() {
        // 40 s song: 4 title + 8 credits + 6 video leave 22 s for 3 photos
        let t = timing(&[None, None, Some(6.0), None], 5.0, 4.0, 8.0, Some(40.0));
        assert!((t.photo - 22.0 / 3.0).abs() < 1e-9, "{}", t.photo);
        assert!((t.total() - 40.0).abs() < 1e-9, "ends with the song: {}", t.total());
        assert!(t.fits);
        // a song too short: every photo gets the minimum and the show runs past the song
        let t = timing(&[None, None], 5.0, 4.0, 8.0, Some(10.0));
        assert_eq!(t.photo, MIN_PHOTO);
        assert!(!t.fits && t.total() > 10.0);
        // only clips: nothing to stretch
        assert!(!timing(&[Some(3.0)], 5.0, 0.0, 0.0, Some(9.0)).fits);
        assert!(timing(&[Some(3.0)], 5.0, 0.0, 0.0, Some(3.0)).fits);
    }

    #[test]
    fn make_lays_out_cards_photos_transitions_and_music() {
        let (mut p, items, song) = project();
        let o = Opts { items: items.clone(), music: Some(song), fit_to_music: true, ..Default::default() };
        let r = make(&mut p, &o).unwrap();
        assert!((r.duration - 40.0).abs() < 1e-6, "fitted to the song: {}", r.duration);
        assert!(r.fits);
        // one main track: blue card, 4 items, blue card - abutting, in order
        let main = p.tracks.iter().find(|t| t.kind == TrackKind::Video && t.clips.len() == 6).expect("one main track");
        let mut t = 0.0;
        for c in &main.clips {
            assert!((c.start - t).abs() < 1e-6, "{} starts at {} not {t}", c.name, c.start);
            t += c.duration;
        }
        assert!((t - 40.0).abs() < 1e-6);
        let (first, last) = (&main.clips[0], &main.clips[5]);
        assert_eq!(first.shape.as_ref().unwrap().fill, retro::BLUE);
        assert_eq!(last.shape.as_ref().unwrap().fill, retro::BLUE);
        assert_eq!(first.shape.as_ref().unwrap().w.value, 1920.0, "the card fills the frame");
        // a classic transition on each of the 5 cuts, every one a pack shader
        let cuts: Vec<&Transition> = main.transitions.iter().filter(|t| t.edge == TransitionEdge::Cut).collect();
        assert_eq!(cuts.len(), 5);
        assert_eq!(r.transitions.len(), 5);
        for tr in &cuts {
            assert_eq!(tr.kind, TransitionKind::Shader);
            assert!(retro::TRANSITIONS.iter().any(|(_, s)| *s == tr.shader));
            assert!(tr.duration <= 1.0 + 1e-9);
        }
        // photos pan and zoom, the video does not
        let photos: Vec<&Clip> = main.clips.iter().filter(|c| c.kind == ClipKind::Image).collect();
        assert_eq!(photos.len(), 3);
        assert!(photos.iter().all(|c| c.scale.is_animated() && c.blocks.iter().any(|b| b.name == "Ken Burns")));
        assert!(!main.clips.iter().any(|c| c.kind == ClipKind::Video && c.scale.is_animated()));
        // the title text sits above the card
        let ti = p.tracks.iter().position(|t| std::ptr::eq(t, main)).unwrap();
        let (tti, title) = p.all_clips().find(|(_, c)| c.kind == ClipKind::Text && c.start == 0.0).unwrap();
        assert!(tti > ti, "title text on a track above the card");
        assert_eq!(title.text.as_ref().unwrap().text, "My Slideshow");
        // the song from 0, cut at the end, with a fade out
        let music: Vec<&Clip> = p.all_clips().map(|(_, c)| c).filter(|c| c.asset == song).collect();
        assert_eq!(music.len(), 1);
        assert_eq!(music[0].start, 0.0);
        assert!((music[0].duration - 40.0).abs() < 1e-6);
    }

    #[test]
    fn make_is_seeded_and_honours_a_chosen_transition() {
        let (mut a, items, _) = project();
        let (mut b, _, _) = project();
        let o = Opts { items: items.clone(), seed: 7, ..Default::default() };
        assert_eq!(make(&mut a, &o).unwrap().transitions, make(&mut b, &o).unwrap().transitions, "same seed, same show");
        let (mut c, _, _) = project();
        let r = make(&mut c, &Opts { items: items.clone(), transition: "classic-heart".into(), title: None, credits: None, ..Default::default() }).unwrap();
        assert_eq!(r.transitions, vec!["Heart"; 3]);
        assert!((r.duration - 21.0).abs() < 1e-6, "3 photos x 5 s + a 6 s clip: {}", r.duration);
        let (mut d, _, _) = project();
        let r = make(&mut d, &Opts { items, transition: "none".into(), ..Default::default() }).unwrap();
        assert!(r.transitions.is_empty());
    }

    #[test]
    fn make_refuses_bad_input_and_changes_nothing() {
        let (mut p, items, song) = project();
        let before = p.to_json();
        assert!(make(&mut p, &Opts::default()).is_err(), "nothing picked");
        assert!(make(&mut p, &Opts { items: vec![song], ..Default::default() }).is_err(), "a song is not a picture");
        assert!(make(&mut p, &Opts { items: items.clone(), transition: "nope".into(), ..Default::default() }).is_err());
        assert!(make(&mut p, &Opts { items, music: Some(9999), ..Default::default() }).is_err());
        assert_eq!(p.to_json(), before);
    }
}
