use crate::model::*;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Text clip styling. Sizes are in project pixels (at project resolution).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TextStyle {
    pub text: String,
    pub font: String,
    /// Em size in project px. Promoted from a plain `f32` (ws:text-titles, wave 3) so it can be
    /// keyframed like `clip.x`/`clip.scale`; `de_scalar_or_animated` accepts both the old bare-number
    /// JSON (`"size": 72.0`) and a full `Animated` object, so every project saved before this change
    /// still loads with an identical (non-animated) value.
    #[serde(default = "text_size_default", deserialize_with = "de_scalar_or_animated")]
    pub size: Animated,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 4],
    /// Outline stroke width in project px. Same promotion/back-compat story as `size`.
    #[serde(default = "crate::model::a0", deserialize_with = "de_scalar_or_animated")]
    pub outline_width: Animated,
    pub outline_color: [u8; 4],
    pub shadow: bool,
    pub shadow_color: [u8; 4],
    pub shadow_x: f32,
    pub shadow_y: f32,
    pub shadow_blur: f32,
    /// 0 = left, 1 = center, 2 = right
    pub align: u8,
    pub line_spacing: f32,
    /// Extra advance per glyph in project px. Same promotion/back-compat story as `size`.
    #[serde(default = "crate::model::a0", deserialize_with = "de_scalar_or_animated")]
    pub letter_spacing: Animated,
    /// Background box behind the text; alpha 0 = none.
    pub box_color: [u8; 4],
    pub box_padding: f32,
    /// Styled sub-ranges of `text` (char-indexed, `[start, end)`) that override some of the run-level
    /// fields above for just that range; every field left `None` keeps inheriting this clip's style.
    /// Empty for every project saved before spans existed (`#[serde(default)]` on the struct covers it).
    pub spans: Vec<TextSpan>,
    // ---- ws:registries-schema-hooks ----
    /// Reveal-in progress (0 = hidden, 1 = fully revealed) — `ws:text-titles` (wave 3) is the sole
    /// consumer, via a t-aware `TextRasterizer`. Defaults to fully revealed (`a1`, NOT `a0`) so a clip
    /// that never touches this field — every pre-overhaul project, and any brand-new text clip — renders
    /// its whole string exactly as before; only an explicit reveal < 1 (or a keyframed ramp) hides
    /// anything. `#[serde(default)]` gives the same fully-revealed value to JSON with no `reveal` key.
    #[serde(default = "crate::model::a1")]
    pub reveal: Animated,
    /// Per-character wave/wobble amount (0 = none, project px), same consumer as `reveal`.
    #[serde(default = "crate::model::a0")]
    pub wave: Animated,
}

/// Default for `TextStyle.size` when the JSON key is entirely absent (defensive — every project ever
/// saved always wrote `size`, so the realistic back-compat path is `de_scalar_or_animated`'s bare-number
/// branch below, not this).
fn text_size_default() -> Animated {
    Animated::new(72.0)
}

/// Accepts either a bare JSON number (every project saved before this promotion: `"size": 72.0`) or a
/// full `Animated` object, so `TextStyle.size`/`letter_spacing`/`outline_width` can be promoted to
/// keyframeable properties without corrupting a single existing `.sedit` file.
fn de_scalar_or_animated<'de, D>(deserializer: D) -> Result<Animated, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ScalarOrAnimated {
        Scalar(f64),
        Full(Animated),
    }
    Ok(match ScalarOrAnimated::deserialize(deserializer)? {
        ScalarOrAnimated::Scalar(v) => Animated::new(v),
        ScalarOrAnimated::Full(a) => a,
    })
}

/// Hash an `Animated` property into a `cache_key()` (value + every keyframe, sign included), mirroring
/// `ShapeStyle::cache_key()`'s `w`/`h` handling verbatim (model/shape.rs) since `Ease` isn't `Hash`
/// (`Bezier` carries `f32` handles).
fn hash_animated(h: &mut impl Hasher, a: &Animated) {
    a.value.to_bits().hash(h);
    a.keys.len().hash(h);
    for k in &a.keys {
        (k.t.to_bits(), k.v.to_bits()).hash(h);
        match k.ease {
            Ease::Linear => 0u8.hash(h),
            Ease::EaseIn => 1u8.hash(h),
            Ease::EaseOut => 2u8.hash(h),
            Ease::EaseInOut => 3u8.hash(h),
            Ease::Hold => 4u8.hash(h),
            Ease::Bezier { x1, y1, x2, y2 } => {
                5u8.hash(h);
                for f in [x1, y1, x2, y2] {
                    f.to_bits().hash(h);
                }
            }
        }
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            text: "Text".into(),
            font: "Segoe UI".into(),
            size: text_size_default(),
            bold: false,
            italic: false,
            color: [255, 255, 255, 255],
            outline_width: a0(),
            outline_color: [0, 0, 0, 255],
            shadow: false,
            shadow_color: [0, 0, 0, 160],
            shadow_x: 4.0,
            shadow_y: 4.0,
            shadow_blur: 2.0,
            align: 1,
            line_spacing: 1.0,
            letter_spacing: a0(),
            box_color: [0, 0, 0, 0],
            box_padding: 8.0,
            spans: Vec::new(),
            reveal: a1(),
            wave: a0(),
        }
    }
}

/// A styled sub-range of `TextStyle::text`, addressed by CHAR index (not byte — `text` may hold
/// multi-byte UTF-8), half-open `[start, end)`. Only run-level fields are overridable here; `align`,
/// `line_spacing`, `box_color` and `box_padding` are paragraph-level and stay clip-wide.
///
/// Rendering (`engine::text`) honors `color`, `font`/`size`/`bold`/`italic` (glyph shape) and
/// `letter_spacing` per span; `outline_width`, `outline_color`, `shadow`, `shadow_color`, `shadow_x`,
/// `shadow_y`, `shadow_blur` are accepted here for a future pass but currently always draw with the
/// clip's base style — see the `ponytail:` comment in `engine::text::TextRasterizer::rasterize`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub font: Option<String>,
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub color: Option<[u8; 4]>,
    pub outline_width: Option<f32>,
    pub outline_color: Option<[u8; 4]>,
    pub shadow: Option<bool>,
    pub shadow_color: Option<[u8; 4]>,
    pub shadow_x: Option<f32>,
    pub shadow_y: Option<f32>,
    pub shadow_blur: Option<f32>,
    pub letter_spacing: Option<f32>,
}

impl TextStyle {
    /// Default look for burnt-in subtitles.
    pub fn subtitle_default() -> Self {
        Self { text: String::new(), size: Animated::new(48.0), outline_width: Animated::new(3.0), ..Self::default() }
    }
    /// Stable hash of every field (for render caches).
    pub fn cache_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        self.font.hash(&mut h);
        for f in [self.shadow_x, self.shadow_y, self.shadow_blur, self.line_spacing, self.box_padding] {
            f.to_bits().hash(&mut h);
        }
        for a in [&self.size, &self.outline_width, &self.letter_spacing, &self.reveal, &self.wave] {
            hash_animated(&mut h, a);
        }
        (self.bold, self.italic, self.shadow, self.align).hash(&mut h);
        (self.color, self.outline_color, self.shadow_color, self.box_color).hash(&mut h);
        self.spans.len().hash(&mut h);
        for s in &self.spans {
            (s.start, s.end).hash(&mut h);
            (&s.font, s.bold, s.italic, s.shadow).hash(&mut h);
            (s.color, s.outline_color, s.shadow_color).hash(&mut h);
            for f in [s.size, s.outline_width, s.shadow_x, s.shadow_y, s.shadow_blur, s.letter_spacing] {
                f.map(f32::to_bits).hash(&mut h);
            }
        }
        h.finish()
    }

    /// Drop spans that can never cover a character of `text` and clamp the rest to its char count —
    /// called after any operation that installs spans wholesale (e.g. pasting a style whose ranges
    /// index a different string). The rasterizer already ignores out-of-range spans defensively; this
    /// keeps them out of the saved file so they can't silently resurrect on a later text edit.
    pub fn clamp_spans(&mut self) {
        let n = self.text.chars().count();
        for s in &mut self.spans {
            s.end = s.end.min(n);
        }
        self.spans.retain(|s| s.start < s.end);
    }

    /// Keep span ranges attached to the characters they styled across ONE text edit (`old` → the
    /// current `self.text`). The edit is located by common char prefix/suffix; boundaries after it
    /// shift by the length delta, and a boundary inside the replaced region clamps to the edit's
    /// edge (so a replaced styled word stays styled, and a span fully inside the edit disappears).
    /// One contiguous edit at a time is all an egui `TextEdit` produces per frame.
    pub fn remap_spans(&mut self, old: &str) {
        if self.spans.is_empty() || old == self.text {
            return;
        }
        let o: Vec<char> = old.chars().collect();
        let n: Vec<char> = self.text.chars().collect();
        let p = o.iter().zip(n.iter()).take_while(|(a, b)| a == b).count();
        let s = o[p..].iter().rev().zip(n[p..].iter().rev()).take_while(|(a, b)| a == b).count();
        let (old_end, new_end) = (o.len() - s, n.len() - s); // o[p..old_end] was replaced by n[p..new_end]
        let delta = new_end as isize - old_end as isize;
        let map = |i: usize, is_end: bool| -> usize {
            if i <= p {
                i
            } else if i >= old_end {
                (i as isize + delta) as usize
            } else if is_end {
                p // the styled tail was replaced — keep the surviving head
            } else {
                new_end // the styled head was replaced — keep the surviving tail
            }
        };
        for sp in &mut self.spans {
            (sp.start, sp.end) = (map(sp.start, false), map(sp.end, true));
        }
        self.clamp_spans();
    }

    /// Remove per-char style overrides covering `[a, b)` (char indices): spans fully inside are
    /// dropped, ones straddling an edge are trimmed, and a span strictly containing the range is
    /// split in two. The inspector's "Clear Style on Selection".
    pub fn clear_span_range(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        let mut split_tails = Vec::new();
        for s in &mut self.spans {
            if s.start < a && s.end > b {
                split_tails.push(TextSpan { start: b, end: s.end, ..s.clone() });
                s.end = a;
            } else if s.start < a {
                s.end = s.end.min(a);
            } else {
                s.start = s.start.max(b);
            }
        }
        self.spans.extend(split_tails);
        self.spans.retain(|s| s.start < s.end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ws:text-titles back-compat: a `TextStyle` JSON exactly as every project saved before this wave
    /// wrote it — `size`/`letter_spacing`/`outline_width` as bare numbers, no `reveal`/`wave` keys at
    /// all — must deserialize to a non-animated `Animated` holding that same value, and `reveal` must
    /// default to FULLY REVEALED (1.0, not 0.0) so the text renders exactly as it always did. This is
    /// the manual "open an old project" check from the plan's verification checklist, pinned as a test.
    #[test]
    fn textstyle_bare_number_json_is_back_compat_and_fully_revealed() {
        let json = r#"{"text":"Hi","size":40.0,"letter_spacing":2.0,"outline_width":3.0}"#;
        let t: TextStyle = serde_json::from_str(json).unwrap();
        assert_eq!(t.size, Animated::new(40.0));
        assert!(!t.size.is_animated());
        assert_eq!(t.letter_spacing, Animated::new(2.0));
        assert_eq!(t.outline_width, Animated::new(3.0));
        // no reveal/wave keys in the JSON at all: reveal must default to fully-revealed, not hidden —
        // this is the whole back-compat guarantee once TextRasterizer wires reveal into rendering.
        assert_eq!(t.reveal.value, 1.0);
        assert!(!t.reveal.is_animated());
        assert_eq!(t.wave.value, 0.0);
        assert!(!t.wave.is_animated());
    }

    /// A `size` written as a full `Animated` object (keyframed) round-trips through
    /// `de_scalar_or_animated` unchanged — the promotion accepts both JSON shapes, not just bare numbers.
    #[test]
    fn textstyle_animated_object_json_round_trips() {
        let json = r#"{"text":"Hi","size":{"value":10.0,"keys":[{"t":0.0,"v":10.0,"ease":"Linear"},{"t":1.0,"v":80.0,"ease":"Linear"}]}}"#;
        let t: TextStyle = serde_json::from_str(json).unwrap();
        assert!(t.size.is_animated());
        assert_eq!(t.size.at(0.0), 10.0);
        assert_eq!(t.size.at(1.0), 80.0);
    }

    /// `TextStyle::default()` (every brand-new text clip) is also fully revealed and has no wave —
    /// the Animation-preset/Reveal/Wave UI is opt-in, not a surprise on a freshly typed text clip.
    #[test]
    fn textstyle_default_is_fully_revealed_no_wave() {
        let t = TextStyle::default();
        assert_eq!(t.reveal.value, 1.0);
        assert_eq!(t.wave.value, 0.0);
        assert_eq!(t.size.value, 72.0);
    }

    /// `cache_key()` changes when `size`/`letter_spacing`/`outline_width`/`reveal`/`wave` change — the
    /// render cache must not serve a stale frame after any of the newly-Animated fields is edited.
    #[test]
    fn cache_key_changes_with_every_promoted_field() {
        let base = TextStyle::default();
        let mut a = base.clone();
        a.size.value = 100.0;
        assert_ne!(base.cache_key(), a.cache_key(), "size");
        let mut b = base.clone();
        b.letter_spacing.value = 5.0;
        assert_ne!(base.cache_key(), b.cache_key(), "letter_spacing");
        let mut c = base.clone();
        c.outline_width.value = 5.0;
        assert_ne!(base.cache_key(), c.cache_key(), "outline_width");
        let mut d = base.clone();
        d.reveal.value = 0.5;
        assert_ne!(base.cache_key(), d.cache_key(), "reveal");
        let mut e = base.clone();
        e.wave.value = 5.0;
        assert_ne!(base.cache_key(), e.cache_key(), "wave");
        let mut f = base.clone();
        f.size.keys.push(Keyframe { t: 1.0, v: 200.0, ease: Ease::Linear });
        assert_ne!(base.cache_key(), f.cache_key(), "adding a keyframe");
    }
}
