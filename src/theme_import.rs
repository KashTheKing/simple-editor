//! Import Theme from other editors' settings files. Only what those files really expose is mapped:
//! Premiere Pro stores a UI *brightness* level and highlight/focus colours, no full palette, so the
//! brightness picks the base greys and the colour (if any) becomes the accent; everything else
//! (clip colours, playhead style, radii) comes from the matching built-in "Premiere Pro" theme.
//! DaVinci Resolve has no readable theme file (its UI colours are fixed) and says so.
//! Key names are matched loosely (case-insensitive substrings) because the formats are undocumented.

use crate::theme::{preset, PaletteOverride};

#[derive(Debug)]
pub struct Imported {
    pub palette: PaletteOverride,
    pub summary: String,
}

/// `file_name` only helps tell apps apart; `text` is the file's contents.
pub fn import(file_name: &str, text: &str) -> Result<Imported, String> {
    let name = file_name.to_lowercase();
    let lower = text.to_lowercase();
    if name.contains("resolve") || lower.contains("davinci") || name.ends_with(".drx") {
        return Err("DaVinci Resolve doesn't keep a readable theme file (its colours are fixed in the app). \
                    Pick \"DaVinci Resolve Dark\" or \"DaVinci Resolve Light\" from the Theme list instead."
            .into());
    }
    let pairs = flatten(text)?;
    let find = |keys: &[&str]| pairs.iter().find(|(k, _)| keys.iter().any(|w| k.contains(w))).map(|(_, v)| v.clone());

    // CapCut (Chromium-style Preferences): only a dark/light flag is stored.
    if name.contains("capcut") || lower.contains("capcut") {
        let dark = find(&["theme", "darkmode", "dark_mode"]).map(|v| !v.to_lowercase().contains("light") && v != "false");
        return match dark {
            Some(d) => {
                let n = if d { "CapCut Dark" } else { "CapCut Light" };
                Ok(Imported {
                    palette: preset(n).unwrap(),
                    summary: format!("CapCut only stores dark/light: mapped to the built-in \"{n}\" theme."),
                })
            }
            None => Err("This CapCut file has no theme setting I can read.".into()),
        };
    }

    let brightness = find(&["brightness"]).and_then(|v| v.trim().trim_end_matches('%').parse::<f32>().ok()).map(|b| {
        // 0..1, 0..100 (percent) or 0..255
        (if b <= 1.0 { b } else if b <= 100.0 { b / 100.0 } else { b / 255.0 }).clamp(0.0, 1.0)
    });
    let accent = pairs
        .iter()
        .filter(|(k, _)| ["highlight", "accent", "focus", "interactive"].iter().any(|w| k.contains(w)))
        .find_map(|(_, v)| parse_color(v));
    if brightness.is_none() && accent.is_none() {
        return Err("Not a theme file I know: no brightness or highlight/accent colour found. \
                    Supported: Simple Editor .sedit-theme, Premiere Pro preferences (brightness + highlight colour), \
                    CapCut preferences (dark/light)."
            .into());
    }
    let b = brightness.unwrap_or(0.2);
    let dark = b < 0.5;
    // Premiere's levels run darkest..lightest; skip the mid greys so text keeps >= 4.5:1 contrast.
    let grey = if dark { 22.0 + b * 2.0 * 40.0 } else { 190.0 + (b - 0.5) * 2.0 * 50.0 };
    let g = |d: f32| [(grey + d).clamp(0.0, 255.0) as u8; 3];
    let s = if dark { 1.0 } else { -1.0 };
    let mut ov = preset(if dark { "Premiere Pro Dark" } else { "Premiere Pro Light" }).unwrap();
    ov.background = Some(g(-6.0 * s));
    ov.panel = Some(g(0.0));
    ov.header = Some(g(12.0 * s));
    ov.border = Some(g(28.0 * s));
    if let Some(a) = accent {
        ov.accent = Some(a);
        ov.playhead = Some(a);
    }
    ov.preset = String::new(); // a derived theme, not the stock preset
    let summary = format!(
        "Mapped {}{} onto a {} theme; clip colours, playhead style and corner radii are the built-in Premiere Pro \
         theme's, since Premiere doesn't store them.",
        if brightness.is_some() { "UI brightness" } else { "(no brightness found)" },
        if accent.is_some() { " + highlight colour" } else { "" },
        if dark { "dark" } else { "light" },
    );
    Ok(Imported { palette: ov, summary })
}

/// Parse "#rrggbb", "rrggbb", "r,g,b" or "r g b" (0..255).
fn parse_color(v: &str) -> Option<[u8; 3]> {
    let v = v.trim().trim_matches('"');
    let h = v.trim_start_matches('#');
    if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        let n = u32::from_str_radix(h, 16).ok()?;
        return Some([(n >> 16) as u8, (n >> 8) as u8, n as u8]);
    }
    let parts: Vec<&str> = v.split([',', ' ']).filter(|x| !x.is_empty()).collect();
    let n: Vec<u8> = parts.iter().filter_map(|x| x.parse().ok()).collect();
    (parts.len() == 3 && n.len() == 3).then(|| [n[0], n[1], n[2]])
}

/// (lowercase key, value) pairs from JSON, `<Key>value</Key>` XML or `key=value` / `key: value` lines.
fn flatten(text: &str) -> Result<Vec<(String, String)>, String> {
    let t = text.trim_start_matches('\u{feff}').trim();
    if t.is_empty() {
        return Err("The file is empty.".into());
    }
    let mut out = Vec::new();
    if t.starts_with('{') || t.starts_with('[') {
        fn walk(v: &serde_json::Value, key: &str, out: &mut Vec<(String, String)>) {
            match v {
                serde_json::Value::Object(m) => m.iter().for_each(|(k, v)| walk(v, &k.to_lowercase(), out)),
                serde_json::Value::Array(a) if !a.is_empty() && a.iter().all(|x| x.is_number()) => {
                    out.push((key.into(), a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")));
                }
                serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
                serde_json::Value::String(s) => out.push((key.into(), s.clone())),
                serde_json::Value::Null => {}
                other => out.push((key.into(), other.to_string())),
            }
        }
        let v: serde_json::Value = serde_json::from_str(t).map_err(|e| format!("Not valid JSON: {e}"))?;
        walk(&v, "", &mut out);
        return Ok(out);
    }
    if t.bytes().take(512).any(|b| b == 0) {
        return Err("This is a binary file; only text/XML/JSON settings files can be read.".into());
    }
    for line in t.lines() {
        let l = line.trim();
        if let (Some(open), Some(close)) = (l.find('<'), l.find("</")) {
            if let Some(gt) = l[open..].find('>') {
                if close > open + gt {
                    let key = l[open + 1..open + gt].split_whitespace().next().unwrap_or("");
                    out.push((key.to_lowercase(), l[open + gt + 1..close].trim().to_string()));
                    continue;
                }
            }
        }
        if let Some((k, v)) = l.split_once('=').or_else(|| l.split_once(':')) {
            out.push((k.trim().trim_matches('"').to_lowercase(), v.trim().trim_matches([',', '"']).to_string()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premiere_brightness_and_highlight_map() {
        let xml = "<PremiereData>\n<UIBrightness>0.1</UIBrightness>\n<FocusIndicatorColor>#2D8CEB</FocusIndicatorColor>\n</PremiereData>";
        let i = import("Adobe Premiere Pro Prefs", xml).unwrap();
        assert_eq!(i.palette.mode, "dark");
        assert_eq!(i.palette.accent, Some([0x2d, 0x8c, 0xeb]));
        assert_eq!(i.palette.panel, Some([30, 30, 30]));
        assert!(i.summary.contains("brightness"));
        let light = import("prefs.json", r#"{"Appearance":{"Brightness":0.95,"Highlight":"10,20,30"}}"#).unwrap();
        assert_eq!(light.palette.mode, "light");
        assert_eq!(light.palette.accent, Some([10, 20, 30]));
    }

    #[test]
    fn imported_text_is_readable_at_every_brightness() {
        for b in ["0", "0.2", "0.49", "0.5", "0.8", "1"] {
            let p = import("p.txt", &format!("Brightness={b}")).unwrap().palette;
            assert!(crate::theme::contrast(p.text.unwrap(), p.panel.unwrap()) >= 4.5, "{b}");
            assert!(crate::theme::contrast(p.text.unwrap(), p.background.unwrap()) >= 4.5, "{b}");
        }
    }

    #[test]
    fn capcut_dark_flag_and_resolve_message() {
        let i = import("CapCut Preferences", r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(i.palette.preset, "CapCut Dark");
        assert!(import("Resolve.cfg", "x=1").unwrap_err().contains("DaVinci Resolve"));
    }

    #[test]
    fn bad_files_give_clean_errors() {
        assert!(import("a.txt", "").is_err());
        assert!(import("a.json", "{ not json").unwrap_err().contains("JSON"));
        assert!(import("a.txt", "hello world\nnothing here").unwrap_err().contains("Not a theme file"));
        assert!(import("a.bin", "\0\0\0\0abc").is_err());
    }
}
