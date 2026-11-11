//! ---- ws:html-layer ----
//! HTML + CSS layers and scenes: the template format (`library/README.md`), and the renderer that
//! drives a headless Microsoft Edge over its DevTools connection (method and measurements: the
//! "creator-kit spike" entry in notes.md).
//!
//! Renderer: ONE Edge process (throwaway profile under the temp dir, network blocked, page scripts
//! off) is started lazily on the first render, kept alive behind `EDGE` and reused; it is closed with
//! `Browser.close` after `IDLE` without a render and on app exit (`shutdown`). Per frame every CSS
//! animation's `currentTime` is set through `Runtime.evaluate` (a host-side evaluation: the page
//! itself never runs script) and the viewport is captured as a PNG with a transparent background.
//! A full render pipes those PNGs into ffmpeg as PNG-in-MOV (`-c:v copy`, straight alpha kept); a
//! page without animations is one PNG. Output files are content-addressed in `cache_dir()/html/`.
//!
//! Shutdown: the spike's preview tool killed Edge's main process and waited, which sometimes hung
//! after pages with infinite animations. Killing only the browser process orphans its renderer /
//! GPU children (a renderer busy with an infinite animation does not notice at once), and those
//! children inherit every inheritable handle of ours - our own stdout pipe included, which the
//! calling shell waits on - and keep the profile folder locked. So: `Browser.close` first (Edge
//! closes its whole tree), wait for the browser with a deadline, and only if it is still there kill
//! the TREE by its own pid (`taskkill /PID <pid> /T /F`, never by image name: the user's own Edge is
//! running), then delete the profile with a few retries, never blocking for long.

use crate::engine::export::{self, Progress};
use crate::media::{ffpipe, Frame};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------------------
// template

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldType {
    Text,
    Color,
    Number,
    Image,
}

impl FieldType {
    fn parse(s: &str) -> Option<FieldType> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "text" => FieldType::Text,
            "color" | "colour" => FieldType::Color,
            "number" => FieldType::Number,
            "image" => FieldType::Image,
            _ => return None,
        })
    }
}

/// One `se:field`: `name | type | default [| min | max]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: FieldType,
    pub default: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// What a template's head says about it.
#[derive(Clone, Debug)]
pub struct Template {
    pub title: String,
    /// `se:kind` scene (full frame, own background); false = layer.
    pub scene: bool,
    /// `se:size` in CSS pixels against a 1080-high frame (default 1920x1080).
    pub size: (u32, u32),
    /// `se:duration` in seconds (default 4, clamped to 0.5..60).
    pub duration: f64,
    pub thumb: Option<f64>,
    pub fields: Vec<Field>,
}

impl Template {
    /// The second the preview picture is taken at: `se:thumb`, else 60 % of the duration.
    pub fn thumb_at(&self) -> f64 {
        self.thumb.unwrap_or(self.duration * 0.6).clamp(0.0, self.duration)
    }
}

/// A transparent 1x1 GIF: an empty image field.
const TRANSPARENT: &str = "data:image/gif;base64,R0lGODlhAQABAAAAACH5BAEKAAEALAAAAAABAAEAAAICTAEAOw==";

/// Index just past the `>` that closes the tag whose text starts at `from` (quote-aware), or the end.
fn tag_end(b: &[u8], from: usize) -> usize {
    let (mut i, mut q) = (from, 0u8);
    while i < b.len() {
        match (q, b[i]) {
            (0, b'"' | b'\'') => q = b[i],
            (0, b'>') => return i + 1,
            (c, x) if c != 0 && x == c => q = 0,
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// One attribute of a tag: lower-case name, raw value, and its byte span within the tag text.
struct Attr<'a> {
    name: String,
    value: &'a str,
    span: (usize, usize),
}

/// The attributes of a tag's text (what follows the tag name, up to and including `>`). Tolerant:
/// double, single or no quotes, any order, valueless attributes.
fn attrs(t: &str) -> Vec<Attr<'_>> {
    let b = t.as_bytes();
    let (mut i, mut out) = (0, Vec::new());
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/' || b[i] == b'>') {
            i += 1;
        }
        if i >= b.len() {
            return out;
        }
        let s = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        if i == s {
            i += 1; // a stray `=`: skip it
            continue;
        }
        let name = t[s..i].to_ascii_lowercase();
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = "";
        if j < b.len() && b[j] == b'=' {
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                let q = b[j];
                let mut k = j + 1;
                while k < b.len() && b[k] != q {
                    k += 1;
                }
                value = &t[j + 1..k];
                i = (k + 1).min(b.len());
            } else {
                let mut k = j;
                while k < b.len() && !b[k].is_ascii_whitespace() && b[k] != b'>' {
                    k += 1;
                }
                value = &t[j..k];
                i = k;
            }
        }
        out.push(Attr { name, value, span: (s, i) });
    }
}

/// The text (after the name) of every `<name …>` opening tag, case-insensitive.
fn open_tags<'a>(src: &'a str, name: &str) -> Vec<&'a str> {
    let low = src.to_ascii_lowercase();
    let pat = format!("<{name}");
    let (b, mut i, mut out) = (src.as_bytes(), 0, Vec::new());
    while let Some(p) = low[i..].find(&pat) {
        let s = i + p + pat.len();
        let end = tag_end(b, s);
        if b.get(s).is_none_or(|c| c.is_ascii_whitespace() || matches!(c, b'/' | b'>')) {
            out.push(&src[s..end]);
        }
        i = end.max(s);
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// HTML-escape a text value (it lands in markup or a CSS string).
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Every `content` of `<meta name="<name>">`, in order.
fn metas(src: &str, name: &str) -> Vec<String> {
    open_tags(src, "meta")
        .into_iter()
        .filter_map(|t| {
            let a = attrs(t);
            a.iter()
                .any(|x| x.name == "name" && x.value.trim().eq_ignore_ascii_case(name))
                .then(|| unescape(a.iter().find(|x| x.name == "content").map_or("", |x| x.value).trim()))
        })
        .collect()
}

fn title_of(src: &str) -> String {
    let low = src.to_ascii_lowercase();
    let Some(t) = open_tags(src, "title").first().map(|t| t.as_ptr() as usize - src.as_ptr() as usize + t.len()) else {
        return String::new();
    };
    let end = low[t..].find("</title").map_or(src.len(), |e| t + e);
    unescape(src[t..end].trim())
}

/// "4s", "4", "1500ms" → seconds.
fn secs(s: &str) -> Option<f64> {
    let s = s.trim().to_ascii_lowercase();
    let v = match s.strip_suffix("ms") {
        Some(ms) => ms.trim().parse::<f64>().ok()? / 1000.0,
        None => s.trim_end_matches('s').trim().parse::<f64>().ok()?,
    };
    v.is_finite().then_some(v)
}

/// "640x220" → (640, 220).
fn size_of(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once(['x', 'X', '×'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn field_of(s: &str) -> Option<Field> {
    let p: Vec<&str> = s.split('|').map(str::trim).collect();
    let name = p.first().filter(|n| !n.is_empty())?.to_string();
    let num = |i: usize| p.get(i).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
    Some(Field {
        name,
        ty: FieldType::parse(p.get(1)?)?,
        default: p.get(2).copied().unwrap_or("").to_string(),
        min: num(3),
        max: num(4),
    })
}

/// Read a template's head. Never fails: what is missing gets its default (`lint` reports it).
pub fn parse(src: &str) -> Template {
    let first = |n: &str| metas(src, n).into_iter().next();
    let (w, h) = first("se:size").and_then(|s| size_of(&s)).unwrap_or((1920, 1080));
    Template {
        title: title_of(src),
        scene: first("se:kind").is_some_and(|k| k.eq_ignore_ascii_case("scene")),
        size: (w.clamp(2, 3840), h.clamp(2, 2160)),
        duration: first("se:duration").and_then(|d| secs(&d)).unwrap_or(4.0).clamp(0.5, 60.0),
        thumb: first("se:thumb").and_then(|d| secs(&d)),
        fields: metas(src, "se:field").iter().filter_map(|f| field_of(f)).collect(),
    }
}

fn valid_color(s: &str) -> bool {
    s.strip_prefix('#').is_some_and(|h| matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `path` as a `file:///` URL, every byte outside a safe set percent-encoded (so it is safe inside a
/// CSS `url()` or an attribute, quoted or not).
pub fn file_url(path: &Path) -> String {
    let mut s = String::from("file:///");
    for b in path.to_string_lossy().replace('\\', "/").trim_start_matches('/').bytes() {
        if b.is_ascii_alphanumeric() || b"/:._~-".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// `n` without a trailing ".0".
fn num_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// The text a field's `{{name}}` becomes: the value from `values` (last one wins), else the default.
pub fn field_value(f: &Field, values: &[(String, String)]) -> String {
    let raw = values.iter().rev().find(|(n, _)| *n == f.name).map_or(f.default.as_str(), |(_, v)| v.as_str()).trim();
    match f.ty {
        FieldType::Text => escape(raw),
        FieldType::Color => {
            [raw, f.default.trim()].into_iter().find(|c| valid_color(c)).unwrap_or("transparent").to_string()
        }
        FieldType::Number => {
            let n = [raw, f.default.trim()].iter().find_map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite()));
            let mut n = n.unwrap_or(0.0);
            if let Some(lo) = f.min {
                n = n.max(lo);
            }
            if let Some(hi) = f.max {
                n = n.min(hi);
            }
            num_text(n)
        }
        FieldType::Image => {
            let safe_data =
                raw.starts_with("data:image/") && !raw.contains(|c: char| c.is_whitespace() || "\"'()<>".contains(c));
            if safe_data {
                raw.to_string()
            } else if !raw.is_empty() && Path::new(raw).is_file() {
                file_url(Path::new(raw))
            } else {
                TRANSPARENT.to_string()
            }
        }
    }
}

/// Every `{{name}}` in `src` (inner text trimmed), in order.
fn placeholders(src: &str) -> Vec<&str> {
    let (mut out, mut rest) = (Vec::new(), src);
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) if !after[..j].contains(['{', '<', '\n']) => {
                out.push(after[..j].trim());
                rest = &after[j + 2..];
            }
            _ => rest = after,
        }
    }
    out
}

/// Replace every `{{name}}` of a declared field; unknown names stay as they are.
pub fn substitute(src: &str, tpl: &Template, values: &[(String, String)]) -> String {
    let (mut out, mut rest) = (String::with_capacity(src.len() + 64), src);
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        if let Some(j) = after.find("}}") {
            if let Some(f) = tpl.fields.iter().find(|f| f.name == after[..j].trim()) {
                out.push_str(&field_value(f, values));
                rest = &after[j + 2..];
                continue;
            }
        }
        out.push_str("{{");
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Case-insensitive replace of an ASCII needle.
fn replace_ci(s: &str, needle: &str, with: &str) -> String {
    let low = s.to_ascii_lowercase();
    let (mut out, mut at) = (String::with_capacity(s.len()), 0);
    while let Some(p) = low[at..].find(needle) {
        out.push_str(&s[at..at + p]);
        out.push_str(with);
        at += p + needle.len();
    }
    out.push_str(&s[at..]);
    out
}

/// No script survives: `<script>…</script>` blocks go, so do `on*=` attributes and `javascript:` URLs
/// (Edge also runs with scripts disabled - this is the second lock).
pub fn sanitize(src: &str) -> String {
    // 1. script blocks (an unclosed one takes the rest of the file)
    let mut s = String::with_capacity(src.len());
    let mut rest = src;
    loop {
        let low = rest.to_ascii_lowercase();
        let Some(i) = low
            .find("<script")
            .filter(|&i| low.as_bytes().get(i + 7).is_none_or(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/')))
        else {
            s.push_str(rest);
            break;
        };
        s.push_str(&rest[..i]);
        rest = match low[i..].find("</script") {
            Some(e) => &rest[tag_end(rest.as_bytes(), i + e)..],
            None => "",
        };
    }
    // 2. event-handler attributes and script URLs inside every tag
    let b = s.as_bytes();
    let (mut out, mut i, mut copied) = (String::with_capacity(s.len()), 0, 0);
    while i < b.len() {
        if b[i] != b'<' || !b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        let name_end =
            (i + 1..b.len()).find(|&k| b[k].is_ascii_whitespace() || matches!(b[k], b'>' | b'/')).unwrap_or(b.len());
        let end = tag_end(b, name_end);
        let text = &s[name_end..end];
        let bad: Vec<(usize, usize)> = attrs(text)
            .into_iter()
            .filter(|a| {
                let v: String = a.value.chars().filter(|c| !c.is_whitespace() && !c.is_control()).collect();
                let v = v.to_ascii_lowercase();
                a.name.starts_with("on") || v.starts_with("javascript:") || v.starts_with("vbscript:")
            })
            .map(|a| a.span)
            .collect();
        if !bad.is_empty() {
            out.push_str(&s[copied..name_end]);
            let mut at = 0;
            for (a, z) in bad {
                out.push_str(&text[at..a]);
                at = z;
            }
            out.push_str(&text[at..]);
            copied = end;
        }
        i = end;
    }
    out.push_str(&s[copied..]);
    // 3. a script URL anywhere else (CSS `url(javascript:…)`)
    replace_ci(&out, "javascript:", "blocked:")
}

/// What breaks the library rules (`library/README.md`, "HTML layers and scenes"); empty = fine.
pub fn lint(src: &str) -> Vec<String> {
    let mut e = Vec::new();
    let low = src.to_ascii_lowercase();
    let tpl = parse(src);
    if tpl.title.is_empty() {
        e.push("missing <title> (the name)".to_string());
    }
    for need in ["se:desc", "se:tags"] {
        if metas(src, need).first().is_none_or(|v| v.is_empty()) {
            e.push(format!("missing <meta name=\"{need}\">"));
        }
    }
    if let Some(k) = metas(src, "se:kind").first() {
        if !k.eq_ignore_ascii_case("layer") && !k.eq_ignore_ascii_case("scene") {
            e.push(format!("se:kind `{k}` is neither layer nor scene"));
        }
    }
    if let Some(s) = metas(src, "se:size").first() {
        match size_of(s) {
            None => e.push(format!("se:size `{s}` is not WxH")),
            Some((w, h)) if w % 2 == 1 || h % 2 == 1 || w == 0 || h == 0 => {
                e.push(format!("se:size {w}x{h}: use even numbers"))
            }
            Some((w, h)) if w > 3840 || h > 2160 => e.push(format!("se:size {w}x{h} is larger than 3840x2160")),
            _ => {}
        }
    }
    if let Some(d) = metas(src, "se:duration").first() {
        match secs(d) {
            Some(v) if (0.5..=60.0).contains(&v) => {}
            _ => e.push(format!("se:duration `{d}` is not 0.5 to 60 seconds")),
        }
    }
    if let Some(t) = metas(src, "se:thumb").first() {
        if secs(t).is_none_or(|v| v < 0.0 || v > tpl.duration) {
            e.push(format!("se:thumb `{t}` is not a second within the duration"));
        }
    }
    let raw_fields = metas(src, "se:field");
    if raw_fields.len() > 12 {
        e.push(format!("{} fields: at most 12", raw_fields.len()));
    }
    let used = placeholders(src);
    let mut names: Vec<String> = Vec::new();
    for raw in &raw_fields {
        let Some(f) = field_of(raw) else {
            e.push(format!("se:field `{raw}` is not `name | text|color|number|image | default`"));
            continue;
        };
        let ok_name = f.name.starts_with(|c: char| c.is_ascii_lowercase())
            && f.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok_name {
            e.push(format!("field name `{}`: lowercase letters, digits and _ only", f.name));
        }
        if names.contains(&f.name) {
            e.push(format!("field `{}` is declared twice", f.name));
        }
        names.push(f.name.clone());
        match f.ty {
            FieldType::Color if !valid_color(f.default.trim()) => {
                e.push(format!("field `{}`: default `{}` is not #rrggbb or #rrggbbaa", f.name, f.default))
            }
            FieldType::Number => {
                if f.default.trim().parse::<f64>().is_err() {
                    e.push(format!("field `{}`: default `{}` is not a number", f.name, f.default));
                }
                if let (Some(a), Some(b)) = (f.min, f.max) {
                    if a > b {
                        e.push(format!("field `{}`: min is above max", f.name));
                    }
                }
            }
            _ => {}
        }
        if !used.contains(&f.name.as_str()) {
            e.push(format!("field `{}` is declared but never used as {{{{{}}}}}", f.name, f.name));
        }
    }
    for u in used {
        if !tpl.fields.iter().any(|f| f.name == u) && !e.iter().any(|m| m.contains(&format!("{{{{{u}}}}} is not"))) {
            e.push(format!("{{{{{u}}}}} is not a declared field"));
        }
    }
    if low.contains("<script") {
        e.push("contains <script> (no JavaScript: motion is CSS only)".into());
    } else if sanitize(src) != src {
        e.push("contains an on* attribute or a javascript: URL".into());
    }
    if low.replace("http://www.w3.org/", "").contains("http://") || low.contains("https://") {
        e.push("contains a network URL (http/https): pictures go inline (svg, data:)".into());
    }
    for (bad, why) in [
        ("@import", "@import (no network, no other files)"),
        ("@font-face", "@font-face (web fonts): use installed Windows fonts with fallbacks"),
        ("<link", "<link> (no external stylesheets)"),
        (":hover", ":hover (nothing reacts to a pointer)"),
        ("prefers-", "a prefers-* media query"),
    ] {
        if low.contains(bad) {
            e.push(format!("contains {why}"));
        }
    }
    for el in ["video", "audio", "iframe", "canvas", "object", "embed", "form", "input", "textarea", "select"] {
        if !open_tags(src, el).is_empty() {
            e.push(format!("contains <{el}> (not allowed)"));
        }
    }
    e
}

/// Template + the page Edge loads: fields substituted, then sanitised.
pub fn prepare(src: &str, values: &[(String, String)]) -> (Template, String) {
    let tpl = parse(src);
    let page = sanitize(&substitute(src, &tpl, values));
    (tpl, page)
}

// ---------------------------------------------------------------------------------------------------
// WebSocket (client side, localhost, no TLS) - std only

/// One client frame (FIN set): a client must mask, and a zero key is a valid mask (localhost needs no
/// secrecy), so the payload goes out as it is.
fn encode_frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 14);
    f.push(0x80 | op);
    match payload.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n < 65536 => {
            f.push(0x80 | 126);
            f.extend((n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend((n as u64).to_be_bytes());
        }
    }
    f.extend([0u8; 4]);
    f.extend(payload);
    f
}

/// One frame: (fin, opcode, unmasked payload).
fn read_frame(r: &mut impl Read) -> Result<(bool, u8, Vec<u8>), String> {
    let io = |e: std::io::Error| e.to_string();
    let mut h = [0u8; 2];
    r.read_exact(&mut h).map_err(io)?;
    let mut len = (h[1] & 0x7f) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        r.read_exact(&mut b).map_err(io)?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).map_err(io)?;
        len = u64::from_be_bytes(b);
    }
    if len > 1 << 30 {
        return Err("websocket frame too large".into());
    }
    let mut mask = [0u8; 4];
    if h[1] & 0x80 != 0 {
        r.read_exact(&mut mask).map_err(io)?;
    }
    let mut p = vec![0u8; len as usize];
    r.read_exact(&mut p).map_err(io)?;
    if h[1] & 0x80 != 0 {
        for (i, b) in p.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
    }
    Ok((h[0] & 0x80 != 0, h[0] & 0x0f, p))
}

/// One whole data message: fragments joined, pings answered on `w`, pongs skipped.
fn read_message(r: &mut impl Read, w: &mut impl Write) -> Result<Vec<u8>, String> {
    let mut msg = Vec::new();
    loop {
        let (fin, op, p) = read_frame(r)?;
        match op {
            0x9 => w.write_all(&encode_frame(0xA, &p)).map_err(|e| e.to_string())?,
            0xA => {}
            0x8 => return Err("websocket closed".into()),
            _ => {
                msg.extend(p);
                if fin {
                    return Ok(msg);
                }
            }
        }
    }
}

struct Ws {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Ws {
    fn connect(port: u16, path: &str) -> Result<Ws, String> {
        let s = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        s.set_nodelay(true).ok();
        s.set_read_timeout(Some(Duration::from_secs(30))).ok();
        let mut w = s.try_clone().map_err(|e| e.to_string())?;
        // no Origin header: Edge then needs no --remote-allow-origins
        write!(
            w,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: c2ltcGxlLWVkaXRvci0xMg==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .map_err(|e| e.to_string())?;
        let mut r = BufReader::with_capacity(1 << 20, s);
        let mut line = String::new();
        r.read_line(&mut line).map_err(|e| e.to_string())?;
        if !line.contains(" 101 ") {
            return Err(format!("DevTools handshake: {}", line.trim()));
        }
        while line != "\r\n" && !line.is_empty() {
            line.clear();
            r.read_line(&mut line).map_err(|e| e.to_string())?;
        }
        Ok(Ws { r, w })
    }
}

/// DevTools protocol over one browser socket, flat sessions.
struct Cdp {
    ws: Ws,
    next: u64,
    session: Option<String>,
    /// `Page.loadEventFired` seen since the last navigation.
    loaded: bool,
    /// The connection failed: this Edge is no use any more.
    broken: bool,
}

impl Cdp {
    fn recv(&mut self) -> Result<Value, String> {
        let m = read_message(&mut self.ws.r, &mut self.ws.w).inspect_err(|_| self.broken = true)?;
        let v: Value = serde_json::from_slice(&m).map_err(|e| e.to_string())?;
        if v["method"] == "Page.loadEventFired" {
            self.loaded = true;
        }
        Ok(v)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let mut m = json!({"id": id, "method": method, "params": params});
        if let Some(s) = &self.session {
            m["sessionId"] = json!(s);
        }
        let sent = self.ws.w.write_all(&encode_frame(1, m.to_string().as_bytes()));
        sent.map_err(|e| {
            self.broken = true;
            format!("Edge: {e}")
        })?;
        loop {
            let v = self.recv().map_err(|e| format!("Edge ({method}): {e}"))?;
            if v["id"].as_u64() == Some(id) {
                return match v.get("error") {
                    Some(err) => Err(format!("Edge ({method}): {err}")),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    }

    fn eval(&mut self, js: &str) -> Result<Value, String> {
        let r = self.call("Runtime.evaluate", json!({"expression": js, "returnByValue": true}))?;
        match r.get("exceptionDetails") {
            Some(x) => Err(format!("Edge: {x}")),
            None => Ok(r["result"]["value"].clone()),
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// the shared Edge

/// Edge is closed after this long without a render (it costs ~1.3 s to start again).
const IDLE: Duration = Duration::from_secs(90);

struct Edge {
    child: Child,
    profile: PathBuf,
    cdp: Cdp,
    used: Instant,
    pages: u64,
}

static EDGE: Mutex<Option<Edge>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// msedge.exe: the usual Windows place, then Program Files, then a per-user install.
pub fn edge_exe() -> Option<PathBuf> {
    let mut c = vec![PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")];
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(v) = std::env::var_os(var) {
            c.push(PathBuf::from(v).join(r"Microsoft\Edge\Application\msedge.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Kill a process tree by the root's own pid (never by image name).
fn kill_tree(child: &mut Child) {
    let pid = child.id().to_string();
    let _ = ffpipe::command(Path::new("taskkill"))
        .args(["/PID", &pid, "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

/// Wait up to `d` for `child` to exit.
fn wait_for(child: &mut Child, d: Duration) -> bool {
    let until = Instant::now() + d;
    loop {
        if !matches!(child.try_wait(), Ok(None)) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn remove_profile(dir: &Path) {
    // the last child processes let go of their files a moment after they exit
    for _ in 0..15 {
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl Edge {
    fn launch() -> Result<Edge, String> {
        let exe = edge_exe()
            .ok_or("Microsoft Edge not found - HTML layers are rendered with the Edge that ships with Windows")?;
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // ponytail: a profile left behind by a crash stays in %TEMP% - sweep `se-edge-*` if it ever matters
        let profile = std::env::temp_dir().join(format!("se-edge-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).map_err(|e| format!("Edge profile: {e}"))?;
        let mut child = ffpipe::command(&exe)
            .args(["--headless", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check"])
            .args(["--hide-scrollbars", "--mute-audio", "--disable-extensions", "--disable-background-networking"])
            .args(["--disable-sync", "--disable-component-update", "--host-resolver-rules=MAP * ~NOTFOUND"])
            .arg(format!("--user-data-dir={}", profile.join("p").display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Microsoft Edge: {e}"))?;
        match Self::connect(&mut child, &profile.join("p")) {
            Ok(cdp) => Ok(Edge { child, profile, cdp, used: Instant::now(), pages: 0 }),
            Err(e) => {
                kill_tree(&mut child);
                wait_for(&mut child, Duration::from_secs(3));
                remove_profile(&profile);
                Err(e)
            }
        }
    }

    fn connect(child: &mut Child, profile: &Path) -> Result<Cdp, String> {
        let t0 = Instant::now();
        let port_file = profile.join("DevToolsActivePort");
        let (port, path) = loop {
            if let Ok(s) = std::fs::read_to_string(&port_file) {
                let mut l = s.lines();
                if let (Some(p), Some(path)) = (l.next().and_then(|p| p.trim().parse::<u16>().ok()), l.next()) {
                    break (p, path.trim().to_string());
                }
            }
            if !matches!(child.try_wait(), Ok(None)) {
                return Err("Microsoft Edge exited at start".into());
            }
            if t0.elapsed() > Duration::from_secs(20) {
                return Err("Microsoft Edge did not start (no DevTools port after 20 s)".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut c = Cdp { ws: Ws::connect(port, &path)?, next: 0, session: None, loaded: false, broken: false };
        let target = c.call("Target.createTarget", json!({"url": "about:blank"}))?;
        let sess = c.call("Target.attachToTarget", json!({"targetId": target["targetId"], "flatten": true}))?;
        c.session = Some(sess["sessionId"].as_str().ok_or("Edge: no session")?.to_string());
        c.call("Page.enable", json!({}))?;
        c.call("Emulation.setScriptExecutionDisabled", json!({"value": true}))?;
        c.call("Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}))?;
        Ok(c)
    }

    /// `Browser.close`, then wait; kill the tree by its own pid only if it is still there.
    fn close(mut self) {
        self.cdp.ws.w.set_read_timeout(Some(Duration::from_secs(2))).ok();
        self.cdp.session = None;
        let _ = self.cdp.call("Browser.close", json!({})); // Edge may drop the socket before it answers
        let _ = self.cdp.ws.w.shutdown(std::net::Shutdown::Both);
        if !wait_for(&mut self.child, Duration::from_secs(3)) {
            kill_tree(&mut self.child);
            wait_for(&mut self.child, Duration::from_secs(2));
        }
        remove_profile(&self.profile);
    }

    /// Load `page` at `size` CSS pixels, `scale` device pixels each; wait for its fonts.
    fn load(&mut self, page: &str, size: (u32, u32), scale: f64) -> Result<(), String> {
        self.pages += 1;
        let file = self.profile.join("page.html");
        std::fs::write(&file, page).map_err(|e| format!("page: {e}"))?;
        let m = json!({"width": size.0, "height": size.1, "deviceScaleFactor": scale, "mobile": false});
        self.cdp.call("Emulation.setDeviceMetricsOverride", m)?;
        self.cdp.loaded = false;
        let url = format!("{}?n={}", file_url(&file), self.pages);
        let r = self.cdp.call("Page.navigate", json!({"url": url}))?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(format!("Edge could not load the page: {e}"));
        }
        while !self.cdp.loaded {
            self.cdp.recv().map_err(|e| format!("Edge (load): {e}"))?;
        }
        let fonts =
            json!({"expression": "document.fonts.ready.then(() => 1)", "awaitPromise": true, "returnByValue": true});
        let _ = self.cdp.call("Runtime.evaluate", fonts);
        Ok(())
    }

    /// Every animation paused at `ms`.
    fn seek(&mut self, ms: f64) -> Result<(), String> {
        self.cdp.eval(&format!("for(const a of document.getAnimations()){{a.pause();a.currentTime={ms}}}")).map(|_| ())
    }

    /// (animation count, any infinite, the last finite end in ms).
    fn timing(&mut self) -> Result<(u64, bool, f64), String> {
        let v = self.cdp.eval(
            "(()=>{let n=0,inf=0,end=0;for(const a of document.getAnimations()){n++;\
             const e=a.effect?a.effect.getComputedTiming().endTime:0;if(e===Infinity)inf=1;else end=Math.max(end,e)}\
             return [n,inf,end]})()",
        )?;
        Ok((v[0].as_u64().unwrap_or(0), v[1].as_u64() == Some(1), v[2].as_f64().unwrap_or(0.0)))
    }

    /// The viewport as PNG bytes.
    fn shot(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        let r = self.cdp.call("Page.captureScreenshot", json!({"format": "png", "optimizeForSpeed": true}))?;
        b64_decode(r["data"].as_str().unwrap_or("").as_bytes(), out);
        if png_size(out).is_none() {
            return Err("Edge returned no picture".into());
        }
        Ok(())
    }
}

/// Run `f` on the shared Edge (started if needed). A broken connection closes that Edge and `f` runs
/// once more on a fresh one.
/// ponytail: one Edge, one tab, one render at a time (a long render job holds up the editor preview
/// until it is done) - a tab per caller if that ever shows.
fn with_edge<T>(mut f: impl FnMut(&mut Edge) -> Result<T, String>) -> Result<T, String> {
    let mut g = lock(&EDGE);
    for attempt in 0..2 {
        if g.is_none() {
            *g = Some(Edge::launch()?);
            std::thread::Builder::new().name("edge-idle".into()).spawn(reap_when_idle).map_err(|e| e.to_string())?;
        }
        let e = g.as_mut().expect("just started");
        e.used = Instant::now();
        let r = f(e);
        e.used = Instant::now();
        match r {
            Err(err) if e.cdp.broken => {
                if let Some(e) = g.take() {
                    e.close();
                }
                if attempt == 1 {
                    return Err(err);
                }
            }
            r => return r,
        }
    }
    unreachable!()
}

/// Closes the shared Edge once it has been idle for `IDLE` (one such thread per started Edge).
fn reap_when_idle() {
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let mut g = lock(&EDGE);
        match g.as_ref() {
            None => return,
            Some(e) if e.used.elapsed() >= IDLE => {
                if let Some(e) = g.take() {
                    e.close();
                }
                return;
            }
            _ => {}
        }
    }
}

/// Close the shared Edge now (app exit). Safe to call any time; a later render starts a new one.
pub fn shutdown() {
    if let Some(e) = lock(&EDGE).take() {
        e.close();
    }
}

// ---------------------------------------------------------------------------------------------------
// pictures

fn b64_decode(s: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
}

/// Width and height from a PNG's IHDR.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Some((be(16), be(20)))
}

/// PNG bytes → straight-alpha RGBA, through ffmpeg (the app has no PNG decoder of its own).
pub fn decode_png(png: &[u8]) -> Result<Frame, String> {
    let (w, h) = png_size(png).ok_or("not a PNG")?;
    let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let mut c = ffpipe::command(&exe)
        .args(["-v", "error", "-f", "png_pipe", "-i", "-", "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdin = c.stdin.take().ok_or("ffmpeg stdin")?;
    let data = png.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let _ = c.stdout.take().ok_or("ffmpeg stdout")?.read_to_end(&mut rgba);
    let _ = c.wait();
    let _ = writer.join();
    if rgba.len() != (w * h * 4) as usize {
        return Err(format!("could not decode the {w}x{h} picture"));
    }
    Ok(Frame { width: w, height: h, pts: 0.0, rgba, bgrx: false })
}

/// The device scale for a project: CSS pixels are 1080-reference pixels.
pub fn scale_for(project_w: u32, project_h: u32) -> f64 {
    (project_w.min(project_h).max(2) as f64 / 1080.0).clamp(0.05, 4.0)
}

/// The picture size a template renders at.
pub fn pixel_size(size: (u32, u32), scale: f64) -> (u32, u32) {
    (((size.0 as f64 * scale).round() as u32).max(1), ((size.1 as f64 * scale).round() as u32).max(1))
}

/// One frame of `html` (fields applied) at second `t`, as PNG bytes. Blocking: never on the UI thread
/// for long (a cold Edge takes ~1.3 s, a warm one a few tens of ms plus the page load).
pub fn render_png(html: &str, fields: &[(String, String)], scale: f64, t: f64) -> Result<Vec<u8>, String> {
    let (tpl, page) = prepare(html, fields);
    let scale = scale.clamp(0.05, 4.0);
    with_edge(|e| {
        e.load(&page, tpl.size, scale)?;
        e.seek((t.max(0.0) * 1000.0).min(1e9))?;
        let mut png = Vec::new();
        e.shot(&mut png)?;
        Ok(png)
    })
}

/// CONTRACT: one frame (straight-alpha RGBA) of `html` at `t` seconds. Blocking; never on the UI thread.
pub fn render_frame(html: &str, fields: &[(String, String)], scale: f64, t: f64) -> Result<Frame, String> {
    decode_png(&render_png(html, fields, scale, t)?)
}

/// What a full render needs; the output file is named by a hash of all of it.
#[derive(Clone, Debug)]
pub struct Spec {
    pub html: String,
    pub fields: Vec<(String, String)>,
    pub scale: f64,
    pub fps: f64,
    pub duration: f64,
}

/// A finished (or cached) render.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// `.mov` (true) or a single `.png` (the page has no animation).
    pub animated: bool,
}

pub fn dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("html")
}

/// `cache_dir()/html/<hash>` (no extension) and the picture size for `spec`.
pub fn out_base(spec: &Spec) -> (PathBuf, (u32, u32)) {
    use std::hash::{Hash, Hasher};
    let (tpl, page) = prepare(&spec.html, &spec.fields);
    let px = pixel_size(tpl.size, spec.scale.clamp(0.05, 4.0));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("html-v1", &page, px, spec.fps.to_bits(), spec.duration.to_bits()).hash(&mut h);
    (dir().join(format!("{:016x}", h.finish())), px)
}

/// The render for `spec` if it is already in the cache (no render).
pub fn cached(spec: &Spec) -> Option<Rendered> {
    let (base, (width, height)) = out_base(spec);
    [("mov", true), ("png", false)].into_iter().find_map(|(ext, animated)| {
        let path = base.with_extension(ext);
        path.is_file().then_some(Rendered { path, width, height, animated })
    })
}

/// Render `spec` into the cache (or reuse what is there): PNG frames piped to ffmpeg as PNG-in-MOV,
/// or one `.png` for a page with no animation. Frames after the last finite animation has ended are
/// the same picture: rendered once, repeated. Progress and cancel through `prog`.
pub fn render_to_file(spec: &Spec, prog: &Progress) -> Result<Rendered, String> {
    if let Some(r) = cached(spec) {
        return Ok(r);
    }
    let (base, _) = out_base(spec);
    let (tpl, page) = prepare(&spec.html, &spec.fields);
    let (scale, fps) = (spec.scale.clamp(0.05, 4.0), spec.fps.clamp(1.0, 240.0));
    let frames = (spec.duration.clamp(0.04, 3600.0) * fps).ceil().max(1.0) as usize;
    std::fs::create_dir_all(dir()).map_err(|e| format!("{}: {e}", dir().display()))?;
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    prog.set(0.0, "Rendering HTML…");
    with_edge(|e| {
        e.load(&page, tpl.size, scale)?;
        let (count, infinite, end_ms) = e.timing()?;
        let mut png = Vec::new();
        if count == 0 {
            e.shot(&mut png)?;
            let out = base.with_extension("png");
            let tmp = export::temp_output(&out);
            std::fs::write(&tmp.0, &png).map_err(|e| format!("{}: {e}", out.display()))?;
            tmp.commit(&out)?;
            let (width, height) = png_size(&png).unwrap_or((1, 1));
            return Ok(Rendered { path: out, width, height, animated: false });
        }
        let out = base.with_extension("mov");
        let tmp = export::temp_output(&out);
        let mut child = ffpipe::command(&ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "image2pipe"])
            .args(["-framerate", &format!("{}/1000", (fps * 1000.0).round())])
            .args(["-c:v", "png", "-i", "-", "-c:v", "copy"])
            .arg(&tmp.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let tail = export::stderr_tail(&mut child);
        let mut stdin = child.stdin.take().ok_or("ffmpeg stdin")?;
        let mut feed = || -> Result<(u32, u32), String> {
            let (mut held, mut size) = (false, (1, 1));
            for i in 0..frames {
                if prog.is_cancelled() {
                    break; // wait_ffmpeg below kills it and says so
                }
                let ms = i as f64 * 1000.0 / fps;
                let frozen = !infinite && ms >= end_ms;
                if !(frozen && held) {
                    e.seek(ms)?;
                    e.shot(&mut png)?;
                    held = frozen;
                    if i == 0 {
                        size = png_size(&png).unwrap_or(size);
                    }
                }
                if stdin.write_all(&png).is_err() {
                    break; // ffmpeg stopped: its own error is the one worth reporting
                }
                prog.set((i + 1) as f32 / frames as f32 * 0.99, "Rendering HTML…");
            }
            Ok(size)
        };
        let fed = feed();
        drop(stdin);
        let size = match fed {
            Ok(s) => s,
            Err(err) => {
                // Edge failed mid-render: stop ffmpeg before the temp file is removed
                let _ = child.kill();
                let _ = child.wait();
                if let Some(t) = tail {
                    let _ = t.join();
                }
                return Err(err);
            }
        };
        export::wait_ffmpeg(&mut child, tail, prog)?;
        tmp.commit(&out)?;
        Ok(Rendered { path: out, width: size.0, height: size.1, animated: true })
    })
}

/// An icon from the library (`library/icons/*.svg`, drawn with `currentColor`) as an HTML layer page
/// with a colour field, so an icon is placed, coloured and rendered like any other HTML layer.
pub fn icon_page(name: &str, svg: &str) -> String {
    let name = name.replace(['<', '>', '&', '"'], " ");
    let svg = svg.trim_start_matches('\u{feff}');
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{name}</title>\n\
         <meta name=\"se:kind\" content=\"layer\">\n<meta name=\"se:desc\" content=\"Icon\">\n\
         <meta name=\"se:tags\" content=\"icon\">\n<meta name=\"se:size\" content=\"512x512\">\n\
         <meta name=\"se:duration\" content=\"4s\">\n<meta name=\"se:field\" content=\"color | color | #ffffff\">\n\
         <style>\nhtml, body {{ margin: 0; background: transparent; overflow: hidden; }}\n\
         body {{ color: {{{{color}}}}; }}\n\
         svg {{ display: block; width: 448px; height: 448px; margin: 32px; \
         filter: drop-shadow(0 6px 14px rgba(0, 0, 0, .45)); }}\n</style>\n</head>\n<body>{svg}</body>\n</html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_becomes_a_page_with_a_colour_field() {
        let page = super::icon_page("Like <3", "<svg viewBox=\"0 0 24 24\"><path d=\"M0 0\"/></svg>");
        assert!(page.contains("color: {{color}};") && page.contains("se:field\" content=\"color | color | #ffffff\""));
        assert!(page.contains("<title>Like  3</title>") && page.contains("<path d=\"M0 0\"/>"));
        assert!(!page.to_ascii_lowercase().contains("<script"));
        assert!(lint(&page).is_empty(), "{:?}", lint(&page));
    }

    const TPL: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<TITLE> Sub &amp; Like </TITLE>
<meta content='scene' name='se:kind'>
<meta name=se:desc content="A button.">
<meta  NAME="se:tags"  content="a, b ,, c">
<meta name="se:size" content="640x220">
<meta name="se:duration" content="1500ms">
<meta name="se:field" content="label | text | Subscribe">
<meta name="se:field" content="accent | color | #e62117">
<meta name="se:field" content="count | number | 120 | 0 | 1000">
<meta name="se:field" content="avatar | image | ">
<style>:root { --a: {{accent}}; } .x { background: url({{avatar}}); }</style></head>
<body><b>{{ label }}</b><i>{{count}}</i>{{unknown}}</body></html>"#;

    #[test]
    fn parses_a_head_whatever_the_quotes_and_order() {
        let t = parse(TPL);
        assert_eq!(t.title, "Sub & Like");
        assert!(t.scene);
        assert_eq!(metas(TPL, "se:desc"), ["A button."]);
        assert_eq!(metas(TPL, "se:tags"), ["a, b ,, c"]);
        assert_eq!(t.size, (640, 220));
        assert_eq!(t.duration, 1.5);
        assert!((t.thumb_at() - 0.9).abs() < 1e-9);
        assert_eq!(t.fields.len(), 4);
        assert_eq!(
            t.fields[2],
            Field {
                name: "count".into(),
                ty: FieldType::Number,
                default: "120".into(),
                min: Some(0.0),
                max: Some(1000.0)
            }
        );
        assert_eq!(t.fields[3].ty, FieldType::Image);
        let d = parse("<html></html>");
        assert_eq!((d.size, d.duration, d.scene, d.fields.len()), ((1920, 1080), 4.0, false, 0));
    }

    #[test]
    fn substitutes_escaped_validated_and_clamped_values() {
        let t = parse(TPL);
        let v = |pairs: &[(&str, &str)]| pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let out =
            substitute(TPL, &t, &v(&[("label", "<b>\"A&B\"</b>"), ("accent", "red;}</style>"), ("count", "5000")]));
        assert!(out.contains("<b>&lt;b&gt;&quot;A&amp;B&quot;&lt;/b&gt;</b>"), "{out}");
        assert!(out.contains("--a: #e62117;"), "an invalid colour falls back to the default: {out}");
        assert!(out.contains("<i>1000</i>"), "clamped to max: {out}");
        assert!(out.contains(&format!("url({TRANSPARENT})")), "an empty image is a transparent pixel");
        assert!(out.contains("{{unknown}}"), "an unknown name stays");
        let out = substitute(TPL, &t, &v(&[("accent", "#00ff0080"), ("count", "-3.5")]));
        assert!(out.contains("--a: #00ff0080;") && out.contains("<i>0</i>") && out.contains("<b>Subscribe</b>"));
        let f = Field { name: "n".into(), ty: FieldType::Number, default: "1".into(), min: None, max: None };
        assert_eq!(field_value(&f, &v(&[("n", "2.25")])), "2.25");
        let img = Field { name: "i".into(), ty: FieldType::Image, default: String::new(), min: None, max: None };
        let exe = std::env::current_exe().unwrap();
        let url = field_value(&img, &[("i".into(), exe.to_string_lossy().into_owned())]);
        assert!(url.starts_with("file:///") && !url.contains('\\') && !url.contains(' '), "{url}");
        assert_eq!(field_value(&img, &[("i".into(), "C:\\nope\\missing.png".into())]), TRANSPARENT);
        assert_eq!(file_url(Path::new(r"C:\a b\(x).png")), "file:///C:/a%20b/%28x%29.png");
    }

    #[test]
    fn sanitising_removes_every_way_to_run_script() {
        let src = "<p>a</p><SCRIPT type=x>alert(1)</script ><p onclick=\"x()\" class='k' ONLOAD=y>b</p>\
                   <a href=\" JaVaScRiPt:alert(1)\">c</a><div style=\"background:url(javascript:x)\">d</div>\
                   <img src=x onerror='alert(1)'/><script>never closed";
        let s = sanitize(src);
        let low = s.to_ascii_lowercase();
        assert!(!low.contains("<script") && !low.contains("alert(1)</") && !low.contains("never closed"), "{s}");
        assert!(!low.contains("onclick") && !low.contains("onload") && !low.contains("onerror"), "{s}");
        assert!(!low.contains("javascript:"), "{s}");
        assert!(s.contains("<p  class='k' >b</p>") || s.contains("class='k'"), "other attributes stay: {s}");
        assert!(s.starts_with("<p>a</p>") && s.contains(">c</a>") && s.contains(">d</div>"));
        let clean = "<style>a > b { color: red }</style><svg viewBox=\"0 0 1 1\"><path d=\"M0 0\"/></svg>";
        assert_eq!(sanitize(clean), clean, "a clean page is left byte for byte");
    }

    #[test]
    fn lint_names_what_breaks_the_rules() {
        let good = TPL.replace("{{unknown}}", "");
        assert!(lint(&good).is_empty(), "{:?}", lint(&good));
        let has = |src: &str, what: &str| {
            let l = lint(src);
            assert!(l.iter().any(|m| m.contains(what)), "expected `{what}` in {l:?}");
        };
        has(TPL, "{{unknown}} is not a declared field");
        has(&good.replace("<b>{{ label }}</b>", ""), "`label` is declared but never used");
        has(&good.replace("<TITLE> Sub &amp; Like </TITLE>", ""), "<title>");
        has(&good.replace("name=se:desc", "name=x"), "se:desc");
        has(&good.replace("640x220", "641x220"), "even");
        has(&good.replace("640x220", "4000x220"), "3840x2160");
        has(&good.replace("1500ms", "90s"), "0.5 to 60");
        has(&good.replace("#e62117", "red"), "#rrggbb");
        has(&good.replace("</body>", "<script>x</script></body>"), "<script>");
        has(&good.replace("<b>", "<b onclick=x>"), "on* attribute");
        has(&good.replace("</style>", "@import url(x.css);</style>"), "@import");
        has(&good.replace("</body>", "<img src=\"https://x.test/a.png\"></body>"), "network URL");
        has(&good.replace("</body>", "<video></video></body>"), "<video>");
        has(&good.replace("</style>", ".x:hover{}</style>"), ":hover");
        has(&good.replace("label | text", "Label | text"), "lowercase");
        let many: String =
            (0..13).map(|i| format!("<meta name=\"se:field\" content=\"f{i} | text | x\">{{{{f{i}}}}}")).collect();
        has(&good.replace("<style>", &format!("{many}<style>")), "at most 12");
        assert!(lint(&good.replace("</body>", "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg></body>")).is_empty());
    }

    #[test]
    fn websocket_frames_round_trip() {
        for n in [0usize, 5, 125, 126, 300, 65535, 65536, 70_000] {
            let payload: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
            let f = encode_frame(1, &payload);
            assert_eq!(f[0], 0x81);
            assert!(f[1] & 0x80 != 0, "client frames are masked");
            let (fin, op, p) = read_frame(&mut std::io::Cursor::new(&f)).unwrap();
            assert!(fin && op == 1 && p == payload, "{n}");
        }
        // a server message in two fragments with a ping between them (unmasked, as servers send)
        let raw = |b0: u8, p: &[u8]| [vec![b0, p.len() as u8], p.to_vec()].concat();
        let stream = [raw(0x01, b"{\"a\":"), raw(0x89, b"hi"), raw(0x80, b"1}")].concat();
        let mut pong = Vec::new();
        let m = read_message(&mut std::io::Cursor::new(&stream), &mut pong).unwrap();
        assert_eq!(m, b"{\"a\":1}");
        assert_eq!(
            read_frame(&mut std::io::Cursor::new(&pong)).unwrap(),
            (true, 0xA, b"hi".to_vec()),
            "the ping was answered"
        );
        // a masked frame decodes
        let masked = [vec![0x82, 0x83, 1, 2, 3, 4], vec![b'a' ^ 1, b'b' ^ 2, b'c' ^ 3]].concat();
        assert_eq!(read_frame(&mut std::io::Cursor::new(&masked)).unwrap().2, b"abc");
        assert!(read_message(&mut std::io::Cursor::new(&raw(0x88, b"")), &mut pong).is_err(), "close");
        let mut out = Vec::new();
        b64_decode(b"aGVsbG8gd29ybGQ=", &mut out);
        assert_eq!(out, b"hello world");
    }

    /// Every library template passes `lint` (read at run time, so new files are covered).
    #[test]
    fn every_library_html_passes_lint() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join("html");
        let mut bad = Vec::new();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "html") {
                let l = lint(&std::fs::read_to_string(&p).unwrap_or_default());
                if !l.is_empty() {
                    bad.push(format!("{}: {}", p.file_name().unwrap_or_default().to_string_lossy(), l.join("; ")));
                }
            }
        }
        assert!(bad.is_empty(), "library/html files breaking the rules:\n{}", bad.join("\n"));
    }

    const MOVING: &str = r#"<!doctype html><html><head><title>t</title><meta name="se:size" content="320x180">
<style>html,body{margin:0;background:transparent;overflow:hidden}
.m{position:absolute;left:0;top:20px;width:20px;height:20px;background:#ff0000;animation:mv 2s linear both}
@keyframes mv{from{left:0}to{left:200px}}
.h{position:absolute;left:250px;top:100px;width:40px;height:40px;background:rgba(255,255,255,.5)}
#s{position:absolute;left:0;top:150px;width:10px;height:10px;background:#00ff00;display:none}</style></head>
<body onload="document.getElementById('s').style.display='block'"><div class="m"></div><div class="h"></div><div id="s"></div>
<script>document.getElementById('s').style.display='block'</script></body></html>"#;

    fn tools_missing() -> bool {
        let missing = edge_exe().is_none() || ffpipe::ffmpeg_exe().is_none();
        if missing {
            eprintln!("SKIPPED: Microsoft Edge or ffmpeg.exe not found");
        }
        missing
    }

    /// The real thing: Edge renders a moving box at exact positions with straight alpha, a script in
    /// the page does not run, and the full render writes a PNG-in-MOV that decodes to the same frames.
    #[test]
    fn renders_real_frames_through_edge() {
        if tools_missing() {
            return;
        }
        let px = |f: &Frame, x: u32, y: u32| -> [u8; 4] {
            f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap()
        };
        let check = |f: &Frame, left: u32| {
            assert_eq!((f.width, f.height), (320, 180));
            assert_eq!(px(f, left + 10, 30), [255, 0, 0, 255], "the box at {left}");
            assert_eq!(px(f, left.saturating_sub(5), 30)[3], 0, "left of the box at {left}");
            assert_eq!(px(f, left + 25, 30)[3], 0, "right of the box at {left}");
            let h = px(f, 270, 120);
            assert!(
                h[0] >= 254 && h[1] >= 254 && h[2] >= 254 && (127..=129).contains(&h[3]),
                "half white, straight alpha: {h:?}"
            );
            assert_eq!(px(f, 5, 5)[3], 0, "transparent background");
            assert_eq!(px(f, 5, 155)[3], 0, "the page's script did not run");
        };
        let t0 = Instant::now();
        let f = render_frame(MOVING, &[], 1.0, 0.5).expect("render");
        eprintln!("first frame (Edge start included): {} ms", t0.elapsed().as_millis());
        check(&f, 50);
        let spec = Spec { html: MOVING.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 2.5 };
        if let Some(r) = cached(&spec) {
            let _ = std::fs::remove_file(r.path);
        }
        let t0 = Instant::now();
        let r = render_to_file(&spec, &Progress::new()).expect("render to file");
        eprintln!(
            "render_to_file: 25 frames 320x180 in {} ms = {:.1} ms a frame",
            t0.elapsed().as_millis(),
            t0.elapsed().as_secs_f64() * 1000.0 / 25.0
        );
        assert!(r.animated && r.path.extension().is_some_and(|e| e == "mov") && (r.width, r.height) == (320, 180));
        let mut v = ffpipe::open_video(&r.path.to_string_lossy()).expect("open the render");
        for (t, left) in [(0.5, 50), (1.5, 150)] {
            let mut f = Frame::default();
            assert!(v.frame_at(t, 320, 180, &mut f), "decode {t}");
            check(&f, left);
        }
        drop(v);
        assert!(cached(&spec).is_some(), "the next ask is a cache hit");
        let _ = std::fs::remove_file(&r.path);
        // a page without animation is one PNG
        let still = MOVING.replace("animation:mv 2s linear both", "");
        let spec = Spec { html: still, ..spec };
        let r = render_to_file(&spec, &Progress::new()).expect("still");
        assert!(!r.animated && r.path.extension().is_some_and(|e| e == "png"));
        let _ = std::fs::remove_file(&r.path);
        shutdown();
    }

    /// Renders every library template's thumbnail frame and every icon, composited over #2b2f3a, into
    /// the folder named by env SE_THUMBS (`html-<id>.png`, `icons-<id>.png`); fails listing every file
    /// whose frame came out fully transparent.
    /// `SE_THUMBS=C:\tmp\thumbs cargo test library_html_thumbs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn library_html_thumbs() {
        if tools_missing() {
            return;
        }
        let out = PathBuf::from(std::env::var("SE_THUMBS").expect("set SE_THUMBS to an output folder"));
        std::fs::create_dir_all(&out).unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("library");
        let mut jobs: Vec<(String, String)> = Vec::new();
        for (sub, ext) in [("html", "html"), ("icons", "svg")] {
            let mut files: Vec<PathBuf> =
                std::fs::read_dir(root.join(sub)).into_iter().flatten().flatten().map(|e| e.path()).collect();
            files.sort();
            for p in files.into_iter().filter(|p| p.extension().is_some_and(|e| e == ext)) {
                let id = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let html = if sub == "icons" { icon_page(&id, &text) } else { text };
                jobs.push((format!("{sub}-{id}"), html));
            }
        }
        let mut empty = Vec::new();
        for (name, html) in &jobs {
            let tpl = parse(html);
            let scale = 480.0 / tpl.size.0.max(tpl.size.1) as f64;
            match render_frame(html, &[], scale, tpl.thumb_at()) {
                Ok(mut f) => {
                    if f.rgba.chunks_exact(4).all(|p| p[3] == 0) {
                        empty.push(name.clone());
                    }
                    for p in f.rgba.chunks_exact_mut(4) {
                        let a = p[3] as u32;
                        for (c, bg) in p.iter_mut().zip([0x2b_u32, 0x2f, 0x3a]) {
                            *c = ((*c as u32 * a + bg * (255 - a) + 127) / 255) as u8;
                        }
                        p[3] = 255;
                    }
                    std::fs::write(out.join(format!("{name}.png")), crate::mcp::png_encode(&f)).unwrap();
                    eprintln!("{name}: {}x{}", f.width, f.height);
                }
                Err(e) => empty.push(format!("{name} ({e})")),
            }
        }
        shutdown();
        assert!(empty.is_empty(), "empty or failed frames:\n{}", empty.join("\n"));
    }
}
