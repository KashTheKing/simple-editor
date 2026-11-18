//! ---- ws:music-burn ----
//! "Make visualizer video": songs (a playlist or a selection) laid back to back on the timeline, each
//! with a visualizer layer of the chosen style (`engine::visualizer::add`, so it is the same linked,
//! baked layer the clip menu makes and stays compatible with visualizer Gallery assets), its cover art
//! and a title card.

use crate::engine::visualizer::{self, Style};
use crate::media::Backend;
use crate::model::*;

/// What one song brings: its library asset, the cover picture (an image asset already in the library,
/// None = no cover) and the title card text ("" = none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub asset: Id,
    pub cover: Option<Id>,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MixOptions {
    /// Seconds of silence between songs.
    pub gap: f64,
    /// Seconds songs overlap (fade out / in).
    pub crossfade: f64,
    pub cover: bool,
    pub title_card: bool,
}

/// The clips made, per song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Made {
    pub audio: Id,
    pub visualizer: Id,
    pub cover: Option<Id>,
    pub title: Option<Id>,
}

/// Build the video from `at`. Nothing is left behind on failure (the caller snapshots for undo anyway).
pub fn build(p: &mut Project, songs: &[Song], at: f64, style: &Style, opts: &MixOptions, backend: Backend) -> Result<Vec<Made>, String> {
    if songs.is_empty() {
        return Err("no songs: pick songs in the Library or a playlist with songs".into());
    }
    let before = p.clone();
    match build_inner(p, songs, at, style, opts, backend) {
        Ok(m) => Ok(m),
        Err(e) => {
            *p = before;
            Err(e)
        }
    }
}

fn build_inner(p: &mut Project, songs: &[Song], at: f64, style: &Style, opts: &MixOptions, backend: Backend) -> Result<Vec<Made>, String> {
    let (w, h) = (p.width as f64, p.height as f64);
    let mut t = at.max(0.0);
    let mut out = Vec::new();
    let mut prev: Option<Id> = None;
    for s in songs {
        let a = p.asset(s.asset).ok_or("a song is no longer in the library")?.clone();
        if a.audio_streams.is_empty() {
            return Err(format!("{} has no sound", a.name()));
        }
        let ids = p.insert_asset_clips(a.id, t, None);
        let audio = ids
            .iter()
            .copied()
            .find(|&c| p.clip(c).is_some_and(|c| c.kind == ClipKind::Audio))
            .ok_or("the song has no sound to place")?;
        // a music video's own picture is not wanted under the visualizer
        let pictures: Vec<Id> = ids.iter().copied().filter(|&c| c != audio).collect();
        if !pictures.is_empty() {
            p.delete_clips(&pictures, false);
        }
        let len = p.clip(audio).map_or(a.duration, |c| c.duration);
        let xf = opts.crossfade.min(len / 2.0);
        if xf > 0.0 {
            if let Some(c) = prev.and_then(|id| p.clip_mut(id)) {
                c.fade_out = xf;
            }
            if prev.is_some() {
                if let Some(c) = p.clip_mut(audio) {
                    c.fade_in = xf;
                }
            }
        }
        let vis = visualizer::add(p, audio, style, backend)?;
        settle_low(p, vis); // every song's visualizer on the same (lowest free) track, its layers above
        let mut made = Made { audio, visualizer: vis, cover: None, title: None };
        if opts.cover {
            if let Some(img) = s.cover.and_then(|id| p.asset(id).cloned()) {
                let c = p.insert_asset_clips(img.id, t, top_video(p));
                if let Some(&cid) = c.first() {
                    if let Some(clip) = p.clip_mut(cid) {
                        clip.duration = len;
                        clip.name = "Cover".into();
                        // 45 % of the frame high, a little above centre (the compositor fits a picture into the
                        // frame first, so the scale is that share whatever the picture's own size)
                        clip.scale = Animated::new(0.45);
                        clip.y = Animated::new(-0.06 * h);
                        if xf > 0.0 && prev.is_some() {
                            clip.opacity = Animated::new(1.0);
                            clip.opacity.key_at(0.0, 0.0);
                            clip.opacity.key_at(xf, 1.0);
                        }
                    }
                    lift_above(p, cid, vis);
                    made.cover = Some(cid);
                }
            }
        }
        if opts.title_card && !s.title.trim().is_empty() {
            let id = p.add_text_clip(t, len);
            if let Some(c) = p.clip_mut(id) {
                c.name = "Title".into();
                c.y = Animated::new(0.33 * h);
                if let Some(tx) = c.text.as_mut() {
                    tx.text = s.title.clone();
                    tx.size = Animated::new((h * 0.05).max(12.0));
                    tx.bold = true;
                    tx.shadow = true;
                    tx.align = 1;
                }
                let _ = w;
            }
            lift_above(p, id, made.cover.unwrap_or(vis));
            made.title = Some(id);
        }
        let end = p.clip(audio).map_or(t + len, |c| c.end());
        t = if opts.crossfade > 0.0 { end - xf } else { end + opts.gap.max(0.0) };
        prev = Some(audio);
        out.push(made);
    }
    p.tidy();
    Ok(out)
}

/// Move clip `id` down to the lowest video track it fits on (so a mix keeps to three tracks, not a stair).
fn settle_low(p: &mut Project, id: Id) {
    let Some(from) = p.tracks.iter().position(|t| t.clips.iter().any(|x| x.id == id)) else { return };
    let Some(i) = p.tracks[from].clips.iter().position(|c| c.id == id) else { return };
    let (start, dur) = (p.tracks[from].clips[i].start, p.tracks[from].clips[i].duration);
    let Some(to) = p.video_tracks().into_iter().find(|&t| t < from && p.tracks[t].fits(start, dur, &[])) else { return };
    let clip = p.tracks[from].clips.remove(i);
    p.tracks[to].clips.push(clip);
    p.tracks[to].clips.sort_by(|a, b| a.start.total_cmp(&b.start));
}

/// Move clip `id` onto a video track drawn over `below`'s (a free one, else a new top track).
fn lift_above(p: &mut Project, id: Id, below: Id) {
    let track_of = |p: &Project, c: Id| p.tracks.iter().position(|t| t.clips.iter().any(|x| x.id == c));
    let (Some(from), Some(floor)) = (track_of(p, id), track_of(p, below)) else { return };
    let Some(i) = p.tracks[from].clips.iter().position(|c| c.id == id) else { return };
    let (start, dur) = (p.tracks[from].clips[i].start, p.tracks[from].clips[i].duration);
    // the lowest free track above `below` (the clip's own track counts as free for it)
    let free = p.video_tracks().into_iter().find(|&t| t > floor && (t == from || p.tracks[t].fits(start, dur, &[])));
    if free == Some(from) {
        return;
    }
    let clip = p.tracks[from].clips.remove(i);
    let to = free.unwrap_or_else(|| p.add_track(TrackKind::Video));
    p.tracks[to].clips.push(clip);
    p.tracks[to].clips.sort_by(|a, b| a.start.total_cmp(&b.start));
}

/// The topmost video track (layers placed there sit over the visualizer; a busy one makes a new one).
fn top_video(p: &Project) -> Option<usize> {
    p.video_tracks().last().copied()
}

/// "Artist - Title", or the one that exists, or "".
pub fn title_line(artist: &str, title: &str) -> String {
    match (artist.trim(), title.trim()) {
        ("", "") => String::new(),
        ("", t) => t.to_string(),
        (a, "") => a.to_string(),
        (a, t) => format!("{t}\n{a}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(path: &std::path::Path) -> Asset {
        crate::media::ffpipe::probe(path.to_str().unwrap()).unwrap()
    }

    /// Two generated songs + a cover: each song gets an audio clip, a linked visualizer over its whole
    /// length, a cover layer above it and a title card above that; the second song starts after the gap.
    #[test]
    fn visualizer_video_makes_the_expected_clips() {
        let Some(ff) = crate::media::ffpipe::ffmpeg_exe() else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let d = std::env::temp_dir().join(format!("se-mixvideo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let run = |args: &[&str]| assert!(std::process::Command::new(&ff).args(["-nostdin", "-v", "error", "-y"]).args(args).status().unwrap().success());
        let a = d.join("a.m4a");
        let b = d.join("b.mp3");
        let png = d.join("cover.png");
        run(&["-f", "lavfi", "-i", "sine=f=220:d=3", "-c:a", "aac", a.to_str().unwrap()]);
        run(&["-f", "lavfi", "-i", "sine=f=660:d=2", b.to_str().unwrap()]);
        run(&["-f", "lavfi", "-i", "color=blue:s=400x400:d=1", "-frames:v", "1", png.to_str().unwrap()]);
        let mut p = Project::new();
        p.width = 1920;
        p.height = 1080;
        let sa = p.add_asset(probe(&a));
        let sb = p.add_asset(probe(&b));
        let cov = p.add_asset(probe(&png));
        let songs = vec![
            Song { asset: sa, cover: Some(cov), title: title_line("Artist", "First") },
            Song { asset: sb, cover: None, title: "Second".into() },
        ];
        let style = visualizer::find(Some("Spectrum Bars")).unwrap();
        let opts = MixOptions { gap: 1.0, crossfade: 0.0, cover: true, title_card: true };
        let made = build(&mut p, &songs, 0.0, &style, &opts, Backend::Ffmpeg).unwrap();
        assert_eq!(made.len(), 2);
        let track_of = |id: Id| p.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == id)).unwrap();
        for (m, (start, len)) in made.iter().zip([(0.0, 3.0), (4.0, 2.0)]) {
            let au = p.clip(m.audio).unwrap();
            assert_eq!(au.kind, ClipKind::Audio);
            assert!((au.start - start).abs() < 1e-6, "start {}", au.start);
            assert!((au.duration - len).abs() < 0.1, "len {}", au.duration);
            let v = p.clip(m.visualizer).unwrap();
            assert_eq!((v.kind, v.start, v.duration), (ClipKind::Shape, au.start, au.duration));
            assert_eq!(visualizer::style_of(&v.effects[0]).as_deref(), Some("Spectrum Bars"));
            assert_eq!(v.link, au.link, "the visualizer is linked to its song");
            assert_ne!(v.link, 0);
            let t = p.clip(m.title.unwrap()).unwrap();
            assert_eq!(t.kind, ClipKind::Text);
            assert!(track_of(t.id) > track_of(v.id), "the title is above the visualizer");
        }
        assert_eq!(p.clip(made[0].title.unwrap()).unwrap().text.as_ref().unwrap().text, "First\nArtist");
        let c = p.clip(made[0].cover.unwrap()).unwrap();
        assert_eq!((c.kind, c.start, c.duration), (ClipKind::Image, 0.0, p.clip(made[0].audio).unwrap().duration));
        assert!(track_of(c.id) > track_of(made[0].visualizer), "the cover sits over the visualizer");
        assert!((c.scale.value - 0.45).abs() < 1e-9, "a share of the frame, not of the picture's pixels");
        assert!(made[1].cover.is_none(), "no cover, no cover layer");
        let vtrack = |id: Id| track_of(id);
        assert_eq!(vtrack(made[0].visualizer), vtrack(made[1].visualizer), "every visualizer on one track");
        // a failure leaves nothing behind
        let before = p.to_json();
        assert!(build(&mut p, &[Song { asset: cov, cover: None, title: String::new() }], 9.0, &style, &opts, Backend::Ffmpeg).is_err());
        assert_eq!(p.to_json(), before);
        let _ = std::fs::remove_dir_all(&d);
    }
}
