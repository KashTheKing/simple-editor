//! Audio stems for export (`ExportOptions.audio` = Streams / Stems): the timeline's audible clips split
//! into groups (by track, by bus, or by `AudioRole`), each returned as a copy of the project in which
//! only that group's clips are enabled. Mixing such a copy through the unchanged `Mixer` gives the stem
//! post-fader, through its bus chain, exactly as it sounds inside the full mix.
//!
//! How a group is isolated: every audio-track clip outside the group gets `enabled = false` - nothing
//! else changes, so track mute/solo, bus mute/solo/gain/filters and the routing stay as they are.
//! - Only top-level audio tracks ever sound (`mixer::mix_tracks`): a linked video clip's audio is its
//!   twin clip on an audio track, and a nested sequence is heard through the Sequence clip on an audio
//!   track (its whole sub-mix belongs to that clip's group - nested tracks are not split further).
//! - Track: the clips of one audio track. A track that `Project::active` silences (muted, or another
//!   track soloed) is silent in the mix too, so it gets no stem.
//! - Bus: the clips whose `Project::bus_of` (clip override, else track bus, else Main) is that bus.
//!   A clip routed into A → B → Main is A's stem (it still flows through B's and Main's processing).
//! - Role: per clip `audio_role`; `AudioRole::Unset` lands in "Other".
//!
//! Every audible clip is in exactly one group, so the stems sum to the mix - apart from non-linear
//! processing on shared buses (Main's compressor/limiter, the final clamp, a filter on a bus several
//! groups feed), which each stem gets on its own, and crossfades between clips of different groups on
//! one track (a disabled neighbour drops the transition).
//!
//! ponytail: N passes decode each source N times; single-pass multi-destination mix_tracks if audio
//! time ever matters.

use crate::engine::export::StemBy;
use crate::model::{AudioRole, Clip, ClipKind, Project, TrackKind};

/// One stem: a display name and the project that mixes to exactly that stem.
pub struct Stem {
    pub title: String,
    pub project: Project,
}

const ROLES: [(AudioRole, &str); 5] = [
    (AudioRole::Dialogue, "Dialogue"),
    (AudioRole::Music, "Music"),
    (AudioRole::Sfx, "SFX"),
    (AudioRole::Ambience, "Ambience"),
    (AudioRole::Unset, "Other"),
];

/// The group of clip `c` on track `ti`, as an index that also orders the stems (track order, bus list
/// order, role order).
fn key(project: &Project, by: StemBy, ti: usize, c: &Clip) -> usize {
    match by {
        StemBy::Track => ti,
        StemBy::Bus => {
            let b = project.bus_of(ti, c);
            project.buses.iter().position(|x| x.id == b).unwrap_or(0)
        }
        StemBy::Role => ROLES.iter().position(|(r, _)| *r == c.audio_role).unwrap_or(ROLES.len() - 1),
    }
}

fn title(project: &Project, by: StemBy, k: usize) -> String {
    let name = match by {
        StemBy::Track => project.tracks[k].name.clone(),
        StemBy::Bus => project.buses.get(k).map(|b| b.name.clone()).unwrap_or_else(|| "Main".into()),
        StemBy::Role => ROLES[k].1.to_string(),
    };
    if name.trim().is_empty() {
        format!("Stem {}", k + 1)
    } else {
        name
    }
}

/// True when the mixer would play `c` (on audio track `ti`) somewhere in `[a, b)`.
fn audible(project: &Project, ti: usize, c: &Clip, (a, b): (f64, f64)) -> bool {
    project.tracks[ti].kind == TrackKind::Audio
        && project.active(ti)
        && c.enabled
        && c.freeze.is_none()
        && matches!(c.kind, ClipKind::Audio | ClipKind::Sequence)
        && c.start < b
        && c.end() > a
}

/// The groups with something audible in `range`, sorted.
fn keys(project: &Project, by: StemBy, range: (f64, f64)) -> Vec<usize> {
    let mut keys: Vec<usize> = project
        .all_clips()
        .filter(|(ti, c)| audible(project, *ti, c, range))
        .map(|(ti, c)| key(project, by, ti, c))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// How many stems `groups` would return, without cloning anything (the size estimate).
pub fn count(project: &Project, by: StemBy, range: (f64, f64)) -> usize {
    keys(project, by, range).len()
}

/// The stems of `project` over the export range `[a, b)`, in group order; groups with nothing audible
/// in the range are skipped.
pub fn groups(project: &Project, by: StemBy, range: (f64, f64)) -> Vec<Stem> {
    keys(project, by, range)
        .into_iter()
        .map(|k| {
            let mut p = project.clone();
            for ti in 0..p.tracks.len() {
                if p.tracks[ti].kind != TrackKind::Audio {
                    continue;
                }
                for ci in 0..p.tracks[ti].clips.len() {
                    // keyed against the original: bus_of reads the (unchanged) routing
                    if key(project, by, ti, &project.tracks[ti].clips[ci]) != k {
                        p.tracks[ti].clips[ci].enabled = false;
                    }
                }
            }
            Stem { title: title(project, by, k), project: p }
        })
        .collect()
}

/// A stem's file name inside the stems folder: `NN <title>.wav` (1-based, two digits), with the
/// characters Windows refuses in file names replaced and trailing dots/spaces trimmed.
pub fn file_name(index: usize, title: &str) -> String {
    let clean: String =
        title.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches(|c: char| c == '.' || c == ' ');
    format!("{:02} {}.wav", index + 1, if clean.is_empty() { "Stem" } else { clean })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo};

    fn project() -> Project {
        let asset = Asset {
            id: 0,
            path: "Z:\\nope\\fake.wav".into(),
            kind: ClipKind::Audio,
            duration: 10.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: "pcm".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        };
        let mut p = Project::from_media(asset);
        // a second audio track holding a copy of the first clip, as Music
        let a1 = p.audio_tracks()[0];
        let mut c = p.tracks[a1].clips[0].clone();
        c.id = 9001;
        c.audio_role = AudioRole::Music;
        let a2 = p.add_track(TrackKind::Audio);
        p.tracks[a2].name = "Music".into();
        p.tracks[a2].clips.push(c);
        p
    }

    fn enabled(s: &Stem) -> Vec<u64> {
        s.project.all_clips().filter(|(ti, c)| s.project.tracks[*ti].kind == TrackKind::Audio && c.enabled).map(|(_, c)| c.id).collect()
    }

    #[test]
    fn by_track_isolates_each_track_and_skips_inactive() {
        let mut p = project();
        let [a1, a2] = [p.audio_tracks()[0], p.audio_tracks()[1]];
        let s = groups(&p, StemBy::Track, (0.0, 100.0));
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].title, "Music");
        assert_eq!(enabled(&s[0]), vec![p.tracks[a1].clips[0].id]);
        assert_eq!(enabled(&s[1]), vec![9001]);
        // solo A2: A1 is silent in the mix, so it gets no stem
        p.tracks[a2].solo = true;
        let s = groups(&p, StemBy::Track, (0.0, 100.0));
        assert_eq!(s.len(), 1);
        assert_eq!(enabled(&s[0]), vec![9001]);
        // a range with nothing in it: no stems at all
        assert!(groups(&p, StemBy::Track, (50.0, 60.0)).is_empty());
    }

    #[test]
    fn by_bus_follows_clip_override_then_track() {
        let mut p = project();
        let [a1, a2] = [p.audio_tracks()[0], p.audio_tracks()[1]];
        // no buses: everything is one Main stem
        let s = groups(&p, StemBy::Bus, (0.0, 100.0));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].title, "Main");
        let fx = p.add_bus("FX");
        p.tracks[a2].bus = fx;
        let s = groups(&p, StemBy::Bus, (0.0, 100.0));
        assert_eq!(s.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Main", "FX"]);
        assert_eq!(enabled(&s[1]), vec![9001]);
        // a clip override beats its track's bus
        p.tracks[a1].clips[0].bus = fx;
        let s = groups(&p, StemBy::Bus, (0.0, 100.0));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].title, "FX");
        assert_eq!(enabled(&s[0]).len(), 2);
        // routing is untouched in the copies
        assert_eq!(s[0].project.tracks[a2].bus, fx);
    }

    #[test]
    fn by_role_is_per_clip_with_unset_as_other() {
        let mut p = project();
        let a2 = p.audio_tracks()[1];
        let mut c = p.tracks[a2].clips[0].clone();
        c.id = 9002;
        c.start = 20.0;
        c.audio_role = AudioRole::Dialogue;
        p.tracks[a2].clips.push(c);
        let s = groups(&p, StemBy::Role, (0.0, 100.0));
        assert_eq!(s.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Dialogue", "Music", "Other"]);
        // the role stem silences the other clip on the same track, not the track
        assert_eq!(enabled(&s[0]), vec![9002]);
        assert_eq!(enabled(&s[1]), vec![9001]);
        assert!(!s[1].project.tracks[a2].muted);
        // the Dialogue clip is outside [0, 10): skipped
        assert_eq!(groups(&p, StemBy::Role, (0.0, 10.0)).len(), 2);
    }

    #[test]
    fn file_names_are_windows_safe() {
        assert_eq!(file_name(0, "Dialogue"), "01 Dialogue.wav");
        assert_eq!(file_name(11, "A/B: \"x\"?. "), "12 A_B_ _x__.wav");
        assert_eq!(file_name(2, "  "), "03 Stem.wav");
    }
}
