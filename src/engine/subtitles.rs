//! Subtitle file formats: SubRip (.srt) and WebVTT (.vtt) — parse (auto-detected) and write.
//! Basic formatting tags (<i>, <b>, {\an8}, VTT cue settings) are stripped on import; text is kept
//! as plain lines. Times are seconds.

use crate::model::{Cue, Project, SubtitleAnim, TextSpan, TextStyle};
use std::borrow::Cow;

// ---- ws:registries-schema-hooks ----
/// The subtitle cue (if any) drawn at timeline time `t`: the cue's text and the project's subtitle
/// style, factoring the cue-lookup + empty-text check duplicated between `engine::compose::render`
/// and `playback`'s layer builder. Borrowed (`Cow::Borrowed`) on the plain path — `compose::render`'s
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
/// flat list, so one active transcript drives karaoke at a time — right for the common case of one
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
/// transcript words. A cue's continuation marks ("…"/" —") ride along with the token they are glued
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
                    span.size = Some(base.size * 1.3);
                    span.bold = Some(true);
                }
            }
            style.spans.push(span);
            Some((text.to_string(), style))
        }
    }
}

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
        assert!((s.size.unwrap() - p.subtitle_style.size * 1.3).abs() < 1e-3);
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
        let toks = tokens("…one two —");
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
