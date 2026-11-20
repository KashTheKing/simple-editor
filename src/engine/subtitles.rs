//! Subtitle file formats: SubRip (.srt) and WebVTT (.vtt) - parse (auto-detected) and write.
//! Basic formatting tags (<i>, <b>, {\an8}, VTT cue settings) are stripped on import; text is kept
//! as plain lines. Times are seconds.

use crate::model::{Cue, Project, SubtitleAnim, TextSpan, TextStyle};
use std::borrow::Cow;

// ---- ws:registries-schema-hooks ----
/// The subtitle cue (if any) drawn at timeline time `t`: the cue's text and the project's subtitle
/// style, factoring the cue-lookup + empty-text check duplicated between `engine::compose::render`
/// and `playback`'s layer builder. Borrowed (`Cow::Borrowed`) on the plain path - `compose::render`'s
/// existing `sub_key` cache guard only clones into its own `sub_style` when the cue actually changes,
/// and a cloning signature here would defeat that guard on every rendered frame.
///
/// ---- ws:transcript-captions ----
/// With `Project.subtitle_anim` set, the cue is karaoke'd against the transcript that covers it
/// (see `karaoke`) and comes back owned: a `TextSpan` over the word being spoken at `t`
/// (Highlight/PopWord) or the text truncated to the words spoken so far (Typewriter). The style's
/// `cache_key` only changes when the active word changes, so the rasterizer cache still holds
/// between words.
pub fn cue_layer_at(project: &Project, t: f64) -> Option<(Cow<'_, str>, Cow<'_, TextStyle>)> {
    if !project.show_subtitles {
        return None;
    }
    let cue = project.cue_at(t)?;
    if cue.text.trim().is_empty() {
        return None;
    }
    let plain = (Cow::Borrowed(cue.text.as_str()), Cow::Borrowed(&project.subtitle_style));
    if project.subtitle_anim == SubtitleAnim::None {
        return Some(plain);
    }
    let Some(words) = active_words(project, cue) else { return Some(plain) };
    match karaoke(&cue.text, &words, t, project.subtitle_anim, &project.subtitle_style) {
        None => Some(plain),
        Some((text, _)) if text.trim().is_empty() => None,
        Some((text, style)) => Some((Cow::Owned(text), Cow::Owned(style))),
    }
}

// ---- ws:transcript-captions ----
/// The words a cue was generated from: those of the FIRST transcript with a word starting inside the
/// cue's span. ponytail (v1 ceiling): `Cue` carries no clip reference and `project.subtitles` is one
/// flat list, so one active transcript drives karaoke at a time - right for the common case of one
/// caption source, wrong only if two transcribed clips' captions overlap on screen simultaneously.
/// Upgrade path: a `Cue.clip: Option<Id>` field.
fn active_words(project: &Project, cue: &Cue) -> Option<Vec<(f64, f64, String)>> {
    project
        .transcripts
        .iter()
        .map(|tr| tr.words.iter().filter(|w| w.0 >= cue.start - 1e-6 && w.0 < cue.end).cloned().collect::<Vec<_>>())
        .find(|v| !v.is_empty())
}

/// Char ranges `[start, end)` of every whitespace-separated token of `text`, in order.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut n = 0usize;
    for (i, c) in text.chars().enumerate() {
        n = i + 1;
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, n));
    }
    out
}

/// Pure karaoke over one cue: `text`'s k-th token is paired with `words[k]` (the transcript words the
/// cue was cut from, in time order); the word containing `t` is the active one. `Highlight(c)`
/// recolours it, `PopWord` grows it (1.3×, bold), `Typewriter` keeps only the tokens whose word has
/// started. None = nothing to change (plain path): no active word for Highlight/PopWord, or no
/// transcript words. A cue's continuation marks ("…"/" - ") ride along with the token they are glued
/// to; a trailing suffix token past the last word is simply never highlighted.
pub fn karaoke(
    text: &str,
    words: &[(f64, f64, String)],
    t: f64,
    anim: SubtitleAnim,
    base: &TextStyle,
) -> Option<(String, TextStyle)> {
    if words.is_empty() {
        return None;
    }
    let toks = tokens(text);
    let paired = toks.len().min(words.len());
    match anim {
        SubtitleAnim::None => None,
        SubtitleAnim::Typewriter => {
            let shown = words[..paired].iter().take_while(|w| w.0 <= t).count();
            let cut = if shown == 0 { 0 } else { toks[shown - 1].1 };
            Some((text.chars().take(cut).collect(), base.clone()))
        }
        SubtitleAnim::Highlight(_) | SubtitleAnim::PopWord => {
            let k = words[..paired].iter().position(|w| t >= w.0 && t < w.1)?;
            let (start, end) = toks[k];
            let mut style = base.clone();
            let mut span = TextSpan { start, end, ..Default::default() };
            match anim {
                SubtitleAnim::Highlight(color) => span.color = Some(color),
                _ => {
                    // ws:text-titles: base.size is now Animated - sample it at this cue's own time.
                    span.size = Some(base.size.at(t) as f32 * 1.3);
                    span.bold = Some(true);
                }
            }
            style.spans.push(span);
            Some((text.to_string(), style))
        }
    }
}

/// Parse SRT, WebVTT or ASS/SSA (auto-detected: an `[Events]` / `[Script Info]` section is ASS,
/// anything else goes by the "WEBVTT" header / timestamp style). Malformed blocks are skipped.
/// Returns (start, end, text) triples in file order (ASS: sorted by start time).
pub fn parse(text: &str) -> Vec<(f64, f64, String)> {
    let text = text.trim_start_matches('\u{feff}');
    // ---- ws:parity-formats ----
    if is_ass(text) {
        return parse_ass(text);
    }
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
    // VTT cue settings ("align:start position:10%") follow the end time - take the first token
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
pub(crate) fn parse_time(s: &str) -> Option<f64> {
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

// ---- ws:parity-formats ----
/// The file extensions the subtitle importers accept (pickers and the `subtitles.import` tool).
pub const IMPORT_EXTS: &[&str] = &["srt", "vtt", "ass", "ssa"];

/// A subtitle file's text: UTF-8 (BOM or not), UTF-16 with a BOM (what Aegisub and Notepad save
/// ASS files as on Windows), else read byte-for-char as Latin-1 so an old ANSI file still imports.
pub fn read_file(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(decode_text(&bytes))
}

/// `read_file`'s decoding, on bytes.
pub fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        _ => match std::str::from_utf8(bytes) {
            Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        },
    }
}

/// Is this an Advanced SubStation Alpha / SubStation Alpha script? Any `[Script Info]` or `[Events]`
/// section header (case-insensitive, leading whitespace allowed) says so.
pub fn is_ass(text: &str) -> bool {
    text.lines().take(4000).any(|l| {
        let l = l.trim().to_ascii_lowercase();
        l == "[script info]" || l == "[events]"
    })
}

/// The `Dialogue:` lines of an ASS/SSA script's `[Events]` section as (start, end, text). The
/// section's `Format:` line names the columns (the defaults are ASS's own order, which SSA shares
/// for Start / End / Text); the Text column is the rest of the line, commas and all. `Comment:` lines
/// and every other section (styles, fonts, graphics) are skipped. Override blocks (`{\i1}`, `{\pos(..)}`,
/// any `{...}`) are removed, drawing-mode runs (`{\p1}` .. `{\p0}`: vector commands, not words) are
/// dropped, `\N` / `\n` become line breaks and `\h` a space. Cues with no text left are skipped.
/// Sorted by start time (ASS files list lines in any order, by layer or by style).
pub fn parse_ass(text: &str) -> Vec<(f64, f64, String)> {
    let mut out: Vec<(f64, f64, String)> = Vec::new();
    let mut in_events = false;
    let (mut start_i, mut end_i, mut text_i, mut cols) = (1usize, 2usize, 9usize, 10usize);
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_events = trimmed.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once(':') else { continue };
        let key = key.trim();
        if key.eq_ignore_ascii_case("format") {
            let names: Vec<String> = rest.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
            let at = |n: &str| names.iter().position(|x| x == n);
            if let (Some(s), Some(e), Some(t)) = (at("start"), at("end"), at("text")) {
                (start_i, end_i, text_i, cols) = (s, e, t, names.len());
            }
            continue;
        }
        if !key.eq_ignore_ascii_case("dialogue") {
            continue;
        }
        // the Text column is the last one and may itself hold commas
        let fields: Vec<&str> = rest.splitn(cols, ',').collect();
        if fields.len() < cols || text_i >= fields.len() {
            continue;
        }
        let (Some(start), Some(end)) = (parse_time(fields[start_i].trim()), parse_time(fields[end_i].trim())) else {
            continue;
        };
        if end < start {
            continue;
        }
        let body = ass_text(fields[text_i]);
        if body.trim().is_empty() {
            continue;
        }
        out.push((start, end, body));
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// One ASS Text field as plain lines: override blocks removed, drawings dropped, escapes resolved.
fn ass_text(s: &str) -> String {
    let mut out = String::new();
    let mut drawing = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut block = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    block.push(c);
                }
                // `\p<n>` switches drawing mode on (n > 0) or off (0); the last one in the block wins
                let mut rest = block.as_str();
                while let Some(i) = rest.find("\\p") {
                    let after = &rest[i + 2..];
                    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                    // `\pos(..)` / `\pbo` are other tags, not a drawing switch
                    if !digits.is_empty() {
                        drawing = digits.parse::<u32>().unwrap_or(0) > 0;
                    }
                    rest = after;
                }
            }
            '\\' => match chars.peek() {
                Some('N') | Some('n') => {
                    chars.next();
                    if !drawing {
                        out.push('\n');
                    }
                }
                Some('h') => {
                    chars.next();
                    if !drawing {
                        out.push(' ');
                    }
                }
                _ if !drawing => out.push(c),
                _ => {}
            },
            _ if !drawing => out.push(c),
            _ => {}
        }
    }
    // a line break at either end (a `\N` used as spacing) and trailing spaces per line go
    out.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
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

    // ---- ws:parity-formats ----
    #[test]
    fn parses_ass() {
        let ass = "\u{feff}[Script Info]\r\nTitle: test\r\nScriptType: v4.00+\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize\r\nStyle: Default,Arial,20\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:05.00,0:00:06.50,Default,,0,0,0,,Second, with a comma\r\nComment: 0,0:00:00.00,0:00:09.00,Default,,0,0,0,,not shown\r\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\an8}Hello {\\i1}world{\\i0}\\NNext\\hline\r\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,{\\p1}m 0 0 l 100 0 100 100{\\p0}\r\nDialogue: 0,0:00:07.00,0:00:08.00,Default,,0,0,0,,{\\pos(10,20)}Placed\r\nDialogue: 0,bad,0:00:08.00,Default,,0,0,0,,skipped\r\n";
        assert!(is_ass(ass));
        let cues = parse(ass);
        assert_eq!(
            cues,
            vec![
                (1.0, 2.5, "Hello world\nNext line".to_string()),
                (5.0, 6.5, "Second, with a comma".to_string()),
                (7.0, 8.0, "Placed".to_string()),
            ],
            "sorted, override blocks and drawings gone, the drawing-only line skipped"
        );
    }

    #[test]
    fn parses_ssa_with_its_own_format_line() {
        // SSA v4: a Marked column first, and a reordered Format line is honoured
        let ssa = "[Script Info]\nScriptType: v4.00\n\n[V4 Styles]\nFormat: Name, Fontname\n\n[Events]\nFormat: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: Marked=0,0:01:00.25,0:01:02.00,Default,NTP,0000,0000,0000,!Effect,Old {\\b1}school{\\b0}\n";
        assert_eq!(parse(ssa), vec![(60.25, 62.0, "Old school".to_string())]);
        let odd = "[Events]\nFormat: Text, End, Start\nDialogue: a, b, c,0:00:02.00,0:00:01.00\n";
        assert!(parse(odd).is_empty(), "Text is not last: the column count still splits it, nothing parses");
        let reordered = "[Events]\nFormat: Start, End, Text\nDialogue: 0:00:01.00,0:00:02.00,Hi, there\n";
        assert_eq!(parse(reordered), vec![(1.0, 2.0, "Hi, there".to_string())]);
        assert!(!is_ass("1\n00:00:01,000 --> 00:00:02,000\n[Events] in a cue is just text\n"));
        assert_eq!(IMPORT_EXTS, &["srt", "vtt", "ass", "ssa"]);
    }

    #[test]
    fn decodes_utf16_and_ansi_files() {
        let text = "[Events]\nFormat: Start, End, Text\nDialogue: 0:00:01.00,0:00:02.00,Café\n";
        let mut le = vec![0xFF, 0xFE];
        le.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let mut be = vec![0xFE, 0xFF];
        be.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
        for bytes in [le, be, text.as_bytes().to_vec()] {
            assert_eq!(parse(&decode_text(&bytes)), vec![(1.0, 2.0, "Café".to_string())]);
        }
        // Latin-1 / ANSI: 0xE9 alone is not UTF-8
        let ansi: Vec<u8> = text.replace('é', "\u{0}").bytes().map(|b| if b == 0 { 0xE9 } else { b }).collect();
        assert_eq!(parse(&decode_text(&ansi)), vec![(1.0, 2.0, "Café".to_string())]);
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

    #[test]
    fn cue_layer_at_matches_render() {
        let mut p = Project::new();
        p.subtitles = vec![
            Cue { id: 1, start: 0.0, end: 1.0, text: String::new() },
            Cue { id: 2, start: 1.0, end: 2.0, text: "Hello".into() },
        ];
        assert!(cue_layer_at(&p, 0.5).is_none(), "an empty-text cue draws nothing");
        let (text, style) = cue_layer_at(&p, 1.5).expect("a non-empty cue at t=1.5");
        assert_eq!(text, "Hello");
        assert_eq!(style.cache_key(), p.subtitle_style.cache_key(), "style is the project's subtitle style");
        p.show_subtitles = false;
        assert!(cue_layer_at(&p, 1.5).is_none(), "subtitles hidden: nothing, even over a real cue");
    }

    // ---- ws:transcript-captions ----
    fn words(list: &[(f64, f64, &str)]) -> Vec<(f64, f64, String)> {
        list.iter().map(|(a, b, w)| (*a, *b, w.to_string())).collect()
    }

    /// A project with one cue "Hello big world" over 1..4 s and a transcript timing its three words.
    fn karaoke_project() -> Project {
        let mut p = Project::new();
        p.subtitles = vec![Cue { id: 1, start: 1.0, end: 4.0, text: "Hello big\nworld".into() }];
        p.transcripts = vec![
            crate::model::Transcript { clip: 9, words: words(&[(10.0, 11.0, "elsewhere")]) },
            crate::model::Transcript {
                clip: 5,
                words: words(&[(1.0, 1.5, "Hello"), (2.0, 2.5, "big"), (3.0, 3.5, "world")]),
            },
        ];
        p
    }

    #[test]
    fn karaoke_active_word_span_at_t() {
        let mut p = karaoke_project();
        p.subtitle_anim = SubtitleAnim::Highlight([255, 0, 0, 255]);
        let (text, style) = cue_layer_at(&p, 2.2).expect("cue at 2.2");
        assert_eq!(text, "Hello big\nworld", "the text is untouched");
        assert_eq!(style.spans.len(), 1, "one span, the active word");
        let s = &style.spans[0];
        assert_eq!((s.start, s.end), (6, 9), "exactly 'big': {s:?}");
        assert_eq!(s.color, Some([255, 0, 0, 255]));
        // the third word sits after the newline: char indices, not byte-or-line-local
        let (_, style) = cue_layer_at(&p, 3.1).unwrap();
        assert_eq!((style.spans[0].start, style.spans[0].end), (10, 15));
        // a gap between words injects nothing: the plain, borrowed path
        let (_, style) = cue_layer_at(&p, 1.7).unwrap();
        assert!(style.spans.is_empty());
        assert!(matches!(style, Cow::Borrowed(_)));
        // PopWord: bigger and bold over the same range
        p.subtitle_anim = SubtitleAnim::PopWord;
        let (_, style) = cue_layer_at(&p, 2.2).unwrap();
        let s = &style.spans[0];
        assert_eq!((s.start, s.end), (6, 9));
        assert_eq!(s.bold, Some(true));
        assert!((s.size.unwrap() as f64 - p.subtitle_style.size.value * 1.3).abs() < 1e-3);
        // Typewriter: only the words spoken so far
        p.subtitle_anim = SubtitleAnim::Typewriter;
        let (text, style) = cue_layer_at(&p, 2.2).unwrap();
        assert_eq!(text, "Hello big");
        assert!(style.spans.is_empty());
        assert_eq!(cue_layer_at(&p, 3.6).unwrap().0, "Hello big\nworld");
        assert!(cue_layer_at(&p, 0.5).is_none(), "outside every cue");
        // no transcript covers the cue: plain path, whatever the mode
        p.transcripts.clear();
        let (text, style) = cue_layer_at(&p, 2.2).unwrap();
        assert_eq!(text, "Hello big\nworld");
        assert!(matches!(style, Cow::Borrowed(_)));
        // the karaoke fn itself: a trailing continuation token past the last word is never active
        let toks = tokens("…one two - ");
        assert_eq!(toks, vec![(0, 4), (5, 8), (9, 10)]);
        assert!(karaoke("x", &[], 0.0, SubtitleAnim::PopWord, &TextStyle::default()).is_none());
    }

    /// Wave-0b parity: with `SubtitleAnim::None` the result is byte-identical to the pre-refactor
    /// compose.rs inline logic (cue text + the project's subtitle style, borrowed), transcripts or not.
    #[test]
    fn cue_layer_at_matches_old_compose_output_when_anim_none() {
        let p = karaoke_project();
        assert_eq!(p.subtitle_anim, SubtitleAnim::None);
        for t in [1.0, 2.2, 3.9] {
            let (text, style) = cue_layer_at(&p, t).expect("cue");
            let cue = p.cue_at(t).unwrap();
            assert_eq!(text, cue.text.as_str());
            assert_eq!(*style, p.subtitle_style);
            assert!(matches!(text, Cow::Borrowed(_)) && matches!(style, Cow::Borrowed(_)));
        }
        assert!(cue_layer_at(&p, 4.0).is_none());
    }
}
