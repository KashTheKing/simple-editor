//! The device around the picture: the phone / tablet body (frame colour, bezel, side buttons), its screen
//! cut-out (Dynamic Island, notch, punch hole), the system bars (iOS or Android status bar, home indicator,
//! gesture handle or three-button bar), a desktop browser window and a TV. Portrait or landscape: landscape
//! is the phone turned with its top to the left, so the cut-out sits at the left edge and the buttons that
//! were on the left side are along the bottom. Draw lists only (`B`), like the rest of the guides.

use super::scene::*;
use super::Device;
use eframe::egui::{pos2, vec2, Color32, Pos2, Rect};

/// Where the body landed.
pub struct Body {
    pub outer: Rect,
    pub screen: Rect,
    /// Points per dp.
    pub k: f32,
    pub bezel: f32,
}

/// `#rrggbb` (anything else = dark grey).
pub fn hex(s: &str) -> Color32 {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0x2a2c2e);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Bezel width in dp: the black glass border between the frame and the screen.
pub fn bezel_dp(dev: &Device) -> f32 {
    match dev.k.as_str() {
        "tablet" => 16.0,
        "tv" => 10.0,
        "desktop" => 0.0,
        _ if dev.os == "ios" => 6.0,
        _ => 7.0,
    }
}

/// Height of the browser window's tab strip + address bar (dp), on top of the page.
pub const BROWSER_BAR: f32 = 76.0;
/// Room under a TV for its stand (dp).
pub const TV_STAND: f32 = 34.0;

/// The size the whole device takes (dp): screen + bezels (+ the browser bars, + the TV stand).
pub fn outer_dp(dev: &Device) -> (f32, f32) {
    let bz = bezel_dp(dev);
    match dev.k.as_str() {
        "desktop" => (dev.w, dev.h + BROWSER_BAR),
        "tv" => (dev.w + 2.0 * bz, dev.h + 2.0 * bz + TV_STAND),
        _ => (dev.w + 2.0 * bz, dev.h + 2.0 * bz),
    }
}

/// Draw the body inside `inner` (centred, as large as fits) and return where its screen is. `dev` is already
/// turned for landscape (`Device::turned`); `land` only moves the side buttons.
pub fn body(b: &mut B, dev: &Device, land: bool, inner: Rect) -> Body {
    let (ow, oh) = outer_dp(dev);
    // side buttons stick out ~3 dp: keep them inside the area
    let k = ((inner.width() - 8.0) / ow).min((inner.height() - 8.0) / oh).max(0.01);
    let outer_all = Rect::from_center_size(inner.center(), vec2(ow, oh) * k);
    let bz = bezel_dp(dev);
    let frame = hex(&dev.col);
    match dev.k.as_str() {
        "desktop" => {
            let outer = outer_all;
            b.rect(outer, 8.0 * k, rgba(32, 33, 36, 255));
            b.frame(outer, 8.0 * k, 1.0, rgba(70, 72, 76, 255));
            let screen = Rect::from_min_max(pos2(outer.min.x, outer.min.y + BROWSER_BAR * k), outer.max);
            b.rect(screen, 0.0, rgba(15, 15, 15, 255));
            Body { outer, screen, k, bezel: 0.0 }
        }
        "tv" => {
            let outer = Rect::from_min_size(outer_all.min, vec2(ow, oh - TV_STAND) * k);
            // stand: a slim neck and a wide foot
            let cx = outer.center().x;
            b.rect(Rect::from_min_max(pos2(cx - 30.0 * k, outer.max.y), pos2(cx + 30.0 * k, outer.max.y + 20.0 * k)), 0.0, rgba(40, 40, 44, 255));
            b.rect(Rect::from_min_max(pos2(cx - 150.0 * k, outer.max.y + 20.0 * k), pos2(cx + 150.0 * k, outer.max.y + 28.0 * k)), 3.0 * k, rgba(52, 52, 58, 255));
            b.rect(outer, 3.0 * k, frame);
            b.frame(outer, 3.0 * k, 1.0, rgba(70, 70, 76, 255));
            let screen = outer.shrink(bz * k);
            b.rect(screen, 0.0, Color32::BLACK);
            Body { outer, screen, k, bezel: bz * k }
        }
        _ => {
            let outer = outer_all;
            side_buttons(b, dev, land, outer, k, frame);
            let rad = (dev.rad + bz) * k;
            b.rect(outer, rad, frame);
            // the metal edge catches light: a lighter line just inside the outline
            b.frame(outer.shrink(1.2 * k), rad - 1.2 * k, 0.8 * k, frame.gamma_multiply(1.6));
            let glass = outer.shrink(2.4 * k);
            b.rect(glass, rad - 2.4 * k, rgba(6, 6, 8, 255));
            let screen = outer.shrink(bz * k);
            b.rect(screen, dev.rad * k, Color32::BLACK);
            Body { outer, screen, k, bezel: bz * k }
        }
    }
}

/// The buttons along the frame, in the frame colour. Portrait: iPhone = Action button + volume on the left,
/// side button (and Camera Control on a Pro) on the right; Android = power and volume on the right.
fn side_buttons(b: &mut B, dev: &Device, land: bool, outer: Rect, k: f32, col: Color32) {
    if dev.k == "tablet" {
        // top button and the volume pair on the top-right edge
        let top = [(0.80, 0.07), (0.90, 0.05)];
        for (at, len) in top {
            seg(b, outer, k, col, land, Edge::Top, at, len);
        }
        return;
    }
    let (left, right): (&[(f32, f32)], &[(f32, f32)]) = if dev.os == "ios" {
        (&[(0.20, 0.04), (0.28, 0.07), (0.37, 0.07)], if dev.cut == "island" { &[(0.30, 0.11), (0.62, 0.06)] } else { &[(0.30, 0.11)] })
    } else {
        (&[], &[(0.24, 0.07), (0.36, 0.13)])
    };
    for &(at, len) in left {
        seg(b, outer, k, col, land, Edge::Left, at, len);
    }
    for &(at, len) in right {
        seg(b, outer, k, col, land, Edge::Right, at, len);
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Top,
}

/// One button: `at` / `len` along the edge as fractions of the PORTRAIT height (top = 0).
#[allow(clippy::too_many_arguments)]
fn seg(b: &mut B, outer: Rect, k: f32, col: Color32, land: bool, e: Edge, at: f32, len: f32) {
    let t = 3.0 * k;
    let r = if !land {
        let (y0, y1) = (outer.min.y + outer.height() * at, outer.min.y + outer.height() * (at + len));
        match e {
            Edge::Left => Rect::from_min_max(pos2(outer.min.x - t, y0), pos2(outer.min.x + 1.0, y1)),
            Edge::Right => Rect::from_min_max(pos2(outer.max.x - 1.0, y0), pos2(outer.max.x + t, y1)),
            Edge::Top => {
                let (x0, x1) = (outer.min.x + outer.width() * at, outer.min.x + outer.width() * (at + len));
                Rect::from_min_max(pos2(x0, outer.min.y - t), pos2(x1, outer.min.y + 1.0))
            }
        }
    } else {
        // turned with the top to the left: the portrait left edge is now the bottom, the right edge the top,
        // and "down the portrait edge" runs left to right
        let (x0, x1) = (outer.min.x + outer.width() * at, outer.min.x + outer.width() * (at + len));
        match e {
            Edge::Left => Rect::from_min_max(pos2(x0, outer.max.y - 1.0), pos2(x1, outer.max.y + t)),
            Edge::Right => Rect::from_min_max(pos2(x0, outer.min.y - t), pos2(x1, outer.min.y + 1.0)),
            Edge::Top => {
                let (y0, y1) = (outer.max.y - outer.height() * (at + len), outer.max.y - outer.height() * at);
                Rect::from_min_max(pos2(outer.min.x - t, y0), pos2(outer.min.x + 1.0, y1))
            }
        }
    };
    b.rect(r, 1.5 * k, col.gamma_multiply(0.9));
}

/// The screen cut-out, drawn on top of everything inside the screen.
pub fn cutout(b: &mut B, dev: &Device, land: bool, screen: Rect, k: f32) {
    let black = Color32::BLACK;
    // the cut-out's centre on the portrait top edge, mapped to this orientation
    let at = |along: f32, down: f32| -> Pos2 {
        if land { pos2(screen.min.x + down * k, screen.center().y + along * k) } else { pos2(screen.center().x + along * k, screen.min.y + down * k) }
    };
    let sz = |w: f32, h: f32| if land { vec2(h, w) * k } else { vec2(w, h) * k };
    match dev.cut.as_str() {
        "island" => b.rect(Rect::from_center_size(at(0.0, 11.0 + 18.5), sz(125.0, 37.0)), 18.5 * k, black),
        "punch" => b.circle(at(0.0, dev.st * 0.5), 5.5 * k, black),
        "notch" => {
            // a rectangle with rounded outer corners hanging from the top edge (convex, so one polygon)
            let (w, h, r) = (162.0, 32.0, 20.0);
            let mut pts = Vec::new();
            pts.push(at(-w / 2.0, -1.0));
            for i in 0..=8 {
                let a = std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
                pts.push(at(-w / 2.0 + r - r * a.cos(), h - r + r * a.sin()));
            }
            for i in 0..=8 {
                let a = std::f32::consts::FRAC_PI_2 * (1.0 - i as f32 / 8.0);
                pts.push(at(w / 2.0 - r + r * a.cos(), h - r + r * a.sin()));
            }
            pts.push(at(w / 2.0, -1.0));
            b.poly(pts, black);
        }
        _ => {}
    }
}

/// Status bar ("9:41" and the signal / Wi-Fi / battery icons, iOS or Android), the home indicator / gesture
/// handle / three-button bar. `ink` is the icon colour (white over video, dark over a light page). Hidden in
/// landscape, the way a full-screen video hides them (an iPhone keeps a dim home indicator).
pub fn system_bars(b: &mut B, dev: &Device, land: bool, screen: Rect, k: f32, ink: Color32) {
    if matches!(dev.k.as_str(), "desktop" | "tv") {
        return;
    }
    let at = |x: f32, y: f32| pos2(screen.min.x + x * k, screen.min.y + y * k);
    let w = screen.width() / k;
    let h = screen.height() / k;
    if land {
        if dev.home > 0.0 {
            b.rect(Rect::from_center_size(at(w / 2.0, h - 8.0), vec2(180.0, 5.0) * k), 2.5 * k, ink.gamma_multiply(0.45));
        }
        return;
    }
    if dev.os == "ios" {
        ios_status(b, dev, screen, k, ink);
    } else {
        android_status(b, dev, screen, k, ink);
    }
    if dev.home > 0.0 {
        b.rect(Rect::from_center_size(at(w / 2.0, h - 8.0), vec2(if dev.k == "tablet" { 300.0 } else { 140.0 }, 5.0) * k), 2.5 * k, ink);
    } else if dev.nav >= 40.0 {
        // Samsung-style three buttons: recents, home, back
        let nb = Rect::from_min_max(pos2(screen.min.x, screen.max.y - dev.nav * k), screen.max);
        b.rect(nb, 0.0, Color32::BLACK);
        let (cy, c) = (nb.center().y, screen.center().x);
        let col = white(0.85);
        for i in 0..3 {
            let x = c - 92.0 * k + (i as f32 - 1.0) * 4.5 * k;
            b.rect(Rect::from_center_size(pos2(x, cy), vec2(1.6, 13.0) * k), 0.8 * k, col);
        }
        b.frame(Rect::from_center_size(pos2(c, cy), vec2(15.0, 15.0) * k), 5.0 * k, 1.7 * k, col);
        let bx = c + 92.0 * k;
        b.line(pos2(bx + 3.5 * k, cy - 7.0 * k), pos2(bx - 3.5 * k, cy), 1.8 * k, col);
        b.line(pos2(bx - 3.5 * k, cy), pos2(bx + 3.5 * k, cy + 7.0 * k), 1.8 * k, col);
    } else if dev.nav > 0.0 {
        b.rect(Rect::from_center_size(at(w / 2.0, h - dev.nav * 0.5), vec2(108.0, 4.0) * k), 2.0 * k, ink.gamma_multiply(0.85));
    }
}

/// iOS: the time centred in the left "ear", signal bars, Wi-Fi and battery centred in the right one.
fn ios_status(b: &mut B, dev: &Device, screen: Rect, k: f32, ink: Color32) {
    let w = screen.width() / k;
    let at = |x: f32, y: f32| pos2(screen.min.x + x * k, screen.min.y + y * k);
    // the ears: either side of the island / notch (a tablet has the whole bar)
    let (cut_w, cy) = match dev.cut.as_str() {
        "island" => (125.0, 11.0 + 18.5),
        "notch" => (162.0, 16.0),
        _ => (0.0, 12.0),
    };
    if dev.k == "tablet" {
        b.text(at(20.0, cy - 7.5), 15.0 * k, ink, true, "9:41", 0);
    } else {
        let left = (w - cut_w) / 4.0;
        b.text(at(left + 4.0, cy - 9.0), 17.0 * k, ink, true, "9:41", 1);
    }
    let right_c = if dev.k == "tablet" { w - 52.0 } else { w - (w - cut_w) / 4.0 - 2.0 };
    // signal: four bars, rising
    let x0 = right_c - 34.0;
    for i in 0..4 {
        let hh = 4.0 + i as f32 * 2.4;
        b.rect(Rect::from_min_max(at(x0 + i as f32 * 4.6, cy + 5.5 - hh), at(x0 + i as f32 * 4.6 + 3.2, cy + 5.5)), 0.9 * k, ink);
    }
    // Wi-Fi: a fan of three arcs over a dot
    let wc = at(right_c - 4.0, cy + 5.0);
    for (i, r) in [3.0f32, 6.8, 10.6].iter().enumerate() {
        if i == 0 {
            b.circle(pos2(wc.x, wc.y - 1.6 * k), 2.2 * k, ink);
            continue;
        }
        let n = 10;
        let pts: Vec<Pos2> = (0..=n)
            .map(|j| {
                let a = -std::f32::consts::FRAC_PI_2 + (j as f32 / n as f32 - 0.5) * 1.55;
                pos2(wc.x + r * k * a.cos(), wc.y + r * k * a.sin())
            })
            .collect();
        for s in pts.windows(2) {
            b.line(s[0], s[1], 2.1 * k, ink);
        }
    }
    // battery: outline at 35 %, the charge, the nub
    let bx = right_c + 10.0;
    let br = Rect::from_min_max(at(bx, cy - 6.0), at(bx + 25.0, cy + 6.0));
    b.frame(br, 3.8 * k, 1.0 * k, ink.gamma_multiply(0.4));
    b.rect(br.shrink(2.0 * k), 2.3 * k, ink);
    b.rect(Rect::from_min_max(at(bx + 26.2, cy - 2.2), at(bx + 27.6, cy + 2.2)), 0.7 * k, ink.gamma_multiply(0.4));
}

/// Android: the time at the left; Wi-Fi, signal and battery (Material Symbols, as the system draws them) at the
/// right, a Galaxy adds the battery percentage.
fn android_status(b: &mut B, dev: &Device, screen: Rect, k: f32, ink: Color32) {
    let w = screen.width() / k;
    let at = |x: f32, y: f32| pos2(screen.min.x + x * k, screen.min.y + y * k);
    let cy = dev.st * 0.5;
    b.text(at(18.0, cy - 7.0), 14.0 * k, ink, false, "12:30", 0);
    let s = 17.0 * k;
    let mut x = w - 18.0;
    if dev.pct != 0 {
        b.text(at(x, cy - 6.5), 13.0 * k, ink, false, "85%", 2);
        x -= 30.0;
    }
    b.icon_at("android/battery_full", at(x - 6.0, cy), s, ink);
    b.icon_at("android/signal_cellular_4_bar", at(x - 24.0, cy), s * 0.92, ink);
    b.icon_at("android/wifi", at(x - 43.0, cy), s, ink);
}

/// The browser window round a desktop page: one tab with the site's name, the window buttons, the address bar.
pub fn browser(b: &mut B, outer: Rect, k: f32, title: &str, url: &str) {
    let at = |x: f32, y: f32| pos2(outer.min.x + x * k, outer.min.y + y * k);
    let w = outer.width() / k;
    // tab strip
    let tab = Rect::from_min_max(at(8.0, 8.0), at(248.0, 40.0));
    b.rect_top(tab, 8.0 * k, rgba(53, 54, 58, 255));
    b.circle(at(24.0, 24.0), 7.0 * k, rgba(255, 0, 51, 255));
    b.poly(vec![at(22.0, 20.5), at(22.0, 27.5), at(27.5, 24.0)], Color32::WHITE);
    b.text(at(38.0, 17.5), 12.0 * k, white(0.9), false, &ellipsize(title, 12.0 * k, false, 180.0 * k), 0);
    b.line(at(256.0, 24.0), at(268.0, 24.0), 1.3 * k, white(0.7));
    b.line(at(262.0, 18.0), at(262.0, 30.0), 1.3 * k, white(0.7));
    // window buttons
    for (i, g) in ["min", "max", "close"].iter().enumerate() {
        let c = at(w - 115.0 + i as f32 * 46.0, 20.0);
        let col = white(0.85);
        match *g {
            "min" => b.line(c + vec2(-5.0, 0.0) * k, c + vec2(5.0, 0.0) * k, 1.0 * k, col),
            "max" => b.frame(Rect::from_center_size(c, vec2(10.0, 10.0) * k), 1.0 * k, 1.0 * k, col),
            _ => {
                b.line(c + vec2(-5.0, -5.0) * k, c + vec2(5.0, 5.0) * k, 1.0 * k, col);
                b.line(c + vec2(-5.0, 5.0) * k, c + vec2(5.0, -5.0) * k, 1.0 * k, col);
            }
        }
    }
    // toolbar: back, forward, reload, the address pill
    let bar = Rect::from_min_max(at(0.0, 40.0), at(w, BROWSER_BAR));
    b.rect(bar, 0.0, rgba(53, 54, 58, 255));
    let cy = 58.0;
    let col = white(0.8);
    for (i, dir) in [-1.0f32, 1.0].iter().enumerate() {
        let c = at(22.0 + i as f32 * 34.0, cy);
        b.line(c + vec2(-6.0 * dir, 0.0) * k, c + vec2(6.0 * dir, 0.0) * k, 1.5 * k, if i == 0 { col } else { white(0.35) });
        b.line(c + vec2(-6.0 * dir, 0.0) * k, c + vec2(-1.0 * dir, -5.0) * k, 1.5 * k, if i == 0 { col } else { white(0.35) });
        b.line(c + vec2(-6.0 * dir, 0.0) * k, c + vec2(-1.0 * dir, 5.0) * k, 1.5 * k, if i == 0 { col } else { white(0.35) });
    }
    b.ring(at(90.0, cy), 6.0 * k, 1.5 * k, col);
    let pill = Rect::from_min_max(at(112.0, cy - 15.0), at(w - 90.0, cy + 15.0));
    b.rect(pill, 15.0 * k, rgba(32, 33, 36, 255));
    b.text(pill.min + vec2(40.0, 8.0) * k, 13.5 * k, white(0.9), false, url, 0);
    b.ring(pill.min + vec2(20.0, 15.0) * k, 5.5 * k, 1.3 * k, white(0.6));
    b.circle(at(w - 54.0, cy), 13.0 * k, rgba(95, 99, 104, 255));
}

/// The mask over everything drawn inside the screen: the backdrop round the rounded screen corners (anything in
/// a square corner disappears), then the body's rim drawn again on top - the glass bezel, the metal frame and
/// the side buttons - so nothing shows outside the screen or over the frame.
pub fn mask(b: &mut B, dev: &Device, land: bool, body: &Body, backdrop: Color32) {
    let (screen, outer, k) = (body.screen, body.outer, body.k);
    let rad = dev.rad * k;
    if matches!(dev.k.as_str(), "desktop" | "tv") {
        let w = 8.0 * k + 2.0;
        b.frame(outer.expand(w * 0.5), if dev.k == "desktop" { 8.0 * k + w * 0.5 } else { 3.0 * k + w * 0.5 }, w, backdrop);
        return;
    }
    let w = body.bezel + rad * 0.45 + 4.0;
    b.frame(screen.expand(w * 0.5), rad + w * 0.5, w, backdrop);
    let frame = hex(&dev.col);
    side_buttons(b, dev, land, outer, k, frame);
    let bz = body.bezel;
    let orad = rad + bz;
    // glass: from the screen edge out to the frame
    let gw = bz - 2.4 * k;
    b.frame(screen.expand(gw * 0.5), rad + gw * 0.5, gw + 0.6, rgba(6, 6, 8, 255));
    // metal: the outer 2.4 dp
    let fw = 2.4 * k;
    b.frame(outer.shrink(fw * 0.5), orad - fw * 0.5, fw, frame);
    b.frame(outer.shrink(1.2 * k), orad - 1.2 * k, 0.8 * k, frame.gamma_multiply(1.6));
}
