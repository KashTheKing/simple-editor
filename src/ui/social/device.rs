//! Device preview: the video inside a drawn phone / tablet / desktop browser, the platform UI on top of it
//! exactly as measured, and the sheets (comments, description, share, more, profile, sound) that open on a
//! tap - with the platform's own behaviour for the video underneath (Shorts-style apps shrink it into the
//! part above the sheet, others cover and dim it, the long-form page keeps the player where it is).
//! A pure function of (data, profile, state): `build` returns draw lists and the tap targets.

use super::profile::SocialProfile;
use super::scene::*;
use super::sheets::sheet_body;
use super::*;
use eframe::egui::{Color32, Rect};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sheet {
    Comments,
    Desc,
    Share,
    More,
    Profile,
    Sound,
}

impl Sheet {
    pub const ALL: [Sheet; 6] = [Sheet::Comments, Sheet::Desc, Sheet::Share, Sheet::More, Sheet::Profile, Sheet::Sound];
    pub fn id(self) -> &'static str {
        match self {
            Sheet::Comments => "comments",
            Sheet::Desc => "desc",
            Sheet::Share => "share",
            Sheet::More => "more",
            Sheet::Profile => "profile",
            Sheet::Sound => "sound",
        }
    }
    /// A silhouette's `tap` name, or the MCP `state` argument ("default" and unknown = None).
    pub fn from_id(s: &str) -> Option<Sheet> {
        Sheet::ALL.into_iter().find(|k| k.id() == s)
    }
    pub fn metrics(self, st: &Sheets) -> &super::Sheet {
        match self {
            Sheet::Comments => &st.comments,
            Sheet::Share => &st.share,
            Sheet::More => &st.more,
            Sheet::Desc | Sheet::Profile | Sheet::Sound => &st.desc,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Open(Sheet),
    /// A player-view button (theatre, fullscreen) in the long-form control bar.
    View(ViewMode),
    Close,
    /// A tap on a sheet's own body: swallowed, so the page under it is not hit.
    Block,
    /// The player's play / pause button (or a tap on a full-screen video): plays / pauses the editor.
    Play,
    /// A progress bar: the click's x along this rectangle seeks the editor there.
    Seek(Rect),
    /// The captions (CC) button, or the settings menu's Subtitles/CC row: shows / hides the project's subtitles.
    Captions,
    /// The player's settings (gear) button: opens / closes its menu.
    Menu,
    /// The speaker: mutes / unmutes the preview's own speaker state (the editor's audio is not touched).
    Mute,
    /// The volume slider: the click's x along this rectangle sets the preview's volume.
    Volume(Rect),
}

/// What the player shows apart from the page: the editor's captions switch, buffering, the settings menu and
/// the speaker. Kept by the viewer, not the project (except `cc`, which is the project's subtitles switch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    /// The project's subtitles are shown (`Project::show_subtitles`): the CC button's state.
    pub cc: bool,
    /// The editor's player is waiting for frames: the platform's own spinner shows.
    pub buffering: bool,
    /// Seconds on a free-running clock, for spinners (only read while `buffering`).
    pub t: f32,
    /// The settings menu is open.
    pub menu: bool,
    pub muted: bool,
    /// 0..1.
    pub volume: f32,
}

impl Default for Player {
    fn default() -> Self {
        Player { cc: false, buffering: false, t: 0.0, menu: false, muted: false, volume: 1.0 }
    }
}

pub struct View<'a> {
    pub plat: &'a Platform,
    pub dev: &'a Device,
    pub prof: &'a SocialProfile,
    pub title: &'a str,
    pub progress: f32,
    pub clock: &'a str,
    pub playing: bool,
    /// The project's aspect (w / h).
    pub pa: f32,
    /// The sheet shown (kept while it slides back down) and how far open it is, 0..1 eased.
    pub open: Option<Sheet>,
    pub e: f32,
    /// The desktop player view as a glide over the layout index (0 default, 1 theatre, 2 fullscreen).
    pub vm: f32,
    /// The device is turned to landscape (phones and tablets only).
    pub land: bool,
    pub pl: Player,
}

pub struct Scene {
    /// Drawn before the video (backdrop, device body, page).
    pub under: Vec<Op>,
    /// Drawn after it (platform UI, status bar, sheets, bezel corners).
    pub over: Vec<Op>,
    /// Where the video is drawn now, and which part of the frame (uv).
    pub video: Rect,
    pub uv: Rect,
    /// The video's rectangle with nothing open: what the preview renders at.
    pub base: Rect,
    /// Tap targets, topmost last.
    pub hits: Vec<(Rect, Hit)>,
    /// The device's rounded screen (the browser window on desktop), the body around it, the bezel width (points).
    pub screen: Rect,
    pub outer: Rect,
    pub bezel: f32,
    /// How many leading `under` ops are backdrop / device body / window chrome, and how many trailing `over`
    /// ops are the corner mask: everything between them is drawn inside `screen`.
    pub chrome_under: usize,
    pub mask_over: usize,
}

/// The rectangle an op covers (None for text and clip markers).
pub fn op_bbox(op: &Op) -> Option<Rect> {
    Some(match op {
        Op::Rect { r, line, .. } => r.expand(line * 0.5),
        Op::Circle { c, r, line, .. } => Rect::from_center_size(*c, vec2(2.0 * r + line, 2.0 * r + line)),
        Op::Line { a, b, w, .. } => Rect::from_two_pos(*a, *b).expand(w * 0.5),
        Op::Poly { p, .. } => p.iter().fold(Rect::NOTHING, |r, q| r.union(Rect::from_min_max(*q, *q))),
        Op::Grad { r, .. } => *r,
        Op::Avatar { c, r } => Rect::from_center_size(*c, vec2(2.0 * r, 2.0 * r)),
        Op::Icon { r, .. } => *r,
        Op::Text { .. } | Op::Clip(_) => return None,
    })
}

/// Keep only what lies inside `clip`: a shape that leaves it is dropped, text whose origin is outside is dropped.
/// (Page content that does not fit the screen - a related video half below the fold - simply is not drawn.)
fn cull(ops: &mut Vec<Op>, clip: Rect) {
    let clip = clip.expand(0.75);
    ops.retain(|op| match (op_bbox(op), op) {
        (Some(bb), _) => clip.contains_rect(bb),
        (None, Op::Text { p, s, .. }) => clip.contains(*p) && p.y + s * 0.9 <= clip.max.y,
        _ => true,
    });
}

pub(super) fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A silhouette's `tap` as a hit: a sheet, or a player view.
pub(super) fn tap_hit(t: &str) -> Option<Hit> {
    match t {
        "theatre" => Some(Hit::View(ViewMode::Theatre)),
        "fullscreen" => Some(Hit::View(ViewMode::Full)),
        _ => Sheet::from_id(t).map(Hit::Open),
    }
}

pub(super) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
pub(super) fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(pos2(lerp(a.min.x, b.min.x, t), lerp(a.min.y, b.min.y, t)), pos2(lerp(a.max.x, b.max.x, t), lerp(a.max.y, b.max.y, t)))
}
/// The largest `aspect` rectangle inside `r`, centred.
pub fn contain(r: Rect, aspect: f32) -> Rect {
    let (w, h) = if r.width() / r.height().max(0.001) > aspect { (r.height() * aspect, r.height()) } else { (r.width(), r.width() / aspect.max(0.001)) };
    Rect::from_center_size(r.center(), vec2(w, h))
}
pub(super) const FULL: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 1.0, y: 1.0 } };

/// (rectangle, uv): `cover` fills `region` and crops the picture to its shape, else the picture is contained.
pub(super) fn place(region: Rect, cover: bool, pa: f32) -> (Rect, Rect) {
    let ra = region.width() / region.height().max(0.001);
    if cover && pa < 1.0 {
        let uv = if pa > ra {
            let w = ra / pa;
            Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
        } else {
            let h = pa / ra;
            Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
        };
        (region, uv)
    } else {
        (contain(region, pa), FULL)
    }
}

pub(super) fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    Color32::from_rgb((f(5.0) * 255.0) as u8, (f(3.0) * 255.0) as u8, (f(1.0) * 255.0) as u8)
}
pub fn name_color(name: &str) -> Color32 {
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    hsv((h % 360) as f32 / 360.0, 0.45, 0.62)
}
pub(super) fn host(net: &str) -> &'static str {
    match net {
        "youtube" => "youtube.com",
        "tiktok" => "tiktok.com",
        "instagram" => "instagram.com",
        "facebook" => "facebook.com",
        "snapchat" => "snapchat.com",
        _ => "x.com",
    }
}

/// Builds the whole device picture inside `area` (the viewer).
pub fn build(v: &View, area: Rect) -> Scene {
    let land = v.land && v.dev.turns();
    let turned;
    let dev: &Device = if land {
        turned = v.dev.turned();
        &turned
    } else {
        v.dev
    };
    let plat = v.plat;
    let mut under = B::new();
    let mut over = B::new();
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    // a TV shows the platform the way a desktop browser's page does, without the browser
    let desktop = matches!(dev.k.as_str(), "desktop" | "tv");
    let ink = Color32::WHITE;

    // ---- the device body: phone / tablet / TV, or a browser window
    let backdrop = rgba(22, 22, 26, 255);
    under.rect(area, 0.0, backdrop);
    // an even margin round the whole device, never less than 10 points
    let inner = area.shrink((area.width().min(area.height()) * 0.04).max(10.0).min(area.width().min(area.height()) * 0.2));
    // the long-form page fades out as the player goes fullscreen
    // a TV plays long form full screen
    let vm = if dev.k == "tv" { 2.0 } else { v.vm };
    let fade = if desktop && plat.pg == "watch" { 1.0 - smooth(vm - 1.0) } else { 1.0 };
    let body = super::frame::body(&mut under, dev, land, inner);
    let (k, screen, outer, bezel) = (body.k, body.screen, body.outer, body.bezel);
    if dev.k == "desktop" {
        under.a = fade;
        let url = if plat.pg == "watch" { "youtube.com/watch?v=dQw4w9WgXcQ".to_string() } else { format!("{}/{}", host(&plat.net), v.prof.handle_text()) };
        super::frame::browser(&mut under, outer, k, &plat.n, &url);
        under.a = 1.0;
    }
    let chrome_under = under.ops.len();
    let navh = if dev.nav >= 40.0 { dev.nav * k } else { 0.0 };
    let app = Rect::from_min_max(screen.min, pos2(screen.max.x, screen.max.y - navh));
    let st_h = if desktop { 0.0 } else { dev.st * k };

    // ---- the page and the video
    let lay = plat.layout(dev);
    let sheet_m = v.open.map(|s| s.metrics(&plat.st).clone());
    let e = if v.open.is_some() { v.e.clamp(0.0, 1.0) } else { 0.0 };
    let cx = Ctx { prof: v.prof, title: v.title, progress: v.progress, clock: v.clock, playing: v.playing };
    let pg = Pg { v, dev, plat, lay, screen, outer, app, k, st_h, desktop, fade, vm, e, sheet_m: sheet_m.as_ref(), cx: &cx, ink };
    let placed = match plat.pg.as_str() {
        "watch" if desktop => super::youtube::watch_desktop(&pg, &mut under, &mut over, &mut hits),
        "watch" => super::youtube::watch_phone(&pg, &mut under, &mut over, &mut hits),
        "feed" => super::vertical::feed(&pg, &mut under, &mut over, &mut hits),
        _ => super::vertical::full(&pg, &mut under, &mut over, &mut hits),
    };
    let Placed { base, video, uv, sheet_top } = placed;

    // ---- the sheet (phones, tablets, and the watch / feed pages on every device)
    let side_panel = desktop && matches!(plat.pg.as_str(), "full" | "watch");
    if let (Some(s), Some(m)) = (v.open, sheet_m.as_ref()) {
        if e > 0.0 && !side_panel {
            let region = if plat.pg == "full" && !desktop && dev.k == "phone" { app } else { app };
            let top_final = match m.m.as_str() {
                "page" => sheet_top.max(region.min.y),
                _ => region.min.y + region.height() * m.sh,
            }
            .max(region.min.y);
            let top = lerp(region.max.y, top_final, e);
            if m.m != "shrink" {
                over.rect(Rect::from_min_max(region.min, pos2(region.max.x, top.max(region.min.y))).intersect(region), 0.0, Color32::from_black_alpha((120.0 * e) as u8));
                hits.push((Rect::from_min_max(region.min, pos2(region.max.x, top_final)), Hit::Close));
            }
            let mut panel = Rect::from_min_max(pos2(region.min.x, top), region.max);
            // a phone turned landscape keeps the sheet clear of the camera cut-out and the rounded corners
            if dev.k == "phone" && region.width() > region.height() {
                (panel.min.x, panel.max.x) = (region.min.x + 48.0 * k, region.max.x - 48.0 * k);
            }
            sheet_body(&mut over, v, s, panel, k, true, &mut hits);
        }
    }

    // ---- system chrome on top of everything (phones and tablets)
    super::frame::system_bars(&mut over, dev, land, screen, k, Color32::WHITE);
    super::frame::cutout(&mut over, dev, land, screen, k);
    // ---- the corner mask: a real mask over everything, not smaller shapes. One ring in the backdrop colour from
    // the rounded edge outwards (anything drawn into a square corner of the screen disappears under it), then the
    // bezel ring in the body colour, then its outline: nothing is visible outside the body's rounded outline.
    // everything drawn so far lies inside the clip: whatever would poke out (page content below the fold, a
    // sheet corner) is dropped here, before the mask goes on
    let clip = if desktop { outer } else { screen };
    let tail = under.ops.split_off(chrome_under);
    let mut tail = tail;
    cull(&mut tail, clip);
    under.ops.extend(tail);
    cull(&mut over.ops, clip);
    // a tap target below the fold (a feed caption on a small phone) ends at the screen's edge: the body, the
    // home button and the backdrop round it take no taps
    hits.retain_mut(|(r, _)| {
        *r = r.intersect(clip);
        r.is_positive()
    });
    let mask_start = over.ops.len();
    over.a = 1.0;
    super::frame::mask(&mut over, dev, land, &body, backdrop);
    let mask_over = over.ops.len() - mask_start;
    Scene { under: under.ops, over: over.ops, video, uv, base, hits, screen, outer, bezel, chrome_under, mask_over }
}

/// Everything a page builder reads: the view, the (turned) device, where the screen is, the sheet state.
#[derive(Clone, Copy)]
pub struct Pg<'a> {
    pub v: &'a View<'a>,
    pub dev: &'a Device,
    pub plat: &'a Platform,
    pub lay: &'a Layout,
    /// The device's screen, the body round it (the browser window on desktop), the part above a three-button bar.
    pub screen: Rect,
    pub outer: Rect,
    pub app: Rect,
    /// Points per dp, and the status bar's height in points (0 on desktop / TV / landscape).
    pub k: f32,
    pub st_h: f32,
    /// A desktop browser or a TV.
    pub desktop: bool,
    /// The long-form page's opacity (it fades as the player goes fullscreen) and the player view glide (0..2).
    pub fade: f32,
    pub vm: f32,
    /// How far the open sheet is (0..1) and its metrics.
    pub e: f32,
    pub sheet_m: Option<&'a super::Sheet>,
    pub cx: &'a Ctx<'a>,
    pub ink: Color32,
}

/// Where a page put the video: `base` with nothing open (what the preview renders at), `video` now, the part
/// of the frame shown (`uv`), and the top of an open sheet for "page" sheets.
pub struct Placed {
    pub base: Rect,
    pub video: Rect,
    pub uv: Rect,
    pub sheet_top: f32,
}
