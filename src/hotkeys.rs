//! Keyboard shortcuts: a fixed list of actions, default bindings, user overrides (stored in Settings),
//! and per-frame polling. Tool selection (the `Action::Tool*` entries below) lives here too, fully
//! rebindable; `ui::tools::handle_hotkeys` polls its own binding for each one with EXACT modifier
//! matching (its poll runs before this table, and egui's logical matching would let a bare tool letter
//! swallow every Shift+<letter> action on the same key) and sets the active tool directly, since the
//! tool strip owns that state. Only bare S (the snap toggle) and Shift+S (cycling the shape tools,
//! still riding on `AddShape`'s default below) stay hardcoded in `ui::tools`, ahead of everything here.
//! Mouse modifiers (Ctrl+Scroll zoom, Alt+Scroll track height, Shift+Scroll pan) are fixed and not part
//! of this table.

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
    TrimToInOut => "trim_in_out", "Trim to In / Out", sc(CTRL_SHIFT, Key::I);
    RippleDeleteInOut => "ripple_in_out", "Ripple Delete In / Out", sc(CTRL_SHIFT, Key::Delete);
    AddText => "add_text", "Add Text Clip", sc(SHIFT, Key::T);
    ZoomIn => "zoom_in", "Zoom In", sc(CTRL, Key::Equals);
    ZoomOut => "zoom_out", "Zoom Out", sc(CTRL, Key::Minus);
    ZoomFit => "zoom_fit", "Zoom to Fit", sc(SHIFT, Key::Z);
    LinkToggle => "link", "Link / Unlink", sc(CTRL, Key::L);
    ToggleEnabled => "toggle_enabled", "Enable / Disable Clip", sc(SHIFT, Key::D);
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
    AddTransitionEnd => "add_transition_end", "Add Transition at Clip End", None;
    TogglePlanner => "toggle_planner", "Show / Hide Planner", sc(CTRL, Key::Num7);
    AutoCut => "auto_cut", "Auto-cut (silence) Panel", sc(CTRL_ALT, Key::A);
    NestSequence => "nest", "Nest Selection into a New Sequence", sc(ALT, Key::N);
    OpenParentSequence => "parent_sequence", "Back to Parent Timeline", sc(ALT, Key::ArrowUp);
    SaveTemplate => "save_template", "Save Selection as Template…", None;
    ApplyFlow => "flow", "Flow Motion Between Selected Clips", None;
    // ---- round 3 ----
    AddLastTransition => "add_last_transition", "Add Last Used Transition", sc(CTRL, Key::T);
    CopyAttributes => "copy_attrs", "Copy Attributes", sc(CTRL_ALT, Key::C);
    PasteAttributes => "paste_attrs", "Paste Attributes…", sc(CTRL_ALT, Key::V);
    // Bare M, per the user's explicit ask ("adding a marker should be able to be done by clicking m");
    // the Marker *tool* sits on Shift+M below. Safe because the tool poll matches modifiers exactly.
    AddMarker => "add_marker", "Add Marker at Playhead", sc(NONE, Key::M);
    ToggleMarkers => "toggle_markers", "Show / Hide Markers", sc(CTRL, Key::Num8);
    ToggleNodes => "toggle_nodes", "Show / Hide Node Editor", sc(CTRL, Key::Num9);
    ToggleMixer => "toggle_mixer", "Show / Hide Mixer", sc(CTRL, Key::Num0);
    ToggleTools => "toggle_tools", "Show / Hide Tools", None;
    AddShape => "add_shape", "Add Shape", sc(SHIFT, Key::S);
    AddAdjustment => "add_adjustment", "Add Adjustment Layer", sc(CTRL_ALT, Key::L);
    AddMask => "add_mask", "Add Mask to Selection", sc(CTRL_SHIFT, Key::M);
    ExportFrame => "export_frame", "Export Frame…", sc(CTRL_SHIFT, Key::F);
    ScreenCapture => "screen_capture", "Screen Recording…", None;
    Voiceover => "voiceover", "Record Voiceover…", sc(CTRL_ALT, Key::R);
    ImportTimeline => "import_timeline", "Import Timeline (Premiere / Resolve XML, EDL)…", None;
    MovieMode => "movie_mode", "Movie Mode (pre-render)", None;
    // ---- clip clipboard (Copy/Paste *Attributes* above is a different feature) ----
    CopyClips => "copy_clips", "Copy Clips", sc(CTRL, Key::C);
    CutClips => "cut_clips", "Cut Clips", sc(CTRL, Key::X);
    PasteClips => "paste_clips", "Paste Clips at Playhead", sc(CTRL, Key::V);
    PasteInPlace => "paste_in_place", "Paste Clips on the First Free Track", sc(CTRL_SHIFT, Key::V);
    PasteInsert => "paste_insert", "Paste Insert (ripple the rest right)", sc(NONE, Key::F10);
    PasteAtTop => "paste_at_top", "Paste on a New Track at the Top", sc(NONE, Key::F9);
    // ---- container clips ----
    AddContainer => "add_container", "Add Container Clip at Playhead", None;
    ReplaceContainerMedia => "replace_container", "Replace Container Media…", None;
    MakeContainer => "make_container", "Convert to Container", None;
    UnmakeContainer => "unmake_container", "Remove Container", None;
    // ---- ws:registries-schema-hooks ----
    // ---- ws:size-diet ----
    // Unbound by design: no free chord in the skeleton keymap. Opened by the What's New WINDOW_DRAWER
    // on a version bump, `ui.action("whats_new")`, or a later Help-menu entry (command-palette owns
    // menus.rs).
    WhatsNew => "whats_new", "What's New", None;
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    // Unbound by design (skeleton keymap): every row below is Mark-instead / editable-result first,
    // reached from the Auto-cut pane's Beats/Loudness/Duck sections or the command palette. AutoDuck
    // and Normalize are declared here (not a wave-0b stub) — audio-dsp-automation's inspector_audio.rs
    // dispatches both and must depend on this workstream landing first.
    DetectBeats => "detect_beats", "Detect Beats → Markers", None;
    SplitAtBeats => "split_at_beats", "Split at Beats", None;
    AutoDuck => "auto_duck", "Duck Music under Dialogue", None;
    Normalize => "normalize", "Normalize Selection", None;
    MatchLoudness => "match_loudness", "Match Loudness across Selection", None;
    // ---- ws:audio-dsp-automation ----
    // ---- ws:color-engine ----
    // Unbound by design: no free chord in the skeleton keymap (per the plan's Actions/hotkeys table).
    AutoColor => "auto_color", "Auto Colour", None;
    ColorMatch => "color_match", "Colour Match to Reference", None;
    BypassGrade => "bypass_grade", "Bypass Grade", None;
    // ---- ws:command-palette ----
    CommandPalette => "command_palette", "Command Palette", sc(CTRL, Key::K);
    CheatSheet => "cheat_sheet", "Keyboard Shortcuts overlay", sc(NONE, Key::F1);
    // Owned exclusively by this workstream (see plans/ui-overhaul/issues/command-palette.md's
    // "Actions and hotkeys" table): ws:layout-modes-onboarding (wave 2) consumes these two variants
    // through its own ACT_HANDLERS/WINDOW_DRAWERS arm but must NEVER redeclare them here — a second
    // `actions!` row for either identifier is a duplicate-enum-variant compile error. Both are inert
    // (fall through App::act's `_ => {}` catch-all) until that wave lands the real behaviour.
    ToggleLayoutMode => "toggle_layout_mode", "Layout Mode: Dynamic / Granular", sc(CTRL_SHIFT, Key::G);
    ShowWelcome => "show_welcome", "Show Welcome Again", None;
    // ---- ws:forgiveness ----
    // Unbound by design (no free chord in the skeleton keymap): Settings ▸ Performance button /
    // palette row / toast button only.
    ClearCaches => "clear_caches", "Clear Caches", None;
    RestoreBackup => "restore_backup", "Restore Autosave…", None;
    UndoSettings => "undo_settings", "Undo Settings Change", None;
    // ---- ws:player-rate-loop ----
    ShuttleBack => "shuttle_back", "Shuttle Reverse", sc(NONE, Key::J);
    ShuttleFwd => "shuttle_fwd", "Shuttle Forward", sc(NONE, Key::L);
    LoopInOut => "loop_in_out", "Loop In→Out", sc(CTRL_SHIFT, Key::L);
    PlayInOut => "play_in_out", "Play In→Out", sc(CTRL_SHIFT, Key::Space);
    PlayAround => "play_around", "Play Around Playhead", sc(NONE, Key::Slash);
    PlayToOut => "play_to_out", "Play to Out", sc(CTRL, Key::Space);
    StepBack10 => "step_back_10", "Step Back 10 Frames", sc(SHIFT, Key::ArrowLeft);
    StepFwd10 => "step_fwd_10", "Step Forward 10 Frames", sc(SHIFT, Key::ArrowRight);
    // Unbound by design: no free chord in the skeleton keymap (transport menu / palette only).
    FastReview => "fast_review", "Fast Review", None;
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    // 25 bound + 6 unbound = 31 (see plans/ui-overhaul/issues/trim-model.md's "Review trail" F7 —
    // the plan text's own earlier "22 bound"/"30 total" counts were a stale recount, corrected there).
    SelectEditPoint => "select_edit_point", "Select Nearest Edit Point", sc(NONE, Key::U);
    CycleEditSide => "cycle_edit_side", "Cycle Edit Point Side", sc(SHIFT, Key::U);
    TrimLeft1 => "trim_left_1", "Trim Edit -1 Frame", sc(NONE, Key::OpenBracket);
    TrimRight1 => "trim_right_1", "Trim Edit +1 Frame", sc(NONE, Key::CloseBracket);
    TrimLeft10 => "trim_left_10", "Trim Edit -10 Frames", sc(CTRL, Key::OpenBracket);
    TrimRight10 => "trim_right_10", "Trim Edit +10 Frames", sc(CTRL, Key::CloseBracket);
    ExtendEdit => "extend_edit", "Extend Edit to Playhead", sc(NONE, Key::E);
    TrimTop => "trim_top", "Trim Start to Playhead (Top)", sc(NONE, Key::Q);
    TrimTail => "trim_tail", "Trim End to Playhead (Tail)", sc(NONE, Key::W);
    SlipLeft => "slip_left", "Slip -1 Frame", sc(ALT, Key::Comma);
    SlipRight => "slip_right", "Slip +1 Frame", sc(ALT, Key::Period);
    MarkClip => "mark_clip", "Mark Clip (In/Out from clip under playhead)", sc(NONE, Key::X);
    GoToIn => "go_to_in", "Go to In", sc(SHIFT, Key::I);
    GoToOut => "go_to_out", "Go to Out", sc(SHIFT, Key::O);
    JoinThroughEdit => "join_through", "Join Through Edit", sc(CTRL, Key::J);
    DuplicateClips => "duplicate", "Duplicate Clips", sc(CTRL, Key::D);
    SelectForward => "select_forward", "Select Forward from Playhead", sc(NONE, Key::A);
    SelectBackward => "select_backward", "Select Backward from Playhead", sc(SHIFT, Key::A);
    SelectAtPlayhead => "select_at_playhead", "Select Clips Under Playhead", sc(CTRL_SHIFT, Key::D);
    PrevKeyframe => "prev_keyframe", "Previous Keyframe", sc(ALT, Key::ArrowLeft);
    NextKeyframe => "next_keyframe", "Next Keyframe", sc(ALT, Key::ArrowRight);
    // bare V is ToolSelect (exact-modifier tool poll); Shift+V is a distinct chord and passes through.
    SpliceInsert => "splice", "Splice (Insert) at Playhead", sc(SHIFT, Key::V);
    OverwriteAtPlayhead => "overwrite", "Overwrite at Playhead", sc(NONE, Key::B);
    LiftInOut => "lift", "Lift In->Out", sc(NONE, Key::Semicolon);
    ExtractInOut => "extract", "Extract In->Out", sc(NONE, Key::Quote);
    CloseGapAtPlayhead => "close_gap", "Close Gap at Playhead", None;
    UnnestClip => "unnest", "Un-nest Sequence Clip", None;
    ReplaceWithLibrarySelection => "replace_clip", "Replace with Library Selection", None;
    ToggleTrackLock => "toggle_track_lock", "Lock / Unlock Track under Cursor", None;
    ToggleTrackRipple => "toggle_track_ripple", "Toggle Ripple (Sync) on Track", None;
    ToggleTrackMagnetic => "toggle_track_magnetic", "Toggle Magnetic Track", None;
    // ---- ws:canvas-handles-monitor ----
    // ---- ws:export-deliver ----
    // bare M is AddMarker (ctrl=false) and Ctrl+Shift+M is AddMask, so Ctrl+M is free (exact match).
    QuickExport => "quick_export", "Quick Export", sc(CTRL, Key::M);
    RenderSelection => "render_selection", "Render Selection (pre-render)", None;
    BakeSelection => "bake_selection", "Render in Place (bake to new asset)", None;
    ExportMarkers => "export_markers", "Export Markers…", None;
    // ---- ws:inspector-gallery ----
    // ---- ws:layout-modes-onboarding ----
    // ---- ws:media-library ----
    // ---- ws:source-monitor ----
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    // ---- ws:pro-monitor ----
    // ---- ws:pro-timeline ----
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
    // ---- tool selection (ui::tools) — polled and dispatched there, not through App::act ----
    ToolSelect => "tool_select", "Select Tool", sc(NONE, Key::V);
    ToolText => "tool_text", "Text Tool", sc(NONE, Key::T);
    ToolDraw => "tool_draw", "Draw Tool", sc(NONE, Key::D);
    // Mask used to be bare M; K (the obvious next pick) is already Stop's key, so this moves to G
    // instead, freeing M for "add marker at playhead" above (the Marker tool itself is on Shift+M).
    ToolMask => "tool_mask", "Mask Tool (repeat to cycle shape)", sc(NONE, Key::G);
    ToolMarker => "tool_marker", "Marker Tool", sc(SHIFT, Key::M);
    ToolCut => "tool_cut", "Cut Tool (Razor)", sc(NONE, Key::C);
    ToolStretch => "tool_stretch", "Stretch Tool", sc(NONE, Key::R);
    ToolSpacer => "tool_spacer", "Spacer Tool", None;
}

pub struct Hotkeys {
    map: HashMap<Action, Option<KeyboardShortcut>>,
    // ---- ws:command-palette ----
    /// Non-`Action` bindings shown alongside the table in the conflict UI — currently just live Luau
    /// `@hotkey` scripts (refreshed at the 1Hz script-meta poll, `ui::app::palette_ctl::tick`), keyed by
    /// the script's display name rather than a path (all the conflict UI needs is a label). Never
    /// persisted — `to_settings`/`from_settings` only round-trip `Action` bindings.
    extra: Vec<(String, KeyboardShortcut)>,
}

impl Hotkeys {
    pub fn defaults() -> Self {
        Self { map: Action::ALL.iter().map(|&a| (a, a.default_shortcut())).collect(), extra: Vec::new() }
    }
    // ---- ws:command-palette ----
    pub fn set_extra(&mut self, extra: Vec<(String, KeyboardShortcut)>) {
        self.extra = extra;
    }
    pub fn extra(&self) -> &[(String, KeyboardShortcut)] {
        &self.extra
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
        self.poll_pass(ctx, false)
    }
    /// Second pass, run *after* the panes are drawn: the clip clipboard keys, which the curve and node
    /// editors also claim while the pointer is over them (whoever is hovered wins, the timeline is the
    /// fallback).
    pub fn poll_late(&self, ctx: &egui::Context) -> Vec<Action> {
        self.poll_pass(ctx, true)
    }
    fn poll_pass(&self, ctx: &egui::Context, late: bool) -> Vec<Action> {
        if ctx.wants_keyboard_input() {
            return Vec::new();
        }
        if !late {
            restore_clipboard_keys(ctx);
        }
        // consume_shortcut ignores *extra* shift/alt, so walking the table in declaration order would
        // let Ctrl+Shift+Z fire plain Undo: try the most specific binding first.
        let mut order: Vec<Action> = Action::ALL.iter().copied().filter(|&a| is_late(a) == late).collect();
        order.sort_by_key(|&a| {
            std::cmp::Reverse(self.get(a).map_or(0u8, |k| k.modifiers.shift as u8 + k.modifiers.alt as u8))
        });
        let mut out = Vec::new();
        ctx.input_mut(|i| {
            for a in order {
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

// ---- ws:command-palette ----
/// Chords the app hard-codes ahead of, or instead of, the `Action` table — bare `S` (snap toggle,
/// `ui::tools::handle_snap_hotkey`), `Shift+S` (shape-tool cycle, `ui::tools::handle_hotkeys` — also
/// `AddShape`'s grandfathered default, a documented exception in `reserved_chords_are_free` below),
/// `Ctrl+Y` (Redo alias, polled directly in `App::update`), `Backspace` (Delete alias, same), `Escape`
/// (fullscreen exit while `self.fullscreen`), `Tab`/`Shift+Tab` (egui's own focus traversal) and
/// `Alt+Space` (Windows' system menu). A rebindable UI that let a user pick one of these would silently
/// lose it to whichever poll runs first — `conflict_all` reports them so the Hotkeys tab can say so.
pub const RESERVED: &'static [(&'static str, Modifiers, Key)] = &[
    ("Toggle snapping (S)", NONE, Key::S),
    ("Cycle shape tool (Shift+S)", SHIFT, Key::S),
    ("Redo (Ctrl+Y alias)", CTRL, Key::Y),
    ("Delete (Backspace alias)", NONE, Key::Backspace),
    ("Exit fullscreen (Esc)", NONE, Key::Escape),
    ("Focus next (Tab)", NONE, Key::Tab),
    ("Focus previous (Shift+Tab)", SHIFT, Key::Tab),
    ("Windows system menu (Alt+Space)", ALT, Key::Space),
];

/// Who already claims a chord: a bound `Action`, or one of the hard-coded `RESERVED` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    Action(Action),
    Fixed(&'static str),
}

impl Hotkeys {
    /// Every claimant of `ks`: a bound `Action` first (`conflict`), else a `RESERVED` row, else free
    /// (`None`) — the honest, whole-app view `Settings ▸ Hotkeys`'s rebind UI needs (`conflict` alone
    /// would let a user "successfully" bind a key that a hard-coded poll would still eat first).
    pub fn conflict_all(&self, ks: KeyboardShortcut) -> Option<Claim> {
        if let Some(a) = self.conflict(ks) {
            return Some(Claim::Action(a));
        }
        RESERVED
            .iter()
            .find(|&&(_, m, k)| canon(&KeyboardShortcut::new(m, k)) == canon(&ks))
            .map(|&(name, ..)| Claim::Fixed(name))
    }
}

/// Section a hotkey belongs to, for the cheat-sheet overlay and the Settings ▸ Hotkeys group headers.
/// Hand-maintained rather than folded into the `actions!` macro (a smaller diff, and grouping needs
/// change far less often than the action list itself — see the issue plan's `// ponytail:` note); the
/// `_ => "Other"` catch-all keeps a future workstream's new `Action` non-breaking even if nobody
/// remembers to extend this match.
pub fn group(a: Action) -> &'static str {
    use Action::*;
    match a {
        NewProject | OpenFile | OpenProject | Save | SaveProjectAs | ExportVideo | ExportLossless | ExportXml
        | ImportMedia | ImportTimeline | ExportFrame | ScreenCapture | Voiceover => "File",
        Undo | Redo | CopyClips | CutClips | PasteClips | PasteInPlace | PasteInsert | PasteAtTop | SelectAll
        | Deselect | CopyAttributes | PasteAttributes | Delete | RippleDelete | NudgeLeft | NudgeRight => "Edit",
        PlayPause | Stop | StepBack | StepForward | GoStart | GoEnd | PrevCut | NextCut => "Playback",
        Split
        | MarkIn
        | MarkOut
        | ClearInOut
        | TrimToInOut
        | RippleDeleteInOut
        | LinkToggle
        | ToggleEnabled
        | AddTransition
        | AddLastTransition
        | AddTransitionEnd
        | Retime
        | FreezeFrame
        | NestSequence
        | OpenParentSequence
        | SaveTemplate
        | ApplyFlow
        | AddContainer
        | ReplaceContainerMedia
        | MakeContainer
        | UnmakeContainer
        | AddVideoTrack
        | AddAudioTrack
        | ZoomIn
        | ZoomOut
        | ZoomFit
        | ToggleSnap => "Timeline",
        AddText | AddShape | AddAdjustment | AddMask | AddSubtitle | AddMarker => "Insert",
        ToggleLibrary | ToggleInspector | ToggleEffects | ToggleTransitions | ToggleCurves | ToggleSubtitles
        | TogglePlanner | ToggleMarkers | ToggleNodes | ToggleMixer | ToggleTools => "Panels",
        ToolSelect | ToolText | ToolDraw | ToolMask | ToolMarker | ToolCut | ToolStretch | ToolSpacer => "Tools",
        CommandPalette | CheatSheet | ToggleLayoutMode | ShowWelcome | Fullscreen | Settings | AutoCut | MovieMode
        | WhatsNew => "General",
        // every current variant has an arm above (same `unreachable_patterns` situation as act()'s own
        // prelude match in ui/app/actions.rs); kept so a future workstream's new Action compiles into
        // "Other" by default instead of forcing an edit here.
        #[allow(unreachable_patterns)]
        _ => "Other",
    }
}

/// Actions the curve and node editors also claim while the pointer is over them, so the timeline only
/// gets them if no pane wanted them. Delete is here for the same reason the clipboard keys are: the early
/// pass runs BEFORE any pane is drawn, so a global Delete would eat the key and remove the selected CLIPS
/// while the user was deleting keyframes or nodes.
fn is_late(a: Action) -> bool {
    matches!(a, Action::CopyClips | Action::CutClips | Action::PasteClips | Action::PasteInPlace | Action::Delete)
}

/// egui-winit swallows the clipboard keys: Ctrl+C / Ctrl+X / Ctrl+V (and Ctrl+Alt+C/V, Shift+Delete,
/// Ctrl+Insert) arrive as `Event::Copy` / `Cut` / `Paste` with the key event *dropped*, so no shortcut
/// on those keys can ever match. Put the key events back. The clipboard event no longer says which key
/// produced it, so the modifiers decide — with Shift down a Cut is Windows' Shift+Delete (Ctrl+Shift+X
/// is nobody's shortcut). Callers skip this while a text field has focus, so text copy/paste is untouched.
fn restore_clipboard_keys(ctx: &egui::Context) {
    ctx.input_mut(|i| {
        let m = i.modifiers;
        let cmd = m.ctrl || m.command;
        let mut keys: Vec<Key> = Vec::new();
        for e in &i.events {
            match e {
                egui::Event::Cut if m.shift => keys.push(Key::Delete),
                egui::Event::Cut if cmd => keys.push(Key::X),
                egui::Event::Copy if cmd => keys.push(Key::C),
                egui::Event::Paste(_) if cmd => keys.push(Key::V),
                _ => {}
            }
        }
        for key in keys {
            i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
        }
    });
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
    /// Feed the events egui-winit *actually* delivers for these chords (the raw key event is gone) and
    /// check the actions still fire — and in the late pass for the clip clipboard, so a hovered curve /
    /// node editor gets first refusal.
    #[test]
    fn clipboard_chords_survive_winit_translation() {
        let h = Hotkeys::defaults();
        let run = |ev: egui::Event, m: Modifiers| {
            let ctx = egui::Context::default();
            let raw = egui::RawInput { modifiers: m, events: vec![ev], ..Default::default() };
            let (mut early, mut late) = (Vec::new(), Vec::new());
            ctx.run(raw, |ctx| {
                early = h.poll(ctx);
                late = h.poll_late(ctx);
            });
            (early, late)
        };
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        let ctrl_shift = Modifiers { shift: true, ..ctrl };
        for (ev, m, want) in [
            (egui::Event::Copy, ctrl, Action::CopyClips),
            (egui::Event::Cut, ctrl, Action::CutClips),
            (egui::Event::Paste("x".into()), ctrl, Action::PasteClips),
            (egui::Event::Paste("x".into()), ctrl_shift, Action::PasteInPlace),
        ] {
            let (early, late) = run(ev, m);
            assert!(early.is_empty(), "{want:?} must wait for the late pass, got {early:?}");
            assert_eq!(late, vec![want]);
        }
        // Windows folds Shift+Delete into Cut too — that one is a normal (early) action
        let (early, _) = run(egui::Event::Cut, Modifiers::SHIFT);
        assert_eq!(early, vec![Action::RippleDelete]);
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

    // ---- ws:command-palette ----

    #[test]
    fn reserved_chords_are_free() {
        for &a in Action::ALL {
            let Some(k) = a.default_shortcut() else { continue };
            for &(name, m, key) in RESERVED {
                // AddShape's default IS Shift+S — a documented, grandfathered exception (see RESERVED's
                // doc comment): the shape-tool cycle poll runs first, so the action never actually fires
                // from the key, but its "default" text still needs somewhere to live for the menu/palette.
                if a == Action::AddShape && m == SHIFT && key == Key::S {
                    continue;
                }
                assert!(
                    canon(&k) != canon(&KeyboardShortcut::new(m, key)),
                    "{:?}'s default {} collides with the reserved chord '{name}'",
                    a,
                    Hotkeys::format(&k)
                );
            }
        }
    }

    #[test]
    fn conflict_all_sees_reserved_and_actions() {
        let h = Hotkeys::defaults();
        assert_eq!(h.conflict_all(KeyboardShortcut::new(NONE, Key::S)), Some(Claim::Fixed("Toggle snapping (S)")));
        assert_eq!(h.conflict_all(KeyboardShortcut::new(CTRL, Key::Y)), Some(Claim::Fixed("Redo (Ctrl+Y alias)")));
        assert_eq!(
            h.conflict_all(KeyboardShortcut::new(NONE, Key::Backspace)),
            Some(Claim::Fixed("Delete (Backspace alias)"))
        );
        assert_eq!(
            h.conflict_all(KeyboardShortcut::new(NONE, Key::Escape)),
            Some(Claim::Fixed("Exit fullscreen (Esc)"))
        );
        // a bound action's own chord resolves to Claim::Action, checked ahead of RESERVED
        assert_eq!(h.conflict_all(KeyboardShortcut::new(CTRL, Key::Z)), Some(Claim::Action(Action::Undo)));
        // a genuinely free chord is neither
        assert_eq!(h.conflict_all(KeyboardShortcut::new(CTRL_ALT, Key::F9)), None);
    }

    #[test]
    fn no_duplicate_action_declarations_across_wave1_and_wave2() {
        // compile-time proxy for the audit-fixed blocker: this ws is the sole declaration site for
        // ToggleLayoutMode/ShowWelcome (ws:layout-modes-onboarding, wave 2, may only consume them via
        // ACT_HANDLERS) — a second `actions!` row for either identifier here is a duplicate-variant
        // compile error, so this just pins that today's file has exactly one declaration of each.
        let src = include_str!("hotkeys.rs");
        for name in ["ToggleLayoutMode", "ShowWelcome"] {
            let decl = format!("{name} =>");
            assert_eq!(
                src.matches(decl.as_str()).count(),
                1,
                "{name} must be declared exactly once in hotkeys.rs (found {})",
                src.matches(decl.as_str()).count()
            );
        }
    }

    #[test]
    fn group_covers_every_action() {
        for &a in Action::ALL {
            assert_ne!(group(a), "", "{a:?} has no group");
        }
    }
}
