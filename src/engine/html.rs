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
    /// `se:fullpage`: a page without animation is captured over its whole scroll height (a web page
    /// snapshot), not just the `se:size` viewport. The viewport still decides `vh`, `%` and media queries.
    pub fullpage: bool,
    /// `se:param` (number fields, animatable): each is the CSS variable `--name` while rendering.
    pub params: Vec<Field>,
}

/// Everything about the page's surroundings a render depends on besides its source and fields.
#[derive(Clone, Copy, Default)]
pub struct View<'a> {
    /// The folder of the page's file (relative links resolve there); empty = none.
    pub base: &'a str,
    pub params: &'a [(String, crate::model::Animated)],
    /// The layout width in CSS px (0 = the template's `se:size`).
    pub width: u32,
    pub mouse: Option<&'a crate::model::Mouse>,
}

/// Param tracks as the renderer takes them: (name, constant or keyframed value).
pub type Params = Vec<(String, crate::model::Animated)>;

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
        params: metas(src, "se:param")
            .iter()
            .filter_map(|f| field_of(f))
            .filter(|f| matches!(f.ty, FieldType::Number))
            .collect(),
        // a page with no se: tags at all is an ordinary web page: capture all of it
        fullpage: first("se:fullpage")
            .map_or_else(|| first("se:size").is_none() && first("se:kind").is_none() && first("se:duration").is_none(), |v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no")),
    }
}

// ---------------------------------------------------------------------------------------------------
// params: numbers a page declares (`<meta name="se:param" content="progress | number | 0 | 0 | 100">`),
// keyframed in the editor, handed to the page as CSS variables (`var(--progress)`) frame by frame

/// The page's declared params with their tracks from `set` (a missing one is its default), in
/// declaration order.
pub fn params_of(src: &str, set: &std::collections::BTreeMap<String, crate::model::Animated>) -> Params {
    parse(src)
        .params
        .iter()
        .map(|p| {
            let default = p.default.trim().parse::<f64>().unwrap_or(0.0);
            (p.name.clone(), set.get(&p.name).cloned().unwrap_or_else(|| crate::model::Animated::new(default)))
        })
        .collect()
}

/// (name, value) of every declared param at second `t`; values outside the declared range are clamped.
fn values_at(tpl: &Template, params: &[(String, crate::model::Animated)], t: f64) -> Vec<(String, f64)> {
    tpl.params
        .iter()
        .map(|p| {
            let default = p.default.trim().parse::<f64>().unwrap_or(0.0);
            let v = params.iter().find(|(n, _)| *n == p.name).map_or(default, |(_, a)| a.at(t));
            let v = match (p.min, p.max) {
                (Some(a), Some(b)) if a <= b => v.clamp(a, b),
                _ => v,
            };
            (p.name.clone(), if v.is_finite() { v } else { default })
        })
        .collect()
}

fn animated_params(tpl: &Template, params: &[(String, crate::model::Animated)]) -> bool {
    params.iter().any(|(n, a)| a.is_animated() && tpl.params.iter().any(|p| p.name == *n))
}

// ---------------------------------------------------------------------------------------------------
// linked files: a page that refers to local stylesheets, images and fonts, like a real browser

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A reference found in a page or a stylesheet, as a local file (None: a web address, `data:`, an anchor,
/// a `{{field}}`, nothing).
fn resolve_ref(r: &str, dir: &Path) -> Option<PathBuf> {
    let r = r.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    let low = r.to_ascii_lowercase();
    if r.is_empty()
        || r.starts_with('#')
        || r.contains("{{")
        || ["data:", "http:", "https:", "//", "about:", "blob:", "javascript:", "cid:"].iter().any(|p| low.starts_with(p))
    {
        return None;
    }
    let r = r.split(['?', '#']).next().unwrap_or("");
    let r = r.strip_prefix("file:///").unwrap_or(r);
    if r.is_empty() {
        return None;
    }
    let d = percent_decode(r);
    let p = Path::new(&d);
    Some(if p.is_absolute() || d.as_bytes().get(1) == Some(&b':') { p.to_path_buf() } else { dir.join(p) })
}

/// Every `url(…)`, `@import "…"` and `<link href>` / `<img src>` / `<source src>` / `poster` / SVG `href`.
fn refs_in(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("url(") {
        let t = &rest[i + 4..];
        let end = t.find(')').unwrap_or(t.len());
        v.push(t[..end].trim().trim_matches(|c| c == '"' || c == '\'').to_string());
        rest = &t[end..];
    }
    let mut rest = text;
    while let Some(i) = rest.find("@import") {
        let t = rest[i + 7..].trim_start();
        if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') {
            if let Some(e) = t[1..].find(q) {
                v.push(t[1..1 + e].to_string());
            }
        }
        rest = &rest[i + 7..];
    }
    for (tag, names) in [("link", &["href"][..]), ("img", &["src"]), ("source", &["src"]), ("video", &["poster"]), ("image", &["href", "xlink:href"]), ("use", &["href", "xlink:href"])] {
        for t in open_tags(text, tag) {
            for a in attrs(t) {
                if names.contains(&a.name.as_str()) {
                    v.push(unescape(a.value));
                }
            }
        }
    }
    v
}

/// The local files a page needs (one level of stylesheets inside stylesheets and up to 3 deep), resolved
/// against `base` (the folder the page lives in). Only files that exist are listed.
pub fn local_deps(page: &str, base: &str) -> Vec<PathBuf> {
    if base.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let mut work: Vec<(String, PathBuf, usize)> = vec![(page.to_string(), PathBuf::from(base), 0)];
    while let Some((text, dir, depth)) = work.pop() {
        for r in refs_in(&text) {
            let Some(p) = resolve_ref(&r, &dir) else { continue };
            if out.contains(&p) || !p.is_file() {
                continue;
            }
            let is_css = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("css"));
            if is_css && depth < 3 {
                if let Ok(css) = std::fs::read_to_string(&p) {
                    work.push((css, p.parent().map(Path::to_path_buf).unwrap_or_default(), depth + 1));
                }
            }
            out.push(p);
        }
        if out.len() > 2000 {
            break;
        }
    }
    out
}

/// A number that changes when any linked file does (its name, size and modified time).
pub fn deps_stamp(page: &str, base: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut files = local_deps(page, base);
    files.sort();
    for f in files {
        let m = std::fs::metadata(&f).ok();
        (f, m.as_ref().map(|m| m.len()), m.and_then(|m| m.modified().ok())).hash(&mut h);
    }
    h.finish()
}

/// Remove every `<base …>` tag (a saved page's own base would point its links back at the web).
pub fn strip_base_tags(page: &str) -> String {
    let low = page.to_ascii_lowercase();
    let (mut out, mut at) = (String::with_capacity(page.len()), 0);
    while let Some(i) = low[at..].find("<base").map(|p| p + at) {
        if !low.as_bytes().get(i + 5).is_some_and(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/') {
            out.push_str(&page[at..i + 5]);
            at = i + 5;
            continue;
        }
        out.push_str(&page[at..i]);
        at = tag_end(page.as_bytes(), i);
    }
    out.push_str(&page[at..]);
    out
}

/// `<base href="…">` right after `<head>` so relative links in the page resolve against `base`.
fn with_base(page: String, base: &str) -> String {
    if base.is_empty() || page.to_ascii_lowercase().contains("<base ") {
        return page;
    }
    let tag = format!("<base href=\"{}/\">", file_url(Path::new(base)).trim_end_matches('/'));
    let low = page.to_ascii_lowercase();
    for open in ["<head", "<html"] {
        if let Some(i) = low.find(open).filter(|&i| low.as_bytes().get(i + open.len()).is_some_and(|c| c.is_ascii_whitespace() || *c == b'>')) {
            let at = tag_end(page.as_bytes(), i);
            return format!("{}{}{}", &page[..at], tag, &page[at..]);
        }
    }
    format!("{tag}{page}")
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

/// Params declared but never read as `var(--name)`.
fn unused_params(src: &str) -> Vec<String> {
    parse(src)
        .params
        .iter()
        .filter(|p| !src.contains(&format!("var(--{}", p.name)))
        .map(|p| format!("param '{}' is declared but never used (read it as var(--{}))", p.name, p.name))
        .collect()
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
    e.extend(unused_params(src));
    e
}

/// Template + the page Edge loads: fields substituted, then sanitised.
pub fn prepare(src: &str, values: &[(String, String)]) -> (Template, String) {
    prepare_in(src, values, "")
}

/// `prepare`, for a page that lives in folder `base` (a linked or imported file): its relative links to
/// stylesheets, images and fonts resolve there.
pub fn prepare_in(src: &str, values: &[(String, String)], base: &str) -> (Template, String) {
    let tpl = parse(src);
    let page = with_base(sanitize(&substitute(src, &tpl, values)), base);
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

/// Where the fake mouse has been (so only changes are sent to the page).
struct MouseState {
    x: f64,
    y: f64,
    down: bool,
    ready: bool,
    press_ms: f64,
}

impl Default for MouseState {
    fn default() -> Self {
        MouseState { x: 0.0, y: 0.0, down: false, ready: false, press_ms: -1e9 }
    }
}

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
        Self::launch_with(false)
    }

    /// `network`: let the page reach the web (a private Edge that snapshots a site); off for rendering,
    /// where a page must not fetch anything.
    fn launch_with(network: bool) -> Result<Edge, String> {
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
            .args(["--disable-sync", "--disable-component-update"])
            .args(if network { vec![] } else { vec!["--host-resolver-rules=MAP * ~NOTFOUND"] })
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

    /// Make the page safe to click (links and forms go nowhere) and put a cursor on it.
    fn mouse_setup(&mut self) -> Result<(), String> {
        self.cdp
            .eval(
                r##"(()=>{
for(const a of document.querySelectorAll('a[href],area[href]'))a.setAttribute('href','javascript:void(0)');
for(const f of document.querySelectorAll('form'))f.setAttribute('action','javascript:void(0)');
for(const b of document.querySelectorAll('[formaction]'))b.setAttribute('formaction','javascript:void(0)');
if(!document.getElementById('se-cursor')){
 const c=document.createElement('div');c.id='se-cursor';
 c.style.cssText='position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none';
 c.innerHTML='<div id="se-ring" style="position:absolute;left:-26px;top:-26px;width:52px;height:52px;border-radius:50%;border:4px solid rgba(61,123,253,.95);opacity:0"></div>'
 +'<svg id="se-arrow" width="30" height="40" viewBox="0 0 26 34" style="position:absolute;left:-2px;top:-2px;filter:drop-shadow(0 2px 3px rgba(0,0,0,.45))"><path d="M2 2 L2 26 L8.5 20.5 L13 31 L18 29 L13.5 19 L22 19 Z" fill="#fff" stroke="#111" stroke-width="2" stroke-linejoin="round"/></svg>';
 document.documentElement.appendChild(c);
}
})()"##,
            )
            .map(|_| ())
    }

    /// Move / press / release the page's real mouse to match the fake one, and draw the cursor (with a
    /// ring that opens and fades for 0.45 s after a press). Events go to the page itself, so `:hover` and
    /// `:active` styles apply and the transitions they start can be stepped by `seek_rel`.
    fn mouse_frame(&mut self, st: &mut MouseState, x: f64, y: f64, down: bool, ms: f64) -> Result<(), String> {
        let ev = |ty: &str, buttons: u32| {
            json!({"type": ty, "x": x, "y": y, "button": if buttons > 0 || ty == "mouseReleased" { "left" } else { "none" },
                   "buttons": buttons, "clickCount": u32::from(ty != "mouseMoved")})
        };
        if !st.ready || (x, y) != (st.x, st.y) {
            self.cdp.call("Input.dispatchMouseEvent", ev("mouseMoved", u32::from(down)))?;
        }
        if down && !st.down {
            self.cdp.call("Input.dispatchMouseEvent", ev("mousePressed", 1))?;
            st.press_ms = ms;
        }
        if !down && st.down {
            self.cdp.call("Input.dispatchMouseEvent", ev("mouseReleased", 0))?;
        }
        let ring = ((ms - st.press_ms) / 450.0).clamp(0.0, 1.0);
        let ring = if st.press_ms < -1e8 { 1.0 } else { ring };
        self.cdp.eval(&format!(
            "(()=>{{const c=document.getElementById('se-cursor');if(!c)return;c.style.transform='translate({x}px,{y}px)';\
             const r=document.getElementById('se-ring');r.style.opacity='{}';r.style.transform='scale({})'}})()",
            if ring >= 1.0 { 0.0 } else { 1.0 - ring },
            0.4 + ring * 1.1
        ))?;
        *st = MouseState { x, y, down, ready: true, press_ms: st.press_ms };
        Ok(())
    }

    /// Every animation paused at `ms` since it began: the ones the page had at the start began at 0, one
    /// started later (a hover's transition) began when first seen, so it plays from its own start.
    fn seek_rel(&mut self, ms: f64) -> Result<(), String> {
        self.cdp
            .eval(&format!(
                "(()=>{{const ms={ms};for(const a of document.getAnimations()){{if(a.__t0===undefined)a.__t0=window.__seFirst===undefined?0:ms;\
                 a.pause();a.currentTime=ms-a.__t0}}window.__seFirst=1}})()"
            ))
            .map(|_| ())
    }

    /// The page's params as CSS variables on `<html>` (set from here, so no script runs in the page).
    fn set_params(&mut self, values: &[(String, f64)]) -> Result<(), String> {
        if values.is_empty() {
            return Ok(());
        }
        let obj: serde_json::Map<String, Value> = values.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        self.cdp
            .eval(&format!(
                "(()=>{{const s=document.documentElement.style;for(const [k,v] of Object.entries({})){{s.setProperty('--'+k,String(v))}}}})()",
                Value::Object(obj)
            ))
            .map(|_| ())
    }

    /// The whole scrollable page (at most 16000 CSS px tall) as PNG bytes.
    fn shot_full(&mut self, width: u32, out: &mut Vec<u8>) -> Result<(), String> {
        let h = self
            .cdp
            .eval("Math.max(document.documentElement.scrollHeight,document.body?document.body.scrollHeight:0)")?
            .as_f64()
            .unwrap_or(0.0)
            .clamp(1.0, 16000.0);
        let clip = json!({"x": 0, "y": 0, "width": width, "height": h, "scale": 1});
        let r = self.cdp.call("Page.captureScreenshot", json!({"format": "png", "captureBeyondViewport": true, "clip": clip}))?;
        b64_decode(r["data"].as_str().unwrap_or("").as_bytes(), out);
        if png_size(out).is_none() {
            return Err("Edge returned no picture".into());
        }
        Ok(())
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

/// A page from the web as an MHTML archive, the way a browser's own "save as single file" makes it: loaded
/// in a private Edge (network and scripts on, nothing kept), scrolled once to wake lazy images, then
/// snapshotted. It is what a visitor who is not signed in sees; the page you are signed in to comes from
/// your own browser (see `tools/browser-extension`).
pub fn capture_mhtml(url: &str, width: u32, height: u32, prog: &Progress) -> Result<Vec<u8>, String> {
    let url = url.trim();
    let url = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http and https addresses can be snapshotted".into());
    }
    prog.set(0.05, "Starting Edge…");
    let mut e = Edge::launch_with(true)?;
    let r = (|| -> Result<Vec<u8>, String> {
        let c = &mut e.cdp;
        c.call("Emulation.setScriptExecutionDisabled", json!({"value": false}))?;
        c.call("Emulation.setDefaultBackgroundColorOverride", json!({}))?;
        let session = c.session.take();
        let ua = c.call("Browser.getVersion", json!({}))?["userAgent"].as_str().unwrap_or("").replace("HeadlessChrome", "Chrome");
        c.session = session;
        if !ua.is_empty() {
            c.call("Emulation.setUserAgentOverride", json!({"userAgent": ua}))?;
        }
        let m = json!({"width": width.clamp(320, 3840), "height": height.clamp(240, 2160), "deviceScaleFactor": 1, "mobile": false});
        c.call("Emulation.setDeviceMetricsOverride", m)?;
        prog.set(0.1, "Loading the page…");
        c.loaded = false;
        let nav = c.call("Page.navigate", json!({"url": url}))?;
        if let Some(err) = nav["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(format!("Edge could not open {url}: {err}"));
        }
        while !c.loaded {
            c.recv().map_err(|e| format!("Edge (loading {url}): {e}"))?;
        }
        prog.set(0.5, "Scrolling through the page…");
        let wake = "(async()=>{const h=()=>Math.max(document.documentElement.scrollHeight,document.body?document.body.scrollHeight:0);\
            let y=0;while(y<h()&&y<24000){window.scrollTo(0,y);await new Promise(r=>setTimeout(r,140));y+=innerHeight*0.8}\
            window.scrollTo(0,0);await new Promise(r=>setTimeout(r,700));try{await document.fonts.ready}catch(e){}return h()})()";
        c.call("Runtime.evaluate", json!({"expression": wake, "awaitPromise": true, "returnByValue": true}))?;
        prog.set(0.85, "Saving the page…");
        let snap = c.call("Page.captureSnapshot", json!({"format": "mhtml"}))?;
        let data = snap["data"].as_str().ok_or("Edge returned no snapshot")?;
        Ok(data.as_bytes().to_vec())
    })();
    e.close();
    r
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
pub fn png_file_size(path: &Path) -> Option<(u32, u32)> {
    let mut head = [0u8; 32];
    let mut f = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut f, &mut head).ok()?;
    png_size(&head)
}

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
    render_png_in(html, fields, scale, t, &View::default())
}

/// `render_png` for a page in its surroundings: its folder, `se:param` tracks and fake mouse evaluated at
/// `t`, and the layout width.
pub fn render_png_in(html: &str, fields: &[(String, String)], scale: f64, t: f64, v: &View) -> Result<Vec<u8>, String> {
    let (tpl, page) = prepare_in(html, fields, v.base);
    let scale = scale.clamp(0.05, 4.0);
    let values = values_at(&tpl, v.params, t);
    let size = sized(&tpl, v.width);
    with_edge(|e| {
        e.load(&page, size, scale)?;
        e.set_params(&values)?;
        if let Some(m) = v.mouse {
            // a still of the moment: the mouse is where it is at `t`, its hover has settled
            let mut st = MouseState::default();
            e.mouse_setup()?;
            let (x, y, down) = m.state(t);
            e.mouse_frame(&mut st, x, y, down, t * 1000.0)?;
        }
        let (count, _, _) = e.timing()?;
        let mut png = Vec::new();
        if tpl.fullpage && count == 0 {
            e.shot_full(size.0, &mut png)?;
        } else {
            e.seek((t.max(0.0) * 1000.0).min(1e9))?;
            e.shot(&mut png)?;
        }
        Ok(png)
    })
}

/// CONTRACT: one frame (straight-alpha RGBA) of `html` at `t` seconds. Blocking; never on the UI thread.
pub fn render_frame(html: &str, fields: &[(String, String)], scale: f64, t: f64) -> Result<Frame, String> {
    decode_png(&render_png(html, fields, scale, t)?)
}

/// `render_frame` for a page in its surroundings.
pub fn render_frame_in(html: &str, fields: &[(String, String)], scale: f64, t: f64, v: &View) -> Result<Frame, String> {
    decode_png(&render_png_in(html, fields, scale, t, v)?)
}

/// What a full render needs; the output file is named by a hash of all of it.
#[derive(Clone, Debug)]
pub struct Spec {
    pub html: String,
    pub fields: Vec<(String, String)>,
    pub scale: f64,
    pub fps: f64,
    pub duration: f64,
    /// The folder of the page's file when it has one (relative links resolve there); empty = none.
    pub base: String,
    /// The page's `se:param` tracks.
    pub params: Params,
    /// The layout width in CSS pixels (0 = the template's `se:size`).
    pub width: u32,
    /// The fake mouse.
    pub mouse: Option<crate::model::Mouse>,
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

/// The viewport of a render: the template's `se:size`, or its height at the width asked for.
fn sized(tpl: &Template, width: u32) -> (u32, u32) {
    if width > 0 {
        (width.clamp(2, 3840), tpl.size.1)
    } else {
        tpl.size
    }
}

/// The part of the params a render depends on: declared ones only.
fn params_hashable<'a>(tpl: &Template, params: &'a [(String, crate::model::Animated)]) -> Vec<&'a (String, crate::model::Animated)> {
    params.iter().filter(|(n, _)| tpl.params.iter().any(|p| p.name == *n)).collect()
}

/// `cache_dir()/html/<hash>` (no extension) and the picture size for `spec`.
pub fn out_base(spec: &Spec) -> (PathBuf, (u32, u32)) {
    use std::hash::{Hash, Hasher};
    let (tpl, page) = prepare_in(&spec.html, &spec.fields, &spec.base);
    let px = pixel_size(sized(&tpl, spec.width), spec.scale.clamp(0.05, 4.0));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let params = serde_json::to_string(&params_hashable(&tpl, &spec.params)).unwrap_or_default();
    let mouse = serde_json::to_string(&spec.mouse).unwrap_or_default();
    ("html-v4", &page, px, spec.fps.to_bits(), spec.duration.to_bits(), deps_stamp(&page, &spec.base), params, mouse).hash(&mut h);
    (dir().join(format!("{:016x}", h.finish())), px)
}

/// The render for `spec` if it is already in the cache (no render).
pub fn cached(spec: &Spec) -> Option<Rendered> {
    let (base, (width, height)) = out_base(spec);
    [("mov", true), ("png", false)].into_iter().find_map(|(ext, animated)| {
        let path = base.with_extension(ext);
        if !path.is_file() {
            return None;
        }
        // a full-page capture is as tall as its page: ask the file
        let (width, height) = if animated { (width, height) } else { png_file_size(&path).unwrap_or((width, height)) };
        Some(Rendered { path, width, height, animated })
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
    let (tpl, page) = prepare_in(&spec.html, &spec.fields, &spec.base);
    let size = sized(&tpl, spec.width);
    let (scale, fps) = (spec.scale.clamp(0.05, 4.0), spec.fps.clamp(1.0, 240.0));
    let frames = (spec.duration.clamp(0.04, 3600.0) * fps).ceil().max(1.0) as usize;
    std::fs::create_dir_all(dir()).map_err(|e| format!("{}: {e}", dir().display()))?;
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    prog.set(0.0, "Rendering HTML…");
    // a keyframed param or a mouse moves the picture as surely as a CSS animation does
    let moving_params = animated_params(&tpl, &spec.params) || spec.mouse.is_some();
    with_edge(|e| {
        e.load(&page, size, scale)?;
        e.set_params(&values_at(&tpl, &spec.params, 0.0))?;
        let mut mouse_state = MouseState::default();
        if spec.mouse.is_some() {
            e.mouse_setup()?;
        }
        let (count, infinite, end_ms) = e.timing()?;
        let infinite = infinite || moving_params;
        let mut png = Vec::new();
        if count == 0 && !moving_params {
            if tpl.fullpage {
                e.shot_full(size.0, &mut png)?;
            } else {
                e.shot(&mut png)?;
            }
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
                    e.set_params(&values_at(&tpl, &spec.params, ms / 1000.0))?;
                    if let Some(m) = &spec.mouse {
                        let (x, y, down) = m.state(ms / 1000.0);
                        e.mouse_frame(&mut mouse_state, x, y, down, ms)?;
                        e.seek_rel(ms)?;
                    } else {
                        e.seek(ms)?;
                    }
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

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-html-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn references_to_local_files_are_found_and_web_addresses_are_not() {
        let d = Path::new("C:/site");
        assert_eq!(resolve_ref("css/a.css", d), Some(PathBuf::from("C:/site/css/a.css")));
        assert_eq!(resolve_ref("\"my%20pic.png?v=3#x\"", d), Some(PathBuf::from("C:/site/my pic.png")));
        assert_eq!(resolve_ref("file:///D:/x/y.png", d), Some(PathBuf::from("D:/x/y.png")));
        for no in ["https://a.b/c.css", "//cdn/x.js", "data:image/png;base64,AAAA", "#top", "{{pic}}", "", "about:blank"] {
            assert_eq!(resolve_ref(no, d), None, "{no}");
        }
        let page = r#"<link rel="stylesheet" href="s.css"><img src='p.png'><style>a{background:url("b/c.png")} @import "i.css";</style>"#;
        let r = refs_in(page);
        for want in ["s.css", "p.png", "b/c.png", "i.css"] {
            assert!(r.iter().any(|x| x == want), "{want} in {r:?}");
        }
    }

    #[test]
    fn a_base_folder_goes_into_the_head_and_linked_files_change_the_stamp() {
        let d = temp_dir("deps");
        std::fs::create_dir_all(d.join("img")).unwrap();
        std::fs::write(d.join("s.css"), "body{background:url(img/a.png)} @import 'more.css';").unwrap();
        std::fs::write(d.join("more.css"), "p{color:red}").unwrap();
        std::fs::write(d.join("img/a.png"), b"x").unwrap();
        let page = r#"<html><head><title>t</title><link rel="stylesheet" href="s.css"></head><body></body></html>"#;
        let deps = local_deps(page, &d.to_string_lossy());
        for f in ["s.css", "more.css", "img/a.png"] {
            assert!(deps.contains(&d.join(f)), "{f} in {deps:?}");
        }
        let (_, prepared) = prepare_in(page, &[], &d.to_string_lossy());
        assert!(prepared.contains("<head><base href=\"file:///"), "{prepared}");
        assert!(!prepare_in(page, &[], "").1.contains("<base"));
        let (a, b) = (deps_stamp(page, &d.to_string_lossy()), deps_stamp(page, &d.to_string_lossy()));
        assert_eq!(a, b);
        std::fs::write(d.join("more.css"), "p{color:blue;margin:0}").unwrap(); // a stylesheet imported by a stylesheet
        assert_ne!(a, deps_stamp(page, &d.to_string_lossy()), "a changed nested file changes the render's cache key");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Edge renders what a real browser would: the stylesheet and the picture are separate files.
    #[test]
    fn linked_stylesheets_and_images_render_like_a_browser() {
        if tools_missing() {
            return;
        }
        let d = temp_dir("linked");
        std::fs::write(d.join("site.css"), "body{margin:0;background:transparent} #a{position:absolute;left:0;top:0;width:60px;height:60px;background:rgb(255,0,0)} #b{position:absolute;left:100px;top:0;width:60px;height:60px;background:url(dot.png) no-repeat;background-size:60px 60px}").unwrap();
        // a 1x1 green PNG
        let green = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41,
            0x54, 0x08, 0xD7, 0x63, 0x60, 0xF8, 0xCF, 0x00, 0x00, 0x02, 0x03, 0x01, 0x01, 0x6B, 0x41, 0xDA, 0xEE, 0x00, 0x00, 0x00,
            0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(d.join("dot.png"), green).unwrap();
        let page = r#"<!doctype html><html><head><title>t</title><meta name="se:size" content="200x100"><link rel="stylesheet" href="site.css"></head><body><div id="a"></div><div id="b"></div></body></html>"#;
        let px = |f: &Frame, x: u32, y: u32| -> [u8; 4] { f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap() };
        let base = d.to_string_lossy().into_owned();
        let f = render_frame_in(page, &[], 1.0, 0.0, &View { base: &base, ..Default::default() }).expect("render");
        assert_eq!(px(&f, 10, 10), [255, 0, 0, 255], "the stylesheet applied");
        assert!(px(&f, 130, 10)[1] > 100 && px(&f, 130, 10)[3] == 255, "the css image loaded: {:?}", px(&f, 130, 10));
        // without the folder the same page has neither
        let f = render_frame(page, &[], 1.0, 0.0).expect("render");
        assert_eq!(px(&f, 10, 10)[3], 0, "no folder: no stylesheet");
        // the cache key follows the stylesheet
        let spec = Spec { html: page.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 1.0, base, params: Vec::new(), width: 0, mouse: None };
        let k1 = out_base(&spec).0;
        std::fs::write(d.join("site.css"), "body{margin:0}").unwrap();
        assert_ne!(k1, out_base(&spec).0);
        let _ = std::fs::remove_dir_all(&d);
    }

    const PARAM_PAGE: &str = r#"<!doctype html><html><head><title>p</title><meta name="se:size" content="200x60">
        <meta name="se:param" content="x | number | 10 | 0 | 150"><meta name="se:param" content="size | number | 20 | 5 | 50">
        <meta name="se:param" content="unused | number | 1">
        <style>html,body{margin:0;background:transparent}
        #b{position:absolute;top:0;left:calc(var(--x) * 1px);width:calc(var(--size) * 1px);height:calc(var(--size) * 1px);background:rgb(255,0,0)}</style>
        </head><body><div id="b"></div></body></html>"#;

    fn track(keys: &[(f64, f64)]) -> crate::model::Animated {
        let mut a = crate::model::Animated::new(keys[0].1);
        a.keys = keys.iter().map(|&(t, v)| crate::model::Keyframe { t, v, ease: crate::model::Ease::Linear }).collect();
        a
    }

    #[test]
    fn params_are_declared_defaulted_clamped_and_linted() {
        let tpl = parse(PARAM_PAGE);
        assert_eq!(tpl.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["x", "size", "unused"]);
        let set = std::collections::BTreeMap::from([("x".to_string(), track(&[(0.0, 0.0), (1.0, 100.0)]))]);
        let p = params_of(PARAM_PAGE, &set);
        assert_eq!(p.len(), 3);
        assert_eq!(p[1].1.value, 20.0, "a param nobody set is its default");
        let v = |t| values_at(&tpl, &p, t).into_iter().map(|(_, v)| v).collect::<Vec<_>>();
        assert_eq!(v(0.5), [50.0, 20.0, 1.0]);
        let wild = vec![("x".to_string(), crate::model::Animated::new(9999.0)), ("size".to_string(), crate::model::Animated::new(f64::NAN))];
        assert_eq!(values_at(&tpl, &wild, 0.0), [("x".to_string(), 150.0), ("size".to_string(), 20.0), ("unused".to_string(), 1.0)]);
        // keyed -> the page moves
        assert!(animated_params(&tpl, &p) && !animated_params(&tpl, &params_of(PARAM_PAGE, &Default::default())));
        // declared but never read
        let l = lint(PARAM_PAGE);
        assert!(l.iter().any(|m| m.contains("'unused'")) && !l.iter().any(|m| m.contains("'x'")), "{l:?}");
        // a render's cache key follows the tracks, and ignores undeclared ones
        let spec = |params: Params| Spec { html: PARAM_PAGE.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 1.0, base: String::new(), params, width: 0, mouse: None };
        assert_ne!(out_base(&spec(p.clone())).0, out_base(&spec(params_of(PARAM_PAGE, &Default::default()))).0);
        let mut extra = p.clone();
        extra.push(("nope".into(), crate::model::Animated::new(1.0)));
        assert_eq!(out_base(&spec(p)).0, out_base(&spec(extra)).0);
    }

    /// The real thing: a keyframed number moves a box through CSS, frame by frame, with no CSS animation
    /// and no script in the page; a constant one is applied to a still.
    #[test]
    fn a_keyframed_param_drives_the_page_through_a_css_variable() {
        if tools_missing() {
            return;
        }
        let px = |f: &Frame, x: u32, y: u32| -> [u8; 4] { f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap() };
        let set = std::collections::BTreeMap::from([("x".to_string(), track(&[(0.0, 0.0), (1.0, 100.0)]))]);
        let params = params_of(PARAM_PAGE, &set);
        // one preview frame at 0.5 s: the box is at x = 50 (20 px wide, the default size)
        let f = render_frame_in(PARAM_PAGE, &[], 1.0, 0.5, &View { params: &params, ..Default::default() }).expect("render");
        assert_eq!(px(&f, 55, 10), [255, 0, 0, 255], "inside the box at x=50");
        assert_eq!(px(&f, 45, 10)[3], 0, "left of it");
        assert_eq!(px(&f, 75, 10)[3], 0, "right of it");
        // a constant: still the default 10 px
        let f = render_frame_in(PARAM_PAGE, &[], 1.0, 0.0, &View::default()).expect("render");
        assert_eq!(px(&f, 15, 10), [255, 0, 0, 255]);
        assert_eq!(px(&f, 35, 10)[3], 0);
        // the full render is a video (no CSS animation, but moving params) and its frames agree
        let spec = Spec { html: PARAM_PAGE.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 1.0, base: String::new(), params, width: 0, mouse: None };
        if let Some(r) = cached(&spec) {
            let _ = std::fs::remove_file(r.path);
        }
        let r = render_to_file(&spec, &Progress::new()).expect("render to file");
        assert!(r.animated, "keyframed params make a video");
        let mut v = ffpipe::open_video(&r.path.to_string_lossy()).expect("open");
        for (t, left) in [(0.2, 20u32), (0.8, 80)] {
            let mut f = Frame::default();
            assert!(v.frame_at(t, 200, 60, &mut f), "decode {t}");
            assert_eq!(px(&f, left + 5, 10), [255, 0, 0, 255], "box at {left} for t={t}");
            assert_eq!(px(&f, left.saturating_sub(5), 10)[3], 0);
        }
    }

    /// A fake mouse: its position, :hover and the transition it starts are real, stepped frame by frame.
    #[test]
    fn a_fake_mouse_hovers_and_the_transition_it_starts_plays_from_that_moment() {
        if tools_missing() {
            return;
        }
        let page = r#"<!doctype html><html><head><title>m</title><meta name="se:size" content="200x100">
            <style>html,body{margin:0;background:#fff} #b{position:absolute;left:0;top:0;width:80px;height:40px;background:rgb(255,0,0);transition:background-color .5s linear}
            #b:hover{background:rgb(0,0,255)} #b:active{background:rgb(0,255,0)} a{display:block;position:absolute;top:60px;left:0;width:50px;height:20px;background:#888}</style>
            </head><body><div id="b"></div><a href="https://example.com/next">go</a></body></html>"#;
        let px = |f: &Frame, x: u32, y: u32| -> [u8; 4] { f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap() };
        let mut m = crate::model::Mouse::at(300.0, 20.0);
        m.x = track(&[(0.0, 300.0), (1.0, 40.0), (2.0, 40.0)]);
        // 0 = up; pressed from 2.0 s to 2.2 s
        m.down = track(&[(0.0, 0.0), (2.0, 0.0), (2.01, 1.0), (2.2, 1.0), (2.21, 0.0)]);
        let spec = Spec {
            html: page.into(),
            fields: Vec::new(),
            scale: 1.0,
            fps: 20.0,
            duration: 3.0,
            base: String::new(),
            params: Vec::new(),
            width: 0,
            mouse: Some(m.clone()),
        };
        if let Some(r) = cached(&spec) {
            let _ = std::fs::remove_file(r.path);
        }
        let r = render_to_file(&spec, &Progress::new()).expect("render to file");
        assert!(r.animated, "a mouse makes a video");
        let mut v = ffpipe::open_video(&r.path.to_string_lossy()).expect("open");
        let mut at = |t: f64| {
            let mut f = Frame::default();
            assert!(v.frame_at(t, 200, 100, &mut f), "decode {t}");
            f
        };
        let before = px(&at(0.4), 10, 10);
        assert!(before[0] > 240 && before[2] < 20, "the mouse is far away: still red {before:?}");
        let mid = px(&at(1.0), 10, 10);
        assert!(mid[0] > 60 && mid[2] > 60 && mid[1] < 40, "inside the 0.5 s transition (it began when the pointer arrived): {mid:?}");
        let after = px(&at(1.7), 10, 10);
        assert!(after[2] > 240 && after[0] < 20, "hovered: blue {after:?}");
        // pressed at 2.01 s: :active starts a 0.5 s fade from blue to green - 0.09 s in, a fifth of the way
        let pressed = px(&at(2.1), 10, 10);
        assert!(pressed[1] > 10 && pressed[1] < 120 && pressed[2] < 250, "pressed: :active began its fade to green {pressed:?}");
        let later = px(&at(2.19), 10, 10);
        assert!(later[1] > pressed[1], "and it keeps going: {pressed:?} then {later:?}");
        // the cursor is drawn: the white arrow just right of and below its tip at (40, 20)
        let arrow = px(&at(1.7), 44, 30);
        assert!(arrow[0] > 220 && arrow[1] > 220 && arrow[2] > 220, "the arrow {arrow:?}");
        // the link was clicked and went nowhere: the page is still this one
        let grey = |p: [u8; 4]| p[0].abs_diff(136) < 14 && p[1].abs_diff(136) < 14 && p[2].abs_diff(136) < 14;
        let (l1, l2) = (px(&at(1.7), 45, 75), px(&at(2.9), 45, 75));
        assert!(grey(l1) && grey(l2), "the link's grey box is still there: {l1:?} then {l2:?}");
        // a still of a moment (the editor preview): the hover has settled
        let still = render_frame_in(page, &[], 1.0, 1.6, &View { mouse: Some(&m), ..Default::default() }).expect("render");
        let s = px(&still, 10, 10);
        assert!(s[2] > 240 && s[0] < 20, "still at 1.6 s: blue {s:?}");
    }

    /// `se:fullpage`: a page taller than its viewport is captured whole.
    #[test]
    fn a_full_page_capture_is_as_tall_as_the_page() {
        if tools_missing() {
            return;
        }
        let page = r#"<!doctype html><html><head><title>t</title><meta name="se:size" content="200x100"><meta name="se:fullpage" content="auto"></head><body style="margin:0"><div style="height:450px;background:#0a0"></div></body></html>"#;
        let f = render_frame(page, &[], 1.0, 0.0).expect("render");
        assert_eq!((f.width, f.height), (200, 450), "the whole page, not the 100 px viewport");
        let plain = page.replace(r#"<meta name="se:fullpage" content="auto">"#, "");
        assert_eq!(render_frame(&plain, &[], 1.0, 0.0).expect("render").height, 100);
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
        let spec = Spec { html: MOVING.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 2.5, base: String::new(), params: Vec::new(), width: 0, mouse: None };
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
