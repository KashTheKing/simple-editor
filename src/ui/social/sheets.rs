//! The sheets a tap opens (comments, description, share, more, profile, sound).

use super::profile::net_name;
use super::device::*;
use super::device::Sheet;
use super::scene::*;
use super::*;
use eframe::egui::{Color32, Pos2, Rect};

// ---------------------------------------------------------------- sheets

pub(super) fn close_x(b: &mut B, r: Rect, k: f32, hits: &mut Vec<(Rect, Hit)>) {
    let c = pos2(r.max.x - 26.0 * k, r.min.y + 28.0 * k);
    let d = 6.0 * k;
    b.line(c + vec2(-d, -d), c + vec2(d, d), 1.8 * k, Color32::WHITE);
    b.line(c + vec2(-d, d), c + vec2(d, -d), 1.8 * k, Color32::WHITE);
    hits.push((Rect::from_center_size(c, vec2(40.0, 40.0) * k), Hit::Close));
}

/// One sheet's panel and its contents inside `r`.
pub(super) fn sheet_body(b: &mut B, v: &View, s: Sheet, r: Rect, k: f32, round: bool, hits: &mut Vec<(Rect, Hit)>) {
    if s == Sheet::Comments && round && app_comments(b, v, r, k, hits) {
        return;
    }
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

// ---------------------------------------------------------------- each app's own comments sheet

/// The comments sheet the way the platform's app draws it: YouTube (dark, "Comments" + count, Top / Newest,
/// like / dislike / reply), TikTok (white, "N comments" centred, a heart and count at the right of each row,
/// "Add comment..." with @ and emoji), Instagram (dark, "Comments" centred, a heart at the right). False when
/// the platform has no own style (the generic sheet is drawn).
pub(super) fn app_comments(b: &mut B, v: &View, r: Rect, k: f32, hits: &mut Vec<(Rect, Hit)>) -> bool {
    match v.plat.net.as_str() {
        "youtube" => youtube_comments(b, v, r, k, hits),
        "tiktok" => tiktok_comments(b, v, r, k, hits),
        "instagram" => instagram_comments(b, v, r, k, hits),
        _ => return false,
    }
    true
}

fn sheet_frame(b: &mut B, r: Rect, k: f32, bg: Color32, handle: Color32, hits: &mut Vec<(Rect, Hit)>) {
    hits.push((r, Hit::Block));
    b.rect_top(r, 12.0 * k, bg);
    b.rect(Rect::from_center_size(pos2(r.center().x, r.min.y + 8.0 * k), vec2(36.0, 4.0) * k), 2.0 * k, handle);
    hits.push((Rect::from_center_size(pos2(r.center().x, r.min.y + 10.0 * k), vec2(120.0, 24.0) * k), Hit::Close));
}

fn x_close(b: &mut B, c: Pos2, k: f32, col: Color32, hits: &mut Vec<(Rect, Hit)>) {
    let d = 6.5 * k;
    b.line(c + vec2(-d, -d), c + vec2(d, d), 1.8 * k, col);
    b.line(c + vec2(-d, d), c + vec2(d, -d), 1.8 * k, col);
    hits.push((Rect::from_center_size(c, vec2(40.0, 40.0) * k), Hit::Close));
}

fn initial_avatar(b: &mut B, c: Pos2, r: f32, name: &str) {
    b.circle(c, r, name_color(name));
    let ch = name.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
    b.text(pos2(c.x, c.y - r * 0.55), r * 1.05, Color32::WHITE, true, &ch, 1);
}

fn youtube_comments(b: &mut B, v: &View, r: Rect, k: f32, hits: &mut Vec<(Rect, Hit)>) {
    let p = v.prof;
    let (txt, sub) = (rgba(241, 241, 241, 255), rgba(170, 170, 170, 255));
    sheet_frame(b, r, k, rgba(33, 33, 33, 255), rgba(255, 255, 255, 60), hits);
    let x0 = r.min.x + 16.0 * k;
    b.text(pos2(x0, r.min.y + 24.0 * k), 18.0 * k, txt, true, "Comments", 0);
    b.text(pos2(x0 + text_w("Comments", 18.0 * k, true) + 8.0 * k, r.min.y + 26.0 * k), 16.0 * k, sub, false, &p.comments, 0);
    b.icon_at("yt/sort", pos2(r.max.x - 64.0 * k, r.min.y + 34.0 * k), 24.0 * k, txt);
    x_close(b, pos2(r.max.x - 24.0 * k, r.min.y + 34.0 * k), k, txt, hits);
    // Top / Newest
    let mut cx = x0;
    for (i, l) in ["Top", "Newest"].iter().enumerate() {
        let cw = text_w(l, 14.0 * k, true) + 24.0 * k;
        let c = Rect::from_min_size(pos2(cx, r.min.y + 58.0 * k), vec2(cw, 32.0 * k));
        b.rect(c, 8.0 * k, if i == 0 { txt } else { rgba(255, 255, 255, 26) });
        b.text(pos2(c.center().x, c.min.y + 8.0 * k), 14.0 * k, if i == 0 { rgba(15, 15, 15, 255) } else { txt }, true, l, 1);
        cx += cw + 8.0 * k;
    }
    b.rect(Rect::from_min_max(pos2(r.min.x, r.min.y + 100.0 * k), pos2(r.max.x, r.min.y + 101.0 * k)), 0.0, rgba(255, 255, 255, 26));
    let input_h = 56.0 * k;
    b.clip(Some(Rect::from_min_max(pos2(r.min.x, r.min.y + 101.0 * k), pos2(r.max.x, r.max.y - input_h))));
    let mut y = r.min.y + 114.0 * k;
    let w = r.width() - 32.0 * k;
    for c in &p.comment_list {
        initial_avatar(b, pos2(x0 + 12.0 * k, y + 12.0 * k), 12.0 * k, &c.name);
        b.text(pos2(x0 + 36.0 * k, y), 12.0 * k, sub, false, &format!("@{} · {} ago", c.name, c.when), 0);
        let lines = wrap(&c.text, 14.0 * k, false, w - 48.0 * k);
        for (i, l) in lines.iter().take(4).enumerate() {
            b.text(pos2(x0 + 36.0 * k, y + 18.0 * k + i as f32 * 19.0 * k), 14.0 * k, txt, false, l, 0);
        }
        let ty = y + 18.0 * k + lines.len().min(4) as f32 * 19.0 * k + 14.0 * k;
        b.icon_at("yt/like", pos2(x0 + 44.0 * k, ty), 16.0 * k, txt);
        b.text(pos2(x0 + 58.0 * k, ty - 6.0 * k), 12.0 * k, sub, false, &c.likes, 0);
        let dx = x0 + 70.0 * k + text_w(&c.likes, 12.0 * k, false) + 16.0 * k;
        b.icon_at("yt/dislike", pos2(dx, ty), 16.0 * k, txt);
        b.text(pos2(dx + 30.0 * k, ty - 6.0 * k), 12.0 * k, sub, true, "Reply", 0);
        b.icon_at("yt/more_v", pos2(r.max.x - 22.0 * k, y + 6.0 * k), 18.0 * k, txt);
        y = ty + 24.0 * k;
    }
    b.clip(None);
    let ir = Rect::from_min_max(pos2(r.min.x, r.max.y - input_h), r.max);
    b.rect(ir, 0.0, rgba(33, 33, 33, 255));
    b.rect(Rect::from_min_max(ir.min, pos2(ir.max.x, ir.min.y + 1.0 * k)), 0.0, rgba(255, 255, 255, 26));
    b.circle(pos2(x0 + 12.0 * k, ir.center().y), 12.0 * k, rgba(70, 70, 76, 255));
    b.avatar(pos2(x0 + 12.0 * k, ir.center().y), 12.0 * k);
    let pill = Rect::from_min_max(pos2(x0 + 34.0 * k, ir.center().y - 18.0 * k), pos2(r.max.x - 16.0 * k, ir.center().y + 18.0 * k));
    b.rect(pill, 18.0 * k, rgba(255, 255, 255, 26));
    b.text(pos2(pill.min.x + 14.0 * k, pill.min.y + 10.0 * k), 14.0 * k, sub, false, "Add a comment...", 0);
}

fn tiktok_comments(b: &mut B, v: &View, r: Rect, k: f32, hits: &mut Vec<(Rect, Hit)>) {
    let p = v.prof;
    let (ink, grey) = (rgba(22, 24, 35, 255), rgba(138, 139, 145, 255));
    hits.push((r, Hit::Block));
    b.rect_top(r, 10.0 * k, Color32::WHITE);
    b.text(pos2(r.center().x, r.min.y + 16.0 * k), 13.0 * k, ink, true, &format!("{} comments", p.comments), 1);
    x_close(b, pos2(r.max.x - 22.0 * k, r.min.y + 24.0 * k), k, ink, hits);
    let input_h = 54.0 * k;
    b.clip(Some(Rect::from_min_max(pos2(r.min.x, r.min.y + 46.0 * k), pos2(r.max.x, r.max.y - input_h))));
    let x0 = r.min.x + 16.0 * k;
    let mut y = r.min.y + 52.0 * k;
    let w = r.width() - 32.0 * k;
    for c in &p.comment_list {
        initial_avatar(b, pos2(x0 + 16.0 * k, y + 16.0 * k), 16.0 * k, &c.name);
        b.text(pos2(x0 + 44.0 * k, y), 13.0 * k, grey, true, &c.name, 0);
        let lines = wrap(&c.text, 15.0 * k, false, w - 90.0 * k);
        for (i, l) in lines.iter().take(4).enumerate() {
            b.text(pos2(x0 + 44.0 * k, y + 19.0 * k + i as f32 * 20.0 * k), 15.0 * k, ink, false, l, 0);
        }
        let ty = y + 19.0 * k + lines.len().min(4) as f32 * 20.0 * k + 4.0 * k;
        b.text(pos2(x0 + 44.0 * k, ty), 13.0 * k, grey, false, &format!("{} ago", c.when), 0);
        b.text(pos2(x0 + 52.0 * k + text_w(&format!("{} ago", c.when), 13.0 * k, false), ty), 13.0 * k, grey, true, "Reply", 0);
        b.icon_at("tt/heart_outline", pos2(r.max.x - 28.0 * k, y + 22.0 * k), 18.0 * k, grey);
        b.text(pos2(r.max.x - 28.0 * k, y + 34.0 * k), 12.0 * k, grey, false, &c.likes, 1);
        y = ty + 34.0 * k;
    }
    b.clip(None);
    let ir = Rect::from_min_max(pos2(r.min.x, r.max.y - input_h), r.max);
    b.rect(ir, 0.0, Color32::WHITE);
    b.rect(Rect::from_min_max(ir.min, pos2(ir.max.x, ir.min.y + 1.0 * k)), 0.0, rgba(22, 24, 35, 30));
    b.circle(pos2(x0 + 16.0 * k, ir.center().y), 16.0 * k, rgba(70, 70, 76, 255));
    b.avatar(pos2(x0 + 16.0 * k, ir.center().y), 16.0 * k);
    let pill = Rect::from_min_max(pos2(x0 + 40.0 * k, ir.center().y - 18.0 * k), pos2(r.max.x - 16.0 * k, ir.center().y + 18.0 * k));
    b.rect(pill, 18.0 * k, rgba(241, 241, 242, 255));
    b.text(pos2(pill.min.x + 14.0 * k, pill.min.y + 10.0 * k), 15.0 * k, grey, false, "Add comment...", 0);
    b.icon_at("tt/at", pos2(pill.max.x - 52.0 * k, pill.center().y), 22.0 * k, ink);
    b.icon_at("tt/emoji", pos2(pill.max.x - 22.0 * k, pill.center().y), 22.0 * k, ink);
}

fn instagram_comments(b: &mut B, v: &View, r: Rect, k: f32, hits: &mut Vec<(Rect, Hit)>) {
    let p = v.prof;
    let (txt, grey) = (rgba(245, 245, 245, 255), rgba(168, 168, 168, 255));
    sheet_frame(b, r, k, rgba(38, 38, 38, 255), rgba(255, 255, 255, 80), hits);
    b.text(pos2(r.center().x, r.min.y + 24.0 * k), 16.0 * k, txt, true, "Comments", 1);
    b.rect(Rect::from_min_max(pos2(r.min.x, r.min.y + 54.0 * k), pos2(r.max.x, r.min.y + 55.0 * k)), 0.0, rgba(255, 255, 255, 30));
    let input_h = 64.0 * k;
    b.clip(Some(Rect::from_min_max(pos2(r.min.x, r.min.y + 55.0 * k), pos2(r.max.x, r.max.y - input_h))));
    let x0 = r.min.x + 16.0 * k;
    let mut y = r.min.y + 68.0 * k;
    let w = r.width() - 32.0 * k;
    for c in &p.comment_list {
        initial_avatar(b, pos2(x0 + 16.0 * k, y + 16.0 * k), 16.0 * k, &c.name);
        b.text(pos2(x0 + 44.0 * k, y), 12.0 * k, txt, true, &c.name, 0);
        b.text(pos2(x0 + 50.0 * k + text_w(&c.name, 12.0 * k, true), y), 12.0 * k, grey, false, &c.when, 0);
        let lines = wrap(&c.text, 14.0 * k, false, w - 80.0 * k);
        for (i, l) in lines.iter().take(4).enumerate() {
            b.text(pos2(x0 + 44.0 * k, y + 17.0 * k + i as f32 * 19.0 * k), 14.0 * k, txt, false, l, 0);
        }
        let ty = y + 17.0 * k + lines.len().min(4) as f32 * 19.0 * k + 4.0 * k;
        b.text(pos2(x0 + 44.0 * k, ty), 12.0 * k, grey, true, "Reply", 0);
        b.icon_at("ig/heart", pos2(r.max.x - 26.0 * k, y + 18.0 * k), 14.0 * k, grey);
        b.text(pos2(r.max.x - 26.0 * k, y + 28.0 * k), 11.0 * k, grey, false, &c.likes, 1);
        y = ty + 30.0 * k;
    }
    b.clip(None);
    let ir = Rect::from_min_max(pos2(r.min.x, r.max.y - input_h), r.max);
    b.rect(ir, 0.0, rgba(38, 38, 38, 255));
    b.rect(Rect::from_min_max(ir.min, pos2(ir.max.x, ir.min.y + 1.0 * k)), 0.0, rgba(255, 255, 255, 30));
    b.circle(pos2(x0 + 18.0 * k, ir.center().y), 18.0 * k, rgba(70, 70, 76, 255));
    b.avatar(pos2(x0 + 18.0 * k, ir.center().y), 18.0 * k);
    let pill = Rect::from_min_max(pos2(x0 + 46.0 * k, ir.center().y - 20.0 * k), pos2(r.max.x - 16.0 * k, ir.center().y + 20.0 * k));
    b.frame(pill, 20.0 * k, 1.0 * k, rgba(255, 255, 255, 50));
    b.text(pos2(pill.min.x + 16.0 * k, pill.min.y + 11.0 * k), 14.0 * k, grey, false, &ellipsize(&format!("Add a comment for {}...", p.handle), 14.0 * k, false, pill.width() - 30.0 * k), 0);
}
