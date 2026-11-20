//! ---- ws:zones-markers ----
//! Timecodes in text as links that move the playhead: project notes, marker comments, the Project Context
//! pane - any text a user writes about the edit. `find` is the pure parser; `label` draws a text with its
//! timecodes as links; a click lands in `request_seek` and the app seeks on its next frame
//! (`App` FRAME_HOOKS: `tools_zones::frame`), so a widget deep in a pane needs no handle on the player.
//!
//! What counts as a timecode: `M:SS`, `MM:SS`, `H:MM:SS` (also `HH:`), each optionally with a decimal part
//! (`1:23.5`). Seconds (and minutes in the hour form) are two digits below 60. It must stand on its own:
//! no digit, letter, `:` or `.` right before it, no digit or `:` right after it - so `16:9`, `10:1:23`
//! and `v1:23a` are not timecodes.

use eframe::egui;
use std::ops::Range;

/// Every timecode in `text`: its byte range and its time in seconds, in order.
pub fn find(text: &str) -> Vec<(Range<usize>, f64)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b':' || b[i - 1] == b'.');
        if b[i].is_ascii_digit() && before_ok {
            if let Some((end, t)) = parse_at(b, i) {
                out.push((i..end, t));
                i = end;
                continue;
            }
        }
        // skip the rest of a run of digits, so "123:45" is not read from its middle
        if b[i].is_ascii_digit() {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The seconds of `text` when the whole of it is one timecode (the tests pin `find` with it).
#[cfg(test)]
pub fn parse(text: &str) -> Option<f64> {
    let t = text.trim();
    match find(t).as_slice() {
        [(r, s)] if r.start == 0 && r.end == t.len() => Some(*s),
        _ => None,
    }
}

/// Digits at `b[i..]`: (end, value) of the run, at most `max` of them.
fn digits(b: &[u8], i: usize, max: usize) -> Option<(usize, u64)> {
    let mut j = i;
    let mut v = 0u64;
    while j < b.len() && b[j].is_ascii_digit() && j - i < max {
        v = v * 10 + (b[j] - b'0') as u64;
        j += 1;
    }
    (j > i).then_some((j, v))
}

/// A timecode starting exactly at `i`.
fn parse_at(b: &[u8], i: usize) -> Option<(usize, f64)> {
    let (j, first) = digits(b, i, 3)?;
    if b.get(j) != Some(&b':') {
        return None;
    }
    let two = |at: usize| -> Option<(usize, u64)> {
        let (e, v) = digits(b, at, 2)?;
        (e - at == 2 && v < 60).then_some((e, v))
    };
    let (mut end, second) = two(j + 1)?;
    let mut secs = if b.get(end) == Some(&b':') {
        // H:MM:SS - the first field is hours (at most two digits), the second minutes
        let (e, third) = two(end + 1)?;
        if j - i > 2 {
            return None;
        }
        end = e;
        first as f64 * 3600.0 + second as f64 * 60.0 + third as f64
    } else {
        first as f64 * 60.0 + second as f64
    };
    // a decimal part: `.` then digits (a full stop ending a sentence is not one)
    if b.get(end) == Some(&b'.') && b.get(end + 1).is_some_and(|c| c.is_ascii_digit()) {
        let (e, _) = digits(b, end + 1, 3)?;
        let frac: f64 = std::str::from_utf8(&b[end..e]).ok()?.parse().ok()?;
        secs += frac;
        end = e;
    }
    // must end here: not followed by more digits or another field
    if b.get(end).is_some_and(|c| c.is_ascii_digit() || *c == b':') {
        return None;
    }
    Some((end, secs))
}

/// The `markdown::Span::Link` url a timecode link carries.
pub const SEEK_SCHEME: &str = "seek:";

/// Ask the app to move the playhead to `t` on its next frame.
pub fn request_seek(ctx: &egui::Context, t: f64) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("timecode_links_seek"), t));
    ctx.request_repaint();
}

/// The seek a link asked for since the last call (the app's frame hook takes it).
pub fn take_seek(ctx: &egui::Context) -> Option<f64> {
    ctx.data_mut(|d| d.remove_temp::<f64>(egui::Id::new("timecode_links_seek")))
}

/// One timecode as a link: a click asks for the seek. Returns the click's response.
pub fn link(ui: &mut egui::Ui, text: impl Into<egui::RichText>, t: f64) -> egui::Response {
    let r = ui.link(text.into()).on_hover_text("Move the playhead here");
    if r.clicked() {
        request_seek(ui.ctx(), t);
    }
    r
}

/// `text` as a wrapped paragraph with its timecodes as links (plain text when it has none). Returns the
/// time of a link clicked this frame.
pub fn label(ui: &mut egui::Ui, text: &str) -> Option<f64> {
    let found = find(text);
    if found.is_empty() {
        ui.add(egui::Label::new(text).wrap());
        return None;
    }
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut at = 0;
        for (r, t) in &found {
            if r.start > at {
                ui.label(&text[at..r.start]);
            }
            if link(ui, &text[r.clone()], *t).clicked() {
                clicked = Some(*t);
            }
            at = r.end;
        }
        if at < text.len() {
            ui.label(&text[at..]);
        }
    });
    clicked
}

/// Pieces of `text`: (piece, Some(seconds) for a timecode). For renderers with their own spans (markdown).
pub fn pieces(text: &str) -> Vec<(&str, Option<f64>)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (r, t) in find(text) {
        if r.start > at {
            out.push((&text[at..r.start], None));
        }
        out.push((&text[r.clone()], Some(t)));
        at = r.end;
    }
    if at < text.len() {
        out.push((&text[at..], None));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(s: &str) -> Vec<(&str, f64)> {
        find(s).into_iter().map(|(r, t)| (&s[r], t)).collect()
    }

    #[test]
    fn the_three_forms_and_decimals() {
        assert_eq!(times("cut at 1:23 please"), vec![("1:23", 83.0)]);
        assert_eq!(times("01:02:03"), vec![("01:02:03", 3723.0)]);
        assert_eq!(times("see 1:23.5, then 12:00."), vec![("1:23.5", 83.5), ("12:00", 720.0)]);
        assert_eq!(times("1:02:03.25"), vec![("1:02:03.25", 3723.25)]);
        assert_eq!(times("(0:05)"), vec![("0:05", 5.0)]);
        assert_eq!(times("90:00"), vec![("90:00", 5400.0)], "minutes past an hour");
    }

    #[test]
    fn things_that_are_not_timecodes() {
        for s in ["16:9", "1:60", "10:1:23", "v1:23", "1:234", "x.1:23", "123:45:00", "1:2", ":30", "1:23:4"] {
            assert!(times(s).is_empty(), "{s} -> {:?}", times(s));
        }
        assert_eq!(times("1:23a"), vec![("1:23", 83.0)], "a letter after is fine (1:23s)");
        assert_eq!(parse(" 2:30 "), Some(150.0));
        assert_eq!(parse("2:30 x"), None);
    }

    #[test]
    fn pieces_cover_the_text() {
        let s = "intro 0:10 to 0:20 end";
        let p = pieces(s);
        assert_eq!(p.iter().map(|(t, _)| *t).collect::<String>(), s);
        assert_eq!(p.iter().filter_map(|(_, t)| *t).collect::<Vec<_>>(), vec![10.0, 20.0]);
        // multi-byte text around a timecode keeps the byte ranges on char boundaries
        assert_eq!(times("é 1:00 ü"), vec![("1:00", 60.0)]);
    }

    #[test]
    fn a_click_reaches_the_app_through_the_context() {
        let ctx = egui::Context::default();
        request_seek(&ctx, 12.5);
        assert_eq!(take_seek(&ctx), Some(12.5));
        assert_eq!(take_seek(&ctx), None, "taken once");
    }
}
