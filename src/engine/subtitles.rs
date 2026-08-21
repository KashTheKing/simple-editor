//! Subtitle file formats: SubRip (.srt) and WebVTT (.vtt) — parse (auto-detected) and write.
//! Basic formatting tags (<i>, <b>, {\an8}, VTT cue settings) are stripped on import; text is kept
//! as plain lines. Times are seconds.

use crate::model::Cue;

/// Parse SRT or WebVTT (auto-detected by the "WEBVTT" header / timestamp style). Malformed blocks are
/// skipped. Returns (start, end, text) triples in file order.
pub fn parse(text: &str) -> Vec<(f64, f64, String)> {
    let text = text.trim_start_matches('\u{feff}');
    let mut out = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            parse_block(&block, &mut out);
            block.clear();
        } else {
            block.push(line);
        }
    }
    out
}

/// One blank-line separated block: [optional index / cue-id lines,] timestamp line, text lines.
/// The timestamp line is the first containing "-->" (this also skips WEBVTT/NOTE/STYLE blocks).
fn parse_block(block: &[&str], out: &mut Vec<(f64, f64, String)>) {
    let Some(ts) = block.iter().position(|l| l.contains("-->")) else { return };
    let Some((a, b)) = block[ts].split_once("-->") else { return };
    let Some(start) = parse_time(a.trim()) else { return };
    // VTT cue settings ("align:start position:10%") follow the end time — take the first token
    let Some(end) = b.trim().split_whitespace().next().and_then(parse_time) else { return };
    if end < start {
        return;
    }
    let mut text = String::new();
    for l in &block[ts + 1..] {
        if !text.is_empty() {
            text.push('\n');
        }
        strip_tags(l, &mut text);
    }
    out.push((start, end, text.trim().to_string()));
}

/// "HH:MM:SS,mmm" / "HH:MM:SS.mmm" / "MM:SS.mmm" → seconds.
fn parse_time(s: &str) -> Option<f64> {
    let mut total = 0.0;
    let parts: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    for p in &parts[..parts.len() - 1] {
        total = total * 60.0 + p.trim().parse::<u32>().ok()? as f64;
    }
    let secs: f64 = parts[parts.len() - 1].replace(',', ".").trim().parse().ok()?;
    if !(0.0..60.0).contains(&secs) {
        return None;
    }
    Some(total * 60.0 + secs)
}

/// Append `s` to `out` with `<...>` tags and `{\...}` override codes removed.
fn strip_tags(s: &str, out: &mut String) {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
            }
            '{' if chars.peek() == Some(&'\\') => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
}

fn fmt_time(t: f64, sep: char) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, sep, ms % 1000)
}

pub fn to_srt(cues: &[Cue]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for (i, c) in cues.iter().enumerate() {
        let _ = write!(s, "{}\n{} --> {}\n{}\n\n", i + 1, fmt_time(c.start, ','), fmt_time(c.end, ','), c.text);
    }
    s
}

pub fn to_vtt(cues: &[Cue]) -> String {
    use std::fmt::Write;
    let mut s = String::from("WEBVTT\n\n");
    for c in cues {
        let _ = write!(s, "{} --> {}\n{}\n\n", fmt_time(c.start, '.'), fmt_time(c.end, '.'), c.text);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_srt() {
        let srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>world</i>\r\n\r\n2\r\n00:01:00,250 --> 00:01:02,000\r\n{\\an8}Two\r\nlines\r\n\r\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], (1.0, 2.5, "Hello world".to_string()));
        assert_eq!(cues[1], (60.25, 62.0, "Two\nlines".to_string()));
    }

    #[test]
    fn parses_vtt() {
        let vtt = "WEBVTT - some file\n\nNOTE a comment\nspanning lines\n\nintro\n00:01.000 --> 00:04.000 align:start position:10%\n<b>Never</b> gonna\n\n01:00:00.000 --> 01:00:30.000\n<c.yellow>styled</c>\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], (1.0, 4.0, "Never gonna".to_string()));
        assert_eq!(cues[1], (3600.0, 3630.0, "styled".to_string()));
    }

    #[test]
    fn skips_malformed_blocks() {
        let srt = "1\n00:00:01,000 --> nonsense\nbad\n\n2\nno timestamp at all\n\n3\n00:00:02,000 --> 00:00:01,000\nend before start\n\n4\n00:00:03,000 --> 00:00:04,000\ngood\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0], (3.0, 4.0, "good".to_string()));
        assert!(parse("").is_empty());
        assert!(parse("garbage\nonly").is_empty());
    }

    #[test]
    fn writes_and_round_trips() {
        let cues = vec![
            Cue { id: 1, start: 0.5, end: 2.0, text: "One".into() },
            Cue { id: 2, start: 3661.25, end: 3662.0, text: "A\nB".into() },
        ];
        let srt = to_srt(&cues);
        assert!(srt.starts_with("1\n00:00:00,500 --> 00:00:02,000\nOne\n\n2\n01:01:01,250 --> "), "{srt}");
        let vtt = to_vtt(&cues);
        assert!(vtt.starts_with("WEBVTT\n\n00:00:00.500 --> 00:00:02.000\nOne\n\n"), "{vtt}");
        for text in [srt, vtt] {
            let back = parse(&text);
            assert_eq!(back.len(), 2);
            for (got, want) in back.iter().zip(&cues) {
                assert!((got.0 - want.start).abs() < 1e-9 && (got.1 - want.end).abs() < 1e-9);
                assert_eq!(got.2, want.text);
            }
        }
    }
}
