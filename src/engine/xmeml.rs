//! Final Cut Pro 7 XML (xmeml v5) exporter — the interchange format both Premiere Pro
//! (File → Import) and DaVinci Resolve (File → Import → Timeline) read.
//! Video tracks (with clipitems: file path, in/out/start/end in frames, opacity/position if animated)
//! and audio tracks (one per project audio track, channel-source by stream). Text clips are exported as
//! a generator-less gap (Premiere/Resolve text isn't interchangeable) — mention it in a comment.
//! Paths as `file://localhost/C:/...` pathurls. Timebase = round(fps), NTSC flag when fps is fractional.

use crate::model::{Asset, Clip, ClipKind, Project, TrackKind};
use std::collections::HashSet;
use std::fmt::Write;

pub fn export_xmeml(project: &Project) -> String {
    let fps = project.fps.max(1.0);
    let fr = |t: f64| (t * fps).round() as i64;
    let rate = format!(
        "<rate><timebase>{}</timebase><ntsc>{}</ntsc></rate>",
        fps.round() as i64,
        if (fps - fps.round()).abs() > 1e-3 { "TRUE" } else { "FALSE" }
    );
    let mut x = String::with_capacity(4096);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"5\">\n");
    x.push_str("<sequence id=\"sequence-1\">\n");
    let _ = writeln!(x, "<name>{}</name>", esc(&project.name));
    let _ = writeln!(x, "<duration>{}</duration>\n{rate}", fr(project.duration()));
    let _ = writeln!(
        x,
        "<timecode>{rate}<string>00:00:00:00</string><frame>0</frame><displayformat>NDF</displayformat></timecode>"
    );
    x.push_str("<media>\n<video>\n");
    let _ = writeln!(
        x,
        "<format><samplecharacteristics><width>{}</width><height>{}</height>{rate}<pixelaspectratio>square</pixelaspectratio></samplecharacteristics></format>",
        project.width, project.height
    );
    let mut defined: HashSet<u64> = HashSet::new();
    let mut n_item = 0;
    for kind in [TrackKind::Video, TrackKind::Audio] {
        if kind == TrackKind::Audio {
            x.push_str("</video>\n<audio>\n");
            x.push_str("<format><samplecharacteristics><depth>16</depth><samplerate>48000</samplerate></samplecharacteristics></format>\n");
        }
        for (ti, track) in project.tracks.iter().enumerate().filter(|(_, t)| t.kind == kind) {
            x.push_str("<track>\n");
            for c in &track.clips {
                let Some(asset) = project.asset(c.asset).filter(|_| c.uses_asset()) else {
                    let _ = writeln!(
                        x,
                        "<!-- {} clip \"{}\" at {}..{} skipped (not interchangeable) -->",
                        if c.kind == ClipKind::Text { "text" } else { "unresolved" },
                        esc(&c.name),
                        fr(c.start),
                        fr(c.end())
                    );
                    continue;
                };
                n_item += 1;
                let src_dur = if asset.kind == ClipKind::Image { c.duration } else { asset.duration };
                let _ = writeln!(x, "<clipitem id=\"clipitem-{n_item}\">\n<name>{}</name>", esc(&c.name));
                let _ =
                    writeln!(x, "<enabled>{}</enabled>\n<duration>{}</duration>\n{rate}", tf(c.enabled), fr(src_dur));
                let _ = writeln!(
                    x,
                    "<start>{}</start>\n<end>{}</end>\n<in>{}</in>\n<out>{}</out>",
                    fr(c.start),
                    fr(c.end()),
                    fr(c.src_in),
                    fr(c.src_end())
                );
                if defined.insert(asset.id) {
                    write_file(&mut x, asset, &rate, fr(src_dur));
                } else {
                    let _ = writeln!(x, "<file id=\"file-{}\"/>", asset.id);
                }
                if kind == TrackKind::Video {
                    write_filters(&mut x, project, c);
                } else {
                    let _ = writeln!(
                        x,
                        "<sourcetrack><mediatype>audio</mediatype><trackindex>{}</trackindex></sourcetrack>",
                        c.audio_stream + 1
                    );
                }
                x.push_str("</clipitem>\n");
            }
            let _ = writeln!(x, "<enabled>{}</enabled>\n<locked>FALSE</locked>\n</track>", tf(project.active(ti)));
        }
    }
    x.push_str("</audio>\n</media>\n</sequence>\n</xmeml>\n");
    x
}

fn write_file(x: &mut String, asset: &Asset, rate: &str, frames: i64) {
    let _ = writeln!(
        x,
        "<file id=\"file-{}\">\n<name>{}</name>\n<pathurl>{}</pathurl>\n{rate}\n<duration>{frames}</duration>\n<media>",
        asset.id,
        esc(&asset.name()),
        esc(&pathurl(&asset.path))
    );
    if asset.has_video() {
        let _ = writeln!(
            x,
            "<video><samplecharacteristics><width>{}</width><height>{}</height>{rate}</samplecharacteristics></video>",
            asset.width, asset.height
        );
    }
    if !asset.audio_streams.is_empty() {
        // ponytail: every stream reported as one stereo track — FCP XML has no notion of separate streams
        x.push_str("<audio><samplecharacteristics><depth>16</depth><samplerate>48000</samplerate></samplecharacteristics><channelcount>2</channelcount></audio>\n");
    }
    x.push_str("</media>\n</file>\n");
}

/// Static opacity / basic motion when non-default (keyframes flattened to the clip-start value).
fn write_filters(x: &mut String, project: &Project, c: &Clip) {
    // ponytail: animated properties are written as their value at clip start — emit <keyframe>s if needed
    if !c.opacity.is_default(1.0) {
        let _ = writeln!(
            x,
            "<filter><effect><name>Opacity</name><effectid>opacity</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype>\
<parameter><parameterid>opacity</parameterid><name>opacity</name><valuemin>0</valuemin><valuemax>100</valuemax><value>{}</value></parameter></effect></filter>",
            (c.opacity.at(0.0) * 100.0).clamp(0.0, 100.0)
        );
    }
    if !c.scale.is_default(1.0) || !c.x.is_default(0.0) || !c.y.is_default(0.0) || !c.rotation.is_default(0.0) {
        let _ = writeln!(
            x,
            "<filter><effect><name>Basic Motion</name><effectid>basic</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype>\
<parameter><parameterid>scale</parameterid><name>Scale</name><valuemin>0</valuemin><valuemax>1000</valuemax><value>{}</value></parameter>\
<parameter><parameterid>rotation</parameterid><name>Rotation</name><valuemin>-8640</valuemin><valuemax>8640</valuemax><value>{}</value></parameter>\
<parameter><parameterid>center</parameterid><name>Center</name><value><horiz>{}</horiz><vert>{}</vert></value></parameter></effect></filter>",
            c.scale.at(0.0) * 100.0,
            c.rotation.at(0.0),
            c.x.at(0.0) / project.width.max(1) as f64,
            c.y.at(0.0) / project.height.max(1) as f64
        );
    }
}

fn tf(b: bool) -> &'static str {
    if b {
        "TRUE"
    } else {
        "FALSE"
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(ch),
        }
    }
    o
}

/// `C:\a b\v.mp4` → `file://localhost/C:/a%20b/v.mp4`
fn pathurl(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.trim_start_matches('/');
    let mut o = String::from("file://localhost/");
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => o.push(b as char),
            _ => {
                let _ = write!(o, "%{b:02X}");
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AudioStreamInfo;

    fn asset() -> Asset {
        Asset {
            id: 0,
            path: "C:\\My Videos\\clip & co.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 29.97,
            audio_streams: (0..2)
                .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
                .collect(),
            codec: "h264".into(),
        }
    }

    fn count(s: &str, pat: &str) -> usize {
        s.matches(pat).count()
    }

    #[test]
    fn xmeml_shape() {
        let mut p = Project::from_media(asset());
        p.split_at(4.0, None);
        p.add_text_clip(1.0, 2.0);
        p.tracks[0].clips[0].opacity.value = 0.5;
        p.tracks[0].clips[1].scale.value = 1.5;
        let x = export_xmeml(&p);
        assert!(x.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"5\">"));
        assert!(x.contains("<pathurl>file://localhost/C:/My%20Videos/clip%20%26%20co.mp4</pathurl>"), "{x}");
        assert!(x.contains("<name>clip &amp; co.mp4</name>"));
        assert!(x.contains("<timebase>30</timebase><ntsc>TRUE</ntsc>"));
        for tag in
            ["track", "clipitem", "sequence", "media", "video", "audio", "filter", "xmeml", "effect", "parameter"]
        {
            let open = count(&x, &format!("<{tag}>")) + count(&x, &format!("<{tag} "));
            assert_eq!(open, count(&x, &format!("</{tag}>")), "{tag}");
        }
        assert_eq!(count(&x, "<track>"), 4); // V1 V2(text) A1 A2
        assert_eq!(count(&x, "<clipitem id="), 6); // 2 video + 2×2 audio
        let fid = p.assets[0].id;
        assert_eq!(count(&x, &format!("<file id=\"file-{fid}\">")), 1); // defined once
        assert_eq!(count(&x, &format!("<file id=\"file-{fid}\"/>")), 5);
        assert!(x.contains("<!-- text clip \"Text\" at 30..90 skipped"));
        assert!(x.contains("<effectid>opacity</effectid>") && x.contains("<value>50</value>"));
        assert!(x.contains("<effectid>basic</effectid>") && x.contains("<value>150</value>"));
        assert!(x.contains("<trackindex>2</trackindex>"));
        // second clip: start 4 s → frame 120 at 29.97
        assert!(x.contains("<start>120</start>"), "{x}");
        assert!(x.ends_with("</xmeml>\n"));
    }
}
