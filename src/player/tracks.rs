//! The audio and subtitle tracks a file carries, for the player's track selector, and the cues of the
//! one picked. Listed with ffprobe the first time the selector opens (not at startup); a subtitle
//! stream is pulled out with ffmpeg as SRT when it is switched on. A `<name>.srt` / `.vtt` beside the
//! file is offered too, as Media Player does.

use crate::media::ffpipe;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// 0-based index among the file's streams of this kind (`0:a:N` / `0:s:N`).
    pub index: usize,
    pub label: String,
    /// A side-car subtitle file instead of a stream.
    pub file: Option<PathBuf>,
    /// Picture-based subtitles (PGS, VobSub): listed, not shown.
    pub bitmap: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracks {
    pub audio: Vec<Track>,
    pub subtitles: Vec<Track>,
}

/// "English", "Commentary (eng)", "Track 2" ...
fn label(kind: &str, n: usize, lang: &str, title: &str, codec: &str) -> String {
    let lang = language(lang);
    let base = match (title.trim(), lang.as_str()) {
        ("", "") => format!("{kind} {}", n + 1),
        ("", l) => l.to_string(),
        (t, "") => t.to_string(),
        (t, l) if t.eq_ignore_ascii_case(l) => t.to_string(),
        (t, l) => format!("{t} ({l})"),
    };
    if codec.is_empty() {
        base
    } else {
        format!("{base} · {}", codec.to_ascii_uppercase())
    }
}

/// ISO 639-2 codes people meet most, spelled out; anything else as it is.
fn language(code: &str) -> String {
    let name = match code.to_ascii_lowercase().as_str() {
        "" | "und" => "",
        "eng" | "en" => "English",
        "spa" | "es" => "Spanish",
        "fre" | "fra" | "fr" => "French",
        "ger" | "deu" | "de" => "German",
        "ita" | "it" => "Italian",
        "por" | "pt" => "Portuguese",
        "jpn" | "ja" => "Japanese",
        "kor" | "ko" => "Korean",
        "chi" | "zho" | "zh" => "Chinese",
        "rus" | "ru" => "Russian",
        "ara" | "ar" => "Arabic",
        "hin" | "hi" => "Hindi",
        "dut" | "nld" | "nl" => "Dutch",
        "pol" | "pl" => "Polish",
        "tur" | "tr" => "Turkish",
        other => return other.to_string(),
    };
    name.to_string()
}

/// List `path`'s audio and subtitle streams (ffprobe), plus side-car subtitle files.
pub fn probe(path: &Path) -> Tracks {
    let mut t = Tracks::default();
    if let Some(exe) = ffpipe::ffprobe_exe() {
        let out = ffpipe::command(&exe)
            .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name:stream_tags=language,title", "-of", "json"])
            .arg(path)
            .output();
        if let Ok(out) = out {
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            for s in v["streams"].as_array().into_iter().flatten() {
                let tag = |k: &str| s["tags"][k].as_str().unwrap_or("").to_string();
                let codec = s["codec_name"].as_str().unwrap_or("");
                match s["codec_type"].as_str() {
                    Some("audio") => {
                        let n = t.audio.len();
                        t.audio.push(Track {
                            index: n,
                            label: label("Track", n, &tag("language"), &tag("title"), codec),
                            file: None,
                            bitmap: false,
                        });
                    }
                    Some("subtitle") => {
                        let n = t.subtitles.len();
                        let bitmap = matches!(codec, "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "xsub");
                        t.subtitles.push(Track {
                            index: n,
                            label: label("Subtitles", n, &tag("language"), &tag("title"), ""),
                            file: None,
                            bitmap,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    for ext in ["srt", "vtt"] {
        let side = path.with_extension(ext);
        if side.is_file() {
            let n = t.subtitles.len();
            let name = side.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            t.subtitles.push(Track { index: n, label: name, file: Some(side), bitmap: false });
        }
    }
    t
}

/// The cues of subtitle track `track` of `path`: (start, end, text).
pub fn cues(path: &Path, track: &Track) -> Result<Vec<(f64, f64, String)>, String> {
    if track.bitmap {
        return Err("picture subtitles cannot be shown".into());
    }
    let text = match &track.file {
        Some(f) => std::fs::read_to_string(f).map_err(|e| e.to_string())?,
        None => {
            let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg not found")?;
            let out = ffpipe::command(&exe)
                .args(["-v", "error", "-i"])
                .arg(path)
                .args(["-map", &format!("0:s:{}", track.index), "-f", "srt", "pipe:1"])
                .output()
                .map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    };
    Ok(crate::engine::subtitles::parse(&text))
}

/// The cue showing at `t`, if any (cues in file order; a few hundred, so a scan is fine).
pub fn cue_at(cues: &[(f64, f64, String)], t: f64) -> Option<&str> {
    cues.iter().find(|(a, b, _)| t >= *a && t < *b).map(|(_, _, s)| s.as_str())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 4 s 320x180 clip with two audio tracks (English, Commentary) and one English subtitle track,
    /// made once per test run with ffmpeg. None (test skipped) without ffmpeg.
    pub(crate) fn two_audio_one_sub() -> Option<PathBuf> {
        let ff = ffpipe::ffmpeg_exe()?;
        let dir = std::env::temp_dir().join(format!("se-player-tracks-{}", std::process::id()));
        let out = dir.join("tracks.mkv");
        if out.is_file() {
            return Some(out);
        }
        std::fs::create_dir_all(&dir).ok()?;
        let srt = dir.join("subs.srt");
        std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,000\nHello there\n\n2\n00:00:02,500 --> 00:00:03,500\nSecond line\n\n").ok()?;
        let tmp = dir.join("tracks.tmp.mkv");
        let ok = ffpipe::command(&ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-f", "lavfi", "-i", "sine=frequency=880:duration=4"])
            .arg("-i")
            .arg(&srt)
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:s"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-c:s", "srt"])
            .args(["-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "title=Commentary"])
            .args(["-metadata:s:s:0", "language=eng"])
            .arg(&tmp)
            .status()
            .ok()?
            .success();
        if !ok {
            return None;
        }
        std::fs::rename(&tmp, &out).ok()?;
        Some(out)
    }

    #[test]
    fn track_selector_lists_two_audio_tracks_and_one_subtitle_track() {
        let Some(f) = two_audio_one_sub() else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let t = probe(&f);
        let audio: Vec<&str> = t.audio.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(audio, ["English · AAC", "Commentary (English) · AAC"]);
        assert_eq!(t.subtitles.len(), 1);
        assert_eq!(t.subtitles[0].label, "English");
        let c = cues(&f, &t.subtitles[0]).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(cue_at(&c, 1.0), Some("Hello there"));
        assert_eq!(cue_at(&c, 2.2), None);
    }

    #[test]
    fn labels() {
        assert_eq!(label("Track", 1, "", "", ""), "Track 2");
        assert_eq!(label("Track", 0, "jpn", "", "opus"), "Japanese · OPUS");
        assert_eq!(label("Subtitles", 0, "eng", "English", ""), "English");
        assert_eq!(label("Subtitles", 0, "xyz", "Signs", ""), "Signs (xyz)");
    }
}
