//! ---- feat-retro-2000s-pack ----
//! "Classic 2000s": our own remakes of the look of the early-2000s home-movie editors - blue title
//! cards with white rounded hand-lettering, the shaped wipes and irises, sepia and old film, and the
//! "pick some photos and make a slideshow" button (`engine::slideshow`). Nothing here is anyone else's
//! asset: every shader is written for this app, the font is Comic Sans MS when the PC has it, else the
//! bundled Comic Neue (SIL OFL, `assets/fonts/OFL-ComicNeue.txt`).
//!
//! The pack's shaders live in the repo's `library/` like every other library item (tagged
//! `classic 2000s`, which is the Gallery's "Classic 2000s" category), and are also compiled in here so
//! the pack works offline: `ensure_installed` copies them into the user's library folder once per pack
//! version, on a background thread, and the slideshow reads its transitions from `TRANSITIONS`.

use crate::model::*;
use crate::settings::Template;
use std::path::Path;

/// The Gallery category every tab shows the pack under.
pub const CATEGORY: &str = "Classic 2000s";
/// The library tag that puts a shader in `CATEGORY` (tags are lowercase).
#[cfg_attr(not(test), allow(dead_code))]
pub const TAG: &str = "classic 2000s";
/// Comic Sans MS when installed (never bundled), else the bundled Comic Neue (`engine::text::BUNDLED_FONTS`).
pub const FONT: &str = "Comic Sans MS, Comic Neue";
/// The signature title card's royal blue.
pub const BLUE: [u8; 4] = [30, 60, 190, 255];
/// Titles are laid out on a 1080p frame (the slideshow uses the project's own size).
const W: f64 = 1920.0;
const H: f64 = 1080.0;
/// Bump when the pack's files change, so `ensure_installed` copies the new ones once.
const PACK_VERSION: u32 = 1;

pub const TRANSITIONS: &[(&str, &str)] = &[
    ("blinds", include_str!("../../library/transitions/blinds.glsl")),
    ("classic-bars", include_str!("../../library/transitions/classic-bars.glsl")),
    ("classic-bow-tie", include_str!("../../library/transitions/classic-bow-tie.glsl")),
    ("classic-checkerboard", include_str!("../../library/transitions/classic-checkerboard.glsl")),
    ("classic-diagonal", include_str!("../../library/transitions/classic-diagonal.glsl")),
    ("classic-dissolve", include_str!("../../library/transitions/classic-dissolve.glsl")),
    ("classic-eye-iris", include_str!("../../library/transitions/classic-eye-iris.glsl")),
    ("classic-fade-black", include_str!("../../library/transitions/classic-fade-black.glsl")),
    ("classic-fade-white", include_str!("../../library/transitions/classic-fade-white.glsl")),
    ("classic-flip", include_str!("../../library/transitions/classic-flip.glsl")),
    ("classic-heart", include_str!("../../library/transitions/classic-heart.glsl")),
    ("classic-keyhole", include_str!("../../library/transitions/classic-keyhole.glsl")),
    ("classic-rectangle-zoom", include_str!("../../library/transitions/classic-rectangle-zoom.glsl")),
    ("classic-roll", include_str!("../../library/transitions/classic-roll.glsl")),
    ("classic-shatter-in", include_str!("../../library/transitions/classic-shatter-in.glsl")),
    ("classic-spiral", include_str!("../../library/transitions/classic-spiral.glsl")),
    ("classic-sweep", include_str!("../../library/transitions/classic-sweep.glsl")),
    ("classic-whirlwind", include_str!("../../library/transitions/classic-whirlwind.glsl")),
    ("classic-zigzag", include_str!("../../library/transitions/classic-zigzag.glsl")),
    ("clock-wipe", include_str!("../../library/transitions/clock-wipe.glsl")),
    ("iris-circle", include_str!("../../library/transitions/iris-circle.glsl")),
    ("iris-diamond", include_str!("../../library/transitions/iris-diamond.glsl")),
    ("iris-star", include_str!("../../library/transitions/iris-star.glsl")),
    ("page-turn", include_str!("../../library/transitions/page-turn.glsl")),
    ("pixelate", include_str!("../../library/transitions/pixelate.glsl")),
    ("ripple", include_str!("../../library/transitions/ripple.glsl")),
];
pub const EFFECTS: &[(&str, &str)] = &[
    ("classic-3d-ripple", include_str!("../../library/effects/classic-3d-ripple.glsl")),
    ("classic-black-white", include_str!("../../library/effects/classic-black-white.glsl")),
    ("classic-blur", include_str!("../../library/effects/classic-blur.glsl")),
    ("classic-edge-detect", include_str!("../../library/effects/classic-edge-detect.glsl")),
    ("classic-film-age", include_str!("../../library/effects/classic-film-age.glsl")),
    ("classic-posterize", include_str!("../../library/effects/classic-posterize.glsl")),
    ("classic-sepia", include_str!("../../library/effects/classic-sepia.glsl")),
    ("classic-watercolor", include_str!("../../library/effects/classic-watercolor.glsl")),
    ("mirror", include_str!("../../library/effects/mirror.glsl")),
    ("old-film", include_str!("../../library/effects/old-film.glsl")),
];

/// The pack's transition source by id (`classic-heart`), or None.
pub fn transition(id: &str) -> Option<&'static str> {
    TRANSITIONS.iter().find(|(i, _)| *i == id).map(|(_, s)| *s)
}

/// The pack's source of a library id (`transitions/classic-heart`), or None.
pub fn source_of(full_id: &str) -> Option<&'static str> {
    let (dir, id) = full_id.split_once('/')?;
    let items = match dir {
        "transitions" => TRANSITIONS,
        "effects" => EFFECTS,
        _ => return None,
    };
    items.iter().find(|(i, _)| *i == id).map(|(_, s)| *s)
}

/// Is this library item (by its tags) part of the pack?
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_classic(tags: &[String]) -> bool {
    tags.iter().any(|t| t.eq_ignore_ascii_case(TAG))
}

/// Copy the pack's shaders into the library folder `root` (`engine::library::root()` in the app) unless
/// this pack version already did; a file the user has there is never overwritten. Returns how many
/// files were written.
pub fn install_into(root: &Path) -> std::io::Result<usize> {
    let marker = root.join(format!(".classic-2000s-{PACK_VERSION}"));
    if marker.exists() {
        return Ok(0);
    }
    let mut n = 0;
    for (dir, items) in [("transitions", TRANSITIONS), ("effects", EFFECTS)] {
        let d = root.join(dir);
        std::fs::create_dir_all(&d)?;
        for (id, src) in items {
            let p = d.join(format!("{id}.glsl"));
            if !p.exists() {
                std::fs::write(&p, src)?;
                n += 1;
            }
        }
    }
    std::fs::write(&marker, b"")?;
    Ok(n)
}

/// `install_into` the user's library folder, once per run, off the UI thread.
pub fn ensure_installed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(|| {
            let _ = install_into(&crate::engine::library::root());
        });
    });
}

/// White rounded lettering with a hard drop shadow: the pack's text look.
pub fn text_style(text: &str, size: f64) -> TextStyle {
    TextStyle {
        text: text.into(),
        font: FONT.into(),
        size: Animated::new(size),
        bold: true,
        color: [255, 255, 255, 255],
        shadow: true,
        shadow_color: [0, 0, 0, 200],
        shadow_x: 4.0,
        shadow_y: 4.0,
        shadow_blur: 1.5,
        line_spacing: 1.15,
        ..Default::default()
    }
}

/// A full-frame royal-blue card `w` x `h` (the frame's size in project pixels).
pub fn blue_card(dur: f64, w: f64, h: f64) -> Clip {
    let mut c = Clip::new(0, ClipKind::Shape, "Blue Background", 0.0, dur);
    if let Some(s) = c.shape.as_mut() {
        s.fill = BLUE;
        s.w = Animated::new(w);
        s.h = Animated::new(h);
    }
    c.exposed = vec!["shape.fill".into()];
    c
}

/// A text clip in the pack's look.
pub fn text_clip(name: &str, text: &str, size: f64, dur: f64) -> Clip {
    let mut c = Clip::new(0, ClipKind::Text, name, 0.0, dur);
    c.text = Some(text_style(text, size));
    c.exposed = vec!["text.text".into(), "text.color".into(), "text.size".into()];
    c
}

fn block(c: &mut Clip, name: &str, t: f64, dur: f64, w: f64, h: f64) {
    if let Some(b) = find_block(&[], name) {
        c.place_block(&b, t, Some(dur), w, h);
    }
}

/// Keys straight on a property (linear unless said).
fn keys(a: &mut Animated, k: &[(f64, f64, Ease)]) {
    a.keys = k.iter().map(|&(t, v, ease)| Keyframe { t, v, ease }).collect();
    if let Some(f) = k.first() {
        a.value = f.1;
    }
}

/// The default credits for "Credits Roll".
pub const CREDITS: &str = "Directed by\nYour Name\n\nStarring\nEveryone Here\n\nProduced by\nYour Name\n\nMusic\nYour Song\n\nThanks for watching!";

/// A credits roll: `text` scrolls from below the frame to above it over `dur`.
pub fn credits_roll(text: &str, dur: f64, h: f64) -> Clip {
    let mut c = text_clip("Credits", text, 60.0, dur);
    let lines = text.lines().count().max(1) as f64;
    let half = lines * 60.0 * 1.15 * 0.5;
    keys(&mut c.y, &[(0.0, (h * 0.5 + half), Ease::Linear), (dur, -(h * 0.5 + half), Ease::Linear)]);
    c
}

/// Every title of the pack, in Gallery order.
pub const TITLES: [&str; 11] = [
    "Classic Blue Title",
    "Classic Basic Title",
    "Classic Fade Title",
    "Classic Fly-In Title",
    "Classic Typewriter",
    "Classic News Banner",
    "Classic Zoom Title",
    "Classic Spin Title",
    "Classic Moving Title",
    "Classic Credits Roll",
    "Classic The End",
];

/// A title's clips (bottom layer first) on a `w` x `h` frame, or None for another name.
pub fn title_clips(name: &str, w: f64, h: f64) -> Option<Vec<Clip>> {
    let card = |text: &str, dur: f64| {
        let mut t = text_clip("Title", text, 110.0, dur);
        block(&mut t, "Fade In", 0.0, 0.6, w, h);
        block(&mut t, "Fade Out", dur - 0.6, 0.6, w, h);
        vec![blue_card(dur, w, h), t]
    };
    Some(match name {
        "Classic Blue Title" => card("My Movie", 4.0),
        "Classic The End" => card("The End", 4.0),
        "Classic Basic Title" => vec![text_clip("Title", "Your Title Here", 96.0, 4.0)],
        "Classic Fade Title" => {
            let mut t = text_clip("Title", "Your Title Here", 96.0, 4.0);
            block(&mut t, "Fade In", 0.0, 1.0, w, h);
            block(&mut t, "Fade Out", 3.0, 1.0, w, h);
            vec![t]
        }
        "Classic Fly-In Title" => {
            let mut t = text_clip("Title", "Your Title Here", 96.0, 4.0);
            block(&mut t, "Slide In Left", 0.0, 0.8, w, h);
            block(&mut t, "Slide Out Right", 3.2, 0.8, w, h);
            vec![t]
        }
        "Classic Typewriter" => {
            let mut t = text_clip("Title", "Once upon a time...", 80.0, 5.0);
            if let Some(s) = t.text.as_mut() {
                keys(&mut s.reveal, &[(0.0, 0.0, Ease::Linear), (2.0, 1.0, Ease::Linear)]);
            }
            vec![t]
        }
        "Classic News Banner" => {
            let mut bar = blue_card(5.0, w, 120.0);
            bar.name = "Banner".into();
            bar.y = Animated::new(h * 0.5 - 170.0);
            let mut stripe = Clip::new(0, ClipKind::Shape, "Stripe", 0.0, 5.0);
            if let Some(s) = stripe.shape.as_mut() {
                s.fill = [255, 205, 40, 255];
                s.w = Animated::new(w);
                s.h = Animated::new(10.0);
            }
            stripe.y = Animated::new(h * 0.5 - 235.0);
            let mut t = text_clip("Headline", "Breaking news from the family", 56.0, 5.0);
            if let Some(s) = t.text.as_mut() {
                s.align = 0;
            }
            t.x = Animated::new(-(w * 0.5 - 80.0));
            t.y = bar.y.clone();
            for c in [&mut bar, &mut stripe, &mut t] {
                block(c, "Slide In Left", 0.0, 0.6, w, h);
            }
            vec![bar, stripe, t]
        }
        "Classic Zoom Title" => {
            let mut t = text_clip("Title", "Your Title Here", 96.0, 4.0);
            keys(&mut t.scale, &[(0.0, 0.05, Ease::EaseOut), (1.0, 1.0, Ease::Linear)]);
            block(&mut t, "Fade Out", 3.4, 0.6, w, h);
            vec![t]
        }
        "Classic Spin Title" => {
            let mut t = text_clip("Title", "Your Title Here", 96.0, 4.0);
            keys(&mut t.rotation, &[(0.0, -720.0, Ease::EaseOut), (1.2, 0.0, Ease::Linear)]);
            keys(&mut t.scale, &[(0.0, 0.1, Ease::EaseOut), (1.2, 1.0, Ease::Linear)]);
            vec![t]
        }
        "Classic Moving Title" => {
            let mut t = text_clip("Title", "Your Title Here", 90.0, 6.0);
            keys(&mut t.x, &[(0.0, -(w * 0.35), Ease::Linear), (6.0, (w * 0.35), Ease::Linear)]);
            keys(&mut t.y, &[(0.0, (h * 0.15), Ease::EaseInOut), (3.0, -(h * 0.15), Ease::EaseInOut), (6.0, (h * 0.15), Ease::Linear)]);
            vec![t]
        }
        "Classic Credits Roll" => vec![blue_card(12.0, w, h), credits_roll(CREDITS, 12.0, h)],
        _ => return None,
    })
}

/// The pack's titles as Gallery templates (the Titles tab, `titles.place`).
pub fn titles() -> Vec<Template> {
    TITLES
        .iter()
        .filter_map(|&n| {
            let clips = title_clips(n, W, H)?;
            let layers = (0..clips.len() as u32).collect();
            // the Gallery picture's moment: the text fully in (a card's first frame is plain blue)
            let thumb_t = match n {
                "Classic Credits Roll" => 3.2,
                "Classic Typewriter" | "Classic Moving Title" => 2.5,
                _ => 1.5,
            };
            let data = crate::engine::templates::TemplateData { clips, layers, thumb_t, ..Default::default() };
            Some(Template { name: n.into(), json: serde_json::to_string(&data).ok()? })
        })
        .collect()
}

/// The Titles tab card that runs Make Slideshow on the selection instead of placing a title.
pub const SLIDESHOW_CARD: &str = "Make Slideshow";

/// The pack's "speed up / slow down" presets (Gallery ▸ Speed), as constant speed curves.
pub fn speed_presets() -> Vec<crate::settings::CurvePreset> {
    let flat = |name: &str, v: f64| crate::settings::CurvePreset {
        name: name.into(),
        keys: vec![Keyframe { t: 0.0, v, ease: Ease::Linear }, Keyframe { t: 1.0, v, ease: Ease::Linear }],
        absolute: false,
    };
    vec![flat("Speed Up, Double", 2.0), flat("Speed Up, Quadruple", 4.0), flat("Slow Down, Half", 0.5), flat("Slow Down, Quarter", 0.25)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_pack_shaders_lint_clean_and_carry_the_tag() {
        for (kind, items) in [(true, TRANSITIONS), (false, EFFECTS)] {
            for (id, src) in items {
                let bad = crate::engine::shaderlib::lint(src, kind);
                assert!(bad.is_empty(), "{id}: {bad:?}");
                let m = crate::engine::shaderlib::parse_meta(src);
                assert!(is_classic(&m.tags), "{id} is not tagged `{TAG}`: {:?}", m.tags);
            }
        }
        assert!(TRANSITIONS.len() >= 26 && EFFECTS.len() >= 10);
    }

    #[test]
    fn every_library_file_tagged_classic_is_in_the_pack() {
        let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library");
        for (dir, items) in [("transitions", TRANSITIONS), ("effects", EFFECTS)] {
            for e in std::fs::read_dir(lib.join(dir)).unwrap().flatten() {
                let p = e.path();
                let src = std::fs::read_to_string(&p).unwrap_or_default();
                let id = p.file_stem().unwrap().to_string_lossy().into_owned();
                if is_classic(&crate::engine::shaderlib::parse_meta(&src).tags) {
                    assert!(items.iter().any(|(i, _)| *i == id), "{dir}/{id} is tagged `{TAG}` but missing from the pack");
                }
            }
        }
    }

    #[test]
    fn classic_titles_parse_and_use_the_pack_font() {
        let ts = titles();
        assert_eq!(ts.len(), TITLES.len(), "every title encodes");
        for t in &ts {
            let (clips, _) = crate::engine::presets::decode_template(t).expect("decodes");
            assert!(!clips.is_empty(), "{}", t.name);
            for c in clips.iter().filter(|c| c.kind == ClipKind::Text) {
                assert_eq!(c.text.as_ref().unwrap().font, FONT, "{}", t.name);
                assert!(!c.exposed.is_empty(), "{}: something to customize", t.name);
            }
        }
        // the signature card: royal blue under white text
        let (clips, _) = crate::engine::presets::decode_template(&ts[0]).unwrap();
        assert_eq!(clips[0].shape.as_ref().unwrap().fill, BLUE);
        assert_eq!(clips[1].text.as_ref().unwrap().color, [255, 255, 255, 255]);
        assert!(clips[1].text.as_ref().unwrap().shadow, "a drop shadow");
    }

    #[test]
    fn installed_pack_items_are_named_and_tagged_without_a_catalogue() {
        let installed: std::collections::HashSet<String> =
            ["transitions/classic-heart".to_string(), "effects/classic-sepia".to_string(), "effects/my-own".to_string()].into();
        let rows = crate::engine::library::merge(&[], &installed);
        let heart = rows.iter().find(|r| r.id == "transitions/classic-heart").unwrap();
        assert_eq!(heart.name, "Heart");
        assert!(!heart.yours && heart.installed && is_classic(&heart.tags));
        assert!(rows.iter().find(|r| r.id == "effects/my-own").unwrap().yours, "a user's own file stays theirs");
        assert_eq!(source_of("effects/classic-sepia"), Some(EFFECTS.iter().find(|e| e.0 == "classic-sepia").unwrap().1));
        assert_eq!(source_of("luts/classic-sepia"), None);
    }

    #[test]
    fn credits_roll_scrolls_from_below_to_above() {
        let c = credits_roll(CREDITS, 10.0, 1080.0);
        assert!(c.y.at(0.0) > 540.0 && c.y.at(10.0) < -540.0, "{} .. {}", c.y.at(0.0), c.y.at(10.0));
        assert!(c.text.as_ref().unwrap().text.starts_with("Directed by"));
    }

    #[test]
    fn install_copies_once_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!("se-retro-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("transitions")).unwrap();
        std::fs::write(root.join("transitions/blinds.glsl"), "mine").unwrap();
        let n = install_into(&root).unwrap();
        assert_eq!(n, TRANSITIONS.len() + EFFECTS.len() - 1, "all but the user's own file");
        assert_eq!(std::fs::read_to_string(root.join("transitions/blinds.glsl")).unwrap(), "mine");
        // removed by the user afterwards: not put back
        std::fs::remove_file(root.join("effects/classic-sepia.glsl")).unwrap();
        assert_eq!(install_into(&root).unwrap(), 0);
        assert!(!root.join("effects/classic-sepia.glsl").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
