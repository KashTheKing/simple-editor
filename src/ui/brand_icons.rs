//! Platform marks for the Export pane's preset cards (YouTube, TikTok, Instagram, …). The pictures are
//! Simple Icons' CC0 SVG paths, rasterised ahead of time to 64 px white-on-transparent PNGs
//! (`scripts/brand-icons.sh`, sources and notice in `library/icons/brands/`) and embedded here, so the
//! exe needs no SVG renderer. Each is tinted at paint time in the caller's colour (the Export pane
//! paints them in its text colour, one calm tone). The marks only identify the platform; the trademarks
//! belong to their owners.

use eframe::egui::{self, Color32};

/// (key, PNG bytes).
const BRANDS: &[(&str, &[u8])] = &[
    ("youtube", include_bytes!("../../library/icons/brands/youtube.png")),
    ("youtubeshorts", include_bytes!("../../library/icons/brands/youtubeshorts.png")),
    ("tiktok", include_bytes!("../../library/icons/brands/tiktok.png")),
    ("instagram", include_bytes!("../../library/icons/brands/instagram.png")),
    ("x", include_bytes!("../../library/icons/brands/x.png")),
    ("facebook", include_bytes!("../../library/icons/brands/facebook.png")),
    ("vimeo", include_bytes!("../../library/icons/brands/vimeo.png")),
    ("twitch", include_bytes!("../../library/icons/brands/twitch.png")),
];

/// Every brand key, in picker order.
pub fn keys() -> impl Iterator<Item = &'static str> {
    BRANDS.iter().map(|b| b.0)
}

/// Is `key` a brand mark (rather than a tool glyph name)?
pub fn is_brand(key: &str) -> bool {
    BRANDS.iter().any(|b| b.0 == key)
}

/// The mark's texture, decoded and uploaded once per context (kept in the context's temp data).
pub fn texture(ctx: &egui::Context, key: &str) -> Option<egui::TextureHandle> {
    let (key, bytes) = BRANDS.iter().find(|b| b.0 == key)?;
    let id = egui::Id::new(("brand_icon", *key));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let f = crate::engine::html::decode_png(bytes).ok()?;
    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
    let opts = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let t = ctx.load_texture(format!("brand-{key}"), img, opts);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

/// Paint the mark `key` filling `rect` (square), tinted `tint`. False when `key` is not a brand.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, key: &str, tint: Color32) -> bool {
    let Some(t) = texture(painter.ctx(), key) else { return false };
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(t.id(), rect, uv, tint);
    true
}

/// The brand keys a preset's card shows: its own `icon` list, else guessed from its name (a
/// settings.json from before the field existed).
pub fn guess(name: &str) -> Vec<&'static str> {
    let n = name.to_ascii_lowercase();
    let mut out = Vec::new();
    if n.contains("shorts") {
        out.push("youtubeshorts");
    } else if n.contains("youtube") {
        out.push("youtube");
    }
    for (word, key) in [("reels", "instagram"), ("instagram", "instagram"), ("tiktok", "tiktok"), ("twitter", "x")] {
        if n.contains(word) && !out.contains(&key) {
            out.push(key);
        }
    }
    for key in ["facebook", "vimeo", "twitch"] {
        if n.contains(key) {
            out.push(key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mark_decodes_to_a_64px_alpha_mask() {
        for (key, bytes) in BRANDS {
            let f = crate::engine::html::decode_png(bytes).unwrap_or_else(|e| panic!("{key}: {e}"));
            assert_eq!((f.width, f.height), (64, 64), "{key}");
            // white glyph on a transparent ground: some pixels on, some off
            let on = f.rgba.chunks_exact(4).filter(|p| p[3] > 200).count();
            assert!(on > 200 && on < 64 * 64 - 200, "{key}: {on} opaque px");
            assert!(
                f.rgba.chunks_exact(4).all(|p| p[3] < 8 || p[0] > 240),
                "{key}: the glyph is white (tinted at paint)"
            );
        }
    }

    #[test]
    fn names_guess_their_platforms() {
        assert_eq!(guess("YouTube 4K"), vec!["youtube"]);
        assert_eq!(guess("Shorts / Reels / TikTok"), vec!["youtubeshorts", "instagram", "tiktok"]);
        assert_eq!(guess("Instagram 1:1"), vec!["instagram"]);
        assert!(guess("My cut").is_empty());
        assert!(keys().all(is_brand));
    }
}
