//! A small, in-house Markdown renderer for planner notes and the What's New window — replaces
//! `egui_commonmark` (which pulled in `egui_commonmark_backend`, `egui_extras` and `pulldown-cmark`,
//! ~2 MB of the release exe) with headings (1-3), bold/italic/code spans, bullet lists, fenced code
//! blocks, links and `---` rules. No tables/images/blockquotes/numbered lists: planner notes never used
//! them (see plans/ui-overhaul/issues/size-diet.md).
//!
//! ponytail: `show` re-parses `src` on every call — no cache field. Notes are short prose and egui
//! re-lays-out every frame regardless; add a hash-keyed `Vec<Block>` cache only if a note's body ever
//! gets long enough for re-parsing to measurably matter.

use crate::theme::Palette;
use eframe::egui::{self, RichText};

/// One inline run within a paragraph or list item.
#[derive(Debug, Clone, PartialEq)]
pub enum Span {
    Plain(String),
    Bold(String),
    Italic(String),
    Code(String),
    /// (display text, url) — opened via `explorer.exe <url>`, the app's existing link-open pattern.
    Link(String, String),
}

/// One block-level element, in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Heading level 1..=3.
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    /// One entry (already inline-parsed) per bullet.
    BulletList(Vec<Vec<Span>>),
    /// Fenced code block, verbatim — no inline spans inside it.
    Code(String),
    /// `---` / `***` / `___` on a line by itself.
    Rule,
}

/// Parse `src` into a flat list of blocks. Pure: no egui, no I/O.
pub fn parse(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            i += 1;
        } else if trimmed.starts_with("```") {
            i += 1;
            let mut body = String::new();
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(lines[i]);
                i += 1;
            }
            i += 1; // skip the closing fence, if any (an unterminated fence just runs to EOF)
            blocks.push(Block::Code(body));
        } else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            blocks.push(Block::Rule);
            i += 1;
        } else if let Some(level) = heading_level(trimmed) {
            let text = trimmed[level as usize + 1..].trim();
            blocks.push(Block::Heading(level, parse_spans(text)));
            i += 1;
        } else if is_bullet(trimmed) {
            let mut items = Vec::new();
            while i < lines.len() && is_bullet(lines[i].trim()) {
                items.push(parse_spans(bullet_text(lines[i].trim())));
                i += 1;
            }
            blocks.push(Block::BulletList(items));
        } else {
            // a paragraph: join contiguous plain lines with a space, like every other markdown renderer
            let mut text = trimmed.to_string();
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                let is_special =
                    t.is_empty() || t.starts_with("```") || t == "---" || heading_level(t).is_some() || is_bullet(t);
                if is_special {
                    break;
                }
                text.push(' ');
                text.push_str(t);
                i += 1;
            }
            blocks.push(Block::Paragraph(parse_spans(&text)));
        }
    }
    blocks
}

/// "# " through "###### " -> 1..=3 (levels past 3 collapse to 3; a note never needs deeper nesting).
fn heading_level(line: &str) -> Option<u8> {
    let n = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&n) && line.as_bytes().get(n) == Some(&b' ') {
        Some(n.min(3) as u8)
    } else {
        None
    }
}

fn is_bullet(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ")
}

fn bullet_text(line: &str) -> &str {
    line[2..].trim()
}

/// `**bold**`, `_italic_`, `` `code` `` and `[text](url)`, scanned left to right with no nesting —
/// everything else is plain text. A note body is short prose, not a spec document.
fn parse_spans(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = find(&chars, i + 2, "**") {
                flush_plain(&mut spans, &mut plain);
                spans.push(Span::Bold(chars[i + 2..end].iter().collect()));
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '_' && chars.get(i + 1).is_some_and(|c| !c.is_whitespace()) {
            if let Some(end) = find(&chars, i + 1, "_") {
                flush_plain(&mut spans, &mut plain);
                spans.push(Span::Italic(chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '`' {
            if let Some(end) = find(&chars, i + 1, "`") {
                flush_plain(&mut spans, &mut plain);
                spans.push(Span::Code(chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(rb) = find(&chars, i + 1, "]") {
                if chars.get(rb + 1) == Some(&'(') {
                    if let Some(rp) = find(&chars, rb + 2, ")") {
                        flush_plain(&mut spans, &mut plain);
                        let label: String = chars[i + 1..rb].iter().collect();
                        let url: String = chars[rb + 2..rp].iter().collect();
                        spans.push(Span::Link(label, url));
                        i = rp + 1;
                        continue;
                    }
                }
            }
        }
        plain.push(chars[i]);
        i += 1;
    }
    flush_plain(&mut spans, &mut plain);
    spans
}

fn flush_plain(spans: &mut Vec<Span>, plain: &mut String) {
    if !plain.is_empty() {
        spans.push(Span::Plain(std::mem::take(plain)));
    }
}

/// First index at/after `from` where `needle` (ASCII) starts, or `None`.
fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || from + needle.len() > chars.len() {
        return None;
    }
    (from..=chars.len() - needle.len()).find(|&i| chars[i..i + needle.len()] == needle[..])
}

/// Paint `src` as rendered markdown into `ui`. `palette` colours the bullet dot and code-block
/// background to match the surrounding custom-painted chrome; everything else follows egui's own
/// (already palette-derived) visuals, same as a plain `ui.label`.
pub fn show(ui: &mut egui::Ui, src: &str, palette: &Palette) {
    for block in parse(src) {
        match block {
            Block::Heading(level, spans) => {
                let size = match level {
                    1 => 20.0,
                    2 => 17.0,
                    _ => 15.0,
                };
                ui.horizontal_wrapped(|ui| show_spans(ui, &spans, size, true));
                ui.add_space(2.0);
            }
            Block::Paragraph(spans) => {
                ui.horizontal_wrapped(|ui| show_spans(ui, &spans, 14.0, false));
            }
            Block::BulletList(items) => {
                for item in &items {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("•").color(palette.text_dim));
                        show_spans(ui, item, 14.0, false);
                    });
                }
            }
            Block::Code(body) => {
                egui::Frame::group(ui.style()).fill(palette.panel).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(body).monospace()));
                });
            }
            Block::Rule => {
                ui.separator();
            }
        }
    }
}

fn show_spans(ui: &mut egui::Ui, spans: &[Span], size: f32, heading: bool) {
    for span in spans {
        let base = |t: &str| {
            let r = RichText::new(t).size(size);
            if heading {
                r.strong()
            } else {
                r
            }
        };
        match span {
            Span::Plain(t) => ui.label(base(t)),
            Span::Bold(t) => ui.label(base(t).strong()),
            Span::Italic(t) => ui.label(base(t).italics()),
            Span::Code(t) => ui.label(base(t).monospace().background_color(ui.visuals().extreme_bg_color)),
            Span::Link(text, url) => {
                let r = ui.link(base(text));
                if r.clicked() {
                    let _ = std::process::Command::new("explorer").arg(url).spawn();
                }
                r
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_levels_1_to_3() {
        let blocks = parse("# One\n## Two\n### Three\n#### Four");
        assert_eq!(blocks[0], Block::Heading(1, vec![Span::Plain("One".into())]));
        assert_eq!(blocks[1], Block::Heading(2, vec![Span::Plain("Two".into())]));
        assert_eq!(blocks[2], Block::Heading(3, vec![Span::Plain("Three".into())]));
        assert_eq!(blocks[3], Block::Heading(3, vec![Span::Plain("Four".into())]), "past 3 collapses to 3");
    }

    #[test]
    fn inline_spans_within_a_paragraph() {
        let blocks = parse("plain **bold** and _italic_ and `code` text");
        let Block::Paragraph(spans) = &blocks[0] else { panic!("expected a paragraph, got {:?}", blocks[0]) };
        assert_eq!(
            spans,
            &vec![
                Span::Plain("plain ".into()),
                Span::Bold("bold".into()),
                Span::Plain(" and ".into()),
                Span::Italic("italic".into()),
                Span::Plain(" and ".into()),
                Span::Code("code".into()),
                Span::Plain(" text".into()),
            ]
        );
    }

    #[test]
    fn bullet_list_items() {
        let blocks = parse("- one\n- two **bold**\n* three");
        let Block::BulletList(items) = &blocks[0] else { panic!("expected a bullet list, got {:?}", blocks[0]) };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], vec![Span::Plain("one".into())]);
        assert_eq!(items[1], vec![Span::Plain("two ".into()), Span::Bold("bold".into())]);
        assert_eq!(items[2], vec![Span::Plain("three".into())]);
    }

    #[test]
    fn fenced_code_block_is_verbatim() {
        let blocks = parse("```\nlet x = 1;\nlet y = **not bold**;\n```");
        assert_eq!(blocks, vec![Block::Code("let x = 1;\nlet y = **not bold**;".into())]);
    }

    #[test]
    fn link_extracts_text_and_url() {
        let blocks = parse("see [the docs](https://example.com/x) for more");
        let Block::Paragraph(spans) = &blocks[0] else { panic!("expected a paragraph") };
        assert!(spans.contains(&Span::Link("the docs".into(), "https://example.com/x".into())));
    }

    #[test]
    fn rule_line() {
        assert_eq!(parse("above\n---\nbelow"), vec![
            Block::Paragraph(vec![Span::Plain("above".into())]),
            Block::Rule,
            Block::Paragraph(vec![Span::Plain("below".into())]),
        ]);
    }

    #[test]
    fn show_runs_without_panicking() {
        let ctx = egui::Context::default();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let src = "# Title\n\nSome **bold**, _italic_, `code` and a [link](https://example.com).\n\n- a\n- b\n\n```\nfenced\n```\n\n---\n";
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, src, &palette));
        });
    }
}
