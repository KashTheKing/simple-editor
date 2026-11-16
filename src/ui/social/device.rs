//! Device preview: the video inside a drawn phone / tablet / desktop browser, the platform UI on top of it
//! exactly as measured, and the sheets (comments, description, share, more, profile, sound) that open on a
//! tap - with the platform's own behaviour for the video underneath (Shorts-style apps shrink it into the
//! part above the sheet, others cover and dim it, the long-form page keeps the player where it is).
//! A pure function of (data, profile, state): `build` returns draw lists and the tap targets.

use super::profile::{net_name, SocialProfile};
use super::scene::*;
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
}

pub struct View<'a> {
    pub plat: &'a Platform,
    pub dev: &'a Device,
    pub prof: &'a SocialProfile,
    pub title: &'a str,
    pub progress: f32,
    pub clock: &'a str,
    /// The project's aspect (w / h).
    pub pa: f32,
    /// The sheet shown (kept while it slides back down) and how far open it is, 0..1 eased.
    pub open: Option<Sheet>,
    pub e: f32,
    /// The desktop player view as a glide over the layout index (0 default, 1 theatre, 2 fullscreen).
    pub vm: f32,
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

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A silhouette's `tap` as a hit: a sheet, or a player view.
fn tap_hit(t: &str) -> Option<Hit> {
    match t {
        "theatre" => Some(Hit::View(ViewMode::Theatre)),
        "fullscreen" => Some(Hit::View(ViewMode::Full)),
        _ => Sheet::from_id(t).map(Hit::Open),
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(pos2(lerp(a.min.x, b.min.x, t), lerp(a.min.y, b.min.y, t)), pos2(lerp(a.max.x, b.max.x, t), lerp(a.max.y, b.max.y, t)))
}
/// The largest `aspect` rectangle inside `r`, centred.
pub fn contain(r: Rect, aspect: f32) -> Rect {
    let (w, h) = if r.width() / r.height().max(0.001) > aspect { (r.height() * aspect, r.height()) } else { (r.width(), r.width() / aspect.max(0.001)) };
    Rect::from_center_size(r.center(), vec2(w, h))
}
const FULL: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 1.0, y: 1.0 } };

/// (rectangle, uv): `cover` fills `region` and crops the picture to its shape, else the picture is contained.
fn place(region: Rect, cover: bool, pa: f32) -> (Rect, Rect) {
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

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    Color32::from_rgb((f(5.0) * 255.0) as u8, (f(3.0) * 255.0) as u8, (f(1.0) * 255.0) as u8)
}
fn name_color(name: &str) -> Color32 {
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    hsv((h % 360) as f32 / 360.0, 0.45, 0.62)
}
fn host(net: &str) -> &'static str {
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
    let (dev, plat) = (v.dev, v.plat);
    let mut under = B::new();
    let mut over = B::new();
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    let desktop = dev.k == "desktop";
    let ink = Color32::WHITE;

    // ---- the device body: phone / tablet bezel, or a browser window
    let backdrop = rgba(22, 22, 26, 255);
    under.rect(area, 0.0, backdrop);
    // an even margin round the whole device (bezel included), never less than 10 points
    let inner = area.shrink((area.width().min(area.height()) * 0.04).max(10.0).min(area.width().min(area.height()) * 0.2));
    // the long-form page fades out as the player goes fullscreen
    let fade = if desktop && plat.pg == "watch" { 1.0 - smooth(v.vm - 1.0) } else { 1.0 };
    let (k, app, screen);
    let (outer, bezel, chrome_under);
    if desktop {
        let bar = 40.0;
        k = (inner.width() / dev.w).min(inner.height() / (dev.h + bar));
        let win = Rect::from_center_size(inner.center(), vec2(dev.w, dev.h + bar) * k);
        (outer, bezel) = (win, 0.0);
        under.a = fade;
        under.rect(win, dev.rad * k, rgba(36, 36, 40, 255));
        let at = |x: f32, y: f32| pos2(win.min.x + x * k, win.min.y + y * k);
        for (i, c) in [rgba(255, 95, 86, 255), rgba(255, 189, 46, 255), rgba(39, 201, 63, 255)].into_iter().enumerate() {
            under.circle(at(20.0 + i as f32 * 20.0, bar / 2.0), 6.0 * k, c);
        }
        under.rect(Rect::from_min_max(at(100.0, 8.0), at(330.0, bar)), 8.0 * k, rgba(15, 15, 17, 255));
        under.text(at(116.0, 13.0), 12.0 * k, white(0.8), false, &ellipsize(&plat.n, 12.0 * k, false, 190.0 * k), 0);
        under.rect(Rect::from_min_max(at(350.0, 7.0), at(dev.w - 120.0, bar - 7.0)), 13.0 * k, rgba(15, 15, 17, 255));
        let url = format!("{}/{}", host(&plat.net), v.prof.handle_text());
        under.text(at(368.0, 12.0), 13.0 * k, white(0.7), false, &url, 0);
        screen = Rect::from_min_max(at(0.0, bar), win.max);
        under.rect(screen, 0.0, rgba(15, 15, 15, 255));
        under.a = 1.0;
        app = screen;
        chrome_under = under.ops.len();
    } else {
        let bz = if dev.k == "tablet" { 12.0 } else { 7.0 };
        k = (inner.width() / (dev.w + 2.0 * bz)).min(inner.height() / (dev.h + 2.0 * bz));
        outer = Rect::from_center_size(inner.center(), vec2(dev.w + 2.0 * bz, dev.h + 2.0 * bz) * k);
        bezel = bz * k;
        under.rect(outer, (dev.rad + bz) * k, rgba(12, 12, 14, 255));
        under.frame(outer, (dev.rad + bz) * k, 1.0, rgba(80, 80, 88, 255));
        screen = outer.shrink(bz * k);
        under.rect(screen, dev.rad * k, Color32::BLACK);
        chrome_under = under.ops.len();
        let navh = if dev.nav >= 40.0 { dev.nav * k } else { 0.0 };
        app = Rect::from_min_max(screen.min, pos2(screen.max.x, screen.max.y - navh));
    }
    let at = |x: f32, y: f32| pos2(screen.min.x + x * k, screen.min.y + y * k);
    let st_h = if desktop { 0.0 } else { dev.st * k };

    // ---- the page and the video
    let lay = plat.layout(dev);
    let sheet_m = v.open.map(|s| s.metrics(&plat.st).clone());
    let e = if v.open.is_some() { v.e.clamp(0.0, 1.0) } else { 0.0 };
    let cx = Ctx { prof: v.prof, title: v.title, progress: v.progress, clock: v.clock };
    let (base, video, uv, sheet_top);
    match plat.pg.as_str() {
        "watch" if desktop => {
            // the desktop player: default (video left, sidebar right), theatre (wide player, sidebar below)
            // and fullscreen (browser and page gone); sizes are px of a 1920x1080 viewport, measured
            // one scale for both axes: the 1920x1080 viewport keeps its proportions in the page
            let (kx, ky) = (screen.width() / 1920.0, screen.width() / 1920.0);
            let win = outer;
            let get = |m: ViewMode| -> Rect {
                let r = plat.vw.get(m.key()).copied().unwrap_or(match m {
                    ViewMode::Default => [16.0, 68.0, 1360.0, 824.0],
                    ViewMode::Theatre => [0.0, 56.0, 1920.0, 967.0],
                    ViewMode::Full => [0.0, 0.0, 1920.0, 1080.0],
                });
                if m == ViewMode::Full {
                    win
                } else {
                    Rect::from_min_max(pos2(screen.min.x + r[0] * kx, screen.min.y + r[1] * ky), pos2(screen.min.x + r[2] * kx, screen.min.y + r[3] * ky))
                }
            };
            let f = v.vm.clamp(0.0, 2.0);
            let i = (f.floor() as usize).min(1);
            let (a, b) = (ViewMode::ALL[i], ViewMode::ALL[i + 1]);
            let player = lerp_rect(get(a), get(b), smooth(f - i as f32));
            let dflt = get(ViewMode::Default);
            let (r, u) = place(player, false, v.pa);
            (base, video, uv) = (r, r, u);
            // the page: header, related videos, title / channel / description / comments
            let mut pg = B::new();
            pg.a = fade;
            let hh = 56.0 * ky;
            pg.rect(Rect::from_min_size(screen.min, vec2(screen.width(), hh)), 0.0, rgba(15, 15, 15, 255));
            glyph(&mut pg, "menu", pos2(screen.min.x + 30.0 * kx, screen.min.y + hh / 2.0), 9.0 * kx, white(0.85));
            pg.rect(Rect::from_center_size(pos2(screen.min.x + 120.0 * kx, screen.min.y + hh / 2.0), vec2(90.0, 20.0) * kx), 5.0 * kx, rgba(255, 255, 255, 40));
            pg.rect(Rect::from_center_size(pos2(screen.center().x, screen.min.y + hh / 2.0), vec2(560.0, 36.0) * kx), 18.0 * kx, rgba(255, 255, 255, 22));
            pg.circle(pos2(screen.max.x - 36.0 * kx, screen.min.y + hh / 2.0), 15.0 * kx, rgba(70, 70, 76, 255));
            pg.avatar(pos2(screen.max.x - 36.0 * kx, screen.min.y + hh / 2.0), 14.0 * kx);
            let t = smooth(f.min(1.0));
            let cards_x = dflt.max.x + 24.0 * kx;
            let cards_y = lerp(dflt.min.y, player.max.y + 16.0 * ky, t);
            for n in 0..6 {
                let y = cards_y + n as f32 * 100.0 * ky;
                if y + 90.0 * ky > screen.max.y {
                    break;
                }
                pg.rect(Rect::from_min_size(pos2(cards_x, y), vec2(170.0, 94.0) * kx), 6.0 * kx, rgba(60, 60, 66, 255));
                pg.rect(Rect::from_min_size(pos2(cards_x + 182.0 * kx, y + 8.0 * ky), vec2(150.0, 10.0) * kx), 4.0 * kx, rgba(90, 90, 96, 255));
                pg.rect(Rect::from_min_size(pos2(cards_x + 182.0 * kx, y + 28.0 * ky), vec2(100.0, 9.0) * kx), 4.0 * kx, rgba(70, 70, 76, 255));
            }
            let content = Rect::from_min_size(pos2(dflt.min.x, player.min.y), vec2(dflt.width(), player.height()));
            let mut page_hits = Vec::new();
            watch_page(&mut pg, v, content, app, kx, true, &mut page_hits);
            if fade > 0.5 {
                hits.extend(page_hits);
            }
            under.ops.extend(pg.ops);
            under.rect(player, 0.0, Color32::BLACK);
            // the control bar sits at fixed px size on the player box, not on the video inside it
            let map = Map::new(lay.refsz, vec2(player.width() / kx, player.height() / kx), player.min, kx);
            let tap = draw_ui(&mut over, lay, &map, &cx, false);
            if e <= 0.0 {
                hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
            }
            sheet_top = player.max.y;
        }
        "watch" => {
            // player on top, the page under it
            let (col, _top) = if desktop || dev.k == "tablet" {
                let w = app.width() * if desktop { 0.66 } else { 1.0 };
                (Rect::from_min_size(pos2(app.min.x + app.width() * 0.04, app.min.y + 16.0 * k), vec2(w - app.width() * 0.04, 0.0)), 0.0)
            } else {
                (Rect::from_min_size(pos2(app.min.x, app.min.y + st_h), vec2(app.width(), 0.0)), 0.0)
            };
            let player = Rect::from_min_size(col.min, vec2(col.width(), col.width() * 9.0 / 16.0));
            let (r, u) = place(player, false, v.pa);
            (base, video, uv) = (r, r, u);
            let map = Map::fill(lay.refsz, player);
            under.rect(player, 0.0, Color32::BLACK);
            if desktop {
                // related videos: generic cards
                let x0 = player.max.x + app.width() * 0.02;
                for i in 0..5 {
                    let y = app.min.y + 16.0 * k + i as f32 * 100.0 * k;
                    if y + 90.0 * k > app.max.y {
                        break;
                    }
                    under.rect(Rect::from_min_size(pos2(x0, y), vec2(150.0 * k, 84.0 * k)), 6.0 * k, rgba(60, 60, 66, 255));
                    under.rect(Rect::from_min_size(pos2(x0 + 160.0 * k, y + 6.0 * k), vec2(130.0 * k, 9.0 * k)), 4.0 * k, rgba(90, 90, 96, 255));
                    under.rect(Rect::from_min_size(pos2(x0 + 160.0 * k, y + 24.0 * k), vec2(90.0 * k, 8.0 * k)), 4.0 * k, rgba(70, 70, 76, 255));
                }
            }
            let mut page = B::new();
            watch_page(&mut page, v, player, app, k, desktop, &mut hits);
            under.ops.extend(page.ops);
            let mut ui = B::new();
            let tap = draw_ui(&mut ui, lay, &map, &cx, false);
            over.ops.extend(ui.ops);
            if e <= 0.0 {
                hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
            }
            sheet_top = player.max.y;
        }
        "feed" => {
            let col = if desktop || dev.k == "tablet" {
                Rect::from_center_size(app.center(), vec2(app.width().min(460.0 * k), app.height())).intersect(app)
            } else {
                app
            };
            let hdr = 52.0 * k;
            let top = col.min.y + if desktop { 16.0 * k } else { st_h };
            let media = Rect::from_min_size(pos2(col.min.x, top + hdr), vec2(col.width(), col.width() / plat.aspect()));
            under.rect(col.intersect(Rect::from_min_max(pos2(col.min.x, top), pos2(col.max.x, col.max.y))), 0.0, rgba(0, 0, 0, 255));
            // header
            under.circle(pos2(col.min.x + 28.0 * k, top + hdr / 2.0), 17.0 * k, rgba(70, 70, 76, 255));
            under.avatar(pos2(col.min.x + 28.0 * k, top + hdr / 2.0), 16.0 * k);
            under.text(pos2(col.min.x + 54.0 * k, top + hdr / 2.0 - 8.0 * k), 14.0 * k, ink, true, &v.prof.handle_text(), 0);
            glyph(&mut under, "more", pos2(col.max.x - 24.0 * k, top + hdr / 2.0), 9.0 * k, ink);
            let (r, u) = place(media, false, v.pa);
            (base, video, uv) = (r, r, u);
            under.rect(media, 0.0, rgba(18, 18, 20, 255));
            let map = Map::fill(lay.refsz, media);
            let tap = draw_ui(&mut over, lay, &map, &cx, false);
            // footer: actions, likes, caption
            let fy = media.max.y + 10.0 * k;
            let mut foot = B::new();
            let share_glyph = if plat.net == "instagram" { "plane" } else { "share" };
            for (i, n) in ["like", "comment", share_glyph].iter().enumerate() {
                let c = pos2(col.min.x + (24.0 + i as f32 * 40.0) * k, fy + 14.0 * k);
                glyph(&mut foot, n, c, 11.0 * k, ink);
                let hit = Rect::from_center_size(c, vec2(36.0, 36.0) * k);
                match *n {
                    "comment" => hits.push((hit, Hit::Open(Sheet::Comments))),
                    "share" | "plane" => hits.push((hit, Hit::Open(Sheet::Share))),
                    _ => {}
                }
            }
            glyph(&mut foot, "save", pos2(col.max.x - 24.0 * k, fy + 14.0 * k), 10.0 * k, ink);
            foot.text(pos2(col.min.x + 14.0 * k, fy + 40.0 * k), 13.0 * k, ink, true, &format!("{} likes", v.prof.likes), 0);
            let s = 13.0 * k;
            let cw = col.width() - 28.0 * k;
            let (lines, cut) = collapse(wrap(&v.prof.caption_full(), s, false, cw), 2, s, false, cw, text_w("… more", s, false));
            let y0 = fy + 60.0 * k;
            foot.text(pos2(col.min.x + 14.0 * k, y0), s, ink, true, &v.prof.handle_text(), 0);
            for (i, l) in lines.iter().enumerate() {
                foot.text(pos2(col.min.x + 14.0 * k, y0 + (i as f32 + 1.2) * s * 1.3), s, ink, false, l, 0);
            }
            if cut {
                let x = col.min.x + 14.0 * k + text_w(lines.last().map_or("", |l| l), s, false);
                foot.text(pos2(x, y0 + (lines.len() as f32 + 0.2) * s * 1.3), s, white(0.6), false, "… more", 0);
            }
            hits.push((Rect::from_min_size(pos2(col.min.x, y0), vec2(col.width(), s * 4.0)), Hit::Open(Sheet::Desc)));
            over.ops.extend(foot.ops);
            if e <= 0.0 {
                hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
            }
            sheet_top = media.min.y;
        }
        _ => {
            // full-screen vertical: a phone fills its screen, a tablet / desktop shows a column
            let region = if desktop {
                let page = app.shrink(16.0 * k);
                let fw = contain(page, plat.aspect()).width();
                let _ = fw;
                contain(page, plat.aspect())
            } else if dev.k == "tablet" {
                contain(app.shrink(24.0 * k), plat.aspect())
            } else {
                app
            };
            let cover = lay.fit == "cover" && !desktop;
            let (b0, u0) = place(region, cover, v.pa);
            base = b0;
            let m = sheet_m.as_ref();
            let mut region_now = region;
            let (mut r, mut u) = (b0, u0);
            let st_top = if desktop { 0.0 } else { st_h };
            if desktop {
                // the side panel: the video slides left, the panel takes its place
                if e > 0.0 {
                    let dx = -(region.width() * 0.62) * e;
                    region_now = region.translate(vec2(dx, 0.0));
                    let (b1, u1) = place(region_now, false, v.pa);
                    (r, u) = (b1, u1);
                }
            } else if let Some(m) = m.filter(|m| m.m == "shrink") {
                let top = region.min.y + st_top + 6.0 * k;
                let target_region = Rect::from_min_max(pos2(region.min.x, top), pos2(region.max.x, (region.min.y + region.height() * m.sh - 8.0 * k).max(top + 40.0 * k)));
                let (tr, _) = place(target_region, false, v.pa);
                r = lerp_rect(b0, tr, e);
                u = lerp_rect(u0, FULL, e);
            }
            (video, uv) = (r, u);
            // the platform UI maps onto the region (the frame on desktop and tablet)
            let ui_alpha = match m {
                Some(m) if m.m == "shrink" => 1.0 - e,
                _ if desktop => 1.0,
                _ => 1.0,
            };
            let size = if desktop || dev.k == "tablet" { vec2(region.width() / (region.width() / lay.refsz[0]), region.height() / (region.width() / lay.refsz[0])) } else { region.size() / k };
            let kk = if desktop || dev.k == "tablet" { region.width() / lay.refsz[0] } else { k };
            let mut map = Map::new(lay.refsz, if desktop || dev.k == "tablet" { size } else { vec2(region.width() / k, region.height() / k) }, region_now.min, kk);
            if dev.k == "phone" {
                map = map.insets((dev.st - 24.0).max(0.0), dev.home + if dev.nav < 40.0 { dev.nav } else { 0.0 }, 4.0);
            }
            let mut ui = B::new();
            ui.a = ui_alpha;
            let tap = draw_ui(&mut ui, lay, &map, &cx, desktop);
            over.ops.extend(ui.ops);
            if e <= 0.0 {
                hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
            }
            sheet_top = region.min.y + region.height() * 0.0;
            if desktop && e > 0.0 {
                // side panel
                let pw = region.width() * 1.05;
                let x0 = lerp(app.max.x, region_now.max.x + 20.0 * k, e).min(app.max.x);
                let panel = Rect::from_min_max(pos2(x0, app.min.y + 8.0 * k), pos2((x0 + pw).min(app.max.x - 8.0 * k), app.max.y - 8.0 * k));
                if panel.width() > 20.0 {
                    let mut p = B::new();
                    if let Some(s) = v.open {
                        sheet_body(&mut p, v, s, panel, k, false, &mut hits);
                    }
                    over.ops.extend(p.ops);
                }
            }
        }
    }

    // ---- the sheet (phones, tablets, and the watch / feed pages on every device)
    let side_panel = desktop && plat.pg == "full";
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
            let panel = Rect::from_min_max(pos2(region.min.x, top), region.max);
            sheet_body(&mut over, v, s, panel, k, true, &mut hits);
        }
    }

    // ---- system chrome on top of everything (phones and tablets)
    if !desktop {
        let a = 0.95;
        let t = "9:41";
        if dev.cut == "island" {
            over.text(at(54.0 * 0.0 + 42.0, 18.0), 16.0 * k, white(a), true, t, 1);
        } else {
            over.text(at(24.0, 6.0 + (dev.st - 18.0).max(0.0) / 2.0 - 2.0), 14.0 * k, white(a), true, t, 0);
        }
        // signal, battery
        let by = dev.st * 0.5 - 3.0;
        for i in 0..4 {
            let hh = 3.0 + i as f32 * 2.0;
            over.rect(Rect::from_min_max(at(dev.w - 78.0 + i as f32 * 4.5, by + 8.0 - hh), at(dev.w - 75.0 + i as f32 * 4.5, by + 8.0)), 0.5 * k, white(a));
        }
        over.frame(Rect::from_min_max(at(dev.w - 52.0, by), at(dev.w - 28.0, by + 11.0)), 3.5 * k, 1.0, white(0.6));
        over.rect(Rect::from_min_max(at(dev.w - 50.5, by + 1.5), at(dev.w - 33.0, by + 9.5)), 2.0 * k, white(a));
        over.rect(Rect::from_min_max(at(dev.w - 26.5, by + 3.5), at(dev.w - 25.0, by + 7.5)), 0.7 * k, white(0.6));
        match dev.cut.as_str() {
            "island" => over.rect(Rect::from_center_size(at(dev.w / 2.0, 30.0), vec2(125.0, 37.0) * k), 18.5 * k, Color32::BLACK),
            "punch" => over.circle(at(dev.w / 2.0, 22.0), 9.0 * k, Color32::BLACK),
            _ => {}
        }
        if dev.home > 0.0 {
            over.rect(Rect::from_center_size(at(dev.w / 2.0, dev.h - 9.0), vec2(134.0, 5.0) * k), 2.5 * k, white(0.85));
        } else if dev.nav > 0.0 && dev.nav < 40.0 {
            over.rect(Rect::from_center_size(at(dev.w / 2.0, dev.h - 8.0), vec2(108.0, 4.0) * k), 2.0 * k, white(0.8));
        } else if dev.nav >= 40.0 {
            let nb = Rect::from_min_max(pos2(screen.min.x, screen.max.y - dev.nav * k), screen.max);
            over.rect(nb, 0.0, Color32::BLACK);
            let cy = nb.center().y;
            let h = 8.0 * k;
            over.poly(vec![pos2(screen.center().x - 80.0 * k, cy - h), pos2(screen.center().x - 80.0 * k, cy + h), pos2(screen.center().x - 80.0 * k - h * 1.4, cy)], white(0.8));
            over.ring(pos2(screen.center().x, cy), h, 1.5, white(0.8));
            over.frame(Rect::from_center_size(pos2(screen.center().x + 80.0 * k, cy), vec2(h * 1.7, h * 1.7)), 2.0 * k, 1.5, white(0.8));
        }
    }
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
    let mask_start = over.ops.len();
    over.a = 1.0;
    let rad = dev.rad * k;
    let w = bezel + rad * 0.6 + 2.0;
    let m = if desktop { outer } else { screen };
    over.frame(m.expand(w * 0.5), rad + w * 0.5, w, backdrop);
    if !desktop {
        over.frame(screen.expand(bezel * 0.5), rad + bezel * 0.5, bezel, rgba(12, 12, 14, 255));
        over.frame(outer, rad + bezel, 1.0, rgba(80, 80, 88, 255));
    }
    let mask_over = over.ops.len() - mask_start;
    Scene { under: under.ops, over: over.ops, video, uv, base, hits, screen, outer, bezel, chrome_under, mask_over }
}

// ---------------------------------------------------------------- the long-form page under the player

fn watch_page(b: &mut B, v: &View, player: Rect, app: Rect, k: f32, desktop: bool, hits: &mut Vec<(Rect, Hit)>) {
    let p = v.prof;
    let x0 = player.min.x + 14.0 * k;
    let w = player.width() - 28.0 * k;
    let mut y = player.max.y + 12.0 * k;
    let s = 16.0 * k;
    let title = if v.title.is_empty() { "Your video title goes here" } else { v.title };
    let lines = wrap(title, s, true, w);
    for l in lines.iter().take(2) {
        b.text(pos2(x0, y), s, Color32::WHITE, true, l, 0);
        y += s * 1.3;
    }
    b.text(pos2(x0, y + 2.0 * k), 12.0 * k, white(0.6), false, &format!("{} views  2 days ago  {}", p.views, p.tags), 0);
    y += 26.0 * k;
    b.circle(pos2(x0 + 18.0 * k, y + 18.0 * k), 18.0 * k, rgba(70, 70, 76, 255));
    b.avatar(pos2(x0 + 18.0 * k, y + 18.0 * k), 17.0 * k);
    b.text(pos2(x0 + 46.0 * k, y + 4.0 * k), 14.0 * k, Color32::WHITE, true, &p.display, 0);
    b.text(pos2(x0 + 46.0 * k, y + 22.0 * k), 11.5 * k, white(0.6), false, &format!("{} subscribers", p.followers), 0);
    let pill = Rect::from_min_size(pos2(x0 + w - 96.0 * k, y + 6.0 * k), vec2(96.0 * k, 30.0 * k));
    b.rect(pill, 15.0 * k, Color32::WHITE);
    b.text(pos2(pill.center().x, pill.min.y + 8.0 * k), 13.0 * k, Color32::BLACK, true, "Subscribe", 1);
    y += 50.0 * k;
    // like / share / save chips
    let mut cx = x0;
    for (g, label) in [("thumb", p.likes.as_str()), ("share", "Share"), ("save", "Save")] {
        let cw = (text_w(label, 12.5 * k, true) + 50.0 * k).max(70.0 * k);
        let r = Rect::from_min_size(pos2(cx, y), vec2(cw, 32.0 * k));
        b.rect(r, 16.0 * k, rgba(255, 255, 255, 28));
        glyph(b, g, pos2(r.min.x + 17.0 * k, r.center().y), 8.0 * k, Color32::WHITE);
        b.text(pos2(r.min.x + 32.0 * k, r.min.y + 8.0 * k), 12.5 * k, Color32::WHITE, true, label, 0);
        if g == "share" {
            hits.push((r, Hit::Open(Sheet::Share)));
        }
        cx += cw + 8.0 * k;
    }
    y += 46.0 * k;
    // description card
    let card = Rect::from_min_size(pos2(x0, y), vec2(w, 54.0 * k));
    if card.max.y < app.max.y {
        b.rect(card, 10.0 * k, rgba(255, 255, 255, 22));
        b.text(card.min + vec2(10.0, 8.0) * k, 12.0 * k, Color32::WHITE, true, &format!("{} views  2 days ago", p.views), 0);
        let (ls, _) = collapse(wrap(&p.caption_full(), 12.0 * k, false, w - 20.0 * k), 1, 12.0 * k, false, w - 20.0 * k, text_w("… more", 12.0 * k, false));
        b.text(card.min + vec2(10.0, 28.0) * k, 12.0 * k, white(0.75), false, ls.first().map_or("", |s| s), 0);
        b.text(card.max - vec2(52.0, 22.0) * k, 12.0 * k, white(0.6), false, "… more", 0);
        hits.push((card, Hit::Open(Sheet::Desc)));
    }
    y += 66.0 * k;
    let cc = Rect::from_min_size(pos2(x0, y), vec2(w, 60.0 * k));
    if cc.max.y < app.max.y + 30.0 * k {
        b.rect(cc, 10.0 * k, rgba(255, 255, 255, 22));
        b.text(cc.min + vec2(10.0, 8.0) * k, 13.0 * k, Color32::WHITE, true, &format!("Comments  {}", p.comments), 0);
        let c0 = p.comment_list.first().cloned().unwrap_or_default();
        b.circle(cc.min + vec2(22.0, 40.0) * k, 11.0 * k, name_color(&c0.name));
        b.text(cc.min + vec2(40.0, 33.0) * k, 12.0 * k, white(0.8), false, &ellipsize(&c0.text, 12.0 * k, false, w - 60.0 * k), 0);
        hits.push((cc, Hit::Open(Sheet::Comments)));
    }
    let _ = desktop;
}

// ---------------------------------------------------------------- sheets

fn close_x(b: &mut B, r: Rect, k: f32, hits: &mut Vec<(Rect, Hit)>) {
    let c = pos2(r.max.x - 26.0 * k, r.min.y + 28.0 * k);
    let d = 6.0 * k;
    b.line(c + vec2(-d, -d), c + vec2(d, d), 1.8 * k, Color32::WHITE);
    b.line(c + vec2(-d, d), c + vec2(d, -d), 1.8 * k, Color32::WHITE);
    hits.push((Rect::from_center_size(c, vec2(40.0, 40.0) * k), Hit::Close));
}

/// One sheet's panel and its contents inside `r`.
fn sheet_body(b: &mut B, v: &View, s: Sheet, r: Rect, k: f32, round: bool, hits: &mut Vec<(Rect, Hit)>) {
    let p = v.prof;
    let bg = rgba(33, 33, 36, 255);
    hits.push((r, Hit::Block));
    if round {
        b.rect_top(r, 12.0 * k, bg);
        b.rect(Rect::from_center_size(pos2(r.center().x, r.min.y + 8.0 * k), vec2(36.0, 4.0) * k), 2.0 * k, white(0.3));
        hits.push((Rect::from_center_size(pos2(r.center().x, r.min.y + 10.0 * k), vec2(120.0, 24.0) * k), Hit::Close));
    } else {
        b.rect(r, 12.0 * k, bg);
    }
    let (x0, w) = (r.min.x + 16.0 * k, r.width() - 32.0 * k);
    let title = match s {
        Sheet::Comments => format!("Comments  {}", p.comments),
        Sheet::Desc => "Description".to_string(),
        Sheet::Share => "Share".to_string(),
        Sheet::More => "More".to_string(),
        Sheet::Profile => net_name(&p.net).to_string(),
        Sheet::Sound => "Sound".to_string(),
    };
    b.text(pos2(x0, r.min.y + 20.0 * k), 16.0 * k, Color32::WHITE, true, &title, 0);
    close_x(b, r, k, hits);
    b.clip(Some(Rect::from_min_max(pos2(r.min.x, r.min.y + 52.0 * k), r.max)));
    let top = r.min.y + 58.0 * k;
    match s {
        Sheet::Comments => {
            let sz = 13.0 * k;
            let mut y = top;
            let bottom = r.max.y - 52.0 * k;
            for c in &p.comment_list {
                let lines = wrap(&c.text, sz, false, w - 100.0 * k);
                let rows = lines.len().min(3);
                if y > bottom {
                    break;
                }
                b.circle(pos2(x0 + 16.0 * k, y + 16.0 * k), 16.0 * k, name_color(&c.name));
                b.text(pos2(x0 + 16.0 * k, y + 9.0 * k), 13.0 * k, Color32::WHITE, true, &c.name.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default(), 1);
                b.text(pos2(x0 + 42.0 * k, y), 11.5 * k, white(0.65), true, &format!("@{}  {}", c.name, c.when), 0);
                for (i, l) in lines.iter().take(3).enumerate() {
                    b.text(pos2(x0 + 42.0 * k, y + 17.0 * k + i as f32 * sz * 1.3), sz, Color32::WHITE, false, l, 0);
                }
                let ry = y + 17.0 * k + rows as f32 * sz * 1.3 + 4.0 * k;
                b.text(pos2(x0 + 42.0 * k, ry), 11.5 * k, white(0.55), false, "Reply", 0);
                glyph(b, "like", pos2(r.max.x - 42.0 * k, y + 18.0 * k), 7.0 * k, white(0.7));
                b.text(pos2(r.max.x - 42.0 * k, y + 29.0 * k), 11.0 * k, white(0.6), false, &c.likes, 1);
                y = ry + 26.0 * k;
            }
            // the input row
            b.clip(Some(r));
            let ir = Rect::from_min_max(pos2(r.min.x, r.max.y - 52.0 * k), r.max);
            b.rect(ir, 0.0, bg);
            b.circle(pos2(x0 + 14.0 * k, ir.min.y + 24.0 * k), 14.0 * k, rgba(70, 70, 76, 255));
            b.avatar(pos2(x0 + 14.0 * k, ir.min.y + 24.0 * k), 13.0 * k);
            let pill = Rect::from_min_max(pos2(x0 + 36.0 * k, ir.min.y + 8.0 * k), pos2(r.max.x - 16.0 * k, ir.min.y + 40.0 * k));
            b.rect(pill, 16.0 * k, rgba(255, 255, 255, 24));
            b.text(pos2(pill.min.x + 14.0 * k, pill.min.y + 8.0 * k), 13.0 * k, white(0.55), false, "Add a comment…", 0);
        }
        Sheet::Desc => {
            let mut y = top;
            let sz = 16.0 * k;
            for l in wrap(&p.caption_full(), sz, true, w).iter().take(4) {
                b.text(pos2(x0, y), sz, Color32::WHITE, true, l, 0);
                y += sz * 1.3;
            }
            y += 10.0 * k;
            for (i, (val, lab)) in [(p.likes.as_str(), "Likes"), (p.views.as_str(), "Views"), ("Nov 13", "2026")].iter().enumerate() {
                let cx = x0 + w * (i as f32 + 0.5) / 3.0;
                b.text(pos2(cx, y), 16.0 * k, Color32::WHITE, true, val, 1);
                b.text(pos2(cx, y + 20.0 * k), 11.5 * k, white(0.55), false, lab, 1);
            }
            y += 52.0 * k;
            b.circle(pos2(x0 + 18.0 * k, y + 18.0 * k), 18.0 * k, rgba(70, 70, 76, 255));
            b.avatar(pos2(x0 + 18.0 * k, y + 18.0 * k), 17.0 * k);
            b.text(pos2(x0 + 46.0 * k, y + 3.0 * k), 14.0 * k, Color32::WHITE, true, &p.display, 0);
            if p.verified {
                b.circle(pos2(x0 + 52.0 * k + text_w(&p.display, 14.0 * k, true), y + 11.0 * k), 5.0 * k, white(0.7));
            }
            b.text(pos2(x0 + 46.0 * k, y + 21.0 * k), 11.5 * k, white(0.6), false, &format!("{} followers", p.followers), 0);
            y += 56.0 * k;
            b.text(pos2(x0, y), 13.0 * k, rgba(120, 170, 255, 255), false, &p.tags, 0);
            if !p.link.is_empty() {
                b.text(pos2(x0, y + 22.0 * k), 13.0 * k, rgba(120, 170, 255, 255), false, &ellipsize(&p.link, 13.0 * k, false, w), 0);
            }
        }
        Sheet::Share => {
            let labels = [("Copy link", "share"), ("Message", "comment"), ("Story", "plus"), ("Save", "save"), ("More", "more")];
            let step = w / 5.0;
            for (i, (l, g)) in labels.iter().enumerate() {
                let c = pos2(x0 + step * (i as f32 + 0.5), top + 30.0 * k);
                b.circle(c, 26.0 * k, name_color(l));
                glyph(b, g, c, 10.0 * k, Color32::WHITE);
                b.text(pos2(c.x, c.y + 32.0 * k), 11.5 * k, white(0.85), false, l, 1);
            }
            let lr = Rect::from_min_size(pos2(x0, top + 100.0 * k), vec2(w, 40.0 * k));
            b.rect(lr, 10.0 * k, rgba(255, 255, 255, 22));
            let url = if p.link.is_empty() { format!("{}/{}", host(&p.net), p.handle) } else { p.link.clone() };
            b.text(lr.min + vec2(12.0, 12.0) * k, 13.0 * k, white(0.75), false, &ellipsize(&url, 13.0 * k, false, w - 90.0 * k), 0);
            b.text(pos2(lr.max.x - 14.0 * k, lr.min.y + 12.0 * k), 13.0 * k, rgba(120, 170, 255, 255), true, "Copy", 2);
        }
        Sheet::More => {
            for (i, (l, g)) in [("Captions", "cc"), ("Quality", "gear"), ("Playback speed", "clock"), ("Not interested", "thumb_down"), ("Report", "badge")].iter().enumerate() {
                let y = top + i as f32 * 50.0 * k;
                glyph(b, g, pos2(x0 + 14.0 * k, y + 22.0 * k), 9.0 * k, Color32::WHITE);
                b.text(pos2(x0 + 44.0 * k, y + 14.0 * k), 14.0 * k, Color32::WHITE, false, l, 0);
            }
        }
        Sheet::Profile | Sheet::Sound => {
            let c = pos2(r.center().x, top + 34.0 * k);
            if s == Sheet::Profile {
                b.circle(c, 34.0 * k, rgba(70, 70, 76, 255));
                b.avatar(c, 33.0 * k);
                b.text(pos2(c.x, c.y + 42.0 * k), 17.0 * k, Color32::WHITE, true, &p.display, 1);
                b.text(pos2(c.x, c.y + 64.0 * k), 12.0 * k, white(0.6), false, &format!("{}  {} followers", p.handle_text(), p.followers), 1);
            } else {
                b.rect(Rect::from_center_size(c, vec2(64.0, 64.0) * k), 10.0 * k, rgba(255, 255, 255, 24));
                glyph(b, "note", c, 20.0 * k, Color32::WHITE);
                b.text(pos2(c.x, c.y + 42.0 * k), 15.0 * k, Color32::WHITE, true, &ellipsize(&p.sound, 15.0 * k, true, w), 1);
                b.text(pos2(c.x, c.y + 64.0 * k), 12.0 * k, white(0.6), false, &format!("{} videos", p.views), 1);
            }
            let pill = Rect::from_center_size(pos2(c.x, c.y + 98.0 * k), vec2(130.0, 34.0) * k);
            b.rect(pill, 17.0 * k, if s == Sheet::Profile { RED } else { Color32::WHITE });
            b.text(pos2(pill.center().x, pill.min.y + 9.0 * k), 13.0 * k, if s == Sheet::Profile { Color32::WHITE } else { Color32::BLACK }, true, if s == Sheet::Profile { "Follow" } else { "Use this sound" }, 1);
            let gy = pill.max.y + 14.0 * k;
            let tw = (w - 8.0 * k) / 3.0;
            for i in 0..3 {
                let tr = Rect::from_min_size(pos2(x0 + i as f32 * (tw + 4.0 * k), gy), vec2(tw, tw * 4.0 / 3.0));
                b.rect(tr, 4.0 * k, rgba(70 + i as u8 * 12, 70, 80, 255));
            }
        }
    }
    b.clip(None);
}
