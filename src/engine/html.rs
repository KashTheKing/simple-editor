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
//! A full render pipes those PNGs into ffmpeg and encodes H.264 (`encode_args`): `<hash>.mp4` when the
//! page is opaque, `<hash>.stacked.mp4` (colour over alpha, as `media::proxy`'s alpha proxies) when it has
//! transparency; a page without animations is one PNG. Output files are content-addressed in
//! `cache_dir()/html/`. (Up to html-v4 renders were PNG-in-MOV, ~100 MB each: `cache_lru` deletes those.)
//!
//! Shutdown: the spike's preview tool killed Edge's main process and waited, which sometimes hung
//! after pages with infinite animations. Killing only the browser process orphans its renderer /
//! GPU children (a renderer busy with an infinite animation does not notice at once), and those
//! children inherit every inheritable handle of ours - our own stdout pipe included, which the
//! calling shell waits on - and keep the profile folder locked. So: `Browser.close` first (Edge
//! closes its whole tree), wait for the browser with a deadline, and only if it is still there kill
//! the TREE by its own pid (`taskkill /PID <pid> /T /F`, never by image name: the user's own Edge is
//! running), then delete the profile with a few retries, never blocking for long.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::model::HtmlRenderer;
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
    /// A font family (installed, or one of the app's own fonts): becomes a CSS font-family stack.
    Font,
    /// One of named options: `a=valueA, b=valueB, c` (the label is stored, the value lands in the page).
    Select,
    /// On / off: `on | value when on | value when off` (default 1 / 0).
    Toggle,
    /// A number with a slider: `default | min | max | step | unit`.
    Range,
    /// A corner radius (a range: 0-200 px unless given).
    Radius,
    /// A size (a range: 0-400 px unless given).
    Size,
    /// A shadow: `size colour` (`20 #00000080`) -> `0 10px 20px #00000080`, or `none` at 0.
    Shadow,
    /// An outline / border: `width colour` (`2 #ffffff`) -> `2px solid #ffffff`, or `none` at 0.
    Outline,
}

impl FieldType {
    fn parse(s: &str) -> Option<FieldType> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "text" => FieldType::Text,
            "color" | "colour" => FieldType::Color,
            "number" => FieldType::Number,
            "image" => FieldType::Image,
            "font" => FieldType::Font,
            "select" | "choice" => FieldType::Select,
            "toggle" | "bool" | "checkbox" => FieldType::Toggle,
            "range" | "slider" => FieldType::Range,
            "radius" => FieldType::Radius,
            "size" => FieldType::Size,
            "shadow" => FieldType::Shadow,
            "outline" | "border" => FieldType::Outline,
            _ => return None,
        })
    }
}

/// One `se:field`: `name | type | default [| more]` - number: `| min | max | unit`; range / radius /
/// size: `| min | max | step | unit`; select: `| a=valueA, b=valueB`; toggle: `| on value | off value`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Field {
    pub name: String,
    pub ty: FieldType,
    pub default: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// What follows the default, as written.
    pub extra: Vec<String>,
}

impl Default for FieldType {
    fn default() -> Self {
        FieldType::Text
    }
}

impl Field {
    /// A number kind (number, range, radius, size): drawn as a number / slider.
    pub fn numeric(&self) -> bool {
        matches!(self.ty, FieldType::Number | FieldType::Range | FieldType::Radius | FieldType::Size)
    }
    /// (min, max) of a number kind, with the defaults of radius / size.
    pub fn bounds(&self) -> (Option<f64>, Option<f64>) {
        match self.ty {
            FieldType::Radius => (self.min.or(Some(0.0)), self.max.or(Some(200.0))),
            FieldType::Size => (self.min.or(Some(0.0)), self.max.or(Some(400.0))),
            _ => (self.min, self.max),
        }
    }
    /// The slider step of a range kind (0 = any).
    pub fn step(&self) -> f64 {
        match self.ty {
            FieldType::Range | FieldType::Radius | FieldType::Size => {
                self.extra.get(2).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| *v > 0.0).unwrap_or(0.0)
            }
            _ => 0.0,
        }
    }
    /// The CSS unit a number kind's value gets (`px` for radius / size unless given; none for number).
    pub fn unit(&self) -> String {
        let at = if self.ty == FieldType::Number { 2 } else { 3 };
        let u = self.extra.get(at).map(|u| u.trim().to_string()).unwrap_or_default();
        let u: String = u.chars().filter(|c| c.is_ascii_alphabetic() || *c == '%').take(4).collect();
        if u.is_empty() && matches!(self.ty, FieldType::Radius | FieldType::Size) {
            "px".into()
        } else {
            u
        }
    }
    /// (label, value) of a select.
    pub fn options(&self) -> Vec<(String, String)> {
        let list = self.extra.first().map(String::as_str).unwrap_or("");
        list.split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(|o| match o.split_once('=') {
                Some((l, v)) => (l.trim().to_string(), v.trim().to_string()),
                None => (o.to_string(), o.to_string()),
            })
            .collect()
    }
}

/// Is `v` on (a toggle's value)?
pub fn toggle_on(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true" | "yes")
}

/// A value that may go into CSS as it is: no characters that end a declaration, a rule or a tag.
fn css_safe(v: &str) -> String {
    v.chars().filter(|c| !"{};<>\\\"'`".contains(*c) && !c.is_control()).collect::<String>().trim().to_string()
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
    /// Which program draws it (the layer's Renderer setting).
    pub renderer: HtmlRenderer,
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
    open_tags_in(src, &src.to_ascii_lowercase(), name)
}

/// `open_tags` with `src`'s lower-case copy already made.
fn open_tags_in<'a>(src: &'a str, low: &str, name: &str) -> Vec<&'a str> {
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

/// Every `content` of `<meta name="<name>">`, in order (a template's own data, like `se:thread`).
pub fn meta_values(src: &str, name: &str) -> Vec<String> {
    metas(src, name)
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
    let ty = FieldType::parse(p.get(1)?)?;
    let numeric = matches!(ty, FieldType::Number | FieldType::Range | FieldType::Radius | FieldType::Size);
    Some(Field {
        name,
        ty,
        default: p.get(2).copied().unwrap_or("").to_string(),
        min: if numeric { num(3) } else { None },
        max: if numeric { num(4) } else { None },
        extra: p.iter().skip(3).map(|v| v.to_string()).collect(),
    })
}

/// Read a template's head. Never fails: what is missing gets its default (`lint` reports it).
/// Remembered per source: the HTML layer scan asks for every layer every 2 s, and a saved web page is
/// ~1 MB that `metas` would lower-case once per tag it looks for.
pub fn parse(src: &str) -> Template {
    static MEMO: Mutex<Option<std::collections::HashMap<u64, Template>>> = Mutex::new(None);
    if src.len() < 16 * 1024 {
        return parse_uncached(src); // cheaper than the hash
    }
    let key = blitz::source_key(src);
    if let Some(t) = lock(&MEMO).as_ref().and_then(|m| m.get(&key)) {
        return t.clone();
    }
    let t = parse_uncached(src);
    let mut g = lock(&MEMO);
    let m = g.get_or_insert_with(Default::default);
    if m.len() > 64 {
        m.clear();
    }
    m.insert(key, t.clone());
    t
}

fn parse_uncached(src: &str) -> Template {
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
            .filter(Field::numeric)
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

/// The page's declared params at second `t` (for the live preview).
pub fn param_values(src: &str, params: &[(String, crate::model::Animated)], t: f64) -> Vec<(String, f64)> {
    values_at(&parse(src), params, t)
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
    // one lower-case copy for all eight tag kinds (a saved web page is ~1 MB, mostly `data:` images)
    let low = text.to_ascii_lowercase();
    for (tag, names) in [("link", &["href"][..]), ("img", &["src"]), ("source", &["src"]), ("video", &["src", "poster"]), ("audio", &["src"]), ("track", &["src"]), ("image", &["href", "xlink:href"]), ("use", &["href", "xlink:href"])] {
        for t in open_tags_in(text, &low, tag) {
            for a in attrs(t) {
                if names.contains(&a.name.as_str())
                    && !a.value.trim_start().get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:"))
                {
                    v.push(unescape(a.value));
                }
            }
        }
    }
    v
}

/// The local files a page needs (one level of stylesheets inside stylesheets and up to 3 deep), resolved
/// against `base` (the folder the page lives in; empty = only absolute references, such as the project
/// media `prepare_in` resolved). Only files that exist are listed.
pub fn local_deps(page: &str, base: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut work: Vec<(String, PathBuf, usize)> = vec![(page.to_string(), PathBuf::from(base), 0)];
    while let Some((text, dir, depth)) = work.pop() {
        for r in refs_in(&text) {
            let Some(p) = resolve_ref(&r, &dir) else { continue };
            if p.is_relative() || out.contains(&p) || !p.is_file() {
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
    let mut files = local_deps(page, base);
    files.sort();
    stamp_of(&files)
}

/// `deps_stamp` of an already listed (sorted) set of files: a stat each.
fn stamp_of(files: &[PathBuf]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
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

// ---------------------------------------------------------------------------------------------------
// project media: `<img src="image.png">` / `url(bg.jpg)` find the project's own files by name

/// What a page may use by name: the project's folder and its media assets (oldest first). Set by the
/// app whenever it looks at its HTML layers (`html_ctl::reconcile`).
/// ponytail: one global for the one open project; a parameter if two projects ever render at once.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Media {
    pub dir: Option<PathBuf>,
    /// Asset files in import order (the last is the most recent).
    pub assets: Vec<PathBuf>,
    /// The files directly inside `dir` (listed when set).
    pub dir_files: Vec<PathBuf>,
}

static MEDIA: std::sync::RwLock<Option<Media>> = std::sync::RwLock::new(None);

impl Media {
    /// `dir` and `assets`, with the folder listed.
    pub fn new(dir: Option<PathBuf>, assets: Vec<PathBuf>) -> Media {
        let dir_files = dir
            .as_ref()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
            .unwrap_or_default();
        Media { dir, assets, dir_files }
    }
}

pub fn set_media(m: Media) {
    let mut g = MEDIA.write().unwrap_or_else(|e| e.into_inner());
    if g.as_ref() != Some(&m) {
        *g = Some(m);
        PREP_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn media() -> Media {
    MEDIA.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// One reference of a page to project media.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaRef {
    Found { name: String, path: PathBuf, warning: Option<String> },
    Missing(String),
}

/// What `r` (as written in the page) is in the project: a path under the project folder, then a file
/// name (any case) among the assets and the folder's files, then the same name without its extension if
/// only one file has it. Several matches: the one in the project folder, else the newest asset, with a
/// warning. None for a reference that is no file name (a web address, `data:`, a field, an anchor).
fn find_media(r: &str, m: &Media) -> Option<MediaRef> {
    let raw = r.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    let low = raw.to_ascii_lowercase();
    if raw.is_empty()
        || raw.starts_with('#')
        || raw.contains("{{")
        || low.contains(':') // http:, data:, file:, C:\ ... - an address, not a name
        || raw.starts_with("//")
        || raw.starts_with('/')
        || raw.starts_with('\\')
    {
        return None;
    }
    let name = percent_decode(raw.split(['?', '#']).next().unwrap_or(""));
    if name.is_empty() || name.split(['/', '\\']).any(|c| c == "..") {
        return Some(MediaRef::Missing(raw.to_string()));
    }
    if let Some(d) = &m.dir {
        let p = d.join(&name);
        if p.is_file() {
            return Some(MediaRef::Found { name: raw.to_string(), path: p, warning: None });
        }
    }
    let file = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();
    let in_dir = |p: &PathBuf| m.dir_files.contains(p);
    // the folder's files, then the assets newest first
    let mut pool: Vec<&PathBuf> = m.dir_files.iter().chain(m.assets.iter().rev()).collect();
    pool.dedup_by(|a, b| a == b);
    let fname = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = |p: &PathBuf| p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut hits: Vec<&PathBuf> = pool.iter().copied().filter(|p| fname(p).eq_ignore_ascii_case(&file)).collect();
    // "logo" (no extension) is logo.png when only one file is called logo
    if hits.is_empty() && Path::new(&file).extension().is_none() {
        let want = Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        hits = pool.iter().copied().filter(|p| stem(p).eq_ignore_ascii_case(&want)).collect();
        let mut uniq = hits.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() != 1 {
            return Some(MediaRef::Missing(raw.to_string()));
        }
    }
    if hits.is_empty() {
        return Some(MediaRef::Missing(raw.to_string()));
    }
    let mut uniq = hits.clone();
    uniq.sort();
    uniq.dedup();
    let pick = hits.iter().copied().find(|p| in_dir(p)).or_else(|| hits.first().copied())?.clone();
    let warning = (uniq.len() > 1).then(|| {
        format!("\"{raw}\" matches {} files - using {} (rename one, or write the path)", uniq.len(), pick.display())
    });
    Some(MediaRef::Found { name: raw.to_string(), path: pick, warning })
}

/// Every reference of `page` to project media (resolved or missing), in order, each once. References
/// that are files next to a linked page (`base`) are the page's own and are left to it.
pub fn media_refs(page: &str, base: &str) -> Vec<MediaRef> {
    media_refs_in(page, base, &media())
}

fn media_refs_in(page: &str, base: &str, m: &Media) -> Vec<MediaRef> {
    let mut out: Vec<MediaRef> = Vec::new();
    for r in refs_in(page) {
        if !base.is_empty() && resolve_ref(&r, Path::new(base)).is_some_and(|p| p.is_file()) {
            continue;
        }
        if let Some(x) = find_media(&r, m) {
            let key = |x: &MediaRef| match x {
                MediaRef::Found { name, .. } | MediaRef::Missing(name) => name.clone(),
            };
            if !out.iter().any(|o| key(o) == key(&x)) {
                out.push(x);
            }
        }
    }
    out
}

/// Point every reference that names project media at its file (`file:///…`, quoted as it was written).
fn with_media(page: String, base: &str) -> String {
    with_media_in(page, base, &media())
}

fn with_media_in(page: String, base: &str, m: &Media) -> String {
    let mut page = page;
    for r in media_refs_in(&page, base, m) {
        let MediaRef::Found { name, path, .. } = r else { continue };
        let url = file_url(&path);
        for (a, b) in [("\"", "\""), ("'", "'"), ("(", ")")] {
            page = page.replace(&format!("{a}{name}{b}"), &format!("{a}{url}{b}"));
        }
        page = page.replace(&format!("( {name} )"), &format!("({url})"));
    }
    page
}

/// Warnings about the page's media for the editor and agents: "missing: x.png", ambiguous names.
pub fn media_warnings(src: &str, values: &[(String, String)], base: &str) -> Vec<String> {
    let tpl = parse(src);
    let page = sanitize(&substitute(src, &tpl, values));
    media_refs(&page, base)
        .into_iter()
        .filter_map(|r| match r {
            MediaRef::Missing(n) => Some(format!("missing: {n} (no file of that name in the project or its folder)")),
            MediaRef::Found { warning, .. } => warning,
        })
        .collect()
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
        FieldType::Number | FieldType::Range | FieldType::Radius | FieldType::Size => {
            let n = [raw, f.default.trim()].iter().find_map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite()));
            let mut n = n.unwrap_or(0.0);
            let (lo, hi) = f.bounds();
            if let Some(lo) = lo {
                n = n.max(lo);
            }
            if let Some(hi) = hi {
                n = n.min(hi);
            }
            format!("{}{}", num_text(n), f.unit())
        }
        FieldType::Font => font_stack(&[raw, f.default.trim()].into_iter().map(css_safe).find(|v| !v.is_empty()).unwrap_or_default()),
        FieldType::Select => {
            let opts = f.options();
            let pick = |v: &str| opts.iter().find(|(l, _)| l.eq_ignore_ascii_case(v.trim())).map(|(_, v)| css_safe(v));
            pick(raw).or_else(|| pick(&f.default)).or_else(|| opts.first().map(|(_, v)| css_safe(v))).unwrap_or_default()
        }
        FieldType::Toggle => {
            let on = if raw.is_empty() { toggle_on(&f.default) } else { toggle_on(raw) };
            let v = f.extra.get(if on { 0 } else { 1 }).map(String::as_str).unwrap_or(if on { "1" } else { "0" });
            css_safe(v)
        }
        FieldType::Shadow | FieldType::Outline => {
            let (size, colour) = size_colour(raw).or_else(|| size_colour(&f.default)).unwrap_or((0.0, "transparent".into()));
            match (f.ty, size > 0.0) {
                (_, false) => "none".into(),
                (FieldType::Shadow, true) => format!("0 {}px {}px {colour}", num_text(size / 2.0), num_text(size)),
                _ => format!("{}px solid {colour}", num_text(size)),
            }
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

/// `"size colour"` (`20 #00000080`) of a shadow / outline field.
pub fn size_colour(v: &str) -> Option<(f64, String)> {
    let mut it = v.split_whitespace();
    let size = it.next()?.trim_end_matches("px").parse::<f64>().ok().filter(|v| v.is_finite())?.clamp(0.0, 500.0);
    let colour = it.next().filter(|c| valid_color(c)).unwrap_or("#000000").to_string();
    Some((size, colour))
}

/// The app's own fonts (Settings ▸ user fonts): (family, file). A font field naming one gets an
/// @font-face pointing at its file, so Edge draws it even though Windows does not have it installed.
static APP_FONTS: std::sync::RwLock<Vec<(String, PathBuf)>> = std::sync::RwLock::new(Vec::new());

pub fn set_app_fonts(fonts: Vec<(String, PathBuf)>) {
    *APP_FONTS.write().unwrap_or_else(|e| e.into_inner()) = fonts;
    PREP_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// A family as a CSS font-family stack with fallbacks that exist on every Windows.
/// Single quotes, so it also fits a double-quoted `style="…"` attribute.
fn font_stack(family: &str) -> String {
    let family = family.trim().trim_matches(|c| c == '"' || c == '\'');
    if family.is_empty() {
        return "'Segoe UI', Arial, sans-serif".into();
    }
    format!("'{family}', 'Segoe UI', Arial, sans-serif")
}

/// `<style>@font-face …</style>` after `<head>` for every app font a font field of `tpl` uses.
fn with_app_fonts(page: String, tpl: &Template, values: &[(String, String)]) -> String {
    let fonts = APP_FONTS.read().unwrap_or_else(|e| e.into_inner());
    if fonts.is_empty() {
        return page;
    }
    let mut css = String::new();
    for f in tpl.fields.iter().filter(|f| f.ty == FieldType::Font) {
        let raw = values.iter().rev().find(|(n, _)| *n == f.name).map_or(f.default.as_str(), |(_, v)| v.as_str());
        let fam = css_safe(raw);
        if let Some((name, file)) = fonts.iter().find(|(n, _)| n.eq_ignore_ascii_case(&fam)) {
            css.push_str(&format!("@font-face{{font-family:\"{name}\";src:url(\"{}\")}}", file_url(file)));
        }
    }
    if css.is_empty() {
        return page;
    }
    let tag = format!("<style>{css}</style>");
    let low = page.to_ascii_lowercase();
    match low.find("<head").filter(|&i| low.as_bytes().get(i + 5).is_some_and(|c| c.is_ascii_whitespace() || *c == b'>')) {
        Some(i) => {
            let at = tag_end(page.as_bytes(), i);
            format!("{}{}{}", &page[..at], tag, &page[at..])
        }
        None => format!("{tag}{page}"),
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
            e.push(format!(
                "se:field `{raw}` is not `name | type | default` (type: text, color, number, image, font, select, toggle, range, radius, size, shadow, outline)"
            ));
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
            FieldType::Number | FieldType::Range | FieldType::Radius | FieldType::Size => {
                if f.default.trim().parse::<f64>().is_err() {
                    e.push(format!("field `{}`: default `{}` is not a number", f.name, f.default));
                }
                if let (Some(a), Some(b)) = f.bounds() {
                    if a > b {
                        e.push(format!("field `{}`: min is above max", f.name));
                    }
                }
            }
            FieldType::Select => {
                let opts = f.options();
                if opts.is_empty() {
                    e.push(format!("field `{}`: a select needs options: `name | select | default | a=value, b=value`", f.name));
                } else if !opts.iter().any(|(l, _)| l.eq_ignore_ascii_case(f.default.trim())) {
                    e.push(format!("field `{}`: default `{}` is not one of its options", f.name, f.default));
                }
            }
            FieldType::Toggle if !matches!(f.default.trim().to_ascii_lowercase().as_str(), "on" | "off" | "1" | "0" | "true" | "false" | "yes" | "no") => {
                e.push(format!("field `{}`: a toggle's default is on or off", f.name))
            }
            FieldType::Shadow | FieldType::Outline if size_colour(&f.default).is_none() => {
                e.push(format!("field `{}`: default `{}` is not `size #colour` (e.g. `20 #00000080`)", f.name, f.default))
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
    let p = prepared(src, values, base, blitz::source_key(src));
    (p.tpl.clone(), p.page.clone())
}

/// A prepared page, and the local files it links to (listed again at most every `RELIST`: a file the
/// page names that appears later is still noticed, without a scan of the page every 2 s).
struct Prepared {
    tpl: Template,
    page: String,
    deps: Mutex<Option<(std::time::Instant, Vec<PathBuf>)>>,
}

const RELIST: std::time::Duration = std::time::Duration::from_secs(10);

impl Prepared {
    /// `deps_stamp(page, base)`, from the remembered list while it is fresh and unchanged.
    fn deps_stamp(&self, base: &str) -> u64 {
        let mut d = lock(&self.deps);
        if let Some((_, files)) = d.as_ref().filter(|(at, _)| at.elapsed() < RELIST) {
            return stamp_of(files);
        }
        let mut files = local_deps(&self.page, base);
        files.sort();
        let stamp = stamp_of(&files);
        *d = Some((std::time::Instant::now(), files));
        stamp
    }
}

/// `prepare_in`, remembered: it is a pure function of the source, the values, the base and the two
/// globals it reads (project media, app fonts - `PREP_GEN` counts their changes), and the HTML layer
/// scan asks for every layer every 2 s plus once per still a render writes. Uncached it lower-cases and
/// copies a ~1 MB web page a few dozen times: seconds of UI thread per scan on a project with six of them.
/// ponytail: cleared when it holds 32 pages; an LRU if projects ever hold more web pages than that.
fn prepared(src: &str, values: &[(String, String)], base: &str, src_key: u64) -> std::sync::Arc<Prepared> {
    use std::hash::{Hash, Hasher};
    static MEMO: Mutex<Option<std::collections::HashMap<u64, std::sync::Arc<Prepared>>>> = Mutex::new(None);
    let mut h = std::hash::DefaultHasher::new();
    (src_key, values, base, PREP_GEN.load(std::sync::atomic::Ordering::Relaxed)).hash(&mut h);
    let key = h.finish();
    if let Some(p) = lock(&MEMO).as_ref().and_then(|m| m.get(&key)) {
        return p.clone();
    }
    let (tpl, page) = prepare_uncached(src, values, base);
    let p = std::sync::Arc::new(Prepared { tpl, page, deps: Mutex::new(None) });
    let mut g = lock(&MEMO);
    let m = g.get_or_insert_with(Default::default);
    if m.len() > 32 {
        m.clear();
    }
    m.insert(key, p.clone());
    p
}

#[cfg(test)]
thread_local!(static PREPARES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });

/// Bumped whenever `set_media` / `set_app_fonts` change what `prepare_in` would produce.
static PREP_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn prepare_uncached(src: &str, values: &[(String, String)], base: &str) -> (Template, String) {
    #[cfg(test)]
    PREPARES.with(|n| n.set(n.get() + 1));
    let tpl = parse(src);
    let page = with_base(with_media(sanitize(&substitute(src, &tpl, values)), base), base);
    let page = with_app_fonts(page, &tpl, values);
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
        let mut w = crate::mcp::clone_socket(&s).map_err(|e| e.to_string())?;
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
    /// The page loaded now, as `load` keys it.
    page: Option<u64>,
    /// `<video>` elements in it (`timing`): `seek` moves them too.
    videos: u64,
    /// One of them did not get to its frame in time: not waited for again on this page.
    video_stuck: bool,
    /// The browser process's own pid, from DevTools: not `child` when Edge started itself again.
    browser_pid: Option<u32>,
}

static EDGE: Mutex<Option<Edge>> = Mutex::new(None);

/// Short renders (an interim still, `html.check`) waiting for the Edge. A long render steps aside
/// between two frames while this is above 0, so a layer just added shows in a moment even while
/// another layer's whole animation is rendering.
static SHORT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn short_waiting() -> bool {
    SHORT.load(std::sync::atomic::Ordering::SeqCst) > 0
}

/// Interim stills written so far (`html_ctl` re-points the asset when this moves).
static STILLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn stills_written() -> u64 {
    STILLS.load(std::sync::atomic::Ordering::SeqCst)
}

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

/// Put `child` (and so every process it starts: Edge's renderers, GPU process) in a job of ours that
/// Windows closes when this process ends, however it ends - a crash, a killed test run - and closing it
/// kills them all. Before this, every test run or crash left a headless Edge tree behind (~0.3-0.9 GB
/// each). Best effort: without the job, `close` / `shutdown` still do the normal clean-up.
pub(crate) fn tie_to_us(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::*;
    static JOB: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, size).ok()?;
        Some(job.0 as usize) // never closed: Windows closes it when we exit
    });
    if let Some(job) = *job {
        let _ = unsafe { AssignProcessToJobObject(HANDLE(job as _), HANDLE(child.as_raw_handle())) };
    }
}

/// Kill the process tree of `pid` (an Edge browser that is not our direct child), never by image name.
fn kill_pid_tree(pid: u32) {
    let _ = ffpipe::command(Path::new("taskkill"))
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
        let data = capture_profile(network).unwrap_or_else(|| profile.join("p")); // ws:browser-layer
        let mut child = crate::media::bg::command(&exe)
            .args(["--headless", "--remote-debugging-port=0", "--no-first-run", "--no-default-browser-check"])
            .args(["--hide-scrollbars", "--mute-audio", "--disable-extensions", "--disable-background-networking"])
            .args(["--disable-sync", "--disable-component-update"])
            .args(if network { vec![] } else { vec!["--host-resolver-rules=MAP * ~NOTFOUND"] })
            .arg(format!("--user-data-dir={}", data.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Microsoft Edge: {e}"))?;
        tie_to_us(&child);
        match Self::connect(&mut child, &data) {
            Ok(mut cdp) => {
                // which process is the browser (Edge may have started itself again)
                let sess = cdp.session.take(); // a browser-level call
                let info = cdp.call("SystemInfo.getProcessInfo", json!({})).unwrap_or_default();
                cdp.session = sess;
                let browser_pid = info["processInfo"]
                    .as_array()
                    .and_then(|a| a.iter().find(|p| p["type"] == "browser"))
                    .and_then(|p| p["id"].as_u64())
                    .map(|p| p as u32);
                Ok(Edge { child, profile, cdp, used: Instant::now(), pages: 0, page: None, videos: 0, video_stuck: false, browser_pid })
            }
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
            // Edge 157 can start itself again and let the first process go (exit code 0): its browser
            // then writes the port file as usual (it stays in our job). Fail only on an error exit, or a
            // clean exit after which no browser shows up within a few seconds.
            match child.try_wait() {
                Ok(Some(st)) if !st.success() => return Err(format!("Microsoft Edge exited at start ({st})")),
                Ok(Some(_)) if t0.elapsed() > Duration::from_secs(8) => return Err("Microsoft Edge exited at start".into()),
                _ => {}
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
        let relaunched = !matches!(self.child.try_wait(), Ok(None));
        let _ = self.cdp.call("Browser.close", json!({})); // Edge may drop the socket before it answers
        let _ = self.cdp.ws.w.shutdown(std::net::Shutdown::Both);
        if relaunched {
            // the browser is another process than `child`: give Browser.close a moment, then make sure
            if let Some(pid) = self.browser_pid {
                std::thread::sleep(Duration::from_millis(500));
                kill_pid_tree(pid);
            }
        } else if !wait_for(&mut self.child, Duration::from_secs(3)) {
            kill_tree(&mut self.child);
            wait_for(&mut self.child, Duration::from_secs(2));
        }
        remove_profile(&self.profile);
    }

    /// Load `page` at `size` CSS pixels, `scale` device pixels each; wait for its fonts.
    /// Skipped when that exact page is still loaded (a render resumed after a short one stepped in, an
    /// `html.check` of another moment): every render sets all its params and seeks every frame anyway.
    /// A page with a `<base>` (linked files that may have changed) is always loaded again.
    fn load(&mut self, page: &str, size: (u32, u32), scale: f64) -> Result<(), String> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (page, size, scale.to_bits()).hash(&mut h);
        let key = h.finish();
        if self.page == Some(key) {
            return Ok(());
        }
        self.page = None;
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
        self.videos = self.cdp.eval("document.querySelectorAll('video').length")?.as_u64().unwrap_or(0);
        self.video_stuck = false;
        if !page.contains("<base ") {
            self.page = Some(key);
        }
        Ok(())
    }

    /// Every animation paused at `ms`.
    /// Every <video> too: muted, paused and moved to `ms` (modulo its length when it loops), then this
    /// waits until each has that frame decoded (polled from here: the page runs no script of its own, so
    /// its `seeked` events reach no listener), at most 3 s. A page without a video pays nothing extra.
    fn seek(&mut self, ms: f64) -> Result<(), String> {
        self.cdp.eval(&format!("for(const a of document.getAnimations()){{a.pause();a.currentTime={ms}}}"))?;
        if self.videos == 0 {
            return Ok(());
        }
        let js = format!(
            "[...document.querySelectorAll('video')].every(v=>{{v.muted=true;v.autoplay=false;if(!v.paused)v.pause();\
             if(v.error||v.networkState===3)return true;if(v.readyState<1)return false;let t={ms}/1000;const d=v.duration;\
             if(d>0&&isFinite(d))t=v.loop?t%d:Math.min(t,Math.max(0,d-0.001));\
             if(Math.abs(v.currentTime-t)>0.0005&&!v.seeking){{v.currentTime=t;return false}}\
             return !v.seeking&&v.readyState>=2}})"
        );
        // a video that never gets ready (a codec Edge lacks) is waited for once per page, not every frame
        let until = Instant::now() + Duration::from_secs(if self.video_stuck { 0 } else { 3 });
        loop {
            if self.cdp.eval(&js)? == json!(true) {
                return Ok(());
            }
            if Instant::now() >= until {
                self.video_stuck = true;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    /// (animation count, each animation's active phase [start, end] in ms). Outside every active phase
    /// nothing moves: a frame there is the picture of the frame before (`moves_between`).
    fn timing(&mut self) -> Result<(u64, Vec<(f64, f64)>), String> {
        let v = self.cdp.eval(
            "(()=>{const iv=[];let n=0;for(const a of document.getAnimations()){n++;\
             const t=a.effect?a.effect.getComputedTiming():null;if(!t){iv.push([0,1e15]);continue}\
             const s=t.delay||0;iv.push([s,Math.min(s+t.activeDuration,1e15)])}\
             const nv=document.querySelectorAll('video').length;for(let i=0;i<nv;i++){n++;iv.push([0,1e15])}\
             return [n,iv,nv]})()",
        )?;
        self.videos = v[2].as_u64().unwrap_or(0);
        let iv = v[1].as_array().map_or(Vec::new(), |a| {
            a.iter().map(|p| (p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(1e15))).collect()
        });
        Ok((v[0].as_u64().unwrap_or(0), iv))
    }

    /// Make the page safe to click (links and forms go nowhere) and put a cursor on it.
    fn mouse_setup(&mut self) -> Result<(), String> {
        self.page = None; // the page is changed (cursor, hover state): the next render loads it fresh
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
/// `short`: a quick render that goes ahead of a long one (see `SHORT`); a long one waits while any is.
/// ponytail: one Edge, one tab, one render at a time - two windows render a 1080p animation ~1.45x
/// faster (measured), a window per job if that ever matters more than the code.
fn with_edge<T>(short: bool, mut f: impl FnMut(&mut Edge) -> Result<T, String>) -> Result<T, String> {
    use std::sync::atomic::Ordering::SeqCst;
    let mut g = if short {
        SHORT.fetch_add(1, SeqCst);
        let g = lock(&EDGE);
        SHORT.fetch_sub(1, SeqCst);
        g
    } else {
        while short_waiting() {
            std::thread::sleep(Duration::from_millis(2));
        }
        lock(&EDGE)
    };
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
            let y=0;while(y<h()&&y<24000){window.scrollTo(0,y);await new Promise(r=>setTimeout(r,220));y+=innerHeight*0.8}\
            window.scrollTo(0,0);await new Promise(r=>setTimeout(r,700));\
            /* ws:browser-layer: until the pictures are in (placeholders gone), or nothing changes for 3 s, at most 20 s */\
            const pend=()=>[...document.images].filter(i=>!i.complete||!i.naturalWidth).length+document.querySelectorAll('[class*=shimmer],[class*=skeleton]').length;\
            for(let t0=Date.now(),last=-1,same=0;Date.now()-t0<20000&&pend()>0&&same<12;){const p=pend();same=p===last?same+1:0;last=p;await new Promise(r=>setTimeout(r,250))}try{await document.fonts.ready}catch(e){}return h()})()";
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

/// Start the shared Edge in the background if it is not running (a hint that an HTML layer is coming:
/// the Gallery's HTML / Icons tab is open). At most once per `IDLE`, so an open tab does not keep
/// restarting an Edge that idled out.
pub fn prewarm() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = lock(&LAST);
    // with the fast renderer installed Edge is the exception (a layer it cannot draw): started when needed, not here
    if last.is_some_and(|t| t.elapsed() < IDLE) || edge_exe().is_none() || blitz::available() {
        return;
    }
    *last = Some(Instant::now());
    let _ = std::thread::Builder::new().name("edge-prewarm".into()).spawn(|| {
        let _ = with_edge(false, |_| Ok(()));
    });
}

// ---------------------------------------------------------------------------------------------------
// the live preview: the page running in real time in a tab of its own, for editing (not the timeline)

/// What a live preview shows: `page` (prepared: fields in, sanitised) at `size` CSS px x `scale`,
/// params as CSS variables, held at second `at`, or (`playing`) running in real time from `at`, looping
/// over `duration` when `looping`. `epoch` changes when the user presses play / seeks, so the same
/// `at` can be started again. Held, the picture is the moment the frame-exact renderer gives.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveReq {
    pub page: String,
    pub size: (u32, u32),
    pub scale: f64,
    pub params: Vec<(String, f64)>,
    pub at: f64,
    pub playing: bool,
    pub looping: bool,
    pub epoch: u64,
    pub duration: f64,
    /// Drawn by the fast renderer (`engine::blitz`) instead of an Edge tab, decided by the caller.
    pub fast: bool,
    /// The folder of the page's file, and a key of its source (for remembering a failure) - fast only.
    pub base: String,
    pub src_key: u64,
    /// The fake mouse's position and button at `at`, when the layer has one (fast only: hover / active).
    pub ptr: Option<(f64, f64, bool)>,
}

/// Live previews that can be on screen at once, each with a tab of its own (one shared tab made two
/// previews with different requests fight: every frame swapped the page between them - the blink).
pub const LIVE_INSPECTOR: usize = 0;
pub const LIVE_EDITOR: usize = 1;
const LIVE_SLOTS: usize = 2;

#[derive(Default)]
struct LiveState {
    req: Option<LiveReq>,
    /// Bumped with every new `req`.
    gen: u64,
    seen: Option<Instant>,
    /// The latest picture: (the `gen` it shows, the frame).
    frame: Option<(u64, std::sync::Arc<Frame>)>,
    wake: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    thread: bool,
    tx: Option<std::sync::mpsc::Sender<LiveEv>>,
}

const LIVE_EMPTY: LiveState =
    LiveState { req: None, gen: 0, seen: None, frame: None, wake: None, thread: false, tx: None };
static LIVE: Mutex<[LiveState; LIVE_SLOTS]> = Mutex::new([LIVE_EMPTY; LIVE_SLOTS]);

enum LiveEv {
    /// A new request in `LIVE`.
    Req,
    /// A DevTools message from the socket.
    Msg(Value),
    Closed,
}

/// Show `req` live; call it every frame the preview is drawn (a preview not drawn for a second stops
/// streaming, after a minute its tab closes). `wake` is called when a new picture is in. Never blocks.
/// Returns the latest picture and whether it already shows `req`.
pub fn live_show(slot: usize, req: LiveReq, wake: impl Fn() + Send + Sync + 'static) -> Option<(std::sync::Arc<Frame>, bool)> {
    let mut all = lock(&LIVE);
    let g = &mut all[slot.min(LIVE_SLOTS - 1)];
    g.seen = Some(Instant::now());
    if g.req.as_ref() != Some(&req) {
        g.req = Some(req);
        g.gen += 1;
        if let Some(tx) = &g.tx {
            let _ = tx.send(LiveEv::Req);
        }
    }
    let fast = g.req.as_ref().is_some_and(|r| r.fast);
    if !g.thread && !cfg!(test) && (fast || edge_exe().is_some()) {
        // ponytail: no live Edge in unit tests (the idle-repaint tests draw the editor)
        g.wake = Some(std::sync::Arc::new(wake));
        g.thread = true;
        let slot = slot.min(LIVE_SLOTS - 1);
        let _ = std::thread::Builder::new().name("html-live".into()).spawn(move || live_thread(slot, fast));
    }
    let gen = g.gen;
    g.frame.as_ref().map(|(fg, f)| (f.clone(), *fg == gen))
}

/// One DevTools connection of the live tab: commands from this thread, every message from the socket
/// through the same channel as the requests (a reader thread), so screencast frames are handled while
/// a command waits for its answer.
struct LiveConn {
    w: TcpStream,
    rx: std::sync::mpsc::Receiver<LiveEv>,
    next: u64,
    session: String,
    dirty: bool,
    /// The `gen` of the request the page shows now.
    shown: u64,
    loaded: bool,
    slot: usize,
}

impl LiveConn {
    fn send(&mut self, method: &str, params: Value) -> Result<u64, String> {
        self.next += 1;
        let m = json!({"id": self.next, "method": method, "params": params, "sessionId": self.session});
        self.w.write_all(&encode_frame(1, m.to_string().as_bytes())).map_err(|e| format!("Edge: {e}"))?;
        Ok(self.next)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.send(method, params)?;
        loop {
            match self.rx.recv_timeout(Duration::from_secs(20)) {
                Ok(LiveEv::Msg(v)) if v["id"].as_u64() == Some(id) => {
                    return match v.get("error") {
                        Some(e) => Err(format!("Edge ({method}): {e}")),
                        None => Ok(v["result"].clone()),
                    };
                }
                Ok(LiveEv::Msg(v)) => self.event(v)?,
                Ok(LiveEv::Req) => self.dirty = true,
                Ok(LiveEv::Closed) | Err(_) => return Err(format!("Edge ({method}): the live tab is gone")),
            }
        }
    }

    fn eval(&mut self, js: &str) -> Result<(), String> {
        self.call("Runtime.evaluate", json!({"expression": js})).map(|_| ())
    }

    /// A screencast frame: decode, publish, ack. Anything else: noted (the load event) or ignored.
    fn event(&mut self, v: Value) -> Result<(), String> {
        match v["method"].as_str() {
            Some("Page.screencastFrame") => {
                let p = &v["params"];
                let _ = self.send("Page.screencastFrameAck", json!({"sessionId": p["sessionId"]}));
                let mut png = Vec::new();
                b64_decode(p["data"].as_str().unwrap_or("").as_bytes(), &mut png);
                if let Ok(f) = decode_png(&png) {
                    let wake = {
                        let mut all = lock(&LIVE);
                        let g = &mut all[self.slot];
                        g.frame = Some((self.shown, std::sync::Arc::new(f)));
                        g.wake.clone()
                    };
                    if let Some(w) = wake {
                        w();
                    }
                }
            }
            Some("Page.loadEventFired") => self.loaded = true,
            _ => {}
        }
        Ok(())
    }
}

fn live_thread(slot: usize, fast: bool) {
    let r = if fast { live_run_fast(slot) } else { live_run(slot) };
    let mut all = lock(&LIVE);
    all[slot].thread = false;
    all[slot].tx = None;
    drop(all);
    if let Err(e) = r {
        eprintln!("html live preview: {e}");
    }
}

fn live_run(slot: usize) -> Result<(), String> {
    // the shared Edge (started if needed): its DevTools address, read from its profile
    let (port, path) = with_edge(true, |e| {
        let s = std::fs::read_to_string(e.profile.join("p").join("DevToolsActivePort")).map_err(|e| e.to_string())?;
        let mut l = s.lines();
        let port = l.next().and_then(|p| p.trim().parse::<u16>().ok()).ok_or("no DevTools port")?;
        Ok((port, l.next().unwrap_or("").trim().to_string()))
    })?;
    let ws = Ws::connect(port, &path)?;
    let (tx, rx) = std::sync::mpsc::channel();
    let w = crate::mcp::clone_socket(&ws.w).map_err(|e| e.to_string())?;
    {
        let tx = tx.clone();
        let (mut r, mut pong) = (ws.r, ws.w);
        r.get_ref().set_read_timeout(None).ok();
        std::thread::Builder::new()
            .name("html-live-read".into())
            .spawn(move || loop {
                match read_message(&mut r, &mut pong).ok().and_then(|m| serde_json::from_slice::<Value>(&m).ok()) {
                    Some(v) => {
                        if tx.send(LiveEv::Msg(v)).is_err() {
                            return;
                        }
                    }
                    None => {
                        let _ = tx.send(LiveEv::Closed);
                        return;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    lock(&LIVE)[slot].tx = Some(tx);
    let mut c = LiveConn { w, rx, next: 0, session: String::new(), dirty: true, shown: 0, loaded: false, slot };
    // a window of its own (a background tab never paints), browser-level calls without a session
    c.next += 1;
    let id = c.next;
    let m = json!({"id": id, "method": "Target.createTarget", "params": {"url": "about:blank", "newWindow": true}});
    c.w.write_all(&encode_frame(1, m.to_string().as_bytes())).map_err(|e| e.to_string())?;
    let target = loop {
        match c.rx.recv_timeout(Duration::from_secs(20)) {
            Ok(LiveEv::Msg(v)) if v["id"].as_u64() == Some(id) => break v["result"]["targetId"].clone(),
            Ok(LiveEv::Closed) | Err(_) => return Err("Edge: no live tab".into()),
            _ => {}
        }
    };
    c.next += 1;
    let id = c.next;
    let m = json!({"id": id, "method": "Target.attachToTarget", "params": {"targetId": target, "flatten": true}});
    c.w.write_all(&encode_frame(1, m.to_string().as_bytes())).map_err(|e| e.to_string())?;
    c.session = loop {
        match c.rx.recv_timeout(Duration::from_secs(20)) {
            Ok(LiveEv::Msg(v)) if v["id"].as_u64() == Some(id) => break v["result"]["sessionId"].as_str().unwrap_or("").to_string(),
            Ok(LiveEv::Closed) | Err(_) => return Err("Edge: no live session".into()),
            _ => {}
        }
    };
    c.call("Page.enable", json!({}))?;
    c.call("Emulation.setScriptExecutionDisabled", json!({"value": true}))?;
    c.call("Emulation.setDefaultBackgroundColorOverride", json!({"color": {"r": 0, "g": 0, "b": 0, "a": 0}}))?;
    let file = std::env::temp_dir().join(format!("se-live-{}-{slot}.html", std::process::id()));
    let (mut cur, mut frame_id, mut casting) = (None::<LiveReq>, String::new(), false);
    let mut loop_at: Option<Instant> = None;
    let mut touched = Instant::now();
    let result = (|| -> Result<(), String> {
        loop {
            match c.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(LiveEv::Msg(v)) => c.event(v)?,
                Ok(LiveEv::Req) => c.dirty = true,
                Ok(LiveEv::Closed) => return Err("Edge closed the live tab".into()),
                Err(_) => {}
            }
            let (seen, req, gen) = {
                let all = lock(&LIVE);
                let g = &all[slot];
                (g.seen.map_or(Duration::MAX, |s| s.elapsed()), g.req.clone(), g.gen)
            };
            if seen > Duration::from_secs(60) || req.as_ref().is_some_and(|r| r.fast) {
                return Ok(()); // closed for good, or the layer now uses the fast renderer (a new thread)
            }
            if seen > Duration::from_secs(1) {
                if casting {
                    // not on screen: stop streaming and stop the page itself (a looping animation in a
                    // hidden tab still costs a core); shown again, it is set up afresh
                    c.call("Page.stopScreencast", json!({}))?;
                    c.eval("for(const a of document.getAnimations())a.pause();for(const v of document.querySelectorAll('video'))v.pause()")?;
                    casting = false;
                    loop_at = None;
                    if let Some(r) = cur.as_mut() {
                        r.epoch = r.epoch.wrapping_add(1 << 40); // re-applies the time and play state
                    }
                    c.dirty = true;
                }
                continue;
            }
            if touched.elapsed() > Duration::from_secs(10) {
                touched = Instant::now();
                let _ = with_edge(true, |_| Ok(())); // the shared Edge is in use: not idle
            }
            if c.dirty {
                c.dirty = false;
                if let Some(req) = req {
                    let old = cur.take();
                    let (sized, paged) = (old.as_ref().map(|o| (o.size, o.scale)) != Some((req.size, req.scale)), old.as_ref().map(|o| &o.page) != Some(&req.page));
                    if sized {
                        let m = json!({"width": req.size.0, "height": req.size.1, "deviceScaleFactor": req.scale, "mobile": false});
                        c.call("Emulation.setDeviceMetricsOverride", m)?;
                    }
                    if paged && frame_id.is_empty() {
                        std::fs::write(&file, &req.page).map_err(|e| format!("page: {e}"))?;
                        c.loaded = false;
                        let r = c.call("Page.navigate", json!({"url": file_url(&file)}))?;
                        frame_id = r["frameId"].as_str().unwrap_or("").to_string();
                        let until = Instant::now() + Duration::from_secs(10);
                        while !c.loaded && Instant::now() < until {
                            if let Ok(LiveEv::Msg(v)) = c.rx.recv_timeout(Duration::from_millis(50)) {
                                c.event(v)?;
                            }
                        }
                    } else if paged {
                        // the same tab, a new document: no navigation (a field edit lands in ~10 ms)
                        c.call("Page.setDocumentContent", json!({"frameId": frame_id, "html": req.page}))?;
                    }
                    if paged || old.as_ref().map(|o| &o.params) != Some(&req.params) {
                        let obj: serde_json::Map<String, Value> = req.params.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                        c.eval(&format!(
                            "(()=>{{const s=document.documentElement.style;for(const [k,v] of Object.entries({})){{s.setProperty('--'+k,String(v))}}}})()",
                            Value::Object(obj)
                        ))?;
                    }
                    // time and play state: only when they (or the document) changed, so a param or field
                    // edit while playing does not jump back to the start
                    let timed = old.as_ref().map(|o| (o.at.to_bits(), o.playing, o.epoch))
                        != Some((req.at.to_bits(), req.playing, req.epoch));
                    if paged || timed {
                        let ms = req.at.max(0.0) * 1000.0;
                        let js = if req.playing {
                            "for(const a of document.getAnimations()){a.currentTime=MS;a.play()}\
                             for(const v of document.querySelectorAll('video')){v.muted=true;v.loop=true;try{v.currentTime=MS/1000}catch(e){}v.play().catch(()=>{})}"
                        } else {
                            "for(const a of document.getAnimations()){a.pause();a.currentTime=MS}\
                             for(const v of document.querySelectorAll('video')){v.muted=true;v.pause();try{v.currentTime=MS/1000}catch(e){}}"
                        };
                        c.eval(&js.replace("MS", &format!("{ms}")))?;
                        let left = (req.duration - req.at).max(0.05);
                        loop_at = (req.playing && req.looping).then(|| Instant::now() + Duration::from_secs_f64(left));
                    }
                    c.shown = gen;
                    cur = Some(req);
                    if !casting {
                        c.call("Page.startScreencast", json!({"format": "png", "everyNthFrame": 1}))?;
                        casting = true;
                    }
                }
            } else if !casting && cur.is_some() {
                c.call("Page.startScreencast", json!({"format": "png", "everyNthFrame": 1}))?;
                casting = true;
            }
            if let Some(t) = loop_at.filter(|t| *t <= Instant::now()) {
                // loop: from the start (the UI's clock wraps the same way)
                c.eval("for(const a of document.getAnimations()){a.currentTime=0;a.play()}")?;
                loop_at = Some(t + Duration::from_secs_f64(cur.as_ref().map_or(4.0, |r| r.duration.clamp(0.5, 600.0))));
            }
        }
    })();
    let _ = c.send("Target.closeTarget", json!({"targetId": target}));
    let _ = std::fs::remove_file(&file);
    result
}

/// The live preview of a slot drawn by the fast renderer: one `se-html` process, a page loaded in it, and
/// a frame drawn at the preview's own clock (30 a second while playing, one per change otherwise) and
/// published as raw RGBA (no PNG, no browser tab). A failure is remembered (the layer is Edge's from now
/// on) and ends the thread; the caller's next `live_show` starts the Edge one.
fn live_run_fast(slot: usize) -> Result<(), String> {
    let r = (|| -> Result<(), String> {
        let mut helper: Option<blitz::Helper> = None;
        let mut loaded: Option<(u64, (String, (u32, u32), u64, String))> = None;
        let (mut shown_gen, mut last_t, mut timing, mut started) = (0u64, f64::NAN, None::<(u64, u64, bool)>, (Instant::now(), 0.0f64));
        loop {
            let (seen, req, gen) = {
                let all = lock(&LIVE);
                let g = &all[slot];
                (g.seen.map_or(Duration::MAX, |s| s.elapsed()), g.req.clone(), g.gen)
            };
            if seen > Duration::from_secs(60) {
                return Ok(());
            }
            let Some(req) = req else {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            };
            if !req.fast {
                return Ok(());
            }
            if seen > Duration::from_secs(1) {
                // not on screen: nothing is drawn; shown again, the clock starts afresh
                timing = None;
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if helper.is_none() {
                helper = Some(blitz::Helper::spawn()?);
            }
            let h = helper.as_mut().unwrap();
            let key = (req.page.clone(), req.size, req.scale.to_bits(), req.base.clone());
            if loaded.as_ref().map(|l| &l.1) != Some(&key) {
                if let Some((id, _)) = loaded.take() {
                    h.free(id);
                }
                let info = h.load(&req.page, &req.base, req.size, req.scale)?;
                loaded = Some((info.id, key));
                last_t = f64::NAN;
            }
            let id = loaded.as_ref().unwrap().0;
            let d = req.duration.max(0.05);
            let tk = (req.at.to_bits(), req.epoch, req.playing);
            if timing != Some(tk) {
                timing = Some(tk);
                started = (Instant::now(), req.at);
            }
            let t = if req.playing {
                let x = started.1 + started.0.elapsed().as_secs_f64();
                if req.looping { x % d } else { x.min(d) }
            } else {
                req.at
            };
            if req.playing || gen != shown_gen || t != last_t {
                let t0 = Instant::now();
                let pic = h.frame(id, t, &req.params, req.ptr, false)?;
                let f = Frame { width: pic.w, height: pic.h, pts: 0.0, rgba: pic.bytes, bgrx: false, gpu: None };
                let wake = {
                    let mut all = lock(&LIVE);
                    let g = &mut all[slot];
                    g.frame = Some((gen, std::sync::Arc::new(f)));
                    g.wake.clone()
                };
                if let Some(w) = wake {
                    w();
                }
                shown_gen = gen;
                last_t = t;
                if req.playing {
                    std::thread::sleep(Duration::from_millis(33).saturating_sub(t0.elapsed()));
                    continue;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    })();
    if let Err(e) = &r {
        let key = lock(&LIVE)[slot].req.as_ref().map(|q| q.src_key);
        if let Some(k) = key {
            blitz::remember_failure_key(k, e);
        }
    }
    r
}

/// Close the shared Edge and the fast renderer now (app exit). Safe to call any time; a later render
/// starts a new one.
pub fn shutdown() {
    blitz::shutdown();
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
    if let Some(&size) = lock(&PNG_SIZES).get(path) {
        return Some(size);
    }
    let mut head = [0u8; 32];
    let mut f = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut f, &mut head).ok()?;
    let size = png_size(&head)?;
    lock(&PNG_SIZES).insert(path.to_path_buf(), size);
    Some(size)
}

/// Sizes of the cache's PNGs, remembered: their names are content hashes (a path always holds the same
/// picture), and the HTML layer scan asks for them every 2 s and after every still. Opening a freshly
/// written page-tall PNG blocked the UI thread for seconds while the virus scanner read it.
/// ponytail: grows by one entry per picture rendered this session - bytes, not worth evicting.
static PNG_SIZES: Mutex<std::collections::BTreeMap<PathBuf, (u32, u32)>> =
    Mutex::new(std::collections::BTreeMap::new());

pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Some((be(16), be(20)))
}

/// PNG bytes (Edge's: 8-bit RGB or RGBA) → straight-alpha RGBA, in-process.
pub fn decode_png(png: &[u8]) -> Result<Frame, String> {
    let mut d = png::Decoder::new(std::io::Cursor::new(png));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().map_err(|e| format!("PNG: {e}"))?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("PNG: too big")?];
    let info = r.next_frame(&mut buf).map_err(|e| format!("PNG: {e}"))?;
    let (w, h) = (info.width, info.height);
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
    };
    if rgba.len() != (w * h * 4) as usize {
        return Err(format!("could not decode the {w}x{h} picture"));
    }
    Ok(Frame { width: w, height: h, pts: 0.0, rgba, bgrx: false, gpu: None })
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
    // the fast renderer (engine::blitz) first; Edge below is also the fallback when it fails
    if blitz::choose(v.renderer, html).engine == blitz::Engine::Blitz {
        let ptr = v.mouse.map(|m| m.state(t));
        match blitz::png_still(&page, v.base, size, scale, t, &values, ptr, tpl.fullpage) {
            Ok(png) => return Ok(png),
            Err(e) => blitz::remember_failure(html, &e),
        }
    }
    with_edge(true, |e| {
        e.load(&page, size, scale)?;
        e.set_params(&values)?;
        if let Some(m) = v.mouse {
            // a still of the moment: the mouse is where it is at `t`, its hover has settled
            let mut st = MouseState::default();
            e.mouse_setup()?;
            let (x, y, down) = m.state(t);
            e.mouse_frame(&mut st, x, y, down, t * 1000.0)?;
        }
        let (count, _) = e.timing()?;
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
    /// Seconds of the layer's own time to render first (around the playhead): a long render writes them
    /// as soon as they are in, at full rate, with the draft filling the rest (`interim`). Not part of
    /// the cache key: it only changes the order frames are rendered in.
    pub focus: Option<(f64, f64)>,
    /// Which program draws it (the layer's Renderer setting; `Auto` picks by `engine::blitz::choose`).
    pub renderer: HtmlRenderer,
}

/// Why this layer is slower than a plain CSS layer to render (empty = the quick path: the fast renderer
/// in parallel, unchanged frames reused). Shown on the clip's mark in the timeline. Effects, speed and
/// masks are not here: they work on the rendered frames on the GPU and cost the render nothing.
pub fn slow_reasons(h: &crate::model::HtmlSource) -> std::sync::Arc<Vec<String>> {
    use std::hash::{Hash, Hasher};
    static MEMO: Mutex<Option<std::collections::HashMap<u64, std::sync::Arc<Vec<String>>>>> = Mutex::new(None);
    let mut k = std::hash::DefaultHasher::new();
    (&h.src, h.mouse.is_some(), format!("{:?}{:?}", h.renderer, h.params), h.duration.to_bits()).hash(&mut k);
    let key = k.finish();
    if let Some(v) = lock(&MEMO).as_ref().and_then(|m| m.get(&key)) {
        return v.clone();
    }
    let v = std::sync::Arc::new(slow_reasons_uncached(h));
    let mut g = lock(&MEMO);
    let m = g.get_or_insert_with(Default::default);
    if m.len() > 256 {
        m.clear();
    }
    m.insert(key, v.clone());
    v
}

fn slow_reasons_uncached(h: &crate::model::HtmlSource) -> Vec<String> {
    let mut why = Vec::new();
    let choice = blitz::choose(h.renderer, &h.src);
    if choice.engine == blitz::Engine::Edge {
        why.push(match choice.reason {
            Some(r) => format!("Edge renderer: {r}"),
            None => "Edge renderer: fewer frames at once than the fast renderer".into(),
        });
    }
    if h.mouse.is_some() {
        why.push("Fake mouse: hover and click states are recorded in order, one frame at a time".into());
    }
    let tpl = parse(&h.src);
    if animated_params(&tpl, &params_of(&h.src, &h.params)) {
        why.push("Keyframed params: every frame is drawn, none can be reused".into());
    }
    if h.src.to_ascii_lowercase().contains("infinite") {
        why.push("An infinite animation: every frame of the layer is drawn".into());
    }
    if h.duration > 60.0 {
        why.push(format!("Long layer ({:.0} s): a 5 fps draft plays first while the rest renders", h.duration));
    }
    why
}

/// A finished (or cached) render.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// an H.264 `.mp4` (true) or a single `.png` (the page has no animation).
    pub animated: bool,
}

pub fn dir() -> PathBuf {
    if cfg!(test) {
        // tests render for real: into a temp folder of their own, never the user's cache
        return std::env::temp_dir().join(format!("se-test-html-{}", std::process::id()));
    }
    crate::settings::Settings::cache_dir().join("html")
}

/// The viewport of `src` at layout width `width` (0 = its `se:size`), in CSS pixels.
pub fn viewport(src: &str, width: u32) -> (u32, u32) {
    sized(&parse(src), width)
}

/// An animated render's file: `<hash><part>.mp4`, `.stacked.mp4` when it has transparency. `part` is
/// "" (the render), ".draft" or ".window" (`interim`).
fn anim_path(base: &Path, part: &str, alpha: bool) -> PathBuf {
    let name = base.file_name().unwrap_or_default().to_string_lossy();
    base.with_file_name(format!("{name}{part}{}", if alpha { ".stacked.mp4" } else { ".mp4" }))
}

/// A draft or window file (`interim`): only shown while the render runs, never kept or proxied.
pub fn is_interim(path: &Path) -> bool {
    let n = path.file_name().unwrap_or_default().to_string_lossy();
    n.contains(".draft.") || n.contains(".window.")
}

/// Every file a render may have left for the cache name `stem` (16 hex digits): the render (either
/// layout), its interims and its still, and the PNG-in-MOV files of html-v4 and before.
pub fn render_files(stem: &str) -> Vec<PathBuf> {
    if stem.len() != 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Vec::new(); // not a render (pending.png, a zoom detail, a temp file)
    }
    let base = dir().join(stem);
    let mut v: Vec<PathBuf> = ["", ".draft", ".window"]
        .iter()
        .flat_map(|part| [anim_path(&base, part, false), anim_path(&base, part, true)])
        .collect();
    v.extend(["png", "still.png", "mov", "draft.mov", "window.mov"].map(|e| base.with_extension(e)));
    v
}

/// The interims of the render named `base` (drafts, windows, the still): done with once it is in.
pub fn interim_files(base: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> =
        [".draft", ".window"].iter().flat_map(|p| [anim_path(base, p, false), anim_path(base, p, true)]).collect();
    v.push(base.with_extension("still.png"));
    v
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
    out_base_for(spec, blitz::choose(spec.renderer, &spec.html).engine == blitz::Engine::Blitz)
}

/// The cached render and the interim picture for `spec`, from ONE `out_base` (what the HTML layer
/// scan needs per layer): (base, size, cached, interim). `interim` is only looked up on a cache miss.
pub fn lookup(spec: &Spec) -> (PathBuf, (u32, u32), Option<Rendered>, Option<Rendered>) {
    let (base, (w, h)) = out_base(spec);
    let done = cached_at(&base, w, h);
    let interim = if done.is_none() { interim_at(&base, w, h) } else { None };
    (base, (w, h), done, interim)
}

/// `out_base` for a renderer: the fast renderer's pictures are not Edge's (different anti-aliasing, a few
/// CSS differences), so they live in files of their own; Edge's names are the ones from before.
fn out_base_for(spec: &Spec, fast: bool) -> (PathBuf, (u32, u32)) {
    use std::hash::{Hash, Hasher};
    let prep = prepared(&spec.html, &spec.fields, &spec.base, blitz::source_key(&spec.html));
    let (tpl, page) = (&prep.tpl, &prep.page);
    let px = pixel_size(sized(tpl, spec.width), spec.scale.clamp(0.05, 4.0));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let params = serde_json::to_string(&params_hashable(tpl, &spec.params)).unwrap_or_default();
    let mouse = serde_json::to_string(&spec.mouse).unwrap_or_default();
    ("html-v5", page, px, spec.fps.to_bits(), spec.duration.to_bits(), prep.deps_stamp(&spec.base), params, mouse)
        .hash(&mut h);
    if fast {
        "blitz-1".hash(&mut h);
    }
    (dir().join(format!("{:016x}", h.finish())), px)
}

/// The render for `spec` if it is already in the cache (no render).
pub fn cached(spec: &Spec) -> Option<Rendered> {
    let (base, (width, height)) = out_base(spec);
    cached_at(&base, width, height)
}

fn cached_at(base: &Path, width: u32, height: u32) -> Option<Rendered> {
    let (ew, eh) = even((width, height));
    let anim = [anim_path(base, "", false), anim_path(base, "", true)].into_iter().find(|p| p.is_file());
    if let Some(path) = anim {
        return Some(Rendered { path, width: ew, height: eh, animated: true });
    }
    let path = base.with_extension("png");
    // a full-page capture is as tall as its page: ask the file
    path.is_file().then(|| {
        let (width, height) = png_file_size(&path).unwrap_or((width, height));
        Rendered { path, width, height, animated: false }
    })
}

/// H.264 4:2:0 wants even sizes: an odd one is encoded a pixel bigger.
fn even((w, h): (u32, u32)) -> (u32, u32) {
    ((w.max(2) + 1) & !1, (h.max(2) + 1) & !1)
}

/// The interim still of `spec` (its moment `thumb_at`), if one was written: what the asset shows while
/// its whole animation renders (`render_to_file` writes it first).
#[cfg(test)] // the app asks through `lookup` (one cache key per layer)
pub fn interim(spec: &Spec) -> Option<Rendered> {
    let (base, (width, height)) = out_base(spec);
    interim_at(&base, width, height)
}

fn interim_at(base: &Path, width: u32, height: u32) -> Option<Rendered> {
    let (ew, eh) = even((width, height));
    for part in [".window", ".draft"] {
        for alpha in [false, true] {
            let p = anim_path(base, part, alpha);
            if p.is_file() {
                return Some(Rendered { path: p, width: ew, height: eh, animated: true });
            }
        }
    }
    let path = base.with_extension("still.png");
    let (width, height) = png_file_size(&path)?;
    Some(Rendered { path, width, height, animated: false })
}

/// Can the picture change between the moments `a` < `b` (ms), given every animation's active phase?
/// Before and after its active phase an animation holds still (with or without a fill).
fn moves_between(phases: &[(f64, f64)], a: f64, b: f64) -> bool {
    phases.iter().any(|&(s, e)| e > a && s <= b)
}

fn write_file(out: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = export::temp_output(out);
    std::fs::write(&tmp.0, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    tmp.commit(out)?;
    if let Some(size) = png_size(bytes) {
        lock(&PNG_SIZES).insert(out.to_path_buf(), size); // the scan never has to open it
    }
    Ok(())
}

/// Render `spec` into the cache (or reuse what is there): PNG frames piped to ffmpeg as H.264
/// (`encode_args`), or one `.png` for a page with no animation. First (as a short render, ahead of any long one) the
/// still `interim` shows. A frame where no animation is in its active phase since the frame before is
/// that same picture: rendered once, repeated (holds between an intro and an outro, the end). The
/// frame loop steps aside for short renders between frames (not with a fake mouse: its hover state
/// lives in the page). Progress and cancel through `prog`.
pub fn render_to_file(spec: &Spec, prog: &Progress) -> Result<Rendered, String> {
    if let Some(r) = cached(spec) {
        return Ok(r);
    }
    if blitz::choose(spec.renderer, &spec.html).engine == blitz::Engine::Blitz {
        match render_to_file_with(spec, prog, true) {
            Err(e) if !prog.is_cancelled() => {
                // the fast renderer could not draw it (panic, hang, bad page): Edge does, and Auto remembers
                blitz::remember_failure(&spec.html, &e);
                eprintln!("html: fast renderer failed ({e}); drawing with Edge");
            }
            r => return r,
        }
    }
    render_to_file_with(spec, prog, false)
}

/// `render_to_file` with the renderer decided: `fast` = the `se-html` helper, else Edge. The GPU's H.264
/// encoder when one works (`encoders::best_for`), else libx264; a GPU encode that fails (a driver out of
/// sessions, say) renders again in software.
fn render_to_file_with(spec: &Spec, prog: &Progress, fast: bool) -> Result<Rendered, String> {
    let enc = crate::engine::encoders::best_for("mp4");
    match render_encoded(spec, prog, fast, enc.as_deref()) {
        Err(e) if enc.is_some() && e.starts_with(ENCODER_FAILED) && !prog.is_cancelled() => {
            eprintln!("html: {e}; software encode");
            render_encoded(spec, prog, fast, None)
        }
        r => r,
    }
}

/// The error of a render whose encoder (not its frames) failed: `render_to_file_with` retries those.
const ENCODER_FAILED: &str = "encoder failed: ";

/// ffmpeg output args of an animated render at `fps`, `size` (even) pixels: H.264 4:2:0, a keyframe
/// every half second and no B-frames (seeks and scrubs decode at most half a second), constant frame
/// rate. Opaque = a plain picture; `alpha` = the stacked colour-over-alpha layout of the alpha proxies
/// (`proxy::stacked_filter`), which `proxy::StackedAlpha` folds back at decode. `enc` = a GPU encoder
/// (`encoders::best_for`), anything else = libx264. Quality ~CRF 18: a 5 s 1080p60 title is a few MB
/// where the PNG-in-MOV of html-v4 was ~150 MB.
fn encode_args(enc: Option<&str>, alpha: bool, size: (u32, u32), fps: f64) -> Vec<String> {
    let wh = format!("{}:{}", size.0, size.1);
    let vf = if alpha { crate::media::proxy::stacked_filter_to(&wh) } else { format!("scale={wh},format=yuv420p") };
    let codec: &[&str] = match enc {
        Some("h264_nvenc") => {
            &["-c:v", "h264_nvenc", "-preset", "p6", "-tune", "hq", "-rc", "vbr", "-cq", "19", "-b:v", "0"]
        }
        Some("h264_qsv") => &["-c:v", "h264_qsv", "-preset", "slow", "-global_quality", "19"],
        Some("h264_amf") => &["-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "19", "-qp_p", "21"],
        _ => &["-c:v", "libx264", "-preset", "medium", "-crf", "18"],
    };
    let rate = format!("{}/1000", (fps * 1000.0).round());
    let gop = ((fps / 2.0).round() as u32).max(1).to_string();
    let mut v: Vec<String> = ["-vf", vf.as_str()].iter().chain(codec).map(|a| a.to_string()).collect();
    v.extend(
        ["-g", &gop, "-bf", "0", "-pix_fmt", "yuv420p", "-r", &rate, "-an", "-movflags", "+faststart", "-f", "mp4"]
            .map(String::from),
    );
    v
}

/// Is this PNG opaque (every pixel's alpha 255)? An RGB / grey PNG is without reading a pixel.
fn opaque(png: &[u8]) -> bool {
    if png.len() > 25 && matches!(png[25], 0 | 2) {
        return true;
    }
    decode_png(png).is_ok_and(|f| f.rgba.chunks_exact(4).all(|p| p[3] == 255))
}

/// Does the layer have transparency? Asked of four moments: the start, the still, the middle, the last
/// frame. ponytail: a page opaque at all four but see-through in between (a mid-layer cut-out) loses it;
/// check every frame as it is written if one ever does. A fake mouse is always stacked (its hover state
/// is replayed in order, so no moments out of order are shot).
fn probe_alpha(times: [f64; 4], mut shot: impl FnMut(f64) -> Result<Vec<u8>, String>) -> Result<bool, String> {
    for ms in times {
        if !opaque(&shot(ms)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Render with encoder `enc` (None = libx264).
fn render_encoded(spec: &Spec, prog: &Progress, fast: bool, enc: Option<&str>) -> Result<Rendered, String> {
    let (base, (pw, ph)) = out_base_for(spec, fast);
    if let Some(r) = cached_at(&base, pw, ph) {
        return Ok(r);
    }
    let (tpl, page) = prepare_in(&spec.html, &spec.fields, &spec.base);
    let vp = sized(&tpl, spec.width);
    let (scale, fps) = (spec.scale.clamp(0.05, 4.0), spec.fps.clamp(1.0, 240.0));
    let frames = (spec.duration.clamp(0.04, 3600.0) * fps).ceil().max(1.0) as usize;
    std::fs::create_dir_all(dir()).map_err(|e| format!("{}: {e}", dir().display()))?;
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    prog.set(0.0, "Rendering HTML…");
    // a keyframed param or a mouse moves the picture as surely as a CSS animation does
    let moving = animated_params(&tpl, &spec.params) || spec.mouse.is_some();
    let still_ms = tpl.thumb_at().min(spec.duration).max(0.0) * 1000.0;
    let probe_ms = [0.0, still_ms, frames as f64 / 2.0 * 1000.0 / fps, (frames - 1) as f64 * 1000.0 / fps];
    // 1: the animations, and the picture to show until the render is done (or the only one)
    let mut shooter: Option<blitz::Shooter> = None;
    let first = if fast {
        let mut sh = blitz::Shooter::open(&page, &spec.base, vp, scale)?;
        if !sh.info.animated && !moving {
            sh.close();
            let png = blitz::png_still(&page, &spec.base, vp, scale, 0.0, &values_at(&tpl, &spec.params, 0.0), None, tpl.fullpage)?;
            let out = base.with_extension("png");
            write_file(&out, &png)?;
            let (width, height) = png_size(&png).unwrap_or((1, 1));
            return Ok(Rendered { path: out, width, height, animated: false });
        }
        if spec.mouse.is_none() {
            let png = sh.png(still_ms / 1000.0, &values_at(&tpl, &spec.params, still_ms / 1000.0), None)?;
            write_file(&base.with_extension("still.png"), &png)?;
            STILLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        // the one window in which the page can move: a finite animation's, or all of time
        let phases = match (sh.info.animated, sh.info.ends) {
            (false, _) => Vec::new(),
            (true, Some(e)) => vec![(0.0, e * 1000.0)],
            (true, None) => vec![(0.0, f64::INFINITY)],
        };
        let alpha = spec.mouse.is_some()
            || probe_alpha(probe_ms, |ms| sh.png(ms / 1000.0, &values_at(&tpl, &spec.params, ms / 1000.0), None))?;
        shooter = Some(sh);
        Ok((phases, alpha))
    } else {
      with_edge(true, |e| {
        e.load(&page, vp, scale)?;
        let (count, phases) = e.timing()?;
        let mut png = Vec::new();
        if count == 0 && !moving {
            e.set_params(&values_at(&tpl, &spec.params, 0.0))?;
            if tpl.fullpage {
                e.shot_full(vp.0, &mut png)?;
            } else {
                e.shot(&mut png)?;
            }
            let out = base.with_extension("png");
            write_file(&out, &png)?;
            let (width, height) = png_size(&png).unwrap_or((1, 1));
            return Ok(Err(Rendered { path: out, width, height, animated: false }));
        }
        if spec.mouse.is_none() {
            e.set_params(&values_at(&tpl, &spec.params, still_ms / 1000.0))?;
            e.seek(still_ms)?;
            e.shot(&mut png)?;
            write_file(&base.with_extension("still.png"), &png)?;
            STILLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let alpha = spec.mouse.is_some()
            || probe_alpha(probe_ms, |ms| {
                e.set_params(&values_at(&tpl, &spec.params, ms / 1000.0))?;
                e.seek(ms)?;
                let mut p = Vec::new();
                e.shot(&mut p)?;
                Ok(p)
            })?;
        Ok(Ok((phases, alpha)))
      })?
    };
    let (phases, alpha) = match first {
        Ok(p) => p,
        Err(done) => return Ok(done),
    };
    let size = even((pw, ph));
    // 2: every frame. A chunk is Ok(true) when the render is over (done, cancelled, ffmpeg gone), Ok(false)
    // when it steps aside for a short render.
    // H.264 that Media Foundation plays like footage (the PNG-in-MOV of html-v4 cost ~36 ms a frame to
    // decode and needed a proxy besides): no proxy is built for it
    let out = anim_path(&base, "", alpha);
    let tmp = export::temp_output(&out);
    let mut cmd = crate::media::bg::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-f", "image2pipe"])
        .args(["-framerate", &format!("{}/1000", (fps * 1000.0).round())])
        .args(["-c:v", "png", "-i", "-"]);
    let mut child = cmd
        .args(encode_args(enc, alpha, size, fps))
        .arg(&tmp.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    let mut stdin = child.stdin.take().ok_or("ffmpeg stdin")?;
    let yields = spec.mouse.is_none();
    let to_render = frames_to_render(&phases, moving, frames, fps);
    let workers = if spec.mouse.is_none() { worker_count_for(to_render, fast) } else { 1 };
    let fed = if workers > 1 {
        if let Some(sh) = shooter.take() {
            sh.close(); // the workers open their own
        }
        let job = Job { page: &page, base: &spec.base, fast, vp, scale, fps, frames, tpl: &tpl, params: &spec.params, phases: &phases, moving, enc, alpha, size };
        // a long one: a 5 fps draft the clip can play first, then the frames around the playhead at full
        // rate (with the draft filling the rest), then every frame
        let long = frames as f64 / fps > 8.0;
        let draft = long.then(|| anim_path(&base, ".draft", alpha));
        let window = spec.focus.filter(|_| long).map(|(a, b)| {
            let f = |t: f64| ((t * fps).round().max(0.0) as usize).min(frames);
            (f(a), f(b).max(f(a) + 1).min(frames))
        });
        let window = window.filter(|&(a, b)| b - a < frames).map(|w| (w, anim_path(&base, ".window", alpha)));
        let window = window.as_ref().map(|(w, p)| (*w, p.as_path()));
        render_parallel(&job, workers, &mut stdin, draft.as_deref(), window, prog)
    } else if fast {
        // one helper process, in frame order (also the only way a fake mouse's hover state is replayed)
        let mut sh = match shooter.take() {
            Some(s) => s,
            None => blitz::Shooter::open(&page, &spec.base, vp, scale)?,
        };
        let r = (|| -> Result<(), String> {
            let (mut png, mut last_ms) = (Vec::new(), None::<f64>);
            for i in 0..frames {
                if prog.is_cancelled() {
                    return Ok(()); // wait_ffmpeg below kills it and says so
                }
                let ms = i as f64 * 1000.0 / fps;
                if moving || last_ms.is_none_or(|a| moves_between(&phases, a, ms)) {
                    let ptr = spec.mouse.as_ref().map(|m| m.state(ms / 1000.0));
                    png = sh.png(ms / 1000.0, &values_at(&tpl, &spec.params, ms / 1000.0), ptr)?;
                    last_ms = Some(ms);
                }
                if stdin.write_all(&png).is_err() {
                    return Ok(()); // ffmpeg stopped: its own error is the one worth reporting
                }
                prog.set((i + 1) as f32 / frames as f32 * 0.99, "Rendering HTML…");
            }
            Ok(())
        })();
        sh.close();
        r
    } else {
        let mut i = 0usize;
        let (mut png, mut last_ms, mut mouse_state, mut mouse_ready) = (Vec::new(), None::<f64>, MouseState::default(), false);
        let mut chunk = |e: &mut Edge| -> Result<bool, String> {
            e.load(&page, vp, scale)?;
            if spec.mouse.is_some() && !mouse_ready {
                e.set_params(&values_at(&tpl, &spec.params, 0.0))?;
                e.mouse_setup()?;
                mouse_ready = true;
            }
            while i < frames {
                if prog.is_cancelled() {
                    return Ok(true); // wait_ffmpeg below kills it and says so
                }
                if yields && short_waiting() {
                    return Ok(false);
                }
                let ms = i as f64 * 1000.0 / fps;
                if moving || last_ms.is_none_or(|a| moves_between(&phases, a, ms)) {
                    e.set_params(&values_at(&tpl, &spec.params, ms / 1000.0))?;
                    if let Some(m) = &spec.mouse {
                        let (x, y, down) = m.state(ms / 1000.0);
                        e.mouse_frame(&mut mouse_state, x, y, down, ms)?;
                        e.seek_rel(ms)?;
                    } else {
                        e.seek(ms)?;
                    }
                    e.shot(&mut png)?;
                    last_ms = Some(ms);
                }
                if stdin.write_all(&png).is_err() {
                    return Ok(true); // ffmpeg stopped: its own error is the one worth reporting
                }
                i += 1;
                prog.set(i as f32 / frames as f32 * 0.99, "Rendering HTML…");
            }
            Ok(true)
        };
        loop {
            match with_edge(false, &mut chunk) {
                Ok(false) => {}
                r => break r.map(|_| ()),
            }
        }
    };
    drop(stdin);
    if let Err(err) = fed {
        // Edge failed mid-render: stop ffmpeg before the temp file is removed
        let _ = child.kill();
        let _ = child.wait();
        if let Some(t) = tail {
            let _ = t.join();
        }
        return Err(err);
    }
    export::wait_ffmpeg(&mut child, tail, prog).map_err(|e| match enc {
        Some(enc) if e != export::CANCELLED => format!("{ENCODER_FAILED}{enc}: {e}"),
        _ => e,
    })?;
    tmp.commit(&out)?;
    // the draft and window were only for while it rendered (one still open in a decoder goes when the
    // asset is re-pointed: `html_ctl`)
    for f in interim_files(&base).iter().filter(|f| f.extension().is_some_and(|e| e == "mp4")) {
        let _ = std::fs::remove_file(f);
    }
    Ok(Rendered { path: out, width: size.0, height: size.1, animated: true })
}

/// How many of `frames` need a screenshot (the rest repeat the frame before: nothing moved).
fn frames_to_render(phases: &[(f64, f64)], moving: bool, frames: usize, fps: f64) -> usize {
    let mut last: Option<f64> = None;
    (0..frames)
        .filter(|&i| {
            let ms = i as f64 * 1000.0 / fps;
            let r = moving || last.is_none_or(|a| moves_between(phases, a, ms));
            if r {
                last = Some(ms);
            }
            r
        })
        .count()
}

/// Edge processes a long render is split over: half the cores, at most 4 (measured: 2 = 1.6x, 4 = 2x, 8 =
/// 2.3x with ~0.9 GB of Edge each at 1080p), one per 60 frames to render;
/// a short one stays on the shared Edge. Env `SE_HTML_WORKERS` overrides (1 = the shared Edge only).
fn worker_count(to_render: usize) -> usize {
    if let Some(n) = std::env::var("SE_HTML_WORKERS").ok().and_then(|v| v.trim().parse::<usize>().ok()) {
        return n.clamp(1, 16);
    }
    if to_render < 150 {
        return 1;
    }
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores / 2).clamp(1, 4).min(to_render / 60).max(1)
}

/// `worker_count`, or for the fast renderer (a worker is ~25 MB, not ~900 MB): up to 6, one per 40 frames.
fn worker_count_for(to_render: usize, fast: bool) -> usize {
    if !fast || std::env::var("SE_HTML_WORKERS").is_ok() {
        return worker_count(to_render);
    }
    if to_render < 80 {
        return 1;
    }
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores.saturating_sub(1)).clamp(1, 6).min(to_render / 40).max(1)
}

/// A render worker's frame source: its own Edge page, or its own `se-html` process.
enum WorkerSrc {
    Edge(Edge),
    Fast(blitz::Shooter),
}

impl WorkerSrc {
    fn open(job: &Job) -> Result<WorkerSrc, String> {
        if job.fast {
            return blitz::Shooter::open(job.page, job.base, job.vp, job.scale).map(WorkerSrc::Fast);
        }
        let mut e = Edge::launch()?;
        e.load(job.page, job.vp, job.scale)?;
        Ok(WorkerSrc::Edge(e))
    }

    /// PNG of the frame at `ms` (params applied).
    fn shoot(&mut self, job: &Job, ms: f64) -> Result<Vec<u8>, String> {
        let values = values_at(job.tpl, job.params, ms / 1000.0);
        match self {
            WorkerSrc::Fast(s) => s.png(ms / 1000.0, &values, None),
            WorkerSrc::Edge(e) => {
                e.set_params(&values)?;
                e.seek(ms)?;
                let mut v = Vec::new();
                e.shot(&mut v)?;
                Ok(v)
            }
        }
    }

    fn close(self) {
        match self {
            WorkerSrc::Fast(s) => s.close(),
            WorkerSrc::Edge(e) => e.close(),
        }
    }
}

/// What every worker of a parallel render needs.
struct Job<'a> {
    page: &'a str,
    base: &'a str,
    fast: bool,
    vp: (u32, u32),
    scale: f64,
    fps: f64,
    frames: usize,
    tpl: &'a Template,
    params: &'a [(String, crate::model::Animated)],
    phases: &'a [(f64, f64)],
    moving: bool,
    /// The encoder, the layout and the (even) size of the files written (`encode_args`).
    enc: Option<&'a str>,
    alpha: bool,
    size: (u32, u32),
}

/// Frames per piece of work a worker takes (in order, so the writer never waits on far-ahead frames).
const PIECE: usize = 30;
/// The draft of a long render: this many frames a second, rendered first, playable at once
/// (`interim`); the full render reuses them.
const DRAFT_FPS: f64 = 5.0;

/// An interim writer (draft, window): ffmpeg fed PNG bytes on its stdin, encoded as the render is.
struct MovOut {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    tail: Option<std::thread::JoinHandle<String>>,
    tmp: Option<export::TempFile>,
    out: PathBuf,
}

impl MovOut {
    fn start(out: &Path, fps: f64, job: &Job) -> Result<MovOut, String> {
        let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
        let tmp = export::temp_output(out);
        let mut child = crate::media::bg::command(&ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "image2pipe"])
            .args(["-framerate", &format!("{}/1000", (fps * 1000.0).round())])
            .args(["-c:v", "png", "-i", "-"])
            .args(encode_args(job.enc, job.alpha, job.size, fps))
            .arg(&tmp.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let tail = export::stderr_tail(&mut child);
        let stdin = child.stdin.take();
        Ok(MovOut { child, stdin, tail, tmp: Some(tmp), out: out.to_path_buf() })
    }

    /// Close the input, wait, and move the file into place.
    fn finish(mut self, prog: &Progress) -> Result<(), String> {
        drop(self.stdin.take());
        export::wait_ffmpeg(&mut self.child, self.tail.take(), prog)?;
        self.tmp.take().ok_or("no temp file")?.commit(&self.out)
    }
}

impl Drop for MovOut {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            // not finished: stop ffmpeg before the temp file is removed
            drop(self.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The frames of `job` rendered by `n` Edge processes of their own (a profile each, closed when done),
/// each taking the next `PIECE` frames of a shared list as it finishes one, and written to `out` in
/// order. Frames are exactly those of the one-Edge render: every frame is seeked and captured the same
/// way, at `job.fps`. The list: with `draft` every k-th frame first (`DRAFT_FPS`), written as `draft`
/// once in (a short video the clip plays while the rest renders); with `window` (frames [a, b))
/// those next, written as a full-length, full-rate MOV where frames outside the window repeat the
/// nearest draft frame; then the rest, nearest the window first.
/// ponytail: frames rendered ahead of the one being written wait in memory (every frame of a 60 s
/// 1080p layer is ~150-300 MB of PNG); a spill to disk if that ever matters.
fn render_parallel(
    job: &Job,
    n: usize,
    out: &mut impl Write,
    draft: Option<&Path>,
    window: Option<((usize, usize), &Path)>,
    prog: &Progress,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Arc;
    let k = if draft.is_some() { (job.fps / DRAFT_FPS).round().max(2.0) as usize } else { 0 };
    let is_draft = |i: usize| k > 0 && i % k == 0;
    let draft_idx: Vec<usize> = (0..job.frames).filter(|&i| is_draft(i)).collect();
    let (wa, wb) = window.map_or((0, 0), |w| w.0);
    let win_idx: Vec<usize> = (wa..wb).filter(|&i| !is_draft(i)).collect();
    let mut rest: Vec<usize> = (0..job.frames).filter(|&i| !is_draft(i) && !(wa..wb).contains(&i)).collect();
    // outward from the window: the frames playback reaches next come next
    rest.sort_by_key(|&i| if i < wa { (wa - i) * 2 } else { (i - wb) * 2 + 1 });
    let order: Vec<usize> = draft_idx.iter().chain(&win_idx).chain(&rest).copied().collect();
    let pieces: Vec<&[usize]> = order.chunks(PIECE).collect();
    let next = AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(usize, Arc<Vec<u8>>), String>>(n * PIECE * 2);
    std::thread::scope(|sc| {
        for _ in 0..n {
            let (tx, next, pieces) = (tx.clone(), &next, &pieces);
            sc.spawn(move || {
                crate::media::bg::lower_thread();
                let mut e = match WorkerSrc::open(job) {
                    Ok(e) => e,
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        return;
                    }
                };
                let r = (|| -> Result<(), String> {
                    loop {
                        crate::media::bg::pause_point(|| prog.is_cancelled());
                        let p = next.fetch_add(1, SeqCst);
                        if p >= pieces.len() || prog.is_cancelled() {
                            return Ok(());
                        }
                        let mut last: Option<(f64, Arc<Vec<u8>>)> = None;
                        for &i in pieces[p] {
                            let ms = i as f64 * 1000.0 / job.fps;
                            let png = match &last {
                                Some((a, p)) if !job.moving && *a <= ms && !moves_between(job.phases, *a, ms) => p.clone(),
                                _ => {
                                    let p = Arc::new(e.shoot(job, ms)?);
                                    last = Some((ms, p.clone()));
                                    p
                                }
                            };
                            if tx.send(Ok((i, png))).is_err() {
                                return Ok(()); // the writer stopped
                            }
                        }
                    }
                })();
                e.close();
                if let Err(err) = r {
                    let _ = tx.send(Err(err));
                }
            });
        }
        drop(tx);
        // every frame in so far, kept until the window file is written (it needs them all again)
        let mut all: Vec<Option<Arc<Vec<u8>>>> = if window.is_some() { vec![None; job.frames] } else { Vec::new() };
        let mut got: std::collections::HashMap<usize, Arc<Vec<u8>>> = std::collections::HashMap::new();
        let (mut want, mut dpos, mut in_count) = (0usize, 0usize, 0usize);
        let (mut draft_done, mut window_done) = (draft_idx.is_empty(), window.is_none());
        let mut dout = match draft {
            Some(d) if !draft_idx.is_empty() => Some(MovOut::start(d, job.fps / k as f64, job)?),
            _ => None,
        };
        for m in rx {
            let (i, png) = m?;
            in_count += 1;
            if !window_done {
                all[i] = Some(png.clone());
            }
            got.insert(i, png);
            // the draft first: it only needs its own frames
            if let Some(d) = dout.as_mut() {
                while dpos < draft_idx.len() {
                    let Some(png) = got.get(&draft_idx[dpos]).or(all.get(draft_idx[dpos]).and_then(|x| x.as_ref())) else { break };
                    if d.stdin.as_mut().is_none_or(|w| w.write_all(png).is_err()) {
                        break;
                    }
                    dpos += 1;
                }
                if dpos == draft_idx.len() {
                    if let Some(d) = dout.take() {
                        if d.finish(&Progress::new()).is_ok() {
                            STILLS.fetch_add(1, SeqCst);
                        }
                    }
                    draft_done = true;
                }
            }
            // then the window, full rate, the draft standing in for what is not rendered yet
            if draft_done && !window_done && in_count >= draft_idx.len() + win_idx.len() && win_idx.iter().all(|&i| all[i].is_some()) {
                if let Some((_, path)) = window {
                    let mut w = MovOut::start(path, job.fps, job)?;
                    let mut ok = true;
                    for i in 0..job.frames {
                        let held = if k > 0 { i - i % k } else { i };
                        let Some(png) = all[i].as_ref().or(all[held].as_ref()) else { continue };
                        if w.stdin.as_mut().is_none_or(|s| s.write_all(png).is_err()) {
                            ok = false;
                            break;
                        }
                    }
                    if ok && w.finish(&Progress::new()).is_ok() {
                        STILLS.fetch_add(1, SeqCst);
                    }
                }
                window_done = true;
                all = Vec::new();
            }
            while let Some(png) = got.remove(&want) {
                if out.write_all(&png).is_err() {
                    return Ok(()); // ffmpeg stopped: its own error is the one worth reporting
                }
                want += 1;
            }
            let label = if !draft_done { "draft" } else if !window_done { "around the playhead" } else { "every frame" };
            prog.set(in_count as f32 / job.frames as f32 * 0.99, format!("Rendering HTML: {label} ({n} at once)…"));
        }
        Ok(())
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

// ---------------------------------------------------------------------------------------------------
// ---- ws:browser-layer ---- what `engine::webpage` needs from the Edge kept here

/// A page loaded in the shared Edge, for a caller that needs more than a frame of it.
pub struct PageSession<'a>(&'a mut Edge);

impl PageSession<'_> {
    /// The value of a JavaScript expression (it runs although the page's own scripts never do).
    pub fn eval(&mut self, js: &str) -> Result<Value, String> {
        self.0.cdp.eval(js)
    }
    /// PNG of the part `rect` (x, y, w, h in CSS px, anywhere on the page) drawn at `scale` picture px
    /// per CSS px.
    pub fn shot(&mut self, rect: [f64; 4], scale: f64) -> Result<Vec<u8>, String> {
        let clip = json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3], "scale": scale});
        let args = json!({"format": "png", "captureBeyondViewport": true, "clip": clip});
        let r = self.0.cdp.call("Page.captureScreenshot", args)?;
        let mut out = Vec::new();
        b64_decode(r["data"].as_str().unwrap_or("").as_bytes(), &mut out);
        png_size(&out).map(|_| out).ok_or_else(|| "Edge returned no picture".to_string())
    }
}

/// Load `page` (as `prepare_in` returns it) at `width` CSS px in the shared Edge and run `f` on it.
pub fn with_page<T>(page: &str, width: u32, f: &mut dyn FnMut(&mut PageSession) -> Result<T, String>) -> Result<T, String> {
    with_edge(true, |e| {
        e.load(page, (width, 1080), 1.0)?;
        f(&mut PageSession(e))
    })
}

/// The browser profile `capture_mhtml` runs on instead of a private, throw-away one: the folder the
/// user signed in to (`webpage::sign_in_profile`), so a capture shows what they see. None = private.
pub static CAPTURE_PROFILE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn capture_profile(network: bool) -> Option<PathBuf> {
    lock(&CAPTURE_PROFILE).clone().filter(|_| network)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: opening a project with six saved web pages (~780 KB of HTML each, mostly `data:`
    /// images) blocked the UI thread for minutes - the 2 s HTML layer scan re-prepared every page three
    /// times per pass. Now a page is prepared once, and its cache name is exactly the old one (renders
    /// already on disk stay valid).
    #[test]
    fn big_web_page_scan_prepares_once_and_keeps_its_cache_name() {
        use std::hash::{Hash, Hasher};
        let img = format!("<img src=\"data:image/png;base64,{}\">", "QUJD".repeat(200_000));
        let page = format!("<html><head><title>Saved page</title><script>x()</script></head><body>{img}<p onclick=\"y()\">hi</p></body></html>");
        let spec = Spec {
            html: page,
            fields: Vec::new(),
            scale: 1.0,
            fps: 60.0,
            duration: 4.0,
            base: String::new(),
            params: Vec::new(),
            width: 0,
            mouse: None,
            focus: None,
            renderer: HtmlRenderer::Edge,
        };
        let before = PREPARES.with(|n| n.get());
        let (a, px) = out_base(&spec);
        let (b, _, _, _) = lookup(&spec);
        assert_eq!(a, b);
        assert_eq!(PREPARES.with(|n| n.get()) - before, 1, "the second scan reuses the prepared page");
        // the name the code before the memo gave it
        let (tpl, page) = prepare_uncached(&spec.html, &spec.fields, &spec.base);
        assert_eq!(px, pixel_size(sized(&tpl, 0), 1.0));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let params = serde_json::to_string(&params_hashable(&tpl, &spec.params)).unwrap_or_default();
        let mouse = serde_json::to_string(&spec.mouse).unwrap_or_default();
        ("html-v5", &page, px, spec.fps.to_bits(), spec.duration.to_bits(), deps_stamp(&page, ""), params, mouse)
            .hash(&mut h);
        assert_eq!(a, dir().join(format!("{:016x}", h.finish())));
        assert!(!page.contains("<script") && !page.contains("onclick"), "still sanitised");
    }

    #[test]
    fn slow_reasons_name_the_mouse_and_infinite_animations_only_when_used() {
        let has = |v: &[String], w: &str| v.iter().any(|r| r.starts_with(w));
        let mut h = crate::model::HtmlSource { src: "<style>p{animation:a 1s}</style><p>hi</p>".into(), duration: 4.0, ..Default::default() };
        let plain = slow_reasons(&h);
        assert!(!has(&plain, "Fake mouse") && !has(&plain, "An infinite") && !has(&plain, "Long"));
        h.src = "<style>p{animation:a 1s infinite}</style><p>hi</p>".into();
        h.mouse = Some(crate::model::Mouse::at(0.0, 0.0));
        h.duration = 90.0;
        let slow = slow_reasons(&h);
        assert!(has(&slow, "Fake mouse") && has(&slow, "An infinite") && has(&slow, "Long"));
    }

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
                max: Some(1000.0),
                extra: vec!["0".into(), "1000".into()],
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
        let f = Field { name: "n".into(), ty: FieldType::Number, default: "1".into(), min: None, max: None, extra: Vec::new() };
        assert_eq!(field_value(&f, &v(&[("n", "2.25")])), "2.25");
        let img = Field { name: "i".into(), ty: FieldType::Image, default: String::new(), min: None, max: None, extra: Vec::new() };
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
        let f = render_frame_in(page, &[], 1.0, 0.0, &View { base: &base, renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
        assert_eq!(px(&f, 10, 10), [255, 0, 0, 255], "the stylesheet applied");
        assert!(px(&f, 130, 10)[1] > 100 && px(&f, 130, 10)[3] == 255, "the css image loaded: {:?}", px(&f, 130, 10));
        // without the folder the same page has neither
        let f = render_frame_in(page, &[], 1.0, 0.0, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
        assert_eq!(px(&f, 10, 10)[3], 0, "no folder: no stylesheet");
        // the cache key follows the stylesheet
        let spec = Spec { html: page.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 1.0, base, params: Vec::new(), width: 0, mouse: None, focus: None, renderer: HtmlRenderer::Edge };
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
        let spec = |params: Params| Spec { html: PARAM_PAGE.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 1.0, base: String::new(), params, width: 0, mouse: None, focus: None, renderer: HtmlRenderer::Edge };
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
        let f = render_frame_in(PARAM_PAGE, &[], 1.0, 0.5, &View { params: &params, renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
        assert_eq!(px(&f, 55, 10), [255, 0, 0, 255], "inside the box at x=50");
        assert_eq!(px(&f, 45, 10)[3], 0, "left of it");
        assert_eq!(px(&f, 75, 10)[3], 0, "right of it");
        // a constant: still the default 10 px
        let f = render_frame_in(PARAM_PAGE, &[], 1.0, 0.0, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
        assert_eq!(px(&f, 15, 10), [255, 0, 0, 255]);
        assert_eq!(px(&f, 35, 10)[3], 0);
        // the full render is a video (no CSS animation, but moving params) and its frames agree
        let spec = Spec { html: PARAM_PAGE.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 1.0, base: String::new(), params, width: 0, mouse: None, focus: None, renderer: HtmlRenderer::Edge };
        if let Some(r) = cached(&spec) {
            let _ = std::fs::remove_file(r.path);
        }
        let r = render_to_file(&spec, &Progress::new()).expect("render to file");
        assert!(r.animated, "keyframed params make a video");
        let mut v = crate::media::open_video(&r.path.to_string_lossy(), crate::media::Backend::Auto).expect("open");
        for (t, left) in [(0.2, 20u32), (0.8, 80)] {
            let mut f = Frame::default();
            assert!(v.frame_at(t, 200, 60, &mut f), "decode {t}");
            let b = px(&f, left + 5, 10);
            assert!(b[0] > 230 && b[1] < 24 && b[2] < 24 && b[3] == 255, "box at {left} for t={t}: {b:?}");
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
            focus: None,
            renderer: HtmlRenderer::Edge,
        };
        if let Some(r) = cached(&spec) {
            let _ = std::fs::remove_file(r.path);
        }
        let r = render_to_file(&spec, &Progress::new()).expect("render to file");
        assert!(r.animated, "a mouse makes a video");
        let mut v = crate::media::open_video(&r.path.to_string_lossy(), crate::media::Backend::Auto).expect("open");
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
        let still = render_frame_in(page, &[], 1.0, 1.6, &View { mouse: Some(&m), renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
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
        let f = render_frame_in(page, &[], 1.0, 0.0, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
        assert_eq!((f.width, f.height), (200, 450), "the whole page, not the 100 px viewport");
        let plain = page.replace(r#"<meta name="se:fullpage" content="auto">"#, "");
        assert_eq!(render_frame_in(&plain, &[], 1.0, 0.0, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render").height, 100);
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
        let f = render_frame_in(MOVING, &[], 1.0, 0.5, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
        eprintln!("first frame (Edge start included): {} ms", t0.elapsed().as_millis());
        check(&f, 50);
        let spec = Spec { html: MOVING.into(), fields: Vec::new(), scale: 1.0, fps: 10.0, duration: 2.5, base: String::new(), params: Vec::new(), width: 0, mouse: None, focus: None, renderer: HtmlRenderer::Edge };
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
        assert!(r.animated && r.path.to_string_lossy().ends_with(".stacked.mp4") && (r.width, r.height) == (320, 180));
        let mut v = crate::media::open_video(&r.path.to_string_lossy(), crate::media::Backend::Auto).expect("open the render");
        for (t, left) in [(0.5, 50u32), (1.5, 150)] {
            let mut f = Frame::default();
            assert!(v.frame_at(t, 320, 180, &mut f), "decode {t}");
            // H.264: the box's colour within a few levels, its alpha snapped to 0 / 255 by `StackedAlpha`
            let b = px(&f, left + 10, 30);
            assert!(b[0] > 240 && b[1] < 16 && b[2] < 16 && b[3] == 255, "the box at {left}: {b:?}");
            assert_eq!(px(&f, left.saturating_sub(5), 30)[3], 0, "left of the box at {left}");
            assert_eq!(px(&f, 5, 5)[3], 0, "transparent background");
            let h = px(&f, 270, 120);
            assert!(h[3].abs_diff(128) <= 4 && h[0] >= 240, "half white, straight alpha within a few levels: {h:?}");
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
            match render_frame_in(html, &[], scale, tpl.thumb_at(), &View { renderer: HtmlRenderer::Edge, ..Default::default() }) {
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

    /// Plain file names in a page find the project's media: exact path under the project folder, any
    /// case, the folder before the pool, a warning when a name is ambiguous, "missing" when nothing has
    /// it, CSS url() too; the page then points at the file, and the cache key follows the file.
    #[test]
    fn file_names_in_a_page_find_the_project_media() {
        let root = temp_dir("media");
        let (proj, a, b) = (root.join("My Project"), root.join("a"), root.join("b"));
        for d in [&proj, &proj.join("sub"), &a, &b] {
            std::fs::create_dir_all(d).unwrap();
        }
        for f in [proj.join("Image.PNG"), proj.join("sub").join("x.png"), a.join("logo.png"), b.join("logo.png"), a.join("clip ü.mp4"), a.join("star.svg")] {
            std::fs::write(&f, b"x").unwrap();
        }
        let m = Media::new(Some(proj.clone()), vec![a.join("logo.png"), a.join("clip ü.mp4"), b.join("logo.png"), a.join("star.svg")]);
        let page = r#"<img src="image.png"><img src='logo.png'><img src="sub/x.png"><video src="clip%20%C3%BC.mp4"></video>
<img src="nope.png"><img src="star"><img src="https://x.test/a.png"><img src="{{pic}}">
<style>.b{background:url(Image.png)} .c{background:url( "logo.png" )}</style>"#;
        let refs = media_refs_in(page, "", &m);
        let found = |n: &str| refs.iter().find_map(|r| match r {
            MediaRef::Found { name, path, warning } if name == n => Some((path.clone(), warning.clone())),
            _ => None,
        });
        let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
        let (img, w) = found("image.png").unwrap();
        assert!(same(&img, &proj.join("Image.PNG")) && w.is_none(), "any case, in the project folder: {img:?}");
        assert_eq!(found("sub/x.png").unwrap().0, proj.join("sub").join("x.png"), "a path under the project folder");
        let (logo, warn) = found("logo.png").unwrap();
        assert_eq!(logo, b.join("logo.png"), "two assets named logo.png: the newest");
        assert!(warn.is_some_and(|w| w.contains("matches 2 files")));
        assert_eq!(found("clip%20%C3%BC.mp4").unwrap().0, a.join("clip ü.mp4"), "escaped spaces and unicode");
        assert_eq!(found("star").unwrap().0, a.join("star.svg"), "no extension, one file of that name");
        assert!(same(&found("Image.png").unwrap().0, &proj.join("Image.PNG")), "CSS url()");
        assert!(refs.contains(&MediaRef::Missing("nope.png".into())), "{refs:?}");
        assert!(!refs.iter().any(|r| matches!(r, MediaRef::Found { name, .. } | MediaRef::Missing(name) if name.contains("x.test") || name.contains("{{"))));
        // the page points at the files, and nothing outside them is reachable by name
        let out = with_media_in(page.to_string(), "", &m);
        assert!(out.contains(&format!("\"{}\"", file_url(&img))) && out.contains(&format!("({})", file_url(&found("Image.png").unwrap().0))));
        assert!(out.contains(&format!("( \"{}\" )", file_url(&b.join("logo.png")))), "url( \"logo.png\" )");
        assert!(out.contains("\"nope.png\""), "a missing name is left as it is");
        assert!(find_media("../secret.txt", &m) == Some(MediaRef::Missing("../secret.txt".into())));
        // the cache key: the same file keeps it, a changed file changes it
        let stamp = || deps_stamp(&with_media_in(page.to_string(), "", &m), "");
        let s0 = stamp();
        assert_eq!(s0, stamp());
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(proj.join("Image.PNG"), b"a bigger picture").unwrap();
        assert_ne!(s0, stamp(), "replacing image.png on disk re-renders the layer");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `<video>` in a page is at the frame's own time in the frame-exact renderer (seeked by the
    /// app, page scripts off): red for its first second, blue for its second.
    #[test]
    fn a_video_in_a_page_is_seeked_to_every_frame() {
        if tools_missing() {
            return;
        }
        let dir = temp_dir("video");
        let v = dir.join("two colours.mp4");
        let ok = ffpipe::command(&ffpipe::ffmpeg_exe().unwrap())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=red:s=160x90:r=30:d=1", "-f", "lavfi", "-i", "color=c=blue:s=160x90:r=30:d=1"])
            .args(["-filter_complex", "[0][1]concat=n=2:v=1", "-pix_fmt", "yuv420p", "-c:v", "libx264"])
            .arg(&v)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg made the test video");
        let page = format!(
            "<html><head><meta name=\"se:size\" content=\"160x90\"><style>html,body{{margin:0}}video{{display:block;width:160px;height:90px}}</style></head>\
             <body><video src=\"{}\"></video></body></html>",
            file_url(&v)
        );
        for (t, want) in [(0.5, [255, 0, 0]), (1.5, [0, 0, 255]), (0.2, [255, 0, 0])] {
            let t0 = Instant::now();
            let f = render_frame_in(&page, &[], 1.0, t, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).expect("render");
            eprintln!("video frame at {t}: {} ms", t0.elapsed().as_millis());
            let p = &f.rgba[((45 * f.width + 80) * 4) as usize..][..3];
            // the colour of the frame shown (a decoded H.264 red is not exactly 255,0,0): the right one dominates
            let (r, b) = (p[0] as i32, p[2] as i32);
            let ok = if want[0] > 0 { r > b + 40 && r > p[1] as i32 + 40 } else { b > r + 40 && b > p[1] as i32 + 40 };
            assert!(ok, "at {t}: {:?} (rgba {:?})", p, &f.rgba[((45 * f.width + 80) * 4) as usize..][..4]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two previews on screen at once (the Inspector and the editor window) asking for different
    /// moments each frame: each keeps its own request (and tab), so neither one's picture is swapped
    /// away by the other (the blink: one shared tab flipped between the two every frame).
    #[test]
    fn two_live_previews_do_not_take_turns() {
        let req = |at: f64| LiveReq {
            page: "<html></html>".into(),
            size: (100, 50),
            scale: 1.0,
            params: Vec::new(),
            at,
            playing: false,
            looping: true,
            epoch: 0,
            duration: 4.0,
            fast: false,
            base: String::new(),
            src_key: 0,
            ptr: None,
        };
        let gen = |slot: usize| lock(&LIVE)[slot].gen;
        let (g0, g1) = (gen(LIVE_INSPECTOR), gen(LIVE_EDITOR));
        for _ in 0..20 {
            live_show(LIVE_INSPECTOR, req(6.0), || {});
            live_show(LIVE_EDITOR, req(1.5), || {});
        }
        assert!(gen(LIVE_INSPECTOR) - g0 <= 1 && gen(LIVE_EDITOR) - g1 <= 1, "a held preview's request is stable");
    }

    /// The UI-style field kinds: each lands in the page as valid CSS, clamped, with its fallbacks.
    #[test]
    fn font_select_toggle_range_shadow_and_outline_fields_become_css() {
        let src = r#"<html><head><title>t</title>
<meta name="se:field" content="font | font | Segoe UI">
<meta name="se:field" content="corner | select | rounded | sharp=0px, rounded=12px, pill=999px">
<meta name="se:field" content="glow | toggle | on | 0 0 24px #fff | none">
<meta name="se:field" content="radius | radius | 12">
<meta name="se:field" content="pad | range | 20 | 0 | 80 | 2 | px">
<meta name="se:field" content="gap | number | 1.5 | 0 | 4 | em">
<meta name="se:field" content="shadow | shadow | 20 #00000080">
<meta name="se:field" content="line | outline | 0 #ffffff">
</head><body style="font-family:{{font}};border-radius:{{corner}};box-shadow:{{glow}};--r:{{radius}};padding:{{pad}};gap:{{gap}};filter:{{shadow}};outline:{{line}}"></body></html>"#;
        let tpl = parse(src);
        assert_eq!(tpl.fields.len(), 8);
        assert!(lint(src).iter().all(|l| l.starts_with("missing")), "{:?}", lint(src));
        let v = |k: &str, val: &str| substitute(src, &tpl, &[(k.into(), val.into())]);
        let base = substitute(src, &tpl, &[]);
        assert!(base.contains("font-family:'Segoe UI', 'Segoe UI', Arial, sans-serif"), "{base}");
        assert!(base.contains("border-radius:12px") && base.contains("box-shadow:0 0 24px #fff"));
        assert!(base.contains("--r:12px") && base.contains("padding:20px") && base.contains("gap:1.5em"));
        assert!(base.contains("filter:0 10px 20px #00000080") && base.contains("outline:none"));
        assert!(v("font", "Komika Axis").contains("'Komika Axis', 'Segoe UI'"));
        assert!(!v("font", "x;}</style><b>").contains("</style>"), "no way out of the declaration");
        assert!(v("corner", "Pill").contains("border-radius:999px") && v("corner", "bogus").contains("border-radius:12px"));
        assert!(v("glow", "off").contains("box-shadow:none"));
        assert!(v("pad", "500").contains("padding:80px") && v("radius", "-3").contains("--r:0px"), "clamped");
        assert!(v("line", "2 #ff0000").contains("outline:2px solid #ff0000"));
        // a select without options and a bad toggle are reported
        let bad = src.replace("sharp=0px, rounded=12px, pill=999px", "").replace("| on |", "| maybe |");
        let l = lint(&bad);
        assert!(l.iter().any(|m| m.contains("needs options")) && l.iter().any(|m| m.contains("on or off")), "{l:?}");
        // an app font gets an @font-face (Windows does not have it), an installed one does not
        set_app_fonts(vec![("Komika Axis".into(), PathBuf::from("C:/fonts/komika.ttf"))]);
        let (_, page) = prepare(src, &[("font".into(), "Komika Axis".into())]);
        assert!(page.contains(r#"@font-face{font-family:"Komika Axis";src:url("file:///C:/fonts/komika.ttf")}"#), "{page}");
        assert!(!prepare(src, &[]).1.contains("@font-face"));
        set_app_fonts(Vec::new());
        // a range kind is a param (animatable): a CSS variable with its slider bounds
        let p = parse(&src.replace("se:field\" content=\"pad", "se:param\" content=\"pad"));
        assert_eq!(p.params.iter().map(|f| (f.name.as_str(), f.bounds())).collect::<Vec<_>>(), [("pad", (Some(0.0), Some(80.0)))]);
    }

    #[test]
    fn frames_between_active_phases_are_not_rendered_again() {
        // an intro 100-650 ms, an outro from 5200 ms
        let p = [(100.0, 650.0), (5200.0, 5800.0)];
        assert!(!moves_between(&p, 0.0, 50.0), "before the intro starts");
        assert!(moves_between(&p, 50.0, 100.0), "the intro's first moment (a fill-less start jumps)");
        assert!(moves_between(&p, 600.0, 616.0) && moves_between(&p, 640.0, 656.0), "inside, and across its end");
        assert!(!moves_between(&p, 650.0, 5000.0), "the hold");
        assert!(moves_between(&p, 5190.0, 5210.0), "the outro");
        assert!(!moves_between(&p, 5800.0, 6000.0), "after everything");
        assert!(moves_between(&[(0.0, 1e15)], 1e6, 1e6 + 16.0), "an infinite one always moves");
    }

    /// Every library template against an older copy of itself (folder in env SE_OLD_TEMPLATES): the
    /// same picture at three moments with every field at its default (a new field must not change the
    /// look). `SE_OLD_TEMPLATES=… cargo test library_templates_look_unchanged -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn library_templates_look_unchanged() {
        if tools_missing() {
            return;
        }
        let old = PathBuf::from(std::env::var("SE_OLD_TEMPLATES").expect("SE_OLD_TEMPLATES"));
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("library/html");
        let mut bad = Vec::new();
        let mut names: Vec<PathBuf> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names.into_iter().filter(|p| p.extension().is_some_and(|e| e == "html")) {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let (a, b) = (std::fs::read_to_string(old.join(&name)).unwrap(), std::fs::read_to_string(&p).unwrap());
            if a == b {
                continue;
            }
            let tpl = parse(&b);
            let scale = 480.0 / tpl.size.0.max(tpl.size.1) as f64;
            for t in [tpl.duration * 0.2, tpl.thumb_at(), tpl.duration * 0.9] {
                let (fa, fb) = (render_frame_in(&a, &[], scale, t, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).unwrap(), render_frame_in(&b, &[], scale, t, &View { renderer: HtmlRenderer::Edge, ..Default::default() }).unwrap());
                // anti-aliasing differs by a level or two between any two loads: count real changes
                let diff = fa.rgba.iter().zip(&fb.rgba).filter(|(x, y)| x.abs_diff(**y) > 8).count();
                eprintln!("{name} at {t:.2}: {diff} bytes differ");
                if diff > fa.rgba.len() / 1000 {
                    bad.push(format!("{name} at {t:.2}: {diff}"));
                }
            }
            assert!(lint(&b).is_empty(), "{name}: {:?}", lint(&b));
        }
        shutdown();
        assert!(bad.is_empty(), "looks changed:\n{}", bad.join("\n"));
    }

    /// A long, always-moving layer (ticker, progress bar, spinner) rendered whole: wall time and ms a
    /// frame. Env: SE_BENCH_SECS (60), SE_BENCH_FPS (60), SE_BENCH_SIZE (1920x1080), SE_HTML_WORKERS.
    /// `cargo test bench_html_long -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_html_long() {
        if tools_missing() {
            return;
        }
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let secs: f64 = env("SE_BENCH_SECS", "60").parse().unwrap();
        let fps: f64 = env("SE_BENCH_FPS", "60").parse().unwrap();
        let size = env("SE_BENCH_SIZE", "1920x1080");
        let page = format!(
            "<html><head><title>long</title><meta name=\"se:size\" content=\"{size}\"><meta name=\"se:duration\" content=\"{secs}s\">\
             <style>html,body{{margin:0;width:100vw;height:100vh;overflow:hidden;background:transparent;font-family:'Segoe UI',Arial}}\
             .t{{position:absolute;bottom:8vh;left:0;white-space:nowrap;font-size:64px;font-weight:800;color:#fff;\
             background:rgba(10,12,20,.8);padding:12px 24px;animation:tick {secs}s linear both}}\
             .p{{position:absolute;bottom:4vh;left:5vw;height:16px;width:90vw;background:#333}}.p i{{display:block;height:100%;\
             background:#f59e0b;transform-origin:0 50%;animation:grow {secs}s linear both}}\
             .r{{position:absolute;top:10vh;left:10vw;width:200px;height:200px;border-radius:50%;border:20px solid #3d7bfd;\
             border-top-color:transparent;animation:spin 1s linear infinite}}\
             @keyframes tick{{from{{transform:translateX(100vw)}}to{{transform:translateX(-300%)}}}}\
             @keyframes grow{{from{{transform:scaleX(0)}}to{{transform:scaleX(1)}}}}@keyframes spin{{to{{transform:rotate(360deg)}}}}</style></head>\
             <body><div class=\"r\"></div><div class=\"t\">BREAKING · a sixty second ticker that never stops moving · BREAKING · more news</div>\
             <div class=\"p\"><i></i></div></body></html>"
        );
        // SE_BENCH_FOCUS: the second the playhead is at (10 s around it are rendered first)
        let focus = std::env::var("SE_BENCH_FOCUS").ok().and_then(|v| v.parse::<f64>().ok()).map(|t| (t - 5.0, t + 5.0));
        let spec = Spec { html: page, fields: Vec::new(), scale: 1.0, fps, duration: secs, base: String::new(), params: Vec::new(), width: 0, mouse: None, focus, renderer: HtmlRenderer::Edge };
        let (b, _) = out_base(&spec);
        let stem = b.file_name().unwrap().to_string_lossy().into_owned();
        for f in render_files(&stem) {
            let _ = std::fs::remove_file(f);
        }
        let t = Instant::now();
        let s2 = spec.clone();
        let h = std::thread::spawn(move || render_to_file(&s2, &Progress::new()));
        while interim(&spec).is_none() && !h.is_finished() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let still = t.elapsed().as_secs_f64();
        while !interim(&spec).is_some_and(|r| r.animated) && !h.is_finished() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let draft = t.elapsed().as_secs_f64();
        while !interim(&spec).is_some_and(|r| r.path.to_string_lossy().contains(".window.")) && !h.is_finished() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let window = t.elapsed().as_secs_f64();
        let r = h.join().unwrap().expect("render");
        let total = t.elapsed().as_secs_f64();
        let n = (secs * fps).ceil();
        eprintln!(
            "LONG {size} {secs}s @{fps}fps workers={}: still {still:.2} s, playable draft {draft:.1} s, full rate around the playhead {window:.1} s, whole {total:.1} s = {:.1} ms a frame, {:.0} frames/s",
            env("SE_HTML_WORKERS", "auto"),
            total * 1000.0 / n,
            n / total
        );
        eprintln!("{}: {:.1} MB", r.path.display(), std::fs::metadata(&r.path).map_or(0, |m| m.len()) as f64 / 1e6);
        for f in render_files(&stem) {
            let _ = std::fs::remove_file(f);
        }
        shutdown();
    }

    /// The encoder args: opaque = plain H.264, transparency = the stacked colour-over-alpha layout the
    /// alpha proxies use; a GPU encoder by name, libx264 otherwise; CFR, short GOP, no B-frames, mp4.
    #[test]
    fn encode_args_opaque_plain_alpha_stacked() {
        let j = |enc, alpha| encode_args(enc, alpha, (1920, 1080), 59.94).join(" ");
        let opaque = j(None, false);
        assert!(opaque.contains("-vf scale=1920:1080,format=yuv420p") && !opaque.contains("vstack"), "{opaque}");
        assert!(opaque.contains("-c:v libx264") && opaque.contains("-crf 18"), "{opaque}");
        assert!(
            opaque.contains("-r 59940/1000") && opaque.contains("-g 30 -bf 0") && opaque.ends_with("-f mp4"),
            "{opaque}"
        );
        let alpha = j(None, true);
        assert!(alpha.contains(&crate::media::proxy::stacked_filter_to("1920:1080")), "{alpha}");
        assert!(alpha.contains("premultiply") && alpha.contains("alphaextract") && alpha.contains("vstack"), "{alpha}");
        assert!(
            j(Some("h264_nvenc"), true).contains("-c:v h264_nvenc") && !j(Some("h264_nvenc"), true).contains("libx264")
        );
        assert!(j(Some("h264_qsv"), false).contains("-c:v h264_qsv"));
        assert!(j(Some("h264_amf"), false).contains("-c:v h264_amf"));
        assert!(j(Some("hevc_nvenc"), false).contains("-c:v libx264"), "not an H.264 GPU encoder: software");
        // the file names follow the layout, and stacked ones decode as stacked
        let b = dir().join("0123456789abcdef");
        assert!(anim_path(&b, "", false).to_string_lossy().ends_with("0123456789abcdef.mp4"));
        let st = anim_path(&b, ".draft", true);
        assert!(crate::media::proxy::is_stacked(&st) && is_interim(&st) && !is_interim(&anim_path(&b, "", true)));
        assert_eq!(even((1001, 241)), (1002, 242));
        // opaque PNG detection: RGB at once, RGBA by its pixels
        let mut f = Frame::new(4, 4);
        f.rgba.iter_mut().for_each(|v| *v = 255);
        assert!(super::opaque(&crate::mcp::png_encode(&f)));
        f.rgba[3] = 254;
        assert!(!super::opaque(&crate::mcp::png_encode(&f)));
    }

    /// The size goal: a 5 s 1080p60 animated page is a few MB (it was ~150 MB as PNG-in-MOV), opaque as
    /// plain H.264, see-through as stacked H.264 whose alpha decodes back within a few levels.
    /// Prints both sizes (`--nocapture`).
    #[test]
    fn a_5s_1080p60_page_renders_small_with_and_without_alpha() {
        if tools_missing() {
            return;
        }
        let opaque_page = r#"<html><head><title>s</title><meta name="se:size" content="1920x1080"><meta name="se:duration" content="5s">
            <style>html,body{margin:0;overflow:hidden}body{width:100vw;height:100vh;display:grid;place-items:center;font-family:'Segoe UI',Arial;
            background:linear-gradient(120deg,#1b2a6b,#7a1f5c);background-size:200% 200%;animation:d 5s ease-in-out both}
            h1{margin:0;color:#fff;font-size:140px;animation:r 1s ease-out .2s both}
            @keyframes d{from{background-position:0% 50%}to{background-position:100% 50%}}
            @keyframes r{from{opacity:0;transform:translateY(40px)}to{opacity:1;transform:none}}</style></head><body><h1>Chapter One</h1></body></html>"#;
        let alpha_page = r#"<html><head><title>a</title><meta name="se:size" content="1920x1080"><meta name="se:duration" content="5s">
            <style>html,body{margin:0;background:transparent;overflow:hidden;font-family:'Segoe UI',Arial}
            .b{position:absolute;left:80px;bottom:120px;padding:16px 32px;background:rgba(12,14,20,.5);border-left:12px solid #3d7bfd;
            animation:s 5s linear both}h1{margin:0;color:#fff;font-size:64px}
            @keyframes s{from{transform:translateX(-200px)}to{transform:translateX(600px)}}</style></head>
            <body><div class="b"><h1>Your Title</h1></div></body></html>"#;
        let spec = |html: &str| Spec {
            html: html.into(),
            fields: Vec::new(),
            scale: 1.0,
            fps: 60.0,
            duration: 5.0,
            base: String::new(),
            params: Vec::new(),
            width: 0,
            mouse: None,
            focus: None,
            renderer: HtmlRenderer::Auto,
        };
        for (name, page, stacked) in [("opaque", opaque_page, false), ("alpha", alpha_page, true)] {
            let sp = spec(page);
            let stem = out_base(&sp).0.file_name().unwrap().to_string_lossy().into_owned();
            render_files(&stem).into_iter().for_each(|f| {
                let _ = std::fs::remove_file(f);
            });
            let t0 = Instant::now();
            let r = render_to_file(&sp, &Progress::new()).expect("render");
            let mb = std::fs::metadata(&r.path).unwrap().len() as f64 / 1e6;
            eprintln!("SIZE {name}: {} {:.2} MB in {:.1} s", r.path.display(), mb, t0.elapsed().as_secs_f64());
            assert!(r.animated && (r.width, r.height) == (1920, 1080));
            assert_eq!(crate::media::proxy::is_stacked(&r.path), stacked, "{name}: {}", r.path.display());
            assert!(mb < 15.0, "{name}: {mb:.1} MB");
            assert!(
                interim_files(&out_base(&sp).0)
                    .iter()
                    .all(|f| f.extension().is_some_and(|e| e == "png") || !f.exists()),
                "interims gone"
            );
            let mut v = crate::media::open_video(&r.path.to_string_lossy(), crate::media::Backend::Auto).expect("open");
            let mut f = Frame::default();
            assert!(v.frame_at(2.5, 1920, 1080, &mut f), "decode");
            let px =
                |x: u32, y: u32| -> [u8; 4] { f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap() };
            if stacked {
                assert_eq!(px(10, 10)[3], 0, "transparent corner");
                // the bar at 2.5 s: translateX(200px) from left 80 -> x 280..; its 50% black is alpha 128
                let bar = px(300, 1080 - 125);
                assert!(bar[3].abs_diff(128) <= 4 && bar[0] < 40, "half see-through bar within a few levels: {bar:?}");
            } else {
                assert_eq!(px(10, 10)[3], 255, "opaque");
            }
            drop(v);
            // SE_KEEP_RENDERS=1 keeps them (to compare with other formats by hand)
            render_files(&stem).into_iter().filter(|_| std::env::var_os("SE_KEEP_RENDERS").is_none()).for_each(|f| {
                let _ = std::fs::remove_file(f);
            });
        }
        shutdown();
    }

    /// The frames of a parallel render by the fast renderer are those of a single worker, byte for byte, in
    /// order (every worker is a process of its own, taking pieces of the frame list).
    #[test]
    fn fast_parallel_render_equals_one_worker() {
        if blitz::helper_exe().is_none() {
            eprintln!("se-html not built: test skipped (set SE_HTML_HELPER)");
            return;
        }
        let src = r#"<html><head><meta name="se:size" content="160x90"><style>html,body{margin:0;background:transparent}
            .a{position:absolute;top:10px;width:30px;height:30px;background:#e33;animation:m 2s linear infinite alternate}
            @keyframes m{from{left:0}to{left:120px}}</style></head><body><div class="a"></div></body></html>"#;
        let (tpl, page) = prepare_in(src, &[], "");
        let (frames, fps) = (70usize, 20.0);
        let run = |n: usize| -> Vec<u8> {
            let job = Job { page: &page, base: "", fast: true, vp: (160, 90), scale: 1.0, fps, frames, tpl: &tpl, params: &[], phases: &[(0.0, f64::INFINITY)], moving: true, enc: None, alpha: true, size: (160, 90) };
            let mut out = Vec::new();
            render_parallel(&job, n, &mut out, None, None, &Progress::new()).expect("render");
            out
        };
        let one = run(1);
        let three = run(3);
        assert!(!one.is_empty() && one == three, "{} vs {} bytes", one.len(), three.len());
    }

}
