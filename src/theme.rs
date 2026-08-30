//! Match the user's Windows theme: light/dark from the registry, accent colour from DWM.
//! Bare, flat, Windows-Forms-ish visuals; DaVinci-like layout comes from the panels, not styling.

use eframe::egui::{self, Color32, CornerRadius, Theme, Visuals};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

fn reg_dword(root: winreg::HKEY, path: &str, name: &str) -> Option<u32> {
    winreg::RegKey::predef(root).open_subkey(path).ok()?.get_value::<u32, _>(name).ok()
}

/// Windows accent colour (falls back to the default blue).
pub fn system_accent() -> Color32 {
    static ACCENT: OnceLock<Color32> = OnceLock::new();
    *ACCENT.get_or_init(|| {
        reg_dword(winreg::enums::HKEY_CURRENT_USER, r"Software\Microsoft\Windows\DWM", "AccentColor")
            .map(|v| Color32::from_rgb((v & 0xff) as u8, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8))
            .unwrap_or(Color32::from_rgb(0, 120, 212))
    })
}

/// Colours for custom-painted widgets (timeline, preview, waveforms).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: Color32,
    pub bg: Color32,
    pub panel: Color32,
    pub header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub clip_video: Color32,
    pub clip_audio: Color32,
    pub clip_image: Color32,
    pub clip_text: Color32,
    pub clip_sequence: Color32,
    pub clip_shape: Color32,
    pub clip_adjust: Color32,
    pub waveform: Color32,
    pub playhead: Color32,
    pub in_out: Color32,
    pub keyframe: Color32,
    pub selection: Color32,
    /// Corner radius for custom-painted widget-like surfaces (buttons, chips, cards).
    pub rounding: f32,
}

impl Palette {
    pub fn new(dark: bool, accent: Color32) -> Self {
        let g = |v: u8| Color32::from_gray(v);
        if dark {
            Self {
                accent,
                bg: g(28),
                panel: g(38),
                header: g(46),
                border: g(62),
                text: g(225),
                text_dim: g(150),
                clip_video: Color32::from_rgb(62, 98, 142),
                clip_audio: Color32::from_rgb(56, 120, 84),
                clip_image: Color32::from_rgb(110, 84, 140),
                clip_text: Color32::from_rgb(150, 112, 58),
                clip_sequence: Color32::from_rgb(60, 130, 140),
                clip_shape: Color32::from_rgb(120, 100, 60),
                clip_adjust: Color32::from_rgb(96, 96, 110),
                waveform: Color32::from_rgb(150, 215, 170),
                playhead: Color32::from_rgb(235, 70, 70),
                in_out: accent,
                keyframe: Color32::from_rgb(255, 200, 60),
                selection: accent,
                rounding: 2.0,
            }
        } else {
            Self {
                accent,
                bg: g(250),
                panel: g(240),
                header: g(228),
                border: g(200),
                text: g(20),
                text_dim: g(100),
                clip_video: Color32::from_rgb(150, 180, 220),
                clip_audio: Color32::from_rgb(150, 205, 170),
                clip_image: Color32::from_rgb(190, 170, 215),
                clip_text: Color32::from_rgb(225, 195, 140),
                clip_sequence: Color32::from_rgb(150, 205, 215),
                clip_shape: Color32::from_rgb(225, 210, 160),
                clip_adjust: Color32::from_rgb(190, 190, 205),
                waveform: Color32::from_rgb(30, 110, 60),
                playhead: Color32::from_rgb(220, 40, 40),
                in_out: accent,
                keyframe: Color32::from_rgb(200, 140, 0),
                selection: accent,
                rounding: 2.0,
            }
        }
    }
    pub fn clip_color(&self, kind: crate::model::ClipKind) -> Color32 {
        use crate::model::ClipKind::*;
        match kind {
            Video => self.clip_video,
            Audio => self.clip_audio,
            Image => self.clip_image,
            Text => self.clip_text,
            Sequence => self.clip_sequence,
            Shape => self.clip_shape,
            Adjustment => self.clip_adjust,
        }
    }
}

/// Settings-window override for `Palette`'s hand-tuned colours ("Appearance" tab). `mode` picks the
/// base ("system" = today's Windows-derived behaviour, untouched; "light"/"dark" force the base;
/// "custom" keeps following Windows but every field below can still override it); each colour is
/// `None` until the user turns it on, so an untouched `PaletteOverride` changes nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PaletteOverride {
    pub mode: String,
    /// Name of the built-in preset this override started from ("" = none / hand-made).
    pub preset: String,
    pub background: Option<[u8; 3]>,
    pub panel: Option<[u8; 3]>,
    pub header: Option<[u8; 3]>,
    pub border: Option<[u8; 3]>,
    pub text: Option<[u8; 3]>,
    pub text_dim: Option<[u8; 3]>,
    pub accent: Option<[u8; 3]>,
    pub selection: Option<[u8; 3]>,
    pub keyframe: Option<[u8; 3]>,
    pub waveform: Option<[u8; 3]>,
}

/// `palette(ctx)` layered with a user override. "system" mode (the default, everything `None`) is
/// exactly `palette(ctx)` — the round-trip the Settings window's "Reset to system" button restores.
pub fn palette_with(ctx: &egui::Context, ov: &PaletteOverride) -> Palette {
    let dark = match ov.mode.as_str() {
        "light" => false,
        "dark" => true,
        "custom" => ctx.style().visuals.dark_mode,
        _ => return palette(ctx),
    };
    let mut p = Palette::new(dark, ov.accent.map(rgb).unwrap_or_else(system_accent));
    apply_overrides(&mut p, ov);
    p
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Layer the `Some` colour fields of `ov` onto `p` (accent/mode are handled by the caller).
fn apply_overrides(p: &mut Palette, ov: &PaletteOverride) {
    if let Some(c) = ov.background {
        p.bg = rgb(c);
    }
    if let Some(c) = ov.panel {
        p.panel = rgb(c);
    }
    if let Some(c) = ov.header {
        p.header = rgb(c);
    }
    if let Some(c) = ov.border {
        p.border = rgb(c);
    }
    if let Some(c) = ov.text {
        p.text = rgb(c);
    }
    if let Some(c) = ov.text_dim {
        p.text_dim = rgb(c);
    }
    if let Some(c) = ov.selection {
        p.selection = rgb(c);
    }
    if let Some(c) = ov.keyframe {
        p.keyframe = rgb(c);
    }
    if let Some(c) = ov.waveform {
        p.waveform = rgb(c);
    }
}

/// Darken an opaque colour by multiplying its RGB.
fn darken(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

/// Lighten an opaque colour by lerping its RGB toward white.
fn lighten(c: Color32, f: f32) -> Color32 {
    let l = |v: u8| (v as f32 + (255.0 - v as f32) * f) as u8;
    Color32::from_rgb(l(c.r()), l(c.g()), l(c.b()))
}

fn visuals(dark: bool, ov: &PaletteOverride, cozy: bool) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let accent = ov.accent.map(rgb).unwrap_or_else(system_accent);
    let r = CornerRadius::same(if cozy { 8 } else { 2 });
    v.window_corner_radius = r;
    v.menu_corner_radius = r;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    // Presets/custom colours: retint egui-native chrome (menus, combos, windows) to match the
    // custom-painted panes. Untouched overrides leave egui's stock dark/light visuals alone.
    let has_colors = ov.background.is_some() || ov.panel.is_some() || ov.header.is_some() || ov.border.is_some() || ov.text.is_some();
    if has_colors {
        let mut p = Palette::new(dark, accent);
        apply_overrides(&mut p, ov);
        v.panel_fill = p.panel;
        v.window_fill = p.panel;
        v.extreme_bg_color = p.bg;
        v.faint_bg_color = p.header;
        v.override_text_color = Some(p.text);
        v.window_stroke.color = p.border;
        v.widgets.noninteractive.bg_fill = p.panel;
        v.widgets.noninteractive.weak_bg_fill = p.panel;
        v.widgets.noninteractive.bg_stroke.color = p.border;
        v.widgets.inactive.bg_fill = p.header;
        v.widgets.inactive.weak_bg_fill = p.header;
        v.widgets.hovered.bg_fill = lighten(p.header, 0.08);
        v.widgets.hovered.weak_bg_fill = lighten(p.header, 0.08);
        v.widgets.open.bg_fill = p.header;
        v.widgets.open.weak_bg_fill = p.header;
    }
    v.selection.bg_fill = if dark { accent.linear_multiply(0.6) } else { accent.linear_multiply(0.35) };
    v.selection.stroke.color = accent;
    v.hyperlink_color = accent;
    v.widgets.active.bg_fill = accent;
    v.widgets.active.weak_bg_fill = accent;
    v.widgets.hovered.bg_stroke.color = accent;
    v.slider_trailing_fill = true;
    if cozy {
        // Soft borders: at rest a stroke just darker than the button's own fill; lighter on hover.
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive] {
            w.bg_stroke = egui::Stroke::new(1.0, darken(w.weak_bg_fill, 0.75));
        }
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, lighten(v.widgets.hovered.weak_bg_fill, 0.35));
        v.widgets.open.bg_stroke = egui::Stroke::new(1.0, darken(v.widgets.open.weak_bg_fill, 0.75));
    }
    v
}

/// egui's bundled fonts with the Windows UI font (Segoe UI) in front, so text matches the OS.
fn fonts() -> egui::FontDefinitions {
    let mut f = egui::FontDefinitions::default();
    let dir =
        std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into())).join("Fonts");
    if let Ok(bytes) = std::fs::read(dir.join("segoeui.ttf")) {
        f.font_data.insert("Segoe UI".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        f.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "Segoe UI".into());
    }
    f
}

/// Apply the theme preference ("system" | "dark" | "light"), palette override, and look
/// ("cozy" = rounded + soft borders, anything else = sharp) to the egui context.
pub fn apply(ctx: &egui::Context, pref: &str, ov: &PaletteOverride, look: &str) {
    // set_fonts is a no-op when unchanged; egui's own Ctrl+=/Ctrl+-/Ctrl+0 UI zoom would hijack the
    // timeline zoom hotkeys (and persist across runs)
    ctx.set_fonts(fonts());
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let cozy = look != "sharp";
    ctx.set_visuals_of(Theme::Dark, visuals(true, ov, cozy));
    ctx.set_visuals_of(Theme::Light, visuals(false, ov, cozy));
    let theme = match pref {
        "dark" => egui::ThemePreference::Dark,
        "light" => egui::ThemePreference::Light,
        _ => egui::ThemePreference::System,
    };
    ctx.set_theme(theme);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
    });
}

/// Palette for the theme currently in effect.
pub fn palette(ctx: &egui::Context) -> Palette {
    Palette::new(ctx.style().visuals.dark_mode, system_accent())
}

/// Built-in theme preset names, in menu order.
pub const PRESETS: &[&str] = &[
    "Charcoal Dark",
    "Light High Contrast",
    "Cyberpunk Neon",
    "GitHub Dark",
    "GitHub Light",
    "GitHub Dimmed",
    "GitHub High Contrast",
    "One Dark Pro",
    "Dracula",
    "Catppuccin Latte",
    "Catppuccin Frappé",
    "Catppuccin Macchiato",
    "Catppuccin Mocha",
    "Tokyo Night",
    "Tokyo Night Storm",
    "Tokyo Night Moon",
    "Tokyo Night Light",
    "Night Owl",
    "Shades of Purple",
    "SynthWave '84",
    "Nord",
    "Ayu Dark",
    "Ayu Mirage",
    "Ayu Light",
];

/// A built-in theme as a ready-made override. `None` for unknown names.
/// "Charcoal Dark" is the stock look (an empty override apart from forcing dark).
pub fn preset(name: &str) -> Option<PaletteOverride> {
    // (dark, bg, panel, header, border, text, text_dim, accent)
    #[allow(clippy::type_complexity)]
    let core: Option<(bool, [u8; 3], [u8; 3], [u8; 3], [u8; 3], [u8; 3], [u8; 3], [u8; 3])> = match name {
        "Light High Contrast" => {
            Some((false, [255, 255, 255], [245, 245, 245], [232, 232, 232], [110, 110, 110], [0, 0, 0], [55, 55, 55], [0, 90, 180]))
        }
        "Cyberpunk Neon" => {
            Some((true, [10, 10, 18], [18, 18, 30], [28, 28, 46], [70, 55, 105], [230, 240, 255], [135, 145, 175], [0, 255, 240]))
        }
        "GitHub Dark" => {
            Some((true, [13, 17, 23], [22, 27, 34], [33, 38, 45], [48, 54, 61], [230, 237, 243], [139, 148, 158], [31, 111, 235]))
        }
        "GitHub Light" => {
            Some((false, [255, 255, 255], [246, 248, 250], [234, 238, 242], [208, 215, 222], [31, 35, 40], [101, 109, 118], [9, 105, 218]))
        }
        "GitHub Dimmed" => {
            Some((true, [34, 39, 46], [45, 51, 59], [55, 62, 71], [68, 76, 86], [173, 186, 199], [118, 131, 144], [83, 155, 245]))
        }
        "GitHub High Contrast" => {
            Some((true, [1, 4, 9], [13, 17, 23], [33, 38, 45], [110, 118, 129], [240, 246, 252], [190, 198, 208], [64, 158, 255]))
        }
        "One Dark Pro" => {
            Some((true, [33, 37, 43], [40, 44, 52], [50, 56, 66], [63, 70, 82], [171, 178, 191], [122, 130, 143], [97, 175, 239]))
        }
        "Dracula" => {
            Some((true, [33, 34, 44], [40, 42, 54], [68, 71, 90], [98, 114, 164], [248, 248, 242], [155, 160, 180], [189, 147, 249]))
        }
        "Catppuccin Latte" => {
            Some((false, [239, 241, 245], [230, 233, 239], [220, 224, 232], [172, 176, 190], [76, 79, 105], [108, 111, 133], [136, 57, 239]))
        }
        "Catppuccin Frappé" => {
            Some((true, [41, 44, 60], [48, 52, 70], [65, 69, 89], [98, 104, 128], [198, 208, 245], [148, 156, 187], [202, 158, 230]))
        }
        "Catppuccin Macchiato" => {
            Some((true, [30, 32, 48], [36, 39, 58], [54, 58, 79], [91, 96, 120], [202, 211, 245], [147, 154, 183], [198, 160, 246]))
        }
        "Catppuccin Mocha" => {
            Some((true, [24, 24, 37], [30, 30, 46], [49, 50, 68], [88, 91, 112], [205, 214, 244], [147, 153, 178], [203, 166, 247]))
        }
        "Tokyo Night" => {
            Some((true, [22, 22, 30], [26, 27, 38], [41, 46, 66], [59, 66, 97], [169, 177, 214], [120, 124, 153], [122, 162, 247]))
        }
        "Tokyo Night Storm" => {
            Some((true, [31, 35, 53], [36, 40, 59], [48, 52, 70], [65, 72, 104], [169, 177, 214], [120, 124, 153], [122, 162, 247]))
        }
        "Tokyo Night Moon" => {
            Some((true, [30, 32, 48], [34, 36, 54], [47, 52, 70], [68, 74, 100], [200, 211, 245], [130, 139, 184], [130, 170, 255]))
        }
        "Tokyo Night Light" => {
            Some((false, [225, 226, 231], [213, 214, 220], [200, 202, 210], [150, 152, 165], [52, 59, 88], [100, 106, 130], [46, 89, 180]))
        }
        "Night Owl" => {
            Some((true, [1, 22, 39], [5, 30, 52], [13, 42, 66], [60, 90, 120], [214, 222, 235], [99, 119, 153], [130, 170, 255]))
        }
        "Shades of Purple" => {
            Some((true, [35, 33, 66], [45, 43, 85], [58, 55, 105], [95, 90, 160], [255, 255, 255], [165, 160, 210], [250, 214, 73]))
        }
        "SynthWave '84" => {
            Some((true, [30, 27, 44], [38, 35, 53], [52, 45, 74], [110, 80, 140], [240, 238, 250], [150, 140, 180], [255, 126, 219]))
        }
        "Nord" => {
            Some((true, [40, 44, 55], [46, 52, 64], [59, 66, 82], [76, 86, 106], [216, 222, 233], [150, 160, 175], [136, 192, 208]))
        }
        "Ayu Dark" => {
            Some((true, [11, 14, 20], [18, 22, 29], [28, 33, 43], [55, 62, 75], [191, 189, 182], [120, 125, 135], [230, 180, 80]))
        }
        "Ayu Mirage" => {
            Some((true, [26, 30, 40], [31, 36, 48], [42, 48, 62], [70, 78, 95], [204, 202, 195], [135, 140, 150], [255, 204, 102]))
        }
        "Ayu Light" => {
            Some((false, [252, 252, 252], [243, 244, 245], [232, 234, 237], [190, 195, 200], [92, 97, 101], [140, 145, 150], [255, 154, 0]))
        }
        "Charcoal Dark" => None,
        _ => return None,
    };
    let mut ov = match core {
        Some((dark, bg, panel, header, border, text, text_dim, accent)) => PaletteOverride {
            mode: if dark { "dark" } else { "light" }.into(),
            background: Some(bg),
            panel: Some(panel),
            header: Some(header),
            border: Some(border),
            text: Some(text),
            text_dim: Some(text_dim),
            accent: Some(accent),
            ..Default::default()
        },
        None => PaletteOverride { mode: "dark".into(), ..Default::default() },
    };
    // Signature extras for the flashier themes.
    match name {
        "Cyberpunk Neon" => {
            ov.selection = Some([255, 0, 170]);
            ov.keyframe = Some([255, 0, 170]);
        }
        "Dracula" => {
            ov.selection = Some([255, 121, 198]);
            ov.keyframe = Some([241, 250, 140]);
            ov.waveform = Some([80, 250, 123]);
        }
        "Night Owl" => ov.waveform = Some([127, 219, 202]),
        "Shades of Purple" => ov.selection = Some([255, 98, 220]),
        "SynthWave '84" => {
            ov.selection = Some([114, 241, 184]);
            ov.keyframe = Some([254, 222, 90]);
        }
        _ => {}
    }
    ov.preset = name.into();
    Some(ov)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_round_trips_and_falls_through() {
        let ctx = egui::Context::default();
        // unset (the default): falls through to today's system-derived palette exactly.
        let derived = palette_with(&ctx, &PaletteOverride::default());
        let sys = palette(&ctx);
        assert_eq!(derived.bg, sys.bg);
        assert_eq!(derived.accent, sys.accent);

        // custom: only the fields turned on differ; the rest still tracks the (overridden) base.
        let mut ov = PaletteOverride { mode: "custom".into(), ..Default::default() };
        ov.background = Some([10, 20, 30]);
        ov.accent = Some([200, 30, 40]);
        let p = palette_with(&ctx, &ov);
        assert_eq!(p.bg, Color32::from_rgb(10, 20, 30));
        assert_eq!(p.accent, Color32::from_rgb(200, 30, 40));
        assert_eq!(p.panel, Palette::new(ctx.style().visuals.dark_mode, Color32::from_rgb(200, 30, 40)).panel);

        // what Settings::save()/load() actually persists.
        let json = serde_json::to_string(&ov).unwrap();
        let back: PaletteOverride = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ov);
    }

    #[test]
    fn every_preset_resolves_and_round_trips() {
        for name in PRESETS {
            let ov = preset(name).unwrap_or_else(|| panic!("missing preset {name}"));
            assert!(ov.mode == "dark" || ov.mode == "light", "{name}");
            assert_eq!(ov.preset, *name);
            let back: PaletteOverride = serde_json::from_str(&serde_json::to_string(&ov).unwrap()).unwrap();
            assert_eq!(back, ov);
        }
        assert!(preset("Nonexistent").is_none());
        // Charcoal Dark = stock: no colour overrides.
        assert_eq!(preset("Charcoal Dark").unwrap().background, None);
    }
}
