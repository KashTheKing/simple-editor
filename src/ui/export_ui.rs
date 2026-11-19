//! Export settings - the Export pane (`Pane::Export`, the Export page's left column; ws:pages made it a
//! pane, it was the "Export" window): resolution (Project / 720p / 1080p / 1440p / 4K /
//! Custom W×H, keeping the project aspect unless unlocked), scaler for up/downscaling (Nearest, Bilinear,
//! Bicubic, Lanczos, Area, Spline → ffmpeg flags), encoder (auto + detected), quality as a 0-100% slider
//! (converted to/from the underlying CRF) with an estimated output size and a quality-loss warning, preset,
//! an "audio only" hint by extension, and the Fast Lossless Cut option when `export::lossless_segments` applies
//! (with the keyframe-accuracy note). "Export…" opens the save dialog and returns the options; the pane
//! stays usable while an export runs (progress + cancel are shown by the app). Settings remember the last
//! scaler/resolution. Returns Some(options) when the user confirmed an export this frame.
//!
//! ws:export-deliver: platform preset tiles (`Settings.export_presets`) sit above the old grid, which is
//! now a collapsed "Advanced" section; an "Export In/Out Range" box binds `project.in_point/out_point`
//! into `ExportOptions.range` (and greys out the lossless cut); a loudness checkbox drives
//! `Settings.loudnorm`; "Add to Queue" returns the same choice flagged for the render queue.
//!
//! Export panel redesign: top to bottom, preset cards (the two built-ins "Max quality" and "Fast
//! preview", then small square platform tiles with their brand marks, then the user's own presets:
//! save / edit / duplicate / delete from a right-click), the project settings (the Inspector's own
//! form, so the Export page needs no Inspector), then the Video / Audio / Advanced sections (the
//! Inspector's collapsible sections; open state in `Settings.export_folds`), and an output bar pinned
//! to the bottom (summary line, Speed, file name + folder, Export… / Add to Queue).

use crate::engine::export::{self, ExportOptions, ExportPreset};
use crate::media::Backend;
use crate::model::Project;
use crate::settings::{ExportPresetRef, Settings};
use crate::ui::brand_icons;
use crate::ui::tools::{draw_glyph, Glyph};
use crate::ui::{combo, duration_text, encoder_options, menu, mmss, ENCODER_PRESETS};
use eframe::egui::{self, Color32, CornerRadius, Stroke, StrokeKind};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct ExportUi {
    /// Set by Export Video… (`App::act_export`, after its preflight): the next frame switches to the
    /// Export page and brings the pane to the front (`windows.rs`), then clears it.
    pub open: bool,
    /// "project" | "720" | "1080" | "1440" | "2160" | "custom"
    pub preset: String,
    pub custom: (u32, u32),
    pub lossless: bool,
    // ---- ws:export-deliver ----
    /// "Export In/Out Range" checkbox.
    pub range: bool,
    /// The preset card currently applied (by name); cleared when the user edits the size by hand.
    pub tile: Option<String>,
    /// Fit-and-pad instead of stretch - set with a tile, cleared with a manual size edit.
    pub letterbox: bool,
    /// The chosen container ("mp4"; "" = mp4): the Format row, set by a preset; the save dialog's default.
    pub ext: String,
    /// Off by default: exports strip metadata unless the user turns this on.
    pub metadata_on: bool,
    /// Editable `(key, value)` rows, seeded from the project the first time it is turned on.
    pub metadata: Vec<(String, String)>,
    /// "Use project background" checkbox - see `ExportChoice::use_project_bg`.
    pub use_project_bg: bool,
    /// The quality percent as last shown for a given stored CRF - pct→CRF→pct round-trips lose up to
    /// 2 % (52 CRF steps vs 101 slider positions), so without this a typed "85" snapped back to 84.
    pub pct_cache: Option<(u32, u32)>,
    // ---- export panel redesign ----
    /// The open "Save current settings as preset…" / "Edit preset…" form.
    pub form: Option<PresetForm>,
    /// The output bar's file name (without extension); "" = the suggestion (`suggested_name`).
    pub out_name: String,
    /// The output bar's folder; None = beside the source video. The save dialog's answer updates it.
    pub out_dir: Option<PathBuf>,
    /// The Frame rate row: output fps, 0 = the project's (set by a preset that names one).
    pub fps: f64,
    // ---- ws:max-duration ----
    /// Include what runs past the timeline's Max length (off: the export stops at it).
    pub overflow: bool,
}

/// The inline preset form: a new preset from the current settings (`editing` None) or an edit of the
/// user preset called `editing` (name, icon, colour).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetForm {
    pub editing: Option<String>,
    pub name: String,
    pub icon: String,
    pub color: Option<[u8; 3]>,
    pub error: String,
}

pub struct ExportChoice {
    pub opts: ExportOptions,
    pub lossless: bool,
    /// The "Use project background" checkbox. `App::start_export_choice` acts on it: off forces the
    /// exported clone's `preview_bg` to `Black` (the pre-setting behaviour), on keeps it as authored.
    /// Works on both frame sources - `engine::compose` clears to `preview_bg` too (`fill_background`).
    pub use_project_bg: bool,
    // ---- ws:export-deliver ----
    /// What Quick Export should re-run next time (stored in `Settings.last_export` when this starts).
    pub last: ExportPresetRef,
}

// ---- ws:export-deliver ----
/// Apply a preset card: size (0 × 0 = the project's own) + container + quality + loudness + encoder +
/// speed (+ the audio layout when the preset names one). A platform tile fits-and-pads; a preset saved
/// from a hand-typed size stretches like that size did. Encoder and speed are always set (empty =
/// auto / balanced), so a preset means the same export whichever card was clicked before it.
pub fn apply_preset(state: &mut ExportUi, settings: &mut Settings, p: &ExportPreset) {
    state.tile = Some(p.name.clone());
    if p.width == 0 || p.height == 0 {
        state.preset = "project".into();
        state.letterbox = false;
        settings.export_resolution = "project".into();
    } else {
        state.preset = "custom".into();
        state.custom = (p.width.max(16), p.height.max(16));
        state.letterbox = !p.stretch;
        settings.export_resolution = format!("{}x{}", p.width, p.height);
    }
    state.ext = if p.ext.is_empty() { "mp4".into() } else { p.ext.clone() };
    state.fps = p.fps;
    state.lossless = false;
    state.pct_cache = None;
    settings.crf = p.crf;
    settings.loudnorm = p.loudnorm;
    settings.encoder = if p.encoder.is_empty() { "auto".into() } else { p.encoder.clone() };
    settings.export_speed = if p.speed.is_empty() { "balanced".into() } else { p.speed.clone() };
    settings.preset = "auto".into();
    if !p.audio.is_empty() {
        settings.export_audio = p.audio.clone();
    }
}

/// Do the current settings still say what preset `p` says? False shows "edited" on its card.
pub fn preset_matches(p: &ExportPreset, state: &ExportUi, settings: &Settings, project: &Project) -> bool {
    let size = preset_size(project.width, project.height, &state.preset, state.custom);
    let size_ok = if p.width == 0 || p.height == 0 {
        size.is_none()
    } else {
        size.unwrap_or((project.width, project.height)) == (p.width, p.height)
    };
    let or = |v: &str, d: &'static str| if v.is_empty() { d.to_string() } else { v.to_string() };
    size_ok
        && default_ext(state) == or(&p.ext, "mp4")
        && settings.crf == p.crf
        && settings.loudnorm == p.loudnorm
        && or(&settings.encoder, "auto") == or(&p.encoder, "auto")
        && or(&settings.export_speed, "balanced") == or(&p.speed, "balanced")
        && matches!(settings.preset.as_str(), "" | "auto")
        && (p.audio.is_empty() || p.audio == settings.export_audio)
        && out_fps(p.fps, project) == out_fps(state.fps, project)
}

/// The output frame rate `fps` asks for: None = the project's own (0, or the same rate).
pub fn out_fps(fps: f64, project: &Project) -> Option<f64> {
    (fps > 0.0 && (fps - project.fps).abs() > 1e-3).then_some(fps)
}

/// The Export pane's current settings as a preset called `name` (what "Save current settings as
/// preset…" and `export.preset_save` store).
pub fn current_as_preset(state: &ExportUi, settings: &Settings, project: &Project, name: &str) -> ExportPreset {
    let (width, height) = preset_size(project.width, project.height, &state.preset, state.custom).unwrap_or((0, 0));
    ExportPreset {
        name: name.trim().to_string(),
        ext: default_ext(state).to_string(),
        width,
        height,
        crf: settings.crf,
        loudnorm: settings.loudnorm,
        encoder: settings.encoder.clone(),
        speed: export::effective_preset(&settings.preset, &settings.export_speed),
        audio: settings.export_audio.clone(),
        stretch: width > 0 && !(state.letterbox && state.tile.is_some()),
        icon: String::new(),
        color: None,
        fps: out_fps(state.fps, project).unwrap_or(0.0),
    }
}

/// Why `name` can't be a (new) user preset name, if it can't: empty, a built-in's or platform tile's
/// name, or another user preset's (`except` = the one being renamed).
fn name_refusal(settings: &Settings, name: &str, except: Option<&str>) -> Option<String> {
    let n = name.trim();
    if n.is_empty() {
        return Some("Give the preset a name".into());
    }
    if export::is_builtin_preset(n) || settings.export_presets.iter().any(|p| p.name.eq_ignore_ascii_case(n)) {
        return Some(format!("\"{n}\" is a built-in preset - pick another name"));
    }
    let clash = settings
        .export_user_presets
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(n) && !except.is_some_and(|e| e.eq_ignore_ascii_case(&p.name)));
    clash.then(|| format!("You already have a preset called \"{n}\""))
}

/// Store `p` as a user preset. `overwrite`: replace a user preset of the same name (the MCP save)
/// instead of refusing it (the form). Built-in and platform names are always refused.
pub fn save_user_preset(settings: &mut Settings, mut p: ExportPreset, overwrite: bool) -> Result<(), String> {
    p.name = p.name.trim().to_string();
    let existing = settings.export_user_presets.iter().position(|q| q.name.eq_ignore_ascii_case(&p.name));
    match (existing, overwrite) {
        (Some(i), true) if name_refusal(settings, &p.name, Some(&p.name)).is_none() => {
            settings.export_user_presets[i] = p;
            Ok(())
        }
        _ => match name_refusal(settings, &p.name, None) {
            Some(why) => Err(why),
            None => {
                settings.export_user_presets.push(p);
                Ok(())
            }
        },
    }
}

/// Rename (and re-icon / re-colour) the user preset `old`. Built-ins and platform tiles refuse. A
/// `Settings.last_export` pointing at it follows the new name, so Quick Export keeps working.
pub fn edit_user_preset(
    settings: &mut Settings,
    old: &str,
    name: &str,
    icon: Option<&str>,
    color: Option<Option<[u8; 3]>>,
) -> Result<(), String> {
    let Some(i) = settings.export_user_presets.iter().position(|p| p.name.eq_ignore_ascii_case(old)) else {
        return Err(if export::is_builtin_preset(old) || settings.export_presets.iter().any(|p| p.name == old) {
            format!("\"{old}\" is built in - duplicate it to make your own")
        } else {
            format!("no preset called \"{old}\"")
        });
    };
    if let Some(why) = name_refusal(settings, name, Some(old)) {
        return Err(why);
    }
    let p = &mut settings.export_user_presets[i];
    let old_name = std::mem::replace(&mut p.name, name.trim().to_string());
    if let Some(icon) = icon {
        p.icon = icon.to_string();
    }
    if let Some(c) = color {
        p.color = c;
    }
    let new_name = p.name.clone();
    if settings.last_export == Some(ExportPresetRef::Preset(old_name)) {
        settings.last_export = Some(ExportPresetRef::Preset(new_name));
    }
    Ok(())
}

/// Delete the user preset `name`. Built-ins and platform tiles refuse.
pub fn delete_user_preset(settings: &mut Settings, name: &str) -> Result<(), String> {
    let before = settings.export_user_presets.len();
    settings.export_user_presets.retain(|p| !p.name.eq_ignore_ascii_case(name));
    if settings.export_user_presets.len() == before {
        return Err(if export::is_builtin_preset(name) || settings.export_presets.iter().any(|p| p.name == name) {
            format!("\"{name}\" is built in and can't be deleted")
        } else {
            format!("no preset called \"{name}\"")
        });
    }
    Ok(())
}

/// Copy any preset (built-in, platform or user) into the user's own as "<name> copy" (", 2", …).
pub fn duplicate_preset(settings: &mut Settings, name: &str) -> Result<String, String> {
    let mut p = settings.export_preset(name).ok_or_else(|| format!("no preset called \"{name}\""))?;
    let base = format!("{} copy", p.name);
    let mut n = base.clone();
    let mut k = 2;
    while name_refusal(settings, &n, None).is_some() {
        n = format!("{base} {k}");
        k += 1;
    }
    p.name = n.clone();
    if p.icon.is_empty() {
        p.icon = brand_icons::guess(name).join(",");
    }
    settings.export_user_presets.push(p);
    Ok(n)
}

/// `ExportOptions.range` for the checkbox state: the project's in/out points (either end defaulting
/// to the timeline's edge), None when off or degenerate.
pub fn export_range(project: &Project, on: bool) -> Option<(f64, f64)> {
    if !on || (project.in_point.is_none() && project.out_point.is_none()) {
        return None;
    }
    let a = project.in_point.unwrap_or(0.0).max(0.0);
    let b = project.out_point.unwrap_or_else(|| project.duration());
    (b > a).then_some((a, b))
}

/// What the export renders: `export_range`, stopped at the Max length unless the panel includes the rest.
pub fn export_span(project: &Project, state: &ExportUi) -> Option<(f64, f64)> {
    let r = export_range(project, state.range);
    if state.overflow {
        r
    } else {
        project.limit_range(r)
    }
}

/// The lossless `-c copy` path cuts whole clips of one source; a range or a frame rate change needs a
/// re-encode.
pub fn lossless_allowed(state: &ExportUi, project: &Project) -> bool {
    !state.range
        && export_span(project, state).is_none()
        && out_fps(state.fps, project).is_none() && export::lossless_segments(project).is_some()
}

/// Everything `pick_and_build` does after the save dialog - pure, so the range/preset wiring is
/// testable without one.
pub fn build_choice(
    state: &ExportUi,
    project: &Project,
    settings: &Settings,
    out_path: std::path::PathBuf,
) -> ExportChoice {
    let size = preset_size(project.width, project.height, &state.preset, state.custom);
    let ext = out_path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let last = match &state.tile {
        Some(name) => ExportPresetRef::Preset(name.clone()),
        None => {
            let (width, height) = size.unwrap_or((0, 0));
            ExportPresetRef::Custom { ext, width, height }
        }
    };
    let lossless = state.lossless && lossless_allowed(state, project);
    ExportChoice {
        opts: ExportOptions {
            out_path,
            encoder: settings.encoder.clone(),
            crf: settings.crf,
            preset: export::effective_preset(&settings.preset, &settings.export_speed),
            backend: Backend::parse(&settings.decoder),
            out_size: size,
            scaler: settings.export_scaler.clone(),
            // placeholder: `App::start_export_choice` swaps in `App::export_frames()` (GPU when on)
            frames: crate::engine::export::FrameSource::Cpu,
            metadata: if state.metadata_on { state.metadata.clone() } else { Vec::new() },
            range: export_span(project, state),
            loudnorm: settings.loudnorm,
            letterbox: state.letterbox && state.tile.is_some(),
            // ---- ws:pro-schema ----
            // ws:export-audio-tracks
            audio: export::AudioLayout::parse(&settings.export_audio),
            stem_by: export::StemBy::parse(&settings.export_stem_by),
            overlay: None,
            fps: out_fps(state.fps, project),
            target_bytes: settings.export_target_bytes(),
        },
        lossless,
        use_project_bg: state.use_project_bg,
        last,
    }
}

const RES_PRESETS: [(&str, &str); 6] = [
    ("project", "Project"),
    ("720", "720p"),
    ("1080", "1080p"),
    ("1440", "1440p"),
    ("2160", "4K"),
    ("custom", "Custom"),
];
/// ffmpeg `scale` flags offered anywhere an export resizes (also used by the Export Frame window).
pub const SCALERS: [(&str, &str); 6] = [
    ("neighbor", "Nearest"),
    ("bilinear", "Bilinear"),
    ("bicubic", "Bicubic"),
    ("lanczos", "Lanczos"),
    ("area", "Area"),
    ("spline", "Spline"),
];
/// The Format row: every container the exporter writes, video first.
const FORMATS: [(&str, &str); 10] = [
    ("mp4", "MP4"),
    ("mov", "MOV"),
    ("mkv", "MKV"),
    ("webm", "WebM"),
    ("avi", "AVI"),
    ("gif", "GIF"),
    ("mp3", "MP3"),
    ("wav", "WAV"),
    ("m4a", "M4A"),
    ("flac", "FLAC"),
];
const SPEEDS: [(&str, &str, &str); 3] = [
    ("speed", "Fast", "Encodes quickest; the file is a little bigger."),
    ("balanced", "Balanced", "A good mix of encode time and file size."),
    ("small", "Small file", "Takes longest; the smallest file at the same quality."),
];

/// The Frame rate row's fixed choices (beside Project and Custom).
const FPS_CHOICES: [f64; 5] = [24.0, 25.0, 30.0, 50.0, 60.0];
/// Glyph icons a user preset can pick (beside the brand marks).
const PICK_GLYPHS: [&str; 6] = ["bookmark", "star", "bolt", "clapperboard", "landscape", "portrait"];

/// Output size for a resolution preset: None = project size. Height presets derive the width from the
/// project aspect (rounded to even).
fn preset_size(pw: u32, ph: u32, preset: &str, custom: (u32, u32)) -> Option<(u32, u32)> {
    match preset {
        "project" | "" => None,
        "custom" => Some((custom.0.max(16), custom.1.max(16))),
        h => {
            let h: u32 = h.parse().ok()?;
            let w = (pw as f64 / ph.max(1) as f64 * h as f64).round() as u32;
            let out = (w + (w & 1), h);
            if out == (pw, ph) {
                None
            } else {
                Some(out)
            }
        }
    }
}

/// "58.2 MB" / "1.3 GB" - 1 decimal place, switching to GB at 1000 MB.
fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.1} MB")
    }
}

/// What a card click / its right-click menu asked for.
enum CardEv {
    Apply(ExportPreset),
    Duplicate(String),
    Edit(ExportPreset),
    Update(String),
    Delete(String),
    New,
}

/// Which group a card belongs to (what its menu may do).
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Builtin,
    Platform,
    User,
}

/// Test-only: remember a widget rect so headless tests can find the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, rect: egui::Rect) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("export_ui", name.to_string())), rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _rect: egui::Rect) {}
/// Test-only: remember whether a label senses clicks or drags (an interactive / selectable widget).
#[cfg(test)]
fn mark_sense(ui: &egui::Ui, name: &str, r: &egui::Response) {
    let live = r.sense.senses_click() || r.sense.senses_drag();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("export_ui_sense", name.to_string())), live));
}
#[cfg(not(test))]
fn mark_sense(_ui: &egui::Ui, _name: &str, _r: &egui::Response) {}

/// ---- ws:pages ---- The Export pane's body (`Pane::Export`, was the Export window): preset cards,
/// the project settings, the Video / Audio / Advanced sections and the pinned output bar with Export… /
/// Add to Queue (both still ask for the file with the save dialog). `project` is the live project (its
/// settings are edited here, `undo` snapshots it first); `main` is what gets exported when that differs
/// (the main timeline while a sequence is open). Returns (Some((choice, queued)) when the user confirmed
/// an export this frame, the project was edited). The export options are settings, saved as they change.
#[allow(clippy::too_many_arguments)]
pub fn body(
    ui: &mut egui::Ui,
    state: &mut ExportUi,
    project: &mut Project,
    main: Option<&Project>,
    settings: &mut Settings,
    encoders: &[String],
    exporting: bool,
    undo: &mut dyn FnMut(&Project),
) -> (Option<(ExportChoice, bool)>, bool) {
    if state.preset.is_empty() {
        init_from_settings(state, settings, main.unwrap_or(project));
    }
    let mut out = None;
    let mut save = false;
    let mut edited = false;
    // the output bar first, pinned to the bottom; the presets and sections scroll above it
    egui::TopBottomPanel::bottom("export_bar")
        .resizable(false)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 2, right: 2, top: 8, bottom: 4 }))
        .show_separator_line(true)
        .show_inside(ui, |ui| {
            out = output_bar(ui, state, main.unwrap_or(project), settings, exporting, &mut save);
        });
    egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("export_body").auto_shrink([false, false]).show(ui, |ui| {
            save |= presets_ui(ui, state, main.unwrap_or(project), settings);
            ui.add_space(6.0);
            ui.separator();
            edited |= crate::ui::inspector::project_section(ui, project, settings, undo);
            edited |= crate::ui::inspector::rescale_prompt(ui, project, undo);
            save |= project_extras(ui, state, main.unwrap_or(project), settings);
            ui.separator();
            let ex = main.unwrap_or(project);
            save |= video_section(ui, state, ex, settings);
            save |= audio_section(ui, state, settings);
            save |= advanced_section(ui, state, ex, settings, encoders);
            ui.add_space(4.0);
        });
    });
    if save {
        settings.save();
    }
    (out, edited)
}

/// Under the project settings: whether the export keeps the project background, and the captions
/// burn-in. True when settings changed.
fn project_extras(ui: &mut egui::Ui, state: &mut ExportUi, project: &Project, settings: &mut Settings) -> bool {
    let mut save = false;
    ui.checkbox(&mut state.use_project_bg, "Use project background").on_hover_text(
        "Off: the export renders on black. On: the Background above is baked in (checkerboard as grey \
         squares; real transparency needs an alpha-capable codec).",
    );
    // follows the preview's subtitle toggle until ticked here
    if !project.subtitles.is_empty() {
        let mut caps = settings.export_captions.unwrap_or(project.show_subtitles);
        if ui
            .checkbox(&mut caps, "Burn in captions")
            .on_hover_text("Draws the subtitles into the picture. Off: the export has none (export an .srt from Captions instead).")
            .changed()
        {
            settings.export_captions = Some(caps);
            save = true;
        }
    }
    save
}

// ---- presets ----

/// The card grid: built-ins, platform tiles, the user's own (+ the "save current" card), and the
/// inline save / edit form. True when settings changed.
fn presets_ui(ui: &mut egui::Ui, state: &mut ExportUi, project: &Project, settings: &mut Settings) -> bool {
    let builtins = export::builtin_export_presets();
    let platform = settings.export_presets.clone();
    let user = settings.export_user_presets.clone();
    let mut ev: Option<CardEv> = None;
    let groups: [(&str, Group, &[ExportPreset]); 3] = [
        ("Presets", Group::Builtin, &builtins),
        ("Platforms", Group::Platform, &platform),
        ("My presets", Group::User, &user),
    ];
    for (title, group, list) in groups {
        if group == Group::Platform && list.is_empty() {
            continue;
        }
        group_label(ui, title);
        let mut cards: Vec<Option<&ExportPreset>> = list.iter().map(Some).collect();
        if group == Group::User {
            cards.push(None); // the "+ Save current settings" card
        }
        let min_w = if group == Group::Platform { TILE_W } else { 150.0 };
        card_grid(ui, &cards, min_w, |ui, p, w| match p {
            Some(p) => {
                let on = state.tile.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(&p.name));
                let edited = on && !preset_matches(p, state, settings, project);
                if let Some(e) = preset_card(ui, p, group, on, edited, w) {
                    ev = Some(e);
                }
            }
            None => {
                if new_card(ui, w).clicked() {
                    ev = Some(CardEv::New);
                }
            }
        });
    }
    let mut changed = false;
    match ev {
        Some(CardEv::Apply(p)) => {
            apply_preset(state, settings, &p);
            changed = true;
        }
        Some(CardEv::Duplicate(name)) => {
            if let Ok(n) = duplicate_preset(settings, &name) {
                state.form = Some(PresetForm {
                    editing: Some(n.clone()),
                    name: n.clone(),
                    icon: settings.export_preset(&n).map(|p| p.icon).unwrap_or_default(),
                    ..Default::default()
                });
                changed = true;
            }
        }
        Some(CardEv::Edit(p)) => {
            state.form = Some(PresetForm {
                editing: Some(p.name.clone()),
                name: p.name,
                icon: p.icon,
                color: p.color,
                error: String::new(),
            });
        }
        Some(CardEv::Update(name)) => {
            if let Some(i) = settings.export_user_presets.iter().position(|p| p.name == name) {
                let old = settings.export_user_presets[i].clone();
                let mut p = current_as_preset(state, settings, project, &name);
                (p.icon, p.color) = (old.icon, old.color);
                settings.export_user_presets[i] = p;
                state.tile = Some(name);
                changed = true;
            }
        }
        Some(CardEv::Delete(name)) => {
            changed |= delete_user_preset(settings, &name).is_ok();
            if state.tile.as_deref() == Some(name.as_str()) {
                state.tile = None;
            }
        }
        Some(CardEv::New) => {
            let mut n = 1;
            while settings.export_preset(&format!("My preset {n}")).is_some() {
                n += 1;
            }
            state.form =
                Some(PresetForm { name: format!("My preset {n}"), icon: "bookmark".into(), ..Default::default() });
        }
        None => {}
    }
    changed |= preset_form(ui, state, project, settings);
    changed
}

/// A small dim group heading above a card row.
fn group_label(ui: &mut egui::Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(title).small().color(ui.visuals().weak_text_color()));
}

/// Lay `cards` out in as many equal columns as fit (cards at least `min_w` pt wide).
fn card_grid<T>(ui: &mut egui::Ui, cards: &[T], min_w: f32, mut draw: impl FnMut(&mut egui::Ui, &T, f32)) {
    let gap = 6.0;
    let avail = ui.available_width().max(min_w);
    let cols = (((avail + gap) / (min_w + gap)).floor() as usize).clamp(1, 8);
    let w = ((avail - gap * (cols - 1) as f32) / cols as f32).floor();
    for row in cards.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for c in row {
                draw(ui, c, w);
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.y);
    }
}

/// A wide card (built-ins, the user's own) and a platform tile (small, near square).
const CARD_H: f32 = 44.0;
const TILE_W: f32 = 72.0;
const TILE_H: f32 = 62.0;

/// Card / tile colours in the app's own widget style: (fill, text). Selected = the standard selection.
fn card_colors(vis: &egui::Visuals, on: bool, hovered: bool) -> (Color32, Color32) {
    if on {
        (vis.selection.bg_fill, vis.selection.stroke.color)
    } else if hovered {
        (vis.widgets.hovered.weak_bg_fill, vis.text_color())
    } else {
        (vis.widgets.inactive.weak_bg_fill, vis.text_color())
    }
}

/// A preset's icon keys: its own list, else guessed from the name.
fn icon_keys(p: &ExportPreset) -> Vec<String> {
    if p.icon.is_empty() {
        brand_icons::guess(&p.name).into_iter().map(String::from).collect()
    } else {
        p.icon.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect()
    }
}

/// Paint up to 3 icons of `size`, side by side from `x` on the line `cy`, in `color`; returns the x
/// after the last one.
fn paint_icons(painter: &egui::Painter, keys: &[String], mut x: f32, cy: f32, size: f32, color: Color32) -> f32 {
    let one = [String::from("bookmark")];
    let keys = if keys.is_empty() { &one[..] } else { keys };
    for k in keys.iter().take(3) {
        let r = egui::Rect::from_center_size(egui::pos2(x + size / 2.0, cy), egui::vec2(size, size));
        if !brand_icons::paint(painter, r, k, color) {
            draw_glyph(painter, r.expand(4.0), Glyph::from_name(k).unwrap_or(Glyph::Bookmark), color);
        }
        x += size + 3.0;
    }
    x
}

/// The tile's short name: the first part of "Shorts / Reels / TikTok" (the icons show the rest).
fn short_name(name: &str) -> &str {
    name.split(" / ").next().unwrap_or(name).trim()
}

/// One preset card (or platform tile when `group` is Platform): icon(s), name (+ size and format on a
/// card), the standard selection fill when selected. Returns what a click or its right-click menu asked for.
fn preset_card(ui: &mut egui::Ui, p: &ExportPreset, group: Group, on: bool, edited: bool, w: f32) -> Option<CardEv> {
    let tile = group == Group::Platform;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, if tile { TILE_H } else { CARD_H }), egui::Sense::click());
    mark(ui, &format!("card:{}", p.name), rect);
    let vis = ui.visuals().clone();
    let (fill, text) = card_colors(&vis, on, r.hovered());
    let weak = if on { text.gamma_multiply(0.8) } else { vis.weak_text_color() };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), fill);
    let keys = icon_keys(p);
    let small = egui::TextStyle::Small.resolve(ui.style());
    let layout = |s: &str, font: egui::FontId, color: Color32, max_w: f32, rows: usize, center: bool| {
        let mut job = egui::text::LayoutJob::simple(s.to_owned(), font, color, max_w);
        job.wrap.max_rows = rows;
        job.wrap.overflow_character = Some('\u{2026}');
        if center {
            job.halign = egui::Align::Center;
        }
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let size_txt = if p.width == 0 || p.height == 0 {
        "Project size".to_string()
    } else {
        format!("{}\u{d7}{}", p.width, p.height)
    };
    if tile {
        // icons on top, the short name under them (two lines at most), both centred
        let n = keys.len().clamp(1, 3) as f32;
        let size = if n > 1.0 { 16.0 } else { 20.0 };
        let row_w = n * size + (n - 1.0) * 3.0;
        paint_icons(&painter, &keys, rect.center().x - row_w / 2.0, rect.top() + 8.0 + size / 2.0, size, text);
        let mut name = short_name(&p.name).to_string();
        if edited {
            name.push('*');
        }
        let g = layout(&name, small, text, rect.width() - 8.0, 2, true);
        painter.galley(egui::pos2(rect.center().x, rect.top() + 14.0 + size), g, text);
    } else {
        let x = paint_icons(&painter, &keys, rect.left() + 10.0, rect.center().y, 18.0, text);
        let tx = x.max(rect.left() + 34.0) + 4.0;
        let max_w = (rect.right() - 8.0 - tx).max(10.0);
        let name_g = layout(&p.name, egui::TextStyle::Body.resolve(ui.style()), text, max_w, 1, false);
        let sub = format!("{size_txt} \u{b7} {}{}", p.ext.to_uppercase(), if edited { " \u{b7} edited" } else { "" });
        let sub_g = layout(&sub, small, weak, max_w, 1, false);
        let total = name_g.size().y + 1.0 + sub_g.size().y;
        let top = rect.center().y - total / 2.0;
        painter.galley(egui::pos2(tx, top + total - sub_g.size().y), sub_g, weak);
        painter.galley(egui::pos2(tx, top), name_g, text);
    }

    let quality = export::quality_percent_from_crf(p.crf);
    let what = match p.name.as_str() {
        export::PRESET_MAX_QUALITY => {
            "Exactly your project's size and frame rate, at the best quality.\nA big file: 3-4x the default quality's size.\n"
        }
        export::PRESET_FAST_PREVIEW => {
            "Your project's size and frame rate, encoded as fast as possible to check an edit.\n"
        }
        _ if group == Group::Platform && !p.stretch => "Fits the picture inside (black bars, never stretched).\n",
        _ => "",
    };
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!(
        "{}\n{what}{size_txt} \u{b7} {}{} \u{b7} quality {quality}%{}{}{}{}\nRight-click for more.",
        p.name,
        p.ext.to_uppercase(),
        if p.fps > 0.0 { format!(" \u{b7} {} fps", fps_text(p.fps)) } else { String::new() },
        if p.encoder.is_empty() || p.encoder == "auto" { String::new() } else { format!(" \u{b7} {}", p.encoder) },
        if p.speed.is_empty() { String::new() } else { format!(" \u{b7} {} encode", speed_label(&p.speed)) },
        if p.loudnorm { " \u{b7} \u{2212}14 LUFS" } else { "" },
        if edited { "\nEdited since it was applied." } else { "" },
    ));
    let mut ev = r.clicked().then(|| CardEv::Apply(p.clone()));
    menu::context(&r, |ui| {
        if menu::row(ui, Some(Glyph::Play), "Use this preset", "").clicked() {
            ev = Some(CardEv::Apply(p.clone()));
        }
        if menu::row(ui, Some(Glyph::Copy), "Duplicate", "").clicked() {
            ev = Some(CardEv::Duplicate(p.name.clone()));
        }
        if group == Group::User {
            if menu::row(ui, Some(Glyph::Pencil), "Rename / icon\u{2026}", "").clicked() {
                ev = Some(CardEv::Edit(p.clone()));
            }
            if menu::row(ui, Some(Glyph::FloppyDisk), "Update with current settings", "").clicked() {
                ev = Some(CardEv::Update(p.name.clone()));
            }
            ui.separator();
            if menu::row(ui, Some(Glyph::Cross), "Delete", "").clicked() {
                ev = Some(CardEv::Delete(p.name.clone()));
            }
        } else {
            ui.separator();
            ui.add_enabled_ui(false, |ui| menu::row(ui, None, "Built in", ""))
                .inner
                .on_disabled_hover_text("Duplicate it to change it.");
        }
    });
    ev
}

/// The "+ Save current settings" card at the end of My presets.
fn new_card(ui: &mut egui::Ui, w: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, CARD_H), egui::Sense::click());
    mark(ui, "card:+new", rect);
    let vis = ui.visuals();
    let c = if r.hovered() { vis.strong_text_color() } else { vis.weak_text_color() };
    let stroke = Stroke::new(1.0, vis.widgets.noninteractive.bg_stroke.color);
    ui.painter().rect_stroke(rect.shrink(0.5), CornerRadius::same(4), stroke, StrokeKind::Inside);
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+  Save current settings\u{2026}", font, c);
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(
        "Keep everything set below (size, frame rate, format, quality, speed, audio) as your own preset card.",
    )
}

/// The inline save / edit form under the cards. True when settings changed.
fn preset_form(ui: &mut egui::Ui, state: &mut ExportUi, project: &Project, settings: &mut Settings) -> bool {
    let Some(mut f) = state.form.take() else { return false };
    let mut keep = true;
    let mut changed = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.strong(if f.editing.is_some() { "Edit preset" } else { "Save current settings as a preset" });
        let mut submit = false;
        egui::Grid::new("export_preset_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(200.0));
            submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
            ui.label("Icon");
            ui.horizontal_wrapped(|ui| {
                let text = ui.visuals().text_color();
                let accent = ui.visuals().selection.bg_fill;
                for key in brand_icons::keys().chain(PICK_GLYPHS) {
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
                    let on = f.icon == key;
                    if on || r.hovered() {
                        let s = Stroke::new(if on { 2.0 } else { 1.0 }, if on { accent } else { text });
                        ui.painter().rect_stroke(rect, CornerRadius::same(4), s, StrokeKind::Inside);
                    }
                    if !brand_icons::paint(ui.painter(), rect.shrink(5.0), key, text) {
                        draw_glyph(ui.painter(), rect, Glyph::from_name(key).unwrap_or(Glyph::Bookmark), text);
                    }
                    if r.clicked() {
                        f.icon = key.to_string();
                    }
                }
            });
            ui.end_row();
        });
        if !f.error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &f.error);
        }
        ui.horizontal(|ui| {
            submit |= ui.add(accent_button(ui, "Save")).clicked();
            if ui.button("Cancel").clicked() {
                keep = false;
            }
        });
        if submit {
            let res = match &f.editing {
                Some(old) => edit_user_preset(settings, old, &f.name, Some(&f.icon), Some(f.color)),
                None => {
                    let mut p = current_as_preset(state, settings, project, &f.name);
                    (p.icon, p.color) = (f.icon.clone(), f.color);
                    save_user_preset(settings, p, false)
                }
            };
            match res {
                Ok(()) => {
                    // the saved card is the selected one: it describes exactly what is set now
                    if f.editing.is_none() || state.tile.as_deref() == f.editing.as_deref() {
                        state.tile = Some(f.name.trim().to_string());
                    }
                    keep = false;
                    changed = true;
                }
                Err(e) => f.error = e,
            }
        }
    });
    if keep {
        state.form = Some(f);
    }
    changed
}

// ---- sections ----

/// A collapsible section, the Inspector's own (`inspector::section`), so the Export pane looks like
/// every other panel. Open state in `Settings.export_folds["export.<id>"]` (missing = `default_open`).
/// True when the open state changed.
fn section(
    ui: &mut egui::Ui,
    folds: &mut std::collections::BTreeMap<String, bool>,
    id: &str,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let key = format!("export.{id}");
    let before = folds.get(&key).copied();
    let top = ui.cursor().min;
    crate::ui::inspector::section(ui, &key, title, default_open, folds, None, body);
    mark(ui, &format!("sec:{id}"), egui::Rect::from_min_size(top, egui::vec2(ui.available_width(), 18.0)));
    folds.get(&key).copied() != before
}

/// Video: resolution, frame rate, format, quality (+ the lossless cut when it applies).
fn video_section(ui: &mut egui::Ui, state: &mut ExportUi, project: &Project, settings: &mut Settings) -> bool {
    let mut save = false;
    let mut size_changed = false;
    let mut folds = std::mem::take(&mut settings.export_folds);
    let size = preset_size(project.width, project.height, &state.preset, state.custom);
    let (w, h) = size.unwrap_or((project.width, project.height));
    save |= section(ui, &mut folds, "video", "Video", true, |ui| {
        egui::Grid::new("export_video").num_columns(2).spacing([12.0, 7.0]).show(ui, |ui| {
            ui.label("Resolution").on_hover_text("The picture size of the file. Project = the size you edit at.");
            ui.horizontal(|ui| {
                size_changed |= combo(ui, "export_res", &mut state.preset, &RES_PRESETS, Some(90.0));
                if state.preset == "custom" {
                    let (mut cw, mut ch) = state.custom;
                    size_changed |= ui.add(egui::DragValue::new(&mut cw).range(16..=8192)).changed();
                    ui.label("×");
                    size_changed |= ui.add(egui::DragValue::new(&mut ch).range(16..=8192)).changed();
                    state.custom = (cw, ch);
                } else {
                    ui.weak(format!("{w}×{h}"));
                }
            });
            ui.end_row();

            ui.label("Frame rate").on_hover_text("Frames per second in the file. Project = the rate you edit at.");
            ui.horizontal(|ui| save |= fps_row(ui, state, project.fps));
            ui.end_row();

            ui.label("Format");
            let mut ext = default_ext(state).to_string();
            if combo(ui, "export_format", &mut ext, &FORMATS, Some(130.0)) {
                state.ext = ext;
                save = true;
            }
            ui.end_row();

            if !state.lossless {
                ui.label("Quality").on_hover_text("Higher = sharper picture and a bigger file.");
                ui.vertical(|ui| {
                    // the shown percent is cached per stored CRF: 101 slider positions quantize onto 52 CRF
                    // values, so recomputing from CRF every frame snapped a typed "85" back to 84
                    let mut pct = match state.pct_cache {
                        Some((c, p)) if c == settings.crf => p,
                        _ => export::quality_percent_from_crf(settings.crf),
                    };
                    let r = ui.add(egui::Slider::new(&mut pct, 0..=100).suffix("%"));
                    save |= r.drag_stopped() || (r.changed() && !r.dragged());
                    settings.crf = export::crf_from_quality_percent(pct);
                    state.pct_cache = Some((settings.crf, pct));
                    // bands anchored so the default (CRF 18 ~ 65 %, visually lossless for x264) does NOT warn
                    let pct = export::quality_percent_from_crf(settings.crf);
                    let (warning, tip) = match pct {
                        95..=100 => ("Near-lossless", "Files will be very large."),
                        65..=94 => ("", ""),
                        45..=64 => ("Slightly reduced quality", "Fine for most uploads."),
                        25..=44 => ("Medium quality loss", "Good for quick previews."),
                        10..=24 => ("Large quality loss", "The picture will look blocky."),
                        _ => {
                            ("Barely recognizable", "The video will barely be recognizable at this compression level.")
                        }
                    };
                    if !warning.is_empty() {
                        // near-lossless is a note, not a warning
                        let c = if pct >= 95 { ui.visuals().weak_text_color() } else { ui.visuals().warn_fg_color };
                        ui.colored_label(c, egui::RichText::new(warning).small()).on_hover_text(tip);
                    }
                });
                ui.end_row();
            }
        });
        if export::lossless_segments(project).is_some() {
            // ws:export-deliver: a range needs a re-encode, so the -c copy path greys out while it's on
            if state.range {
                state.lossless = false;
            }
            ui.add_enabled(!state.range, egui::Checkbox::new(&mut state.lossless, "Fast lossless cut"))
                .on_hover_text("Instant, no quality loss; cuts snap to keyframes (may shift up to a few frames). Saves in the source's own format.")
                .on_disabled_hover_text("Not with a range: cuts on keyframes can't honour In/Out points");
        } else {
            state.lossless = false;
        }
    });
    settings.export_folds = folds;
    if size_changed {
        save = true;
        // a hand-edited size is no longer "the tile": plain stretch, like every custom export before
        state.tile = None;
        state.letterbox = false;
        let (w, h) = preset_size(project.width, project.height, &state.preset, state.custom)
            .unwrap_or((project.width, project.height));
        settings.export_resolution = if state.preset == "project" { "project".into() } else { format!("{w}x{h}") };
    }
    save
}

/// The Frame rate row: Project / 24 / 25 / 30 / 50 / 60 / Custom (a number). True when it changed.
fn fps_row(ui: &mut egui::Ui, state: &mut ExportUi, project_fps: f64) -> bool {
    let before = state.fps;
    let custom = state.fps > 0.0 && !FPS_CHOICES.iter().any(|f| (f - state.fps).abs() < 1e-3);
    let text = if state.fps <= 0.0 {
        format!("Project \u{b7} {} fps", fps_text(project_fps))
    } else if custom {
        "Custom".to_string()
    } else {
        format!("{} fps", fps_text(state.fps))
    };
    egui::ComboBox::from_id_salt("export_fps").selected_text(text).width(130.0).show_ui(ui, |ui| {
        if ui.selectable_label(state.fps <= 0.0, format!("Project \u{b7} {} fps", fps_text(project_fps))).clicked() {
            state.fps = 0.0;
        }
        for f in FPS_CHOICES {
            if ui.selectable_label((state.fps - f).abs() < 1e-3, format!("{} fps", fps_text(f))).clicked() {
                state.fps = f;
            }
        }
        if ui.selectable_label(custom, "Custom").clicked() && !custom {
            state.fps = 29.97;
        }
    });
    if custom {
        ui.add(egui::DragValue::new(&mut state.fps).range(1.0..=240.0).speed(0.1).suffix(" fps"));
    }
    state.fps != before
}

/// "30" / "29.97".
fn fps_text(fps: f64) -> String {
    let s = format!("{fps:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn speed_label(s: &str) -> &'static str {
    SPEEDS.iter().find(|(k, _, _)| *k == s).map(|(_, l, _)| *l).unwrap_or("Balanced")
}

/// Audio: Mix / Separate tracks / WAV stems, loudness.
fn audio_section(ui: &mut egui::Ui, state: &ExportUi, settings: &mut Settings) -> bool {
    let mut save = false;
    let mut folds = std::mem::take(&mut settings.export_folds);
    let mut changed = false;
    save |= section(ui, &mut folds, "audio", "Audio", true, |ui| {
        // ---- ws:export-audio-tracks ----
        changed |= audio_row(ui, state, settings);
        // ws:export-deliver
        changed |= ui
            .checkbox(&mut settings.loudnorm, "Normalize loudness (−14 LUFS)")
            .on_hover_text("Evens out the loudness to the level YouTube and Spotify play at (ffmpeg loudnorm). Skipped when there is no audio.")
            .changed();
        if export::AUDIO_EXTS.contains(&default_ext(state)) {
            ui.weak("Sound only: this format has no picture.");
        } else if default_ext(state) == "gif" {
            ui.weak("GIF has no sound.");
        }
    });
    settings.export_folds = folds;
    save | changed
}

// ---- ws:player-metadata ----
/// Fill the export's metadata rows from `p`: each field it fills replaces (or adds) that key; its Explorer
/// tags go in as `keywords` (also written as Windows' Tags once the file is out, `export_keywords`).
pub fn apply_meta_preset(rows: &mut Vec<(String, String)>, p: &crate::engine::tags::MetaPreset) {
    let kw = (!p.keywords.trim().is_empty()).then(|| ("keywords".to_string(), p.keywords.trim().to_string()));
    for (k, v) in p.values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.clone())).chain(kw) {
        match rows.iter_mut().find(|(rk, _)| rk.trim() == k) {
            Some(r) => r.1 = v,
            None => rows.push((k, v)),
        }
    }
}

/// The Explorer tags an export with these metadata rows asks for (its `keywords` row).
pub fn export_keywords(on: bool, rows: &[(String, String)]) -> Vec<String> {
    if !on {
        return Vec::new();
    }
    rows.iter().find(|(k, _)| k.trim() == "keywords").map(|(_, v)| crate::engine::tags::split_keywords(v)).unwrap_or_default()
}

/// Advanced: range, metadata, background, burn-ins, scaler / encoder / preset, smart render, GPU note.
fn advanced_section(
    ui: &mut egui::Ui,
    state: &mut ExportUi,
    project: &Project,
    settings: &mut Settings,
    encoders: &[String],
) -> bool {
    let mut save = false;
    let mut folds = std::mem::take(&mut settings.export_folds);
    let has_range = project.in_point.is_some() || project.out_point.is_some();
    if !has_range {
        state.range = false;
    }
    let size = preset_size(project.width, project.height, &state.preset, state.custom);
    let mut body_save = false;
    save |= section(ui, &mut folds, "advanced", "Advanced", false, |ui| {
        // ---- ws:export-deliver: range ----
        ui.add_enabled(has_range, egui::Checkbox::new(&mut state.range, "Export In/Out range only"))
            .on_disabled_hover_text("Set In/Out points on the timeline first (I / O)")
            .on_hover_text("Only the part between the In and Out points is written.");

        if ui
            .checkbox(&mut state.metadata_on, "Write metadata")
            .on_hover_text(
                "Title, artist and other tags in the file. Off: no title, encoder or creation time is written.",
            )
            .changed()
            && state.metadata.is_empty()
        {
            state.metadata = vec![
                ("title".into(), project.name.clone()),
                ("artist".into(), String::new()),
                ("comment".into(), String::new()),
            ];
        }
        if state.metadata_on {
            // ---- ws:player-metadata: a saved preset fills the fields in one click ----
            if !settings.metadata_presets.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    egui::ComboBox::from_id_salt("export_meta_preset").selected_text("Apply preset").show_ui(ui, |ui| {
                        for p in &settings.metadata_presets {
                            if ui.selectable_label(false, &p.name).clicked() {
                                apply_meta_preset(&mut state.metadata, p);
                            }
                        }
                    });
                });
            }
            let mut drop: Option<usize> = None;
            for (i, (k, v)) in state.metadata.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.add(egui::TextEdit::singleline(k).desired_width(70.0).hint_text("key"));
                    ui.add(egui::TextEdit::singleline(v).desired_width(140.0).hint_text("value"));
                    if ui.small_button("\u{2715}").on_hover_text("Remove").clicked() {
                        drop = Some(i);
                    }
                });
            }
            if let Some(i) = drop {
                state.metadata.remove(i);
            }
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                if ui.small_button("+ field").clicked() {
                    state.metadata.push((String::new(), String::new()));
                }
            });
        }
        // burn-in: the social guide (captions sit under the project settings)
        body_save |= ui
            .add_enabled(settings.guide.is_some(), egui::Checkbox::new(&mut settings.export_guide, "Burn in platform guide"))
            .on_hover_text(
                "Draws the Platform Emulator's guide shown on the Preview (right-click the Preview) over every frame, playing, its \
                 progress bar and time following the exported video (the In/Out range when that is what you export). \
                 A Device preview burns in as its UI preview.",
            )
            .on_disabled_hover_text("Show a guide first: right-click the Preview ▸ Platform Emulator ▸ Show Guides on Preview.")
            .changed();

        ui.add_space(4.0);
        egui::Grid::new("export_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Scaler")
                .on_hover_text("How the picture is resized when the export size differs from the project's.");
            ui.horizontal(|ui| {
                body_save |= combo(ui, "export_scaler", &mut settings.export_scaler, &SCALERS, None);
                if size.is_none() {
                    ui.weak("(same size)");
                }
            });
            ui.end_row();

            ui.label("Encoder").on_hover_text(
                "auto = your graphics card's hardware encoder when there is one (fast), else x264 (software).",
            );
            let opts: Vec<(&str, &str)> = std::iter::once(("auto", "auto"))
                .chain(encoder_options(encoders).into_iter().map(|e| (e, e)))
                .collect();
            body_save |= combo(ui, "export_encoder", &mut settings.encoder, &opts, None);
            ui.end_row();

            ui.label("Encoder preset");
            // ws:export-encoders: "auto" follows the Speed choice; a named preset wins over it
            let opts: Vec<(&str, &str)> =
                std::iter::once(("auto", "auto (Speed)")).chain(ENCODER_PRESETS.iter().map(|p| (*p, *p))).collect();
            body_save |= combo(ui, "export_preset", &mut settings.preset, &opts, None);
            ui.end_row();

            ui.label("Smart render");
            body_save |= ui
                .checkbox(&mut settings.export_smart_render, "Experimental")
                .on_hover_text(
                    "Copy the untouched stretches of a single H.264/HEVC source instead of \
                     re-encoding them. Not wired into exports yet - every export is a full encode.",
                )
                .changed();
            ui.end_row();
        });
        if settings.gpu || settings.preview_quality < 100 {
            // Settings ▸ Performance only changes what you watch, never what is written
            ui.weak("ⓘ The export is always full quality").on_hover_text(
                "GPU preview and preview quality (Settings ▸ Performance) change only what you watch, never the exported picture.",
            );
        }
    });
    settings.export_folds = folds;
    save | body_save
}

// ---- output bar ----

/// A filled accent-coloured button (the primary action).
fn accent_button<'a>(ui: &egui::Ui, text: &'a str) -> egui::Button<'a> {
    let accent = ui.visuals().selection.bg_fill;
    egui::Button::new(egui::RichText::new(text).strong().color(crate::ui::tools::on_accent(accent)))
        .fill(accent)
        .min_size(egui::vec2(96.0, 28.0))
}

/// The file name the output bar suggests: "<project>_<preset slug>" / "<project>_edit".
fn suggested_name(state: &ExportUi, project: &Project) -> String {
    let suffix = match state.tile.as_deref() {
        Some(t) if export::is_builtin_preset(t) => {
            if t.eq_ignore_ascii_case(export::PRESET_FAST_PREVIEW) {
                "preview".into()
            } else {
                "edit".into()
            }
        }
        Some(t) => slug(t),
        None => "edit".into(),
    };
    format!("{}_{suffix}", project.name)
}

/// The folder the save dialog opens in: the bar's, else beside the source video.
fn out_folder(state: &ExportUi, project: &Project) -> Option<PathBuf> {
    state
        .out_dir
        .clone()
        .or_else(|| project.source_video.as_ref().and_then(|p| Path::new(p).parent().map(Path::to_path_buf)))
}

/// A row label in the output bar: slightly grey, plain text (no hover, no text selection).
fn bar_label(ui: &mut egui::Ui, text: &str) {
    let c = ui.visuals().text_color().gamma_multiply(0.7);
    let r = ui.add(egui::Label::new(egui::RichText::new(text).color(c)).selectable(false));
    mark_sense(ui, text, &r);
}

/// The pinned bar: summary line, Speed, file name / folder, Export… / Add to Queue.
fn output_bar(
    ui: &mut egui::Ui,
    state: &mut ExportUi,
    project: &Project,
    settings: &mut Settings,
    exporting: bool,
    save: &mut bool,
) -> Option<(ExportChoice, bool)> {
    let mut out = None;
    let size = preset_size(project.width, project.height, &state.preset, state.custom);
    let (w, h) = size.unwrap_or((project.width, project.height));
    let range = export_span(project, state);
    let export_dur = range.map(|(a, b)| b - a).unwrap_or_else(|| project.duration());
    let ext = default_ext(state).to_string();
    // summary: preset · size · fps · duration · estimated size
    let mut parts = vec![];
    if state.lossless {
        parts.push("Lossless cut · same as the source".to_string());
    } else if export::AUDIO_EXTS.contains(&ext.as_str()) {
        parts.push(format!("{} sound only", ext.to_uppercase()));
    } else {
        parts.push(format!("{w}×{h}"));
        parts.push(format!("{} fps", fps_text(out_fps(state.fps, project).unwrap_or(project.fps))));
        if project.color.is_hdr() {
            parts.push(format!("{} 10-bit", project.color.short())); // ws:project-color-fps
        }
    }
    parts.push(duration_text(export_dur));
    if !state.lossless && !export::AUDIO_EXTS.contains(&ext.as_str()) {
        // ws:export-audio-tracks: "Separate tracks in file" adds one stream per stem
        let streams = if export::effective_layout(export::AudioLayout::parse(&settings.export_audio), &ext)
            == export::AudioLayout::Streams
        {
            let span = range.unwrap_or((0.0, project.duration()));
            1 + crate::engine::stems::count(project, export::StemBy::parse(&settings.export_stem_by), span) as u32
        } else {
            1
        };
        parts.push(estimated_size(state, project, settings, (w, h, export_dur, streams), &ext));
    }
    ui.horizontal(|ui| {
        let preset = state.tile.clone().unwrap_or_else(|| "Custom settings".into());
        ui.label(egui::RichText::new(preset).strong());
        ui.label(egui::RichText::new(parts.join(" · ")).strong())
            .on_hover_text("The size is an estimate - the real one depends on the footage.");
    });
    // ---- ws:max-duration: one line when the timeline runs past its Max length ----
    if let (Some(over), Some(max)) = (project.over_by(), project.limit()) {
        let (text, button, tip) = if state.overflow {
            (format!("Includes {} past Max length {}", mmss(over), mmss(max)), "Stop at it", "Export up to the Max length only")
        } else {
            (format!("Stops at Max length {} · {} left out", mmss(max), mmss(over)), "Include it", "Export what runs past the Max length too")
        };
        ui.horizontal(|ui| {
            ui.weak(text);
            let r = ui.small_button(button).on_hover_text(tip);
            mark(ui, "export_overflow", r.rect);
            if r.clicked() {
                state.overflow = !state.overflow;
            }
        });
    }
    ui.add_space(4.0);
    egui::Grid::new("export_bar_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        // ---- ws:export-encoders: the speed/size trade-off (picking one hands the preset back to it) ----
        bar_label(ui, "Speed");
        ui.horizontal(|ui| {
            for (k, label, tip) in SPEEDS {
                if ui.selectable_label(settings.export_speed == k, label).on_hover_text(tip).clicked()
                    && settings.export_speed != k
                {
                    settings.export_speed = k.to_string();
                    settings.preset = "auto".into();
                    *save = true;
                }
            }
        });
        ui.end_row();

        *save |= target_size_row(ui, settings);

        bar_label(ui, "File");
        ui.horizontal(|ui| {
            let ext_w = 44.0;
            let folder = out_folder(state, project);
            let folder_txt = folder
                .as_ref()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "Choose…".into());
            let btn_w = 110.0;
            let name_w = (ui.available_width() - ext_w - btn_w - 16.0).clamp(80.0, 300.0);
            let hint = suggested_name(state, project);
            ui.add(egui::TextEdit::singleline(&mut state.out_name).hint_text(hint).desired_width(name_w))
                .on_hover_text("The file name. Empty: the suggestion shown.");
            bar_label(ui, &format!(".{}", if state.lossless { "same" } else { &ext }));
            let b = egui::Button::new(egui::RichText::new(format!("\u{1F4C1} {folder_txt}"))).truncate();
            let r = ui.add_sized([btn_w, ui.spacing().interact_size.y], b).on_hover_text(match &folder {
                Some(d) => format!("Saves in {}\nClick to pick another folder.", d.display()),
                None => "Pick the folder (or choose one when you export).".into(),
            });
            if r.clicked() {
                let mut d = rfd::FileDialog::new().set_title("Export folder");
                if let Some(dir) = &folder {
                    d = d.set_directory(dir);
                }
                if let Some(dir) = d.pick_folder() {
                    state.out_dir = Some(dir);
                }
            }
        });
        ui.end_row();
    });
    ui.add_space(6.0);
    // ---- ws:project-color-fps: an HDR project can't go to GIF / AVI or an 8-bit encoder ----
    let refusal = export::color_refusal(project.color, &ext, &settings.encoder).filter(|_| !state.lossless);
    if let Some(why) = &refusal {
        ui.colored_label(ui.visuals().error_fg_color, why);
    }
    ui.horizontal(|ui| {
        let can = !exporting && !project.is_empty() && refusal.is_none();
        let r = ui
            .add_enabled(can, accent_button(ui, "Export…"))
            .on_hover_text("Pick where to save, then export now. It runs in the Jobs list while you keep working.");
        mark(ui, "export", r.rect);
        if r.clicked() {
            settings.save();
            if let Some(choice) = pick_and_build(state, project, settings) {
                out = Some((choice, false));
            }
        }
        // ws:export-deliver: queue while one runs (the queue drains in order, one at a time)
        if ui
            .add_enabled(!project.is_empty() && refusal.is_none(), egui::Button::new("Add to Queue").min_size(egui::vec2(0.0, 28.0)))
            .on_hover_text("Pick a file now; it renders after whatever is running or queued.")
            .clicked()
        {
            settings.save();
            if let Some(choice) = pick_and_build(state, project, settings) {
                out = Some((choice, true));
            }
        }
        if exporting {
            ui.weak("export running…");
        }
    });
    out
}

// ---- ws:export-size ----
/// The summary's size: the target in Target size mode, else what `engine::size_probe` measured for
/// these settings, else (while it measures) the formula's guess.
fn estimated_size(
    state: &ExportUi,
    project: &Project,
    settings: &Settings,
    (w, h, dur, streams): (u32, u32, f64, u32),
    ext: &str,
) -> String {
    if let Some(t) = settings.export_target_bytes() {
        return format!("~{}", format_bytes(t));
    }
    let c = build_choice(state, project, settings, format!("estimate.{ext}").into());
    match crate::engine::size_probe::measured(&crate::engine::size_probe::key(&c.opts)) {
        Some(b) => format!("~{}", format_bytes(b)),
        None => {
            let fps = out_fps(state.fps, project).unwrap_or(project.fps);
            format!("~{}…", format_bytes(export::estimate_export_bytes(w, h, fps, dur, settings.crf, streams)))
        }
    }
}

/// The output bar's Target size row: off = Quality decides the size; on = about this many MB.
fn target_size_row(ui: &mut egui::Ui, settings: &mut Settings) -> bool {
    let mut changed = false;
    bar_label(ui, "Target size");
    ui.horizontal(|ui| {
        let mut on = settings.export_target_mb > 0.0;
        if ui
            .checkbox(&mut on, "")
            .on_hover_text("Aim the file at a size instead of a quality. Off: Quality decides the size.")
            .changed()
        {
            settings.export_target_mb = if on { 50.0 } else { 0.0 };
            changed = true;
        }
        if on {
            changed |= ui
                .add(egui::DragValue::new(&mut settings.export_target_mb).range(1.0..=100_000.0).speed(1.0).suffix(" MB"))
                .changed();
        }
    });
    ui.end_row();
    changed
}

// ---- ws:export-audio-tracks ----
/// The container the save dialog will default to (`pick_and_build`'s rule): the Format row's.
fn default_ext(state: &ExportUi) -> &str {
    if state.ext.is_empty() {
        "mp4"
    } else {
        state.ext.as_str()
    }
}

/// The "Audio" row: Mix / Separate tracks in file / WAV stems, and "Split by" when not Mix. Writes
/// `Settings.export_audio` / `export_stem_by`; true when either changed.
fn audio_row(ui: &mut egui::Ui, state: &ExportUi, settings: &mut Settings) -> bool {
    let mut changed = false;
    let multi = export::MULTI_AUDIO_EXTS.contains(&default_ext(state));
    egui::Grid::new("export_audio").num_columns(2).spacing([12.0, 7.0]).show(ui, |ui| {
        ui.label("Tracks");
        ui.horizontal(|ui| {
            let a = &mut settings.export_audio;
            changed |= ui
                .selectable_value(a, "mix".to_string(), "Mix")
                .on_hover_text("One stereo mix of everything.")
                .clicked();
            changed |= ui
                .add_enabled_ui(multi, |ui| ui.selectable_value(a, "streams".to_string(), "Separate tracks in file"))
                .inner
                .on_hover_text(
                    "The mix first (the default track), then one audio track per stem. Only MP4, MOV and MKV \
                     support it - saved as another container, the file gets one mix instead.",
                )
                .on_disabled_hover_text(format!(
                    "{} can't hold several audio tracks - only MP4, MOV and MKV support it",
                    default_ext(state)
                ))
                .clicked();
            changed |= ui
                .selectable_value(a, "stems".to_string(), "WAV stems")
                .on_hover_text(
                    "The file gets the mix; each stem is written as 24-bit WAV in a \"<name> stems\" folder beside it.",
                )
                .clicked();
        });
        ui.end_row();
        if settings.export_audio != "mix" {
            ui.label("Split by");
            const BY: [(&str, &str); 3] = [("track", "Track"), ("bus", "Bus"), ("role", "Role")];
            changed |= combo(ui, "export_stem_by", &mut settings.export_stem_by, &BY, None);
            ui.end_row();
        }
    });
    if settings.export_audio == "streams" && !multi {
        ui.weak("This format gets one mix - pick MP4, MOV or MKV for separate tracks.");
    }
    changed
}

fn init_from_settings(state: &mut ExportUi, _settings: &Settings, project: &Project) {
    // always open at the project's own size: a smaller size picked once used to stick for every export
    state.custom = (project.width, project.height);
    state.preset = "project".into();
}

/// The palette's "Quick export": the save dialog (remembered folder and name), then the Export pane's
/// current settings, without opening the Export page.
pub fn quick_pick(state: &mut ExportUi, project: &Project, settings: &Settings) -> Option<ExportChoice> {
    if state.preset.is_empty() {
        init_from_settings(state, settings, project);
    }
    pick_and_build(state, project, settings)
}

/// Save dialog (same filters as the app's Export, the Format row's first) → ExportChoice.
fn pick_and_build(state: &mut ExportUi, project: &Project, settings: &Settings) -> Option<ExportChoice> {
    let mut d = rfd::FileDialog::new();
    let stem = if state.out_name.trim().is_empty() {
        suggested_name(state, project)
    } else {
        state.out_name.trim().to_string()
    };
    if state.lossless {
        let src = project.source_video.clone().or_else(|| project.assets.first().map(|a| a.path.clone()));
        let ext = src
            .as_ref()
            .and_then(|s| Path::new(s).extension().map(|e| e.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "mp4".into());
        let stem = if state.out_name.trim().is_empty() { format!("{}_cut", project.name) } else { stem };
        d = d
            .add_filter(format!("{} (same container)", ext.to_uppercase()), &[ext.as_str()])
            .set_file_name(format!("{stem}.{ext}"));
    } else {
        // the chosen format's filter leads, so the dialog keeps its extension
        let ext = default_ext(state).to_string();
        const FILTERS: [(&str, &str); 10] = [
            ("MP4 video", "mp4"),
            ("MOV video", "mov"),
            ("MKV video", "mkv"),
            ("WebM video", "webm"),
            ("AVI video", "avi"),
            ("GIF", "gif"),
            ("MP3 audio", "mp3"),
            ("WAV audio", "wav"),
            ("M4A audio", "m4a"),
            ("FLAC audio", "flac"),
        ];
        for (label, e) in FILTERS.iter().filter(|(_, e)| *e == ext).chain(FILTERS.iter().filter(|(_, e)| *e != ext)) {
            d = d.add_filter(*label, &[*e]);
        }
        d = d.add_filter("Any (by extension)", &["*"]).set_file_name(format!("{stem}.{ext}"));
    }
    if let Some(dir) = out_folder(state, project) {
        d = d.set_directory(dir);
    }
    let out_path = d.save_file()?;
    state.out_dir = out_path.parent().map(Path::to_path_buf);
    Some(build_choice(state, project, settings, out_path))
}

/// "Shorts / Reels / TikTok" → "shorts-reels-tiktok": a tile name as a file-name suffix.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            s.push(ch.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    s.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of the Export pane body (the old window's `frame`).
    fn frame(
        ctx: &egui::Context,
        state: &mut ExportUi,
        project: &Project,
        settings: &mut Settings,
    ) -> Option<(ExportChoice, bool)> {
        let mut out = None;
        let _ = ctx.run(Default::default(), |ctx| {
            let mut p = project.clone();
            egui::CentralPanel::default().show(ctx, |ui| {
                out = body(ui, state, &mut p, None, settings, &[], false, &mut |_| {}).0;
            });
        });
        out
    }

    #[test]
    fn preset_sizes() {
        // 16:9 project: 720p derives 1280 wide
        assert_eq!(preset_size(1920, 1080, "720", (0, 0)), Some((1280, 720)));
        assert_eq!(preset_size(1920, 1080, "1440", (0, 0)), Some((2560, 1440)));
        // same as project → None
        assert_eq!(preset_size(1920, 1080, "1080", (0, 0)), None);
        assert_eq!(preset_size(1920, 1080, "project", (0, 0)), None);
        // odd widths are rounded to even: 854.4 → 854
        assert_eq!(preset_size(1280, 720, "480", (0, 0)), Some((854, 480)));
        assert_eq!(preset_size(1920, 1080, "custom", (100, 50)), Some((100, 50)));
    }

    /// Headless: the pane lays out without panicking and returns None with no interaction.
    #[test]
    fn show_headless() {
        let project = Project::new();
        let mut settings = Settings::default();
        let mut state = ExportUi::default();
        let ctx = egui::Context::default();
        for _ in 0..2 {
            assert!(frame(&ctx, &mut state, &project, &mut settings).is_none());
        }
        assert_eq!(state.preset, "project");
    }

    /// ws:pages: 30 idle frames of the Export pane (cards, sections, output bar) request no repaint -
    /// also with Advanced open, a user preset with a brand icon, and the save form open.
    #[test]
    fn assert_no_idle_repaint_export() {
        let project = ranged_project();
        let mut settings = Settings::default();
        let mut state = ExportUi::default();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        for _ in 0..30 {
            assert!(frame(&ctx, &mut state, &project, &mut settings).is_none());
        }
        assert!(!ctx.has_requested_repaint(), "the idle Export pane requested a repaint");
        settings.export_folds.insert("export.advanced".into(), true);
        settings.export_user_presets.push(ExportPreset {
            name: "Mine".into(),
            ext: "mp4".into(),
            icon: "vimeo".into(),
            ..Default::default()
        });
        state.form = Some(PresetForm { name: "x".into(), ..Default::default() });
        for _ in 0..30 {
            assert!(frame(&ctx, &mut state, &project, &mut settings).is_none());
        }
        assert!(!ctx.has_requested_repaint(), "the idle Export pane (all open) requested a repaint");
    }

    // ---- ws:export-deliver ----
    fn ranged_project() -> Project {
        let mut p = Project::new();
        p.width = 1920;
        p.height = 1080;
        p.tracks[0].clips.push(crate::model::Clip::new(1, crate::model::ClipKind::Video, "v", 0.0, 8.0));
        p.in_point = Some(1.0);
        p.out_point = Some(3.0);
        p
    }

    /// The 4 tiles + the sections + the range box lay out across 2 headless frames, and a tile
    /// click's effect (`apply_preset`) lands in state + settings.
    #[test]
    fn show_headless_with_preset_tiles() {
        let project = ranged_project();
        let mut settings = Settings::default();
        assert_eq!(settings.export_presets.len(), 4);
        let mut state = ExportUi::default();
        let ctx = egui::Context::default();
        for _ in 0..2 {
            assert!(frame(&ctx, &mut state, &project, &mut settings).is_none());
        }
        let shorts = settings.export_presets[2].clone();
        apply_preset(&mut state, &mut settings, &shorts);
        assert_eq!(state.custom, (1080, 1920));
        assert_eq!(state.preset, "custom");
        assert!(state.letterbox);
        assert_eq!(settings.crf, export::DEFAULT_CRF);
        assert_eq!(state.tile.as_deref(), Some("Shorts / Reels / TikTok"));
        let c = build_choice(&state, &project, &settings, "x.mp4".into());
        assert_eq!(c.opts.out_size, Some((1080, 1920)));
        assert!(c.opts.letterbox, "a tile fits-and-pads instead of stretching");
        assert_eq!(c.last, ExportPresetRef::Preset(shorts.name));
        assert_eq!(slug("Shorts / Reels / TikTok"), "shorts-reels-tiktok");
    }

    // ---- ws:max-duration ----
    /// By default the export stops at the Max length (the summary shows the clamped length and a note);
    /// "Include it" exports the rest; a lossless cut can't stop mid-clip, so it is off while clamped.
    #[test]
    fn export_stops_at_the_max_length_unless_included() {
        let mut p = ranged_project(); // an 8 s clip
        p.in_point = None;
        p.out_point = None;
        p.max_duration = Some(5.0);
        let settings = Settings::default();
        let mut st = ExportUi::default();
        assert_eq!(build_choice(&st, &p, &settings, "x.mp4".into()).opts.range, Some((0.0, 5.0)));
        st.range = true;
        p.in_point = Some(2.0);
        assert_eq!(export_span(&p, &st), Some((2.0, 5.0)), "an In/Out range is clamped too");
        st.range = false;
        st.lossless = true;
        assert!(!lossless_allowed(&st, &p));
        st.overflow = true;
        assert_eq!(build_choice(&st, &p, &settings, "x.mp4".into()).opts.range, None);
        // the pane lays out with the note
        let ctx = egui::Context::default();
        let mut settings = settings;
        st.overflow = false;
        st.lossless = false;
        frame(&ctx, &mut st, &p, &mut settings);
        assert!(frame(&ctx, &mut st, &p, &mut settings).is_none());
    }

    // ---- ws:export-audio-tracks ----
    #[test]
    fn audio_settings_flow_into_options() {
        let project = ranged_project();
        let mut settings = Settings::default();
        settings.export_audio = "stems".into();
        settings.export_stem_by = "role".into();
        let c = build_choice(&ExportUi::default(), &project, &settings, "x.mp4".into());
        assert_eq!(c.opts.audio, export::AudioLayout::Stems);
        assert_eq!(c.opts.stem_by, export::StemBy::Role);
        // the pane (with its Split by row) still lays out
        let ctx = egui::Context::default();
        let mut st = ExportUi::default();
        assert!(frame(&ctx, &mut st, &project, &mut settings).is_none());
    }

    #[test]
    fn range_checkbox_sets_opts_range_and_disables_lossless() {
        let project = ranged_project();
        let settings = Settings::default();
        let mut state = ExportUi { range: true, lossless: true, ..Default::default() };
        let c = build_choice(&state, &project, &settings, "x.mp4".into());
        assert_eq!(c.opts.range, Some((1.0, 3.0)));
        assert!(!c.lossless, "a range export is never a -c copy");
        assert!(!lossless_allowed(&state, &project));
        assert_eq!(c.last, ExportPresetRef::Custom { ext: "mp4".into(), width: 0, height: 0 });
        state.range = false;
        assert_eq!(build_choice(&state, &project, &settings, "x.mp4".into()).opts.range, None);
        // without in/out points the box is inert even if set
        let plain = Project::new();
        assert_eq!(export_range(&plain, true), None);
        // in only → to the end; a degenerate order → None
        let mut p = ranged_project();
        p.out_point = None;
        assert_eq!(export_range(&p, true), Some((1.0, 8.0)));
        p.out_point = Some(0.5);
        assert_eq!(export_range(&p, true), None);
        // and the pane itself un-ticks lossless while the range box is on
        let mut settings = Settings::default();
        let mut st = ExportUi { range: true, lossless: true, ..Default::default() };
        let ctx = egui::Context::default();
        let _ = frame(&ctx, &mut st, &project, &mut settings);
        assert!(st.range && !st.lossless);
    }

    // ---- export panel redesign ----

    #[test]
    fn builtins_come_first_and_resolve_to_the_project() {
        let settings = Settings::default();
        let all = settings.all_export_presets();
        assert_eq!(all[0].name, export::PRESET_MAX_QUALITY);
        assert_eq!(all[1].name, export::PRESET_FAST_PREVIEW);
        assert_eq!(all[2].name, "YouTube 1080p", "platform tiles follow the built-ins");
        // both keep the project's own size (and so its fps - exports never change the frame rate)
        let mut project = ranged_project();
        project.width = 2560;
        project.height = 1440;
        for (name, fast) in [(export::PRESET_MAX_QUALITY, false), (export::PRESET_FAST_PREVIEW, true)] {
            let p = settings.export_preset(name).unwrap();
            let (mut st, mut s) = (ExportUi::default(), Settings::default());
            apply_preset(&mut st, &mut s, &p);
            let c = build_choice(&st, &project, &s, "x.mp4".into());
            assert_eq!(c.opts.out_size, None, "{name}: the project's size");
            assert!(!c.opts.letterbox);
            assert!(preset_matches(&p, &st, &s, &project));
            if fast {
                assert!(c.opts.crf > 24 && c.opts.encoder == "auto" && c.opts.preset == "speed" && !c.opts.loudnorm);
            } else {
                assert_eq!(c.opts.crf, export::CRF_BEST as u32, "max quality = the slider's 100 %");
                assert_eq!(export::quality_percent_from_crf(c.opts.crf), 100);
                assert_eq!((c.opts.encoder.as_str(), c.opts.preset.as_str()), ("libx264", "small"));
            }
        }
        let max = settings.export_preset(export::PRESET_MAX_QUALITY).unwrap();
        let fast = settings.export_preset(export::PRESET_FAST_PREVIEW).unwrap();
        assert!(max.crf < fast.crf, "max quality has the lower CRF");
    }

    #[test]
    fn builtins_and_platform_tiles_cannot_be_deleted_or_renamed() {
        let mut s = Settings::default();
        for name in [export::PRESET_MAX_QUALITY, export::PRESET_FAST_PREVIEW, "YouTube 1080p"] {
            assert!(delete_user_preset(&mut s, name).is_err(), "{name}");
            assert!(edit_user_preset(&mut s, name, "Mine", None, None).is_err(), "{name}");
        }
        // nor can a user preset take their names
        let p = ExportPreset { name: export::PRESET_FAST_PREVIEW.into(), ..Default::default() };
        assert!(save_user_preset(&mut s, p, true).is_err());
        assert_eq!(s.all_export_presets().len(), 6);
        // duplicating a built-in makes an editable copy
        let n = duplicate_preset(&mut s, export::PRESET_MAX_QUALITY).unwrap();
        assert_eq!(n, format!("{} copy", export::PRESET_MAX_QUALITY));
        assert!(edit_user_preset(&mut s, &n, "Master", Some("star"), Some(Some([1, 2, 3]))).is_ok());
        assert_eq!(s.export_user_presets[0].crf, export::CRF_BEST as u32);
        assert_eq!(s.export_user_presets[0].color, Some([1, 2, 3]));
    }

    #[test]
    fn custom_preset_save_apply_rename_delete_round_trip() {
        let project = ranged_project();
        let mut s = Settings::default();
        let mut st = ExportUi { preset: "720".into(), ext: "webm".into(), ..Default::default() };
        s.crf = 24;
        s.loudnorm = false;
        s.encoder = "h264_nvenc".into();
        s.export_speed = "speed".into();
        s.export_audio = "stems".into();
        let mut p = current_as_preset(&st, &s, &project, "  Vimeo draft ");
        p.icon = "vimeo".into();
        save_user_preset(&mut s, p, false).unwrap();
        let dup = current_as_preset(&st, &s, &project, "vimeo DRAFT");
        assert!(save_user_preset(&mut s, dup, false).is_err(), "names are unique");
        // survives a settings.json round trip, and an old file without the new fields still loads
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let saved = back.export_preset("Vimeo draft").expect("found by name, wherever it lives");
        assert_eq!((saved.width, saved.height, saved.ext.as_str(), saved.crf), (1280, 720, "webm", 24));
        assert_eq!(
            (saved.encoder.as_str(), saved.speed.as_str(), saved.audio.as_str()),
            ("h264_nvenc", "speed", "stems")
        );
        assert!(saved.stretch, "a hand-picked size stretches like it did");
        assert_eq!(back.all_export_presets().last().unwrap().name, "Vimeo draft", "user presets come last");
        let old: ExportPreset =
            serde_json::from_str(r#"{"name":"Old","ext":"mp4","width":1,"height":2,"crf":18,"loudnorm":true}"#)
                .unwrap();
        assert!(old.encoder.is_empty() && old.icon.is_empty() && !old.stretch);
        // apply it on a fresh pane: every setting comes back
        let mut s2 = back.clone();
        s2.crf = 18;
        s2.encoder = "auto".into();
        s2.export_audio = "mix".into();
        st = ExportUi::default();
        apply_preset(&mut st, &mut s2, &saved);
        let c = build_choice(&st, &project, &s2, "x.webm".into());
        assert_eq!(c.opts.out_size, Some((1280, 720)));
        assert!(!c.opts.letterbox);
        assert_eq!(
            (c.opts.crf, c.opts.encoder.as_str(), c.opts.preset.as_str(), c.opts.loudnorm),
            (24, "h264_nvenc", "speed", false)
        );
        assert_eq!(c.opts.audio, export::AudioLayout::Stems);
        assert_eq!(c.last, ExportPresetRef::Preset("Vimeo draft".into()));
        assert!(preset_matches(&saved, &st, &s2, &project));
        s2.crf = 30;
        assert!(!preset_matches(&saved, &st, &s2, &project), "a changed setting shows 'edited'");
        // a rename keeps Quick Export's "last export" pointing at it
        s2.last_export = Some(ExportPresetRef::Preset("Vimeo draft".into()));
        edit_user_preset(&mut s2, "Vimeo draft", "Vimeo", None, None).unwrap();
        assert_eq!(s2.last_export, Some(ExportPresetRef::Preset("Vimeo".into())), "Quick Export follows a rename");
        assert!(s2.export_preset("Vimeo draft").is_none() && s2.export_preset("vimeo").is_some());
        delete_user_preset(&mut s2, "Vimeo").unwrap();
        assert!(s2.export_user_presets.is_empty());
        assert!(delete_user_preset(&mut s2, "Vimeo").is_err());
    }

    #[test]
    fn platform_tiles_reset_encoder_and_speed() {
        let mut s = Settings::default();
        let mut st = ExportUi::default();
        let max = s.export_preset(export::PRESET_MAX_QUALITY).unwrap();
        apply_preset(&mut st, &mut s, &max);
        assert_eq!(s.encoder, "libx264");
        let yt = s.export_preset("YouTube 4K").unwrap();
        apply_preset(&mut st, &mut s, &yt);
        assert_eq!((s.encoder.as_str(), s.export_speed.as_str(), s.crf), ("auto", "balanced", export::DEFAULT_CRF - 2));
        assert_eq!((st.custom, st.letterbox, st.ext.as_str()), ((3840, 2160), true, "mp4"));
    }

    /// The output bar is pinned under the sections: Export…'s rect is below every section header and
    /// card, at the bottom of the pane, whether Advanced is open or closed.
    #[test]
    fn output_bar_sits_below_the_sections() {
        let mut project = ranged_project();
        let mut settings = Settings::default();
        let mut state = ExportUi::default();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let rect = |name: &str| ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("export_ui", name.to_string()))));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 1400.0));
        for open in [false, true] {
            settings.export_folds.insert("export.advanced".into(), open);
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        body(ui, &mut state, &mut project, None, &mut settings, &[], false, &mut |_| {})
                    });
                });
            }
            let export = rect("export").expect("the Export button was drawn");
            for name in ["sec:video", "sec:audio", "sec:advanced", "card:Fast preview", "card:+new"] {
                let r = rect(name).unwrap_or_else(|| panic!("{name} drawn"));
                assert!(export.top() > r.bottom(), "{name} ({r:?}) must be above Export… ({export:?})");
            }
            assert!(export.bottom() > screen.bottom() - 60.0, "the bar is pinned to the bottom: {export:?}");
            // cards come before the sections, the sections in order
            assert!(rect("card:+new").unwrap().bottom() < rect("sec:video").unwrap().top());
            let order: Vec<f32> =
                ["sec:video", "sec:audio", "sec:advanced"].iter().map(|n| rect(n).unwrap().top()).collect();
            assert!(order.windows(2).all(|w| w[0] < w[1]), "Video, Audio, Advanced in that order: {order:?}");
        }
    }

    /// The Frame rate row's choice reaches the exporter, presets carry it, and a rate change rules out
    /// the lossless cut.
    #[test]
    fn frame_rate_flows_into_options_and_presets() {
        let mut project = ranged_project();
        project.fps = 30.0;
        let settings = Settings::default();
        let mut st = ExportUi::default();
        assert_eq!(build_choice(&st, &project, &settings, "x.mp4".into()).opts.fps, None, "project rate");
        st.fps = 30.0;
        assert_eq!(build_choice(&st, &project, &settings, "x.mp4".into()).opts.fps, None, "same as the project");
        st.fps = 60.0;
        assert_eq!(build_choice(&st, &project, &settings, "x.mp4".into()).opts.fps, Some(60.0));
        assert!(!lossless_allowed(&st, &project));
        let p = current_as_preset(&st, &settings, &project, "Sixty");
        assert_eq!(p.fps, 60.0);
        let mut s2 = Settings::default();
        let mut st2 = ExportUi::default();
        apply_preset(&mut st2, &mut s2, &p);
        assert_eq!(build_choice(&st2, &project, &s2, "x.mp4".into()).opts.fps, Some(60.0));
        assert!(preset_matches(&p, &st2, &s2, &project));
        st2.fps = 0.0;
        assert!(!preset_matches(&p, &st2, &s2, &project), "a changed rate shows 'edited'");
        // the pane draws the row (custom rate too) without trouble
        let ctx = egui::Context::default();
        st.fps = 29.97;
        let mut s = Settings::default();
        assert!(frame(&ctx, &mut st, &project, &mut s).is_none());
    }

    /// No em-dash in any string the Export pane shows (its source outside the tests, the preset names).
    #[test]
    fn no_em_dash_in_export_ui_text() {
        let src = include_str!("export_ui.rs");
        let ui_part = &src[..src.find("\nmod tests {").expect("tests module")];
        assert!(!ui_part.contains('\u{2014}'), "an em-dash in export_ui.rs");
        for p in Settings::default().all_export_presets() {
            assert!(!p.name.contains('\u{2014}'), "{}", p.name);
        }
        let exp = include_str!("../engine/export.rs");
        assert!(!exp.contains('\u{2014}'), "an em-dash in engine/export.rs");
        // a stored Quick Export target from before the rename still resolves
        assert!(Settings::default().export_preset(export::PRESET_MAX_QUALITY_OLD).is_some());
    }

    /// The output bar's "Speed" / "File" / extension labels are plain text: no click, no selection.
    #[test]
    fn output_bar_labels_are_not_interactive() {
        let project = ranged_project();
        let mut settings = Settings::default();
        let mut state = ExportUi::default();
        let ctx = egui::Context::default();
        let _ = frame(&ctx, &mut state, &project, &mut settings);
        for name in ["Speed", "File", ".mp4"] {
            let live = ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("export_ui_sense", name.to_string()))));
            assert_eq!(live, Some(false), "{name} must be drawn and not interactive");
        }
    }

    #[test]
    fn section_open_state_persists() {
        // drawing alone stores nothing (the defaults stay the defaults), a stored choice round-trips
        let project = ranged_project();
        let mut settings = Settings::default();
        let mut state = ExportUi::default();
        let ctx = egui::Context::default();
        let _ = frame(&ctx, &mut state, &project, &mut settings);
        assert!(settings.export_folds.is_empty());
        let s = Settings { export_folds: [("advanced".to_string(), true)].into(), ..Default::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.export_folds.get("advanced"), Some(&true));
    }

    /// ws:player-metadata: a preset fills the export's metadata rows (replacing a key that is there),
    /// and its Explorer tags become the `keywords` row the finished file gets as Windows' Tags.
    #[test]
    fn a_metadata_preset_fills_the_export_rows() {
        let mut p = crate::engine::tags::MetaPreset { name: "Ch".into(), keywords: "a; b".into(), ..Default::default() };
        p.values.insert("artist".into(), "Kash".into());
        p.values.insert("genre".into(), "".into());
        let mut rows = vec![("title".to_string(), "My video".to_string()), ("artist".to_string(), String::new())];
        apply_meta_preset(&mut rows, &p);
        assert_eq!(rows, [("title".into(), "My video".into()), ("artist".into(), "Kash".into()), ("keywords".into(), "a; b".into())]);
        assert_eq!(export_keywords(true, &rows), ["a", "b"]);
        assert!(export_keywords(false, &rows).is_empty(), "Write metadata off: nothing");
    }
}
