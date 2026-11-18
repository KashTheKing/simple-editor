//! Full-screen vertical video (Shorts, TikTok, Reels, Facebook Reels, Snapchat Spotlight) and feed posts (an
//! Instagram post, an X post with video). Called by `device::build` for `pg == "full"` and `"feed"`.

use super::device::*;
use super::device::Sheet;
use super::scene::*;
use super::sheets::sheet_body;
use super::*;
use eframe::egui::{Color32, Rect};

pub fn feed(pg: &Pg, under: &mut B, over: &mut B, hits: &mut Vec<(Rect, Hit)>) -> Placed {
    #[allow(unused_variables)]
    let Pg { v, dev, plat, lay, screen, outer, app, k, st_h, desktop, fade, vm, e, sheet_m, cx, ink } = *pg;
    if plat.net == "x" {
        return x_post(pg, under, over, hits);
    }
    let (base, video, uv, sheet_top);
    let col = if desktop || dev.k == "tablet" {
        Rect::from_center_size(app.center(), vec2(app.width().min(460.0 * k), app.height())).intersect(app)
    } else {
        landscape_safe(app, dev, k)
    };
    let hdr = 52.0 * k;
    let top = col.min.y + if desktop { 16.0 * k } else { st_h };
    // the media is as wide as the column, but never taller than the screen leaves above the footer (a phone
    // turned landscape): then it is centred at the height that fits
    let fit_h = (col.max.y - (top + hdr) - 110.0 * k).max(40.0 * k);
    let mw = col.width().min(fit_h * plat.aspect());
    let media = Rect::from_min_size(pos2(col.center().x - mw / 2.0, top + hdr), vec2(mw, mw / plat.aspect()));
    under.rect(col.intersect(Rect::from_min_max(pos2(col.min.x, top), pos2(col.max.x, col.max.y))), 0.0, rgba(0, 0, 0, 255));
    // header
    under.circle(pos2(col.min.x + 28.0 * k, top + hdr / 2.0), 17.0 * k, rgba(70, 70, 76, 255));
    under.avatar(pos2(col.min.x + 28.0 * k, top + hdr / 2.0), 16.0 * k);
    under.text(pos2(col.min.x + 54.0 * k, top + hdr / 2.0 - 8.0 * k), 14.0 * k, ink, true, &v.prof.handle_text(), 0);
    glyph(under, "more", pos2(col.max.x - 24.0 * k, top + hdr / 2.0), 9.0 * k, ink);
    let (r, u) = place(media, false, v.pa);
    (base, video, uv) = (r, r, u);
    under.rect(media, 0.0, rgba(18, 18, 20, 255));
    let map = Map::fill(lay.refsz, media);
    let tap = draw_ui(over, lay, &map, &cx, false);
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
        hits.push((r, Hit::Play));
        hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
    }
    if v.pl.buffering {
        buffering(over, &plat.net, r, k, v.pl.t);
    }
    sheet_top = media.min.y;

    Placed { base, video, uv, sheet_top }
}

/// A phone turned landscape keeps a feed app out of the cut-out / rounded corners at the sides and off the home
/// indicator at the bottom, like iOS's and Android's landscape safe area; upright, the app rect already is.
fn landscape_safe(app: Rect, dev: &Device, k: f32) -> Rect {
    if dev.k == "phone" && app.width() > app.height() {
        Rect::from_min_max(pos2(app.min.x + 48.0 * k, app.min.y), pos2(app.max.x - 48.0 * k, app.max.y - 22.0 * k))
    } else {
        app
    }
}

/// An X post with video, the way x.com's dark theme draws it (measured on a logged-out post page, 2026-11-17):
/// avatar, bold name, grey @handle; the post text ABOVE the video; the video in a 16 px rounded frame; then
/// the action row - reply, repost, like, views with their counts, bookmark and share on the right - in X's
/// own icons (`x/*`).
fn x_post(pg: &Pg, under: &mut B, over: &mut B, hits: &mut Vec<(Rect, Hit)>) -> Placed {
    #[allow(unused_variables)]
    let Pg { v, dev, plat, lay, screen, outer, app, k, st_h, desktop, fade, vm, e, sheet_m, cx, ink } = *pg;
    let grey = rgba(113, 118, 123, 255);
    let text_col = rgba(231, 233, 234, 255);
    let col = if desktop || dev.k == "tablet" {
        Rect::from_center_size(app.center(), vec2(app.width().min(600.0 * k), app.height())).intersect(app)
    } else {
        landscape_safe(app, dev, k)
    };
    let top = col.min.y + if desktop { 12.0 * k } else { st_h + 8.0 * k };
    under.rect(Rect::from_min_max(pos2(col.min.x, top - 8.0 * k), col.max), 0.0, rgba(0, 0, 0, 255));
    let pad = 16.0 * k;
    let ava = pos2(col.min.x + pad + 20.0 * k, top + 20.0 * k);
    under.circle(ava, 20.0 * k, rgba(51, 54, 57, 255));
    under.avatar(ava, 20.0 * k);
    let tx = col.min.x + pad + 52.0 * k;
    let name = v.prof.display.clone();
    under.text(pos2(tx, top + 4.0 * k), 15.0 * k, text_col, true, &name, 0);
    under.text(pos2(tx, top + 23.0 * k), 15.0 * k, grey, false, &v.prof.handle_text(), 0);
    glyph(under, "more", pos2(col.max.x - pad - 8.0 * k, top + 12.0 * k), 8.0 * k, grey);
    // the post text, then the video
    let s = 17.0 * k;
    let tw = col.width() - 2.0 * pad;
    let (lines, _) = collapse(wrap(&v.prof.caption_full(), s, false, tw), 4, s, false, tw, text_w("… Show more", s, false));
    let mut y = top + 52.0 * k;
    for l in &lines {
        under.text(pos2(col.min.x + pad, y), s, text_col, false, l, 0);
        y += s * 1.3;
    }
    hits.push((Rect::from_min_max(pos2(col.min.x, top + 48.0 * k), pos2(col.max.x, y)), Hit::Open(Sheet::Desc)));
    // as wide as the column, never taller than the screen leaves above the action row (a phone turned landscape)
    let fit_h = (col.max.y - (y + 10.0 * k) - 48.0 * k).max(40.0 * k);
    let mw = (col.width() - 2.0 * pad).min(fit_h * plat.aspect());
    let media = Rect::from_min_size(pos2(col.min.x + pad, y + 10.0 * k), vec2(mw, mw / plat.aspect()));
    under.rect(media, 16.0 * k, rgba(22, 24, 28, 255));
    under.frame(media, 16.0 * k, 1.0 * k, rgba(47, 51, 54, 255));
    let (r, u) = place(media, false, v.pa);
    let map = Map::fill(lay.refsz, media);
    let tap = draw_ui(over, lay, &map, &cx, false);
    // the action row: four counted actions spread across the width, bookmark + share at the right
    let fy = media.max.y + 22.0 * k;
    let mut foot = B::new();
    let row = [("x/reply", v.prof.comments.as_str()), ("x/repost", v.prof.shares.as_str()), ("x/like", v.prof.likes.as_str()), ("x/views", v.prof.views.as_str())];
    let span = (col.width() - 2.0 * pad - 70.0 * k) / row.len() as f32;
    for (i, (icon, n)) in row.iter().enumerate() {
        let c = pos2(col.min.x + pad + 10.0 * k + i as f32 * span, fy);
        if !foot.icon_at(icon, c, 19.0 * k, grey) {
            glyph(&mut foot, if *icon == "x/like" { "like" } else { "comment" }, c, 9.0 * k, grey);
        }
        foot.text(pos2(c.x + 16.0 * k, fy - 7.0 * k), 13.0 * k, grey, false, n, 0);
        let hit = Rect::from_center_size(pos2(c.x + 14.0 * k, fy), vec2(44.0, 36.0) * k);
        match *icon {
            "x/reply" => hits.push((hit, Hit::Open(Sheet::Comments))),
            "x/repost" => hits.push((hit, Hit::Open(Sheet::Share))),
            _ => {}
        }
    }
    for (i, icon) in ["x/share", "x/bookmark"].iter().enumerate() {
        let c = pos2(col.max.x - pad - 10.0 * k - i as f32 * 34.0 * k, fy);
        foot.icon_at(icon, c, 19.0 * k, grey);
        if *icon == "x/share" {
            hits.push((Rect::from_center_size(c, vec2(36.0, 36.0) * k), Hit::Open(Sheet::Share)));
        }
    }
    over.ops.extend(foot.ops);
    if e <= 0.0 {
        hits.push((r, Hit::Play));
        hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
    }
    if v.pl.buffering {
        buffering(over, &plat.net, r, k, v.pl.t);
    }
    Placed { base: r, video: r, uv: u, sheet_top: media.min.y }
}

pub fn full(pg: &Pg, under: &mut B, over: &mut B, hits: &mut Vec<(Rect, Hit)>) -> Placed {
    #[allow(unused_variables)]
    let Pg { v, dev, plat, lay, screen, outer, app, k, st_h, desktop, fade, vm, e, sheet_m, cx, ink } = *pg;
    if dev.k == "phone" && app.width() > app.height() {
        return landscape(pg, under, over, hits);
    }
    let (base, video, uv, sheet_top);
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
        // a tap on the video itself plays / pauses, as in every one of these apps
        hits.push((r, Hit::Play));
        hits.extend(tap.into_iter().filter_map(|(r, t)| tap_hit(&t).map(|h| (r, h))));
    }
    if v.pl.buffering {
        buffering(over, &plat.net, r, k, v.pl.t);
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
                sheet_body(&mut p, v, s, panel, k, false, hits);
            }
            over.ops.extend(p.ops);
        }
    }

    Placed { base, video, uv, sheet_top }
}

/// A phone turned on its side: the platform's full-screen player. The video fills the height (bars at the
/// sides for a vertical video), the back arrow at the top left, the creator and caption along the bottom
/// left, the progress bar under them, and the platform's own action buttons (the ones its portrait layout
/// has down the right edge) in a column at the right edge. Everything stays inside the turned screen.
fn landscape(pg: &Pg, under: &mut B, over: &mut B, hits: &mut Vec<(Rect, Hit)>) -> Placed {
    let Pg { v, plat, lay, app, k, e, cx, .. } = *pg;
    under.rect(app, 0.0, Color32::BLACK);
    let (r, uvr) = place(app, false, v.pa);
    let white = Color32::WHITE;
    if e <= 0.0 {
        hits.push((r, Hit::Play));
    }
    over.grad(Rect::from_min_max(pos2(app.min.x, app.max.y - 110.0 * k), app.max), true, 0.55);
    over.grad(Rect::from_min_max(app.min, pos2(app.max.x, app.min.y + 70.0 * k)), false, 0.4);
    // back arrow
    let back = pos2(app.min.x + 34.0 * k, app.min.y + 30.0 * k);
    over.line(back + vec2(6.0, -9.0) * k, back + vec2(-3.0, 0.0) * k, 2.2 * k, white);
    over.line(back + vec2(-3.0, 0.0) * k, back + vec2(6.0, 9.0) * k, 2.2 * k, white);
    // the action column: the portrait layout's right-edge buttons, in order
    let rw = lay.refsz[0];
    let acts: Vec<&El> = lay.ui.iter().filter(|e| e.t == "ico" && (e.r[0] + e.r[2]) * 0.5 > rw * 0.8 && e.r[1] > lay.refsz[1] * 0.3 && e.i != "disc").collect();
    let step = 58.0 * k;
    let col_x = app.max.x - 44.0 * k;
    let n = acts.len() as f32;
    let y0 = app.center().y - (n - 1.0) * step * 0.5 - 10.0 * k;
    for (i, el) in acts.iter().enumerate() {
        let c = pos2(col_x, y0 + i as f32 * step);
        if el.bg != 0 {
            over.circle(c, 21.0 * k, Color32::from_black_alpha(70));
        }
        glyph(over, &el.i, c, 12.0 * k, white);
        let t = if el.lbl.is_empty() { cx.prof.count(&el.n).to_string() } else { el.lbl.clone() };
        over.text(pos2(c.x, c.y + 16.0 * k), 11.0 * k, white, true, &t, 1);
        let h = Rect::from_center_size(c, vec2(48.0, 48.0) * k);
        match tap_hit(&el.tap) {
            Some(hit) if e <= 0.0 => hits.push((h, hit)),
            _ => {}
        }
    }
    // the creator and the caption, then the bar
    let x0 = app.min.x + 24.0 * k;
    let tw = (col_x - 40.0 * k) - x0;
    let by = app.max.y - 22.0 * k;
    over.circle(pos2(x0 + 14.0 * k, by - 52.0 * k), 14.0 * k, rgba(60, 60, 64, 255));
    over.avatar(pos2(x0 + 14.0 * k, by - 52.0 * k), 14.0 * k);
    over.text(pos2(x0 + 36.0 * k, by - 61.0 * k), 15.0 * k, white, true, &ellipsize(&cx.prof.handle_text(), 15.0 * k, true, tw * 0.5), 0);
    over.text(pos2(x0, by - 30.0 * k), 13.0 * k, white, false, &ellipsize(&cx.prof.caption_full(), 13.0 * k, false, tw), 0);
    let track = Rect::from_min_max(pos2(x0, by - 1.5 * k), pos2(col_x - 40.0 * k, by + 1.5 * k));
    over.rect(track, 1.5 * k, rgba(255, 255, 255, 70));
    let fx = track.min.x + track.width() * v.progress.clamp(0.0, 1.0);
    let bar = if plat.net == "youtube" { rgba(255, 0, 51, 255) } else { white };
    over.rect(Rect::from_min_max(track.min, pos2(fx, track.max.y)), 1.5 * k, bar);
    over.circle(pos2(fx, track.center().y), 5.0 * k, bar);
    if e <= 0.0 {
        hits.push((track.expand2(vec2(0.0, 10.0 * k)), Hit::Seek(track)));
    }
    if v.pl.buffering {
        buffering(over, &plat.net, r, k, v.pl.t);
    }
    Placed { base: r, video: r, uv: uvr, sheet_top: app.min.y }
}

/// The platform's own "waiting for video" sign over the video `r`: YouTube's white arc, Instagram's spokes,
/// X's blue ring, TikTok's white line pulsing out from the middle of the bottom edge.
pub(super) fn buffering(b: &mut B, net: &str, r: Rect, k: f32, t: f32) {
    let c = r.center();
    match net {
        "tiktok" => {
            let f = (t * 1.6).fract();
            let w = r.width() * f;
            let a = (1.0 - f).clamp(0.0, 1.0);
            b.rect(Rect::from_center_size(pos2(c.x, r.max.y - 3.0 * k), vec2(w, 2.0 * k)), 1.0 * k, white(a));
        }
        "instagram" => {
            let n = 8;
            let lit = (t * 10.0) as i32;
            for i in 0..n {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                let fade = ((i - lit).rem_euclid(n) as f32 / n as f32).max(0.15);
                let (s, cc) = (a.sin(), a.cos());
                b.line(pos2(c.x + cc * 9.0 * k, c.y + s * 9.0 * k), pos2(c.x + cc * 17.0 * k, c.y + s * 17.0 * k), 3.0 * k, white(fade));
            }
        }
        "x" => {
            b.ring(c, 16.0 * k, 3.0 * k, rgba(29, 155, 240, 60));
            super::youtube::spinner(b, c, 16.0 * k, 3.0 * k, t, rgba(29, 155, 240, 255));
        }
        _ => super::youtube::spinner(b, c, 24.0 * k, 3.5 * k, t, Color32::WHITE),
    }
}
