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

// ---------------------------------------------------------------------------------------------------
// inline <svg> + the page's CSS: what the helper's SVG engine (usvg) never sees, written onto the parts

/// The paint properties a stylesheet may set on an svg part that usvg also takes from a `style` attribute.
const SVG_PAINT: [&str; 13] = [
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-opacity",
    "stop-color",
    "stop-opacity",
    "opacity",
];

/// One compound selector (`rect.a#b`), lower-case.
#[derive(Debug, Default)]
struct Compound {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
}

/// None = something this matcher does not know (attribute selectors, pseudo-classes, ...).
fn compound(s: &str) -> Option<Compound> {
    let ok = |ch: char| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
    let mut c = Compound::default();
    let tag: String = s.chars().take_while(|&ch| ok(ch)).collect();
    let mut rest = &s[tag.len()..];
    if !tag.is_empty() {
        c.tag = Some(tag);
    } else if let Some(r) = rest.strip_prefix('*') {
        rest = r;
    }
    while !rest.is_empty() {
        let (kind, r) = rest.split_at(1);
        let name: String = r.chars().take_while(|&ch| ok(ch)).collect();
        if name.is_empty() {
            return None;
        }
        match kind {
            "." => c.classes.push(name.clone()),
            "#" => c.id = Some(name.clone()),
            _ => return None,
        }
        rest = &r[name.len()..];
    }
    Some(c)
}

/// A selector: its compounds from the subject leftwards, each with "the one before must be its parent" (`>`).
fn selector(s: &str) -> Option<Vec<(Compound, bool)>> {
    let s = s.replace('>', " > ");
    let (mut out, mut child) = (Vec::new(), false);
    for tok in s.split_whitespace().rev() {
        if tok == ">" {
            child = true;
            continue;
        }
        if tok.contains(['+', '~', ':', '[', '(']) {
            return None;
        }
        out.push((compound(tok)?, child));
        child = false;
    }
    (!out.is_empty()).then_some(out)
}

#[derive(Clone, Debug)]
struct El {
    tag: String,
    classes: Vec<String>,
    id: String,
}

fn hits(c: &Compound, e: &El) -> bool {
    c.tag.as_ref().is_none_or(|t| *t == e.tag)
        && c.id.as_ref().is_none_or(|i| *i == e.id)
        && c.classes.iter().all(|k| e.classes.contains(k))
}

/// Does `sel` (subject first) match the last element of `stack` (its open ancestors before it)?
/// ponytail: greedy right-to-left without backtracking - exact for the usual `.a b` / `.a > b` chains.
fn matches(sel: &[(Compound, bool)], stack: &[El]) -> bool {
    let Some((last, up)) = stack.split_last() else { return false };
    if !hits(&sel[0].0, last) {
        return false;
    }
    let mut at = up.len();
    for i in 1..sel.len() {
        let parent_only = sel[i - 1].1;
        loop {
            if at == 0 {
                return false;
            }
            at -= 1;
            if hits(&sel[i].0, &up[at]) {
                break;
            }
            if parent_only {
                return false;
            }
        }
    }
    true
}

/// The value of attribute `name` (lower-case) in a start tag's text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        if at == 0 || !lower.as_bytes()[at - 1].is_ascii_whitespace() {
            continue;
        }
        let rest = lower[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let off = tag.len() - rest.trim_start().len();
        return Some(match tag[off..].chars().next() {
            Some(q @ ('"' | '\'')) => {
                let end = tag[off + 1..].find(q).map_or(tag.len(), |j| off + 1 + j);
                &tag[off + 1..end]
            }
            _ => {
                let end =
                    tag[off..].find(|c: char| c.is_whitespace() || c == '>' || c == '/').map_or(tag.len(), |j| off + j);
                &tag[off..end]
            }
        });
    }
    None
}

/// `var(--x)` / `var(--x, fallback)` in `v` replaced from `vars`. Err = a variable it does not know.
fn resolve_vars(v: &str, vars: &std::collections::HashMap<String, String>) -> Result<String, ()> {
    let mut v = v.to_string();
    for _ in 0..8 {
        let Some(i) = v.find("var(") else { return Ok(v) };
        let mut depth = 0;
        let mut end = None;
        for (j, ch) in v[i + 3..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 3 + j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.ok_or(())?;
        let inner = &v[i + 4..end];
        let (name, fallback) = match inner.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner.trim(), None),
        };
        let got = vars.get(&name.to_ascii_lowercase()).map(String::as_str).or(fallback).ok_or(())?.to_string();
        v = format!("{}{}{}", &v[..i], got, &v[end + 1..]);
    }
    Err(())
}

/// The top-level rules of a stylesheet: (selector, declarations). At-rule blocks (`@keyframes`, `@media`,
/// `@font-face`) are left out.
fn rules(css: &str) -> Vec<(&str, &str)> {
    let (mut out, mut depth, mut start, mut sel) = (Vec::new(), 0usize, 0usize, None::<usize>);
    for (i, ch) in css.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    sel = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(b) = sel.take() {
                        let s = css[start..b].trim();
                        if !s.starts_with('@') {
                            out.push((s, &css[b + 1..i]));
                        }
                    }
                    start = i + 1;
                }
            }
            ';' if depth == 0 => start = i + 1, // `@import ...;`
            _ => {}
        }
    }
    out
}

/// (property lower-case, value) of every declaration in a block.
fn decls(block: &str) -> impl Iterator<Item = (String, &str)> {
    block.split(';').filter_map(|d| {
        let (k, v) = d.split_once(':')?;
        Some((k.trim().to_ascii_lowercase(), v.trim().trim_end_matches("!important").trim()))
    })
}

/// `style_blocks` keeping the case (url(#Id) and font names are case-sensitive).
fn style_blocks_cased(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let (mut out, mut at) = (String::new(), 0);
    while let Some(i) = lower[at..].find("<style") {
        let i = at + i;
        let Some(j) = lower[i..].find('>') else { break };
        let body = i + j + 1;
        let end = lower[body..].find("</style").map_or(text.len(), |k| body + k);
        out.push_str(&text[body..end]);
        out.push('\n');
        at = end;
    }
    out
}

/// A rule that paints (or moves) svg parts.
struct SvgRule {
    /// Each selector of the rule with its specificity.
    sels: Vec<(Vec<(Compound, bool)>, u32)>,
    paint: Vec<(String, String)>,
    moves: bool,
}

/// `page` with what its stylesheets say about the parts of its inline `<svg>`s written onto those parts (a
/// `style` attribute: fill, stroke, ...), and `var(--x)` in their paint resolved from the page's custom
/// properties - so the helper's SVG engine, which never sees the stylesheet, draws them as a browser does.
/// Err(why) = a case this cannot resolve (a selector it does not understand, an unknown variable, an
/// animated svg part): `scan` blocks such a page, so it is drawn with Edge.
pub fn inline_svg_styles(page: &str) -> Result<String, String> {
    use std::collections::HashMap;
    let lower = page.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return Ok(page.to_string());
    }
    let css = without_comments(&style_blocks_cased(page));
    let all = rules(&css);
    // custom properties; a name given two different values is ambiguous (which one applies depends on the
    // element): left out, so a use of it is "cannot resolve"
    let mut vars: HashMap<String, String> = HashMap::new();
    let mut ambiguous: Vec<String> = Vec::new();
    for (_, block) in &all {
        for (k, v) in decls(block).filter(|(k, _)| k.starts_with("--")) {
            match vars.get(&k) {
                Some(old) if old != v => ambiguous.push(k),
                _ => {
                    vars.insert(k, v.to_string());
                }
            }
        }
    }
    for k in ambiguous {
        vars.remove(&k);
    }
    let svgs = svg_blocks(&lower);
    let mut list: Vec<SvgRule> = Vec::new();
    for (sel, block) in &all {
        let paint: Vec<(String, String)> =
            decls(block).filter(|(k, _)| SVG_PAINT.contains(&k.as_str())).map(|(k, v)| (k, v.to_string())).collect();
        let moves = decls(block)
            .any(|(k, _)| matches!(k.as_str(), "animation" | "animation-name" | "transition" | "transform"));
        let sel = sel.to_ascii_lowercase();
        // only a rule that can reach an svg part matters (by tag, or a class / id used inside an svg)
        if (paint.is_empty() && !moves) || !styles_svg_parts(&format!("{sel}{{fill:x}}"), &svgs) {
            continue;
        }
        let mut sels = Vec::new();
        for one in sel.split(',').map(str::trim) {
            let s = selector(one).ok_or_else(|| {
                format!("an inline SVG styled by a CSS selector the fast renderer cannot match ({one})")
            })?;
            let sp = s
                .iter()
                .map(|(c, _)| c.id.is_some() as u32 * 10000 + c.classes.len() as u32 * 100 + c.tag.is_some() as u32)
                .sum();
            sels.push((s, sp));
        }
        list.push(SvgRule { sels, paint, moves });
    }
    let unknown = |v: &str| format!("an inline SVG painted with a CSS variable the fast renderer cannot resolve ({v})");
    // walk the tags: the open elements, and for each start tag inside an svg its rewritten text
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut stack: Vec<El> = Vec::new();
    let (mut in_svg, mut at) = (0usize, 0usize);
    while let Some(i) = page[at..].find('<') {
        let start = at + i;
        let Some(j) = page[start..].find('>') else { break };
        let end = start + j + 1;
        at = end;
        let (tag, tl) = (&page[start..end], &lower[start..end]);
        if tl.starts_with("<!--") {
            at = lower[start..].find("-->").map_or(page.len(), |k| start + k + 3);
            continue;
        }
        if tl.starts_with("<style") || tl.starts_with("<script") {
            let close = if tl.starts_with("<style") { "</style" } else { "</script" };
            at = lower[end..].find(close).map_or(page.len(), |k| end + k);
            continue;
        }
        let name_of = |t: &str| t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>();
        if let Some(t) = tl.strip_prefix("</") {
            let name = name_of(t);
            if let Some(k) = stack.iter().rposition(|e| e.tag == name) {
                stack.truncate(k);
            }
            if name == "svg" {
                in_svg = in_svg.saturating_sub(1);
            }
            continue;
        }
        let name = name_of(&tl[1..]);
        if name.is_empty() {
            continue; // <!doctype>, <?xml>
        }
        const VOID: [&str; 8] = ["meta", "link", "br", "img", "input", "hr", "source", "wbr"];
        let closed = tl.ends_with("/>") || (in_svg == 0 && VOID.contains(&name.as_str()));
        stack.push(El {
            tag: name.clone(),
            classes: attr(tl, "class").map(|c| c.split_whitespace().map(String::from).collect()).unwrap_or_default(),
            id: attr(tl, "id").unwrap_or("").to_string(),
        });
        if name == "svg" {
            in_svg += 1;
        }
        if in_svg > 0 {
            // the rules that match, lowest specificity first, then in source order: a later one wins
            let mut got: Vec<(u32, usize)> = list
                .iter()
                .enumerate()
                .filter_map(|(n, r)| {
                    r.sels.iter().filter(|(s, _)| matches(s, &stack)).map(|(_, sp)| *sp).max().map(|sp| (sp, n))
                })
                .collect();
            got.sort();
            if name != "svg" && got.iter().any(|&(_, n)| list[n].moves) {
                return Err("an animated part of an inline SVG (a CSS animation or transform on its shapes)".into());
            }
            let mut style: Vec<(String, String)> = Vec::new();
            for &(_, n) in &got {
                for (k, v) in &list[n].paint {
                    let v = resolve_vars(v, &vars).map_err(|_| unknown(v))?;
                    style.retain(|(k2, _)| k2 != k);
                    style.push((k.clone(), v.replace('"', "'")));
                }
            }
            // var() in the tag itself (a presentation attribute, its own style): resolved in place
            let mut new_tag = tag.to_string();
            if tag.contains("var(") {
                let (mut out, mut rest) = (String::new(), tag);
                while let Some(k) = rest.find("var(") {
                    let q = rest[k..].find(['"', '\'']).map_or(rest.len(), |e| k + e); // the value ends at its quote
                    out.push_str(&rest[..k]);
                    out.push_str(
                        &resolve_vars(&rest[k..q], &vars).map_err(|_| unknown(&rest[k..q]))?.replace('"', "'"),
                    );
                    rest = &rest[q..];
                }
                out.push_str(rest);
                new_tag = out;
            }
            if !style.is_empty() {
                let ours: Vec<String> = style.iter().map(|(k, v)| format!("{k}:{v}")).collect();
                let ours = ours.join(";");
                // the element's own style attribute still wins over the stylesheet: it comes after ours
                new_tag = match attr(&new_tag, "style").map(str::to_string) {
                    Some(own) => new_tag.replacen(&format!("\"{own}\""), &format!("\"{ours};{own}\""), 1),
                    None => {
                        let close = if new_tag.ends_with("/>") { new_tag.len() - 2 } else { new_tag.len() - 1 };
                        format!("{} style=\"{ours}\"{}", new_tag[..close].trim_end(), &new_tag[close..])
                    }
                };
            }
            if new_tag != tag {
                edits.push((start, end, new_tag));
            }
        }
        if closed {
            stack.pop();
            if name == "svg" {
                in_svg = in_svg.saturating_sub(1);
            }
        }
    }
    if edits.is_empty() {
        return Ok(page.to_string());
    }
    let mut out = String::with_capacity(page.len() + edits.len() * 64);
    let mut from = 0;
    for (a, b, t) in edits {
        out.push_str(&page[from..a]);
        out.push_str(&t);
        from = b;
    }
    out.push_str(&page[from..]);
    Ok(out)
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
    if values(&t, "mix-blend-mode")
        .iter()
        .chain(values(&t, "background-blend-mode").iter())
        .any(|v| !v.starts_with("normal"))
    {
        block("a blend mode (mix-blend-mode)");
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
    if values(&t, "writing-mode").iter().any(|v| v.contains("vertical")) {
        block("vertical writing (writing-mode)");
    }
    // an inline <svg> whose parts the page's own CSS paints: the helper's SVG engine never sees the stylesheet,
    // so `inline_svg_styles` writes those rules onto the parts first (`blitz::Helper::load`); only what it
    // cannot resolve goes to Edge
    if t.contains("<svg") {
        if let Err(why) = inline_svg_styles(src) {
            block(&why);
        }
    }
    // ponytail: no notes today - text-shadow, ellipsis, text-wrap: balance, justify, backdrop-filter,
    // -webkit-text-stroke and @property all draw as in Edge with se-html 0.1.0 (blitz_tests::
    // lint_rules_against_the_real_helper measures every rule); `notes` stays for the next helper's gaps
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
            "p{mix-blend-mode:screen}",
            "p{position:sticky;top:0}",
            "p{transform:rotateY(40deg)}",
            ".s{perspective:300px}",
            "p::before{content:counter(n)}",
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
        // measured against the real helper (blitz_tests::lint_rules_against_the_real_helper): drawn as in Edge
        for css in [
            "p{text-shadow:0 2px 8px #000}",
            "p{text-overflow:ellipsis;-webkit-line-clamp:2}",
            "p{backdrop-filter:blur(8px)}",
            "p{-webkit-text-stroke:2px #000;paint-order:stroke fill}",
            "@property --n{syntax:'<integer>';inherits:false;initial-value:0}",
            "p{text-wrap:balance;text-align:justify}",
        ] {
            assert_eq!(scan(&format!("<style>{css}</style><p>x</p>")), Findings::default(), "{css}");
        }
    }

    #[test]
    fn inline_svg_is_only_a_problem_when_its_css_cannot_be_written_onto_it() {
        assert!(scan("<svg width=4><circle fill=\"red\"/></svg>").blocks.is_empty());
        // resolved onto the parts (`inline_svg_styles`): the fast renderer draws them
        assert!(scan("<style>.a{fill:#fff}</style><svg><circle class=a /></svg>").blocks.is_empty());
        assert!(scan("<style>.box path{fill:none;stroke:#fff}</style><svg class=box><path d=\"M0 0\"/></svg>")
            .blocks
            .is_empty());
        // what cannot be: Edge
        assert!(!scan("<style>.a:hover{fill:#fff}</style><svg><circle class=a /></svg>").blocks.is_empty());
        assert!(!scan("<svg><circle fill=\"var(--nope)\" /></svg>").blocks.is_empty());
        assert!(!scan("<style>.s{animation:a 1s}</style><svg><rect class=s /></svg>").blocks.is_empty());
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

    #[test]
    fn svg_parts_get_the_page_css_as_a_browser_applies_it() {
        let page = r##"<style>:root{--accent:#ff3b30}.fill .s svg path{fill:var(--accent)}
            #big{stroke:#00F;stroke-width:3}circle{fill:green}.c{fill:red}</style>
            <div class="fill"><div class="s"><svg viewBox="0 0 4 4"><path d="M0 0"/></svg></div></div>
            <div class="base"><svg><path d="M1 1"/><circle class="c" id="big" style="opacity:.5"/><circle/></svg></div>
            <svg><rect fill="var(--accent)" width="2" height="2"/></svg>"##;
        let out = inline_svg_styles(page).unwrap();
        // the descendant rule reaches the path under .fill .s only, var() resolved
        assert!(out.contains(r#"<path d="M0 0" style="fill:#ff3b30"/>"#), "{out}");
        assert!(out.contains(r#"<path d="M1 1"/>"#), "{out}");
        // specificity: #id > .class > tag, and the element's own style comes last (it wins)
        assert!(
            out.contains(r#"<circle class="c" id="big" style="fill:red;stroke:#00F;stroke-width:3;opacity:.5"/>"#),
            "{out}"
        );
        assert!(out.contains(r#"<circle style="fill:green"/>"#), "{out}");
        // a var() in a presentation attribute (an older Sale Badge) is resolved in place
        assert!(out.contains(r##"<rect fill="#ff3b30" width="2""##), "{out}");
        // no svg: untouched
        assert_eq!(
            inline_svg_styles("<p style=\"color:var(--x)\">x</p>").unwrap(),
            "<p style=\"color:var(--x)\">x</p>"
        );
    }
}
