//! Keyboard shortcuts: a fixed list of actions, default bindings, user overrides (stored in Settings),
//! and per-frame polling. Mouse modifiers (Ctrl+Scroll zoom, Alt+Scroll track height, Shift+Scroll pan)
//! are fixed and not part of this table.

use crate::settings::Settings;
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use std::collections::HashMap;

macro_rules! actions {
    ($($v:ident => $id:literal, $label:literal, $sc:expr;)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Action { $($v),* }
        impl Action {
            pub const ALL: &'static [Action] = &[$(Action::$v),*];
            /// Stable id used in settings.json.
            pub fn id(self) -> &'static str { match self { $(Action::$v => $id),* } }
            pub fn label(self) -> &'static str { match self { $(Action::$v => $label),* } }
            pub fn default_shortcut(self) -> Option<KeyboardShortcut> { match self { $(Action::$v => $sc),* } }
            pub fn from_id(s: &str) -> Option<Action> { Self::ALL.iter().copied().find(|a| a.id() == s) }
        }
    };
}

const fn sc(m: Modifiers, k: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(m, k))
}
const NONE: Modifiers = Modifiers::NONE;
const CTRL: Modifiers = Modifiers::CTRL;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: false };
const CTRL_ALT: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: false };

/// Canonical form so `Ctrl` and `Command` (egui sets both on Windows) compare equal.
fn canon(ks: &KeyboardShortcut) -> (bool, bool, bool, Key) {
    (ks.modifiers.ctrl || ks.modifiers.command, ks.modifiers.shift, ks.modifiers.alt, ks.logical_key)
}
fn same(a: Option<KeyboardShortcut>, b: Option<KeyboardShortcut>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => canon(&a) == canon(&b),
        _ => false,
    }
}

actions! {
    NewProject => "new_project", "New Project", sc(CTRL, Key::N);
    OpenFile => "open", "Open Video / Media…", sc(CTRL, Key::O);
    OpenProject => "open_project", "Open Project…", sc(CTRL_SHIFT, Key::O);
    Save => "save", "Save (project, or overwrite opened video)", sc(CTRL, Key::S);
    SaveProjectAs => "save_project_as", "Save Project As…", sc(CTRL_SHIFT, Key::S);
    ExportVideo => "export", "Export Video As…", sc(CTRL, Key::E);
    ExportLossless => "export_lossless", "Fast Lossless Cut…", sc(CTRL_ALT, Key::E);
    ExportXml => "export_xml", "Export Premiere / Resolve XML…", sc(CTRL_SHIFT, Key::E);
    ImportMedia => "import", "Import Media…", sc(CTRL, Key::I);
    Settings => "settings", "Settings…", sc(CTRL, Key::Comma);
    Undo => "undo", "Undo", sc(CTRL, Key::Z);
    Redo => "redo", "Redo", sc(CTRL_SHIFT, Key::Z);
    PlayPause => "play_pause", "Play / Pause", sc(NONE, Key::Space);
    Stop => "stop", "Stop", sc(NONE, Key::K);
    StepBack => "step_back", "Step Back 1 Frame", sc(NONE, Key::ArrowLeft);
    StepForward => "step_fwd", "Step Forward 1 Frame", sc(NONE, Key::ArrowRight);
    GoStart => "go_start", "Go to Start", sc(NONE, Key::Home);
    GoEnd => "go_end", "Go to End", sc(NONE, Key::End);
    PrevCut => "prev_cut", "Previous Cut", sc(NONE, Key::ArrowUp);
    NextCut => "next_cut", "Next Cut", sc(NONE, Key::ArrowDown);
    Split => "split", "Split at Playhead", sc(CTRL, Key::B);
    Delete => "delete", "Delete", sc(NONE, Key::Delete);
    RippleDelete => "ripple_delete", "Ripple Delete", sc(SHIFT, Key::Delete);
    SelectAll => "select_all", "Select All", sc(CTRL, Key::A);
    Deselect => "deselect", "Deselect All", sc(CTRL_SHIFT, Key::A);
    MarkIn => "mark_in", "Mark In", sc(NONE, Key::I);
    MarkOut => "mark_out", "Mark Out", sc(NONE, Key::O);
    ClearInOut => "clear_in_out", "Clear In / Out", sc(ALT, Key::X);
    TrimToInOut => "trim_in_out", "Trim to In / Out", sc(CTRL, Key::T);
    RippleDeleteInOut => "ripple_in_out", "Ripple Delete In / Out", sc(CTRL_SHIFT, Key::Delete);
    AddText => "add_text", "Add Text", sc(NONE, Key::T);
    ZoomIn => "zoom_in", "Zoom In", sc(CTRL, Key::Equals);
    ZoomOut => "zoom_out", "Zoom Out", sc(CTRL, Key::Minus);
    ZoomFit => "zoom_fit", "Zoom to Fit", sc(SHIFT, Key::Z);
    LinkToggle => "link", "Link / Unlink", sc(CTRL, Key::L);
    ToggleEnabled => "toggle_enabled", "Enable / Disable Clip", sc(NONE, Key::D);
    NudgeLeft => "nudge_left", "Nudge Left 1 Frame", sc(NONE, Key::Comma);
    NudgeRight => "nudge_right", "Nudge Right 1 Frame", sc(NONE, Key::Period);
    ToggleSnap => "snap", "Toggle Snapping", sc(NONE, Key::N);
    AddVideoTrack => "add_video_track", "Add Video Track", None;
    AddAudioTrack => "add_audio_track", "Add Audio Track", None;
    ToggleLibrary => "toggle_library", "Show / Hide Library", sc(CTRL, Key::Num1);
    ToggleInspector => "toggle_inspector", "Show / Hide Inspector", sc(CTRL, Key::Num2);
    ToggleEffects => "toggle_effects", "Show / Hide Effects", sc(CTRL, Key::Num3);
    ToggleTransitions => "toggle_transitions", "Show / Hide Transitions", sc(CTRL, Key::Num4);
    ToggleCurves => "toggle_curves", "Show / Hide Curve Editor", sc(CTRL, Key::Num5);
    ToggleSubtitles => "toggle_subtitles", "Show / Hide Subtitles", sc(CTRL, Key::Num6);
    Retime => "retime", "Speed / Retime…", sc(CTRL, Key::R);
    FreezeFrame => "freeze", "Freeze Frame at Playhead", sc(SHIFT, Key::R);
    Fullscreen => "fullscreen", "Fullscreen Playback", sc(NONE, Key::F11);
    AddTransition => "add_transition", "Add Transition at Selected Cut", sc(CTRL_SHIFT, Key::T);
    AddSubtitle => "add_subtitle", "Add Subtitle at Playhead", sc(ALT, Key::S);
    TogglePlanner => "toggle_planner", "Show / Hide Planner", sc(CTRL, Key::Num7);
    AutoCut => "auto_cut", "Auto-cut (silence) Panel", sc(CTRL_ALT, Key::A);
    NestSequence => "nest", "Nest Selection into a New Sequence", sc(ALT, Key::N);
    OpenParentSequence => "parent_sequence", "Back to Parent Timeline", sc(ALT, Key::ArrowUp);
    SaveTemplate => "save_template", "Save Selection as Template…", None;
    ApplyFlow => "flow", "Flow Motion Between Selected Clips", None;
}

pub struct Hotkeys {
    map: HashMap<Action, Option<KeyboardShortcut>>,
}

impl Hotkeys {
    pub fn defaults() -> Self {
        Self { map: Action::ALL.iter().map(|&a| (a, a.default_shortcut())).collect() }
    }
    pub fn from_settings(s: &Settings) -> Self {
        let mut h = Self::defaults();
        for (id, text) in &s.hotkeys {
            if let Some(a) = Action::from_id(id) {
                h.map.insert(a, Self::parse(text));
            }
        }
        h
    }
    /// Store only bindings that differ from the defaults.
    pub fn to_settings(&self, s: &mut Settings) {
        s.hotkeys.clear();
        for &a in Action::ALL {
            let cur = self.get(a);
            if !same(cur, a.default_shortcut()) {
                s.hotkeys.insert(a.id().to_string(), cur.map(|k| Self::format(&k)).unwrap_or_default());
            }
        }
    }
    pub fn get(&self, a: Action) -> Option<KeyboardShortcut> {
        self.map.get(&a).copied().flatten()
    }
    /// Bind `a` to `ks` (None = unbound). Any other action using the same shortcut is unbound.
    pub fn set(&mut self, a: Action, ks: Option<KeyboardShortcut>) {
        if let Some(k) = ks {
            for (_, v) in self.map.iter_mut() {
                if same(*v, Some(k)) {
                    *v = None;
                }
            }
        }
        self.map.insert(a, ks);
    }
    pub fn reset(&mut self, a: Action) {
        self.set(a, a.default_shortcut());
    }
    pub fn reset_all(&mut self) {
        *self = Self::defaults();
    }
    /// Which action (if any) already uses this shortcut.
    pub fn conflict(&self, ks: KeyboardShortcut) -> Option<Action> {
        Action::ALL.iter().copied().find(|&a| same(self.get(a), Some(ks)))
    }
    /// Display text for menus ("Ctrl+B" or "").
    pub fn text(&self, a: Action) -> String {
        self.get(a).map(|k| Self::format(&k)).unwrap_or_default()
    }
    pub fn format(ks: &KeyboardShortcut) -> String {
        let mut s = String::new();
        if ks.modifiers.ctrl || ks.modifiers.command {
            s.push_str("Ctrl+");
        }
        if ks.modifiers.shift {
            s.push_str("Shift+");
        }
        if ks.modifiers.alt {
            s.push_str("Alt+");
        }
        s.push_str(ks.logical_key.name());
        s
    }
    pub fn parse(s: &str) -> Option<KeyboardShortcut> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut m = Modifiers::NONE;
        let mut key = None;
        for part in s.split('+') {
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => m = m.plus(Modifiers::CTRL),
                "shift" => m = m.plus(Modifiers::SHIFT),
                "alt" => m = m.plus(Modifiers::ALT),
                other => key = Key::from_name(other).or_else(|| Key::from_name(part.trim())),
            }
        }
        key.map(|k| KeyboardShortcut::new(m, k))
    }
    /// Consume matching shortcuts this frame. Nothing fires while a text field has focus.
    pub fn poll(&self, ctx: &egui::Context) -> Vec<Action> {
        if ctx.wants_keyboard_input() {
            return Vec::new();
        }
        let mut out = Vec::new();
        ctx.input_mut(|i| {
            for &a in Action::ALL {
                if let Some(ks) = self.get(a) {
                    if i.consume_shortcut(&ks) {
                        out.push(a);
                    }
                }
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_format_roundtrip() {
        for &a in Action::ALL {
            if let Some(k) = a.default_shortcut() {
                let txt = Hotkeys::format(&k);
                assert!(same(Hotkeys::parse(&txt), Some(k)), "{txt}");
            }
        }
    }
    #[test]
    fn no_duplicate_defaults() {
        let mut seen = std::collections::HashSet::new();
        for &a in Action::ALL {
            if let Some(k) = a.default_shortcut() {
                assert!(seen.insert(Hotkeys::format(&k)), "duplicate default {}", Hotkeys::format(&k));
            }
        }
    }
}
