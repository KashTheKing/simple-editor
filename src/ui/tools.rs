//! Editing tools. The viewer's tool rail (`rail`, drawn by `ui::preview` over the Preview's left edge):
//!   Select (V) · Crop (the canvas crop handles) · Text (T) · Shape ▾ (Shift+S cycles; rect, ellipse,
//!   triangle, polygon, star, line, arrow) · Draw (D) · Mask ▾ (G cycles; rect/ellipse/polygon/path)
//! and, while a shape or Draw is active, `style_controls` in a strip along the viewer's top edge: fill
//! and stroke colours and a stroke width; for Draw, a play/record button, the brush colour/width, the
//! playback speed (0.5x / 1x / 2x) and a page toggle. The timeline tools (Cut C, Stretch R, Marker
//! Shift+M, Spacer) are picked from the timeline's toolbar and their keys. Every tool key is its
//! `Action::Tool*` binding (`hotkeys.rs`), remappable in Settings ▸ Hotkeys; `handle_hotkeys` below polls
//! the live binding and the rail's tooltips read it from the menu snapshot.
//!
//! The active tool changes what a click-drag in the Preview does (see `ui::preview`): Select edits the
//! selected clip, a shape tool drags out a new Shape clip at the playhead (Shift locks its aspect ratio,
//! Alt grows it from the press point instead of corner-to-corner), Draw records strokes while the mouse
//! is down (timed, so the sketch replays), Mask edits the selected effect's / clip's mask.

use crate::hotkeys::{Action, Hotkeys};
use crate::model::{MaskShape, ShapeKind, ShapeStyle};
use crate::theme::Palette;
use eframe::egui;
use egui::{Align2, Color32, CornerRadius, FontId, Key, Modifiers, Sense, Stroke, StrokeKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Text,
    Shape(ShapeKind),
    Draw,
    Mask(MaskShape),
    /// Razor: click a clip in the timeline to split it there.
    Cut,
    /// Click the timeline to drop a project marker.
    Marker,
    /// Drag a clip edge to change its speed instead of trimming it.
    Stretch,
    /// Drag the timeline lanes to open (or close) a gap from the press time onward.
    Spacer,
}

pub struct ToolsState {
    pub tool: Tool,
    /// Style applied to the next shape drawn.
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub stroke_width: f32,
    pub sides: u32,
    pub corner: f32,
    /// Draw tool: brush and recording rate.
    pub brush: [u8; 4],
    pub brush_width: f32,
    pub draw_rate: f32,
    pub page: [u8; 4],
    /// Draw tool: a take is running - the app plays the video and drops every stroke into one drawing
    /// until this goes back off (see `App::toggle_draw_recording`).
    pub recording: bool,
    // ---- ws:layout-modes-onboarding ----
    // ---- ws:viewer-surface ----
    /// The rail's Shape / Mask buttons pick the variant used last (the flyout or Shift+S / G changes it).
    pub last_shape: ShapeKind,
    pub last_mask: MaskShape,
}

impl Default for ToolsState {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            fill: [255, 255, 255, 255],
            stroke: [0, 0, 0, 0],
            stroke_width: 4.0,
            sides: 5,
            corner: 0.0,
            brush: [255, 80, 80, 255],
            brush_width: 6.0,
            draw_rate: 1.0,
            page: [0, 0, 0, 0],
            recording: false,
            last_shape: ShapeKind::Rect,
            last_mask: MaskShape::Rect,
        }
    }
}

/// Which way a triangle / arrow glyph points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// What `icon_button` draws: a symbol from Windows' own icon font (`Glyph::icon`, painted in
/// `theme::icons()` - Segoe Fluent Icons / Segoe MDL2 Assets, 0 bytes in the exe). The few with no
/// fitting symbol (`draw_glyph`'s vector arms) are painted with the painter instead. Variant names are
/// persisted (`Settings.icon_overrides`), so a variant is never removed - near-duplicates share a char.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Glyph {
    Cursor,
    Letter(char),
    Rect,
    Ellipse,
    Poly(u32),
    Star,
    Line,
    Arrow,
    Pencil,
    Mask,
    Zoom,
    Razor,
    Flag,
    Hourglass,
    Eye,
    EyeOff,
    Diamond,
    Record,
    Mic,
    Headphone,
    SpeakerOn,
    SpeakerOff,
    Camera,
    FilmStrip,
    /// Two patch boxes joined by a cable - a node graph.
    Nodes,
    /// A stack of sheets - an adjustment layer.
    Layers,
    /// A shooting target: rings and cross ticks - motion tracking.
    Target,
    /// A horseshoe magnet - the snapping toggle.
    Magnet,
    /// Two posts with a double-headed arrow between them - a gap being widened.
    Spacer,
    /// An eighth note - the audio-effects catalogue card (no picture to render for those).
    MusicNote,
    /// A file-explorer folder: a tab sitting on a body.
    Folder,
    /// A container / slot clip.
    Container,
    /// Two crossed strokes - close, delete, clear.
    Cross,
    /// A filled dot - a colour swatch, a bullet, "in use".
    Dot,
    /// Two sheets, one behind the other - copy.
    Copy,
    /// A clipboard - paste.
    Paste,
    /// A filled triangle pointing `Dir` - reorder, collapse, step one frame.
    Tri(Dir),
    /// Two triangles - the previous / next cut.
    Skip(Dir),
    /// A triangle backed against a bar - go to the very start / end.
    Jump(Dir),
    Play,
    Pause,
    Stop,
    /// Four corner brackets - fullscreen.
    Fullscreen,
    /// A window with an arrow leaving it - pop this pane out.
    PopOut,
    /// An arrow into a margin bar - indent (true) / outdent (false).
    Indent(bool),
    /// A lane carrying two blocks - a nested sequence.
    Sequence,
    /// A card with a folded corner - a saved clip template.
    Template,
    /// A six-armed snowflake - a frozen frame.
    Snowflake,
    /// A movie reel: rim, hub, four spoke holes and a tape tail.
    FilmReel,
    /// A filled lightning zigzag - effects / performance.
    Bolt,
    /// An open tray with an arrow dropping into it - import.
    ImportArrow,
    /// The same tray with the arrow rising out - export.
    ExportArrow,
    /// Two overlapping squares with a diagonal across the overlap - a transition.
    Transition,
    /// A caption box with two text bars in its lower half.
    Subtitles,
    /// A cog: ring, eight stubs and a hub - settings.
    Gear,
    /// Three slider tracks, each with its knob at a different position.
    Sliders,
    /// An open-ended wrench head with a diagonal handle.
    Wrench,
    /// A clapperboard: body plus a slanted, hatched top bar.
    Clapperboard,
    /// Five vertical bars around a midline - an audio waveform.
    Waveform,
    /// Axes with a rising curve and two square handles - a value curve.
    CurveIcon,
    /// A clock face with two hands.
    Clock,
    /// A sheet with three ruled lines.
    Notepad,
    /// A ribbon with a notched V bottom.
    Bookmark,
    /// A curved arrow - undo (`Dir::Left`) / redo (`Dir::Right`).
    UndoArrow(Dir),
    /// A save disk: notched square, shutter and label.
    FloppyDisk,
    /// A console window: '>' prompt and an underscore.
    Terminal,
    /// A wide 16:9 outline - landscape format.
    Landscape,
    /// A tall 9:16 outline - portrait / vertical format.
    Portrait,
    /// A square outline - 1:1 format.
    Square,
    /// A rounded rectangle with a play triangle - YouTube-style video badge.
    PlayRect,
    /// A 2x2 grid of rounded squares - a feed / profile grid.
    GridIcon,
    /// Three ruled rows, each with a leading bullet - a row/list view (paired with `GridIcon`).
    ListIcon,
    /// A frame with corner brackets and a centre tick - the social-guide overlay.
    Guides,
    // ---- ws:registries-schema-hooks ----
    // ---- ws:size-diet ----
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    // ---- ws:audio-dsp-automation ----
    /// Three vertical bars of increasing height - the mixer's level / LUFS meter row.
    Meter,
    // ---- ws:color-engine ----
    // ---- ws:command-palette ----
    /// A small key cap grid - the cheat-sheet / Settings ▸ Hotkeys tab.
    Keyboard,
    /// A magnifying glass - the palette's own search field / row.
    Search,
    // ---- ws:forgiveness ----
    // ---- ws:player-rate-loop ----
    // ---- ws:snap-engine ----
    /// A curved arrow around a clip edge - the Roll gesture cursor (wave 2 wires the drag itself).
    RollCursor,
    /// Filmstrip frames sliding sideways under a fixed rect - the Slip gesture cursor (wave 2).
    SlipCursor,
    // ---- ws:trim-model ----
    // ---- ws:canvas-handles-monitor ----
    /// Two overlapping L-shaped crop marks - painted at the pointer over a crop handle.
    Crop,
    /// A three-quarter circular arrow - painted at the pointer over the rotate knob.
    Rotate,
    // ---- ws:export-deliver ----
    /// Two stacked documents with a small clock in the corner - the render queue.
    Queue,
    // ---- ws:inspector-gallery ----
    /// A ring with three spoke handles - the Color section's Primaries wheels.
    Wheel,
    /// A filmstrip-corner tile with a diagonal split - a LUT card / the LUT browser.
    Lut,
    // ---- ws:layout-modes-onboarding ----
    /// A pushpin (head, bar, needle) - a tab pinned against auto-surfacing.
    Pin,
    /// Four corner arrows pointing outward - maximise a pane to the full tile.
    Maximize,
    // ---- ws:media-library ----
    /// A triangle with an exclamation mark - the library's offline-media badge / relink hint.
    Warning,
    /// Two overlapping links - a subclip's tie to its parent asset.
    Chain,
    // ---- ws:source-monitor ----
    /// A bar with a block landing after its end - smart edit "Append at End".
    Append,
    /// Two blocks with arrows pulling them together - smart edit "Close Up" (close the gap).
    CloseUp,
    /// A block floating above a bar with an up arrow - smart edit "Place on Top" (new track above).
    PlaceOnTop,
    /// A viewfinder rect with a record dot - the Source/Record monitor toggle.
    SourceRecord,
    /// A tape cassette: two reels in a shell - Source Tape.
    Tape,
    // ---- ws:timeline-trim-gestures ----
    /// A padlock - the track header's Lock toggle.
    Lock,
    /// Two chain links - the track header's Ripple (sync) toggle.
    Link,
    // ---- ws:transcript-captions ----
    /// Three text lines of decreasing width, the middle one's leading word lit (the Transcript
    /// section / "View transcript" window).
    Transcript,
    // ---- ws:pro-monitor ----
    /// A vertical split with opposite-shaded halves - the Compare (wipe/side-by-side) toggle.
    Compare,
    /// A small oscilloscope trace - the Scopes window toggle.
    Scope,
    /// A 2x2 grid of squares - the multicam angle-grid window.
    Grid4,
    // ---- ws:pro-timeline ----
    /// A filled square with a thin ring - the track-header colour swatch. Named `Swatch`, not
    /// `Palette`, to avoid colliding with the pervasive `use crate::theme::Palette;`.
    Swatch,
    /// Three stacked bars of differing width - the view-preset combo / overview toggle.
    Rows,
    // ---- ws:text-titles ----
    /// A small "T" over a horizontal bar - the Gallery's Titles tab button.
    Titles,
    // ---- ws:docs-refresh ----
    // ---- ws:ui-kit ----
    /// A stopwatch - the rate-stretch tool / Retime (was the overloaded `Hourglass`).
    Speed,
    /// A clock with a back arrow - the History pane and its rows.
    History,
    /// Two chasing arrows - a proxy being built.
    Proxy,
}

impl Glyph {
    /// Every unit variant plus a representative of each parameterized one, for the settings icon
    /// picker (`from_name` resolves back into this list).
    pub const ALL: &'static [Glyph] = &[
        Glyph::Cursor,
        Glyph::Letter('T'),
        Glyph::Rect,
        Glyph::Ellipse,
        Glyph::Poly(5),
        Glyph::Star,
        Glyph::Line,
        Glyph::Arrow,
        Glyph::Pencil,
        Glyph::Mask,
        Glyph::Zoom,
        Glyph::Razor,
        Glyph::Flag,
        Glyph::Hourglass,
        Glyph::Eye,
        Glyph::EyeOff,
        Glyph::Diamond,
        Glyph::Record,
        Glyph::Mic,
        Glyph::Headphone,
        Glyph::SpeakerOn,
        Glyph::SpeakerOff,
        Glyph::Camera,
        Glyph::FilmStrip,
        Glyph::Nodes,
        Glyph::Layers,
        Glyph::Target,
        Glyph::Magnet,
        Glyph::Spacer,
        Glyph::MusicNote,
        Glyph::Folder,
        Glyph::Container,
        Glyph::Cross,
        Glyph::Dot,
        Glyph::Copy,
        Glyph::Paste,
        Glyph::Tri(Dir::Up),
        Glyph::Tri(Dir::Down),
        Glyph::Tri(Dir::Left),
        Glyph::Tri(Dir::Right),
        Glyph::Skip(Dir::Left),
        Glyph::Skip(Dir::Right),
        Glyph::Jump(Dir::Left),
        Glyph::Jump(Dir::Right),
        Glyph::Play,
        Glyph::Pause,
        Glyph::Stop,
        Glyph::Fullscreen,
        Glyph::PopOut,
        Glyph::Indent(true),
        Glyph::Indent(false),
        Glyph::Sequence,
        Glyph::Template,
        Glyph::Snowflake,
        Glyph::FilmReel,
        Glyph::Bolt,
        Glyph::ImportArrow,
        Glyph::ExportArrow,
        Glyph::Transition,
        Glyph::Subtitles,
        Glyph::Gear,
        Glyph::Sliders,
        Glyph::Wrench,
        Glyph::Clapperboard,
        Glyph::Waveform,
        Glyph::CurveIcon,
        Glyph::Clock,
        Glyph::Notepad,
        Glyph::Bookmark,
        Glyph::UndoArrow(Dir::Left),
        Glyph::UndoArrow(Dir::Right),
        Glyph::FloppyDisk,
        Glyph::Terminal,
        Glyph::Landscape,
        Glyph::Portrait,
        Glyph::Square,
        Glyph::PlayRect,
        Glyph::GridIcon,
        Glyph::ListIcon,
        Glyph::Guides,
        // ---- ws:registries-schema-hooks ----
        // ---- ws:size-diet ----
        // ---- ws:split-god-files ----
        // ---- ws:audio-analysis ----
        // ---- ws:audio-dsp-automation ----
        Glyph::Meter,
        // ---- ws:color-engine ----
        // ---- ws:command-palette ----
        Glyph::Keyboard,
        Glyph::Search,
        // ---- ws:forgiveness ----
        // ---- ws:player-rate-loop ----
        // ---- ws:snap-engine ----
        Glyph::RollCursor,
        Glyph::SlipCursor,
        // ---- ws:trim-model ----
        // ---- ws:canvas-handles-monitor ----
        Glyph::Crop,
        Glyph::Rotate,
        // ---- ws:export-deliver ----
        Glyph::Queue,
        // ---- ws:inspector-gallery ----
        Glyph::Wheel,
        Glyph::Lut,
        // ---- ws:layout-modes-onboarding ----
        Glyph::Pin,
        Glyph::Maximize,
        // ---- ws:media-library ----
        Glyph::Warning,
        Glyph::Chain,
        // ---- ws:source-monitor ----
        Glyph::Append,
        Glyph::CloseUp,
        Glyph::PlaceOnTop,
        Glyph::SourceRecord,
        Glyph::Tape,
        // ---- ws:timeline-trim-gestures ----
        Glyph::Lock,
        Glyph::Link,
        // ---- ws:transcript-captions ----
        Glyph::Transcript,
        // ---- ws:pro-monitor ----
        Glyph::Compare,
        Glyph::Scope,
        Glyph::Grid4,
        // ---- ws:pro-timeline ----
        Glyph::Swatch,
        Glyph::Rows,
        // ---- ws:text-titles ----
        Glyph::Titles,
        // ---- ws:docs-refresh ----
        // ---- ws:ui-kit ----
        Glyph::Speed,
        Glyph::History,
        Glyph::Proxy,
    ];

    /// Stable lower-case name of the variant, kept in sync with `from_name` - what a saved icon
    /// choice is stored as. Parameterized variants fold their direction into the name.
    pub fn name(self) -> &'static str {
        match self {
            Glyph::Cursor => "cursor",
            Glyph::Letter(_) => "letter",
            Glyph::Rect => "rect",
            Glyph::Ellipse => "ellipse",
            Glyph::Poly(_) => "poly",
            Glyph::Star => "star",
            Glyph::Line => "line",
            Glyph::Arrow => "arrow",
            Glyph::Pencil => "pencil",
            Glyph::Mask => "mask",
            Glyph::Zoom => "zoom",
            Glyph::Razor => "razor",
            Glyph::Flag => "flag",
            Glyph::Hourglass => "hourglass",
            Glyph::Eye => "eye",
            Glyph::EyeOff => "eye-off",
            Glyph::Diamond => "diamond",
            Glyph::Record => "record",
            Glyph::Mic => "mic",
            Glyph::Headphone => "headphone",
            Glyph::SpeakerOn => "speaker-on",
            Glyph::SpeakerOff => "speaker-off",
            Glyph::Camera => "camera",
            Glyph::FilmStrip => "film-strip",
            Glyph::Nodes => "nodes",
            Glyph::Layers => "layers",
            Glyph::Target => "target",
            Glyph::Magnet => "magnet",
            Glyph::Spacer => "spacer",
            Glyph::MusicNote => "music-note",
            Glyph::Folder => "folder",
            Glyph::Container => "container",
            Glyph::Cross => "cross",
            Glyph::Dot => "dot",
            Glyph::Copy => "copy",
            Glyph::Paste => "paste",
            Glyph::Tri(Dir::Up) => "tri-up",
            Glyph::Tri(Dir::Down) => "tri-down",
            Glyph::Tri(Dir::Left) => "tri-left",
            Glyph::Tri(Dir::Right) => "tri-right",
            Glyph::Skip(Dir::Up) => "skip-up",
            Glyph::Skip(Dir::Down) => "skip-down",
            Glyph::Skip(Dir::Left) => "skip-left",
            Glyph::Skip(Dir::Right) => "skip-right",
            Glyph::Jump(Dir::Up) => "jump-up",
            Glyph::Jump(Dir::Down) => "jump-down",
            Glyph::Jump(Dir::Left) => "jump-left",
            Glyph::Jump(Dir::Right) => "jump-right",
            Glyph::Play => "play",
            Glyph::Pause => "pause",
            Glyph::Stop => "stop",
            Glyph::Fullscreen => "fullscreen",
            Glyph::PopOut => "pop-out",
            Glyph::Indent(true) => "indent",
            Glyph::Indent(false) => "outdent",
            Glyph::Sequence => "sequence",
            Glyph::Template => "template",
            Glyph::Snowflake => "snowflake",
            Glyph::FilmReel => "film-reel",
            Glyph::Bolt => "bolt",
            Glyph::ImportArrow => "import",
            Glyph::ExportArrow => "export",
            Glyph::Transition => "transition",
            Glyph::Subtitles => "subtitles",
            Glyph::Gear => "gear",
            Glyph::Sliders => "sliders",
            Glyph::Wrench => "wrench",
            Glyph::Clapperboard => "clapperboard",
            Glyph::Waveform => "waveform",
            Glyph::CurveIcon => "curve",
            Glyph::Clock => "clock",
            Glyph::Notepad => "notepad",
            Glyph::Bookmark => "bookmark",
            Glyph::UndoArrow(Dir::Left) => "undo",
            Glyph::UndoArrow(_) => "redo",
            Glyph::FloppyDisk => "floppy-disk",
            Glyph::Terminal => "terminal",
            Glyph::Landscape => "landscape",
            Glyph::Portrait => "portrait",
            Glyph::Square => "square",
            Glyph::PlayRect => "play-rect",
            Glyph::GridIcon => "grid",
            Glyph::ListIcon => "list",
            Glyph::Guides => "guides",
            // ---- ws:registries-schema-hooks ----
            // ---- ws:size-diet ----
            // ---- ws:split-god-files ----
            // ---- ws:audio-analysis ----
            // ---- ws:audio-dsp-automation ----
            Glyph::Meter => "meter",
            // ---- ws:color-engine ----
            // ---- ws:command-palette ----
            Glyph::Keyboard => "keyboard",
            Glyph::Search => "search",
            // ---- ws:forgiveness ----
            // ---- ws:player-rate-loop ----
            // ---- ws:snap-engine ----
            Glyph::RollCursor => "roll-cursor",
            Glyph::SlipCursor => "slip-cursor",
            // ---- ws:trim-model ----
            // ---- ws:canvas-handles-monitor ----
            Glyph::Crop => "crop",
            Glyph::Rotate => "rotate",
            // ---- ws:export-deliver ----
            Glyph::Queue => "queue",
            // ---- ws:inspector-gallery ----
            Glyph::Wheel => "wheel",
            Glyph::Lut => "lut",
            // ---- ws:layout-modes-onboarding ----
            Glyph::Pin => "pin",
            Glyph::Maximize => "maximize",
            // ---- ws:media-library ----
            Glyph::Warning => "warning",
            Glyph::Chain => "chain",
            // ---- ws:source-monitor ----
            Glyph::Append => "append",
            Glyph::CloseUp => "close-up",
            Glyph::PlaceOnTop => "place-on-top",
            Glyph::SourceRecord => "source-record",
            Glyph::Tape => "tape",
            // ---- ws:timeline-trim-gestures ----
            Glyph::Lock => "lock",
            Glyph::Link => "link",
            // ---- ws:transcript-captions ----
            Glyph::Transcript => "transcript",
            // ---- ws:pro-monitor ----
            Glyph::Compare => "compare",
            Glyph::Scope => "scope",
            Glyph::Grid4 => "grid4",
            // ---- ws:pro-timeline ----
            Glyph::Swatch => "swatch",
            Glyph::Rows => "rows",
            // ---- ws:text-titles ----
            Glyph::Titles => "titles",
            // ---- ws:docs-refresh ----
            // ---- ws:ui-kit ----
            Glyph::Speed => "speed",
            Glyph::History => "history",
            Glyph::Proxy => "proxy",
        }
    }

    /// The reverse of `name` over `ALL` (so parameterized names come back as their representative).
    pub fn from_name(s: &str) -> Option<Glyph> {
        Self::ALL.iter().copied().find(|g| g.name() == s)
    }

    /// This glyph's symbol in Windows' icon font (Segoe Fluent Icons / Segoe MDL2 Assets codepoints,
    /// every one present in both). `None` = painted as a vector by `draw_glyph` instead.
    pub fn icon(self) -> Option<char> {
        let cp: u32 = match self {
            Glyph::Cursor => 0xE8B0,               // Click
            Glyph::Rect | Glyph::Square => 0xE739, // Checkbox (an empty square)
            Glyph::Ellipse => 0xEA3A,              // CircleRing
            Glyph::Star => 0xE734,                 // FavoriteStar
            Glyph::Line => 0xF7AF,                 // a thin diagonal stroke
            Glyph::Arrow => 0xE72A,                // Forward
            Glyph::Pencil => 0xE70F,               // Edit
            Glyph::Mask => 0xF16A,                 // a dotted ellipse: a marquee, not "contrast"
            Glyph::Zoom => 0xE71E,                 // Zoom
            Glyph::Razor => 0xE8C6,                // Cut
            Glyph::Flag => 0xE7C1,                 // Flag
            Glyph::Hourglass => 0xE916,            // Stopwatch
            Glyph::Eye => 0xE890,                  // View
            Glyph::EyeOff => 0xED1A,               // Hide
            Glyph::Record => 0xE91F,               // a filled circle (painted red)
            Glyph::Dot => 0xECCC,                  // a smaller filled dot
            Glyph::Mic => 0xE720,                  // Microphone
            Glyph::Headphone => 0xE7F6,            // Headphone
            Glyph::SpeakerOn => 0xE767,            // Volume
            Glyph::SpeakerOff => 0xE74F,           // Mute
            Glyph::Camera => 0xE722,               // Camera
            Glyph::FilmStrip => 0xE8B2,            // Movies
            Glyph::Nodes => 0xF22C,                // a node network
            Glyph::Layers => 0xE81E,               // MapLayers
            Glyph::Target => 0xF272,               // a bullseye
            Glyph::MusicNote => 0xEC4F,            // MusicNote
            Glyph::Folder => 0xE8B7,               // Folder
            Glyph::Container => 0xE7B8,            // Package
            Glyph::Cross => 0xE711,                // Cancel
            Glyph::Copy => 0xE8C8,                 // Copy
            Glyph::Paste => 0xE77F,                // Paste
            Glyph::Tri(Dir::Up) => 0xEDDB,         // filled caret triangles
            Glyph::Tri(Dir::Down) => 0xEDDC,
            Glyph::Tri(Dir::Left) => 0xEDD9,
            Glyph::Tri(Dir::Right) => 0xEDDA,
            Glyph::Skip(Dir::Left | Dir::Up) => 0xE627, // rewind
            Glyph::Skip(_) => 0xE628,                   // fast forward
            Glyph::Jump(Dir::Left | Dir::Up) => 0xF8AC, // previous (filled)
            Glyph::Jump(_) => 0xF8AD,                   // next (filled)
            Glyph::Play => 0xF5B0,                      // PlaySolid
            Glyph::Pause => 0xF8AE,                     // a filled pause
            Glyph::Stop => 0xE978,                      // a filled square
            Glyph::Fullscreen => 0xE740,                // FullScreen
            Glyph::PopOut => 0xE8A7,                    // OpenInNewWindow
            Glyph::Indent(true) => 0xE291,              // IncreaseIndent
            Glyph::Indent(false) => 0xE290,             // DecreaseIndent
            Glyph::Sequence => 0xF57B,                  // a lane carrying two blocks
            Glyph::Template => 0xE7C3,                  // Page
            Glyph::Snowflake => 0xEA38,                 // an asterisk snowflake
            Glyph::FilmReel => 0xE714,                  // Video
            Glyph::Bolt => 0xE945,                      // LightningBolt
            Glyph::ImportArrow => 0xE8B5,               // Import
            Glyph::ExportArrow => 0xEDE1,               // Export
            Glyph::Transition => 0xEF1F,                // two overlapping squares
            Glyph::Subtitles => 0xE7F0,                 // CC
            Glyph::Gear => 0xE713,                      // Settings
            Glyph::Sliders => 0xE9E9,                   // Equalizer
            Glyph::Wrench => 0xE90F,                    // Repair
            Glyph::Clapperboard => 0xE7F4,              // TVMonitor
            Glyph::Waveform => 0xF61F,                  // an audio waveform
            Glyph::CurveIcon => 0xEAFC,                 // a rising line graph
            Glyph::Clock => 0xE823,                     // Recent
            Glyph::Notepad => 0xE70B,                   // QuickNote
            Glyph::Bookmark => 0xE728,                  // FavoriteList
            Glyph::UndoArrow(Dir::Left) => 0xE7A7,      // Undo
            Glyph::UndoArrow(_) => 0xE7A6,              // Redo
            Glyph::FloppyDisk => 0xE74E,                // Save
            Glyph::Terminal => 0xE756,                  // CommandPrompt
            Glyph::Landscape => 0xF5A1,                 // a wide device outline
            Glyph::Portrait => 0xF59E,                  // a tall device outline
            Glyph::PlayRect => 0xE786,                  // Slideshow
            Glyph::GridIcon => 0xF0E2,                  // GridView
            Glyph::ListIcon => 0xE8FD,                  // BulletedList
            Glyph::Guides => 0xE9A6,                    // corner brackets
            Glyph::Meter => 0xE908,                     // rising bars
            Glyph::Keyboard => 0xE765,                  // KeyboardClassic
            Glyph::Search => 0xE721,                    // Search
            Glyph::Crop => 0xE7A8,                      // Crop
            Glyph::Rotate => 0xE7AD,                    // Rotate
            Glyph::Queue => 0xEE93,                     // a window with a clock badge
            Glyph::Wheel => 0xE790,                     // Color
            Glyph::Lut => 0xE793,                       // Light (brightness + contrast)
            Glyph::Pin => 0xE718,                       // Pin - a real pushpin
            Glyph::Maximize => 0xE1D9,                  // outward arrows
            Glyph::Warning => 0xE7BA,                   // Warning
            Glyph::Chain | Glyph::Link => 0xE71B,       // Link
            Glyph::Append => 0xE140,                    // an arrow into a box
            Glyph::CloseUp => 0xE73F,                   // BackToWindow (inward arrows)
            Glyph::PlaceOnTop => 0xE11C,                // an arrow up to a bar
            Glyph::SourceRecord => 0xE8AB,              // Switch
            Glyph::Tape => 0xE77C,                      // a cassette
            Glyph::Lock => 0xE72E,                      // Lock
            Glyph::Transcript => 0xE8BD,                // Message (a speech bubble of text)
            Glyph::Compare => 0xE746,                   // a half-filled square
            Glyph::Scope => 0xE9D9,                     // Diagnostic (a trace in a box)
            Glyph::Grid4 => 0xE8A9,                     // ViewAll
            Glyph::Swatch => 0xF354,                    // a filled palette
            Glyph::Rows => 0xE8E4,                      // AlignLeft
            Glyph::Titles => 0xE8D2,                    // Font
            Glyph::Speed => 0xE916,                     // Stopwatch (SpeedHigh's gauge is illegible at 14 px)
            Glyph::History => 0xE81C,                   // History
            Glyph::Proxy => 0xE895,                     // Sync
            // no symbol fits (a snapping magnet, a gap being pushed open, the trim cursors, an n-gon):
            // `draw_glyph` paints these; Letter is plain text
            Glyph::Letter(_) | Glyph::Poly(_) | Glyph::Magnet | Glyph::Spacer => return None,
            // the icon font's diamond is drawn slanted - paint the same one every keyframe uses
            Glyph::Diamond => return None,
            Glyph::RollCursor | Glyph::SlipCursor => return None,
        };
        char::from_u32(cp)
    }
}

// ---- ws:viewer-surface ----
/// Width of the viewer's tool rail, in points.
pub(crate) const RAIL_W: f32 = 32.0;
/// Rail row pitch: a 22 pt `icon_button` plus the gap under it.
const RAIL_ROW: f32 = 26.0;

/// The rail's picture of a shape tool.
fn shape_glyph(k: ShapeKind) -> Glyph {
    match k {
        ShapeKind::Rect => Glyph::Rect,
        ShapeKind::Ellipse => Glyph::Ellipse,
        ShapeKind::Triangle => Glyph::Poly(3),
        ShapeKind::Polygon => Glyph::Poly(5),
        ShapeKind::Star => Glyph::Star,
        ShapeKind::Line => Glyph::Line,
        ShapeKind::Arrow => Glyph::Arrow,
        ShapeKind::Draw => Glyph::Pencil,
    }
}

/// The rail's picture of a mask shape.
fn mask_glyph(m: MaskShape) -> Glyph {
    match m {
        MaskShape::Rect => Glyph::Rect,
        MaskShape::Ellipse => Glyph::Ellipse,
        MaskShape::Polygon => Glyph::Poly(5),
        MaskShape::Path => Glyph::Pencil,
    }
}

/// "Name (key)" with the live binding from the menu snapshot, or just the name when it is unbound.
fn with_key(name: &str, a: Action) -> String {
    match crate::ui::menu::shortcut(a) {
        k if k.is_empty() => name.to_string(),
        k => format!("{name} ({k})"),
    }
}

/// Tool selection (`Action::Tool*`) and Shift+S switch tools (Shift+S also steps through the shape
/// variants; the Mask action steps through the mask variants), ignored while a text field has focus.
/// Bare `S` is `Action::ToggleSnap`'s default, not a tool switch. Polled here rather than through the
/// app's main `Hotkeys::poll` / `App::act` because the tool strip, not `App`, owns `ToolsState`. Returns
/// the new tool when it changed; the key is consumed, so calling this twice in a frame is harmless.
/// The tool a `Action::Tool*` corresponds to, given the currently active tool (only `ToolMask` needs
/// it, to cycle the mask shape). `None` for any other action. Shared by `handle_hotkeys` below and by
/// `App::act`'s fallback arm for the rare case one of these actions fires through the general action
/// table instead of the tool strip's own poll (e.g. invoked via scripting/MCP).
/// The `ShapeStyle` a new shape clip will actually get from the current tool-strip picks - used by
/// BOTH `App::add_shape` (creation) and the preview's live drag preview, so the drawn preview and the
/// created clip can never drift apart in style.
pub fn shape_style_from_tools(tools: &ToolsState, kind: ShapeKind) -> ShapeStyle {
    let mut s = ShapeStyle::new(kind);
    s.fill = tools.fill;
    // line / arrow / drawing have no fill, so a transparent stroke would draw nothing at all:
    // fall back to the brush colour (and then the fill) instead of an invisible clip
    let stroke_only = matches!(kind, ShapeKind::Line | ShapeKind::Arrow | ShapeKind::Draw);
    s.stroke = match (stroke_only, tools.stroke[3], tools.brush[3]) {
        (true, 0, 0) => [tools.fill[0], tools.fill[1], tools.fill[2], 255],
        (true, 0, _) => tools.brush,
        _ => tools.stroke,
    };
    s.stroke_width = tools.stroke_width;
    s.sides = tools.sides;
    s.corner = tools.corner;
    s.draw_rate = tools.draw_rate;
    s.page = tools.page;
    s
}

pub fn tool_for_action(action: Action, cur: Tool) -> Option<Tool> {
    Some(match action {
        Action::ToolSelect => Tool::Select,
        Action::ToolText => Tool::Text,
        Action::ToolDraw => Tool::Draw,
        Action::ToolMask => Tool::Mask(next_mask(cur)),
        Action::ToolMarker => Tool::Marker,
        Action::ToolCut => Tool::Cut,
        Action::ToolStretch => Tool::Stretch,
        Action::ToolSpacer => Tool::Spacer,
        _ => return None,
    })
}

const TOOL_ACTIONS: [Action; 8] = [
    Action::ToolSelect,
    Action::ToolText,
    Action::ToolDraw,
    Action::ToolMask,
    Action::ToolMarker,
    Action::ToolCut,
    Action::ToolStretch,
    Action::ToolSpacer,
];

pub fn handle_hotkeys(ctx: &egui::Context, hotkeys: &Hotkeys, state: &mut ToolsState) -> Option<Tool> {
    if ctx.wants_keyboard_input() {
        return None;
    }
    ctx.input_mut(|i| {
        // Shift+S is RESERVED for this cycle (hotkeys.rs); Add Shape itself is unbound by default
        if crate::hotkeys::consume_exact(i, &egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::S)) {
            let next = Tool::Shape(next_shape(state.tool));
            state.tool = next;
            return Some(next);
        }
        // exact-modifier matching, so no ordering games are needed: a tool on bare C can never steal
        // Ctrl+C, and one on Shift+C never fires from Ctrl+Shift+C.
        for action in TOOL_ACTIONS {
            let tool = tool_for_action(action, state.tool).unwrap();
            if hotkeys.get(action).is_some_and(|ks| crate::hotkeys::consume_exact(i, &ks)) {
                return (tool != state.tool).then(|| {
                    state.tool = tool;
                    tool
                });
            }
        }
        None
    })
}

/// The shape tools in strip order (Draw has its own key, so it is not part of the S cycle).
const SHAPE_CYCLE: [ShapeKind; 7] = [
    ShapeKind::Rect,
    ShapeKind::Ellipse,
    ShapeKind::Triangle,
    ShapeKind::Polygon,
    ShapeKind::Star,
    ShapeKind::Line,
    ShapeKind::Arrow,
];

fn next_shape(cur: Tool) -> ShapeKind {
    match cur {
        Tool::Shape(k) => match SHAPE_CYCLE.iter().position(|c| *c == k) {
            Some(i) => SHAPE_CYCLE[(i + 1) % SHAPE_CYCLE.len()],
            None => SHAPE_CYCLE[0],
        },
        _ => SHAPE_CYCLE[0],
    }
}

fn next_mask(cur: Tool) -> MaskShape {
    match cur {
        Tool::Mask(m) => match MaskShape::ALL.iter().position(|c| *c == m) {
            Some(i) => MaskShape::ALL[(i + 1) % MaskShape::ALL.len()],
            None => MaskShape::ALL[0],
        },
        _ => MaskShape::ALL[0],
    }
}

// ---- ws:viewer-surface ----
/// The viewer's tool rail: a slim translucent column of icons at `at` (the viewer's top-left). Crop is
/// a mode of Select (the selection's edge handles crop instead of scaling), so the two light up
/// exclusively. Shape and Mask pick their last-used variant and open a flyout of the others.
pub(crate) fn rail(ui: &mut egui::Ui, state: &mut ToolsState, crop: &mut bool, palette: &Palette, at: egui::Pos2) {
    match state.tool {
        Tool::Shape(k) if k != ShapeKind::Draw => state.last_shape = k,
        Tool::Mask(m) => state.last_mask = m,
        _ => {}
    }
    let rect = egui::Rect::from_min_size(at, egui::vec2(RAIL_W, 6.0 * RAIL_ROW + 4.0));
    // a press between two buttons must not fall through to the video (it would start a drag there)
    ui.interact(rect, egui::Id::new("rail-bg"), Sense::click_and_drag());
    ui.painter().rect_filled(rect, CornerRadius::same(palette.rounding as u8 + 2), palette.panel.gamma_multiply(0.85));
    let layout = egui::Layout::top_down(egui::Align::Center);
    let mut ui = ui
        .new_child(egui::UiBuilder::new().id_salt("rail").max_rect(rect.shrink2(egui::vec2(0.0, 4.0))).layout(layout));
    ui.spacing_mut().item_spacing.y = RAIL_ROW - 22.0;
    let button = |ui: &mut egui::Ui, g: Glyph, name: &str, tip: &str, on: bool| {
        icon_button(ui, palette, egui::Id::new(("rail", name)), g, tip, on)
    };
    let select = state.tool == Tool::Select;
    if button(&mut ui, Glyph::Cursor, "Select", &with_key("Select", Action::ToolSelect), select && !*crop).clicked() {
        state.tool = Tool::Select;
        *crop = false;
    }
    let on = select && *crop;
    if button(&mut ui, Glyph::Crop, "Crop", "Crop: drag the selected clip's edges", on).clicked() {
        state.tool = Tool::Select;
        *crop = !on;
    }
    let tip = with_key("Text", Action::ToolText);
    if button(&mut ui, Glyph::Letter('T'), "Text", &tip, state.tool == Tool::Text).clicked() {
        state.tool = Tool::Text;
    }
    let on = matches!(state.tool, Tool::Shape(k) if k != ShapeKind::Draw);
    let r = button(&mut ui, shape_glyph(state.last_shape), "Shape", "Shapes (Shift+S cycles)", on);
    if r.clicked() {
        state.tool = Tool::Shape(state.last_shape);
    }
    flyout(&ui, &r, on, palette, |ui| {
        for k in SHAPE_CYCLE {
            if flyout_row(ui, palette, shape_glyph(k), k.name(), state.tool == Tool::Shape(k)) {
                (state.tool, state.last_shape) = (Tool::Shape(k), k);
            }
        }
    });
    if button(&mut ui, Glyph::Pencil, "Draw", &with_key("Draw", Action::ToolDraw), state.tool == Tool::Draw).clicked() {
        state.tool = Tool::Draw;
    }
    let on = matches!(state.tool, Tool::Mask(_));
    let r = button(&mut ui, Glyph::Mask, "Mask", &with_key("Mask", Action::ToolMask), on);
    if r.clicked() {
        state.tool = Tool::Mask(state.last_mask);
    }
    flyout(&ui, &r, on, palette, |ui| {
        for m in MaskShape::ALL {
            if flyout_row(ui, palette, mask_glyph(m), m.name(), state.tool == Tool::Mask(m)) {
                (state.tool, state.last_mask) = (Tool::Mask(m), m);
            }
        }
    });
}

/// A rail button's flyout: a corner mark on the button, and a menu to its right that its click opens
/// (a pick or a click elsewhere closes it).
fn flyout(ui: &egui::Ui, r: &egui::Response, on: bool, palette: &Palette, add: impl FnOnce(&mut egui::Ui)) {
    let c = r.rect.right_bottom() - egui::vec2(3.0, 3.0);
    let fg = if on { on_accent(palette.accent) } else { palette.text_dim };
    let mark = vec![c, c - egui::vec2(4.0, 0.0), c - egui::vec2(0.0, 4.0)];
    ui.painter().add(egui::Shape::convex_polygon(mark, fg, Stroke::NONE));
    egui::Popup::menu(r).align(egui::RectAlign::RIGHT_START).gap(10.0).show(add);
    // clear of the rail
}

/// One flyout row (a `ui::menu` row, so it lines up like every menu); the current pick is outlined.
fn flyout_row(ui: &mut egui::Ui, palette: &Palette, g: Glyph, name: &str, on: bool) -> bool {
    let r = crate::ui::menu::row(ui, Some(g), name, "");
    if on {
        let cr = CornerRadius::same(palette.rounding as u8);
        ui.painter().rect_stroke(r.rect, cr, Stroke::new(1.0, palette.accent), StrokeKind::Inside);
    }
    r.clicked()
}

/// Options of the active tool, for the strip along the viewer's top edge (nothing for a tool without
/// any). Returns true when anything changed.
pub(crate) fn style_controls(ui: &mut egui::Ui, state: &mut ToolsState, palette: &Palette) -> bool {
    let mut changed = false;
    match state.tool {
        Tool::Shape(kind) => {
            let outline_only = matches!(kind, ShapeKind::Line | ShapeKind::Arrow);
            if !outline_only {
                ui.label("Fill");
                changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.fill).changed();
            }
            ui.label("Stroke");
            changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.stroke).changed();
            changed |= ui
                .add(egui::DragValue::new(&mut state.stroke_width).speed(0.2).range(0.0..=200.0))
                .on_hover_text("Stroke width (px)")
                .changed();
            match kind {
                ShapeKind::Polygon | ShapeKind::Star => {
                    ui.label("Sides");
                    changed |= ui.add(egui::DragValue::new(&mut state.sides).range(3..=64)).changed();
                }
                ShapeKind::Rect => {
                    ui.label("Corner");
                    changed |= ui.add(egui::DragValue::new(&mut state.corner).speed(0.5).range(0.0..=2000.0)).changed();
                }
                ShapeKind::Arrow => {
                    ui.label("Head");
                    changed |= ui
                        .add(egui::DragValue::new(&mut state.corner).speed(0.5).range(0.0..=2000.0))
                        .on_hover_text("Arrow head size (px, 0 = follow the stroke width)")
                        .changed();
                }
                _ => {}
            }
        }
        Tool::Draw => {
            // play + record: the app starts the video and keeps every stroke of the take in one drawing
            let rec = state.recording;
            let tip = "Play and record: every stroke joins one drawing until the video stops";
            if icon_button(ui, palette, ui.id().with("draw-rec"), Glyph::Record, tip, rec).clicked() {
                state.recording = !rec;
                changed = true;
            }
            ui.label("Brush");
            changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.brush).changed();
            changed |= ui
                .add(egui::DragValue::new(&mut state.brush_width).speed(0.2).range(0.5..=200.0))
                .on_hover_text("Brush width (px)")
                .changed();
            ui.label("Speed");
            for (rate, label) in [(0.5, "0.5x"), (1.0, "1x"), (2.0, "2x"), (0.0, "all")] {
                let on = (state.draw_rate - rate).abs() < 1e-3;
                if ui
                    .selectable_label(on, label)
                    .on_hover_text(if rate == 0.0 {
                        "Show the whole sketch at once"
                    } else {
                        "Playback speed of the recorded drawing"
                    })
                    .clicked()
                    && !on
                {
                    state.draw_rate = rate;
                    changed = true;
                }
            }
            let mut page = state.page[3] > 0;
            if ui.checkbox(&mut page, "Page").changed() {
                state.page[3] = if page { 255 } else { 0 };
                if page && state.page[..3] == [0, 0, 0] {
                    state.page = [255, 255, 255, 255];
                }
                changed = true;
            }
            if page {
                changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.page).changed();
            }
        }
        // the mask shape is picked on the rail's flyout; the preview adds the mask target to the strip
        Tool::Mask(_) | Tool::Text | Tool::Select | Tool::Cut | Tool::Marker | Tool::Stretch | Tool::Spacer => {}
    }
    changed
}

/// A button showing `icon` and then `text`. An empty `text` gives a bare icon button, centred.
/// Signature frozen after ui-kit (wave-1 workstreams call it).
pub(crate) fn glyph_text_button(ui: &mut egui::Ui, icon: Glyph, text: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let icon_w = 18.0;
    let gap = if text.is_empty() { 0.0 } else { 4.0 };
    let size = egui::vec2(icon_w + gap + galley.size().x + pad.x * 2.0, galley.size().y.max(20.0) + pad.y * 2.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let v = ui.style().interact(&r);
    ui.painter().rect(rect, v.corner_radius, v.weak_bg_fill, v.bg_stroke, StrokeKind::Inside);
    let icon_x = if text.is_empty() { rect.center().x - icon_w / 2.0 } else { rect.left() + pad.x };
    let icon_rect = egui::Rect::from_min_size(egui::pos2(icon_x, rect.top()), egui::vec2(icon_w, rect.height()));
    draw_glyph(ui.painter(), icon_rect, icon, v.text_color());
    let tp = egui::pos2(icon_rect.right() + gap, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(tp, galley, v.text_color());
    r
}

/// Bare square icon button: accent fill when active, a hover outline otherwise.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    icon: Glyph,
    tip: &str,
    active: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    let cr = CornerRadius::same(palette.rounding as u8);
    if active {
        p.rect_filled(rect, cr, palette.accent);
    } else if r.hovered() {
        p.rect_filled(rect, cr, palette.header);
        p.rect_stroke(rect, cr, Stroke::new(1.0, palette.border), StrokeKind::Inside);
    }
    let fg = if active { on_accent(palette.accent) } else { palette.text };
    draw_glyph(p, rect, icon, fg);
    r.on_hover_text(tip)
}

/// Icon-font size `draw_glyph` paints at: the same ~13 px footprint the old vector glyphs had.
pub(crate) const ICON_PX: f32 = 14.0;

/// Paint one tool glyph centred in `rect` (a 24x22 button) in `fg`: its icon-font symbol, or the
/// vector drawing for the few with none (`Glyph::icon`).
pub(crate) fn draw_glyph(p: &egui::Painter, rect: egui::Rect, g: Glyph, fg: Color32) {
    let c = rect.center();
    if let Some(ch) = g.icon() {
        // Record is the one glyph with a colour of its own
        let fg = if g == Glyph::Record { Color32::from_rgb(220, 60, 60) } else { fg };
        // a context `theme::apply` never ran on (a bare test harness) has no icon family, and egui
        // panics on an unbound family - fall back to the default one there
        let icons = crate::theme::icons();
        let family = match p.fonts(|f| f.definitions().families.contains_key(&icons)) {
            true => icons,
            false => egui::FontFamily::Proportional,
        };
        p.text(c, Align2::CENTER_CENTER, ch, FontId::new(ICON_PX, family), fg);
        return;
    }
    let r = 6.0; // half-extent of the drawn icon
    let stroke = Stroke::new(1.4, fg);
    match g {
        Glyph::Letter(ch) => {
            p.text(c, Align2::CENTER_CENTER, ch, FontId::proportional(13.0), fg);
        }
        Glyph::Poly(n) => {
            let pts = (0..n)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n as f32;
                    c + egui::vec2(a.cos() * r, a.sin() * r)
                })
                .collect();
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        // spacer: two posts with a double-headed arrow pushing them apart
        Glyph::Spacer => {
            for x in [-6.5, 6.5] {
                p.line_segment([c + egui::vec2(x, -6.0), c + egui::vec2(x, 6.0)], stroke);
            }
            p.line_segment([c + egui::vec2(-4.5, 0.0), c + egui::vec2(4.5, 0.0)], stroke);
            for (tip, back) in [(-5.5f32, -2.0f32), (5.5, 2.0)] {
                let head = vec![c + egui::vec2(tip, 0.0), c + egui::vec2(back, -3.0), c + egui::vec2(back, 3.0)];
                p.add(egui::Shape::convex_polygon(head, fg, Stroke::NONE));
            }
        }
        // horseshoe magnet: a U-shaped body with a pole cap on each leg tip
        Glyph::Magnet => {
            let arc_c = c + egui::vec2(0.0, -1.0);
            let arc_r = 5.0;
            let arc: Vec<egui::Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 12.0;
                    arc_c + egui::vec2(a.cos() * arc_r, a.sin() * arc_r)
                })
                .collect();
            p.add(egui::Shape::line(arc, stroke));
            p.line_segment([arc_c + egui::vec2(-arc_r, 0.0), arc_c + egui::vec2(-arc_r, 6.5)], stroke);
            p.line_segment([arc_c + egui::vec2(arc_r, 0.0), arc_c + egui::vec2(arc_r, 6.5)], stroke);
            for x in [-arc_r, arc_r] {
                p.rect_filled(
                    egui::Rect::from_center_size(arc_c + egui::vec2(x, 6.5), egui::vec2(3.0, 2.6)),
                    CornerRadius::same(1),
                    fg,
                );
            }
        }
        // roll: a curved arrow wrapped around a vertical bar (the cut) - rolling the edit point.
        Glyph::RollCursor => {
            p.line_segment([c + egui::vec2(0.0, -6.0), c + egui::vec2(0.0, 6.0)], Stroke::new(1.6, fg));
            let arc: Vec<egui::Pos2> = (0..=10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * 1.4 * i as f32 / 10.0;
                    c + egui::vec2(4.5, 0.0) + egui::vec2(a.cos() * 4.0, a.sin() * 4.0)
                })
                .collect();
            p.add(egui::Shape::closed_line(arc.clone(), stroke));
            if let (Some(&a), Some(&b)) = (arc.first(), arc.get(1)) {
                let d = (b - a).normalized();
                let n = egui::vec2(-d.y, d.x);
                p.add(egui::Shape::convex_polygon(vec![a + d * 3.0, a - n * 2.5, a + n * 2.5], fg, Stroke::NONE));
            }
        }
        // slip: two filmstrip frames sliding sideways under a fixed bracket.
        Glyph::SlipCursor => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(13.0, 9.0)),
                CornerRadius::ZERO,
                stroke,
                StrokeKind::Inside,
            );
            for dx in [-6.5f32, 0.0, 6.5] {
                p.line_segment([c + egui::vec2(dx, -6.5), c + egui::vec2(dx, -4.5)], stroke);
                p.line_segment([c + egui::vec2(dx, 4.5), c + egui::vec2(dx, 6.5)], stroke);
            }
            let head = vec![c + egui::vec2(-7.5, 0.0), c + egui::vec2(-4.5, -2.2), c + egui::vec2(-4.5, 2.2)];
            p.add(egui::Shape::convex_polygon(head, fg, Stroke::NONE));
            let head = vec![c + egui::vec2(7.5, 0.0), c + egui::vec2(4.5, -2.2), c + egui::vec2(4.5, 2.2)];
            p.add(egui::Shape::convex_polygon(head, fg, Stroke::NONE));
        }
        Glyph::Diamond => {
            p.add(diamond(c, 5.0, fg, Stroke::NONE));
        }
        _ => {} // every other glyph has an icon (`every_glyph_paints_a_picture` holds that)
    }
}

/// THE keyframe diamond (Inspector ◆, timeline keys, curve editor, the Diamond glyph): a square turned
/// 45° with equal half-extents, its centre and size snapped to whole points so the four edges
/// anti-alias identically instead of leaning (the icon font's glyph did).
pub(crate) fn diamond(c: egui::Pos2, r: f32, fill: Color32, stroke: Stroke) -> egui::Shape {
    let (c, r) = (c.round(), r.round().max(1.0));
    egui::Shape::convex_polygon(
        vec![c - egui::vec2(0.0, r), c + egui::vec2(r, 0.0), c + egui::vec2(0.0, r), c - egui::vec2(r, 0.0)],
        fill,
        stroke,
    )
}

/// Built-in icon for a menu action (None = text-only). The user's Settings → Appearance → Icons
/// override wins over these; abstract actions stay text-only on purpose.
pub(crate) fn action_glyph(a: crate::hotkeys::Action) -> Option<Glyph> {
    use crate::hotkeys::Action::*;
    Some(match a {
        NewProject => Glyph::Template,
        OpenFile | OpenProject => Glyph::Folder,
        Save | SaveProjectAs => Glyph::FloppyDisk,
        ExportVideo | ExportLossless | ExportXml => Glyph::ExportArrow,
        ImportMedia => Glyph::ImportArrow,
        CopyClips => Glyph::Copy,
        CutClips => Glyph::Razor,
        PasteClips => Glyph::Paste,
        Settings => Glyph::Gear,
        Undo => Glyph::UndoArrow(Dir::Left),
        Redo => Glyph::UndoArrow(Dir::Right),
        PlayPause => Glyph::Play,
        Stop => Glyph::Stop,
        Split => Glyph::Razor,
        AddText => Glyph::Letter('T'),
        AddMarker => Glyph::Flag,
        Retime => Glyph::Speed,
        Fullscreen => Glyph::Fullscreen,
        ScreenCapture => Glyph::Camera,
        ToolSelect => Glyph::Cursor,
        ToolText => Glyph::Letter('T'),
        ToolDraw => Glyph::Pencil,
        ToolMask => Glyph::Mask,
        ToolMarker => Glyph::Flag,
        ToolCut => Glyph::Razor,
        ToolStretch => Glyph::Speed,
        _ => return None,
    })
}

/// Paint `icon` where a plain label would go - no button chrome, no hit area.
pub(crate) fn glyph_label(ui: &mut egui::Ui, icon: Glyph, color: Color32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(18.0, ui.spacing().interact_size.y), Sense::hover());
    draw_glyph(ui.painter(), rect, icon, color);
    r
}

/// The label-colour swatch every label menu shows: a filled round chip. A `Button`, so a caller can
/// click it, select it or hang a menu off it.
pub(crate) fn color_chip<'a>(color: Color32, selected: bool, palette: &Palette) -> egui::Button<'a> {
    let ring = if selected { Stroke::new(2.0, palette.accent) } else { Stroke::new(1.0, palette.border) };
    egui::Button::new("").fill(color).stroke(ring).corner_radius(CornerRadius::same(8)).min_size(egui::vec2(15.0, 15.0))
}

/// Readable text colour on top of the accent fill.
pub(crate) fn on_accent(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, Rect, Vec2};

    struct Harness {
        ctx: egui::Context,
        state: ToolsState,
        crop: bool,
        hotkeys: Hotkeys,
        time: f64,
        /// Last frame's painted shapes, to find a flyout row by its text.
        shapes: Vec<egui::epaint::ClippedShape>,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            // size-diet: dropping eframe's `default_fonts` feature left a bare Context with no glyphs
            // at all, and the rail's tooltips / flyout rows need real metrics - see theme::test_fonts.
            ctx.set_fonts(crate::theme::test_fonts());
            let mut h = Self {
                ctx,
                state: ToolsState::default(),
                crop: false,
                hotkeys: Hotkeys::defaults(),
                time: 0.0,
                shapes: Vec::new(),
            };
            h.frame(vec![]);
            h
        }
        /// One frame the way `App::update` runs it: the tool keys are polled, then the rail is drawn.
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
            let Harness { ctx, state, crop, hotkeys, .. } = self;
            let out = ctx.run(input, |ctx| {
                handle_hotkeys(ctx, hotkeys, state);
                egui::CentralPanel::default().show(ctx, |ui| rail(ui, state, crop, &pal, Pos2::new(10.0, 10.0)));
            });
            self.shapes = out.shapes;
        }
        /// Centre of a rail button by its name, from the previous frame's layout.
        fn button(&self, name: &str) -> Pos2 {
            self.ctx
                .read_response(egui::Id::new(("rail", name)))
                .unwrap_or_else(|| panic!("no button {name}"))
                .rect
                .center()
        }
        /// Centre of the painted text `s` (a flyout row), from the previous frame.
        fn text(&self, s: &str) -> Pos2 {
            self.shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.job.text == s => Some(t.pos + t.galley.rect.center().to_vec2()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no text {s}"))
        }
        fn click_at(&mut self, pos: Pos2) {
            self.time += 1.0; // separate clicks, never a double-click
            self.frame(vec![Event::PointerMoved(pos)]);
            let press = |pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![press(true)]);
            self.frame(vec![press(false)]);
            self.frame(vec![]); // a flyout opened by the click shows its rows
        }
        fn click(&mut self, name: &str) {
            self.click_at(self.button(name));
        }
        fn key(&mut self, key: Key) {
            self.key_mod(key, Modifiers::NONE);
        }
        fn key_mod(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
            ]);
        }
    }

    /// Every variant (via `Glyph::ALL`), so a new one cannot be added without deciding what it looks
    /// like - and without giving it a name for the icon picker.
    const ALL_GLYPHS: &[Glyph] = Glyph::ALL;

    /// Tessellated vertices produced by `paint`, on a throwaway context.
    fn painted(paint: impl Fn(&egui::Painter, Rect)) -> usize {
        let ctx = egui::Context::default();
        // size-diet: `Glyph::Letter` paints a real character via the font system, which needs a real
        // font loaded now that eframe's `default_fonts` feature is gone - see theme::test_fonts.
        ctx.set_fonts(crate::theme::test_fonts());
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0))),
            ..Default::default()
        };
        let mut run = || {
            ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(24.0, 22.0));
                    ui.allocate_space(rect.size());
                    paint(ui.painter(), rect);
                });
            })
        };
        run();
        let out = run();
        ctx.tessellate(out.shapes, 1.0)
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                _ => 0,
            })
            .sum()
    }

    /// The keyframe diamond is a true square-on-its-corner (equal half-extents, whole-point centre), and
    /// the Diamond glyph paints that instead of the icon font's slanted one.
    #[test]
    fn keyframe_diamond_is_symmetric_and_pixel_snapped() {
        let egui::Shape::Path(path) = diamond(egui::pos2(10.3, 20.6), 4.4, Color32::WHITE, Stroke::NONE) else {
            panic!("not a path")
        };
        let p = &path.points;
        assert_eq!(p[0], egui::pos2(10.0, 17.0));
        assert_eq!(p[1], egui::pos2(14.0, 21.0));
        assert_eq!(p[2], egui::pos2(10.0, 25.0));
        assert_eq!(p[3], egui::pos2(6.0, 21.0));
        assert!(Glyph::Diamond.icon().is_none(), "the icon font's diamond leans");
    }

    #[test]
    fn every_glyph_paints_a_picture() {
        // a variant that paints nothing (no icon and no vector arm) would be a blank button
        let empty = painted(|_, _| {});
        for g in ALL_GLYPHS {
            let n = painted(|p, rect| draw_glyph(p, rect, *g, Color32::WHITE));
            assert!(n > empty, "{g:?} painted nothing ({n} vs {empty} vertices)");
        }
    }

    // ---- ws:ui-kit ----
    /// Every `Glyph::icon` codepoint is in BOTH Windows icon fonts (Segoe Fluent Icons on 11, Segoe
    /// MDL2 Assets on 10) - a missing one falls through to Segoe UI's tofu box. A font that isn't
    /// installed on this machine is skipped, not failed.
    #[test]
    fn every_icon_exists_in_both_fonts() {
        let dir = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into()));
        let mut checked = 0;
        for file in ["SegoeIcons.ttf", "segmdl2.ttf"] {
            let Ok(bytes) = std::fs::read(dir.join("Fonts").join(file)) else { continue };
            let mut defs = egui::FontDefinitions::empty();
            defs.font_data.insert(file.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            defs.families.insert(egui::FontFamily::Proportional, vec![file.into()]);
            let ctx = egui::Context::default();
            ctx.set_fonts(defs);
            let _ = ctx.run(egui::RawInput::default(), |_| {});
            for g in Glyph::ALL {
                let Some(c) = g.icon() else { continue };
                let ok = ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(ICON_PX), c));
                assert!(ok, "{g:?} -> U+{:04X} is missing from {file}", c as u32);
                checked += 1;
            }
        }
        assert!(checked > 0, "neither icon font is installed");
    }

    /// Not a check: `SE_GLYPH_SHEET=<out.png> cargo test glyph_sheet -- --ignored` paints every glyph
    /// (its 24x22 button box outlined, name beside it) through egui's own tessellator and font atlas
    /// into a PNG at 2x, for eyeballing size/alignment after an icon change.
    #[test]
    #[ignore]
    fn glyph_sheet() {
        let (cols, cell, ppp) = (4, Vec2::new(170.0, 26.0), 2.0);
        let size = Vec2::new(cols as f32 * cell.x, Glyph::ALL.len().div_ceil(cols) as f32 * cell.y);
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut atlas = egui::ColorImage::new([1, 1], vec![Color32::WHITE]);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let mut input =
                egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_gray(32))).show(ctx, |ui| {
                    for (i, g) in Glyph::ALL.iter().enumerate() {
                        let o = Pos2::new((i % cols) as f32 * cell.x + 2.0, (i / cols) as f32 * cell.y + 2.0);
                        let r = Rect::from_min_size(o, Vec2::new(24.0, 22.0));
                        ui.painter().rect_stroke(r, 0.0, Stroke::new(0.5, Color32::from_gray(90)), StrokeKind::Inside);
                        draw_glyph(ui.painter(), r, *g, Color32::WHITE);
                        let at = r.right_center() + Vec2::new(6.0, 0.0);
                        ui.painter().text(at, Align2::LEFT_CENTER, g.name(), FontId::proportional(12.0), Color32::GRAY);
                    }
                });
            });
            for (_, d) in out.textures_delta.set.iter().filter(|(id, _)| *id == egui::TextureId::default()) {
                let egui::ImageData::Color(img) = &d.image;
                match d.pos {
                    None => atlas = (**img).clone(),
                    Some([x0, y0]) => {
                        for (i, px) in img.pixels.iter().enumerate() {
                            atlas[(x0 + i % img.size[0], y0 + i / img.size[0])] = *px;
                        }
                    }
                }
            }
            shapes = out.shapes;
        }
        // ponytail: nearest-texel, no clip rects - enough to judge glyphs, not a general renderer
        let (w, h) = ((size.x * ppp) as usize, (size.y * ppp) as usize);
        let mut buf = vec![[0f32; 4]; w * h];
        let f = |c: Color32| c.to_array().map(|v| v as f32 / 255.0);
        for prim in ctx.tessellate(shapes, ppp) {
            let egui::epaint::Primitive::Mesh(m) = prim.primitive else { continue };
            for t in m.indices.chunks_exact(3) {
                let v = [0, 1, 2].map(|k| m.vertices[t[k] as usize]);
                let p = v.map(|v| v.pos * ppp);
                let area = (p[1] - p[0]).x * (p[2] - p[0]).y - (p[1] - p[0]).y * (p[2] - p[0]).x;
                if area.abs() < 1e-6 {
                    continue;
                }
                let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
                for y in (lo.y.floor().max(0.0) as usize)..(hi.y.ceil() as usize).min(h) {
                    for x in (lo.x.floor().max(0.0) as usize)..(hi.x.ceil() as usize).min(w) {
                        let q = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let e = |a: Pos2, b: Pos2| ((b - a).x * (q - a).y - (b - a).y * (q - a).x) / area;
                        let l = [e(p[1], p[2]), e(p[2], p[0]), e(p[0], p[1])];
                        if l.iter().any(|&l| l < 0.0) {
                            continue;
                        }
                        let uv = v[0].uv.to_vec2() * l[0] + v[1].uv.to_vec2() * l[1] + v[2].uv.to_vec2() * l[2];
                        let [aw, ah] = atlas.size;
                        let texel = atlas
                            [(((uv.x * aw as f32) as usize).min(aw - 1), ((uv.y * ah as f32) as usize).min(ah - 1))];
                        let (tx, c) = (f(texel), [0, 1, 2].map(|k| f(v[k].color)));
                        let src: [f32; 4] =
                            std::array::from_fn(|i| (c[0][i] * l[0] + c[1][i] * l[1] + c[2][i] * l[2]) * tx[i]);
                        let dst = &mut buf[y * w + x];
                        *dst = std::array::from_fn(|i| src[i] + dst[i] * (1.0 - src[3]));
                    }
                }
            }
        }
        let rgba = buf
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8))
            .collect();
        let frame = crate::media::Frame { width: w as u32, height: h as u32, pts: 0.0, rgba };
        let out = std::env::var("SE_GLYPH_SHEET").unwrap_or_else(|_| "glyph-sheet.png".into());
        std::fs::write(&out, crate::mcp::png_encode(&frame)).unwrap();
        eprintln!("glyph sheet: {out}");
    }

    // ---- ws:viewer-surface ----
    const RAIL: [&str; 6] = ["Select", "Crop", "Text", "Shape", "Draw", "Mask"];

    /// The rail is one slim column: every viewer tool, top to bottom, inside `RAIL_W`.
    #[test]
    fn rail_stacks_the_six_viewer_tools() {
        let h = Harness::new();
        let at: Vec<Pos2> = RAIL.iter().map(|n| h.button(n)).collect();
        for (w, n) in at.windows(2).zip(RAIL.iter().skip(1)) {
            assert!(w[1].y > w[0].y + 20.0, "{n} sits below the one above it: {w:?}");
            assert!((w[1].x - w[0].x).abs() < 0.5, "{n} is in the same column");
        }
        assert!(at.iter().all(|p| p.x > 10.0 && p.x < 10.0 + RAIL_W), "inside the rail: {at:?}");
    }

    #[test]
    fn rail_clicks_switch_tools() {
        let mut h = Harness::new();
        assert_eq!(h.state.tool, Tool::Select);
        h.click("Text");
        assert_eq!(h.state.tool, Tool::Text);
        h.click("Draw");
        assert_eq!(h.state.tool, Tool::Draw);
        h.click("Mask");
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Rect));
        h.click("Shape");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Rect));
        h.click("Select");
        assert_eq!(h.state.tool, Tool::Select);
    }

    /// Crop is Select with the crop handles on: picking it from any tool lands on Select, a second
    /// click (or Select) turns the handles back off.
    #[test]
    fn crop_is_a_mode_of_select() {
        let mut h = Harness::new();
        h.click("Draw");
        h.click("Crop");
        assert_eq!((h.state.tool, h.crop), (Tool::Select, true));
        h.click("Crop");
        assert_eq!((h.state.tool, h.crop), (Tool::Select, false), "a second click turns it off");
        h.click("Crop");
        h.click("Select");
        assert_eq!((h.state.tool, h.crop), (Tool::Select, false), "Select is the plain handles");
        h.click("Crop");
        h.click("Text");
        h.click("Crop");
        assert!(h.crop && h.state.tool == Tool::Select, "back from another tool: crop on again");
    }

    /// The Shape / Mask buttons pick the variant used last - from the flyout or from the keys.
    #[test]
    fn shape_and_mask_buttons_remember_the_last_variant() {
        let mut h = Harness::new();
        h.key_mod(Key::S, Modifiers::SHIFT);
        h.key_mod(Key::S, Modifiers::SHIFT);
        h.key(Key::V);
        h.click("Shape");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Ellipse), "Shift+S left it on Ellipse");
        // the click opened the flyout: pick Star there
        h.click_at(h.text("Star"));
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Star));
        h.key(Key::V);
        h.click("Shape");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Star), "the flyout pick is remembered");
        h.key(Key::G);
        h.key(Key::G);
        h.key(Key::V);
        h.click("Mask");
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Ellipse), "G twice left it on Ellipse");
        h.click_at(h.text("Path"));
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Path));
    }

    /// Tooltips carry the live binding from the menu snapshot (a rebind shows up there).
    #[test]
    fn rail_tooltips_read_the_live_keys() {
        let hk = Hotkeys::defaults();
        crate::ui::menu::publish(crate::ui::menu::MenuSnapshot::new(&hk, &Default::default(), |_| Ok(())));
        assert_eq!(with_key("Select", Action::ToolSelect), "Select (V)");
        assert_eq!(with_key("Draw", Action::ToolDraw), "Draw (D)");
        assert_eq!(with_key("Mask", Action::ToolMask), "Mask (G)", "freed from M");
        assert_eq!(with_key("Spacer", Action::ToolSpacer), "Spacer", "unbound: no key");
        crate::ui::menu::publish(Default::default());
    }

    #[test]
    fn single_key_shortcuts_switch_tools() {
        let mut h = Harness::new();
        h.key(Key::T);
        assert_eq!(h.state.tool, Tool::Text);
        h.key(Key::D);
        assert_eq!(h.state.tool, Tool::Draw);
        h.key(Key::C);
        assert_eq!(h.state.tool, Tool::Cut);
        h.key(Key::R);
        assert_eq!(h.state.tool, Tool::Stretch);
        h.key(Key::V);
        assert_eq!(h.state.tool, Tool::Select);
        h.key_mod(Key::S, Modifiers::SHIFT);
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Rect));
        h.key_mod(Key::S, Modifiers::SHIFT);
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Ellipse), "Shift+S steps through the shapes");
        // Mask moved off bare M (freed for the Marker tool below) to G, cycling shape same as before
        h.key(Key::G);
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Rect));
        h.key(Key::G);
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Ellipse), "G steps through the mask shapes");
        h.key_mod(Key::M, Modifiers::SHIFT);
        assert_eq!(h.state.tool, Tool::Marker, "Shift+M selects the Marker tool (bare M adds a marker)");
    }

    /// Regression: the tool poll must match modifiers EXACTLY. egui's `consume_shortcut` matches
    /// "logically" (extra Shift/Alt ignored), and since this poll runs before the `Hotkeys` table a
    /// logical match ate every `Shift+<tool letter>` action - Shift+T (Add Text), Shift+D, Shift+R,
    /// and the old Shift+M (Add Marker) all selected tools instead of firing.
    #[test]
    fn shifted_letters_are_left_for_the_action_table() {
        let mut h = Harness::new();
        h.state.tool = Tool::Select;
        for key in [Key::T, Key::D, Key::R, Key::C, Key::V] {
            h.key_mod(key, Modifiers::SHIFT);
            assert_eq!(h.state.tool, Tool::Select, "Shift+{key:?} must not switch tools");
            h.key_mod(key, Modifiers::CTRL);
            assert_eq!(h.state.tool, Tool::Select, "Ctrl+{key:?} must not switch tools");
        }
        // bare M is Add Marker's key now, not a tool
        h.key(Key::M);
        assert_eq!(h.state.tool, Tool::Select, "bare M is left for Action::AddMarker");
    }

    /// Bare S is `Action::ToggleSnap`'s (rebindable) default, so the tool poll leaves it alone.
    #[test]
    fn bare_s_is_left_for_the_snap_action() {
        let mut h = Harness::new();
        h.state.tool = Tool::Draw;
        h.key(Key::S);
        assert_eq!(h.state.tool, Tool::Draw, "bare S must not touch the active tool");
    }

    #[test]
    fn modified_keys_are_left_alone() {
        let mut h = Harness::new();
        h.frame(vec![Event::Key {
            key: Key::V,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }]);
        assert_eq!(h.state.tool, Tool::Select);
        h.state.tool = Tool::Draw;
        h.frame(vec![Event::Key {
            key: Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }]);
        assert_eq!(h.state.tool, Tool::Draw, "Ctrl+S must stay the save shortcut");
    }

    #[test]
    fn hotkeys_are_consumed_once() {
        // the key is consumed, so a second poll in the same frame is a no-op
        let mut state = ToolsState::default();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![Event::Key {
                key: Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            }],
            ..Default::default()
        };
        let hotkeys = Hotkeys::defaults();
        let _ = ctx.run(input, |ctx| {
            assert!(handle_hotkeys(ctx, &hotkeys, &mut state).is_some());
            assert!(handle_hotkeys(ctx, &hotkeys, &mut state).is_none(), "key already consumed");
        });
        assert_eq!(state.tool, Tool::Shape(ShapeKind::Rect), "Shift+S is the shape cycle");
    }

    #[test]
    fn every_tool_renders_its_style_controls() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
        let mut tools =
            vec![Tool::Select, Tool::Text, Tool::Draw, Tool::Cut, Tool::Marker, Tool::Stretch, Tool::Spacer];
        tools.extend(ShapeKind::ALL.map(Tool::Shape));
        tools.extend(MaskShape::ALL.map(Tool::Mask));
        for t in tools {
            let mut state = ToolsState { tool: t, ..Default::default() };
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| assert!(!style_controls(ui, &mut state, &pal), "{t:?} reports no change"));
                });
            });
            assert_eq!(state.tool, t, "{t:?} controls must not change the tool on their own");
        }
    }
}
