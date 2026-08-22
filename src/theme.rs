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
    let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
    let mut p = Palette::new(dark, ov.accent.map(rgb).unwrap_or_else(system_accent));
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
    p
}

fn visuals(dark: bool, accent: Color32) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(2);
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
    v.selection.bg_fill = if dark { accent.linear_multiply(0.6) } else { accent.linear_multiply(0.35) };
    v.selection.stroke.color = accent;
    v.hyperlink_color = accent;
    v.widgets.active.bg_fill = accent;
    v.widgets.active.weak_bg_fill = accent;
    v.widgets.hovered.bg_stroke.color = accent;
    v.slider_trailing_fill = true;
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

/// Apply the theme preference ("system" | "dark" | "light") to the egui context.
pub fn apply(ctx: &egui::Context, pref: &str) {
    // set_fonts is a no-op when unchanged; egui's own Ctrl+=/Ctrl+-/Ctrl+0 UI zoom would hijack the
    // timeline zoom hotkeys (and persist across runs)
    ctx.set_fonts(fonts());
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let accent = system_accent();
    ctx.set_visuals_of(Theme::Dark, visuals(true, accent));
    ctx.set_visuals_of(Theme::Light, visuals(false, accent));
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
}
