//! Library shader files: the `// @key value` header of a shader effect or shader transition. The
//! format is specified in `library/README.md`; `engine::shaders::{user_shader, transition_shader}`
//! turn a file into a complete program.
// ---- ws:shader-library ----

/// One `// @param <id> <default> <min> <max>` line: a named, ranged control (`p<index>` in GLSL).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderParam {
    pub id: String,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}

/// A shader file's header.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderMeta {
    pub name: String,
    pub desc: String,
    pub tags: Vec<String>,
    pub params: Vec<ShaderParam>,
    /// `// @needs previous`: the effect also samples earlier frames (`u_prev` / `u_next` / `u_frames`).
    pub needs_previous: bool,
    pub credit: String,
}

/// Names a `@param` may not take: they are already something in the assembled program.
const RESERVED: &[&str] = &[
    "tex", "uv", "src", "main", "effect", "transition", "progress", "ratio", "texture", "luma", "hash12", "hash11",
    "rgb2hsl", "hsl2rgb", "hue2rgb", "sample_clamped", "mask_at", "apply_mask", "float", "int", "vec2", "vec3", "vec4",
    "bool", "mix", "step", "time",
];

fn valid_id(id: &str) -> bool {
    let mut c = id.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase())
        && c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !RESERVED.contains(&id)
        && !(id.len() == 2 && (id.starts_with('p') || id.starts_with('u')) && id.as_bytes()[1].is_ascii_digit())
        && !id.starts_with("u_")
}

/// Read the header: the `//` lines before the first line of code. Unknown keys and malformed
/// `@param` lines are skipped (`lint` reports them); at most 8 params are kept.
pub fn parse_meta(src: &str) -> ShaderMeta {
    let mut m = ShaderMeta::default();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix("//") else { break };
        let Some(rest) = rest.trim().strip_prefix('@') else { continue };
        let (key, val) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let val = val.trim();
        match key {
            "name" => m.name = val.to_string(),
            "desc" => m.desc = val.to_string(),
            "credit" => m.credit = val.to_string(),
            "tags" => {
                m.tags = val.split(',').map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()).collect()
            }
            "needs" => m.needs_previous |= val.starts_with("previous"),
            "param" => {
                let w: Vec<&str> = val.split_whitespace().collect();
                let n: Vec<f64> = w.iter().skip(1).filter_map(|v| v.parse().ok()).collect();
                if w.len() == 4 && n.len() == 3 && valid_id(w[0]) && n[1] < n[2] && m.params.len() < 8 {
                    let id = w[0].to_string();
                    if !m.params.iter().any(|p| p.id == id) {
                        m.params.push(ShaderParam { id, default: n[0].clamp(n[1], n[2]), min: n[1], max: n[2] });
                    }
                }
            }
            _ => {}
        }
    }
    m
}

/// "glow_size" -> "Glow Size": what the Inspector shows for a `@param` or an HTML field id.
pub fn label(id: &str) -> String {
    id.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What is wrong with a LIBRARY shader file (stricter than what the app accepts from a user):
/// empty = fine. `transition` picks the entry point that must exist.
pub fn lint(src: &str, transition: bool) -> Vec<String> {
    let m = parse_meta(src);
    let mut out = Vec::new();
    for (what, v) in [("@name", &m.name), ("@desc", &m.desc)] {
        if v.is_empty() {
            out.push(format!("missing `// {what}`"));
        }
    }
    if m.tags.is_empty() {
        out.push("missing `// @tags`".into());
    }
    let declared = src.lines().take_while(|l| l.trim().is_empty() || l.trim().starts_with("//"));
    let declared = declared.filter(|l| l.trim().trim_start_matches('/').trim().starts_with("@param")).count();
    if declared != m.params.len() {
        out.push(format!(
            "{declared} `@param` lines but {} are valid (need `<id> <default> <min> <max>`, a lowercase id that is \
             not a reserved name, min < max, no duplicates, at most 8)",
            m.params.len()
        ));
    }
    for bad in ["#version", "precision ", "uniform ", "void main"] {
        if src.lines().any(|l| !l.trim().starts_with("//") && l.contains(bad)) {
            out.push(format!("must not contain `{bad}` (the app adds it)"));
        }
    }
    let entry = if transition { "vec4 transition(" } else { "vec4 effect(" };
    if !src.contains(entry) && (transition || !src.contains("mainImage")) {
        out.push(format!("no `{entry}vec…)` entry point"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_read_until_the_first_line_of_code() {
        let src = "// @name Glow\n// @desc Soft glow.\n// @tags light, Bloom\n// @param glow_size 0.5 0 2\n\
                   // @param tex 1 0 2\n// @param bad 1 2\n// @needs previous\n// @credit Someone, MIT\n\
                   vec4 effect(vec4 src, vec2 uv) { return src; }\n// @param late 1 0 2\n";
        let m = parse_meta(src);
        assert_eq!((m.name.as_str(), m.desc.as_str(), m.credit.as_str()), ("Glow", "Soft glow.", "Someone, MIT"));
        assert_eq!(m.tags, ["light", "bloom"]);
        assert_eq!(m.params, [ShaderParam { id: "glow_size".into(), default: 0.5, min: 0.0, max: 2.0 }]);
        assert!(m.needs_previous);
        assert_eq!(label("glow_size"), "Glow Size");
        assert_eq!(lint(src, false).len(), 1, "the two bad @param lines are one finding");
        assert!(lint("vec4 effect(vec4 s, vec2 uv) { return s; }", false).len() >= 3);
        assert!(lint("// @name A\n// @desc B\n// @tags c\nuniform float x;\nvec4 transition(vec2 uv){}", true)
            .iter()
            .any(|e| e.contains("uniform")));
    }
}
