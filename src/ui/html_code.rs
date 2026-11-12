//! ---- ws:html-layer ----
//! The code editor of an HTML layer: syntax colours and a completion list.
//!
//! `tokens` is a single hand-written pass over the file (HTML tags and attributes, comments, the CSS in
//! `<style>`: selectors, properties, values, at-rules, and `{{field}}` placeholders anywhere); the
//! colours and the completions both read its result. `complete` is a context-aware word completer, not
//! a language server: it knows which HTML tags / attributes / `se:` meta names / CSS properties / CSS
//! values suit the spot the cursor is in, plus this file's own classes, `@keyframes` names, custom
//! properties and `se:field` names, and does not validate anything.
//! `code_edit` is the widget (a `TextEdit` with the colours, and the list under the cursor: Up / Down
//! pick, Tab or Enter accept, Esc closes, Ctrl+Space asks).

use eframe::egui::{self, text::LayoutJob, text::TextFormat, Color32, FontId, Id, Key, Modifiers};
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Punct,
    Tag,
    Attr,
    Str,
    Comment,
    Field,
    AtRule,
    Selector,
    Prop,
    Num,
    Func,
    Keyword,
}

type Tok = (Range<usize>, Kind);

fn find_ci(text: &str, needle: &str, from: usize) -> Option<usize> {
    let (h, n) = (text.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Every coloured stretch of `text`, in order (what is not listed is `Plain`).
pub fn tokens(text: &str) -> Vec<Tok> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < n {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if text[i..].starts_with("<!--") {
            let end = text[i..].find("-->").map_or(n, |e| i + e + 3);
            out.push((i..end, Kind::Comment));
            i = end;
            continue;
        }
        let mut j = i + 1;
        let closing = j < n && b[j] == b'/';
        if closing {
            j += 1;
        }
        let bang = j < n && b[j] == b'!';
        if !(j < n && (b[j].is_ascii_alphabetic() || bang)) {
            i += 1;
            continue;
        }
        out.push((i..j + usize::from(bang), Kind::Punct));
        j += usize::from(bang);
        let name_start = j;
        if bang {
            while j < n && b[j] != b'>' && !b[j].is_ascii_whitespace() {
                j += 1;
            }
        } else {
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b':') {
                j += 1;
            }
        }
        out.push((name_start..j, Kind::Tag));
        let name = text[name_start..j].to_ascii_lowercase();
        loop {
            while j < n && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= n {
                break;
            }
            match b[j] {
                b'>' => {
                    out.push((j..j + 1, Kind::Punct));
                    j += 1;
                    break;
                }
                b'/' | b'=' => {
                    out.push((j..j + 1, Kind::Punct));
                    j += 1;
                }
                q @ (b'"' | b'\'') => {
                    let s = j;
                    j += 1;
                    while j < n && b[j] != q {
                        j += 1;
                    }
                    j = (j + 1).min(n);
                    out.push((s..j, Kind::Str));
                }
                _ => {
                    let s = j;
                    while j < n && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/' | b'"' | b'\'') {
                        j += 1;
                    }
                    out.push((s..j.max(s + 1), Kind::Attr));
                    j = j.max(s + 1);
                }
            }
        }
        i = j;
        if !closing && name == "style" {
            let end = find_ci(text, "</style", i).unwrap_or(n);
            css(text, i, end, &mut out);
            i = end;
        }
    }
    out
}

fn skip_string(b: &[u8], mut j: usize, e: usize) -> usize {
    let q = b[j];
    j += 1;
    while j < e && b[j] != q {
        j += 1;
    }
    (j + 1).min(e)
}

/// The CSS between `<style>` and `</style>`.
fn css(text: &str, s: usize, e: usize, out: &mut Vec<Tok>) {
    let b = text.as_bytes();
    let mut i = s;
    let mut depth = 0usize;
    while i < e {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = text[i..e].find("*/").map_or(e, |x| i + x + 2);
            out.push((i..end, Kind::Comment));
            i = end;
        } else if matches!(c, b'{' | b'}' | b';' | b',') {
            out.push((i..i + 1, Kind::Punct));
            match c {
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
            i += 1;
        } else if c == b'@' {
            let mut j = i + 1;
            while j < e && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            out.push((i..j, Kind::AtRule));
            i = j;
        } else {
            // up to the next `{ } ; :` outside parentheses and strings: what is it?
            let mut j = i;
            let mut paren = 0i32;
            while j < e {
                match b[j] {
                    b'(' => paren += 1,
                    b')' => paren -= 1,
                    b'"' | b'\'' => {
                        j = skip_string(b, j, e);
                        continue;
                    }
                    b'{' | b'}' | b';' | b':' if paren <= 0 => break,
                    _ => {}
                }
                j += 1;
            }
            let word = text[i..j].trim_end();
            let end = i + word.len();
            let term = if j < e { b[j] } else { b'}' };
            let ident = !word.is_empty()
                && word.bytes().all(|x| x.is_ascii_alphanumeric() || x == b'-' || x == b'_' || x == b'\\');
            if term == b':' && depth >= 1 && ident {
                out.push((i..end, Kind::Prop));
                out.push((j..j + 1, Kind::Punct));
                // the value, up to ; or the closing }
                let (vs, mut k, mut p2) = (j + 1, j + 1, 0i32);
                while k < e {
                    match b[k] {
                        b'(' => p2 += 1,
                        b')' => p2 -= 1,
                        b'"' | b'\'' => {
                            k = skip_string(b, k, e);
                            continue;
                        }
                        b';' | b'}' if p2 <= 0 => break,
                        _ => {}
                    }
                    k += 1;
                }
                css_value(text, vs, k, out);
                i = k;
            } else if term == b':' {
                // a selector with a pseudo-class (a:hover, ::before): on to the `{`
                let mut k = j;
                while k < e && b[k] != b'{' && b[k] != b';' {
                    k += 1;
                }
                let sel = text[i..k].trim_end();
                out.push((i..i + sel.len(), Kind::Selector));
                i += sel.len().max(1);
            } else {
                // `{` after it: a selector or keyframe step; no terminator at depth: a property being typed
                let kind = if term == b'{' || depth == 0 { Kind::Selector } else { Kind::Prop };
                if !word.is_empty() {
                    out.push((i..end, kind));
                }
                i = j.max(i + 1);
            }
        }
    }
}

fn css_value(text: &str, s: usize, e: usize, out: &mut Vec<Tok>) {
    let b = text.as_bytes();
    let mut i = s;
    while i < e {
        let c = b[i];
        if c.is_ascii_whitespace() || matches!(c, b',' | b'(' | b')' | b'/' | b'+' | b'*') {
            i += 1;
        } else if c == b'"' || c == b'\'' {
            let end = skip_string(b, i, e);
            out.push((i..end, Kind::Str));
            i = end;
        } else if c == b'#' || c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) || (c == b'-' && b.get(i + 1).is_some_and(|x| x.is_ascii_digit() || *x == b'.')) {
            let mut j = i + 1;
            while j < e && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'%') {
                j += 1;
            }
            out.push((i..j, Kind::Num));
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'-' || c == b'!' {
            let mut j = i + 1;
            while j < e && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'_') {
                j += 1;
            }
            let func = j < e && b[j] == b'(';
            out.push((i..j, if func { Kind::Func } else { Kind::Keyword }));
            i = j;
        } else {
            i += 1;
        }
    }
}

/// `{{name}}` placeholders (a short run of word characters between the braces).
fn field_ranges(text: &str) -> Vec<Range<usize>> {
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(s) = text[from..].find("{{") {
        let s = from + s;
        match text[s + 2..].find("}}") {
            Some(e) if e <= 64 && !text[s + 2..s + 2 + e].contains('\n') => {
                v.push(s..s + 2 + e + 2);
                from = s + 2 + e + 2;
            }
            _ => from = s + 2,
        }
    }
    v
}

fn colour(k: Kind, dark: bool, base: Color32) -> Color32 {
    let c = |d: [u8; 3], l: [u8; 3]| {
        let [r, g, b] = if dark { d } else { l };
        Color32::from_rgb(r, g, b)
    };
    match k {
        Kind::Plain => base,
        Kind::Punct => c([0x80, 0x88, 0x98], [0x60, 0x60, 0x60]),
        Kind::Tag => c([0x56, 0x9c, 0xd6], [0x80, 0x00, 0x00]),
        Kind::Attr => c([0x9c, 0xdc, 0xfe], [0xe5, 0x00, 0x00]),
        Kind::Str => c([0xce, 0x91, 0x78], [0x0a, 0x4b, 0xa5]),
        Kind::Comment => c([0x6a, 0x99, 0x55], [0x00, 0x80, 0x00]),
        Kind::Field => c([0xe0, 0x7b, 0xe0], [0xaf, 0x00, 0xdb]),
        Kind::AtRule => c([0xc5, 0x86, 0xc0], [0xaf, 0x00, 0xdb]),
        Kind::Selector => c([0xd7, 0xba, 0x7d], [0x80, 0x00, 0x00]),
        Kind::Prop => c([0x9c, 0xdc, 0xfe], [0xe5, 0x00, 0x00]),
        Kind::Num => c([0xb5, 0xce, 0xa8], [0x09, 0x86, 0x58]),
        Kind::Func => c([0xdc, 0xdc, 0xaa], [0x79, 0x5e, 0x26]),
        Kind::Keyword => c([0x4e, 0xc9, 0xb0], [0x0a, 0x4b, 0xa5]),
    }
}

/// The coloured layout of `text` (every byte of it is in exactly one section, in order).
pub fn highlight(text: &str, font: &FontId, dark: bool, base: Color32) -> LayoutJob {
    let mut kinds = vec![Kind::Plain; text.len()];
    for (r, k) in tokens(text) {
        for x in &mut kinds[r.start.min(text.len())..r.end.min(text.len())] {
            *x = k;
        }
    }
    for r in field_ranges(text) {
        for x in &mut kinds[r] {
            *x = Kind::Field;
        }
    }
    let mut job = LayoutJob::default();
    let mut s = 0;
    for i in 1..=text.len() {
        let boundary = i == text.len() || (kinds[i] != kinds[s] && text.is_char_boundary(i));
        if boundary {
            job.append(&text[s..i], 0.0, TextFormat::simple(font.clone(), colour(kinds[s], dark, base)));
            s = i;
        }
    }
    job
}

// ---------------------------------------------------------------------------------------------------
// completion data

const TAGS: &[&str] = &[
    "a", "abbr", "article", "aside", "b", "blockquote", "body", "br", "button", "circle", "clipPath", "code", "defs",
    "desc", "details", "div", "ellipse", "em", "figure", "filter", "footer", "g", "h1", "h2", "h3", "h4", "h5", "h6",
    "head", "header", "hr", "html", "i", "img", "li", "line", "linearGradient", "main", "mark", "meta", "nav", "ol",
    "p", "path", "pattern", "polygon", "polyline", "radialGradient", "rect", "section", "small", "span", "stop",
    "strong", "style", "sub", "sup", "svg", "text", "textPath", "title", "tspan", "u", "ul", "use",
];

const ATTRS: &[&str] = &[
    "class", "id", "style", "title", "lang", "src", "alt", "href", "width", "height", "viewBox", "xmlns", "d", "fill",
    "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "stroke-dasharray", "stroke-dashoffset",
    "fill-rule", "opacity", "transform", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "points",
    "offset", "stop-color", "stop-opacity", "clip-path", "gradientUnits", "preserveAspectRatio", "content", "name",
    "charset", "dir", "tabindex",
];

const META_NAMES: &[(&str, &str)] = &[
    ("se:kind", "layer | scene"),
    ("se:desc", "one sentence"),
    ("se:tags", "a, b, c"),
    ("se:size", "WxH in CSS px"),
    ("se:duration", "4s"),
    ("se:thumb", "second"),
    ("se:field", "name | type | default"),
];

const PROPS: &[&str] = &[
    "align-content", "align-items", "align-self", "animation", "animation-delay", "animation-direction",
    "animation-duration", "animation-fill-mode", "animation-iteration-count", "animation-name",
    "animation-play-state", "animation-timing-function", "aspect-ratio", "backdrop-filter", "background",
    "background-clip", "background-color", "background-image", "background-position", "background-repeat",
    "background-size", "border", "border-bottom", "border-color", "border-left", "border-radius", "border-right",
    "border-style", "border-top", "border-width", "bottom", "box-shadow", "box-sizing", "clip-path", "color",
    "column-gap", "content", "counter-increment", "counter-reset", "display", "filter", "flex", "flex-basis",
    "flex-direction", "flex-grow", "flex-shrink", "flex-wrap", "font", "font-family", "font-size", "font-style",
    "font-weight", "gap", "grid", "grid-template-columns", "grid-template-rows", "height", "inset", "isolation",
    "justify-content", "justify-items", "left", "letter-spacing", "line-height", "margin", "margin-bottom",
    "margin-left", "margin-right", "margin-top", "mask", "max-height", "max-width", "min-height", "min-width",
    "mix-blend-mode", "object-fit", "offset-path", "opacity", "order", "outline", "outline-offset", "overflow",
    "overflow-x", "overflow-y", "padding", "padding-bottom", "padding-left", "padding-right", "padding-top",
    "paint-order", "perspective", "place-items", "pointer-events", "position", "right", "rotate", "row-gap", "scale",
    "text-align", "text-decoration", "text-overflow", "text-shadow", "text-transform", "top", "transform",
    "transform-origin", "transform-style", "transition", "translate", "vertical-align", "visibility", "white-space",
    "width", "word-break", "word-spacing", "z-index", "-webkit-background-clip", "-webkit-text-fill-color",
    "-webkit-text-stroke", "-webkit-text-stroke-color", "-webkit-text-stroke-width",
];

const AT_RULES: &[&str] = &["@keyframes", "@property", "@media", "@supports", "@layer"];

const GENERIC_VALUES: &[&str] = &[
    "inherit", "initial", "unset", "none", "auto", "transparent", "currentColor", "var(", "calc(", "rgb(", "rgba(",
    "hsl(", "hsla(", "linear-gradient(", "radial-gradient(", "conic-gradient(",
];

const FONTS: &[&str] = &[
    "\"Segoe UI\"", "Arial", "\"Arial Black\"", "Impact", "Bahnschrift", "\"Trebuchet MS\"", "Verdana", "Georgia",
    "\"Times New Roman\"", "Consolas", "\"Courier New\"", "\"Comic Sans MS\"", "\"Segoe UI Emoji\"", "sans-serif",
    "serif", "monospace",
];

fn values_for(prop: &str) -> &'static [&'static str] {
    match prop {
        "display" => &["flex", "grid", "block", "inline", "inline-block", "inline-flex", "none", "contents"],
        "position" => &["absolute", "relative", "fixed", "static", "sticky"],
        "text-align" => &["left", "center", "right", "justify"],
        "align-items" | "justify-content" | "align-content" | "place-items" | "justify-items" | "align-self" => {
            &["center", "flex-start", "flex-end", "space-between", "space-around", "space-evenly", "stretch", "baseline", "start", "end"]
        }
        "flex-direction" => &["row", "column", "row-reverse", "column-reverse"],
        "flex-wrap" => &["wrap", "nowrap", "wrap-reverse"],
        "font-weight" => &["normal", "bold", "bolder", "lighter", "100", "400", "500", "600", "700", "800", "900"],
        "font-style" => &["normal", "italic", "oblique"],
        "text-transform" => &["uppercase", "lowercase", "capitalize"],
        "text-decoration" => &["underline", "line-through", "overline"],
        "white-space" => &["nowrap", "normal", "pre", "pre-wrap", "pre-line"],
        "overflow" | "overflow-x" | "overflow-y" => &["hidden", "visible", "scroll", "clip"],
        "animation-fill-mode" => &["both", "forwards", "backwards"],
        "animation-direction" => &["normal", "reverse", "alternate", "alternate-reverse"],
        "animation-iteration-count" => &["infinite", "1", "2", "3"],
        "animation-play-state" => &["running", "paused"],
        "animation-timing-function" | "transition-timing-function" => {
            &["linear", "ease", "ease-in", "ease-out", "ease-in-out", "cubic-bezier(", "steps("]
        }
        "animation" | "transition" => &[
            "both", "forwards", "backwards", "infinite", "alternate", "linear", "ease", "ease-in", "ease-out",
            "ease-in-out", "cubic-bezier(", "steps(",
        ],
        "mix-blend-mode" => &["multiply", "screen", "overlay", "darken", "lighten", "color-dodge", "color-burn", "hard-light", "soft-light", "difference", "exclusion"],
        "background-repeat" => &["no-repeat", "repeat", "repeat-x", "repeat-y"],
        "background-size" | "object-fit" => &["cover", "contain", "fill", "scale-down"],
        "background-clip" | "-webkit-background-clip" => &["text", "border-box", "padding-box", "content-box"],
        "box-sizing" => &["border-box", "content-box"],
        "visibility" => &["visible", "hidden"],
        "paint-order" => &["stroke fill", "fill stroke", "markers", "normal"],
        "pointer-events" => &["auto"],
        "transform-origin" => &["center", "top", "bottom", "left", "right"],
        "transform" | "translate" | "rotate" | "scale" => {
            &["translate(", "translateX(", "translateY(", "rotate(", "scale(", "scaleX(", "scaleY(", "skew(", "perspective("]
        }
        "font-family" | "font" => FONTS,
        "border" | "border-top" | "border-bottom" | "border-left" | "border-right" | "outline" => {
            &["solid", "dashed", "dotted", "double"]
        }
        _ => &[],
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// What is shown, and what replaces the typed word.
    pub label: String,
    /// A word on what it is.
    pub detail: String,
    /// Text that goes in after the label (`=""` for an attribute, `: ` for a property).
    pub suffix: &'static str,
    /// Characters the cursor steps back after the insert (to sit between the quotes).
    pub back: usize,
}

fn item(label: impl Into<String>, detail: impl Into<String>) -> Item {
    Item { label: label.into(), detail: detail.into(), suffix: "", back: 0 }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// Byte index where the word being completed starts.
    pub start: usize,
    pub items: Vec<Item>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '@' | '.' | '#')
}

/// (start byte, word) of the word that ends at `cursor`; `se:` names keep their colon.
fn word_before(text: &str, cursor: usize) -> (usize, &str) {
    let mut start = cursor;
    for (i, c) in text[..cursor].char_indices().rev() {
        if is_word(c) {
            start = i;
        } else {
            break;
        }
    }
    if start >= 3 && &text[start - 3..start] == "se:" && text[..start - 3].ends_with('"') {
        start -= 3;
    }
    (start, &text[start..cursor])
}

fn matching<'a>(list: impl IntoIterator<Item = (&'a str, &'a str)>, prefix: &str) -> Vec<Item> {
    let p = prefix.to_ascii_lowercase();
    list.into_iter()
        .filter(|(l, _)| l.to_ascii_lowercase().starts_with(&p))
        .map(|(l, d)| item(l, d))
        .collect()
}

/// Class names, ids, `@keyframes` names and `--custom` properties this file already uses.
fn names_in(text: &str) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let (mut classes, mut ids, mut frames, mut vars) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let push = |v: &mut Vec<String>, s: &str| {
        if !s.is_empty() && !s.contains("{{") && !v.iter().any(|x| x == s) {
            v.push(s.to_string());
        }
    };
    let mut rest = text;
    while let Some(i) = rest.find("class=\"") {
        let tail = &rest[i + 7..];
        let end = tail.find('"').unwrap_or(tail.len());
        for c in tail[..end].split_whitespace() {
            push(&mut classes, c);
        }
        rest = &tail[end..];
    }
    // classes the stylesheet defines (`.name` inside <style>)
    let mut from = 0;
    while let Some(o) = find_ci(text, "<style", from) {
        let end = find_ci(text, "</style", o).unwrap_or(text.len());
        let css = &text[o..end];
        let cb = css.as_bytes();
        for (k, _) in css.match_indices('.') {
            let starts = k == 0 || !(cb[k - 1].is_ascii_alphanumeric() || cb[k - 1] == b'-' || cb[k - 1] == b'_');
            let tail = &css[k + 1..];
            if starts && tail.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                let len = tail.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).unwrap_or(tail.len());
                push(&mut classes, &tail[..len]);
            }
        }
        from = end.max(o + 1);
    }
    let mut rest = text;
    while let Some(i) = rest.find("id=\"") {
        let tail = &rest[i + 4..];
        let end = tail.find('"').unwrap_or(tail.len());
        push(&mut ids, &tail[..end]);
        rest = &tail[end..];
    }
    let mut rest = text;
    while let Some(i) = rest.find("@keyframes") {
        let tail = rest[i + 10..].trim_start();
        let end = tail.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).unwrap_or(tail.len());
        push(&mut frames, &tail[..end]);
        rest = &rest[i + 10..];
    }
    let mut rest = text;
    while let Some(i) = rest.find("--") {
        let tail = &rest[i..];
        let end = tail[2..].find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).map_or(tail.len(), |e| e + 2);
        if end > 2 {
            push(&mut vars, &tail[..end]);
        }
        rest = &rest[i + end.max(2)..];
    }
    (classes, ids, frames, vars)
}

/// The tag a position inside an open `<tag ...` belongs to, and the attribute whose `"` value we are in.
fn open_tag(before: &str) -> Option<(String, Option<String>)> {
    let lt = before.rfind('<')?;
    if before[lt..].contains('>') {
        return None;
    }
    let tag: String = before[lt + 1..].chars().take_while(|c| c.is_alphanumeric() || *c == '-' || *c == ':').collect();
    let body = &before[lt..];
    let quotes = body.matches('"').count();
    let attr = (quotes % 2 == 1).then(|| {
        let q = body.rfind('"').unwrap_or(0);
        let head = body[..q].trim_end().trim_end_matches('=').trim_end();
        head.rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").to_string()
    });
    Some((tag, attr))
}

/// The CSS property a value at `cursor` belongs to (the word before the nearest `:` in this declaration).
fn value_prop(before: &str) -> Option<String> {
    let cut = before.rfind([';', '{', '}'])?;
    let decl = &before[cut + 1..];
    let colon = decl.find(':')?;
    Some(decl[..colon].trim().to_ascii_lowercase())
}

fn in_css(before: &str) -> bool {
    match (find_last_ci(before, "<style"), find_last_ci(before, "</style")) {
        (Some(open), close) => {
            close.is_none_or(|c| c < open) && before[open..].contains('>')
        }
        _ => false,
    }
}

fn find_last_ci(text: &str, needle: &str) -> Option<usize> {
    let (h, n) = (text.as_bytes(), needle.as_bytes());
    if h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).rev().find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// What fits at `cursor` (a byte index): None when nothing should be offered. `force` (Ctrl+Space)
/// offers a list even with nothing typed yet.
pub fn complete(text: &str, cursor: usize, force: bool) -> Option<Completion> {
    let cursor = cursor.min(text.len());
    if !text.is_char_boundary(cursor) {
        return None;
    }
    let (start, prefix) = word_before(text, cursor);
    let before = &text[..start];
    let done = |items: Vec<Item>| {
        let exact = items.len() == 1 && items[0].label == prefix;
        (!items.is_empty() && !exact).then_some(Completion { start, items })
    };
    // {{field}}
    if before.ends_with("{{") {
        let tpl = crate::engine::html::parse(text);
        let items = tpl.fields.iter().map(|f| (f.name.as_str(), format!("{:?}", f.ty).to_lowercase())).collect::<Vec<_>>();
        let mut items = matching(items.iter().map(|(a, b)| (*a, b.as_str())), prefix);
        for it in &mut items {
            it.detail = format!("field, {}", it.detail);
        }
        return done(items);
    }
    if prefix.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    // the tokens up to the cursor say what the word is: the file may be unfinished after it
    let upto = &text[..cursor];
    let toks = tokens(upto);
    let probe = if prefix.is_empty() { cursor.saturating_sub(1) } else { start };
    let kind = toks.iter().find(|(r, _)| r.contains(&probe)).map_or(Kind::Plain, |(_, k)| *k);

    let css = in_css(upto);
    if prefix.is_empty() {
        // nothing typed: only after `<` (or on request)
        if before.ends_with('<') || before.ends_with("</") {
            return done(matching(TAGS.iter().map(|t| (*t, "tag")), ""));
        }
        if !force {
            return None;
        }
    }
    let (classes, ids, frames, vars) = names_in(text);
    let attr_items = |tag: &str| -> Vec<Item> {
        let mut v: Vec<Item> = ATTRS
            .iter()
            .filter(|a| ATTRS.contains(a) && (tag != "meta" || matches!(**a, "name" | "content" | "charset")))
            .filter(|a| a.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()))
            .map(|a| Item { label: a.to_string(), detail: "attribute".into(), suffix: "=\"\"", back: 1 })
            .collect();
        v.sort_by_key(|i| i.label.len());
        v
    };
    if prefix.is_empty() {
        // asked for inside an open tag, after a space: its attributes
        if let Some((tag, None)) = open_tag(upto) {
            return done(attr_items(&tag));
        }
    }
    match kind {
        Kind::Tag => done(matching(TAGS.iter().map(|t| (*t, "tag")), prefix)),
        Kind::Attr | Kind::Punct if open_tag(upto).is_some_and(|(_, a)| a.is_none()) && !(kind == Kind::Punct && !force) => {
            let (tag, _) = open_tag(upto)?;
            done(attr_items(&tag))
        }
        Kind::Str => {
            let (tag, attr) = open_tag(upto)?;
            match (tag.as_str(), attr.as_deref()) {
                ("meta", Some("name")) => done(matching(META_NAMES.iter().map(|(a, b)| (*a, *b)), prefix)),
                (_, Some("class")) => done(matching(classes.iter().map(|c| (c.as_str(), "class")), prefix)),
                (_, Some("id")) => done(matching(ids.iter().map(|c| (c.as_str(), "id")), prefix)),
                _ => None,
            }
        }
        _ if css => {
            if prefix.starts_with('@') {
                return done(matching(AT_RULES.iter().map(|a| (*a, "at-rule")), prefix));
            }
            if prefix.starts_with("--") && value_prop(before).is_some() {
                return done(matching(vars.iter().map(|v| (v.as_str(), "custom property")), prefix));
            }
            let value_ctx = matches!(kind, Kind::Keyword | Kind::Num | Kind::Func | Kind::Str)
                || (prefix.is_empty() && before.trim_end().ends_with(':'));
            if value_ctx {
                let prop = value_prop(before)?;
                let mut list: Vec<(&str, &str)> = values_for(&prop).iter().map(|v| (*v, "value")).collect();
                if prop == "animation" || prop == "animation-name" {
                    list.extend(frames.iter().map(|f| (f.as_str(), "@keyframes")));
                }
                list.extend(GENERIC_VALUES.iter().map(|v| (*v, "value")));
                if prop == "font-family" {
                    list.retain(|(_, d)| *d == "value");
                }
                let mut seen = Vec::new();
                list.retain(|(l, _)| !seen.contains(l) && {
                    seen.push(*l);
                    true
                });
                if prefix.starts_with("var") || prefix.starts_with("--") {
                    list.extend(vars.iter().map(|v| (v.as_str(), "custom property")));
                }
                return done(matching(list, prefix));
            }
            let sel_ctx = kind == Kind::Selector;
            if sel_ctx {
                let mut list: Vec<(String, &str)> = TAGS.iter().map(|t| (t.to_string(), "tag")).collect();
                list.extend(classes.iter().map(|c| (format!(".{c}"), "class")));
                list.extend(ids.iter().map(|c| (format!("#{c}"), "id")));
                return done(matching(list.iter().map(|(a, b)| (a.as_str(), *b)), prefix));
            }
            // a property name (or a custom property being declared)
            let mut items: Vec<Item> = matching(PROPS.iter().map(|p| (*p, "property")), prefix);
            for it in &mut items {
                it.suffix = ": ";
            }
            done(items)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------
// the widget

#[derive(Clone)]
struct Open {
    comp: Completion,
    sel: usize,
}

fn char_to_byte(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

fn byte_to_char(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

/// A multi-line code editor with the colours of `highlight` and the completion list of `complete`.
/// `rows` is its height in text lines. The response is the `TextEdit`'s (`changed()` after typing or
/// after a completion was taken).
pub fn code_edit(ui: &mut egui::Ui, id: Id, text: &mut String, rows: usize) -> egui::Response {
    let ctx = ui.ctx().clone();
    let (dark, base) = (ui.visuals().dark_mode, ui.visuals().text_color());
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let open_id = id.with("completion");
    let mut open: Option<Open> = ui.data(|d| d.get_temp(open_id));
    let mut taken = false;

    // keys that belong to an open list are taken before the text field sees them
    let mut accept: Option<usize> = None;
    if let Some(o) = open.as_mut() {
        let n = o.comp.items.len().max(1);
        let (down, up, tab, enter, esc) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down {
            o.sel = (o.sel + 1) % n;
        }
        if up {
            o.sel = (o.sel + n - 1) % n;
        }
        if tab || enter {
            accept = Some(o.sel);
        }
        if esc {
            open = None;
        }
    }
    let ask = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Space));
    if let (Some(k), Some(o)) = (accept, open.clone()) {
        taken = take(&ctx, id, text, &o.comp, k);
        open = None;
    }

    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(buf.as_str(), &font, dark, base);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let out = egui::TextEdit::multiline(text)
        .id(id)
        .code_editor()
        .lock_focus(true)
        .desired_rows(rows)
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter)
        .show(ui);
    let mut resp = out.response.clone();
    if taken {
        resp.mark_changed();
        ctx.memory_mut(|m| m.request_focus(id));
    }

    // (re)build the list after typing, on request, or drop it when the cursor left the word
    let cursor = out.cursor_range.map(|r| char_to_byte(text, r.primary.index));
    if resp.changed() || ask {
        open = cursor.and_then(|c| complete(text, c, ask)).map(|comp| Open { comp, sel: 0 });
    } else if let (Some(o), Some(c)) = (&open, cursor) {
        let word_ok = c >= o.comp.start && !text.get(o.comp.start..c).is_some_and(|w| w.contains(char::is_whitespace));
        if !word_ok || (!resp.has_focus() && !ctx.input(|i| i.pointer.any_down())) {
            open = None;
        }
    }

    // the list, under the cursor
    if let (Some(o), Some(range)) = (open.clone(), out.cursor_range) {
        let at = out.galley.pos_from_cursor(range.primary).left_bottom() + out.galley_pos.to_vec2();
        let mut clicked: Option<usize> = None;
        egui::Area::new(id.with("completion_area"))
            .order(egui::Order::Foreground)
            .fixed_pos(at + egui::vec2(0.0, 2.0))
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    for (k, it) in o.comp.items.iter().take(9).enumerate() {
                        ui.horizontal(|ui| {
                            let r = ui.selectable_label(k == o.sel, egui::RichText::new(&it.label).monospace());
                            if r.clicked() {
                                clicked = Some(k);
                            }
                            ui.weak(&it.detail);
                        });
                    }
                });
            });
        if let Some(k) = clicked {
            if take(&ctx, id, text, &o.comp, k) {
                resp.mark_changed();
                ctx.memory_mut(|m| m.request_focus(id));
            }
            open = None;
        }
    }
    ui.data_mut(|d| match open {
        Some(o) => d.insert_temp(open_id, o),
        None => d.remove::<Open>(open_id),
    });
    resp
}

/// Replace the word being completed with item `k`, put the cursor after it. False when nothing changed.
fn take(ctx: &egui::Context, id: Id, text: &mut String, comp: &Completion, k: usize) -> bool {
    let Some(it) = comp.items.get(k) else { return false };
    let Some(state) = egui::TextEdit::load_state(ctx, id) else { return false };
    let Some(range) = state.cursor.char_range() else { return false };
    let end = char_to_byte(text, range.primary.index).max(comp.start);
    if comp.start > text.len() || !text.is_char_boundary(comp.start) {
        return false;
    }
    // a following `=` or `:` or `}}` is already there: do not add it twice
    let after = &text[end..];
    let suffix = match it.suffix {
        "=\"\"" if after.starts_with('=') => "",
        ": " if after.starts_with(':') => "",
        s => s,
    };
    let mut ins = it.label.clone();
    ins.push_str(suffix);
    if text[..comp.start].ends_with("{{") && !after.starts_with("}}") {
        ins.push_str("}}");
    }
    let back = if suffix == "=\"\"" { it.back } else { 0 };
    text.replace_range(comp.start..end, &ins);
    let new_pos = byte_to_char(text, comp.start + ins.len()).saturating_sub(back);
    let mut state = state;
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(new_pos))));
    state.store(ctx, id);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<!doctype html>\n<html><head><title>T</title>\n<meta name=\"se:size\" content=\"100x50\">\n\
        <meta name=\"se:field\" content=\"label | text | Hi\">\n<style>\n/* c */\n:root { --accent: #e62117; }\n\
        .box { color: red; animation: pop .5s ease both; width: calc(10px + 2em); }\n\
        @keyframes pop { from { opacity: 0; } to { opacity: 1; } }\n</style></head>\n\
        <body><div class=\"box\" id=\"x\">{{label}}</div></body></html>";

    fn kind_of(text: &str, needle: &str) -> Kind {
        let at = text.find(needle).expect(needle);
        tokens(text).iter().find(|(r, _)| r.contains(&at)).map_or(Kind::Plain, |(_, k)| *k)
    }

    #[test]
    fn colours_cover_the_whole_text_and_tell_the_parts_apart() {
        let job = highlight(PAGE, &FontId::monospace(12.0), true, Color32::WHITE);
        let joined: String = job.sections.iter().map(|s| &job.text[s.byte_range.clone()]).collect();
        assert_eq!(joined, PAGE, "every byte is in exactly one section");
        assert_eq!(kind_of(PAGE, "doctype"), Kind::Tag);
        assert_eq!(kind_of(PAGE, "div class"), Kind::Tag);
        assert_eq!(kind_of(PAGE, "class=\"box\"").clone(), Kind::Attr);
        assert_eq!(kind_of(PAGE, "\"box\""), Kind::Str);
        assert_eq!(kind_of(PAGE, "/* c */"), Kind::Comment);
        assert_eq!(kind_of(PAGE, ".box {"), Kind::Selector);
        assert_eq!(kind_of(PAGE, "color: red"), Kind::Prop);
        assert_eq!(kind_of(PAGE, "red;"), Kind::Keyword);
        assert_eq!(kind_of(PAGE, "#e62117"), Kind::Num);
        assert_eq!(kind_of(PAGE, "calc("), Kind::Func);
        assert_eq!(kind_of(PAGE, "@keyframes"), Kind::AtRule);
        assert_eq!(kind_of(PAGE, "from {"), Kind::Selector);
        assert_eq!(kind_of(PAGE, "opacity: 0"), Kind::Prop);
        // {{label}} is a field wherever it sits
        let at = PAGE.find("{{label}}").unwrap();
        let sec = job.sections.iter().find(|s| s.byte_range.contains(&at)).unwrap();
        assert_eq!(sec.format.color, colour(Kind::Field, true, Color32::WHITE));
    }

    #[test]
    fn odd_text_never_panics() {
        for t in ["", "<", "<<", "<a", "<a b=\"", "<style>", "<style>a{", "{{", "{{x", "é<é é=é>é", "<style>é:é;é{é}", "<!--", "<!doctype"] {
            let _ = highlight(t, &FontId::monospace(12.0), false, Color32::BLACK);
            for c in 0..=t.len() {
                if t.is_char_boundary(c) {
                    let _ = complete(t, c, true);
                    let _ = complete(t, c, false);
                }
            }
        }
    }

    fn at_end(text: &str, force: bool) -> Option<Vec<String>> {
        complete(text, text.len(), force).map(|c| c.items.into_iter().map(|i| i.label).collect())
    }

    #[test]
    fn completion_knows_where_it_is() {
        // tags after `<`
        let t = at_end("<html><body><", false).unwrap();
        assert!(t.contains(&"div".to_string()));
        assert!(at_end("<html><body><di", false).unwrap().iter().all(|l| l.starts_with("di")));
        // attributes inside a tag
        assert!(at_end("<div cla", false).unwrap().contains(&"class".to_string()));
        // meta names
        let m = at_end("<meta name=\"se:", false).unwrap();
        assert!(m.contains(&"se:duration".to_string()) && m.contains(&"se:field".to_string()), "{m:?}");
        // class names the file already uses
        assert!(at_end(&(PAGE_HEAD.to_string() + "<div class=\"bo"), false).unwrap().contains(&"box".to_string()));
        // css properties inside a rule, values after the colon, keyframes names after `animation:`
        let p = at_end("<style>.a { disp", false).unwrap();
        assert_eq!(p, vec!["display".to_string()]);
        let v = at_end("<style>.a { display: fl", false).unwrap();
        assert!(v.contains(&"flex".to_string()), "{v:?}");
        let k = at_end("<style>@keyframes pop {} .a { animation: p", false).unwrap();
        assert!(k.contains(&"pop".to_string()), "{k:?}");
        // at-rules, custom properties, fields
        assert!(at_end("<style>@key", false).unwrap().contains(&"@keyframes".to_string()));
        let c = at_end("<style>:root { --accent: red; } .a { color: var(--ac", false).unwrap();
        assert!(c.contains(&"--accent".to_string()), "{c:?}");
        let f = at_end("<meta name=\"se:field\" content=\"label | text | Hi\"><p>{{la", false).unwrap();
        assert_eq!(f, vec!["label".to_string()]);
        // nothing offered in plain text, on a finished word, or inside a comment
        assert!(at_end("<p>hello wor", false).is_none());
        assert!(at_end("<div>", false).is_none());
        assert!(at_end("<!-- <di", false).is_none());
        // Ctrl+Space with nothing typed asks for what fits: properties in a rule, attributes in a tag
        assert!(at_end("<style>.a { ", true).unwrap().contains(&"color".to_string()));
        assert!(at_end("<div ", true).unwrap().contains(&"class".to_string()));
    }

    const PAGE_HEAD: &str = "<style>.box { color: red; }</style>";

    #[test]
    fn a_taken_completion_replaces_the_word() {
        let ctx = egui::Context::default();
        let id = Id::new("t");
        let mut text = String::from("<div cla");
        let mut st = egui::widgets::text_edit::TextEditState::default();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(8))));
        st.store(&ctx, id);
        let comp = complete(&text, 8, false).unwrap();
        let k = comp.items.iter().position(|i| i.label == "class").unwrap();
        assert!(take(&ctx, id, &mut text, &comp, k));
        assert_eq!(text, "<div class=\"\"");
        let c = egui::TextEdit::load_state(&ctx, id).unwrap().cursor.char_range().unwrap().primary.index;
        assert_eq!(c, text.chars().count() - 1, "the cursor sits between the quotes");
    }
}
