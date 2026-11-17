//! Social profiles: the fake-but-yours account the guide preview shows (handle, name, avatar, caption,
//! counts, sample comments). Kept in settings.json, several per network, one active per network. No
//! credentials, ever: the only network access is `fetch`, which downloads ONE PUBLIC page (and its og:image)
//! with curl.exe and only when the user clicks "Fetch from my public profile" (or runs `profile.fetch`
//! through the assistant guard) - never at start-up, never while drawing.

use crate::media::Frame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const NETS: [(&str, &str); 6] = [
    ("youtube", "YouTube"),
    ("tiktok", "TikTok"),
    ("instagram", "Instagram"),
    ("facebook", "Facebook"),
    ("snapchat", "Snapchat"),
    ("x", "X"),
];

pub fn net_name(net: &str) -> &'static str {
    NETS.iter().find(|n| n.0 == net).map_or("Profile", |n| n.1)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SampleComment {
    pub name: String,
    pub text: String,
    pub likes: String,
    pub when: String,
}

impl Default for SampleComment {
    fn default() -> Self {
        Self { name: String::new(), text: String::new(), likes: String::new(), when: "2h".into() }
    }
}

/// One profile. Every number is a plain string ("12.4K") the user types: it is a mock, not a statistic.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SocialProfile {
    pub id: String,
    /// youtube, tiktok, instagram, facebook, snapchat, x.
    pub net: String,
    /// Label in the lists.
    pub name: String,
    pub handle: String,
    pub display: String,
    pub verified: bool,
    pub followers: String,
    pub caption: String,
    pub tags: String,
    pub sound: String,
    pub likes: String,
    pub comments: String,
    pub shares: String,
    pub saves: String,
    pub views: String,
    pub link: String,
    /// 96x96 PNG, base64 (so the profile travels with settings.json). Empty = the placeholder circle.
    pub avatar: String,
    /// Where the data came from ("Fetched 2026-11-13 from https://..."), shown under the fields.
    pub note: String,
    pub comment_list: Vec<SampleComment>,
}

impl Default for SocialProfile {
    fn default() -> Self {
        Self::sample("youtube")
    }
}

fn c(name: &str, text: &str, likes: &str, when: &str) -> SampleComment {
    SampleComment { name: name.into(), text: text.into(), likes: likes.into(), when: when.into() }
}

impl SocialProfile {
    /// A made-up profile for `net`: what the preview shows until the user makes their own.
    pub fn sample(net: &str) -> SocialProfile {
        SocialProfile {
            id: format!("sample-{net}"),
            net: net.into(),
            name: "Sample".into(),
            handle: "yourchannel".into(),
            display: "Your Channel".into(),
            verified: false,
            followers: "12.4K".into(),
            caption: "I tried the hardest challenge on the internet and you will not believe how it ended".into(),
            tags: "#gaming #challenge".into(),
            sound: "Original sound - yourchannel".into(),
            likes: "12K".into(),
            comments: "482".into(),
            shares: "1.2K".into(),
            saves: "3.1K".into(),
            views: "184K".into(),
            link: String::new(),
            avatar: String::new(),
            note: String::new(),
            comment_list: vec![
                c("blocky_ben", "the ending got me", "1.2K", "2h"),
                c("noob_slayer99", "how many tries did this take??", "384", "5h"),
                c("mia.plays", "I did not expect that jump", "97", "1d"),
                c("rbx_fan", "more obbies please", "41", "3d"),
            ],
        }
    }
    /// "@yourchannel" / "yourchannel" the way the network writes a handle.
    pub fn handle_text(&self) -> String {
        let h = self.handle.trim().trim_start_matches('@');
        match self.net.as_str() {
            "youtube" | "x" => format!("@{h}"),
            _ => h.to_string(),
        }
    }
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() { self.display.clone() } else { self.name.clone() }
    }
    /// Caption with the hashtags after it, the way every network shows them.
    pub fn caption_full(&self) -> String {
        match (self.caption.trim(), self.tags.trim()) {
            (c, "") => c.to_string(),
            ("", t) => t.to_string(),
            (c, t) => format!("{c} {t}"),
        }
    }
    /// The text a silhouette field draws. `title` is the project's name (long form); `time` is the player clock.
    pub fn field(&self, f: &str, title: &str, time: &str) -> String {
        match f {
            "handle" => self.handle_text(),
            "name" => self.display.clone(),
            "caption" => self.caption_full(),
            "sound" => self.sound.clone(),
            "title" => title.to_string(),
            "subs" => format!("{} {}", self.followers, if self.net == "youtube" { "subscribers" } else { "followers" }),
            "views" => format!("{} views", self.views),
            "time" => time.to_string(),
            "now" => time.split(" / ").next().unwrap_or("").to_string(),
            "likes" => self.likes.clone(),
            "comments" => self.comments.clone(),
            "shares" => self.shares.clone(),
            "saves" => self.saves.clone(),
            _ => String::new(),
        }
    }
    pub fn count(&self, n: &str) -> &str {
        match n {
            "likes" => &self.likes,
            "comments" => &self.comments,
            "shares" => &self.shares,
            "saves" => &self.saves,
            _ => "",
        }
    }
}

impl crate::settings::Settings {
    /// The profile the preview uses for `net`: the chosen one, else the first of that network.
    pub fn active_profile(&self, net: &str) -> Option<&SocialProfile> {
        let by_id = self.social_active.get(net).and_then(|id| self.social_profiles.iter().find(|p| &p.id == id && p.net == net));
        by_id.or_else(|| self.social_profiles.iter().find(|p| p.net == net))
    }
    /// A profile id nobody uses yet.
    pub fn new_profile_id(&self) -> String {
        (1..).map(|i| format!("p{i}")).find(|id| !self.social_profiles.iter().any(|p| &p.id == id)).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- avatars

enum Slot {
    Pending,
    Ready(Option<Arc<Frame>>),
}
static CACHE: Mutex<Option<HashMap<u64, Slot>>> = Mutex::new(None);

fn avatar_key(p: &SocialProfile) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.avatar.hash(&mut h);
    h.finish()
}

/// True while a background decode of this profile's picture is still running.
pub fn avatar_pending(p: &SocialProfile) -> bool {
    !p.avatar.is_empty() && CACHE.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|m| matches!(m.get(&avatar_key(p)), Some(Slot::Pending))))
}

/// Straight-alpha RGBA of a profile's avatar, decoded once per distinct picture. `wait` false never
/// blocks: the first call starts a background decode and returns None until it is ready (see
/// `avatar_pending`); a picture that cannot be decoded stays None.
pub fn avatar(p: &SocialProfile, wait: bool) -> Option<Arc<Frame>> {
    if p.avatar.is_empty() {
        return None;
    }
    let key = avatar_key(p);
    let decode = |b64: &str| -> Option<Arc<Frame>> {
        let png = crate::engine::webcap::b64_decode(b64.as_bytes());
        crate::engine::html::decode_png(&png).ok().map(Arc::new)
    };
    {
        let mut g = CACHE.lock().ok()?;
        match g.get_or_insert_with(HashMap::new).get(&key) {
            Some(Slot::Ready(f)) => return f.clone(),
            Some(Slot::Pending) if !wait => return None,
            _ => {}
        }
        if !wait {
            g.as_mut()?.insert(key, Slot::Pending);
            let b64 = p.avatar.clone();
            std::thread::spawn(move || {
                let f = decode(&b64);
                if let Ok(mut g) = CACHE.lock() {
                    g.get_or_insert_with(HashMap::new).insert(key, Slot::Ready(f));
                }
            });
            return None;
        }
    }
    let f = decode(&p.avatar);
    if let Ok(mut g) = CACHE.lock() {
        g.get_or_insert_with(HashMap::new).insert(key, Slot::Ready(f.clone()));
    }
    f
}

/// Any picture file (or an https image that was downloaded to `path`) -> the 96x96 cropped PNG, base64.
pub fn avatar_from_file(path: &std::path::Path) -> Result<String, String> {
    let exe = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let out = crate::media::ffpipe::command(&exe)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=96:96:force_original_aspect_ratio=increase,crop=96:96",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() || out.stdout.len() < 64 {
        return Err("that file is not a picture ffmpeg can read".into());
    }
    Ok(crate::engine::webcap::b64_encode(&out.stdout))
}

// ---------------------------------------------------------------- fetching one public page

/// How many times anything in this module started curl: the tests assert it stays 0 until a click.
pub static NET_CALLS: AtomicUsize = AtomicUsize::new(0);

/// What a fetch found, shown to the user before it is applied to a profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fetched {
    /// The public page that was requested.
    pub url: String,
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub display: String,
    pub followers: String,
    /// Base64 96x96 PNG of the og:image ("" when there was none or it could not be read).
    pub avatar: String,
}

/// A handle ("@YouTube", "nasa") or a URL -> the public profile URL of `net`. Only http(s).
pub fn profile_url(net: &str, input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("enter your public profile URL or handle".into());
    }
    if s.contains("://") {
        return if s.starts_with("https://") || s.starts_with("http://") { Ok(s.to_string()) } else { Err("only http(s) links".into()) };
    }
    let h = s.trim_start_matches('@').trim_matches('/');
    if h.is_empty() || h.contains(|c: char| c.is_whitespace() || "/?#\\\"'<>:(),;=".contains(c)) {
        return Err("that is not a handle".into());
    }
    Ok(match net {
        "youtube" => format!("https://www.youtube.com/@{h}"),
        "tiktok" => format!("https://www.tiktok.com/@{h}"),
        "instagram" => format!("https://www.instagram.com/{h}/"),
        "facebook" => format!("https://www.facebook.com/{h}"),
        "snapchat" => format!("https://www.snapchat.com/@{h}"),
        "x" => format!("https://x.com/{h}"),
        _ => return Err("unknown network".into()),
    })
}

fn entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// `name="value"` / `name='value'` inside one tag (names compared case-insensitively).
fn attr(tag: &str, name: &str) -> Option<String> {
    let low = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || low.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let v = rest[1..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let end = v[1..].find(q)?;
                return Some(entities(&v[1..1 + end]));
            }
            return Some(entities(v.split(|c: char| c.is_whitespace() || c == '>').next()?));
        }
        from = at + name.len();
    }
    None
}

/// (title, description, image) from the og:/twitter: meta tags, falling back to `<title>` and the plain description.
pub fn parse_meta(html: &str) -> (String, String, String) {
    let (mut title, mut desc, mut image) = (String::new(), String::new(), String::new());
    let low = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find("<meta") {
        let start = from + i;
        let end = low[start..].find('>').map_or(low.len(), |e| start + e);
        let tag = &html[start..end];
        let key = attr(tag, "property").or_else(|| attr(tag, "name")).unwrap_or_default().to_ascii_lowercase();
        let content = attr(tag, "content").unwrap_or_default();
        let slot = match key.as_str() {
            "og:title" | "twitter:title" => Some(&mut title),
            "og:description" | "twitter:description" | "description" => Some(&mut desc),
            "og:image" | "twitter:image" => Some(&mut image),
            _ => None,
        };
        if let Some(s) = slot.filter(|s| s.is_empty()) {
            *s = content;
        }
        from = end.max(start + 5);
    }
    if title.is_empty() {
        if let (Some(a), Some(b)) = (low.find("<title>"), low.find("</title>")) {
            if a + 7 <= b {
                title = entities(html[a + 7..b].trim());
            }
        }
    }
    (title, desc, image)
}

/// "NASA (@nasa) • Instagram photos and videos" -> "NASA".
pub fn display_from_title(t: &str) -> String {
    let mut s = t.trim();
    for sep in [" (@", " | ", " • ", " · ", " - YouTube", " on TikTok", " / X", " (", " on "] {
        if let Some(i) = s.find(sep) {
            s = &s[..i];
        }
    }
    s.trim().to_string()
}

/// "80M Followers" / "12.4K subscribers" / "1,234 fans" -> "80M" (the number before the word).
pub fn followers_from(text: &str) -> String {
    let toks: Vec<&str> = text.split_whitespace().collect();
    for (i, w) in toks.iter().enumerate().skip(1) {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase();
        if matches!(w.as_str(), "subscribers" | "followers" | "fans") {
            let n = toks[i - 1].trim_matches(|c: char| !c.is_alphanumeric());
            if n.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return n.to_string();
            }
        }
    }
    String::new()
}

fn curl_args(url: &str) -> Result<std::process::Command, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) links are fetched".into());
    }
    let curl = crate::media::ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found (it ships with Windows 10 1803+)")?;
    NET_CALLS.fetch_add(1, Ordering::SeqCst);
    let mut cmd = crate::media::ffpipe::command(&curl);
    cmd.args(["--silent", "--show-error", "--location", "--max-time", "20", "--max-filesize", "4000000"])
        .args(["--proto", "=http,https", "--proto-redir", "=http,https"])
        .args(["-A", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36"])
        .arg(url)
        .stdin(std::process::Stdio::null());
    Ok(cmd)
}

/// Download ONE public page (no cookies, no login) and its og:image. Blocking (2 requests, 20 s each):
/// background thread, or a tool call the user allowed.
pub fn fetch(net: &str, input: &str) -> Result<Fetched, String> {
    let url = profile_url(net, input)?;
    let page = curl_args(&url)?.output().map_err(|e| format!("curl: {e}"))?;
    if !page.status.success() {
        let msg = String::from_utf8_lossy(&page.stderr).trim().to_string();
        return Err(if msg.is_empty() { format!("{url}: the page did not load") } else { msg });
    }
    let html = String::from_utf8_lossy(&page.stdout);
    let (title, description, image_url) = parse_meta(&html);
    let mut f = Fetched {
        url,
        display: display_from_title(&title),
        followers: followers_from(&format!("{title} {description}")),
        title,
        description,
        image_url,
        avatar: String::new(),
    };
    if !f.image_url.is_empty() {
        let tmp = std::env::temp_dir().join(format!("se-avatar-{}.img", std::process::id()));
        let r = curl_args(&f.image_url).and_then(|mut c| c.arg("-o").arg(&tmp).output().map_err(|e| e.to_string()));
        if r.is_ok_and(|o| o.status.success()) {
            f.avatar = avatar_from_file(&tmp).unwrap_or_default();
        }
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(f)
}

impl Fetched {
    /// What the profile gets: the picture, name and follower count that were found (empty = keep).
    pub fn apply(&self, p: &mut SocialProfile) {
        if !self.avatar.is_empty() {
            p.avatar = self.avatar.clone();
        }
        if !self.display.is_empty() {
            p.display = self.display.clone();
        }
        if !self.followers.is_empty() {
            p.followers = self.followers.clone();
        }
        p.link = self.url.clone();
        p.note = format!("Fetched from {} (public page, no login)", self.url);
    }
    /// Plain lines for the confirmation panel and the MCP reply.
    pub fn report(&self) -> Vec<String> {
        let none = |s: &str| if s.is_empty() { "(not found)".to_string() } else { s.to_string() };
        vec![
            format!("Requested: {}", self.url),
            format!("og:title: {}", none(&self.title)),
            format!("og:description: {}", none(&self.description.chars().take(160).collect::<String>())),
            format!("og:image: {}", none(&self.image_url)),
            format!("Name used: {}", none(&self.display)),
            format!("Followers read: {}", none(&self.followers)),
            format!("Avatar: {}", if self.avatar.is_empty() { "(none: kept the old one)" } else { "downloaded and cropped to 96x96" }),
        ]
    }
}
