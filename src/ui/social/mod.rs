//! Social guides, data first. `assets/guides.json` (embedded; 20 KB) lists, per platform and device
//! class, where the platform's own UI covers the video: zones (danger / caution / safe / crop), the
//! silhouettes of that UI (action column, avatar, caption, progress...) and how the comments / description
//! / share / more sheets move the video. Every rectangle was measured on the real logged-out pages (see
//! each platform's `src` and `ver`) and is stored in dp on a reference screen (`Layout.ref`), so it can be
//! drawn over a 9:16 video (reference = the frame) and, anchored to the nearest screen edge, on any phone.
//!
//! An updated file dropped in `%APPDATA%\SimpleEditor\library\guides\guides.json` replaces the embedded
//! platforms with the same id (and adds new ones) without a release; Settings ▸ Social can fetch the
//! library's copy on a click.
//!
//! `scene` builds draw lists (`Op`) from that data + the active `profile::SocialProfile`; `paint` replays
//! them in the viewer (and runs the interactive device preview), `raster` replays them into a bitmap for
//! `render.frame` / `guides.preview`. Nothing here touches the project or the export.

pub mod device;
pub mod frame;
pub mod icons;
pub mod paint;
pub mod profile;
pub mod raster;
pub mod scene;
pub mod sheets;
pub mod vertical;
pub mod youtube;

use eframe::egui::{self, pos2, vec2, Pos2, Rect};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

pub const EMBEDDED: &str = include_str!("../../../assets/guides.json");

/// What the viewer shows for the chosen platform.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Danger / caution / safe regions with percentages.
    #[default]
    Safe,
    /// The platform's UI drawn as silhouettes with the active profile filled in.
    Ui,
    /// The video inside a drawn device, with the platform UI and its sheets interactive.
    Device,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Safe, Mode::Ui, Mode::Device];
    pub fn id(self) -> &'static str {
        match self {
            Mode::Safe => "safe",
            Mode::Ui => "ui",
            Mode::Device => "device",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Safe => "Safe zone",
            Mode::Ui => "UI preview",
            Mode::Device => "Device preview",
        }
    }
    pub fn from_id(s: &str) -> Mode {
        Mode::ALL.into_iter().find(|m| m.id() == s).unwrap_or_default()
    }
}

/// The desktop long-form player's three layouts (mini-player is not drawn).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ViewMode {
    #[default]
    Default,
    Theatre,
    Full,
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::Default, ViewMode::Theatre, ViewMode::Full];
    pub fn id(self) -> &'static str {
        match self {
            ViewMode::Default => "default",
            ViewMode::Theatre => "theatre",
            ViewMode::Full => "fullscreen",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ViewMode::Default => "Default view",
            ViewMode::Theatre => "Theatre mode",
            ViewMode::Full => "Fullscreen",
        }
    }
    pub fn from_id(s: &str) -> Option<ViewMode> {
        ViewMode::ALL.into_iter().find(|m| m.id() == s || (m.id() == "fullscreen" && s == "full"))
    }
    pub fn idx(self) -> f32 {
        self as u8 as f32
    }
    /// The key in a platform's `vw` table.
    pub fn key(self) -> &'static str {
        match self {
            ViewMode::Default => "default",
            ViewMode::Theatre => "theatre",
            ViewMode::Full => "full",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Data {
    #[allow(dead_code)] // the file's format version: read by nothing yet, kept so a newer file can be refused
    pub v: u32,
    /// "Verified on" date of the whole file (each platform has its own `ver` too).
    pub verified: String,
    pub devices: Vec<Device>,
    pub platforms: Vec<Platform>,
}

/// One device class: its screen in dp, corner radius, cut-out and the system bars the platform UI sits under.
#[derive(Clone, Debug, Deserialize)]
pub struct Device {
    pub id: String,
    pub n: String,
    pub w: f32,
    pub h: f32,
    pub rad: f32,
    /// "island" | "punch" | "none"
    pub cut: String,
    /// Status bar height, home-indicator height, navigation-bar height (dp).
    pub st: f32,
    pub home: f32,
    pub nav: f32,
    /// "phone" | "tablet" | "desktop" | "tv"
    pub k: String,
    /// "ios" | "android" | "" (the status bar and system UI drawn).
    #[serde(default)]
    pub os: String,
    /// Frame colour, "#rrggbb".
    #[serde(default)]
    pub col: String,
    /// 1 = the status bar shows the battery percentage (Galaxy).
    #[serde(default)]
    pub pct: u8,
    /// Home-button models: the face round the screen in dp, [side, top, bottom] in portrait (the home button
    /// sits in the bottom one). All zero = an all-screen device with an even bezel.
    #[serde(default)]
    pub face: [f32; 3],
    /// Pixels per dp, and the panel's real pixels when they are not w x h x scale (Plus models, minis).
    #[serde(default)]
    pub sc: f32,
    #[serde(default)]
    pub px: [u32; 2],
    /// Turned to landscape (`turned`).
    #[serde(skip)]
    pub land: bool,
}

impl Device {
    /// The same device turned to landscape (top to the left): width and height swap, and the status / navigation
    /// bars go away the way a full-screen video hides them (an iPhone keeps its home indicator).
    pub fn turned(&self) -> Device {
        let mut d = self.clone();
        (d.w, d.h) = (self.h, self.w);
        d.st = 0.0;
        d.nav = 0.0;
        d.home = if self.home > 0.0 { 21.0 } else { 0.0 };
        d.land = true;
        d
    }
    /// A home button under the screen (no notch, no island, no home indicator).
    pub fn home_button(&self) -> bool {
        self.face[1] > 0.0
    }
    /// The screen's real pixels (portrait).
    pub fn pixels(&self) -> (u32, u32) {
        let (w, h) = if self.land { (self.h, self.w) } else { (self.w, self.h) };
        if self.px[0] > 0 {
            return (self.px[0], self.px[1]);
        }
        let s = if self.sc > 0.0 { self.sc } else { 1.0 };
        ((w * s).round() as u32, (h * s).round() as u32)
    }
    /// What the picker shows: the size the layout uses ("393 x 852 pt @3x", "1470 x 956 pt", blank for a desktop
    /// at 100 %) and the real pixels ("1179 x 2556 px").
    pub fn resolution(&self) -> (String, String) {
        let (w, h) = if self.land { (self.h, self.w) } else { (self.w, self.h) };
        let (pw, ph) = self.pixels();
        let px = format!("{pw} x {ph} px");
        let sc = (self.sc.max(1.0) * 1000.0).round() / 1000.0;
        let logical = match self.k.as_str() {
            "desktop" | "tv" if pw == w as u32 => String::new(),
            "desktop" | "tv" => format!("{w} x {h} at {}%", (sc * 100.0).round()),
            _ if self.os == "android" => format!("{w} x {h} dp @{sc}x"),
            _ => format!("{w} x {h} pt @{sc}x"),
        };
        (logical, px)
    }
    /// The picker's group: the maker and the kind.
    pub fn group(&self) -> &'static str {
        match (self.k.as_str(), self.os.as_str(), self.cut.as_str()) {
            ("phone", "ios", _) if self.home_button() => "iPhone with home button",
            ("phone", "ios", "notch") => "iPhone with notch",
            ("phone", "ios", _) => "iPhone with Dynamic Island",
            ("phone", _, _) if self.n.starts_with("Pixel") => "Google Pixel",
            ("phone", _, _) if self.n.starts_with("Galaxy") => "Samsung Galaxy",
            ("phone", _, _) => "Android",
            ("tablet", _, _) if self.home_button() => "iPad with home button",
            ("tablet", _, _) => "iPad",
            ("desktop", _, _) => "Desktop and laptop",
            _ => "TV",
        }
    }
}

/// The picker's groups, in order.
pub const DEVICE_GROUPS: [&str; 10] = [
    "iPhone with Dynamic Island",
    "iPhone with notch",
    "iPhone with home button",
    "Google Pixel",
    "Samsung Galaxy",
    "Android",
    "iPad",
    "iPad with home button",
    "Desktop and laptop",
    "TV",
];

impl Device {
    /// Phones and tablets can turn; a browser window and a TV cannot.
    pub fn turns(&self) -> bool {
        matches!(self.k.as_str(), "phone" | "tablet")
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Platform {
    pub id: String,
    pub n: String,
    /// The account family the active profile comes from: youtube, tiktok, instagram, facebook, snapchat, x.
    pub net: String,
    /// Page shape in device preview: "full" (full-screen vertical), "feed" (card in a feed), "watch" (player + page).
    pub pg: String,
    /// Aspect of the platform's own player (w, h).
    pub a: [u32; 2],
    pub ver: String,
    pub src: String,
    pub lay: BTreeMap<String, Layout>,
    pub st: Sheets,
    /// Desktop player layouts (default, theatre, full) as `[x0, y0, x1, y1]` px of a 1920x1080 browser
    /// viewport (fullscreen: of the whole screen). Long form only.
    #[serde(default)]
    pub vw: BTreeMap<String, [f32; 4]>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Layout {
    /// Reference screen (dp) the rectangles below were measured on.
    #[serde(rename = "ref")]
    pub refsz: [f32; 2],
    /// "cover" (video fills the screen, cropped) or "contain" (fits inside, bars).
    #[serde(default)]
    pub fit: String,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub ui: Vec<El>,
    #[serde(default)]
    pub safe: Option<[f32; 4]>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Zone {
    /// d = danger (UI covers it), c = caution (sometimes covered), s = safe, x = crop (a grid thumbnail keeps only this).
    pub k: String,
    pub r: [f32; 4],
    pub l: String,
}

/// One silhouette element. Flat on purpose: one struct, a few optional fields per `t` (type).
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct El {
    /// scr (gradient scrim), bar, ico (icon), ava (avatar), txt, pil (pill), prg (progress), chp (chip).
    pub t: String,
    pub r: [f32; 4],
    /// Icon glyph name.
    pub i: String,
    /// Profile field a text shows (handle, name, caption, sound, title, time...).
    pub f: String,
    /// Literal text (wins over `f`).
    pub x: String,
    /// Label under an icon.
    pub lbl: String,
    /// Count field under an icon (likes, comments, shares, saves).
    pub n: String,
    /// What a tap opens in the device preview: comments, desc, share, more, sound, profile, none.
    pub tap: String,
    /// Colour key: w (white), r (red), a (accent).
    pub c: String,
    /// Scrim direction: u = dark at the bottom, d = dark at the top.
    pub d: String,
    pub a: f32,
    /// Max collapsed lines of a caption.
    pub l: u32,
    /// Font size (dp).
    pub s: f32,
    pub b: u8,
    pub more: u8,
    /// Sits outside the video frame (desktop action columns): skipped over the editor's video.
    pub o: u8,
    pub plus: u8,
    pub fill: u8,
    pub k: u8,
    pub sq: u8,
    /// Alignment: "c" centred.
    pub al: String,
    /// 1 = the icon sits on a translucent dark circle (Shorts' action column).
    pub bg: u8,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Sheets {
    pub comments: Sheet,
    pub desc: Sheet,
    pub share: Sheet,
    pub more: Sheet,
}

/// How a sheet opens: `m` = "shrink" (the video shrinks into the part above the sheet), "cover" (the
/// sheet covers the video, which dims) or "page" (long form: the page scrolls); `sh` = the sheet's top
/// edge as a fraction of the screen height; `ms` = the slide duration; `src` = "est" when not measured.
#[derive(Clone, Debug, Deserialize)]
pub struct Sheet {
    pub m: String,
    pub sh: f32,
    pub ms: u32,
    #[serde(default)]
    pub src: String,
}

impl Data {
    pub fn platform(&self, id: &str) -> Option<&Platform> {
        self.platforms.iter().find(|p| p.id == id)
    }
    pub fn device(&self, id: &str) -> &Device {
        self.devices.iter().find(|d| d.id == id).or_else(|| self.devices.iter().find(|d| d.id == "iphone")).expect("guides.json has devices")
    }
}

impl Platform {
    /// The layout a device class uses: the desktop one on a desktop (and on a tablet for long form), else the phone one.
    pub fn layout(&self, dev: &Device) -> &Layout {
        let want = if dev.k == "desktop" || (dev.k == "tablet" && self.pg == "watch") { "desktop" } else { "phone" };
        self.lay.get(want).or_else(|| self.lay.values().next()).expect("every platform has a layout")
    }
    pub fn aspect(&self) -> f32 {
        self.a[0] as f32 / self.a[1].max(1) as f32
    }
}

/// Parse and validate a guides file.
pub fn parse(text: &str) -> Result<Data, String> {
    let d: Data = serde_json::from_str(text).map_err(|e| format!("guides.json: {e}"))?;
    validate(&d)?;
    Ok(d)
}

/// Every rectangle ordered and finite, aspect and reference sizes non-zero, a safe zone and a phone or
/// desktop layout per platform, devices usable.
pub fn validate(d: &Data) -> Result<(), String> {
    if d.devices.is_empty() || d.platforms.is_empty() {
        return Err("guides.json: no devices or platforms".into());
    }
    for dev in &d.devices {
        if !(dev.w >= 100.0 && dev.h >= 100.0) {
            return Err(format!("device {}: bad size", dev.id));
        }
    }
    let ok = |r: &[f32; 4], rf: [f32; 2]| {
        r.iter().all(|v| v.is_finite()) && r[0] < r[2] && r[1] < r[3] && r[1] >= -rf[1] * 0.2 && r[3] <= rf[1] * 1.2
    };
    for p in &d.platforms {
        if p.a[0] == 0 || p.a[1] == 0 || p.lay.is_empty() {
            return Err(format!("platform {}: no aspect or layout", p.id));
        }
        for (name, l) in &p.lay {
            if !(l.refsz[0] > 0.0 && l.refsz[1] > 0.0) {
                return Err(format!("{}/{name}: bad ref", p.id));
            }
            for z in &l.zones {
                if !ok(&z.r, l.refsz) || !matches!(z.k.as_str(), "d" | "c" | "s" | "x") {
                    return Err(format!("{}/{name}: zone '{}' is not a valid rectangle", p.id, z.l));
                }
            }
            for e in &l.ui {
                if !ok(&e.r, l.refsz) {
                    return Err(format!("{}/{name}: element {} {} is not a valid rectangle", p.id, e.t, e.i));
                }
            }
            if l.safe.is_some_and(|s| !ok(&s, l.refsz)) {
                return Err(format!("{}/{name}: bad safe rect", p.id));
            }
        }
    }
    Ok(())
}

/// `%APPDATA%\SimpleEditor\library\guides\guides.json`.
pub fn user_path() -> std::path::PathBuf {
    crate::engine::library::root().join("guides").join("guides.json")
}

/// Embedded data with the user's file merged over it (same platform / device id replaces, new ids are added).
pub fn merged(user: Option<&str>) -> Data {
    let mut d = parse(EMBEDDED).expect("embedded guides.json is valid (tested)");
    if let Some(Ok(u)) = user.map(parse) {
        d.verified = u.verified;
        for p in u.platforms {
            match d.platforms.iter_mut().find(|q| q.id == p.id) {
                Some(q) => *q = p,
                None => d.platforms.push(p),
            }
        }
        for v in u.devices {
            match d.devices.iter_mut().find(|q| q.id == v.id) {
                // an older file has no scale or pixels: keep the embedded ones
                Some(q) if v.sc == 0.0 => *q = Device { sc: q.sc, px: q.px, ..v },
                Some(q) => *q = v,
                None => d.devices.push(v),
            }
        }
    }
    d
}

/// Validate `text` as a guides file and install it as the user's copy (atomically), then reload.
/// Returns the platform count of the merged data.
pub fn install_user_file(text: &str) -> Result<usize, String> {
    parse(text)?;
    let p = user_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let tmp = p.with_extension("json.part");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(reload().platforms.len())
}

static DATA: RwLock<Option<Arc<Data>>> = RwLock::new(None);

/// The guides in use: loaded on first use (never at start-up), then kept until `reload`.
pub fn data() -> Arc<Data> {
    if let Some(d) = DATA.read().ok().and_then(|g| g.clone()) {
        return d;
    }
    reload()
}

/// Re-read the user's file (after Settings fetched an update, or the user edited it).
pub fn reload() -> Arc<Data> {
    let d = Arc::new(merged(std::fs::read_to_string(user_path()).ok().as_deref()));
    if let Ok(mut g) = DATA.write() {
        *g = Some(d.clone());
    }
    d
}

/// Maps a layout's reference rectangles onto a target box: each coordinate sticks to the nearest edge of
/// the reference screen (so the action column stays at the right edge and the caption at the bottom on a
/// taller or wider phone); an element centred on the reference screen stays centred.
#[derive(Clone, Copy, Debug)]
pub struct Map {
    pub refsz: Vec2,
    /// Target size in dp.
    pub size: Vec2,
    /// Where the target's top-left is, and how many points one dp is.
    pub origin: Pos2,
    pub k: f32,
    /// dp added under the status bar / above the home indicator to every element anchored to the top / bottom
    /// edge (a notch or island pushes the app's own UI down; the reference was measured without one).
    pub top: f32,
    pub bot: f32,
    /// dp added inside the left / right screen edge (the margin real apps keep from a rounded screen).
    pub side: f32,
}
pub type Vec2 = eframe::egui::Vec2;

impl Map {
    pub fn new(refsz: [f32; 2], size: Vec2, origin: Pos2, k: f32) -> Map {
        Map { refsz: vec2(refsz[0], refsz[1]), size, origin, k, top: 0.0, bot: 0.0, side: 0.0 }
    }
    pub fn insets(mut self, top: f32, bot: f32, side: f32) -> Map {
        (self.top, self.bot, self.side) = (top, bot, side);
        self
    }
    /// A target the same shape as the reference, `rect` points on screen: the plain scale (an overlay on a video).
    pub fn fill(refsz: [f32; 2], rect: Rect) -> Map {
        let k = rect.width() / refsz[0].max(1.0);
        Map::new(refsz, vec2(refsz[0], refsz[1]), rect.min, k)
    }
    fn axis(r0: f32, r1: f32, rf: f32, sz: f32, side: f32) -> (f32, f32) {
        let d = sz - rf;
        let mid = (r0 + r1) * 0.5 / rf;
        // a narrow element in the middle stays in the middle; everything else sticks to its nearest edge
        if (0.4..=0.6).contains(&mid) && (r1 - r0) < rf * 0.7 {
            return (r0 + d * 0.5, r1 + d * 0.5);
        }
        // the side margin keeps buttons off a curved screen edge; an edge that is the reference screen's own edge
        // (a full-width scrim, bar or progress bar) stays on the screen's edge
        let edge = |v: f32| match v {
            _ if v <= 0.0 => v,
            _ if v >= rf => v + d,
            _ if v < rf * 0.5 => v + side,
            _ => v + d - side,
        };
        (edge(r0), edge(r1))
    }
    /// dp rectangle on the target (before scaling).
    pub fn dp(&self, r: [f32; 4]) -> [f32; 4] {
        let (x0, x1) = Self::axis(r[0], r[2], self.refsz.x, self.size.x, self.side);
        let (y0, y1) = Self::axis(r[1], r[3], self.refsz.y, self.size.y, 0.0);
        let sh = |v: f32, orig: f32| if orig < self.refsz.y * 0.5 { v + self.top } else { v - self.bot };
        [x0, sh(y0, r[1]), x1, sh(y1, r[3])]
    }
    /// An element keeps its size: its vertical centre sticks to the top edge (header) or the bottom edge (the rest),
    /// so a button that straddles the middle of the reference screen is not stretched by a taller phone. Horizontally each
    /// edge sticks on its own (a caption block grows with the width).
    pub fn rect_el(&self, r: [f32; 4]) -> Rect {
        let [x0, _, x1, _] = self.dp(r);
        let (h, c) = (r[3] - r[1], (r[1] + r[3]) * 0.5);
        let (rf, d) = (self.refsz.y, self.size.y - self.refsz.y);
        // header elements (the top ~30 %) stay with the top edge; everything below moves as one block with the
        // bottom edge, so a column of buttons keeps its spacing on a taller phone
        let cy = if c < rf * 0.3 { c + self.top } else { c + d - self.bot };
        Rect::from_min_max(
            pos2(self.origin.x + x0 * self.k, self.origin.y + (cy - h * 0.5) * self.k),
            pos2(self.origin.x + x1 * self.k, self.origin.y + (cy + h * 0.5) * self.k),
        )
    }
    pub fn rect(&self, r: [f32; 4]) -> Rect {
        let [x0, y0, x1, y1] = self.dp(r);
        Rect::from_min_max(
            pos2(self.origin.x + x0 * self.k, self.origin.y + y0 * self.k),
            pos2(self.origin.x + x1 * self.k, self.origin.y + y1 * self.k),
        )
    }
}

/// What the viewer's menus ask the app to change.
#[derive(Clone, Debug, PartialEq)]
pub enum Set {
    Platform(Option<String>),
    Mode(Mode),
    Device(String),
    Worst(bool),
    View(ViewMode),
    Keep(bool),
    Render(bool),
    /// (network, profile id)
    Profile(String, String),
}

impl Set {
    pub fn apply(self, s: &mut crate::settings::Settings) {
        match self {
            Set::Platform(p) => s.guide = p,
            Set::Mode(m) => s.guide_mode = m.id().into(),
            Set::Device(d) => s.guide_device = d,
            Set::Worst(w) => s.guide_worst = w,
            Set::View(v) => s.guide_view = v.id().into(),
            Set::Keep(k) => s.guides_keep_aspect = k,
            Set::Render(r) => s.guide_render = r,
            Set::Profile(net, id) => {
                s.social_active.insert(net, id);
            }
        }
        s.save();
    }
}

/// The "Social Guides" menu: platform, mode, device, profile and the options. Shared by the viewer's
/// right-click and the Window menu.
pub fn menu_rows(ui: &mut eframe::egui::Ui, s: &crate::settings::Settings) -> Option<Set> {
    use crate::ui::menu;
    let d = data();
    let mut out = None;
    if menu::check(ui, s.guide.is_none(), "Off", "").clicked() {
        out = Some(Set::Platform(None));
    }
    for p in &d.platforms {
        if menu::check(ui, s.guide.as_deref() == Some(p.id.as_str()), &p.n, "").clicked() {
            out = Some(Set::Platform(Some(p.id.clone())));
        }
    }
    ui.separator();
    for m in Mode::ALL {
        if menu::check(ui, Mode::from_id(&s.guide_mode) == m, m.name(), "").clicked() {
            out = Some(Set::Mode(m));
        }
    }
    ui.separator();
    menu::sub(ui, None, "Device", |ui| {
        for dev in &d.devices {
            if menu::check(ui, s.guide_device == dev.id, &dev.n, "").clicked() {
                out = Some(Set::Device(dev.id.clone()));
            }
        }
    });
    if s.guide.as_deref().and_then(|g| d.platform(g)).is_some_and(|p| p.pg == "watch") {
        menu::sub(ui, None, "Player view (desktop)", |ui| {
            for m in ViewMode::ALL {
                if menu::check(ui, ViewMode::from_id(&s.guide_view).unwrap_or_default() == m, m.name(), "").clicked() {
                    out = Some(Set::View(m));
                }
            }
        });
    }
    let net = s.guide.as_deref().and_then(|g| d.platform(g)).map(|p| p.net.clone());
    if let Some(net) = net {
        let profiles: Vec<_> = s.social_profiles.iter().filter(|p| p.net == net).collect();
        menu::sub(ui, None, "Preview as", |ui| {
            let active = s.active_profile(&net).map(|p| p.id.clone());
            if profiles.is_empty() {
                ui.weak("No profile yet: Settings ▸ Social");
            }
            for p in profiles {
                if menu::check(ui, active.as_deref() == Some(p.id.as_str()), &p.label(), "").clicked() {
                    out = Some(Set::Profile(net.clone(), p.id.clone()));
                }
            }
        });
    }
    ui.separator();
    if menu::check(ui, s.guide_worst, "Worst case of all platforms", "").clicked() {
        out = Some(Set::Worst(!s.guide_worst));
    }
    if menu::check(ui, s.guides_keep_aspect, "Maintain Aspect Ratio", "").clicked() {
        out = Some(Set::Keep(!s.guides_keep_aspect));
    }
    if menu::check(ui, s.guide_render, "Draw in render.frame and contact sheets", "").clicked() {
        out = Some(Set::Render(!s.guide_render));
    }
    out
}

#[cfg(test)]
mod tests;
