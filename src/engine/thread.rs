//! ---- ws:thread-viewers ----
//! Fake chat and forum threads as DATA. A thread (cast, messages or comments, look, animation) is a
//! plain struct that generates an HTML + CSS layer (`html()`); the struct itself rides inside the page as
//! `<meta name="se:thread" content="{json}">`, so it survives the project file, Save to Gallery, the
//! library and `html.get` with nothing else to store. Everything the page does over time is CSS
//! (`@keyframes` + `animation-delay` computed from the data, no script), so a frame at second t is exact.
//!
//! The pieces: the data and its edits (add / remove / move / duplicate, the cast), light markup
//! (`**bold**` ... `||spoiler||`), timestamps, a plain-lines script (`name (4:32 PM): text`) both ways,
//! the animation schedule and the page. The look is `library/thread/*.css` (compiled in).

use crate::engine::html::escape;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `<meta name>` the data rides in.
pub const META: &str = "se:thread";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Chat,
    Forum,
    /// A video site's comment section.
    Comments,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Chat => "Chat Thread",
            Kind::Forum => "Forum Thread",
            Kind::Comments => "Video Comments",
        }
    }
}

/// (name, kind, layout, theme): the look presets, what `thread.create style` and the Inspector offer.
pub const PRESETS: &[(&str, Kind, &str, &str)] = &[
    ("Dark chat", Kind::Chat, "cozy", "dark"),
    ("Light chat", Kind::Chat, "cozy", "light"),
    ("Compact chat", Kind::Chat, "compact", "dark"),
    ("Classic forum", Kind::Forum, "classic", "light"),
    ("Card forum", Kind::Forum, "card", "light"),
    ("Dark forum", Kind::Forum, "classic", "dark"),
    ("Light comments", Kind::Comments, "default", "light"),
    ("Dark comments", Kind::Comments, "default", "dark"),
    ("Compact comments", Kind::Comments, "compact", "dark"),
];

/// The preset called `name` (any case, `_` or `-` for a space).
pub fn preset(name: &str) -> Option<&'static (&'static str, Kind, &'static str, &'static str)> {
    let key = name.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    PRESETS.iter().find(|p| p.0.to_ascii_lowercase() == key)
}

const NAME_COLOURS: [&str; 8] = ["#e5534b", "#d97706", "#3fa34d", "#1f9fb4", "#4f6bed", "#8b5cf6", "#d0489a", "#6b7280"];

/// A stable colour for a name without one of its own.
pub fn colour_of(name: &str) -> &'static str {
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    NAME_COLOURS[(h % 8) as usize]
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &u8) -> bool {
    *n == 0
}
fn is_zero32(n: &u32) -> bool {
    *n == 0
}
fn is_zero64(n: &i64) -> bool {
    *n == 0
}

/// One person of the cast: items refer to them by `name`; change them here and every message follows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Member {
    pub name: String,
    /// A project image (its file name) or a file path; empty = a coloured circle with the initial.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub avatar: String,
    /// The name's colour (and the circle's); `#rrggbb`.
    pub color: String,
    /// BOT, verified (a check), creator, or any short text; empty = none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub badge: String,
    /// A DM header's status line under the name ("Online").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Reaction {
    pub emoji: String,
    pub count: u32,
    /// "me" reacted too: drawn highlighted.
    #[serde(skip_serializing_if = "is_false")]
    pub me: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Embed {
    pub title: String,
    pub desc: String,
    /// The colour bar.
    pub color: String,
    /// A small picture on the right.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub thumb: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    /// A message (chat) or a comment (forum).
    #[default]
    Msg,
    /// "alice joined the server": `who` in bold, then `text`.
    System,
    /// A line with a label ("November 13, 2026").
    Divider,
    /// The red "NEW" line.
    Unread,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Item {
    pub id: u32,
    #[serde(skip_serializing_if = "is_msg")]
    pub kind: ItemKind,
    pub who: String,
    pub text: String,
    /// Chat: the day (`2026-11-13`) and the time (`4:32 PM`); either may stay empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub time: String,
    /// Forum: how old ("3 hr. ago").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub age: String,
    /// Chat: the id of the message this answers (a quoted preview above it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<u32>,
    /// Project images (file names) or paths.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed: Option<Embed>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    #[serde(skip_serializing_if = "is_false")]
    pub edited: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// Forum: the vote count.
    #[serde(skip_serializing_if = "is_zero64")]
    pub score: i64,
    /// Forum: award emoji.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub awards: Vec<String>,
    /// Forum: the post's author wrote this (an OP badge).
    #[serde(skip_serializing_if = "is_false")]
    pub op: bool,
    /// Forum: nesting (0 = top level).
    #[serde(skip_serializing_if = "is_zero")]
    pub depth: u8,
    /// Forum: its replies are folded away.
    #[serde(skip_serializing_if = "is_false")]
    pub collapsed: bool,
    /// Forum: a "N more replies" stub under it.
    #[serde(skip_serializing_if = "is_zero32")]
    pub more: u32,
    /// Seconds after the item before it (default: the thread's gap).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    /// Chat: seconds of "is typing..." before it (default: the thread's; 0 = none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typing: Option<f64>,
    /// A highlight pulse this many seconds after it appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pulse: Option<f64>,
    /// Typed out in the message box (chat) / comment box (video comments) before it is sent. Default:
    /// chat messages from `me` when the animation types them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typed: Option<bool>,
    /// Typing speed of this item, characters per second (default: the thread's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cps: Option<f64>,
    /// Seconds between the last key and Send (default: the thread's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause: Option<f64>,
    /// Already there when the clip starts (no animation), whatever its place in the list.
    #[serde(skip_serializing_if = "is_false")]
    pub there: bool,
    /// Video comments: the creator gave it a heart.
    #[serde(skip_serializing_if = "is_false")]
    pub hearted: bool,
}

fn is_msg(k: &ItemKind) -> bool {
    *k == ItemKind::Msg
}

/// The forum post.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Post {
    pub title: String,
    pub who: String,
    pub flair: String,
    pub flair_color: String,
    pub score: i64,
    /// The comment count shown; 0 = the number of comments.
    pub comments: u32,
    pub awards: Vec<String>,
    pub age: String,
    pub body: String,
    pub image: String,
    /// A link preview line ("example.com").
    pub link: String,
    pub thumb: String,
    /// "Best", "Top", "New"...
    pub sort: String,
}

/// How the thread plays: items arrive one after another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Anim {
    pub on: bool,
    /// Seconds before the first item.
    pub start: f64,
    /// Seconds between items.
    pub gap: f64,
    /// Chat: "is typing..." seconds before each message (0 = none).
    pub typing: f64,
    /// pop, slide, fade or cut.
    pub effect: String,
    /// Keep the newest item in view (the older ones scroll away).
    pub autoscroll: bool,
    /// Vote counts count up.
    pub tick: bool,
    /// The first N items are there from the start (history).
    pub shown: u32,
    /// Seconds the finished thread stays.
    pub hold: f64,
    /// `me`'s messages are typed out in the message box, then sent (the others' show "is typing...").
    pub compose: bool,
    /// Typing speed, characters per second.
    pub cps: f64,
    /// Typos: a wrong key now and then, deleted with backspace.
    pub typos: bool,
    /// Seconds between the last key and Send.
    pub pause: f64,
}

impl Default for Anim {
    fn default() -> Self {
        Anim { on: true, start: 0.6, gap: 1.4, typing: 0.0, effect: "pop".into(), autoscroll: true, tick: true, shown: 0, hold: 2.0, compose: false, cps: 14.0, typos: false, pause: 0.5 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Thread {
    pub kind: Kind,
    /// cozy / compact (chat), classic / card (forum).
    pub layout: String,
    /// dark or light.
    pub theme: String,
    pub accent: String,
    /// A font family (installed or one of the app's fonts).
    pub font: String,
    pub radius: u32,
    /// compact, normal or roomy.
    pub density: String,
    /// Text size in px (against a 1080-high frame).
    pub size: u32,
    pub width: u32,
    pub height: u32,
    /// Chat: the channel; forum: the community.
    pub title: String,
    pub topic: String,
    /// Chat: the server's name.
    pub server: String,
    /// The server / community picture.
    pub icon: String,
    /// today, time, 24h, full or relative.
    pub time_format: String,
    /// "now" for Today / Yesterday / relative ("2026-11-13 4:40 PM"); empty = the last message's moment.
    pub now: String,
    /// Chat: consecutive messages from one person share a header.
    pub group: bool,
    /// Chat: the server rail on the left.
    pub rail: bool,
    /// Chat: the member list on the right.
    pub sidebar: bool,
    /// Chat: the channel bar on top.
    pub header: bool,
    /// Chat: the message box at the bottom (video comments: the comment box).
    pub composer: bool,
    /// Chat: server channel (`server`), a two-person DM (`dm`) or a group DM (`group`).
    pub mode: String,
    /// The person at this screen: their messages are the ones typed in the message box.
    pub me: String,
    /// Chat: the channel list (server) or conversation list (DMs) beside the messages.
    pub channels: bool,
    /// Chat: the bar with the search box on top.
    pub topbar: bool,
    /// Chat: the "X is typing..." line above the message box.
    pub indicator: bool,
    /// Chat: date dividers show.
    pub dividers: bool,
    /// Chat: the red NEW line shows.
    pub unread_line: bool,
    /// Chat: a scrollbar on the right of the messages.
    pub scrollbar: bool,
    /// No window background: just the messages (over video).
    pub transparent: bool,
    pub cast: Vec<Member>,
    pub items: Vec<Item>,
    pub post: Post,
    pub anim: Anim,
}

impl Default for Thread {
    fn default() -> Self {
        Thread {
            kind: Kind::Chat,
            layout: "cozy".into(),
            theme: "dark".into(),
            accent: "#5b6cf0".into(),
            font: "Segoe UI".into(),
            radius: 12,
            density: "normal".into(),
            size: 28,
            width: 1160,
            height: 940,
            title: "general".into(),
            topic: String::new(),
            server: String::new(),
            icon: String::new(),
            time_format: "today".into(),
            now: String::new(),
            group: true,
            rail: false,
            sidebar: false,
            header: false,
            composer: false,
            mode: "server".into(),
            me: String::new(),
            channels: false,
            topbar: false,
            indicator: false,
            dividers: false,
            unread_line: false,
            scrollbar: false,
            transparent: false,
            cast: Vec::new(),
            items: Vec::new(),
            post: Post::default(),
            anim: Anim::default(),
        }
    }
}

/// When an item shows up.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Slot {
    pub appear: f64,
    /// Seconds of "is typing..." that end at `appear`.
    pub typing: f64,
    /// Present from the start (no animation).
    pub shown: bool,
    /// When its typing starts (indicator or message box); `appear` is when it is sent / shows.
    pub type_start: f64,
    /// Typed in the message box (else `typing` is an "is typing..." indicator).
    pub compose: bool,
    /// The moment Send is pressed (comment box): `appear` minus a little.
    pub press: f64,
}

// ---------------------------------------------------------------------------------------------------
// dates and times

fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let s = s.trim();
    let sep = if s.contains('-') { '-' } else if s.contains('/') { '/' } else { return None };
    let p: Vec<&str> = s.split(sep).collect();
    if p.len() != 3 {
        return None;
    }
    let n: Vec<i32> = p.iter().map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    let (y, m, d) = if sep == '-' || p[0].trim().len() == 4 {
        (n[0], n[1], n[2])
    } else {
        (if n[2] < 100 { 2000 + n[2] } else { n[2] }, n[0], n[1])
    };
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m as u32, d as u32))
}

fn parse_time(s: &str) -> Option<(u32, u32)> {
    let low: String = s.to_ascii_lowercase().split_whitespace().collect();
    let (body, pm) = match (low.strip_suffix("pm"), low.strip_suffix("am")) {
        (Some(b), _) => (b, Some(true)),
        (_, Some(b)) => (b, Some(false)),
        _ => (low.as_str(), None),
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None if pm.is_some() => (body.parse::<u32>().ok()?, 0),
        None => return None,
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    (h < 24 && m < 60).then_some((h, m))
}

/// Days since 1970-01-01 of a civil date.
fn days(d: (i32, u32, u32)) -> i64 {
    let (y, m, d) = (d.0 as i64 - i64::from(d.1 <= 2), d.1 as i64, d.2 as i64);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

fn civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m as u32, d as u32)
}

fn clock12((h, m): (u32, u32)) -> String {
    format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, m, if h < 12 { "AM" } else { "PM" })
}

fn month_name(m: u32) -> &'static str {
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
        [(m.clamp(1, 12) - 1) as usize]
}

/// A formatted count: 950, 1.2k, 12.4k, 1.1m.
pub fn count_text(n: i64) -> String {
    let (neg, v) = (n < 0, n.unsigned_abs() as f64);
    let t = |x: f64, s: &str| {
        let r = format!("{:.1}", (x * 10.0).floor() / 10.0);
        format!("{}{s}", r.trim_end_matches('0').trim_end_matches('.'))
    };
    let s = if v < 1000.0 {
        format!("{}", v as i64)
    } else if v < 1e6 {
        t(v / 1e3, "k")
    } else {
        t(v / 1e6, "m")
    };
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

// ---------------------------------------------------------------------------------------------------
// light markup

const EMOJI: &[(&str, &str)] = &[
    ("smile", "😄"), ("joy", "😂"), ("rofl", "🤣"), ("heart", "❤️"), ("fire", "🔥"), ("thumbsup", "👍"), ("+1", "👍"),
    ("thumbsdown", "👎"), ("-1", "👎"), ("eyes", "👀"), ("skull", "💀"), ("sob", "😭"), ("cry", "😢"), ("pray", "🙏"),
    ("clap", "👏"), ("tada", "🎉"), ("100", "💯"), ("thinking", "🤔"), ("wave", "👋"), ("sweat_smile", "😅"),
    ("angry", "😠"), ("rage", "😡"), ("scream", "😱"), ("grin", "😁"), ("wink", "😉"), ("sunglasses", "😎"),
    ("ok_hand", "👌"), ("muscle", "💪"), ("raised_hands", "🙌"), ("star", "⭐"), ("sparkles", "✨"), ("check", "✅"),
    ("x", "❌"), ("warning", "⚠️"), ("rocket", "🚀"), ("ghost", "👻"), ("party", "🥳"), ("pleading", "🥺"),
    ("unamused", "😒"), ("gem", "💎"), ("crown", "👑"), ("trophy", "🏆"), ("gold", "🥇"), ("silver", "🥈"),
    ("bronze", "🥉"), ("heart_eyes", "😍"), ("upvote", "⬆️"), ("clown", "🤡"), ("cap", "🧢"), ("pensive", "😔"),
    ("shrug", "🤷"), ("facepalm", "🤦"), ("lol", "😆"), ("point_right", "👉"), ("zzz", "💤"),
];

/// `:name:` as the emoji, or the text unchanged.
pub fn emoji(s: &str) -> String {
    let t = s.trim();
    t.strip_prefix(':')
        .and_then(|r| r.strip_suffix(':'))
        .and_then(|k| EMOJI.iter().find(|(n, _)| *n == k.to_ascii_lowercase()))
        .map_or_else(|| t.to_string(), |(_, e)| e.to_string())
}

/// HTML-escape, and keep `://` out of the page's text (the library rules reject network URLs).
fn esc(s: &str) -> String {
    escape(s).replace("://", "&#58;//")
}

/// The first index of `pat` in `s` after position `from`, with something in between.
fn close(s: &str, from: usize, pat: &str) -> Option<usize> {
    s.get(from..)?.find(pat).filter(|&i| i > 0).map(|i| from + i)
}

/// Inline markup: `code`, ||spoiler||, **bold**, __underline__, ~~strike~~, *italic*, _italic_, :emoji:,
/// links, @mentions, #channels.
fn inline(s: &str) -> String {
    inline_x(s, false)
}

/// `inline`, with `m:ss` timestamps made into links (video comments).
fn inline_x(s: &str, ts: bool) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let (mut i, prev_ws) = (0, |s: &str, i: usize| i == 0 || s[..i].chars().next_back().is_some_and(|c| c.is_whitespace() || "([{>\"'".contains(c)));
    while i < s.len() {
        let rest = &s[i..];
        let mut done = false;
        for (open, tag, raw) in [("`", "code", true), ("||", "span class=\"sp\"", false), ("**", "b", false), ("__", "u", false), ("~~", "s", false)] {
            if let Some(r) = rest.strip_prefix(open) {
                if let Some(e) = close(r, 0, open) {
                    let inner = &r[..e];
                    let body = if raw { esc(inner) } else { inline_x(inner, ts) };
                    let name = tag.split(' ').next().unwrap_or(tag);
                    out.push_str(&format!("<{tag}>{body}</{name}>"));
                    i += open.len() * 2 + e;
                    done = true;
                    break;
                }
            }
        }
        if done {
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        match c {
            '*' | '_' if rest.len() > 1 && !rest[1..].starts_with(c) && (c == '*' || prev_ws(s, i)) => {
                if let Some(e) = close(rest, 1, &c.to_string()).filter(|&e| !rest[..e].ends_with(' ')) {
                    out.push_str(&format!("<i>{}</i>", inline_x(&rest[1..e], ts)));
                    i += e + 1;
                    continue;
                }
                out.push(c);
            }
            ':' => {
                let name: String = rest[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || "_+-".contains(*c)).collect();
                match EMOJI.iter().find(|(n, _)| *n == name.to_ascii_lowercase()).filter(|_| !name.is_empty() && rest[1 + name.len()..].starts_with(':')) {
                    Some((_, e)) => {
                        out.push_str(e);
                        i += name.len() + 2;
                        continue;
                    }
                    None => out.push(':'),
                }
            }
            'h' if rest.starts_with("http://") || rest.starts_with("https://") => {
                let url: String = rest.chars().take_while(|c| !c.is_whitespace() && !"<>\"".contains(*c)).collect();
                let url = url.trim_end_matches(['.', ',', ')', '!', '?', ';']);
                out.push_str(&format!("<span class=\"lk\">{}</span>", esc(url)));
                i += url.len();
                continue;
            }
            '@' | '#' if prev_ws(s, i) => {
                let word: String = rest[1..].chars().take_while(|c| c.is_alphanumeric() || "_-.".contains(*c)).collect();
                let word = word.trim_end_matches('.');
                if !word.is_empty() && (c == '@' || word.chars().next().is_some_and(char::is_alphabetic)) {
                    out.push_str(&format!("<span class=\"mn\">{c}{}</span>", esc(word)));
                    i += 1 + word.len();
                    continue;
                }
                out.push(c);
            }
            '0'..='9' if ts && prev_ws(s, i) => {
                let word: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == ':').collect();
                let parts: Vec<&str> = word.split(':').collect();
                if rest[word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric()) && (2..=3).contains(&parts.len()) && !parts[0].is_empty() && parts[0].len() <= 3 && parts[1..].iter().all(|p| p.len() == 2) {
                    out.push_str(&format!("<span class=\"lk\">{word}</span>"));
                    i += word.len();
                    continue;
                }
                out.push(c);
            }
            '\n' => out.push_str("<br>"),
            _ => out.push_str(&esc(&c.to_string())),
        }
        i += c.len_utf8();
    }
    out
}

/// A message body: ``` fenced code, > quotes, and the inline markup in between.
pub fn markup(text: &str) -> String {
    markup_x(text, false)
}

/// `markup`, with `m:ss` timestamps as links.
pub fn markup_x(text: &str, ts: bool) -> String {
    let (mut out, mut para, mut quote): (String, Vec<&str>, Vec<&str>) = (String::new(), Vec::new(), Vec::new());
    let mut code: Option<Vec<&str>> = None;
    fn flush(out: &mut String, para: &mut Vec<&str>, quote: &mut Vec<&str>, ts: bool) {
        if !para.is_empty() {
            out.push_str(&inline_x(&para.join("\n"), ts));
            para.clear();
        }
        if !quote.is_empty() {
            out.push_str(&format!("<blockquote>{}</blockquote>", inline_x(&quote.join("\n"), ts)));
            quote.clear();
        }
    }
    for line in text.split('\n') {
        match (&mut code, line.trim_start().strip_prefix("```")) {
            (Some(c), Some(after)) => {
                out.push_str(&format!("<pre><code>{}</code></pre>", esc(&c.join("\n"))));
                code = None;
                if !after.trim().is_empty() {
                    para.push(after);
                }
            }
            (Some(c), None) => c.push(line),
            (None, Some(lang)) => {
                flush(&mut out, &mut para, &mut quote, ts);
                // ```rust on the opening line names a language: dropped; ```text``` on one line is code
                match lang.strip_suffix("```") {
                    Some(one) => out.push_str(&format!("<pre><code>{}</code></pre>", esc(one))),
                    None => code = Some(Vec::new()),
                }
            }
            (None, None) => match line.strip_prefix("> ").or_else(|| line.strip_prefix(">>> ")) {
                Some(q) => {
                    if !para.is_empty() {
                        flush(&mut out, &mut para, &mut quote, ts);
                    }
                    quote.push(q);
                }
                None => {
                    if !quote.is_empty() {
                        flush(&mut out, &mut para, &mut quote, ts);
                    }
                    para.push(line);
                }
            },
        }
    }
    if let Some(c) = code {
        out.push_str(&format!("<pre><code>{}</code></pre>", esc(&c.join("\n"))));
    }
    flush(&mut out, &mut para, &mut quote, ts);
    out
}

/// The text without its markup characters (a quoted preview).
fn plain(s: &str) -> String {
    let t: String = s.chars().filter(|c| !"*_~`|".contains(*c)).map(|c| if c == '\n' { ' ' } else { c }).collect();
    let t = t.trim();
    if t.chars().count() > 64 {
        format!("{}…", t.chars().take(64).collect::<String>())
    } else {
        t.to_string()
    }
}

// ---------------------------------------------------------------------------------------------------
// the data

fn colour_ok(s: &str) -> bool {
    s.strip_prefix('#').is_some_and(|h| matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn pick_colour(c: &str, default: &str) -> String {
    if colour_ok(c.trim()) { c.trim().to_string() } else { default.to_string() }
}

fn secs(t: f64) -> String {
    let s = format!("{:.3}", t.max(0.0));
    s.trim_end_matches('0').trim_end_matches('.').to_string() + "s"
}

impl Item {
    pub fn msg(who: &str, text: &str) -> Item {
        Item { who: who.into(), text: text.into(), ..Default::default() }
    }
}

impl Thread {
    /// An empty thread of `kind` in the look called `style` (a preset name; unknown = the kind's first).
    /// A chat starts as "messages only": no surrounding window UI.
    pub fn blank(kind: Kind, style: &str) -> Thread {
        let mut t = Thread { kind, ..Default::default() };
        let p = preset(style).filter(|p| p.1 == kind).or_else(|| PRESETS.iter().find(|p| p.1 == kind)).copied();
        match kind {
            Kind::Forum => {
                t.title = "r/ShortStories".into();
                t.accent = "#e8523a".into();
                t.height = 980;
                t.radius = 10;
                t.post = Post { sort: "Best".into(), score: 1, ..Default::default() };
            }
            Kind::Comments => {
                t.title = "Video".into();
                t.accent = "#3ea6ff".into();
                t.height = 980;
                t.radius = 10;
                t.size = 26;
                t.header = true;
                t.composer = true;
                t.post = Post { sort: "Sort by".into(), ..Default::default() };
            }
            Kind::Chat => {}
        }
        if let Some(p) = p {
            t.layout = p.2.into();
            t.theme = p.3.into();
        }
        t
    }

    /// A thread with starter content (3 messages / a post and 3 comments / 4 comments): looks right at once.
    pub fn starter(kind: Kind, style: &str) -> Thread {
        let mut t = Thread::blank(kind, style);
        let person = |t: &mut Thread, n: &str, c: &str| t.upsert(Member { name: n.into(), color: c.into(), ..Default::default() });
        match kind {
            Kind::Chat => {
                t.topic = "talk about anything, be nice".into();
                t.server = "The Squad".into();
                t.now = "2026-11-13 4:40 PM".into();
                person(&mut t, "Maya", "#e5534b");
                person(&mut t, "Jordan", "#3fa34d");
                t.items = vec![
                    Item { who: "Maya".into(), text: "ok so I finally found out **who** has been eating my lunch :eyes:".into(), date: "2026-11-13".into(), time: "4:31 PM".into(), ..Default::default() },
                    Item { who: "Jordan".into(), text: "wait. tell me everything".into(), date: "2026-11-13".into(), time: "4:32 PM".into(), reactions: vec![Reaction { emoji: ":joy:".into(), count: 3, me: true }], ..Default::default() },
                    Item { who: "Maya".into(), text: "it was `dave` the whole time. I have screenshots @Jordan".into(), date: "2026-11-13".into(), time: "4:32 PM".into(), ..Default::default() },
                ];
            }
            Kind::Forum => {
                person(&mut t, "Maya", "#e5534b");
                person(&mut t, "Jordan", "#3fa34d");
                person(&mut t, "dave_the_lunch_thief", "#4f6bed");
                person(&mut t, "throwaway_4821", "#8b5cf6");
                t.post = Post {
                    title: "My roommate keeps eating my lunch, so I finally set a trap".into(),
                    who: "throwaway_4821".into(),
                    flair: "Story".into(),
                    flair_color: "#e8523a".into(),
                    score: 12400,
                    comments: 0,
                    awards: vec![":gold:".into(), ":trophy:".into()],
                    age: "5 hr. ago".into(),
                    body: "Three weeks of missing sandwiches. Yesterday I made one with **way** too much hot sauce.\n\nHe has not left the bathroom since.".into(),
                    sort: "Best".into(),
                    ..Default::default()
                };
                t.items = vec![
                    Item { who: "Maya".into(), text: "Honestly a legend. The sandwich was the *jury* and the sauce was the verdict.".into(), age: "4 hr. ago".into(), score: 4200, awards: vec![":gold:".into()], ..Default::default() },
                    Item { who: "dave_the_lunch_thief".into(), text: "I regret everything.".into(), age: "3 hr. ago".into(), score: 2100, depth: 1, ..Default::default() },
                    Item { who: "Jordan".into(), text: "Label your food and he will still eat it. Respect the trap :fire:".into(), age: "2 hr. ago".into(), score: 987, ..Default::default() },
                ];
            }
            Kind::Comments => {
                t.title = "My roommate keeps eating my lunch (I set a trap)".into();
                t.me = "@You".into();
                t.upsert(Member { name: "@MayaPlays".into(), color: "#e5534b".into(), badge: "creator".into(), ..Default::default() });
                person(&mut t, "@jordan.codes", "#3fa34d");
                person(&mut t, "@dave_watches", "#4f6bed");
                person(&mut t, "@You", "#8b5cf6");
                t.post.who = "@MayaPlays".into();
                t.post.comments = 1284;
                t.items = vec![
                    Item { who: "@jordan.codes".into(), text: "this part at 0:42 got me \u{1F602}\u{1F602} who else rewatched it?".into(), age: "3 days ago".into(), score: 12400, pinned: true, hearted: true, ..Default::default() },
                    Item { who: "@dave_watches".into(), text: "Rewatched it 5 times. no regrets.".into(), age: "2 days ago".into(), score: 321, depth: 1, ..Default::default() },
                    Item { who: "@MayaPlays".into(), text: "thank you all!! :heart_eyes:".into(), age: "1 day ago".into(), score: 1800, depth: 1, ..Default::default() },
                    Item { who: "@dave_watches".into(), text: "The editing is unreal. Subscribed.\nGreat video, **seriously**.".into(), age: "5 hours ago".into(), score: 987, edited: true, ..Default::default() },
                ];
            }
        }
        for i in 0..t.items.len() {
            t.items[i].id = i as u32 + 1;
        }
        t
    }

    /// A DM / group DM / server channel where the person at the screen (`me`) is typed out in the message box
    /// and the others "are typing...": a conversation that plays by itself.
    pub fn chat_typing(mode: &str, style: &str) -> Thread {
        let mut t = Thread::blank(Kind::Chat, style);
        t.mode = match mode.trim().to_ascii_lowercase().as_str() {
            "dm" => "dm",
            "group" => "group",
            _ => "server",
        }
        .into();
        t.me = "Maya".into();
        t.now = "2026-11-13 4:40 PM".into();
        (t.header, t.composer, t.indicator) = (true, true, true);
        t.anim = Anim { compose: true, typing: 0.9, gap: 0.45, pause: 0.4, cps: 16.0, start: 0.4, hold: 1.5, ..Default::default() };
        let people: &[(&str, &str, &str)] = &[("Maya", "#e5534b", ""), ("Jordan", "#3fa34d", "Online"), ("Dave", "#4f6bed", "")];
        for (n, c, st) in people.iter().take(if t.mode == "dm" { 2 } else { 3 }) {
            t.upsert(Member { name: n.to_string(), color: c.to_string(), status: st.to_string(), ..Default::default() });
        }
        let script = match t.mode.as_str() {
            "dm" => "Jordan (2026-11-13 4:31 PM): hey, did you see what dave posted\nMaya: no?? what did he post\nJordan: go look in the server. it's unreal\nMaya: ok I'm looking. WHY IS HE LIKE THIS :skull:",
            "group" => "Jordan (2026-11-13 4:31 PM): who's coming saturday\nDave: me\nMaya: I'm in. what time?\nJordan: 7. don't be late\nMaya: no promises :eyes:",
            _ => "Jordan (2026-11-13 4:31 PM): did anyone watch the new episode\nDave: not yet, no spoilers\nMaya: I just finished it. I have **thoughts**\nJordan: tell me everything\nMaya: ok. first of all. the ending.",
        };
        if t.mode == "server" {
            t.title = "general".into();
            t.server = "The Squad".into();
            t.topic = "talk about anything, be nice".into();
            (t.rail, t.channels) = (true, true);
        } else if t.mode == "group" {
            t.title = "weekend plans".into();
        }
        t.from_script(script, true);
        t
    }

    /// A video comment section where a comment is typed into the box, the Comment button is pressed, and
    /// the comment lands at the top (the creator answers it).
    pub fn comments_typing(style: &str) -> Thread {
        let mut t = Thread::starter(Kind::Comments, style);
        for it in &mut t.items {
            it.there = true;
        }
        t.anim = Anim { compose: true, cps: 15.0, pause: 0.6, start: 0.6, hold: 1.5, ..Default::default() };
        let mine = Item { who: "@You".into(), text: "this is exactly how I felt watching it \u{1F602} 0:42 is peak".into(), age: "Just now".into(), score: 12, typed: Some(true), ..Default::default() };
        let reply = Item { who: "@MayaPlays".into(), text: "haha thank you!! :heart_eyes:".into(), age: "Just now".into(), score: 3, depth: 1, delay: Some(0.7), ..Default::default() };
        t.add(mine, Some(0));
        t.add(reply, Some(1));
        t
    }

    /// The starter called `name` (chat, chat_typing, forum, comments, comments_typing); `mode` is the chat
    /// layout (server, dm, group).
    pub fn recipe(name: &str, mode: &str, style: &str) -> Option<Thread> {
        Some(match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "chat" => {
                let mut t = Thread::starter(Kind::Chat, style);
                if matches!(mode, "dm" | "group" | "server") {
                    t.mode = mode.into();
                }
                t
            }
            "chat_typing" => Thread::chat_typing(mode, style),
            "forum" => Thread::starter(Kind::Forum, style),
            "comments" => Thread::starter(Kind::Comments, style),
            "comments_typing" => Thread::comments_typing(style),
            _ => return None,
        })
    }

    /// Chat: no window UI around the messages at all?
    pub fn messages_only(&self) -> bool {
        !(self.rail || self.channels || self.sidebar || self.topbar || self.header || self.composer || self.indicator || self.dividers || self.unread_line || self.scrollbar)
    }

    /// Chat: switch every piece of surrounding UI off.
    pub fn set_messages_only(&mut self) {
        (self.rail, self.channels, self.sidebar, self.topbar, self.header) = (false, false, false, false, false);
        (self.composer, self.indicator, self.dividers, self.unread_line, self.scrollbar) = (false, false, false, false, false);
    }

    /// Switch the look to preset `name`: layout, theme and the preset's accent / radius.
    pub fn apply_preset(&mut self, name: &str) -> bool {
        let Some(p) = preset(name).filter(|p| p.1 == self.kind) else { return false };
        self.layout = p.2.into();
        self.theme = p.3.into();
        true
    }

    /// The preset this look is (layout + theme), if it is one of them.
    pub fn preset_name(&self) -> Option<&'static str> {
        PRESETS.iter().find(|p| p.1 == self.kind && p.2 == self.layout && p.3 == self.theme).map(|p| p.0)
    }

    /// Repair what a hand-made or older file may get wrong: layout for the kind, depths, ids, colours.
    pub fn normalize(&mut self) {
        let layouts: &[&str] = match self.kind {
            Kind::Chat => &["cozy", "compact"],
            Kind::Forum => &["classic", "card"],
            Kind::Comments => &["default", "compact"],
        };
        if !layouts.contains(&self.layout.as_str()) {
            self.layout = layouts[0].into();
        }
        if !matches!(self.theme.as_str(), "dark" | "light") {
            self.theme = "dark".into();
        }
        if !matches!(self.density.as_str(), "compact" | "normal" | "roomy") {
            self.density = "normal".into();
        }
        self.accent = pick_colour(&self.accent, "#5b6cf0");
        self.width = self.width.clamp(320, 3840) / 2 * 2;
        self.height = self.height.clamp(200, 2160) / 2 * 2;
        self.size = self.size.clamp(10, 80);
        let mut seen = std::collections::HashSet::new();
        let mut next = self.items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let mut prev_depth = None::<u8>;
        for it in &mut self.items {
            if it.id == 0 || !seen.insert(it.id) {
                it.id = next;
                seen.insert(next);
                next += 1;
            }
            it.depth = match self.kind {
                Kind::Forum => it.depth.min(prev_depth.map_or(0, |d| d + 1)).min(8),
                Kind::Comments => it.depth.min(prev_depth.map_or(0, |d| d + 1)).min(1),
                Kind::Chat => 0,
            };
            prev_depth = Some(it.depth);
        }
    }

    pub fn member(&self, who: &str) -> Option<&Member> {
        let w = who.trim();
        self.cast.iter().find(|m| m.name.eq_ignore_ascii_case(w))
    }

    /// Add or update (by name) a cast member; a new one gets a colour.
    pub fn upsert(&mut self, mut m: Member) {
        m.name = m.name.trim().to_string();
        if m.name.is_empty() {
            return;
        }
        m.color = pick_colour(&m.color, colour_of(&m.name));
        match self.cast.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&m.name)) {
            Some(c) => *c = m,
            None => self.cast.push(m),
        }
    }

    /// Rename a member; every item and the post follow.
    pub fn rename_member(&mut self, from: &str, to: &str) -> bool {
        let to = to.trim();
        if to.is_empty() || self.member(from).is_none() || (self.member(to).is_some() && !from.eq_ignore_ascii_case(to)) {
            return false;
        }
        for m in &mut self.cast {
            if m.name.eq_ignore_ascii_case(from) {
                m.name = to.into();
            }
        }
        for it in &mut self.items {
            if it.who.eq_ignore_ascii_case(from) {
                it.who = to.into();
            }
        }
        if self.post.who.eq_ignore_ascii_case(from) {
            self.post.who = to.into();
        }
        true
    }

    /// Remove a member from the cast (their items keep the name and fall back to a coloured circle).
    pub fn remove_member(&mut self, name: &str) -> bool {
        let n = self.cast.len();
        self.cast.retain(|m| !m.name.eq_ignore_ascii_case(name.trim()));
        self.cast.len() != n
    }

    fn next_id(&self) -> u32 {
        self.items.iter().map(|i| i.id).max().unwrap_or(0) + 1
    }

    pub fn index_of(&self, id: u32) -> Option<usize> {
        self.items.iter().position(|i| i.id == id)
    }

    /// The moment after `it`'s: one minute later, same day (a new message follows the one before it).
    fn bump(&self, prev: &Item) -> (String, String) {
        let day = parse_date(&prev.date);
        match parse_time(&prev.time) {
            Some((h, m)) => {
                let mins = h * 60 + m + 1;
                let (date, over) = (prev.date.clone(), mins >= 1440);
                let date = match (day, over) {
                    (Some(d), true) => {
                        let (y, mo, dd) = civil(days(d) + 1);
                        format!("{y:04}-{mo:02}-{dd:02}")
                    }
                    _ => date,
                };
                (date, clock12(((mins / 60) % 24, mins % 60)))
            }
            None => (prev.date.clone(), String::new()),
        }
    }

    /// Add `it` (a new id; its person joins the cast; a chat message without a time follows the one
    /// before it) at index `at` (default: the end). Returns its id.
    pub fn add(&mut self, mut it: Item, at: Option<usize>) -> u32 {
        it.id = self.next_id();
        if !it.who.trim().is_empty() && self.member(&it.who).is_none() {
            let m = Member { name: it.who.trim().into(), ..Default::default() };
            self.upsert(m);
        }
        let at = at.unwrap_or(self.items.len()).min(self.items.len());
        if self.kind == Kind::Chat && it.kind == ItemKind::Msg && it.time.is_empty() {
            if let Some(prev) = self.items[..at].iter().rev().find(|p| p.kind == ItemKind::Msg) {
                (it.date, it.time) = if it.date.is_empty() { self.bump(prev) } else { (it.date.clone(), self.bump(prev).1) };
            }
        }
        if self.kind != Kind::Chat && it.kind == ItemKind::Msg && it.score == 0 {
            it.score = 1;
        }
        let id = it.id;
        self.items.insert(at, it);
        self.normalize();
        id
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let n = self.items.len();
        self.items.retain(|i| i.id != id);
        for i in &mut self.items {
            if i.reply == Some(id) {
                i.reply = None;
            }
        }
        self.items.len() != n
    }

    /// Move to index `to` (clamped).
    pub fn move_to(&mut self, id: u32, to: usize) -> bool {
        let Some(from) = self.index_of(id) else { return false };
        let it = self.items.remove(from);
        self.items.insert(to.min(self.items.len()), it);
        self.normalize();
        true
    }

    pub fn move_by(&mut self, id: u32, delta: i32) -> bool {
        let Some(from) = self.index_of(id) else { return false };
        let to = (from as i32 + delta).clamp(0, self.items.len() as i32 - 1) as usize;
        self.move_to(id, to)
    }

    /// A copy right after the original (new id).
    pub fn duplicate(&mut self, id: u32) -> Option<u32> {
        let at = self.index_of(id)?;
        let mut c = self.items[at].clone();
        c.reply = None;
        let nid = self.next_id();
        c.id = nid;
        self.items.insert(at + 1, c);
        Some(nid)
    }

    /// Where item `i` and its replies end (exclusive): the replies are the deeper items right after it.
    fn tree_end(&self, i: usize) -> usize {
        let d = self.items[i].depth;
        (i + 1..self.items.len()).find(|&k| self.items[k].depth <= d).unwrap_or(self.items.len())
    }

    /// Forum / comments: nest an item and its replies one level deeper / shallower.
    pub fn indent(&mut self, id: u32, delta: i32) -> bool {
        let Some(i) = self.index_of(id) else { return false };
        let end = self.tree_end(i);
        if delta > 0 && i == 0 {
            return true; // nothing above to nest under
        }
        for it in &mut self.items[i..end] {
            it.depth = (it.depth as i32 + delta).clamp(0, 8) as u8;
        }
        self.normalize();
        true
    }

    /// Move an item with its replies so it starts before row `slot` of the list as it is now (`len` = the
    /// end); `depth` nests it there. Dropping it on itself changes nothing.
    pub fn move_tree(&mut self, id: u32, slot: usize, depth: Option<u8>) -> bool {
        let Some(i) = self.index_of(id) else { return false };
        let (d, end) = (self.items[i].depth, self.tree_end(i));
        if (i..=end).contains(&slot) && depth.is_none_or(|x| x == d) {
            return true;
        }
        let block: Vec<Item> = self.items.drain(i..end).collect();
        let to = if slot >= end { slot - block.len() } else { slot }.min(self.items.len());
        let delta = depth.map_or(0, |x| x as i32 - d as i32);
        let block = block.into_iter().map(|mut it| {
            it.depth = (it.depth as i32 + delta).clamp(0, 8) as u8;
            it
        });
        self.items.splice(to..to, block);
        self.normalize();
        true
    }

    /// One step up / down past the neighbour of the same level (with its replies).
    pub fn step_tree(&mut self, id: u32, delta: i32) -> bool {
        let Some(i) = self.index_of(id) else { return false };
        let d = self.items[i].depth;
        let slot = if delta < 0 {
            match (0..i).rev().find(|&k| self.items[k].depth <= d) {
                Some(k) if self.items[k].depth == d => k,
                _ => return false,
            }
        } else {
            let end = self.tree_end(i);
            if end >= self.items.len() || self.items[end].depth != d {
                return false;
            }
            self.tree_end(end)
        };
        self.move_tree(id, slot, None)
    }

    /// Merge a patch (`{field: value}`) into item `id`: unknown fields are an error.
    pub fn patch_item(&mut self, id: u32, patch: &Value) -> Result<(), String> {
        let i = self.index_of(id).ok_or_else(|| format!("no item {id}"))?;
        let mut v = serde_json::to_value(&self.items[i]).map_err(|e| e.to_string())?;
        merge(&mut v, patch);
        v["id"] = id.into();
        let it: Item = serde_json::from_value(v).map_err(|e| e.to_string())?;
        self.items[i] = it;
        if !self.items[i].who.trim().is_empty() && self.member(&self.items[i].who).is_none() {
            let m = Member { name: self.items[i].who.trim().into(), ..Default::default() };
            self.upsert(m);
        }
        self.normalize();
        Ok(())
    }

    /// Merge a patch into the thread itself (not its items / cast, which have their own calls unless
    /// given whole): unknown fields are an error.
    pub fn patch(&mut self, patch: &Value) -> Result<(), String> {
        let mut patch = patch.clone();
        let style = patch.as_object_mut().and_then(|o| o.remove("style"));
        let mut v = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        merge(&mut v, &patch);
        let kind = self.kind;
        *self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if self.kind != kind {
            return Err("kind cannot change: make a new thread".into());
        }
        if let Some(name) = style.as_ref().and_then(Value::as_str) {
            if !self.apply_preset(name) {
                let names: Vec<&str> = PRESETS.iter().filter(|p| p.1 == kind).map(|p| p.0).collect();
                return Err(format!("unknown style '{name}' ({})", names.join(", ")));
            }
        }
        for it in self.items.clone() {
            if !it.who.trim().is_empty() && self.member(&it.who).is_none() {
                self.upsert(Member { name: it.who.trim().into(), ..Default::default() });
            }
        }
        self.normalize();
        Ok(())
    }

    // -----------------------------------------------------------------------------------------------
    // the script: plain lines <-> items

    /// Turn lines into items and add them (`replace`: instead of what is there). Chat: `name: text`,
    /// `name (4:32 PM): text`, `name (11/13/2026 4:32 PM): text`, `--- label` (divider), `* alice joined`
    /// (system line), `!unread`. Forum: `# title`, `## body`, `name (3h, 420): text` with `>` or
    /// indentation for depth. A line that is not `name: text` continues the message before it, and a
    /// `\n` is a line break. Returns how many items it added.
    pub fn from_script(&mut self, text: &str, replace: bool) -> usize {
        if replace {
            self.items.clear();
        }
        let before = self.items.len();
        for raw in text.lines() {
            let line = raw.trim_end();
            if line.trim().is_empty() {
                continue;
            }
            let t = line.trim_start();
            if self.kind != Kind::Chat {
                if let Some(b) = t.strip_prefix("## ") {
                    self.post.body = b.replace("\\n", "\n");
                    continue;
                }
                if let Some(h) = t.strip_prefix("# ") {
                    self.post.title = h.trim().into();
                    continue;
                }
            }
            if self.kind == Kind::Chat {
                if let Some(l) = t.strip_prefix("---") {
                    self.add(Item { kind: ItemKind::Divider, text: l.trim_matches(['-', ' ']).into(), ..Default::default() }, None);
                    continue;
                }
                if t.eq_ignore_ascii_case("!unread") {
                    self.add(Item { kind: ItemKind::Unread, text: "NEW".into(), ..Default::default() }, None);
                    continue;
                }
                if let Some(s) = t.strip_prefix("* ") {
                    let (who, rest) = s.split_once(' ').unwrap_or((s, ""));
                    self.add(Item { kind: ItemKind::System, who: who.into(), text: rest.into(), ..Default::default() }, None);
                    continue;
                }
            }
            // depth: `>` marks, else the indent (2 spaces or a tab per level)
            let (depth, body) = if self.kind != Kind::Chat {
                let marks = t.chars().take_while(|c| *c == '>' || *c == ' ').filter(|c| *c == '>').count();
                if t.starts_with('>') {
                    (marks, t.trim_start_matches(['>', ' ']))
                } else {
                    let ws: usize = line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 2 } else { 1 }).sum();
                    (ws / 2, t)
                }
            } else {
                (0, t)
            };
            match split_speaker(body) {
                Some((who, meta, msg)) => {
                    let mut it = Item { who: who.into(), text: msg.replace("\\n", "\n"), depth: depth.min(8) as u8, ..Default::default() };
                    self.apply_meta(&mut it, meta);
                    self.add(it, None);
                }
                None => match self.items.last_mut().filter(|i| i.kind == ItemKind::Msg) {
                    Some(last) => {
                        last.text.push('\n');
                        last.text.push_str(&t.replace("\\n", "\n"));
                    }
                    None => {
                        self.add(Item { who: "?".into(), text: t.into(), ..Default::default() }, None);
                    }
                },
            }
        }
        self.items.len().saturating_sub(before)
    }

    /// `(4:32 PM)`, `(11/13/2026 4:32 PM)`, `(3h, 420)`: what is inside the brackets of a script line.
    fn apply_meta(&self, it: &mut Item, meta: &str) {
        for part in meta.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if self.kind == Kind::Chat {
                let (date, time): (Vec<&str>, Vec<&str>) = part.split_whitespace().partition(|w| parse_date(w).is_some());
                if let Some(d) = date.first() {
                    it.date = d.to_string();
                }
                if !time.is_empty() {
                    it.time = time.join(" ");
                }
            } else {
                match count_of(part) {
                    Some(n) => it.score = n,
                    None => it.age = part.to_string(),
                }
            }
        }
    }

    /// The items as script lines (what `from_script` reads back).
    pub fn to_script(&self) -> String {
        let mut out = String::new();
        if self.kind != Kind::Chat {
            if !self.post.title.is_empty() {
                out.push_str(&format!("# {}\n", self.post.title));
            }
            if !self.post.body.is_empty() {
                out.push_str(&format!("## {}\n", self.post.body.replace('\n', "\\n")));
            }
        }
        for it in &self.items {
            match it.kind {
                ItemKind::Divider => out.push_str(&format!("--- {}\n", it.text)),
                ItemKind::Unread => out.push_str("!unread\n"),
                ItemKind::System => out.push_str(&format!("* {} {}\n", it.who, it.text)),
                ItemKind::Msg => {
                    let mut meta: Vec<String> = Vec::new();
                    if self.kind == Kind::Chat {
                        let when = format!("{} {}", it.date, it.time).trim().to_string();
                        if !when.is_empty() {
                            meta.push(when);
                        }
                    } else {
                        if !it.age.is_empty() {
                            meta.push(it.age.clone());
                        }
                        meta.push(it.score.to_string());
                    }
                    let meta = if meta.is_empty() { String::new() } else { format!(" ({})", meta.join(", ")) };
                    let pre = ">".repeat(it.depth as usize);
                    let pre = if pre.is_empty() { pre } else { format!("{pre} ") };
                    out.push_str(&format!("{pre}{}{meta}: {}\n", it.who, it.text.replace('\n', "\\n")));
                }
            }
        }
        out
    }

    // -----------------------------------------------------------------------------------------------
    // time

    /// "now" as minutes since the epoch, and the day number it falls on.
    fn now_ref(&self) -> Option<(i64, i64)> {
        let (d, t) = if !self.now.trim().is_empty() {
            let (date, time): (Vec<&str>, Vec<&str>) = self.now.split_whitespace().partition(|w| parse_date(w).is_some());
            (date.first().and_then(|d| parse_date(d)), parse_time(&time.join(" ")))
        } else {
            let last = self.items.iter().rev().find(|i| i.kind == ItemKind::Msg && !i.time.is_empty())?;
            (parse_date(&last.date), parse_time(&last.time))
        };
        let day = d.map_or(0, days);
        let (h, m) = t.unwrap_or((0, 0));
        Some((day * 1440 + (h * 60 + m) as i64, day))
    }

    /// An item's timestamp in the thread's format.
    pub fn stamp(&self, it: &Item) -> String {
        let t = parse_time(&it.time);
        let d = parse_date(&it.date);
        let clock = t.map_or_else(|| it.time.clone(), clock12);
        let now = self.now_ref();
        let ymd = |d: (i32, u32, u32)| format!("{}/{}/{}", d.1, d.2, d.0);
        match self.time_format.as_str() {
            "time" => clock,
            "24h" => t.map_or(clock, |(h, m)| format!("{h:02}:{m:02}")),
            "full" => d.map_or(clock.clone(), |d| format!("{} {clock}", ymd(d))).trim().to_string(),
            "relative" => match (now, t) {
                (Some((n, day)), Some((h, m))) => {
                    let at = d.map_or(day, days) * 1440 + (h * 60 + m) as i64;
                    let diff = (n - at).max(0);
                    let (v, unit) = match diff {
                        0 => return "just now".into(),
                        1..=59 => (diff, "minute"),
                        60..=1439 => (diff / 60, "hour"),
                        _ => (diff / 1440, "day"),
                    };
                    format!("{v} {unit}{} ago", if v == 1 { "" } else { "s" })
                }
                _ => clock,
            },
            _ => {
                let diff = match (now, d) {
                    (Some((_, day)), Some(d)) => day - days(d),
                    _ => 0,
                };
                match diff {
                    0 => format!("Today at {clock}"),
                    1 => format!("Yesterday at {clock}"),
                    _ => d.map_or(clock.clone(), |d| format!("{} {clock}", ymd(d))),
                }
            }
        }
    }

    /// A divider's label from a date: "November 13, 2026".
    pub fn date_label(date: &str) -> String {
        parse_date(date).map_or_else(|| date.to_string(), |(y, m, d)| format!("{} {d}, {y}", month_name(m)))
    }

    // -----------------------------------------------------------------------------------------------
    // animation

    /// Forum: items hidden under a collapsed comment.
    fn hidden(&self) -> Vec<bool> {
        let mut out = vec![false; self.items.len()];
        let mut fold: Option<u8> = None;
        for (i, it) in self.items.iter().enumerate() {
            if self.kind == Kind::Chat {
                break;
            }
            if let Some(d) = fold {
                if it.depth > d {
                    out[i] = true;
                    continue;
                }
                fold = None;
            }
            if it.collapsed {
                fold = Some(it.depth);
            }
        }
        out
    }

    /// Chat: does a divider / unread line show (the thread's toggles)?
    pub fn item_shown(&self, it: &Item) -> bool {
        match (self.kind, it.kind) {
            (Kind::Chat, ItemKind::Divider) => self.dividers,
            (Kind::Chat, ItemKind::Unread) => self.unread_line,
            _ => true,
        }
    }

    /// When every item shows up. An item with typing starts that `gap` seconds after the one before shows;
    /// it shows once the typing (the "is typing..." line, or the keys in the box plus the pause before
    /// Send) is over.
    pub fn schedule(&self) -> Vec<Slot> {
        let (a, hidden) = (&self.anim, self.hidden());
        let (mut t, mut first) = (a.start.max(0.0), true);
        self.items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                if !a.on || (i as u32) < a.shown || it.there || hidden[i] || !self.item_shown(it) {
                    return Slot { shown: true, ..Default::default() };
                }
                let typed = self.is_typed(it);
                let pause = if typed { it.pause.unwrap_or(a.pause).clamp(0.0, 10.0) } else { 0.0 };
                let typing = if typed {
                    self.plan_of(it).1
                } else if self.kind == Kind::Chat && it.kind == ItemKind::Msg && self.indicator {
                    it.typing.unwrap_or(a.typing).clamp(0.0, 30.0)
                } else {
                    0.0
                };
                let delay = it.delay.unwrap_or(if first { 0.0 } else { a.gap }).max(0.0);
                let start = if first { a.start.max(0.0) + delay } else { t + delay };
                t = start + typing + pause;
                first = false;
                Slot { appear: t, typing, shown: false, type_start: start, compose: typed, press: start + typing + pause * 0.4 }
            })
            .collect()
    }

    /// Seconds of the whole thing: the last item arrives, settles, and stays `hold` seconds.
    pub fn duration(&self) -> f64 {
        if !self.anim.on {
            return 4.0;
        }
        let last = self.schedule().iter().filter(|s| !s.shown).map(|s| s.appear).fold(0.0, f64::max);
        (last + 0.5 + self.anim.hold.max(0.0)).max(self.anim.start + 1.5).clamp(1.0, 300.0)
    }

    // -----------------------------------------------------------------------------------------------
    // the page

    fn avatar(&self, who: &str, class: &str) -> String {
        let m = self.member(who);
        let name = m.map_or(who, |m| m.name.as_str());
        let bg = m.map_or_else(|| colour_of(who).to_string(), |m| pick_colour(&m.color, colour_of(who)));
        let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        // the picture is the circle's own background (no <img> in a clipped, rounded box: in the exported render such an
        // avatar was sometimes not painted); without one, the initial on the colour
        let (style, inner) = match m.and_then(|m| img_src(&m.avatar)) {
            Some(src) => (format!("background:{bg} center/cover no-repeat url('{src}')"), String::new()),
            None => (format!("background:{bg}"), format!("<span>{}</span>", esc(&initial))),
        };
        format!("<div class=\"av{class}\" style=\"{style}\">{inner}</div>")
    }

    fn name_html(&self, who: &str) -> String {
        let colour = self.member(who).map(|m| pick_colour(&m.color, colour_of(who)));
        let style = colour.map(|c| format!(" style=\"color:{c}\"")).unwrap_or_default();
        let badge = self.member(who).map(|m| m.badge.trim()).filter(|b| !b.is_empty()).map(|b| {
            let b = if b.eq_ignore_ascii_case("verified") { "✓" } else { b };
            format!("<span class=\"bdg\">{}</span>", esc(b))
        });
        format!("<b class=\"un\"{style}>{}</b>{}", esc(who), badge.unwrap_or_default())
    }

    /// A score as text, or counting up from 1 (a strip of the numbers, stepped through).
    fn score_html(&self, v: i64, start: f64) -> String {
        if !self.anim.on || !self.anim.tick || v <= 1 {
            return count_text(v);
        }
        let n = (v - 1).min(12) as usize;
        let vals: Vec<String> = (0..=n)
            .map(|k| {
                let x = k as f64 / n as f64;
                count_text(1 + ((v - 1) as f64 * (1.0 - (1.0 - x) * (1.0 - x))).round() as i64)
            })
            .collect();
        format!("<span class=\"tk\"><span class=\"tkc\" style=\"--n:{n};--td:1.2s;--ts:{}\">{}</span></span>", secs(start), vals.join("<br>"))
    }

    fn body_html(&self, it: &Item, slot: &Slot) -> String {
        let anim = self.anim.on && !slot.shown;
        let mut out = String::new();
        let mut text = markup_x(&it.text, self.kind == Kind::Comments);
        if it.edited {
            text.push_str("<span class=\"ed\">(edited)</span>");
        }
        if !text.is_empty() {
            out.push_str(&format!("<div class=\"tx\">{text}</div>"));
        }
        if !it.images.is_empty() {
            out.push_str("<div class=\"att\">");
            for im in it.images.iter().filter_map(|i| img_src(i)) {
                out.push_str(&format!("<img src=\"{im}\" alt=\"\" decoding=\"sync\">"));
            }
            out.push_str("</div>");
        }
        if let Some(e) = &it.embed {
            let thumb = img_src(&e.thumb).map(|s| format!("<img src=\"{s}\" alt=\"\" decoding=\"sync\">")).unwrap_or_default();
            out.push_str(&format!(
                "<div class=\"emb\" style=\"--ec:{}\"><div><div class=\"et\">{}</div><div class=\"ed2\">{}</div></div>{thumb}</div>",
                pick_colour(&e.color, &self.accent), esc(&e.title), markup(&e.desc)
            ));
        }
        if !it.reactions.is_empty() {
            out.push_str("<div class=\"rx\">");
            for (k, r) in it.reactions.iter().enumerate() {
                let delay = if anim { format!(" style=\"animation-delay:{}\"", secs(slot.appear + 0.45 + k as f64 * 0.18)) } else { String::new() };
                out.push_str(&format!("<span class=\"r{}\"{delay}>{} <b>{}</b></span>", if r.me { " me" } else { "" }, esc(&emoji(&r.emoji)), r.count));
            }
            out.push_str("</div>");
        }
        out
    }

    /// One wrapper: `.it` (grows in) > `.in` (clipped, pops in) > content. `style`: extra CSS variables.
    fn wrap(&self, class: &str, slot: &Slot, it: &Item, inner: &str, style: &str) -> String {
        let anim = self.anim.on && !slot.shown;
        let a = if anim { " a" } else { "" };
        let d = if anim { format!("--d:{};", secs(slot.appear)) } else { String::new() };
        let style = if style.is_empty() && d.is_empty() { String::new() } else { format!(" style=\"{style}{d}\"") };
        let pulse = match it.pulse {
            Some(p) if self.anim.on => format!("<i class=\"pl\" style=\"animation-delay:{}\"></i>", secs(slot.appear + p.max(0.0))),
            _ => String::new(),
        };
        format!("<div class=\"it {class}{a}\"{style}><div class=\"in\">{pulse}{inner}</div></div>")
    }

    fn chat_item(&self, i: usize, slot: &Slot) -> String {
        let it = &self.items[i];
        match it.kind {
            ItemKind::Divider => {
                let label = if it.text.is_empty() { Thread::date_label(&it.date) } else { it.text.clone() };
                self.wrap("", slot, it, &format!("<div class=\"dv\"><i></i><span>{}</span><i></i></div>", esc(&label)), "")
            }
            ItemKind::Unread => self.wrap("", slot, it, &format!("<div class=\"dv nw\"><i></i><span>{}</span></div>", esc(if it.text.is_empty() { "NEW" } else { &it.text })), ""),
            ItemKind::System => {
                let stamp = if it.time.is_empty() { String::new() } else { format!("<time class=\"pn\">{}</time>", esc(&self.stamp(it))) };
                self.wrap("", slot, it, &format!("<div class=\"sy\"><span class=\"si\">→</span><span><b>{}</b> {}</span>{stamp}</div>", esc(&it.who), esc(&it.text)), "")
            }
            ItemKind::Msg => {
                let compact = self.layout == "compact";
                let prev = i.checked_sub(1).map(|p| &self.items[p]);
                let grouped = self.group
                    && !compact
                    && it.reply.is_none()
                    && prev.is_some_and(|p| p.kind == ItemKind::Msg && p.who.eq_ignore_ascii_case(&it.who) && close_in_time(p, it));
                let reply = it.reply.and_then(|r| self.index_of(r)).map(|r| {
                    let q = &self.items[r];
                    let c = self.member(&q.who).map(|m| pick_colour(&m.color, colour_of(&q.who)));
                    format!(
                        "<div class=\"rp\">{}<b{}>{}</b><span>{}</span></div>",
                        self.avatar(&q.who, ""),
                        c.map(|c| format!(" style=\"color:{c}\"")).unwrap_or_default(),
                        esc(&q.who),
                        inline(&plain(&q.text))
                    )
                });
                let body = self.body_html(it, slot);
                let stamp = esc(&self.stamp(it));
                let row = if compact {
                    format!("<div class=\"row\"><time>{stamp}</time><div class=\"bd\">{}{}</div></div>", self.name_html(&it.who), body)
                } else if grouped {
                    format!("<div class=\"row\"><div class=\"gt\"></div><div class=\"bd\">{body}</div></div>")
                } else {
                    let pin = if it.pinned { "<span class=\"pn\">📌 pinned</span>" } else { "" };
                    format!(
                        "<div class=\"row\">{}<div class=\"bd\"><div class=\"hd\">{}<time>{stamp}</time>{pin}</div>{body}</div></div>",
                        self.avatar(&it.who, ""),
                        self.name_html(&it.who)
                    )
                };
                self.wrap(if grouped { "m" } else { "m f" }, slot, it, &format!("{}{row}", reply.unwrap_or_default()), "")
            }
        }
    }

    fn typing_html(&self, slots: &[Slot]) -> String {
        let mut out = String::new();
        for (it, s) in self.items.iter().zip(slots) {
            if s.typing <= 0.0 || s.shown || s.compose {
                continue;
            }
            let n = (s.typing / 0.9).ceil().max(1.0);
            let dots: String = (0..3).map(|k| format!("<i style=\"animation-delay:{};animation-iteration-count:{n}\"></i>", secs(s.type_start + k as f64 * 0.15))).collect();
            out.push_str(&format!(
                "<div class=\"ty\" style=\"animation-delay:{};animation-duration:{}\"><span class=\"dots\">{dots}</span><span><b>{}</b> is typing…</span></div>",
                secs(s.type_start), secs(s.typing), esc(&it.who)
            ));
        }
        out
    }

    /// The other person of a DM (the first of the cast who is not `me`).
    fn partner(&self) -> Option<&Member> {
        self.cast.iter().find(|m| !m.name.eq_ignore_ascii_case(&self.me)).or(self.cast.first())
    }

    fn chat_header(&self) -> String {
        match self.mode.as_str() {
            "dm" => {
                let p = self.partner().map(|m| m.name.clone()).unwrap_or_default();
                let status = self.member(&p).map(|m| m.status.clone()).filter(|s| !s.is_empty()).unwrap_or_default();
                let st = if status.is_empty() { String::new() } else { format!("<span class=\"st\"><i class=\"dot\"></i>{}</span>", esc(&status)) };
                format!("<div class=\"hd0 dmh\">{}<b>{}</b>{st}</div>", self.avatar(&p, " sm"), esc(&p))
            }
            "group" => {
                let others: Vec<&Member> = self.cast.iter().filter(|m| !m.name.eq_ignore_ascii_case(&self.me)).collect();
                let stack: String = others.iter().take(3).map(|m| self.avatar(&m.name, " sm")).collect();
                let names = if self.title.is_empty() { others.iter().map(|m| m.name.clone()).collect::<Vec<_>>().join(", ") } else { self.title.clone() };
                format!("<div class=\"hd0 dmh\"><span class=\"stack\">{stack}</span><b>{}</b></div>", esc(&names))
            }
            _ => {
                let topic = if self.topic.is_empty() { String::new() } else { format!("<span class=\"sep\"></span><span class=\"topic\">{}</span>", esc(&self.topic)) };
                format!("<div class=\"hd0\"><span class=\"hash\">#</span><b>{}</b>{topic}</div>", esc(&self.title))
            }
        }
    }

    fn chat_list(&self) -> String {
        let mut out = String::from("<div class=\"chs\">");
        if self.mode == "server" {
            out.push_str(&format!("<h3>{}</h3><h4>TEXT CHANNELS</h4>", esc(if self.server.is_empty() { "Server" } else { &self.server })));
            let mut names = vec![self.title.clone()];
            names.extend(["random", "memes", "videos"].iter().map(|s| s.to_string()).filter(|s| *s != self.title));
            for (k, n) in names.iter().enumerate() {
                out.push_str(&format!("<div{}># {}</div>", if k == 0 { " class=\"on\"" } else { "" }, esc(n)));
            }
        } else {
            out.push_str("<h4>DIRECT MESSAGES</h4>");
            let partner = self.partner().map(|m| m.name.clone()).unwrap_or_default();
            for m in &self.cast {
                if m.name.eq_ignore_ascii_case(&self.me) {
                    continue;
                }
                let on = self.mode == "group" || m.name.eq_ignore_ascii_case(&partner);
                out.push_str(&format!("<div{}>{}{}</div>", if on { " class=\"on\"" } else { "" }, self.avatar(&m.name, " sm"), esc(&m.name)));
            }
        }
        out.push_str("</div>");
        out
    }

    fn chat_body(&self) -> String {
        let slots = self.schedule();
        let mut out = String::new();
        if self.rail {
            let initial: String = self.server.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "S".into());
            let icon = img_src(&self.icon).map(|s| format!("<img src=\"{s}\" alt=\"\" decoding=\"sync\">")).unwrap_or_default();
            out.push_str(&format!("<div class=\"rail\"><i class=\"on\"><span>{}</span>{icon}</i><i></i><i></i><i></i></div>", esc(&initial)));
        }
        if self.channels {
            out.push_str(&self.chat_list());
        }
        out.push_str("<div class=\"main\">");
        if self.topbar {
            out.push_str("<div class=\"tb\"><span class=\"srch\">Search</span></div>");
        }
        if self.header {
            out.push_str(&self.chat_header());
        }
        out.push_str(&format!("<div class=\"vp{}\"><div class=\"col\">", if self.anim.autoscroll || !self.anim.on { " end" } else { "" }));
        for i in 0..self.items.len() {
            if self.item_shown(&self.items[i]) {
                out.push_str(&self.chat_item(i, &slots[i]));
            }
        }
        out.push_str("</div>");
        if self.scrollbar {
            out.push_str("<div class=\"sbar\"><i></i></div>");
        }
        out.push_str("</div>");
        if self.composer || self.indicator {
            let ph = match self.mode.as_str() {
                "dm" => format!("Message @{}", self.partner().map(|m| m.name.clone()).unwrap_or_default()),
                "group" => "Message the group".to_string(),
                _ => format!("Message #{}", self.title),
            };
            let tbox: String = self.items.iter().zip(&slots).filter(|(_, s)| s.compose).map(|(it, s)| self.tbox(it, s)).collect();
            let ind = if self.indicator { format!("<div class=\"tyw\">{}</div>", self.typing_html(&slots)) } else { String::new() };
            let cmp = if self.composer { format!("<div class=\"cmp\"><span class=\"ph\">{}</span>{tbox}</div>", esc(&ph)) } else { String::new() };
            out.push_str(&format!("<div class=\"foot\">{ind}{cmp}</div>"));
        }
        out.push_str("</div>");
        if self.sidebar {
            out.push_str(&format!("<div class=\"side\"><h4>MEMBERS — {}</h4>", self.cast.len()));
            for m in &self.cast {
                out.push_str(&format!("<div>{}{}</div>", self.avatar(&m.name, ""), self.name_html(&m.name)));
            }
            out.push_str("</div>");
        }
        out
    }

    fn forum_body(&self) -> String {
        let (slots, hidden) = (self.schedule(), self.hidden());
        let p = &self.post;
        let n_comments = if p.comments > 0 { p.comments as usize } else { self.items.len() };
        let up = "<svg viewBox=\"0 0 16 16\"><path d=\"M8 1.5 15 9.5H10.5V14.5h-5V9.5H1z\"/></svg>";
        let down = up.replacen("<svg", "<svg class=\"dn\"", 1);
        let awards = |a: &[String]| if a.is_empty() { String::new() } else { format!("<span class=\"aw\">{}</span>", a.iter().map(|x| esc(&emoji(x))).collect::<Vec<_>>().join("")) };
        let icon = img_src(&self.icon).map(|s| format!("<img src=\"{s}\" alt=\"\" decoding=\"sync\">")).unwrap_or_default();
        let initial: String = self.title.trim_start_matches(|c: char| !c.is_alphanumeric()).chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "R".into());
        let mut out = format!(
            "<div class=\"top\"><span class=\"ci\"><span>{}</span>{icon}</span><b>{}</b><span class=\"so\">{}</span></div><div class=\"main\"><div class=\"vp{}\"><div class=\"col\">",
            esc(&initial), esc(&self.title), esc(&self.topic), if self.anim.on && self.anim.autoscroll { " end" } else { "" }
        );
        // the post (there from the start)
        let flair = if p.flair.is_empty() { String::new() } else { format!("<span class=\"fl\" style=\"--fc:{}\">{}</span>", pick_colour(&p.flair_color, &self.accent), esc(&p.flair)) };
        let thumb = img_src(&p.thumb).map(|s| format!("<img class=\"th\" src=\"{s}\" alt=\"\" decoding=\"sync\">")).unwrap_or_default();
        let image = img_src(&p.image).map(|s| format!("<div class=\"pimg\"><img src=\"{s}\" alt=\"\" decoding=\"sync\"></div>")).unwrap_or_default();
        let link = if p.link.is_empty() { String::new() } else { format!("<div class=\"pl2\">{} ↗</div>", esc(&p.link)) };
        let body = if p.body.is_empty() { String::new() } else { format!("<div class=\"tx\">{}</div>", markup(&p.body)) };
        let by = if p.who.is_empty() { String::new() } else { format!("<span>Posted by <b>u/{}</b></span><span>•</span>", esc(&p.who)) };
        out.push_str(&format!(
            "<div class=\"post\"><div class=\"vt\">{up}<span>{}</span>{down}</div><div class=\"pb\"><div class=\"meta\"><b>{}</b><span>•</span>{by}<span>{}</span>{}</div><h1>{}{flair}</h1>{body}{image}{link}<div class=\"acts\"><span>💬 {} Comments</span><span>🎁 Award</span><span>↗ Share</span><span>⚑ Save</span></div></div>{thumb}</div>",
            self.score_html(p.score, 0.3),
            esc(&self.title), esc(&p.age), awards(&p.awards), esc(&p.title), count_text(n_comments as i64)
        ));
        out.push_str(&format!("<div class=\"sb\">Sort by: {} ▾</div>", esc(if p.sort.is_empty() { "Best" } else { &p.sort })));
        for (i, it) in self.items.iter().enumerate() {
            if hidden[i] {
                continue;
            }
            let slot = &slots[i];
            let lines: String = (0..it.depth).map(|k| format!("<i class=\"ln\" style=\"--k:{k}\"></i>")).collect();
            let hidden_kids = self.items[i + 1..].iter().zip(&hidden[i + 1..]).take_while(|(_, h)| **h).count();
            let op = if it.op || it.who.eq_ignore_ascii_case(&p.who) && !p.who.is_empty() { "<span class=\"op\">OP</span>" } else { "" };
            let head = format!(
                "<div class=\"ch\">{}<b>{}</b>{op}<span>•</span><span>{}</span>{}</div>",
                self.avatar(&it.who, ""), esc(&it.who), esc(&it.age), awards(&it.awards)
            );
            let tail = if it.collapsed {
                format!("<div class=\"cc\">[+] {hidden_kids} hidden {}</div>", if hidden_kids == 1 { "reply" } else { "replies" })
            } else {
                let more = if it.more > 0 { format!("<div class=\"mr\">{} more {}</div>", it.more, if it.more == 1 { "reply" } else { "replies" }) } else { String::new() };
                format!(
                    "{}<div class=\"ca\"><span class=\"vv\">{up}{}{down}</span><span>Reply</span><span>Share</span></div>{more}",
                    self.body_html(it, slot), self.score_html(it.score, slot.appear + 0.25)
                )
            };
            let inner = format!("<div class=\"bx\" style=\"--dp:{}\">{lines}<div class=\"cb\">{head}{tail}</div></div>", it.depth);
            out.push_str(&self.wrap("cm", slot, it, &inner, &format!("--dp:{};", it.depth)));
        }
        out.push_str("</div></div></div>");
        out
    }

    /// The whole page.
    pub fn html(&self) -> String {
        let mut t = self.clone();
        t.normalize();
        let me = &t;
        let (dur, last) = (me.duration(), me.schedule().iter().filter(|s| !s.shown).map(|s| s.appear).fold(0.0, f64::max));
        let json = serde_json::to_string(me).unwrap_or_default().replace("://", ":\\/\\/");
        let (fx, fd, gd) = match me.anim.effect.as_str() {
            "slide" => ("slide", ".4s", ".3s"),
            "fade" => ("fade", ".4s", ".3s"),
            "cut" => ("fade", ".001s", ".001s"),
            _ => ("pop", ".4s", ".3s"),
        };
        let anim = if me.anim.on { " anim" } else { "" };
        let (css_kind, body) = match me.kind {
            Kind::Chat => (include_str!("../../library/thread/chat.css"), me.chat_body()),
            Kind::Forum => (include_str!("../../library/thread/forum.css"), me.forum_body()),
            Kind::Comments => (include_str!("../../library/thread/comments.css"), me.comments_body()),
        };
        let (name, desc, tags) = match me.kind {
            Kind::Chat => ("Chat Thread", "A fake chat thread: avatars, coloured names, timestamps and messages that arrive one after another.", "chat, thread, messages, discord-style, skit"),
            Kind::Forum => ("Forum Thread", "A fake forum thread: a post with votes, flair and nested comments that arrive one after another.", "forum, thread, comments, post, skit"),
            Kind::Comments => ("Video Comments", "A fake video comment section: a comment typed into the box and posted, likes, replies, hearts and pins.", "comments, video, thread, typing, skit"),
        };
        let wrap_class = match me.kind {
            Kind::Chat => format!("win chat {} c-{} {}{}", me.theme, me.layout, density(&me.density), if me.transparent { " tr" } else { "" }),
            Kind::Comments => format!("win fwin comments {} c-{} {}{}", me.theme, me.layout, density(&me.density), if me.transparent { " tr" } else { "" }),
            Kind::Forum => format!("win fwin forum {} f-{} {}", me.theme, me.layout, density(&me.density)),
        };
        let vars = format!(
            "--accent:{};--r:{}px;--fs:{}px;--fx:{fx};--fd:{fd};--gd:{gd}",
            pick_colour(&me.accent, "#5b6cf0"), me.radius.min(200), me.size
        );
        let mut page = format!(
            "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{name}</title>\n<meta name=\"se:kind\" content=\"layer\">\n<meta name=\"se:desc\" content=\"{desc}\">\n<meta name=\"se:tags\" content=\"{tags}\">\n<meta name=\"se:size\" content=\"{}x{}\">\n<meta name=\"se:duration\" content=\"{:.1}s\">\n<meta name=\"se:thumb\" content=\"{:.1}\">\n<meta name=\"se:field\" content=\"font | font | {}\">\n<meta name=\"se:param\" content=\"scroll | number | 0 | -4000 | 4000\">\n<meta name=\"{META}\" content=\"{}\">\n<style>\n{}\n{}</style>\n</head>\n",
            me.width, me.height, dur.min(60.0), (last + 0.8).min(dur).min(60.0),
            escape(&me.font.replace('|', " ")), escape(&json), include_str!("../../library/thread/base.css"), css_kind
        );
        page.push_str(&format!("<body class=\"{}\" style=\"{vars}\"><div class=\"{wrap_class}\">{body}</div></body>\n</html>\n", anim.trim()));
        page
    }

    /// The thread as JSON text (what `thread.get` and Export show).
    pub fn json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// "420", "+3", "4.2k", "1.1m", "12 pts" as a number.
fn count_of(s: &str) -> Option<i64> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.trim_end_matches("points").trim_end_matches("pts").trim_end_matches("point").trim().trim_start_matches('+');
    let (num, mul) = match s.chars().last()? {
        'k' => (&s[..s.len() - 1], 1e3),
        'm' => (&s[..s.len() - 1], 1e6),
        _ => (s, 1.0),
    };
    num.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| (v * mul).round() as i64)
}

fn density(d: &str) -> &'static str {
    match d {
        "compact" => "d-compact",
        "roomy" => "d-roomy",
        _ => "d-normal",
    }
}

/// Same day and within seven minutes (or no times to compare): messages one person sent in a row.
fn close_in_time(a: &Item, b: &Item) -> bool {
    match (parse_time(&a.time), parse_time(&b.time)) {
        (Some((h1, m1)), Some((h2, m2))) => {
            a.date == b.date && (h2 * 60 + m2).abs_diff(h1 * 60 + m1) <= 7
        }
        _ => true,
    }
}

/// `name (meta): text` / `name: text` -> (name, meta, text). A name is short and has no markup or link;
/// the colon of a time inside the brackets does not count.
fn split_speaker(s: &str) -> Option<(&str, &str, &str)> {
    let mut colon = s.find(':')?;
    if let Some(o) = s.find('(').filter(|&o| o < colon) {
        let c = o + s[o..].find(')')?;
        colon = c + s[c..].find(':')?;
    }
    let (head, rest) = (s[..colon].trim(), &s[colon + 1..]);
    if !(rest.starts_with(' ') || rest.is_empty()) || head.is_empty() {
        return None;
    }
    let (name, meta) = match head.strip_suffix(')').and_then(|h| h.rsplit_once('(')) {
        Some((n, m)) => (n.trim(), m.trim()),
        None => (head, ""),
    };
    let ok = !name.is_empty() && name.chars().count() <= 32 && !name.contains(|c: char| "*`|<>/\\".contains(c)) && !name.contains("  ");
    ok.then(|| (name, meta, rest.trim()))
}

fn img_src(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let p = std::path::Path::new(s);
    if s.contains(['/', '\\']) || p.is_absolute() {
        if p.is_file() {
            return Some(crate::engine::html::file_url(p));
        }
        return p.file_name().map(|n| escape(&n.to_string_lossy()));
    }
    Some(escape(s))
}

/// Merge `patch` into `into`: objects key by key, anything else replaces.
fn merge(into: &mut Value, patch: &Value) {
    match (into, patch) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                if v.is_null() {
                    a.remove(k);
                } else {
                    merge(a.entry(k.clone()).or_insert(Value::Null), v);
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}

// ---------------------------------------------------------------------------------------------------
// in the page's source

/// The thread a page carries, if it is one.
pub fn from_src(src: &str) -> Option<Thread> {
    let json = crate::engine::html::meta_values(src, META).into_iter().next()?;
    let mut t: Thread = serde_json::from_str(&json.replace(":\\/\\/", "://")).ok()?;
    t.normalize();
    Some(t)
}

/// Write `t` into a layer's source: the regenerated page, the font that is the thread's own, and a
/// length that fits the animation (it only grows: a layer someone lengthened stays).
pub fn store(h: &mut crate::model::HtmlSource, t: &Thread) {
    h.src = t.html();
    h.fields.remove("font");
    h.duration = h.duration.max(t.duration());
}

/// The page without its thread data: an ordinary HTML layer to edit by hand.
pub fn detach(src: &str) -> String {
    let needle = format!("name=\"{META}\"");
    match src.find(&needle) {
        Some(at) => {
            let start = src[..at].rfind("<meta").unwrap_or(at);
            let end = src[at..].find('>').map_or(src.len(), |e| at + e + 1);
            let end = if src[end..].starts_with('\n') { end + 1 } else { end };
            format!("{}{}", &src[..start], &src[end..])
        }
        None => src.to_string(),
    }
}

mod comments;
mod typing;
pub use comments::comma;
pub use typing::{type_plan, typed_at, Key};

#[cfg(test)]
mod tests;
