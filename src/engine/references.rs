//! ---- feat/reference-clips ----
//! Find a Reference: the pure half of the pane (`ui::reference_ui`, `ui::app::reference_ctl`).
//!
//! - `SITES`: where a quote / reaction / meme is usually found, and the search address for each. The pane
//!   opens these in the user's browser; nothing here ever fetches or scrapes them.
//! - GIPHY and Tenor in-app search through their official APIs, with the user's OWN free key (Settings).
//!   One request per search the user typed, nothing sent but the query and the key.
//! - "Just a part": parsing a time range ("1:23-1:45") for yt-dlp's `--download-sections`.
//! - Credits text for every reference in the project (`Copy credits`).
//!
//! Never scrape or bulk-download: every network call here is one search or one file the user picked.

use serde_json::Value;
use std::path::Path;

/// One place to look for a reference clip. `url` holds `{q}` (the query, percent-encoded) or `{slug}`
/// (the query lower-cased with words joined by '-', what GIPHY / Tenor search pages use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    pub id: &'static str,
    pub name: &'static str,
    /// The button's tooltip: what the site is good for.
    pub what: &'static str,
    pub url: &'static str,
    /// What kind of reference it suits (`suggest` orders by it).
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A line someone said in a film or show.
    Quote,
    /// A reaction GIF / meme.
    Reaction,
    /// Anything else: a scene, a moment, a song.
    Video,
}

pub const SITES: &[Site] = &[
    Site {
        id: "yarn",
        name: "Yarn",
        what: "Film and TV clips found by the words said in them",
        url: "https://getyarn.io/yarn-find?text={q}",
        kind: Kind::Quote,
    },
    Site {
        id: "playphrase",
        name: "PlayPhrase",
        what: "Every film scene where a phrase is said, back to back",
        url: "https://www.playphrase.me/#/search?q={q}",
        kind: Kind::Quote,
    },
    Site {
        id: "clipcafe",
        name: "Clip.cafe",
        what: "Short film quote clips with the line as a caption",
        url: "https://clip.cafe/?s={q}",
        kind: Kind::Quote,
    },
    Site {
        id: "tenor",
        name: "Tenor",
        what: "Reaction GIFs and stickers",
        url: "https://tenor.com/search/{slug}-gifs",
        kind: Kind::Reaction,
    },
    Site {
        id: "giphy",
        name: "GIPHY",
        what: "Reaction GIFs, memes and stickers",
        url: "https://giphy.com/search/{slug}",
        kind: Kind::Reaction,
    },
    Site {
        id: "youtube",
        name: "YouTube",
        what: "Anything else: scenes, trailers, interviews, music",
        url: "https://www.youtube.com/results?search_query={q}",
        kind: Kind::Video,
    },
];

pub fn site(id: &str) -> Option<&'static Site> {
    let id = id.trim();
    SITES.iter().find(|s| s.id.eq_ignore_ascii_case(id) || s.name.eq_ignore_ascii_case(id))
}

/// RFC 3986 percent-encoding of everything but the unreserved characters (space becomes %20).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// "Hello there!" -> "hello-there": lower-case words of letters / digits joined by '-' (percent-encoded
/// for anything outside ASCII, so "café" survives).
pub fn slug(s: &str) -> String {
    let words: Vec<String> =
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| encode(&w.to_lowercase())).collect();
    words.join("-")
}

/// The search address of `site` for `query` (trimmed; surrounding quotes dropped - the sites match words).
pub fn search_url(site: &Site, query: &str) -> String {
    let q = clean_query(query);
    site.url.replace("{q}", &encode(&q)).replace("{slug}", &slug(&q))
}

/// The query without surrounding whitespace or quote marks (straight or curly).
pub fn clean_query(query: &str) -> String {
    query
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}'))
        .trim()
        .to_string()
}

/// What `query` most likely is: a quoted line or a sentence is a Quote, one or two words (or a word like
/// "reaction" / "meme") is a Reaction, else a Video.
pub fn guess_kind(query: &str) -> Kind {
    let t = query.trim();
    let lower = t.to_lowercase();
    let quoted = t.starts_with(['"', '\u{201C}', '\'']) && t.len() > 2;
    if ["reaction", "meme", "gif", "facepalm", "eye roll", "shrug"].iter().any(|w| lower.contains(w)) {
        return Kind::Reaction;
    }
    let words = t.split_whitespace().count();
    if quoted || words >= 4 || t.ends_with(['!', '?']) {
        Kind::Quote
    } else if words <= 2 {
        Kind::Reaction
    } else {
        Kind::Video
    }
}

/// `SITES` ordered best-first for `query` (the guessed kind's sites first, in table order).
pub fn suggest(query: &str) -> Vec<&'static Site> {
    let k = guess_kind(query);
    let mut v: Vec<&Site> = SITES.iter().filter(|s| s.kind == k).collect();
    v.extend(SITES.iter().filter(|s| s.kind != k));
    v
}

// ---------------------------------------------------------------- "Just a part"

/// "83", "1:23", "1:02:03", "1:23.5" -> seconds. None for anything else (or a negative / non-finite value).
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut total = 0.0;
    for (i, p) in parts.iter().enumerate() {
        let p = p.trim();
        // only the last field may have a fraction; the others are whole minutes / hours
        let v: f64 = if i + 1 == parts.len() { p.parse().ok()? } else { p.parse::<u32>().ok()? as f64 };
        if i > 0 && v >= 60.0 {
            return None; // "1:75" is a typo, not 2:15
        }
        total = total * 60.0 + v;
    }
    (total.is_finite() && total >= 0.0).then_some(total)
}

/// "1:23-1:45", "1:23 – 1:45", "83 to 105" -> (start, end) seconds, end after start. None when it does not
/// read as a range.
pub fn parse_range(s: &str) -> Option<(f64, f64)> {
    let s = s.trim();
    let (a, b) = s
        .split_once(" to ")
        .or_else(|| s.split_once('\u{2013}')) // en dash
        .or_else(|| s.split_once('\u{2014}')) // em dash
        .or_else(|| s.split_once('-'))?;
    let (a, b) = (parse_time(a)?, parse_time(b)?);
    (b > a).then_some((a, b))
}

/// 83.5 -> "1:23.5", 3725 -> "1:02:05".
pub fn fmt_time(t: f64) -> String {
    let t = t.max(0.0);
    let whole = t.floor() as u64;
    let frac = t - whole as f64;
    let (h, m, s) = (whole / 3600, (whole / 60) % 60, whole % 60);
    let mut out = if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") };
    let tenths = (frac * 10.0).round() as u64;
    if tenths > 0 && tenths < 10 {
        out.push_str(&format!(".{tenths}"));
    }
    out
}

/// yt-dlp arguments that download only `range` (cut at keyframes re-encoded so the part starts clean).
pub fn ytdlp_section_args(range: (f64, f64)) -> [String; 3] {
    ["--download-sections".into(), format!("*{:.3}-{:.3}", range.0, range.1), "--force-keyframes-at-cuts".into()]
}

/// A start time in a pasted link: YouTube's `t=83` / `t=1m23s` / `start=83`. None when there is none.
pub fn url_start_time(url: &str) -> Option<f64> {
    let query = url.split_once(['?', '#']).map(|(_, q)| q)?;
    for kv in query.split(['&', '#', '?']) {
        let Some((k, v)) = kv.split_once('=') else { continue };
        if !matches!(k, "t" | "start" | "time_continue") {
            continue;
        }
        let v = v.trim_end_matches('s');
        if let Ok(n) = v.parse::<f64>() {
            return Some(n);
        }
        // 1h2m3s
        let mut total = 0.0;
        let mut num = String::new();
        for c in v.chars() {
            match c {
                '0'..='9' => num.push(c),
                'h' | 'm' => {
                    let n: f64 = num.parse().ok()?;
                    total += n * if c == 'h' { 3600.0 } else { 60.0 };
                    num.clear();
                }
                _ => return None,
            }
        }
        if !num.is_empty() {
            total += num.parse::<f64>().ok()?;
        }
        return Some(total);
    }
    None
}

/// The site a pasted link is from ("YouTube", "TikTok", the host otherwise) - what the reference's source
/// is credited as.
pub fn site_of_url(url: &str) -> String {
    let host = url
        .trim()
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_start_matches("m.")
        .to_lowercase();
    let known = [
        ("youtube.com", "YouTube"),
        ("youtu.be", "YouTube"),
        ("giphy.com", "GIPHY"),
        ("tenor.com", "Tenor"),
        ("getyarn.io", "Yarn"),
        ("yarn.co", "Yarn"),
        ("playphrase.me", "PlayPhrase"),
        ("clip.cafe", "Clip.cafe"),
        ("tiktok.com", "TikTok"),
        ("vimeo.com", "Vimeo"),
        ("x.com", "X"),
        ("twitter.com", "X"),
        ("instagram.com", "Instagram"),
        ("reddit.com", "Reddit"),
    ];
    known
        .iter()
        .find(|(h, _)| host == *h || host.ends_with(&format!(".{h}")))
        .map(|(_, n)| n.to_string())
        .unwrap_or(host)
}

// ---------------------------------------------------------------- GIPHY / Tenor

/// The two GIF services with an official search API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Provider {
    #[default]
    Giphy,
    Tenor,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Giphy, Provider::Tenor];
    pub fn name(self) -> &'static str {
        match self {
            Provider::Giphy => "GIPHY",
            Provider::Tenor => "Tenor",
        }
    }
    /// The attribution line their terms ask for next to results.
    pub fn attribution(self) -> &'static str {
        match self {
            Provider::Giphy => "Powered by GIPHY",
            Provider::Tenor => "Powered by Tenor",
        }
    }
    /// Where the user gets a free key (shown in Settings and when the key is missing).
    pub fn key_page(self) -> &'static str {
        match self {
            Provider::Giphy => "https://developers.giphy.com/dashboard/",
            Provider::Tenor => "https://developers.google.com/tenor/guides/quickstart",
        }
    }
}

/// One GIF search result: everything the pane shows and what an import needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GifHit {
    pub provider: Provider,
    pub id: String,
    pub title: String,
    /// The result's own page (kept as the reference's source).
    pub page: String,
    /// The MP4 rendition (imported; a GIF file would play worse and weigh more).
    pub mp4: String,
    /// A small still for the results grid.
    pub still: String,
    pub width: u32,
    pub height: u32,
    /// The uploader, when the service names one.
    pub credit: String,
}

/// The search request. `limit` is clamped to 1..=50. Only the query and the user's key are sent.
pub fn search_request(p: Provider, key: &str, query: &str, limit: u32) -> String {
    let (q, key, n) = (encode(&clean_query(query)), encode(key.trim()), limit.clamp(1, 50));
    match p {
        Provider::Giphy => format!("https://api.giphy.com/v1/gifs/search?api_key={key}&q={q}&limit={n}&rating=pg-13"),
        Provider::Tenor => format!(
            "https://tenor.googleapis.com/v2/search?q={q}&key={key}&client_key=simple_editor&limit={n}\
             &media_filter=mp4,gifpreview&contentfilter=medium"
        ),
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// GIPHY sends sizes as strings ("480"), Tenor as numbers.
fn n(v: &Value) -> u32 {
    v.as_u64().map(|x| x as u32).or_else(|| v.as_str().and_then(|t| t.parse().ok())).unwrap_or(0)
}

/// A service's search reply -> hits (results without an MP4 are skipped). Err with the service's own
/// message when it refused (a bad key says so).
pub fn parse_search(p: Provider, body: &str) -> Result<Vec<GifHit>, String> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| format!("{} replied with something unreadable: {e}", p.name()))?;
    match p {
        Provider::Giphy => {
            // {"meta":{"status":401,"msg":"Unauthorized"}} or {"message":"Invalid authentication credentials"}
            let status = v["meta"]["status"].as_u64().unwrap_or(200);
            if status != 200 || v.get("data").is_none() {
                let msg = v["meta"]["msg"].as_str().or(v["message"].as_str()).unwrap_or("error");
                return Err(format!("GIPHY: {msg} (check the key in Settings)"));
            }
            Ok(v["data"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .filter_map(|d| {
                    let img = &d["images"];
                    let mp4 = img["original"]["mp4"].as_str().or(img["original_mp4"]["mp4"].as_str())?.to_string();
                    let still = [&img["fixed_height_still"]["url"], &img["original_still"]["url"]]
                        .iter()
                        .find_map(|u| u.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let credit = d["user"]["display_name"]
                        .as_str()
                        .filter(|x| !x.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| s(&d["username"]));
                    Some(GifHit {
                        provider: p,
                        id: s(&d["id"]),
                        title: s(&d["title"]),
                        page: s(&d["url"]),
                        mp4,
                        still,
                        width: n(&img["original"]["width"]),
                        height: n(&img["original"]["height"]),
                        credit,
                    })
                })
                .collect())
        }
        Provider::Tenor => {
            // {"error":{"code":400,"message":"API key not valid...","status":"INVALID_ARGUMENT"}}
            if let Some(e) = v.get("error") {
                let msg = e["message"].as_str().unwrap_or("error");
                return Err(format!("Tenor: {msg} (check the key in Settings)"));
            }
            Ok(v["results"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .filter_map(|r| {
                    let f = &r["media_formats"];
                    let mp4 = f["mp4"]["url"].as_str()?.to_string();
                    let dims = &f["mp4"]["dims"];
                    let title = r["content_description"].as_str().or(r["title"].as_str()).unwrap_or_default();
                    Some(GifHit {
                        provider: p,
                        id: s(&r["id"]),
                        title: title.to_string(),
                        page: s(&r["itemurl"]),
                        mp4,
                        still: f["gifpreview"]["url"]
                            .as_str()
                            .or(f["tinygifpreview"]["url"].as_str())
                            .unwrap_or_default()
                            .to_string(),
                        width: n(&dims[0]),
                        height: n(&dims[1]),
                        credit: String::new(),
                    })
                })
                .collect())
        }
    }
}

/// A file name for a GIF import: "<title> [<provider>-<id>].mp4", kept to safe characters.
pub fn gif_file_name(h: &GifHit) -> String {
    let title: String = h
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title: String = title.chars().take(60).collect();
    let id: String = h.id.chars().filter(|c| c.is_ascii_alphanumeric()).take(32).collect();
    let stem = if title.is_empty() { h.provider.name().to_string() } else { title };
    format!("{stem} [{}-{id}].mp4", h.provider.name().to_lowercase())
}

// ---------------------------------------------------------------- network (curl.exe, background threads only)

/// GET `url` and return the body. Blocking: call from a worker. curl's own error text on failure.
pub fn http_get(url: &str) -> Result<String, String> {
    let curl =
        crate::media::ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found (it ships with Windows 10 1803+)")?;
    let out = crate::media::ffpipe::command(&curl)
        // no --fail: an API's error reply carries the reason (a bad key), which parse_search reports
        .args(["--location", "--silent", "--show-error", "--max-time", "20", "--proto", "=https"])
        .arg(url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if msg.is_empty() { format!("request failed ({})", out.status) } else { msg });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One search, start to finish (worker thread).
pub fn search(p: Provider, key: &str, query: &str, limit: u32) -> Result<Vec<GifHit>, String> {
    if key.trim().is_empty() {
        return Err(format!("Add your free {} key in Settings ▸ General first", p.name()));
    }
    if clean_query(query).is_empty() {
        return Err("Type what to search for".into());
    }
    parse_search(p, &http_get(&search_request(p, key, query, limit))?)
}

/// Download one file to `dest` through `<dest>.part` + rename (worker thread). HTTPS only.
pub fn download(url: &str, dest: &Path, cancel: impl Fn() -> bool) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("not an https address: {url}"));
    }
    let curl = crate::media::ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found")?;
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut tmp = dest.as_os_str().to_os_string();
    tmp.push(".part");
    let tmp = std::path::PathBuf::from(tmp);
    let mut child = crate::media::ffpipe::command(&curl)
        .args(["--fail", "--location", "--silent", "--show-error", "--max-time", "300", "--proto", "=https", "-o"])
        .arg(&tmp)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let status = loop {
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err(crate::engine::export::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("curl: {e}")),
        }
    };
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut e, &mut err);
        }
        let _ = std::fs::remove_file(&tmp);
        let err = err.trim();
        return Err(if err.is_empty() { format!("download failed ({status})") } else { err.to_string() });
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("{}: {e}", dest.display()))
}

// ---------------------------------------------------------------- credits

/// The one-time note shown before the first reference import (Settings remembers it was seen).
pub const FAIR_USE_NOTE: &str = "Clips from films, shows and GIF sites belong to their owners. A short excerpt used for \
commentary, criticism, review, parody or teaching can be fair use (or fair dealing), but that depends on how you use it \
and where you publish. Keep clips short, credit them, and check each platform's rules. Simple Editor keeps where each \
reference came from (Copy credits lists them) and only ever downloads the one link you chose: no scraping, no bulk \
downloads.";

/// One credits line: `Title - Site (credit), 1:23-1:45: <url>`.
pub fn credit_line(title: &str, site: &str, credit: &str, range: Option<(f64, f64)>, url: &str) -> String {
    let mut s = if title.trim().is_empty() { site.to_string() } else { format!("{} - {site}", title.trim()) };
    if !credit.trim().is_empty() {
        s.push_str(&format!(" ({})", credit.trim()));
    }
    if let Some((a, b)) = range {
        s.push_str(&format!(", {}-{}", fmt_time(a), fmt_time(b)));
    }
    if !url.trim().is_empty() {
        s.push_str(&format!(": {}", url.trim()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_site_builds_an_https_search_address() {
        for s in SITES {
            let u = search_url(s, "  \u{201C}Hello there!\u{201D} ");
            assert!(u.starts_with("https://"), "{u}");
            assert!(!u.contains('{') && !u.contains(' '), "{} left a placeholder or a space: {u}", s.name);
            assert!(u.to_lowercase().contains("hello"), "{u}");
        }
        assert_eq!(
            search_url(site("youtube").unwrap(), "a&b c"),
            "https://www.youtube.com/results?search_query=a%26b%20c"
        );
        assert_eq!(search_url(site("GIPHY").unwrap(), "Mind Blown!"), "https://giphy.com/search/mind-blown");
        assert_eq!(search_url(site("tenor").unwrap(), "eye roll"), "https://tenor.com/search/eye-roll-gifs");
        assert_eq!(SITES.len(), 6, "Yarn, PlayPhrase, Clip.cafe, Tenor, GIPHY, YouTube");
    }

    #[test]
    fn encode_and_slug() {
        assert_eq!(encode("é ok/?"), "%C3%A9%20ok%2F%3F");
        assert_eq!(slug("  Café -- au lait "), "caf%C3%A9-au-lait");
        assert_eq!(slug("!!!"), "");
    }

    #[test]
    fn guess_kind_and_suggest_order() {
        assert_eq!(guess_kind("\"I'll be back\""), Kind::Quote);
        assert_eq!(guess_kind("you shall not pass"), Kind::Quote);
        assert_eq!(guess_kind("facepalm"), Kind::Reaction);
        assert_eq!(guess_kind("mind blown"), Kind::Reaction);
        assert_eq!(guess_kind("rocket launch slow"), Kind::Video);
        assert_eq!(suggest("you shall not pass")[0].id, "yarn");
        assert_eq!(suggest("shrug")[0].id, "tenor");
        assert_eq!(suggest("rocket launch slow")[0].id, "youtube");
        assert_eq!(suggest("x").len(), SITES.len(), "every site, reordered");
    }

    #[test]
    fn times_and_ranges_parse() {
        assert_eq!(parse_time("83"), Some(83.0));
        assert_eq!(parse_time("1:23"), Some(83.0));
        assert_eq!(parse_time("1:02:03"), Some(3723.0));
        assert_eq!(parse_time("0:10.5"), Some(10.5));
        assert_eq!(parse_time("1:75"), None);
        assert_eq!(parse_time("a:10"), None);
        assert_eq!(parse_time(""), None);
        assert_eq!(parse_range("1:23-1:45"), Some((83.0, 105.0)));
        assert_eq!(parse_range("1:23 \u{2013} 1:45"), Some((83.0, 105.0)));
        assert_eq!(parse_range("10 to 20.5"), Some((10.0, 20.5)));
        assert_eq!(parse_range("0:20-0:10"), None, "end before start");
        assert_eq!(parse_range("1:23"), None);
        assert_eq!(fmt_time(83.0), "1:23");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(10.5), "0:10.5");
        assert_eq!(ytdlp_section_args((83.0, 105.5))[1], "*83.000-105.500");
    }

    #[test]
    fn start_time_from_links() {
        assert_eq!(url_start_time("https://youtu.be/abc?t=83"), Some(83.0));
        assert_eq!(url_start_time("https://www.youtube.com/watch?v=abc&t=1m23s"), Some(83.0));
        assert_eq!(url_start_time("https://www.youtube.com/watch?v=abc&t=1h0m5s"), Some(3605.0));
        assert_eq!(url_start_time("https://www.youtube.com/watch?v=abc"), None);
        assert_eq!(site_of_url("https://m.youtube.com/watch?v=1"), "YouTube");
        assert_eq!(site_of_url("https://media.giphy.com/media/x/giphy.mp4"), "GIPHY");
        assert_eq!(site_of_url("https://example.org/a.mp4"), "example.org");
    }

    #[test]
    fn search_requests_send_only_the_query_and_key() {
        let g = search_request(Provider::Giphy, " K1 ", "\"mind blown\"", 500);
        assert_eq!(g, "https://api.giphy.com/v1/gifs/search?api_key=K1&q=mind%20blown&limit=50&rating=pg-13");
        let t = search_request(Provider::Tenor, "K2", "shrug", 0);
        assert!(t.starts_with("https://tenor.googleapis.com/v2/search?q=shrug&key=K2&"), "{t}");
        assert!(t.contains("limit=1") && !t.contains("locale") && !t.contains("country"), "{t}");
    }

    #[test]
    fn giphy_reply_parses() {
        let body = r#"{"data":[
            {"id":"abc","title":"Mind Blown GIF","url":"https://giphy.com/gifs/abc","username":"studio",
             "user":{"display_name":"The Studio"},
             "images":{"original":{"mp4":"https://media.giphy.com/media/abc/giphy.mp4","width":"480","height":"270"},
                       "fixed_height_still":{"url":"https://media.giphy.com/media/abc/200_s.gif"}}},
            {"id":"nomp4","title":"x","images":{"original":{"url":"https://x/y.gif"}}}
        ],"meta":{"status":200,"msg":"OK"}}"#;
        let hits = parse_search(Provider::Giphy, body).unwrap();
        assert_eq!(hits.len(), 1, "a result without an MP4 is skipped");
        let h = &hits[0];
        assert_eq!((h.width, h.height), (480, 270));
        assert_eq!(h.credit, "The Studio");
        assert_eq!(h.page, "https://giphy.com/gifs/abc");
        assert!(h.still.ends_with("200_s.gif"));
        assert_eq!(gif_file_name(h), "Mind Blown GIF [giphy-abc].mp4");
        let bad = parse_search(Provider::Giphy, r#"{"data":[],"meta":{"status":401,"msg":"Unauthorized"}}"#);
        assert!(bad.unwrap_err().contains("Unauthorized"));
        assert!(parse_search(Provider::Giphy, r#"{"message":"Invalid authentication credentials"}"#).is_err());
        assert!(parse_search(Provider::Giphy, "<html>").is_err());
    }

    #[test]
    fn tenor_reply_parses() {
        let body = r#"{"results":[{"id":"123","content_description":"Shrug GIF","itemurl":"https://tenor.com/view/shrug-123",
            "media_formats":{"mp4":{"url":"https://media.tenor.com/a/b.mp4","dims":[498,280]},
                             "gifpreview":{"url":"https://media.tenor.com/a/c.png"}}}],"next":"5"}"#;
        let hits = parse_search(Provider::Tenor, body).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].width, hits[0].height), (498, 280));
        assert_eq!(hits[0].title, "Shrug GIF");
        assert_eq!(gif_file_name(&hits[0]), "Shrug GIF [tenor-123].mp4");
        let bad = parse_search(Provider::Tenor, r#"{"error":{"code":400,"message":"API key not valid"}}"#);
        assert!(bad.unwrap_err().contains("API key not valid"));
    }

    #[test]
    fn missing_key_or_query_never_touches_the_network() {
        assert!(search(Provider::Giphy, " ", "cat", 10).unwrap_err().contains("Settings"));
        assert!(search(Provider::Tenor, "k", " \"\" ", 10).unwrap_err().contains("search"));
        assert!(download("http://example.org/a.mp4", Path::new("x.mp4"), || false).is_err(), "https only");
    }

    #[test]
    fn credits_lines() {
        assert_eq!(
            credit_line("Shrug GIF", "Tenor", "", None, "https://tenor.com/view/shrug-123"),
            "Shrug GIF - Tenor: https://tenor.com/view/shrug-123"
        );
        assert_eq!(
            credit_line("", "YouTube", "Some Channel", Some((83.0, 105.0)), "u"),
            "YouTube (Some Channel), 1:23-1:45: u"
        );
    }
}
