//! CMX 3600 EDL export - the route into Avid Media Composer (and a fallback for Premiere / Resolve).
//! One EDL per video track; the first one also carries every audio track (`A`, `A2`, `A3` … events,
//! the notation `engine::import::channels` reads back). Each event is followed by
//! `* FROM CLIP NAME:` (the file name) and `* SOURCE FILE:` (the full path, which the importer
//! relinks by). Cross fades / dips / pushes become `D` events, wipes `W001`, both with their length in
//! frames. Record timecode starts at 00:00:00:00, like the XML export.

use crate::model::{Asset, Id, Project, TransitionEdge, TransitionKind};
use std::collections::HashMap;
use std::fmt::Write;

/// `(suffix, text)` per EDL: "V1" first (with the audio), then "V2", "V3" … for video tracks that have
/// clips. A project without video tracks gives one audio-only EDL.
pub fn export_edl(project: &Project) -> Vec<(String, String)> {
    let mut reels = Reels::default();
    let video = project.video_tracks();
    let audio = project.audio_tracks();
    let mut out = Vec::new();
    for (k, &ti) in video.iter().enumerate() {
        if k > 0 && project.tracks[ti].clips.is_empty() {
            continue;
        }
        let tag = format!("V{}", k + 1);
        let mut s = header(project, if k == 0 { String::new() } else { format!(" {tag}") });
        let mut n = 0;
        events(&mut s, &mut n, project, ti, "V", &mut reels);
        if k == 0 {
            for (j, &ai) in audio.iter().enumerate() {
                let chan = if j == 0 { "A".to_string() } else { format!("A{}", j + 1) };
                events(&mut s, &mut n, project, ai, &chan, &mut reels);
            }
        }
        out.push((tag, s));
    }
    if video.is_empty() {
        let mut s = header(project, String::new());
        let mut n = 0;
        for (j, &ai) in audio.iter().enumerate() {
            let chan = if j == 0 { "A".to_string() } else { format!("A{}", j + 1) };
            events(&mut s, &mut n, project, ai, &chan, &mut reels);
        }
        out.push(("A".to_string(), s));
    }
    out
}

fn header(project: &Project, suffix: String) -> String {
    let fps = project.fps.max(1.0);
    // `* FPS:` is a comment to other readers; `import::read_edl` takes the rate from it
    format!(
        "TITLE: {}{suffix}\nFCM: {}\n* FPS: {fps}\n\n",
        one_line(&project.name),
        if drop_frame(fps) { "DROP FRAME" } else { "NON-DROP FRAME" }
    )
}

/// 29.97 is written as drop-frame timecode (the reader turns `FCM: DROP FRAME` back into 29.97).
// ponytail: 59.94 DF (4 frames dropped) is written non-drop - add when a 59.94 project needs Avid
fn drop_frame(fps: f64) -> bool {
    fps.round() as i64 == 30 && (fps - 30.0).abs() > 1e-3
}

/// Frame count → `HH:MM:SS:FF`, or `HH:MM:SS;FF` drop-frame at 29.97.
pub(crate) fn tc(frames: i64, fps: f64) -> String {
    let per = fps.round().max(1.0) as i64;
    let mut f = frames.max(0);
    let df = drop_frame(fps);
    if df {
        // frame numbers ;00 and ;01 are skipped every minute except every tenth
        let (d, m) = (f / 17982, f % 17982);
        f += 18 * d + 2 * ((m - 2) / 1798);
    }
    format!(
        "{:02}:{:02}:{:02}{}{:02}",
        (f / (per * 3600)) % 24,
        (f / (per * 60)) % 60,
        (f / per) % 60,
        if df { ';' } else { ':' },
        f % per
    )
}

/// Source → reel name: the file stem, upper-case alphanumerics, at most 8 characters (CMX limit),
/// made unique with a numeric tail when two sources share the first 8.
#[derive(Default)]
struct Reels {
    by_asset: HashMap<Id, String>,
}

impl Reels {
    fn of(&mut self, a: &Asset) -> String {
        if let Some(r) = self.by_asset.get(&a.id) {
            return r.clone();
        }
        let stem = std::path::Path::new(&a.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut base: String =
            stem.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).take(8).collect();
        if base.is_empty() {
            base = "AX".into();
        }
        let mut reel = base.clone();
        let mut i = 1;
        while self.by_asset.values().any(|r| *r == reel) {
            let tail = i.to_string();
            reel = format!("{}{tail}", &base[..base.len().min(8 - tail.len())]);
            i += 1;
        }
        self.by_asset.insert(a.id, reel.clone());
        reel
    }
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Every clip of track `ti` as events on channel `chan`.
fn events(s: &mut String, n: &mut usize, project: &Project, ti: usize, chan: &str, reels: &mut Reels) {
    let fps = project.fps.max(1.0);
    let fr = |t: f64| (t * fps).round() as i64;
    let track = &project.tracks[ti];
    for c in &track.clips {
        let Some(asset) = project.asset(c.asset).filter(|_| c.uses_asset()) else {
            let _ = writeln!(s, "* SKIPPED: {} at {} (not interchangeable)\n", one_line(&c.name), tc(fr(c.start), fps));
            continue;
        };
        let reel = reels.of(asset);
        let (ri, ro) = (fr(c.start), fr(c.end()));
        // ponytail: speed / reverse are not written (no M2 lines) - the source runs for the record length
        let si = fr(c.src_in);
        let so = si + (ro - ri);
        *n += 1;
        // past 999 the number grows to 4 digits instead of wrapping to 000 (`import::read_edl` takes any)
        let num = *n;
        let tr = track.transitions.iter().find(|t| t.right == c.id && t.edge == TransitionEdge::Cut);
        let dissolve = tr.and_then(|t| {
            let (l, r) = track.transition_clips(t)?;
            let (_, h) = t.cut_half(l, r)?;
            Some((t.kind, (2.0 * h * fps).round() as i64, l?))
        });
        match dissolve {
            // the outgoing side as a zero-length "from" event at the cut, then the incoming as D/W.
            // ponytail: the dissolve starts at the cut (CMX "start" alignment) instead of centred on it
            // - the record times of both clips stay exact, which is what the app's reader expects.
            Some((kind, frames, left)) if frames > 0 => {
                let left_asset = project.asset(left.asset).filter(|_| left.uses_asset());
                if let Some(la) = left_asset {
                    let lr = reels.of(la);
                    let lo = fr(left.src_in) + (fr(left.end()) - fr(left.start));
                    line(s, num, &lr, chan, "C", "", [lo, lo, ri, ri], fps);
                    names(s, la);
                }
                let code = if kind == TransitionKind::Wipe { "W001" } else { "D" };
                line(s, num, &reel, chan, code, &format!("{frames:03}"), [si, so, ri, ro], fps);
            }
            _ => line(s, num, &reel, chan, "C", "", [si, so, ri, ro], fps),
        }
        names(s, asset);
    }
}

#[allow(clippy::too_many_arguments)]
fn line(s: &mut String, num: usize, reel: &str, chan: &str, kind: &str, dur: &str, t: [i64; 4], fps: f64) {
    let _ = writeln!(
        s,
        "{num:03}  {reel:<8} {chan:<5} {kind:<4} {dur:>3} {} {} {} {}",
        tc(t[0], fps),
        tc(t[1], fps),
        tc(t[2], fps),
        tc(t[3], fps)
    );
}

fn names(s: &mut String, a: &Asset) {
    let _ = writeln!(s, "* FROM CLIP NAME: {}", one_line(&a.name()));
    let _ = writeln!(s, "* SOURCE FILE: {}", one_line(&a.path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_and_reels() {
        assert_eq!(tc(0, 30.0), "00:00:00:00");
        assert_eq!(tc(30 * 3600 + 15, 30.0), "01:00:00:15");
        assert_eq!(tc(250, 25.0), "00:00:10:00");
        // 29.97 DF: frame 1800 is 00:01:00;02 (;00 and ;01 are skipped), minute 10 keeps ;00
        assert_eq!(tc(1800, 30000.0 / 1001.0), "00:01:00;02");
        assert_eq!(tc(17982, 30000.0 / 1001.0), "00:10:00;00");
        let mut r = Reels::default();
        let a = |id, path: &str| Asset { id, path: path.into(), ..crate::engine::import::placeholder(path) };
        assert_eq!(r.of(&a(1, r"C:\x\my clip-final.mp4")), "MYCLIPFI");
        assert_eq!(r.of(&a(2, r"C:\y\my clip-final2.mp4")), "MYCLIPF1", "8 chars, unique");
        assert_eq!(r.of(&a(1, r"C:\x\my clip-final.mp4")), "MYCLIPFI", "same source, same reel");
        assert_eq!(r.of(&a(3, r"C:\y\---.mp4")), "AX");
    }
}
