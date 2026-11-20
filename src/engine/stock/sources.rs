//! ---- ws:gallery-stock ----
//! Per-site requests and response parsers. Every parser is pure (text in, hits out) so the tests can run them
//! on the saved sample responses in `fixtures/`. Each site's documented API:
//! - Wikimedia Commons: MediaWiki Action API, `generator=search` over the File namespace + `imageinfo`
//!   (licence from `extmetadata`). No key; a descriptive User-Agent is required.
//! - Internet Archive: `advancedsearch.php` (Solr query, licence filtered server-side) + `/metadata/<id>`
//!   for the file list. No key.
//! - NASA Image and Video Library: `images-api.nasa.gov/search` + the item's `collection.json`. No key.
//! - Openverse: `api.openverse.org/v1/images|audio/`. Anonymous access (rate-limited).
//! - Pexels (`Authorization: <key>`), Pixabay (`key=` parameter, as its docs say), Freesound
//!   (`Authorization: Token <key>`): the user's own key.

use super::{Hit, Kind, Licence, LicenceLevel, NetError, Orientation, Query, Request, Source};
use serde_json::Value;

/// Results per page and source.
pub const PER_PAGE: u32 = 30;

/// Percent-encode a query value (everything but the RFC 3986 unreserved set).
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A URL with spaces and other unsafe characters in its path made safe (NASA's asset URLs have spaces), and
/// http upgraded to https.
pub fn safe_url(u: &str) -> String {
    let u = u.trim();
    let u = u.strip_prefix("http://").map(|r| format!("https://{r}")).unwrap_or_else(|| u.to_string());
    let mut out = String::new();
    for c in u.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '"' => out.push_str("%22"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// Drop a Commons URL's `?utm_...` tracking query.
fn strip_utm(u: &str) -> String {
    match u.split_once("?utm_") {
        Some((a, _)) => a.to_string(),
        None => u.to_string(),
    }
}

/// HTML -> plain text (Commons' `Artist` is HTML): tags removed, the common entities decoded, spaces collapsed.
pub fn plain(html: &str) -> String {
    let mut s = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => s.push(c),
            _ => {}
        }
    }
    let s = s
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn s_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Array(a) => a.first().map(s_of).unwrap_or_default(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn u32_of(v: &Value) -> u32 {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0).min(u32::MAX as u64) as u32
}

fn f64_of(v: &Value) -> f64 {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0)
}

/// A Creative Commons URL -> its short name ("https://creativecommons.org/licenses/by-sa/4.0/" -> "CC BY-SA 4.0").
pub fn cc_name(url: &str) -> String {
    let l = url.to_ascii_lowercase();
    let parts: Vec<&str> = l.split('/').filter(|p| !p.is_empty()).collect();
    let ver = parts.iter().rev().find(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit())).copied().unwrap_or("");
    let ver = if ver.is_empty() { String::new() } else { format!(" {ver}") };
    if l.contains("publicdomain/zero") {
        format!("CC0{ver}")
    } else if l.contains("publicdomain/mark") {
        format!("Public Domain Mark{ver}")
    } else if l.contains("publicdomain") {
        "Public domain".into()
    } else if let Some(i) = parts.iter().position(|p| *p == "licenses") {
        let code = parts.get(i + 1).copied().unwrap_or("");
        format!("CC {}{ver}", code.to_ascii_uppercase())
    } else {
        String::new()
    }
}

// ------------------------------------------------------------------------------------------- requests

/// The search request for `src`, or None when it cannot look for `q.kind` (or needs a key it does not have).
pub fn request(src: Source, q: &Query, key: Option<&str>) -> Option<Request> {
    if !src.kinds().contains(&q.kind) {
        return None;
    }
    let key = key.map(str::trim).filter(|k| !k.is_empty());
    if src.needs_key() && key.is_none() {
        return None;
    }
    let text = q.text.trim();
    let page = q.page();
    let plain = |url: String| Some(Request { url, headers: Vec::new() });
    match src {
        Source::Commons => {
            let ft = match q.kind {
                Kind::Video => "filetype:video",
                Kind::Image => "filetype:bitmap",
                Kind::Music | Kind::Sfx => "filetype:audio",
            };
            let mut search = format!("{ft} {text}");
            if q.min_res > 0 && !q.kind.is_audio() {
                search.push_str(&format!(" filew:>{} fileh:>{}", q.min_res - 1, q.min_res - 1));
            }
            plain(format!(
                "https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2&generator=search\
                 &gsrnamespace=6&gsrlimit={PER_PAGE}&gsroffset={}&gsrsearch={}&prop=imageinfo\
                 &iiprop=url%7Csize%7Cmime%7Cextmetadata%7Cmediatype&iiurlwidth=320\
                 &iiextmetadatafilter=LicenseShortName%7CArtist%7CLicenseUrl%7CObjectName",
                (page - 1) * PER_PAGE,
                enc(search.trim())
            ))
        }
        Source::Archive => {
            let media = if q.kind == Kind::Video { "movies" } else { "audio" };
            let mut lic = vec!["licenseurl:*publicdomain*"];
            if q.kind == Kind::Video {
                lic.insert(0, "collection:prelinger");
            }
            if q.licence >= LicenceLevel::By {
                lic.push("licenseurl:*\\/by\\/*");
            }
            if q.licence >= LicenceLevel::BySa {
                lic.push("licenseurl:*by-sa*");
            }
            let words = if text.is_empty() { "*".to_string() } else { format!("({text})") };
            let solr = format!("{words} AND mediatype:{media} AND ({}) AND NOT collection:deemphasize", lic.join(" OR "));
            let fl = ["identifier", "title", "creator", "licenseurl", "collection", "mediatype"]
                .iter()
                .map(|f| format!("&fl%5B%5D={f}"))
                .collect::<String>();
            plain(format!(
                "https://archive.org/advancedsearch.php?q={}{fl}&rows={PER_PAGE}&page={page}&output=json",
                enc(&solr)
            ))
        }
        Source::Nasa => {
            let media = match q.kind {
                Kind::Video => "video",
                Kind::Image => "image",
                _ => "audio",
            };
            plain(format!(
                "https://images-api.nasa.gov/search?q={}&media_type={media}&page={page}&page_size={PER_PAGE}",
                enc(if text.is_empty() { "space" } else { text })
            ))
        }
        Source::Openverse => {
            let lic = match q.licence {
                LicenceLevel::Pd => "cc0,pdm",
                LicenceLevel::By => "cc0,pdm,by",
                LicenceLevel::BySa => "cc0,pdm,by,by-sa",
            };
            let (path, extra) = match q.kind {
                Kind::Image => {
                    let ar = match q.orientation {
                        Orientation::Landscape => "&aspect_ratio=wide",
                        Orientation::Portrait => "&aspect_ratio=tall",
                        Orientation::Square => "&aspect_ratio=square",
                        Orientation::Any => "",
                    };
                    ("images", ar.to_string())
                }
                Kind::Music => ("audio", "&category=music".to_string()),
                _ => ("audio", "&category=sound_effect".to_string()),
            };
            plain(format!(
                "https://api.openverse.org/v1/{path}/?q={}&page={page}&page_size={PER_PAGE}&license={lic}&mature=false{extra}",
                enc(text)
            ))
        }
        Source::Pexels => {
            let o = match q.orientation {
                Orientation::Landscape => "&orientation=landscape",
                Orientation::Portrait => "&orientation=portrait",
                Orientation::Square => "&orientation=square",
                Orientation::Any => "",
            };
            let path = if q.kind == Kind::Video { "videos/search" } else { "v1/search" };
            Some(Request {
                url: format!(
                    "https://api.pexels.com/{path}?query={}&per_page={PER_PAGE}&page={page}{o}",
                    enc(if text.is_empty() { "nature" } else { text })
                ),
                headers: vec![format!("Authorization: {}", key?)],
            })
        }
        Source::Pixabay => {
            let o = match q.orientation {
                Orientation::Landscape => "&orientation=horizontal",
                Orientation::Portrait => "&orientation=vertical",
                _ => "",
            };
            let (path, extra) = if q.kind == Kind::Video { ("api/videos/", "") } else { ("api/", "&image_type=photo") };
            let min = if q.min_res > 0 && q.kind == Kind::Image {
                format!("&min_width={0}&min_height={0}", q.min_res)
            } else {
                String::new()
            };
            plain(format!(
                "https://pixabay.com/{path}?key={}&q={}&per_page={PER_PAGE}&page={page}&safesearch=true{extra}{o}{min}",
                enc(key?),
                enc(text)
            ))
        }
        Source::Freesound => {
            let lic = if q.licence == LicenceLevel::Pd {
                "license:\"Creative Commons 0\"".to_string()
            } else {
                "license:(\"Creative Commons 0\" OR \"Attribution\")".to_string()
            };
            let filter = if q.kind == Kind::Music { format!("{lic} tag:music") } else { lic };
            Some(Request {
                url: format!(
                    "https://freesound.org/apiv2/search/text/?query={}&page={page}&page_size={PER_PAGE}\
                     &fields=id,name,username,license,previews,duration,url,images&filter={}",
                    enc(text),
                    enc(&filter)
                ),
                headers: vec![format!("Authorization: Token {}", key?)],
            })
        }
    }
}

/// The request that downloads `hit`'s file (stock files are public CDN links: no key goes with them).
pub fn file_request(hit: &Hit, _key: Option<&str>) -> Request {
    Request { url: safe_url(&hit.file_url), headers: Vec::new() }
}

// ------------------------------------------------------------------------------------------- parsers

/// Parse one search response of `src` for `kind`.
pub fn parse(src: Source, kind: Kind, text: &str) -> Result<Vec<Hit>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{}: bad response ({e})", src.name()))?;
    Ok(match src {
        Source::Commons => commons(&v, kind),
        Source::Archive => archive(&v, kind),
        Source::Nasa => nasa(&v, kind),
        Source::Openverse => openverse(&v, kind),
        Source::Pexels => pexels(&v, kind),
        Source::Pixabay => pixabay(&v, kind),
        Source::Freesound => freesound(&v, kind),
    })
}

fn commons(v: &Value, kind: Kind) -> Vec<Hit> {
    let Some(pages) = v["query"]["pages"].as_array() else { return Vec::new() };
    let mut pages: Vec<&Value> = pages.iter().collect();
    pages.sort_by_key(|p| p["index"].as_u64().unwrap_or(u64::MAX));
    pages
        .into_iter()
        .filter_map(|p| {
            let ii = p["imageinfo"].get(0)?;
            let mt = ii["mediatype"].as_str().unwrap_or("");
            let got = match mt {
                "VIDEO" => Kind::Video,
                "BITMAP" | "DRAWING" => Kind::Image,
                // a sound is Music or SFX, whichever was searched; a video or picture search skips it
                "AUDIO" if kind.is_audio() => kind,
                _ => return None,
            };
            // a video search also finds audio-only .ogg / .webm files: they belong to Music / SFX
            if got.is_audio() != kind.is_audio() || (got == Kind::Image) != (kind == Kind::Image) {
                return None;
            }
            let meta = &ii["extmetadata"];
            let lic_name = plain(meta["LicenseShortName"]["value"].as_str().unwrap_or(""));
            let lic_url = meta["LicenseUrl"]["value"].as_str().unwrap_or("").to_string();
            let licence = Licence::parse(&lic_name).min_known(Licence::parse(&lic_url));
            let file_title = p["title"].as_str().unwrap_or("").trim_start_matches("File:");
            let stem = file_title.rsplit_once('.').map_or(file_title, |(a, _)| a);
            let title = plain(meta["ObjectName"]["value"].as_str().unwrap_or(""));
            let thumb = ii["thumburl"].as_str().unwrap_or("");
            Some(Hit {
                id: format!("commons:{}", p["pageid"].as_u64()?),
                source: Some(Source::Commons),
                kind: got,
                title: if title.is_empty() { stem.to_string() } else { title },
                author: plain(meta["Artist"]["value"].as_str().unwrap_or("")),
                page_url: ii["descriptionurl"].as_str().unwrap_or("").to_string(),
                licence: Some(licence),
                licence_name: lic_name,
                licence_url: lic_url.trim_end_matches("deed.en").to_string(),
                thumb_url: if thumb.contains("/file-type-icons/") { String::new() } else { strip_utm(thumb) },
                file_url: strip_utm(ii["url"].as_str()?),
                width: u32_of(&ii["width"]),
                height: u32_of(&ii["height"]),
                duration: f64_of(&ii["duration"]),
                ..Default::default()
            })
        })
        .collect()
}

impl Licence {
    /// Of two readings (the short name and the URL), the one that says more: a known group beats Other.
    fn min_known(self, other: Licence) -> Licence {
        match (self, other) {
            (Licence::Other, o) => o,
            (s, Licence::Other) => s,
            // an NC / ND url wins over a short name that reads as BY
            (s, o) => s.max(o),
        }
    }
}

fn archive(v: &Value, kind: Kind) -> Vec<Hit> {
    let Some(docs) = v["response"]["docs"].as_array() else { return Vec::new() };
    docs.iter()
        .filter_map(|d| {
            let id = s_of(&d["identifier"]);
            if id.is_empty() {
                return None;
            }
            let media = s_of(&d["mediatype"]);
            let got = match media.as_str() {
                "movies" => Kind::Video,
                "audio" | "etree" => kind,
                _ => return None,
            };
            if got.is_audio() != kind.is_audio() {
                return None;
            }
            let lic_url = s_of(&d["licenseurl"]);
            let prelinger = d["collection"]
                .as_array()
                .map(|a| a.iter().any(|c| c.as_str() == Some("prelinger")))
                .unwrap_or_else(|| s_of(&d["collection"]) == "prelinger");
            let (licence, lic_name) = if !lic_url.is_empty() {
                (Licence::parse(&lic_url), cc_name(&lic_url))
            } else if prelinger {
                (Licence::Pd, "Public domain (Prelinger Archives)".to_string())
            } else {
                (Licence::Other, String::new())
            };
            Some(Hit {
                id: format!("archive:{id}"),
                source: Some(Source::Archive),
                kind: got,
                title: s_of(&d["title"]),
                author: s_of(&d["creator"]),
                page_url: format!("https://archive.org/details/{id}"),
                licence: Some(licence),
                licence_name: lic_name,
                licence_url: lic_url,
                thumb_url: format!("https://archive.org/services/img/{id}"),
                detail: id,
                ..Default::default()
            })
        })
        .collect()
}

pub const NASA_LICENCE: &str = "NASA media (no copyright in the US)";
pub const NASA_GUIDE: &str = "https://www.nasa.gov/nasa-brand-center/images-and-media/";

fn nasa(v: &Value, kind: Kind) -> Vec<Hit> {
    let Some(items) = v["collection"]["items"].as_array() else { return Vec::new() };
    items
        .iter()
        .filter_map(|it| {
            let d = it["data"].get(0)?;
            let id = s_of(&d["nasa_id"]);
            let links = it["links"].as_array().cloned().unwrap_or_default();
            let thumb = links
                .iter()
                .find(|l| l["rel"] == "preview" && l["render"] == "image")
                .or_else(|| links.iter().find(|l| l["render"] == "image"))
                .map(|l| safe_url(&s_of(&l["href"])))
                .unwrap_or_default();
            let canon = links.iter().find(|l| l["rel"] == "canonical");
            let author = [s_of(&d["photographer"]), s_of(&d["secondary_creator"])]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    let c = s_of(&d["center"]);
                    if c.is_empty() { "NASA".into() } else { format!("NASA/{c}") }
                });
            let mut h = Hit {
                id: format!("nasa:{id}"),
                source: Some(Source::Nasa),
                kind,
                title: s_of(&d["title"]),
                author,
                page_url: format!("https://images.nasa.gov/details/{}", enc(&id)),
                licence: Some(Licence::Pd),
                licence_name: NASA_LICENCE.into(),
                licence_url: NASA_GUIDE.into(),
                thumb_url: thumb,
                detail: safe_url(&s_of(&it["href"])),
                ..Default::default()
            };
            if kind == Kind::Image {
                if let Some(c) = canon {
                    h.file_url = safe_url(&s_of(&c["href"]));
                    h.width = u32_of(&c["width"]);
                    h.height = u32_of(&c["height"]);
                }
            }
            (!id.is_empty()).then_some(h)
        })
        .collect()
}

fn openverse(v: &Value, kind: Kind) -> Vec<Hit> {
    let Some(rs) = v["results"].as_array() else { return Vec::new() };
    rs.iter()
        .filter_map(|r| {
            let lic = s_of(&r["license"]);
            let ver = s_of(&r["license_version"]);
            let name = match lic.as_str() {
                "cc0" => format!("CC0 {ver}"),
                "pdm" => format!("Public Domain Mark {ver}"),
                "" => String::new(),
                other => format!("CC {} {ver}", other.to_ascii_uppercase()),
            };
            let ms = f64_of(&r["duration"]);
            Some(Hit {
                id: format!("openverse:{}", s_of(&r["id"])),
                source: Some(Source::Openverse),
                kind,
                title: s_of(&r["title"]),
                author: s_of(&r["creator"]),
                author_url: s_of(&r["creator_url"]),
                page_url: s_of(&r["foreign_landing_url"]),
                licence: Some(Licence::parse(&lic)),
                licence_name: name.trim().to_string(),
                licence_url: s_of(&r["license_url"]),
                thumb_url: s_of(&r["thumbnail"]),
                file_url: {
                    let u = s_of(&r["url"]);
                    if u.is_empty() {
                        return None;
                    }
                    u
                },
                width: u32_of(&r["width"]),
                height: u32_of(&r["height"]),
                duration: ms / 1000.0,
                ..Default::default()
            })
        })
        .collect()
}

pub const PEXELS_LICENCE: (&str, &str) = ("Pexels License", "https://www.pexels.com/license/");
pub const PIXABAY_LICENCE: (&str, &str) = ("Pixabay Content License", "https://pixabay.com/service/license-summary/");

fn pexels(v: &Value, kind: Kind) -> Vec<Hit> {
    let free = |h: &mut Hit| {
        h.source = Some(Source::Pexels);
        h.licence = Some(Licence::Free);
        h.licence_name = PEXELS_LICENCE.0.into();
        h.licence_url = PEXELS_LICENCE.1.into();
    };
    if kind == Kind::Video {
        let Some(vs) = v["videos"].as_array() else { return Vec::new() };
        return vs
            .iter()
            .filter_map(|x| {
                let files: Vec<&Value> = x["video_files"]
                    .as_array()?
                    .iter()
                    .filter(|f| s_of(&f["file_type"]) == "video/mp4" && !s_of(&f["link"]).is_empty())
                    .collect();
                // the biggest file up to 4K, and the smallest for the hover preview
                let best = files.iter().filter(|f| u32_of(&f["width"]) <= 3840).max_by_key(|f| u32_of(&f["width"]))?;
                let small = files.iter().min_by_key(|f| u32_of(&f["width"]).max(1));
                let mut h = Hit {
                    id: format!("pexels:{}", s_of(&x["id"])),
                    kind,
                    title: pexels_title(&s_of(&x["url"]), &s_of(&x["id"])),
                    author: s_of(&x["user"]["name"]),
                    author_url: s_of(&x["user"]["url"]),
                    page_url: s_of(&x["url"]),
                    thumb_url: s_of(&x["image"]),
                    preview_url: small.map(|f| s_of(&f["link"])).unwrap_or_default(),
                    file_url: s_of(&best["link"]),
                    width: u32_of(&best["width"]),
                    height: u32_of(&best["height"]),
                    duration: f64_of(&x["duration"]),
                    ..Default::default()
                };
                free(&mut h);
                Some(h)
            })
            .collect();
    }
    let Some(ps) = v["photos"].as_array() else { return Vec::new() };
    ps.iter()
        .filter_map(|x| {
            let alt = s_of(&x["alt"]);
            let mut h = Hit {
                id: format!("pexels:{}", s_of(&x["id"])),
                kind,
                title: if alt.is_empty() { pexels_title(&s_of(&x["url"]), &s_of(&x["id"])) } else { alt },
                author: s_of(&x["photographer"]),
                author_url: s_of(&x["photographer_url"]),
                page_url: s_of(&x["url"]),
                thumb_url: s_of(&x["src"]["medium"]),
                file_url: {
                    let u = s_of(&x["src"]["original"]);
                    if u.is_empty() {
                        return None;
                    }
                    u
                },
                width: u32_of(&x["width"]),
                height: u32_of(&x["height"]),
                ..Default::default()
            };
            free(&mut h);
            Some(h)
        })
        .collect()
}

/// `https://www.pexels.com/video/waves-crashing-on-rocks-123/` -> "Waves crashing on rocks".
fn pexels_title(url: &str, id: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let words = last.trim_end_matches(id).trim_end_matches('-').replace('-', " ");
    let mut c = words.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => format!("Pexels {id}"),
    }
}

fn pixabay(v: &Value, kind: Kind) -> Vec<Hit> {
    let Some(hs) = v["hits"].as_array() else { return Vec::new() };
    hs.iter()
        .filter_map(|x| {
            let tags = s_of(&x["tags"]);
            let title = tags.split(',').map(str::trim).filter(|t| !t.is_empty()).take(3).collect::<Vec<_>>().join(", ");
            let user = s_of(&x["user"]);
            let mut h = Hit {
                id: format!("pixabay:{}", s_of(&x["id"])),
                source: Some(Source::Pixabay),
                kind,
                title: if title.is_empty() { format!("Pixabay {}", s_of(&x["id"])) } else { title },
                author_url: format!("https://pixabay.com/users/{user}-{}/", s_of(&x["user_id"])),
                author: user,
                page_url: s_of(&x["pageURL"]),
                licence: Some(Licence::Free),
                licence_name: PIXABAY_LICENCE.0.into(),
                licence_url: PIXABAY_LICENCE.1.into(),
                ..Default::default()
            };
            if kind == Kind::Video {
                let vs = &x["videos"];
                let best = ["large", "medium", "small"].iter().map(|k| &vs[*k]).find(|f| !s_of(&f["url"]).is_empty())?;
                h.file_url = s_of(&best["url"]);
                h.width = u32_of(&best["width"]);
                h.height = u32_of(&best["height"]);
                h.preview_url = ["tiny", "small"].iter().map(|k| s_of(&vs[*k]["url"])).find(|u| !u.is_empty()).unwrap_or_default();
                h.thumb_url = ["medium", "small", "tiny", "large"]
                    .iter()
                    .map(|k| s_of(&vs[*k]["thumbnail"]))
                    .find(|u| !u.is_empty())
                    .unwrap_or_default();
                h.duration = f64_of(&x["duration"]);
            } else {
                h.file_url = s_of(&x["largeImageURL"]);
                h.thumb_url = s_of(&x["webformatURL"]);
                // largeImageURL is at most 1280 px on its long side
                let (w, hh) = (u32_of(&x["imageWidth"]) as f64, u32_of(&x["imageHeight"]) as f64);
                let s = if w.max(hh) > 1280.0 { 1280.0 / w.max(hh) } else { 1.0 };
                h.width = (w * s).round() as u32;
                h.height = (hh * s).round() as u32;
                if h.file_url.is_empty() {
                    return None;
                }
            }
            Some(h)
        })
        .collect()
}

fn freesound(v: &Value, kind: Kind) -> Vec<Hit> {
    let Some(rs) = v["results"].as_array() else { return Vec::new() };
    rs.iter()
        .filter_map(|r| {
            let lic_url = s_of(&r["license"]);
            let user = s_of(&r["username"]);
            Some(Hit {
                id: format!("freesound:{}", s_of(&r["id"])),
                source: Some(Source::Freesound),
                kind,
                title: s_of(&r["name"]),
                author_url: format!("https://freesound.org/people/{}/", enc(&user)),
                author: user,
                page_url: s_of(&r["url"]),
                licence: Some(Licence::parse(&lic_url)),
                licence_name: cc_name(&lic_url),
                licence_url: lic_url,
                thumb_url: s_of(&r["images"]["waveform_m"]),
                file_url: {
                    let u = s_of(&r["previews"]["preview-hq-mp3"]);
                    if u.is_empty() {
                        return None;
                    }
                    u
                },
                duration: f64_of(&r["duration"]),
                ..Default::default()
            })
        })
        .collect()
}

// ------------------------------------------------------------------------------------------- second requests

/// The file to download for a hit whose search result does not name it (Archive, NASA video / audio), and its
/// size in bytes when the site says (0 = unknown). Blocking.
pub fn resolve(hit: &Hit) -> Result<(String, u64), NetError> {
    match hit.source {
        Some(Source::Archive) => {
            let body = super::get(&Request { url: format!("https://archive.org/metadata/{}", enc(&hit.detail)), headers: Vec::new() }, 20)?;
            archive_file(&hit.detail, hit.kind, &String::from_utf8_lossy(&body)).ok_or(NetError::Failed("no playable file in this item".into()))
        }
        Some(Source::Nasa) => {
            let body = super::get(&Request { url: hit.detail.clone(), headers: Vec::new() }, 20)?;
            nasa_file(hit.kind, &String::from_utf8_lossy(&body), false)
                .map(|u| (u, 0))
                .ok_or(NetError::Failed("no file in this item".into()))
        }
        _ if !hit.file_url.is_empty() => Ok((hit.file_url.clone(), 0)),
        _ => Err(NetError::Failed("nothing to download".into())),
    }
}

/// The small file the hover preview reads (falls back to the full file). Blocking (Archive / NASA ask once).
pub fn preview_url(hit: &Hit) -> Result<String, NetError> {
    if !hit.preview_url.is_empty() {
        return Ok(hit.preview_url.clone());
    }
    if hit.source == Some(Source::Nasa) && hit.kind == Kind::Video {
        let body = super::get(&Request { url: hit.detail.clone(), headers: Vec::new() }, 20)?;
        if let Some(u) = nasa_file(hit.kind, &String::from_utf8_lossy(&body), true) {
            return Ok(u);
        }
    }
    if !hit.file_url.is_empty() {
        return Ok(safe_url(&hit.file_url));
    }
    resolve(hit).map(|(u, _)| u)
}

/// From an Archive `/metadata/<id>` response: the file to download (an h.264 .mp4 up to 1.5 GB for a film,
/// an .mp3 for audio) and its size.
pub fn archive_file(id: &str, kind: Kind, text: &str) -> Option<(String, u64)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let files = v["files"].as_array()?;
    let size = |f: &Value| s_of(&f["size"]).parse::<u64>().unwrap_or(0);
    let name = |f: &Value| s_of(&f["name"]);
    let ends = |f: &Value, exts: &[&str]| {
        let n = name(f).to_ascii_lowercase();
        exts.iter().any(|e| n.ends_with(e))
    };
    let pick: Option<&Value> = if kind == Kind::Video {
        files
            .iter()
            .filter(|f| ends(f, &[".mp4"]) && size(f) <= 1_500_000_000)
            .max_by_key(|f| size(f))
            .or_else(|| files.iter().filter(|f| ends(f, &[".mov", ".mpeg", ".mpg", ".ogv", ".avi"])).min_by_key(|f| size(f)))
    } else {
        files
            .iter()
            .find(|f| ends(f, &[".mp3"]))
            .or_else(|| files.iter().find(|f| ends(f, &[".ogg", ".flac", ".wav", ".m4a"])))
    };
    let f = pick?;
    let path: String = name(f).split('/').map(enc).collect::<Vec<_>>().join("/");
    Some((format!("https://archive.org/download/{}/{path}", enc(id)), size(f)))
}

/// From a NASA item's `collection.json` (a list of URLs): the file to download, or with `preview` the small one
/// for the hover preview.
pub fn nasa_file(kind: Kind, text: &str, preview: bool) -> Option<String> {
    let list: Vec<String> = serde_json::from_str(text).ok()?;
    let find = |suffixes: &[&str]| {
        suffixes.iter().find_map(|s| list.iter().find(|u| u.to_ascii_lowercase().ends_with(s))).map(|u| safe_url(u))
    };
    match kind {
        Kind::Video if preview => find(&["~preview.mp4", "~mobile.mp4", "~small.mp4", "~medium.mp4"]),
        Kind::Video => find(&["~large.mp4", "~orig.mp4", "~medium.mp4", ".mp4", ".mov"]),
        Kind::Image => find(&["~orig.jpg", "~orig.png", "~large.jpg", ".jpg", ".png"]),
        _ => find(&["~orig.mp3", ".mp3", "~orig.m4a", ".m4a", "~orig.wav", ".wav"]),
    }
}
