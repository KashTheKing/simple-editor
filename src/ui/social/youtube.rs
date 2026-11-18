//! YouTube long form: the desktop watch page (top bar, player and its controls, the page under it) and the
//! phone app's watch page. Called by `device::build` for `pg == "watch"` platforms.

use super::device::*;
use super::device::Sheet;
use super::scene::*;
use super::*;
use eframe::egui::{Color32, Pos2, Rect};

/// YouTube's own greys: text, secondary text, the chip / pill fill, the page.
fn txt() -> Color32 {
    rgba(241, 241, 241, 255)
}
fn sub() -> Color32 {
    rgba(170, 170, 170, 255)
}
fn chip() -> Color32 {
    rgba(255, 255, 255, 26)
}
const PAGE: Color32 = Color32::from_rgb(15, 15, 15);
const YT_RED: Color32 = Color32::from_rgb(255, 0, 51);

/// The desktop watch page, laid out the way youtube.com lays it out in a browser this wide (CSS px = dp):
/// the sticky top bar; the player (default: in the left column next to the related videos; theatre: the full
/// width under the top bar; fullscreen: the whole screen, page gone) with its control bar and settings menu;
/// under it the title, the channel row with Subscribe, the like / dislike pill, Share and the more button, the
/// description and the comments. "Comments" scrolls the page to them, as on the real page (no sheet on desktop).
pub fn watch_desktop(pg: &Pg, under: &mut B, over: &mut B, hits: &mut Vec<(Rect, Hit)>) -> Placed {
    let Pg { v, dev, screen, outer, k, fade, vm, e, .. } = *pg;
    let u = k; // points per CSS px
    let pw = screen.width() / u;
    let ph = screen.height() / u;
    let tv = dev.k == "tv";
    // ---- where the player is in each view (CSS px of the page)
    let side = 402.0f32.min(pw * 0.3).max(300.0);
    let col_w = (pw - 24.0 - 24.0 - side - 24.0).clamp(320.0, 1280.0);
    let dflt = Rect::from_min_size(pos2(24.0, 56.0 + 24.0), vec2(col_w, col_w * 9.0 / 16.0));
    let th_h = (ph - 56.0 - 169.0).min(pw * 9.0 / 16.0).max(200.0);
    let theatre = Rect::from_min_size(pos2(0.0, 56.0), vec2(pw, th_h));
    let f = vm.clamp(0.0, 2.0);
    let t1 = smooth(f.min(1.0));
    let player_page = lerp_rect(dflt, theatre, t1);
    let x0 = 24.0;
    let below = player_page.max.y + 12.0;
    let side_x = 24.0 + col_w + 24.0;
    let side_y = lerp(56.0 + 24.0, below, t1);
    // the comments "sheet" scrolls the page instead (desktop has none): lay out once to find them
    let at0 = |x: f32, y: f32| pos2(screen.min.x + x * u, screen.min.y + y * u);
    let comments_top = page_under(&mut B::new(), v, at0, u, x0, below, col_w, &mut Vec::new(), false);
    let scroll = if v.open == Some(Sheet::Comments) && f < 1.5 { e * (comments_top - 56.0 - 12.0).max(0.0) } else { 0.0 };
    let at = |x: f32, y: f32| pos2(screen.min.x + x * u, screen.min.y + (y - scroll) * u);
    let mut page = B::new();
    let mut page_hits = Vec::new();
    page.a = fade;
    page.rect(Rect::from_min_max(at0(0.0, 56.0), screen.max), 0.0, PAGE);
    page.clip(Some(Rect::from_min_max(at0(0.0, 56.0), screen.max)));
    page_under(&mut page, v, at, u, x0, below, col_w, &mut page_hits, true);
    related(&mut page, at, u, side_x, side_y, side.min(pw - side_x - 24.0), screen.max.y + scroll * u);
    page.clip(None);
    masthead(&mut page, at0, u, pw);
    let modal = v.open.is_some_and(|s| s != Sheet::Comments) && e > 0.0;
    if fade > 0.5 && !modal {
        hits.extend(page_hits);
    }
    under.ops.extend(page.ops);
    // ---- the player: glides between the views; fullscreen fills the window (the whole TV)
    let pr = |r: Rect| Rect::from_min_max(at(r.min.x, r.min.y), at(r.max.x, r.max.y));
    let player = if f <= 1.0 {
        pr(player_page)
    } else {
        let full = if tv { screen } else { outer };
        lerp_rect(pr(theatre), full, smooth(f - 1.0))
    };
    let round = if f < 1.0 { 12.0 * u * (1.0 - t1) } else { 0.0 };
    let page_top = at0(0.0, 56.0).y;
    let (r0, uv0) = place(player, false, v.pa);
    // scrolled under the top bar: show only the part of the picture still below it
    let (r, uvr) = if f <= 1.0 && r0.min.y < page_top {
        let cut = ((page_top - r0.min.y) / r0.height().max(1.0)).clamp(0.0, 1.0);
        let uvr = Rect::from_min_max(pos2(uv0.min.x, uv0.min.y + uv0.height() * cut), uv0.max);
        (Rect::from_min_max(pos2(r0.min.x, page_top), pos2(r0.max.x, r0.max.y.max(page_top))), uvr)
    } else {
        (r0, uv0)
    };
    if player.max.y > page_top || f > 1.0 {
        under.clip(Some(Rect::from_min_max(pos2(screen.min.x, if f > 1.0 { outer.min.y } else { page_top }), outer.max)));
        under.rect(player, round, Color32::BLACK);
        under.clip(None);
    }
    if !modal {
        hits.push((player, Hit::Play));
    }
    let view = if f > 1.5 { ViewMode::Full } else if f > 0.5 { ViewMode::Theatre } else { ViewMode::Default };
    // the controls belong to the player: clipped to the page area while the page scrolls them under the top bar
    over.clip(Some(if f > 1.0 { outer } else { Rect::from_min_max(at0(0.0, 56.0), screen.max) }));
    let mut ctl = Vec::new();
    controls(over, &mut ctl, player, u, v, view);
    over.clip(None);
    if !modal {
        hits.extend(ctl);
    }
    if v.open.is_some_and(|s| s != Sheet::Comments) && e > 0.0 {
        share_dialog(over, v.open.unwrap_or(Sheet::Share), hits, screen, u, e);
    }
    Placed { base: r0, video: r, uv: uvr, sheet_top: player.max.y }
}

/// The sticky top bar: guide, logo, search box and the voice search, Create, notifications, the avatar.
fn masthead(b: &mut B, at: impl Fn(f32, f32) -> Pos2, u: f32, pw: f32) {
    b.rect(Rect::from_min_max(at(0.0, 0.0), at(pw, 56.0)), 0.0, PAGE);
    let ink = txt();
    b.icon_at("yt/guide", at(36.0, 28.0), 24.0 * u, ink);
    b.icon_at("yt/logo", at(72.0 + 45.0, 28.0), 20.0 * u, Color32::WHITE);
    // search: input + button, then the microphone; centred, at most 640 wide
    let sw = (pw - 2.0 * 230.0).clamp(220.0, 640.0);
    let x0 = (pw - sw - 52.0) / 2.0;
    let input = Rect::from_min_max(at(x0, 8.0), at(x0 + sw - 40.0, 48.0));
    b.rect(input, 20.0 * u, rgba(18, 18, 18, 255));
    b.frame(input, 20.0 * u, 1.0 * u, rgba(48, 48, 48, 255));
    b.text(at(x0 + 16.0, 19.0), 16.0 * u, rgba(136, 136, 136, 255), false, "Search", 0);
    let btn = Rect::from_min_max(at(x0 + sw - 64.0, 8.0), at(x0 + sw, 48.0));
    b.rect(btn, 20.0 * u, rgba(34, 34, 34, 255));
    b.rect(Rect::from_min_max(btn.min, pos2(btn.center().x, btn.max.y)), 0.0, rgba(34, 34, 34, 255));
    b.frame(btn, 20.0 * u, 1.0 * u, rgba(48, 48, 48, 255));
    b.icon_at("yt/search", btn.center() + vec2(-2.0, 0.0) * u, 24.0 * u, ink);
    b.circle(at(x0 + sw + 32.0, 28.0), 20.0 * u, rgba(34, 34, 34, 255));
    b.icon_at("yt/mic", at(x0 + sw + 32.0, 28.0), 24.0 * u, ink);
    // right: Create, notifications, avatar
    let cr = Rect::from_min_max(at(pw - 206.0, 10.0), at(pw - 104.0, 46.0));
    b.rect(cr, 18.0 * u, rgba(39, 39, 39, 255));
    b.icon_at("yt/create", at(pw - 206.0 + 22.0, 28.0), 24.0 * u, ink);
    b.text(at(pw - 206.0 + 40.0, 19.5), 14.0 * u, ink, true, "Create", 0);
    b.icon_at("yt/bell", at(pw - 76.0, 28.0), 24.0 * u, ink);
    let badge = Rect::from_min_size(at(pw - 72.0, 10.0), vec2(22.0, 15.0) * u);
    b.rect(badge, 7.5 * u, rgba(204, 0, 0, 255));
    b.text(pos2(badge.center().x, badge.min.y + 1.5 * u), 10.5 * u, Color32::WHITE, true, "9+", 1);
    b.circle(at(pw - 32.0, 28.0), 16.0 * u, rgba(70, 70, 76, 255));
    b.avatar(at(pw - 32.0, 28.0), 16.0 * u);
}

/// Everything under the player in the left column: title, the channel row, the actions, the description and
/// the comments. Returns where the comments start (page px, unscrolled). `draw` false only measures.
#[allow(clippy::too_many_arguments)]
fn page_under(b: &mut B, v: &View, at: impl Fn(f32, f32) -> Pos2, u: f32, x0: f32, y0: f32, w: f32, hits: &mut Vec<(Rect, Hit)>, draw: bool) -> f32 {
    let p = v.prof;
    let mut y = y0;
    let title = if v.title.is_empty() { "Your video title goes here" } else { v.title };
    for l in wrap(title, 20.0, true, w).iter().take(2) {
        if draw {
            b.text(at(x0, y), 20.0 * u, txt(), true, l, 0);
        }
        y += 28.0;
    }
    y += 8.0;
    // channel row: avatar, name + tick, subscribers, Subscribe; the actions at the right
    let cy = y + 20.0;
    if draw {
        b.circle(at(x0 + 20.0, cy), 20.0 * u, rgba(70, 70, 76, 255));
        b.avatar(at(x0 + 20.0, cy), 20.0 * u);
        let name = ellipsize(&p.display, 16.0, true, 180.0);
        b.text(at(x0 + 52.0, cy - 19.0), 16.0 * u, txt(), true, &name, 0);
        let nw = text_w(&name, 16.0, true);
        if p.verified {
            verified(b, at(x0 + 52.0 + nw + 10.0, cy - 10.0), 6.0 * u);
        }
        let subs = format!("{} subscribers", p.followers);
        b.text(at(x0 + 52.0, cy + 2.0), 12.0 * u, sub(), false, &subs, 0);
        let sx = x0 + 52.0 + nw.max(text_w(&subs, 12.0, false)) + 30.0;
        let pill = Rect::from_min_max(at(sx, cy - 18.0), at(sx + 96.0, cy + 18.0));
        b.rect(pill, 18.0 * u, rgba(241, 241, 241, 255));
        b.text(pos2(pill.center().x, pill.min.y + 9.5 * u), 14.0 * u, PAGE, true, "Subscribe", 1);
        hits.push((pill, Hit::Block));
        // actions, right-aligned: [like | dislike] [Share] [more]
        let mut rx = x0 + w;
        let more = Rect::from_min_max(at(rx - 36.0, cy - 18.0), at(rx, cy + 18.0));
        b.rect(more, 18.0 * u, chip());
        b.icon_at("yt/more_h", more.center(), 24.0 * u, txt());
        hits.push((more, Hit::Block));
        rx -= 36.0 + 8.0;
        let share = Rect::from_min_max(at(rx - 92.0, cy - 18.0), at(rx, cy + 18.0));
        b.rect(share, 18.0 * u, chip());
        b.icon_at("yt/share", pos2(share.min.x + 26.0 * u, share.center().y), 24.0 * u, txt());
        b.text(pos2(share.min.x + 46.0 * u, share.min.y + 9.5 * u), 14.0 * u, txt(), true, "Share", 0);
        hits.push((share, Hit::Open(Sheet::Share)));
        rx -= 92.0 + 8.0;
        let lw = text_w(&p.likes, 14.0, true);
        let lr = Rect::from_min_max(at(rx - (lw + 108.0), cy - 18.0), at(rx, cy + 18.0));
        b.rect(lr, 18.0 * u, chip());
        b.icon_at("yt/like", pos2(lr.min.x + 24.0 * u, lr.center().y), 24.0 * u, txt());
        b.text(pos2(lr.min.x + 44.0 * u, lr.min.y + 9.5 * u), 14.0 * u, txt(), true, &p.likes, 0);
        let dx = lr.min.x + (lw + 58.0) * u;
        b.rect(Rect::from_min_max(pos2(dx, lr.min.y + 6.0 * u), pos2(dx + 1.0 * u, lr.max.y - 6.0 * u)), 0.0, rgba(255, 255, 255, 51));
        b.icon_at("yt/dislike", pos2(lr.max.x - 26.0 * u, lr.center().y), 24.0 * u, txt());
        hits.push((lr, Hit::Block));
    }
    y += 40.0 + 12.0;
    // description box
    let lines = wrap(&p.caption_full(), 14.0, false, w - 24.0);
    let (shown, cut) = collapse(lines, 2, 14.0, false, w - 24.0, text_w("...more", 14.0, true));
    let dh = 12.0 + 20.0 + shown.len() as f32 * 20.0 + 12.0;
    if draw {
        let card = Rect::from_min_max(at(x0, y), at(x0 + w, y + dh));
        b.rect(card, 12.0 * u, chip());
        let head = format!("{} views  2 weeks ago  ", p.views);
        b.text(at(x0 + 12.0, y + 12.0), 14.0 * u, txt(), true, &head, 0);
        b.text(at(x0 + 12.0 + text_w(&head, 14.0, true), y + 12.0), 14.0 * u, sub(), true, &p.tags, 0);
        for (i, l) in shown.iter().enumerate() {
            b.text(at(x0 + 12.0, y + 32.0 + i as f32 * 20.0), 14.0 * u, txt(), false, l, 0);
            if cut && i + 1 == shown.len() {
                b.text(at(x0 + 12.0 + text_w(l, 14.0, false) + 4.0, y + 32.0 + i as f32 * 20.0), 14.0 * u, txt(), true, "...more", 0);
            }
        }
        hits.push((card, Hit::Block));
    }
    y += dh + 24.0;
    if draw {
        comments(b, v, &at, u, x0, y, w, hits);
    }
    y
}

/// The desktop comments: "N Comments", Sort by, the add-a-comment row, then the comment threads.
#[allow(clippy::too_many_arguments)]
fn comments(b: &mut B, v: &View, at: &impl Fn(f32, f32) -> Pos2, u: f32, x0: f32, mut y: f32, w: f32, hits: &mut Vec<(Rect, Hit)>) {
    let p = v.prof;
    let head = format!("{} Comments", p.comments);
    b.text(at(x0, y), 20.0 * u, txt(), true, &head, 0);
    let sx = x0 + text_w(&head, 20.0, true) + 32.0;
    b.icon_at("yt/sort", at(sx + 12.0, y + 13.0), 24.0 * u, txt());
    b.text(at(sx + 32.0, y + 5.0), 14.0 * u, txt(), true, "Sort by", 0);
    y += 52.0;
    b.circle(at(x0 + 20.0, y + 20.0), 20.0 * u, rgba(70, 70, 76, 255));
    b.avatar(at(x0 + 20.0, y + 20.0), 20.0 * u);
    b.text(at(x0 + 56.0, y + 6.0), 14.0 * u, sub(), false, "Add a comment...", 0);
    b.rect(Rect::from_min_max(at(x0 + 56.0, y + 28.0), at(x0 + w, y + 29.0)), 0.0, rgba(113, 113, 113, 255));
    hits.push((Rect::from_min_max(at(x0, y), at(x0 + w, y + 40.0)), Hit::Block));
    y += 72.0;
    for c in &p.comment_list {
        let ax = x0 + 20.0;
        b.circle(at(ax, y + 20.0), 20.0 * u, super::device::name_color(&c.name));
        let initial = c.name.chars().next().map(|ch| ch.to_ascii_uppercase().to_string()).unwrap_or_default();
        b.text(at(ax, y + 9.0), 18.0 * u, Color32::WHITE, false, &initial, 1);
        let handle = format!("@{}", c.name);
        b.text(at(x0 + 56.0, y), 13.0 * u, txt(), true, &handle, 0);
        b.text(at(x0 + 56.0 + text_w(&handle, 13.0, true) + 6.0, y + 0.5), 12.0 * u, sub(), false, &format!("{} ago", expand_when(&c.when)), 0);
        let lines = wrap(&c.text, 14.0, false, w - 100.0);
        for (i, l) in lines.iter().take(3).enumerate() {
            b.text(at(x0 + 56.0, y + 20.0 + i as f32 * 20.0), 14.0 * u, txt(), false, l, 0);
        }
        let ty = y + 20.0 + lines.len().min(3) as f32 * 20.0 + 6.0;
        b.icon_at("yt/like", at(x0 + 64.0, ty + 12.0), 20.0 * u, txt());
        b.text(at(x0 + 80.0, ty + 5.0), 12.0 * u, sub(), false, &c.likes, 0);
        let dx = x0 + 80.0 + text_w(&c.likes, 12.0, false) + 22.0;
        b.icon_at("yt/dislike", at(dx, ty + 12.0), 20.0 * u, txt());
        b.text(at(dx + 26.0, ty + 5.0), 12.0 * u, txt(), true, "Reply", 0);
        b.icon_at("yt/more_v", at(x0 + w - 12.0, y + 10.0), 24.0 * u, txt());
        y = ty + 24.0 + 24.0;
    }
}

/// "2h" -> "2 hours", "3d" -> "3 days" (the desktop page spells it out).
pub(super) fn expand_when(w: &str) -> String {
    let n: String = w.chars().take_while(|c| c.is_ascii_digit()).collect();
    let unit = w[n.len()..].trim();
    let num: u32 = n.parse().unwrap_or(1);
    let name = match unit {
        "s" => "second",
        "m" | "min" => "minute",
        "h" => "hour",
        "d" => "day",
        "w" => "week",
        "mo" => "month",
        "y" => "year",
        _ => return w.to_string(),
    };
    format!("{num} {name}{}", if num == 1 { "" } else { "s" })
}

/// The grey verified tick after a channel name.
fn verified(b: &mut B, c: Pos2, r: f32) {
    b.circle(c, r, sub());
    b.line(c + vec2(-0.45, 0.0) * r, c + vec2(-0.1, 0.35) * r, 0.28 * r, PAGE);
    b.line(c + vec2(-0.1, 0.35) * r, c + vec2(0.5, -0.35) * r, 0.28 * r, PAGE);
}

/// The related videos: chips, then thumbnails (with a duration) and their text, as far down as the screen goes.
#[allow(clippy::too_many_arguments)]
fn related(b: &mut B, at: impl Fn(f32, f32) -> Pos2, u: f32, x0: f32, y0: f32, w: f32, bottom: f32) {
    if w < 200.0 {
        return;
    }
    let mut cx = x0;
    for (i, c) in ["All", "Related", "For you", "Recently uploaded"].iter().enumerate() {
        let cw = text_w(c, 14.0, true) + 24.0;
        if cx + cw > x0 + w {
            break;
        }
        let r = Rect::from_min_max(at(cx, y0), at(cx + cw, y0 + 32.0));
        b.rect(r, 8.0 * u, if i == 0 { rgba(241, 241, 241, 255) } else { chip() });
        b.text(pos2(r.center().x, r.min.y + 8.0 * u), 14.0 * u, if i == 0 { PAGE } else { txt() }, true, c, 1);
        cx += cw + 12.0;
    }
    let mut y = y0 + 32.0 + 12.0;
    let tw = 168.0;
    for n in 0..14 {
        let th = Rect::from_min_max(at(x0, y), at(x0 + tw, y + 94.0));
        if th.max.y > bottom {
            break;
        }
        let shade = 40 + (n * 13 % 30) as u8;
        b.rect(th, 8.0 * u, rgba(shade, shade + 4, shade + 10, 255));
        let d = ["12:04", "8:47", "21:30", "3:59", "15:12", "6:21"][n % 6];
        let dw = text_w(d, 12.0, true) + 8.0;
        let badge = Rect::from_min_max(at(x0 + tw - 4.0 - dw, y + 94.0 - 22.0), at(x0 + tw - 4.0, y + 94.0 - 4.0));
        b.rect(badge, 4.0 * u, Color32::from_black_alpha(200));
        b.text(pos2(badge.center().x, badge.min.y + 2.0 * u), 12.0 * u, Color32::WHITE, true, d, 1);
        let tx = x0 + tw + 8.0;
        b.rect(Rect::from_min_max(at(tx, y + 2.0), at(x0 + w - 16.0, y + 14.0)), 3.0 * u, rgba(64, 64, 64, 255));
        b.rect(Rect::from_min_max(at(tx, y + 22.0), at(tx + (w - tw - 8.0) * 0.6, y + 34.0)), 3.0 * u, rgba(64, 64, 64, 255));
        b.rect(Rect::from_min_max(at(tx, y + 46.0), at(tx + (w - tw - 8.0) * 0.45, y + 56.0)), 3.0 * u, rgba(44, 44, 44, 255));
        y += 94.0 + 8.0;
    }
}

// ---------------------------------------------------------------- the player's own UI

/// A translucent pill behind a control group (it shows over a bright picture).
fn pill(b: &mut B, r: Rect) {
    b.rect(r, r.height() * 0.5, Color32::from_black_alpha(96));
}

/// The control bar: the progress bar (red, scrubber), then play / next / the speaker and its slider / the time
/// on the left, captions / settings / miniplayer / theatre / fullscreen on the right; the settings menu when open;
/// the spinner while buffering; the title over the top in fullscreen. Fixed CSS-px sizes whatever the player's size.
fn controls(b: &mut B, hits: &mut Vec<(Rect, Hit)>, player: Rect, u: f32, v: &View, view: ViewMode) {
    let pl = v.pl;
    let (pmin, pmax) = (player.min, player.max);
    let w = player.width() / u;
    b.grad(Rect::from_min_max(pos2(pmin.x, (pmax.y - 150.0 * u).max(pmin.y)), pmax), true, 0.75);
    if view == ViewMode::Full {
        b.grad(Rect::from_min_max(pmin, pos2(pmax.x, pmin.y + 110.0 * u)), false, 0.6);
        let title = if v.title.is_empty() { "Your video title goes here" } else { v.title };
        b.circle(pos2(pmin.x + 40.0 * u, pmin.y + 36.0 * u), 20.0 * u, rgba(70, 70, 76, 255));
        b.avatar(pos2(pmin.x + 40.0 * u, pmin.y + 36.0 * u), 20.0 * u);
        b.text(pos2(pmin.x + 72.0 * u, pmin.y + 24.0 * u), 20.0 * u, Color32::WHITE, false, &ellipsize(title, 20.0 * u, false, (w - 120.0) * u), 0);
    }
    let at = |x: f32, y: f32| pos2(pmin.x + x * u, pmax.y - y * u);
    // progress
    let pad = 12.0;
    let track = Rect::from_min_max(at(pad, 60.5), at(w - pad, 57.5));
    b.rect(track, 0.0, rgba(255, 255, 255, 51));
    let fx = |f: f32| track.min.x + track.width() * f.clamp(0.0, 1.0);
    b.rect(Rect::from_min_max(track.min, pos2(fx(v.progress + 0.12), track.max.y)), 0.0, rgba(255, 255, 255, 102));
    b.rect(Rect::from_min_max(track.min, pos2(fx(v.progress), track.max.y)), 0.0, YT_RED);
    b.circle(pos2(fx(v.progress), track.center().y), 6.5 * u, YT_RED);
    hits.push((track.expand2(vec2(0.0, 8.0 * u)), Hit::Seek(track)));
    // left group
    let cy = 29.0;
    let play = Rect::from_center_size(at(pad + 24.0, cy), vec2(48.0, 48.0) * u);
    pill(b, play);
    b.icon_at(if v.playing { "yt/pause" } else { "yt/play" }, play.center(), 36.0 * u, Color32::WHITE);
    hits.push((play, Hit::Play));
    let next = Rect::from_center_size(at(pad + 48.0 + 8.0 + 20.0, cy), vec2(40.0, 40.0) * u);
    pill(b, next);
    b.icon_at("yt/next", next.center(), 24.0 * u, Color32::WHITE);
    hits.push((next, Hit::Block));
    // the speaker; the slider slides out (shown here: it is the thing to check)
    let vx = (next.max.x - pmin.x) / u + 8.0;
    let vol = Rect::from_min_max(at(vx, cy + 20.0), at(vx + 40.0 + 72.0, cy - 20.0));
    pill(b, vol);
    let vname = if pl.muted || pl.volume <= 0.001 { "yt/volume_mute" } else if pl.volume < 0.5 { "yt/volume_low" } else { "yt/volume_high" };
    let spk = Rect::from_center_size(at(vx + 20.0, cy), vec2(40.0, 40.0) * u);
    b.icon_at(vname, spk.center(), 24.0 * u, Color32::WHITE);
    hits.push((spk, Hit::Mute));
    let sl = Rect::from_min_max(at(vx + 44.0, cy + 1.5), at(vx + 104.0, cy - 1.5));
    let level = if pl.muted { 0.0 } else { pl.volume.clamp(0.0, 1.0) };
    b.rect(sl, 1.5 * u, rgba(255, 255, 255, 76));
    b.rect(Rect::from_min_max(sl.min, pos2(sl.min.x + sl.width() * level, sl.max.y)), 1.5 * u, Color32::WHITE);
    b.circle(pos2(sl.min.x + sl.width() * level, sl.center().y), 6.0 * u, Color32::WHITE);
    hits.push((sl.expand2(vec2(4.0 * u, 10.0 * u)), Hit::Volume(sl)));
    // the time
    let clock = v.clock;
    let tw = text_w(clock, 14.0, true) + 28.0;
    let tx = (vol.max.x - pmin.x) / u + 8.0;
    let time = Rect::from_min_max(at(tx, cy + 20.0), at(tx + tw, cy - 20.0));
    if time.max.x < pmax.x - 260.0 * u {
        pill(b, time);
        b.text(pos2(time.center().x, time.min.y + 11.0 * u), 14.0 * u, Color32::WHITE, true, clock, 1);
    }
    // right group: captions, settings, miniplayer, theatre / default view, fullscreen
    let names = [
        if pl.cc { "yt/cc_on" } else { "yt/cc_off" },
        "yt/settings",
        "yt/miniplayer",
        if view == ViewMode::Theatre { "yt/default_view" } else { "yt/theater" },
        "yt/fullscreen",
    ];
    let hitv = [Hit::Captions, Hit::Menu, Hit::Block, Hit::View(ViewMode::Theatre), Hit::View(ViewMode::Full)];
    let n = if view == ViewMode::Full { 4.0 } else { 5.0 };
    let right = Rect::from_min_max(at(w - pad - 48.0 * n, cy + 20.0), at(w - pad, cy - 20.0));
    pill(b, right);
    let mut slot = 0.0;
    for (i, (name, h)) in names.iter().zip(hitv).enumerate() {
        if view == ViewMode::Full && i == 3 {
            continue; // fullscreen has no theatre button
        }
        let c = at(w - pad - 48.0 * (n - slot) + 24.0, cy);
        slot += 1.0;
        b.icon_at(name, c, 24.0 * u, Color32::WHITE);
        hits.push((Rect::from_center_size(c, vec2(48.0, 40.0) * u), h));
    }
    if pl.menu {
        settings_menu(b, hits, at(w - pad, 60.5 + 12.0), u, pl.cc);
    }
    if pl.buffering {
        spinner(b, player.center(), 32.0 * u, 4.0 * u, pl.t, Color32::WHITE);
    }
}

/// The settings menu, its bottom-right corner at `br`: the toggles and the rows with their values. The
/// Subtitles/CC row switches the project's subtitles.
fn settings_menu(b: &mut B, hits: &mut Vec<(Rect, Hit)>, br: Pos2, u: f32, cc: bool) {
    let rows: [(&str, &str, &str); 7] = [
        ("yt/set_ambient", "Ambient mode", "toggle:on"),
        ("yt/set_annotations", "Annotations", "toggle:off"),
        ("yt/set_audio", "Audio track", "English (US) original"),
        ("yt/set_cc", "Subtitles/CC", if cc { "English" } else { "Off" }),
        ("yt/set_sleep", "Sleep timer", "Off"),
        ("yt/set_speed", "Playback speed", "Normal"),
        ("yt/set_quality", "Quality", "Auto (1080p)"),
    ];
    let (w, rh) = (372.0, 48.0);
    let h = rows.len() as f32 * rh + 16.0;
    let panel = Rect::from_min_max(pos2(br.x - w * u, br.y - h * u), br);
    b.rect(panel, 12.0 * u, rgba(28, 28, 28, 230));
    hits.push((panel, Hit::Block));
    for (i, (icon, label, val)) in rows.iter().enumerate() {
        let y = panel.min.y + (8.0 + i as f32 * rh) * u;
        let row = Rect::from_min_max(pos2(panel.min.x, y), pos2(panel.max.x, y + rh * u));
        let cy = row.center().y;
        b.icon_at(icon, pos2(row.min.x + 28.0 * u, cy), 24.0 * u, Color32::WHITE);
        b.text(pos2(row.min.x + 64.0 * u, cy - 8.0 * u), 15.0 * u, rgba(238, 238, 238, 255), false, label, 0);
        if let Some(on) = val.strip_prefix("toggle:") {
            let on = on == "on";
            let tr = Rect::from_center_size(pos2(row.max.x - 40.0 * u, cy), vec2(36.0, 14.0) * u);
            b.rect(tr, 7.0 * u, if on { rgba(255, 255, 255, 110) } else { rgba(255, 255, 255, 40) });
            b.circle(pos2(if on { tr.max.x - 4.0 * u } else { tr.min.x + 4.0 * u }, cy), 10.0 * u, if on { Color32::WHITE } else { rgba(190, 190, 190, 255) });
        } else {
            b.icon_at("yt/chevron_right", pos2(row.max.x - 22.0 * u, cy), 24.0 * u, Color32::WHITE);
            b.text(pos2(row.max.x - 38.0 * u, cy - 7.5 * u), 14.0 * u, rgba(238, 238, 238, 255), false, val, 2);
        }
        hits.push((row, if *label == "Subtitles/CC" { Hit::Captions } else { Hit::Block }));
    }
}

/// YouTube's buffering spinner: a white arc that grows, shrinks and turns.
pub(super) fn spinner(b: &mut B, c: Pos2, r: f32, w: f32, t: f32, col: Color32) {
    use std::f32::consts::TAU;
    let len = (0.12 + 0.6 * (0.5 + 0.5 * (t * 3.2).sin())) * TAU;
    let a0 = t * 5.0;
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + len * i as f32 / n as f32;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    for s in pts.windows(2) {
        b.line(s[0], s[1], w, col);
    }
}

/// The Share dialog (and the description / more dialogs, which desktop shows the same way) over the dimmed page.
fn share_dialog(b: &mut B, which: Sheet, hits: &mut Vec<(Rect, Hit)>, screen: Rect, u: f32, e: f32) {
    b.rect(screen, 0.0, Color32::from_black_alpha((150.0 * e) as u8));
    hits.push((screen, Hit::Close));
    let d = Rect::from_center_size(screen.center(), vec2(520.0, 250.0) * u);
    b.rect(d, 12.0 * u, rgba(40, 40, 40, 255));
    hits.push((d, Hit::Block));
    let title = match which {
        Sheet::Share => "Share",
        Sheet::Desc => "Description",
        _ => "More",
    };
    b.text(d.min + vec2(24.0, 22.0) * u, 16.0 * u, txt(), false, title, 0);
    let close = d.right_top() + vec2(-28.0, 30.0) * u;
    b.line(close + vec2(-7.0, -7.0) * u, close + vec2(7.0, 7.0) * u, 1.6 * u, txt());
    b.line(close + vec2(-7.0, 7.0) * u, close + vec2(7.0, -7.0) * u, 1.6 * u, txt());
    hits.push((Rect::from_center_size(close, vec2(36.0, 36.0) * u), Hit::Close));
    for (i, (name, col)) in [("Embed", rgba(60, 60, 60, 255)), ("WhatsApp", rgba(37, 211, 102, 255)), ("Facebook", rgba(24, 119, 242, 255)), ("X", rgba(0, 0, 0, 255)), ("Email", rgba(136, 136, 136, 255)), ("Reddit", rgba(255, 69, 0, 255))].iter().enumerate() {
        let c = d.min + vec2(24.0 + 30.0 + i as f32 * 78.0, 100.0) * u;
        b.circle(c, 30.0 * u, *col);
        b.text(pos2(c.x, c.y + 40.0 * u), 12.0 * u, txt(), false, name, 1);
    }
    let link = Rect::from_min_max(d.min + vec2(24.0, 180.0) * u, pos2(d.max.x - 24.0 * u, d.min.y + 228.0 * u));
    b.rect(link, 12.0 * u, rgba(15, 15, 15, 255));
    b.frame(link, 12.0 * u, 1.0 * u, rgba(255, 255, 255, 26));
    b.text(link.min + vec2(16.0, 15.0) * u, 14.0 * u, txt(), false, "https://youtu.be/dQw4w9WgXcQ", 0);
    let copy = Rect::from_min_max(pos2(link.max.x - 78.0 * u, link.min.y + 6.0 * u), pos2(link.max.x - 8.0 * u, link.max.y - 6.0 * u));
    b.rect(copy, 18.0 * u, rgba(62, 166, 255, 255));
    b.text(pos2(copy.center().x, copy.min.y + 10.0 * u), 14.0 * u, PAGE, true, "Copy", 1);
}

/// The YouTube app's watch page on a phone or tablet. Portrait: the player under the status bar (its controls
/// shown while paused: previous / play / next, captions and settings at the top, the time and fullscreen at the
/// bottom, the red bar along its bottom edge), then the title, the channel row with Subscribe, the action chips,
/// the comments teaser and the next video. Landscape: the app's fullscreen player.
pub fn watch_phone(pg: &Pg, under: &mut B, over: &mut B, hits: &mut Vec<(Rect, Hit)>) -> Placed {
    let Pg { v, app, k, st_h, e, .. } = *pg;
    let land = app.width() > app.height();
    if land {
        under.rect(app, 0.0, Color32::BLACK);
        let player = app;
        let (r, uvr) = place(player, false, v.pa);
        if e <= 0.0 {
            hits.push((player, Hit::Play));
        }
        phone_controls(over, hits, player, app, k, v, true);
        return Placed { base: r, video: r, uv: uvr, sheet_top: app.min.y };
    }
    under.rect(app, 0.0, PAGE);
    let player = Rect::from_min_size(pos2(app.min.x, app.min.y + st_h), vec2(app.width(), app.width() * 9.0 / 16.0));
    under.rect(Rect::from_min_max(app.min, pos2(app.max.x, player.min.y)), 0.0, Color32::BLACK);
    under.rect(player, 0.0, Color32::BLACK);
    let (r, uvr) = place(player, false, v.pa);
    if e <= 0.0 {
        hits.push((player, Hit::Play));
    }
    let mut page = B::new();
    phone_page(&mut page, v, player, app, k, if e <= 0.0 { Some(&mut *hits) } else { None });
    under.ops.extend(page.ops);
    phone_controls(over, hits, player, app, k, v, false);
    Placed { base: r, video: r, uv: uvr, sheet_top: player.max.y }
}

/// The player's own controls in the app; `app` is the screen above the system bars (the settings sheet rises from
/// its bottom).
fn phone_controls(b: &mut B, hits: &mut Vec<(Rect, Hit)>, player: Rect, app: Rect, k: f32, v: &View, land: bool) {
    let pl = v.pl;
    let shown = !v.playing || pl.menu || pl.buffering;
    let w = Color32::WHITE;
    // the bar: along the bottom edge of the player (portrait), or above the bottom of the screen (landscape)
    let (x0, x1, by) = if land { (player.min.x + 24.0 * k, player.max.x - 24.0 * k, player.max.y - 44.0 * k) } else { (player.min.x, player.max.x, player.max.y - 1.5 * k) };
    let track = Rect::from_min_max(pos2(x0, by - 1.5 * k), pos2(x1, by + 1.5 * k));
    let fx = track.min.x + track.width() * v.progress.clamp(0.0, 1.0);
    if shown {
        b.rect(player, 0.0, Color32::from_black_alpha(90));
        b.rect(track, 0.0, rgba(255, 255, 255, 76));
        b.rect(Rect::from_min_max(track.min, pos2(fx, track.max.y)), 0.0, YT_RED);
        b.circle(pos2(fx, track.center().y), 6.0 * k, YT_RED);
        hits.push((track.expand2(vec2(0.0, 12.0 * k)), Hit::Seek(track)));
        // previous / play / next in the middle
        let c = player.center();
        for (dx, name, r) in [(-96.0, "yt/prev", 24.0), (0.0, if v.playing { "yt/pause" } else { "yt/play" }, 32.0), (96.0, "yt/next", 24.0)] {
            let cc = c + vec2(dx * k, 0.0);
            b.circle(cc, r * k, Color32::from_black_alpha(80));
            b.icon_at(name, cc, if dx == 0.0 { 44.0 } else { 26.0 } * k, w);
            hits.push((Rect::from_center_size(cc, vec2(2.0 * r, 2.0 * r) * k), if dx == 0.0 { Hit::Play } else { Hit::Block }));
        }
        // top right: captions, settings (and, in landscape, the title at the top left)
        let ty = player.min.y + if land { 28.0 } else { 22.0 } * k;
        let cc_at = pos2(player.max.x - 64.0 * k, ty);
        b.icon_at(if pl.cc { "yt/cc_on" } else { "yt/cc_off" }, cc_at, 24.0 * k, w);
        hits.push((Rect::from_center_size(cc_at, vec2(44.0, 40.0) * k), Hit::Captions));
        let gear = pos2(player.max.x - 24.0 * k, ty);
        b.icon_at("yt/settings", gear, 24.0 * k, w);
        hits.push((Rect::from_center_size(gear, vec2(40.0, 40.0) * k), Hit::Menu));
        b.icon_at("yt/cast", pos2(player.max.x - 104.0 * k, ty), 24.0 * k, w);
        if land {
            b.icon_at("yt/chevron_down", pos2(player.min.x + 28.0 * k, ty), 24.0 * k, w);
            let title = if v.title.is_empty() { "Your video title goes here" } else { v.title };
            b.text(pos2(player.min.x + 52.0 * k, ty - 9.0 * k), 16.0 * k, w, true, &ellipsize(title, 16.0 * k, true, player.width() * 0.6), 0);
        }
        // bottom: the time at the left, fullscreen at the right
        let ly = if land { by - 22.0 * k } else { player.max.y - 22.0 * k };
        let tx = if land { x0 } else { player.min.x + 12.0 * k };
        let (now, total) = v.clock.split_once(" / ").unwrap_or((v.clock, ""));
        b.text(pos2(tx, ly - 7.0 * k), 12.0 * k, w, true, now, 0);
        b.text(pos2(tx + text_w(now, 12.0 * k, true), ly - 7.0 * k), 12.0 * k, rgba(255, 255, 255, 178), true, &format!(" / {total}"), 0);
        b.icon_at(if land { "yt/default_view" } else { "yt/fullscreen" }, pos2(player.max.x - 24.0 * k, ly), 24.0 * k, w);
        if land {
            // the action row under the bar
            let ay = player.max.y - 18.0 * k;
            let mut ax = x0;
            for (name, label) in [("yt/like", v.prof.likes.as_str()), ("yt/dislike", ""), ("yt/share", "Share"), ("yt/save", "Save"), ("yt/more_h", "")] {
                let lw = if label.is_empty() { 0.0 } else { text_w(label, 12.0 * k, true) + 6.0 * k };
                b.icon_at(name, pos2(ax + 10.0 * k, ay), 20.0 * k, w);
                b.text(pos2(ax + 24.0 * k, ay - 7.0 * k), 12.0 * k, w, true, label, 0);
                ax += 24.0 * k + lw + 22.0 * k;
            }
        }
    } else {
        // playing: only the thin red line
        b.rect(Rect::from_min_max(pos2(track.min.x, track.max.y - 2.0 * k), pos2(fx, track.max.y)), 0.0, YT_RED);
    }
    if pl.buffering {
        spinner(b, player.center(), 22.0 * k, 3.0 * k, pl.t, w);
    }
    if pl.menu {
        // the app's settings: a sheet from the bottom of the screen
        let rows = [("yt/set_quality", "Quality", "Auto (1080p)"), ("yt/set_cc", "Captions", if pl.cc { "English" } else { "Off" }), ("yt/set_speed", "Playback speed", "Normal"), ("yt/set_sleep", "Sleep timer", "Off"), ("yt/settings", "Additional settings", "")];
        let rh = 52.0 * k;
        let sh = rows.len() as f32 * rh + 28.0 * k;
        let screen_bottom = app.max.y;
        let r = Rect::from_min_max(pos2(player.min.x + if land { player.width() * 0.25 } else { 0.0 }, screen_bottom - sh), pos2(player.max.x - if land { player.width() * 0.25 } else { 0.0 }, screen_bottom));
        b.rect_top(r, 12.0 * k, rgba(33, 33, 33, 255));
        b.rect(Rect::from_center_size(pos2(r.center().x, r.min.y + 9.0 * k), vec2(32.0, 4.0) * k), 2.0 * k, rgba(255, 255, 255, 60));
        hits.push((r, Hit::Block));
        for (i, (icon, label, val)) in rows.iter().enumerate() {
            let row = Rect::from_min_size(pos2(r.min.x, r.min.y + 20.0 * k + i as f32 * rh), vec2(r.width(), rh));
            let cy = row.center().y;
            b.icon_at(icon, pos2(row.min.x + 28.0 * k, cy), 24.0 * k, w);
            b.text(pos2(row.min.x + 60.0 * k, cy - 8.0 * k), 15.0 * k, w, false, label, 0);
            if !val.is_empty() {
                b.text(pos2(row.min.x + 60.0 * k + text_w(label, 15.0 * k, false) + 6.0 * k, cy - 8.0 * k), 15.0 * k, sub(), false, &format!("· {val}"), 0);
            }
            hits.push((row, if *label == "Captions" { Hit::Captions } else { Hit::Block }));
        }
    }
}

/// The app's page under the player (portrait).
fn phone_page(b: &mut B, v: &View, player: Rect, app: Rect, k: f32, mut hits: Option<&mut Vec<(Rect, Hit)>>) {
    let p = v.prof;
    let x0 = app.min.x + 12.0 * k;
    let w = app.width() - 24.0 * k;
    let mut y = player.max.y + 12.0 * k;
    let title = if v.title.is_empty() { "Your video title goes here" } else { v.title };
    for l in wrap(title, 18.0 * k, true, w).iter().take(2) {
        b.text(pos2(x0, y), 18.0 * k, txt(), true, l, 0);
        y += 24.0 * k;
    }
    let meta = format!("{} views  2w ago  {}", p.views, p.tags);
    let meta = ellipsize(&meta, 12.0 * k, false, w - 50.0 * k);
    b.text(pos2(x0, y + 2.0 * k), 12.0 * k, sub(), false, &meta, 0);
    b.text(pos2(x0 + text_w(&meta, 12.0 * k, false) + 6.0 * k, y + 2.0 * k), 12.0 * k, txt(), true, "...more", 0);
    if let Some(h) = hits.as_deref_mut() {
        h.push((Rect::from_min_size(pos2(x0, y - 50.0 * k), vec2(w, 70.0 * k)), Hit::Open(Sheet::Desc)));
    }
    y += 28.0 * k;
    // channel row
    let cy = y + 18.0 * k;
    b.circle(pos2(x0 + 16.0 * k, cy), 16.0 * k, rgba(70, 70, 76, 255));
    b.avatar(pos2(x0 + 16.0 * k, cy), 16.0 * k);
    let name = ellipsize(&p.display, 14.0 * k, true, w * 0.45);
    b.text(pos2(x0 + 42.0 * k, cy - 8.0 * k), 14.0 * k, txt(), true, &name, 0);
    b.text(pos2(x0 + 48.0 * k + text_w(&name, 14.0 * k, true), cy - 7.0 * k), 12.0 * k, sub(), false, &p.followers, 0);
    let pill = Rect::from_min_max(pos2(x0 + w - 92.0 * k, cy - 18.0 * k), pos2(x0 + w, cy + 18.0 * k));
    b.rect(pill, 18.0 * k, rgba(241, 241, 241, 255));
    b.text(pos2(pill.center().x, pill.min.y + 10.0 * k), 14.0 * k, PAGE, true, "Subscribe", 1);
    y += 48.0 * k;
    // action chips (they scroll sideways: the last ones run off the edge)
    let mut cx = x0;
    let chips: [(&str, &str); 5] = [("yt/like", p.likes.as_str()), ("yt/share", "Share"), ("yt/save", "Save"), ("yt/more_h", "Clip"), ("yt/miniplayer", "Download")];
    b.clip(Some(app));
    for (i, (icon, label)) in chips.iter().enumerate() {
        let lw = text_w(label, 14.0 * k, true);
        let cw = if i == 0 { lw + 92.0 * k } else { lw + 52.0 * k };
        let r = Rect::from_min_size(pos2(cx, y), vec2(cw, 36.0 * k));
        b.rect(r, 18.0 * k, chip());
        b.icon_at(icon, pos2(r.min.x + 20.0 * k, r.center().y), 20.0 * k, txt());
        b.text(pos2(r.min.x + 36.0 * k, r.min.y + 10.0 * k), 14.0 * k, txt(), true, label, 0);
        if i == 0 {
            let dx = r.min.x + 36.0 * k + lw + 10.0 * k;
            b.rect(Rect::from_min_max(pos2(dx, r.min.y + 8.0 * k), pos2(dx + 1.0 * k, r.max.y - 8.0 * k)), 0.0, rgba(255, 255, 255, 51));
            b.icon_at("yt/dislike", pos2(r.max.x - 22.0 * k, r.center().y), 20.0 * k, txt());
        }
        if *label == "Share" {
            if let Some(h) = hits.as_deref_mut() {
                h.push((r, Hit::Open(Sheet::Share)));
            }
        }
        cx += cw + 8.0 * k;
    }
    b.clip(None);
    y += 48.0 * k;
    // the comments teaser
    let card = Rect::from_min_size(pos2(x0, y), vec2(w, 76.0 * k));
    b.rect(card, 12.0 * k, chip());
    b.text(card.min + vec2(12.0, 10.0) * k, 14.0 * k, txt(), true, "Comments", 0);
    b.text(card.min + vec2(20.0 + text_w("Comments", 14.0, true), 10.0) * k, 14.0 * k, sub(), false, &p.comments, 0);
    let c0 = p.comment_list.first().cloned().unwrap_or_default();
    b.circle(card.min + vec2(24.0, 50.0) * k, 12.0 * k, super::device::name_color(&c0.name));
    b.text(card.min + vec2(44.0, 42.0) * k, 13.0 * k, txt(), false, &ellipsize(&c0.text, 13.0 * k, false, w - 64.0 * k), 0);
    if let Some(h) = hits.as_deref_mut() {
        h.push((card, Hit::Open(Sheet::Comments)));
    }
    y += 76.0 * k + 16.0 * k;
    // the next video
    let th = Rect::from_min_size(pos2(app.min.x, y), vec2(app.width(), app.width() * 9.0 / 16.0));
    if th.max.y < app.max.y + th.height() * 0.6 {
        b.rect(th.intersect(app), 0.0, rgba(48, 50, 56, 255));
    }
}
