//! ---- ws:html-layer ----
//! Pre-flight for the fast HTML renderer (`engine::blitz`, the `se-html` helper): scan a layer's HTML / CSS
//! for what that renderer cannot draw. A hit in `blocks` makes `Auto` draw the layer with Edge (and the
//! Inspector says why); `notes` are differences small enough to keep the fast path (the layer is drawn,
//! without the effect). Textual on purpose: no CSS parser, no dependency, same cost as `html::lint`.
//! What each rule means (and the upstream issue behind it) is in `BLITZ_REPORT.md` and
//! `website/docs/html-layers.mdx`.

/// What the scan found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Findings {
    /// Cannot be drawn: `Auto` uses Edge. Each is a short phrase that completes "uses ...".
    pub blocks: Vec<String>,
    /// Drawn, but a little different.
    pub notes: Vec<String>,
}

/// The values of every declaration of property `name` in `text` (already lower-case): from after the
/// colon to the next `;`, `}`, `"` or `<`. A longer property that merely ends in `name`
/// (`backdrop-filter` for `filter`) is not a match.
fn values<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before = if at == 0 { b' ' } else { b[at - 1] };
        if before.is_ascii_alphanumeric() || before == b'-' || before == b'_' {
            continue;
        }
        let rest = text[from..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else { continue };
        let end = rest.find([';', '}', '"', '<']).unwrap_or(rest.len());
        out.push(rest[..end].trim());
    }
    out
}

fn has_any(v: &str, words: &[&str]) -> bool {
    words.iter().any(|w| v.contains(w))
}

/// Everything between `<style ...>` and `</style>` (lower-case input).
fn style_blocks(text: &str) -> String {
    let (mut out, mut rest) = (String::new(), text);
    while let Some(i) = rest.find("<style") {
        let Some(j) = rest[i..].find('>') else { break };
        let body = &rest[i + j + 1..];
        let end = body.find("</style").unwrap_or(body.len());
        out.push_str(&body[..end]);
        out.push('\n');
        rest = &body[end..];
    }
    out
}

/// The text without its `/* comments */` (a comment may mention what a rule looks for).
fn without_comments(t: &str) -> String {
    let (mut out, mut rest) = (String::with_capacity(t.len()), t);
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The inline `<svg>...</svg>` blocks of the page (lower-case input), concatenated.
fn svg_blocks(t: &str) -> String {
    let (mut out, mut rest) = (String::new(), t);
    while let Some(i) = rest.find("<svg") {
        let body = &rest[i..];
        let end = body.find("</svg").map_or(body.len(), |j| j + 5);
        out.push_str(&body[..end]);
        rest = &body[end..];
    }
    out
}

/// Does a stylesheet rule that sets `fill` / `stroke` reach something INSIDE an inline svg? The helper's SVG
/// engine never sees the page's CSS, but it does take what is set on the `<svg>` element itself (`.icon svg
/// { fill: #fff }`), so only a rule whose subject is an svg part (`path`, `circle`, ... or a class / id that
/// appears inside an svg) counts.
fn styles_svg_parts(css: &str, svgs: &str) -> bool {
    const PARTS: [&str; 12] = ["path", "circle", "rect", "g", "line", "polygon", "polyline", "ellipse", "use", "text", "stop", "tspan"];
    for rule in css.split('}') {
        let Some((sel, decls)) = rule.split_once('{') else { continue };
        if values(decls, "fill").is_empty() && values(decls, "stroke").is_empty() && values(decls, "stroke-dasharray").is_empty() {
            continue;
        }
        for one in sel.split(',') {
            // the subject: the last compound selector
            let subject = one.trim().rsplit(|c: char| c.is_whitespace() || c == '>' || c == '+' || c == '~').next().unwrap_or("");
            let tag: String = subject.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if PARTS.contains(&tag.as_str()) {
                return true;
            }
            for tok in subject.split(['.', '#']).skip(1) {
                let name: String = tok.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                if !name.is_empty() && svgs.contains(&name) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn scan(src: &str) -> Findings {
    let t = without_comments(&src.to_ascii_lowercase());
    let mut f = Findings::default();
    let mut block = |s: &str| {
        if !f.blocks.iter().any(|b| b == s) {
            f.blocks.push(s.to_string());
        }
    };
    const COLOUR: [&str; 8] = ["hue-rotate", "saturate", "brightness", "contrast", "grayscale", "sepia", "invert", "opacity("];
    if values(&t, "filter").iter().any(|v| has_any(v, &COLOUR) || v.contains("url(")) {
        block("colour filters (hue-rotate, saturate, brightness, contrast, grayscale, sepia, invert)");
    }
    if !values(&t, "backdrop-filter").is_empty() || !values(&t, "-webkit-backdrop-filter").is_empty() {
        block("a backdrop filter");
    }
    if values(&t, "mix-blend-mode").iter().chain(values(&t, "background-blend-mode").iter()).any(|v| !v.starts_with("normal")) {
        block("a blend mode (mix-blend-mode)");
    }
    if !values(&t, "-webkit-text-stroke").is_empty() || !values(&t, "text-stroke").is_empty() || !values(&t, "paint-order").is_empty() {
        block("outlined text (-webkit-text-stroke)");
    }
    if values(&t, "position").iter().any(|v| v.starts_with("sticky")) {
        block("position: sticky");
    }
    if !values(&t, "perspective").is_empty()
        || values(&t, "transform-style").iter().any(|v| v.contains("preserve-3d"))
        || values(&t, "transform").iter().any(|v| has_any(v, &["rotatex", "rotatey", "rotate3d", "matrix3d", "perspective("]))
    {
        block("3D transforms");
    }
    if values(&t, "content").iter().any(|v| v.contains("counter(") || v.contains("counters(")) {
        block("CSS counters (content: counter())");
    }
    if t.contains("@property") {
        block("registered custom properties (@property)");
    }
    if values(&t, "writing-mode").iter().any(|v| v.contains("vertical")) {
        block("vertical writing (writing-mode)");
    }
    // an inline <svg> whose parts the page's own CSS paints: the helper's SVG engine never sees the stylesheet
    if t.contains("<svg") && styles_svg_parts(&style_blocks(&t), &svg_blocks(&t)) {
        block("an inline SVG styled by the page's CSS (fill / stroke of its parts in a stylesheet)");
    }
    // drawn, but different
    if values(&t, "text-shadow").iter().any(|v| !v.starts_with("none") && !v.is_empty()) {
        f.notes.push("text-shadow is drawn without the shadow".into());
    }
    // an ellipsis / a line clamp only show when the text is longer than its box: the text is cut instead
    if values(&t, "text-overflow").iter().any(|v| v.contains("ellipsis")) || !values(&t, "-webkit-line-clamp").is_empty() {
        f.notes.push("text that does not fit is cut without an ellipsis".into());
    }
    if values(&t, "text-wrap").iter().any(|v| v.starts_with("balance") || v.starts_with("pretty")) {
        f.notes.push("lines are broken greedily (text-wrap: balance is not applied)".into());
    }
    if values(&t, "text-align").iter().any(|v| v.starts_with("justify")) {
        f.notes.push("justified text is drawn left-aligned".into());
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(css: &str) -> bool {
        !scan(&format!("<style>{css}</style><p>x</p>")).blocks.is_empty()
    }

    #[test]
    fn the_known_gaps_choose_edge() {
        for css in [
            "p{filter:hue-rotate(90deg)}",
            "p{filter: blur(4px) saturate(1.4)}",
            "p{backdrop-filter:blur(8px)}",
            "p{-webkit-backdrop-filter: blur(3px)}",
            "p{mix-blend-mode:screen}",
            "p{-webkit-text-stroke:2px #000}",
            "p{position:sticky;top:0}",
            "p{transform:rotateY(40deg)}",
            ".s{perspective:300px}",
            "p::before{content:counter(n)}",
            "@property --n{syntax:'<integer>';inherits:false;initial-value:0}",
            "p{writing-mode:vertical-rl}",
        ] {
            assert!(blocked(css), "{css}");
        }
    }

    #[test]
    fn what_the_helper_draws_stays_on_the_fast_path() {
        for css in [
            "p{filter:blur(4px)}",
            "p{filter:drop-shadow(0 4px 8px #000)}",
            "p{backdrop:none;transform:rotate(20deg) scale(1.2) translate3d(0,0,0)}",
            "p{position:absolute;top:0;mix-blend-mode:normal}",
            "p{display:flex;gap:8px;animation:a 1s ease both}@keyframes a{from{opacity:0}to{opacity:1}}",
            "p::before{content:'x'}",
            ".backdrop-filter-ish{color:red}",
        ] {
            assert!(!blocked(css), "{css}");
        }
        let f = scan("<style>p{text-shadow:0 2px 8px #000}</style>");
        assert!(f.blocks.is_empty() && f.notes.len() == 1);
        let f = scan("<style>p{text-overflow:ellipsis;-webkit-line-clamp:2}</style>");
        assert!(f.blocks.is_empty() && f.notes.len() == 1, "{f:?}");
    }

    #[test]
    fn inline_svg_is_only_a_problem_when_the_page_css_styles_it() {
        assert!(scan("<svg width=4><circle fill=\"red\"/></svg>").blocks.is_empty());
        assert!(!scan("<style>.a{fill:#fff}</style><svg><circle class=a /></svg>").blocks.is_empty());
        assert!(!scan("<style>.box path{fill:none;stroke:#fff}</style><svg class=box><path d=\"M0 0\"/></svg>").blocks.is_empty());
        // set on the <svg> element itself it is understood
        assert!(scan("<style>.icon svg{width:40px;fill:#fff}</style><div class=icon><svg viewBox=\"0 0 4 4\"><path d=\"M0 0\"/></svg></div>").blocks.is_empty());
        // a comment that merely names a thing is not a use of it
        assert!(scan("<style>/* no @property, no counter() here */ p{color:red}</style>").blocks.is_empty());
        assert!(scan("<style>.a{fill:#fff}</style><p>no svg</p>").blocks.is_empty());
    }

    #[test]
    fn a_property_that_only_ends_in_the_name_is_not_a_match() {
        assert!(!blocked("p{--my-filter:hue-rotate(4deg)}"));
        assert!(!blocked("p{x-position:sticky}"));
    }
}
