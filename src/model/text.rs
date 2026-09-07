use crate::model::*;
use serde::{Deserialize, Serialize};

/// Text clip styling. Sizes are in project pixels (at project resolution).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TextStyle {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 4],
    pub outline_width: f32,
    pub outline_color: [u8; 4],
    pub shadow: bool,
    pub shadow_color: [u8; 4],
    pub shadow_x: f32,
    pub shadow_y: f32,
    pub shadow_blur: f32,
    /// 0 = left, 1 = center, 2 = right
    pub align: u8,
    pub line_spacing: f32,
    pub letter_spacing: f32,
    /// Background box behind the text; alpha 0 = none.
    pub box_color: [u8; 4],
    pub box_padding: f32,
    /// Styled sub-ranges of `text` (char-indexed, `[start, end)`) that override some of the run-level
    /// fields above for just that range; every field left `None` keeps inheriting this clip's style.
    /// Empty for every project saved before spans existed (`#[serde(default)]` on the struct covers it).
    pub spans: Vec<TextSpan>,
    // ---- ws:registries-schema-hooks ----
    /// Reveal-in progress (0 = hidden, 1 = fully revealed), driven by a t-aware `TextRasterizer` once
    /// ws:text-titles (wave 3) wires it. `#[serde(default)]` no-ops it to a constant 0 for every clip
    /// saved before this field existed. Does NOT replace `size`/`letter_spacing`/`outline_width` —
    /// those stay plain `f32` this wave (see the workstream's review trail).
    #[serde(default = "crate::model::a0")]
    pub reveal: Animated,
    /// Per-character wave/wobble amount (0 = none), same consumer as `reveal`.
    #[serde(default = "crate::model::a0")]
    pub wave: Animated,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            text: "Text".into(),
            font: "Segoe UI".into(),
            size: 72.0,
            bold: false,
            italic: false,
            color: [255, 255, 255, 255],
            outline_width: 0.0,
            outline_color: [0, 0, 0, 255],
            shadow: false,
            shadow_color: [0, 0, 0, 160],
            shadow_x: 4.0,
            shadow_y: 4.0,
            shadow_blur: 2.0,
            align: 1,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            box_color: [0, 0, 0, 0],
            box_padding: 8.0,
            spans: Vec::new(),
            reveal: a0(),
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
        Self { text: String::new(), size: 48.0, outline_width: 3.0, ..Self::default() }
    }
    /// Stable hash of every field (for render caches).
    pub fn cache_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        self.font.hash(&mut h);
        for f in [
            self.size,
            self.outline_width,
            self.shadow_x,
            self.shadow_y,
            self.shadow_blur,
            self.line_spacing,
            self.letter_spacing,
            self.box_padding,
        ] {
            f.to_bits().hash(&mut h);
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

    /// A `TextStyle` JSON with no `reveal`/`wave` fields (every clip saved before this workstream)
    /// deserializes both as a no-op constant `0`, and the existing scalar fields are untouched.
    #[test]
    fn textstyle_reveal_wave_default_and_scalars_unchanged() {
        let json = r#"{"text":"Hi","size":40.0,"letter_spacing":2.0,"outline_width":3.0}"#;
        let t: TextStyle = serde_json::from_str(json).unwrap();
        assert_eq!(t.reveal.value, 0.0);
        assert!(!t.reveal.is_animated());
        assert_eq!(t.wave.value, 0.0);
        assert!(!t.wave.is_animated());
        // still plain f32 scalars, unmigrated this wave
        assert_eq!(t.size, 40.0);
        assert_eq!(t.letter_spacing, 2.0);
        assert_eq!(t.outline_width, 3.0);
    }
}
