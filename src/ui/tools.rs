//! Tool bar: a thin, movable strip that sits between the viewport and the timeline (its own dockable
//! pane, so it can also be popped out). One row of icon buttons:
//!   Select (V) · Text (T) · Rectangle · Ellipse · Triangle · Polygon · Star · Line · Arrow · Draw (D) ·
//!   Mask (rect/ellipse/polygon/path) · Zoom
//! plus a magnet button that lights up while snapping is on (S, or Settings.snap's own `N` shortcut),
//! then, for shape tools, fill and stroke colour buttons and a stroke-width DragValue; for Draw, a
//! play/record button, the brush colour/width, the playback speed (0.5x / 1x / 2x) and a page toggle.
//!
//! The active tool changes what a click-drag in the Preview does (see `ui::preview`): Select edits the
//! selected clip, a shape tool drags out a new Shape clip at the playhead (Shift locks its aspect ratio,
//! Alt grows it from the press point instead of corner-to-corner), Draw records strokes while the mouse
//! is down (timed, so the sketch replays), Mask edits the selected effect's / clip's mask.

use crate::model::{MaskShape, ShapeKind};
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
    Zoom,
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
    /// Draw tool: a take is running — the app plays the video and drops every stroke into one drawing
    /// until this goes back off (see `App::toggle_draw_recording`).
    pub recording: bool,
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

impl Dir {
    /// Unit vector along the pointing direction (y grows downward, as everywhere in egui).
    fn unit(self) -> egui::Vec2 {
        match self {
            Dir::Up => egui::vec2(0.0, -1.0),
            Dir::Down => egui::vec2(0.0, 1.0),
            Dir::Left => egui::vec2(-1.0, 0.0),
            Dir::Right => egui::vec2(1.0, 0.0),
        }
    }
}

/// The strip, left to right: (tool, icon, name). `Tool::Mask` stands for whichever mask shape is
/// selected (the combo next to it picks one), the shape tools each get their own button.
/// What `icon_button` draws. Painted with the painter, not typed: nearly every obvious character
/// (polygon, pencil, half-disc, magnifier, close, play, reorder arrows) is missing from Segoe UI and
/// egui's bundled fonts and came out as a tofu box.
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
    /// Two patch boxes joined by a cable — a node graph.
    Nodes,
    /// A stack of sheets — an adjustment layer.
    Layers,
    /// A shooting target: rings and cross ticks — motion tracking.
    Target,
    /// A horseshoe magnet — the snapping toggle.
    Magnet,
    /// Two posts with a double-headed arrow between them — a gap being widened.
    Spacer,
    /// An eighth note — the audio-effects catalogue card (no picture to render for those).
    MusicNote,
    /// A file-explorer folder: a tab sitting on a body.
    Folder,
    /// A container / slot clip.
    Container,
    /// Two crossed strokes — close, delete, clear.
    Cross,
    /// A filled dot — a colour swatch, a bullet, "in use".
    Dot,
    /// Two sheets, one behind the other — copy.
    Copy,
    /// A clipboard — paste.
    Paste,
    /// A filled triangle pointing `Dir` — reorder, collapse, step one frame.
    Tri(Dir),
    /// Two triangles — the previous / next cut.
    Skip(Dir),
    /// A triangle backed against a bar — go to the very start / end.
    Jump(Dir),
    Play,
    Pause,
    Stop,
    /// Four corner brackets — fullscreen.
    Fullscreen,
    /// A window with an arrow leaving it — pop this pane out.
    PopOut,
    /// An arrow into a margin bar — indent (true) / outdent (false).
    Indent(bool),
    /// A lane carrying two blocks — a nested sequence.
    Sequence,
    /// A card with a folded corner — a saved clip template.
    Template,
    /// A six-armed snowflake — a frozen frame.
    Snowflake,
    /// A movie reel: rim, hub, four spoke holes and a tape tail.
    FilmReel,
    /// A filled lightning zigzag — effects / performance.
    Bolt,
    /// An open tray with an arrow dropping into it — import.
    ImportArrow,
    /// The same tray with the arrow rising out — export.
    ExportArrow,
    /// Two overlapping squares with a diagonal across the overlap — a transition.
    Transition,
    /// A caption box with two text bars in its lower half.
    Subtitles,
    /// A cog: ring, eight stubs and a hub — settings.
    Gear,
    /// Three slider tracks, each with its knob at a different position.
    Sliders,
    /// An open-ended wrench head with a diagonal handle.
    Wrench,
    /// A clapperboard: body plus a slanted, hatched top bar.
    Clapperboard,
    /// Five vertical bars around a midline — an audio waveform.
    Waveform,
    /// Axes with a rising curve and two square handles — a value curve.
    CurveIcon,
    /// A clock face with two hands.
    Clock,
    /// A sheet with three ruled lines.
    Notepad,
    /// A ribbon with a notched V bottom.
    Bookmark,
    /// A curved arrow — undo (`Dir::Left`) / redo (`Dir::Right`).
    UndoArrow(Dir),
    /// A save disk: notched square, shutter and label.
    FloppyDisk,
    /// A console window: '>' prompt and an underscore.
    Terminal,
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
    ];

    /// Stable lower-case name of the variant, kept in sync with `from_name` — what a saved icon
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
        }
    }

    /// The reverse of `name` over `ALL` (so parameterized names come back as their representative).
    pub fn from_name(s: &str) -> Option<Glyph> {
        Self::ALL.iter().copied().find(|g| g.name() == s)
    }
}

const STRIP: [(Tool, Glyph, &str); 16] = [
    (Tool::Select, Glyph::Cursor, "Select"),
    (Tool::Cut, Glyph::Razor, "Cut"),
    (Tool::Marker, Glyph::Flag, "Marker"),
    (Tool::Stretch, Glyph::Hourglass, "Stretch"),
    (Tool::Spacer, Glyph::Spacer, "Spacer"),
    (Tool::Text, Glyph::Letter('T'), "Text"),
    (Tool::Shape(ShapeKind::Rect), Glyph::Rect, "Rectangle"),
    (Tool::Shape(ShapeKind::Ellipse), Glyph::Ellipse, "Ellipse"),
    (Tool::Shape(ShapeKind::Triangle), Glyph::Poly(3), "Triangle"),
    (Tool::Shape(ShapeKind::Polygon), Glyph::Poly(5), "Polygon"),
    (Tool::Shape(ShapeKind::Star), Glyph::Star, "Star"),
    (Tool::Shape(ShapeKind::Line), Glyph::Line, "Line"),
    (Tool::Shape(ShapeKind::Arrow), Glyph::Arrow, "Arrow"),
    (Tool::Draw, Glyph::Pencil, "Draw"),
    (Tool::Mask(MaskShape::Rect), Glyph::Mask, "Mask"),
    (Tool::Zoom, Glyph::Zoom, "Zoom"),
];

/// Single-key shortcut of a tool (used for the tooltips and by `handle_hotkeys`). The shape tools cycle
/// on Shift+S instead (bare `S` toggles snapping), so their tooltip is built by hand in `show`.
pub fn tool_hotkey(tool: Tool) -> Option<Key> {
    match tool {
        Tool::Select => Some(Key::V),
        Tool::Text => Some(Key::T),
        Tool::Draw => Some(Key::D),
        Tool::Mask(_) => Some(Key::M),
        Tool::Cut => Some(Key::C),
        Tool::Stretch => Some(Key::R),
        Tool::Shape(_) | Tool::Zoom | Tool::Marker | Tool::Spacer => None,
    }
}

/// V / T / D / M and Shift+S switch tools (Shift+S also steps through the shape variants; M steps
/// through the mask variants), ignored while a text field has focus or an unlisted modifier is held.
/// Bare `S` is snapping's key (see `handle_snap_hotkey`), not a tool switch. Returns the new tool when it
/// changed; the key is consumed, so calling this twice in a frame is harmless.
pub fn handle_hotkeys(ctx: &egui::Context, state: &mut ToolsState) -> Option<Tool> {
    if ctx.wants_keyboard_input() {
        return None;
    }
    ctx.input_mut(|i| {
        // most-specific shortcut first: consume_key's own ctrl/command matching is exact, so this can
        // never fire for e.g. Ctrl+Shift+S (Save As)
        // ponytail: this also claims hotkeys.rs's default Shift+S (Action::AddShape) before the action
        // table sees it — same trade-off the tool strip already makes for V/T/D/M. AddShape stays
        // reachable from the Insert menu; give it a fresh binding in Settings > Hotkeys if that regresses.
        if i.consume_key(Modifiers::SHIFT, Key::S) {
            let next = Tool::Shape(next_shape(state.tool));
            state.tool = next;
            return Some(next);
        }
        if !i.modifiers.is_none() {
            return None;
        }
        let next = if i.consume_key(Modifiers::NONE, Key::V) {
            Tool::Select
        } else if i.consume_key(Modifiers::NONE, Key::T) {
            Tool::Text
        } else if i.consume_key(Modifiers::NONE, Key::D) {
            Tool::Draw
        } else if i.consume_key(Modifiers::NONE, Key::M) {
            Tool::Mask(next_mask(state.tool))
        } else if i.consume_key(Modifiers::NONE, Key::C) {
            Tool::Cut
        } else if i.consume_key(Modifiers::NONE, Key::R) {
            Tool::Stretch
        } else {
            return None;
        };
        (next != state.tool).then(|| {
            state.tool = next;
            next
        })
    })
}

/// Bare `S` (no modifiers) toggles snapping — claimed here, ahead of the action table, so it can never
/// race with Shift+S's shape cycle above or with the `N` binding in `hotkeys.rs` (`Action::ToggleSnap`,
/// still live and unaffected). `*snap` flips in place; the caller persists it. Returns true when it fired.
pub fn handle_snap_hotkey(ctx: &egui::Context, snap: &mut bool) -> bool {
    if ctx.wants_keyboard_input() {
        return false;
    }
    let pressed = ctx.input_mut(|i| i.modifiers.is_none() && i.consume_key(Modifiers::NONE, Key::S));
    if pressed {
        *snap = !*snap;
    }
    pressed
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

/// Returns true when the tool, `*snap` or the style changed (the app may want to repaint the preview
/// overlay). `snap` is `Settings.snap` — owned by the app, not this strip, so it comes in by reference.
pub fn show(ui: &mut egui::Ui, state: &mut ToolsState, palette: &Palette, snap: &mut bool) -> bool {
    let mut changed = handle_hotkeys(ui.ctx(), state).is_some();
    changed |= handle_snap_hotkey(ui.ctx(), snap);
    let base = ui.id();
    // wrapped: at a small pane width the strip must fold onto a second row, not clip its last buttons
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tool, icon, name) in STRIP {
            let active = same_tool(tool, state.tool);
            let tip = if matches!(tool, Tool::Shape(_)) {
                format!("{name} (Shift+S)")
            } else {
                match tool_hotkey(tool) {
                    Some(k) => format!("{name} ({})", k.name()),
                    // the spacer's gesture is not readable from its picture
                    None if tool == Tool::Spacer => format!("{name} — drag the lanes to open or close a gap"),
                    None => name.to_string(),
                }
            };
            if icon_button(ui, palette, base.with(("tool", name)), icon, &tip, active).clicked() && !active {
                state.tool = tool;
                changed = true;
            }
        }
        ui.separator();
        if icon_button(ui, palette, base.with("snap"), Glyph::Magnet, "Snapping (S)", *snap).clicked() {
            *snap = !*snap;
            changed = true;
        }
        ui.separator();
        changed |= style_controls(ui, state, palette);
    });
    changed
}

/// The Mask button lights up for every mask shape (the combo beside it picks one); everything else is
/// an exact match.
fn same_tool(entry: Tool, cur: Tool) -> bool {
    matches!((entry, cur), (Tool::Mask(_), Tool::Mask(_))) || entry == cur
}

/// Style controls for the active tool. Returns true when anything changed.
fn style_controls(ui: &mut egui::Ui, state: &mut ToolsState, palette: &Palette) -> bool {
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
        Tool::Mask(shape) => {
            ui.label("Mask");
            let mut pick = shape;
            egui::ComboBox::from_id_salt("tool-mask-shape").selected_text(shape.name()).width(90.0).show_ui(ui, |ui| {
                for m in MaskShape::ALL {
                    ui.selectable_value(&mut pick, m, m.name());
                }
            });
            if pick != shape {
                state.tool = Tool::Mask(pick);
                changed = true;
            }
        }
        Tool::Text | Tool::Select | Tool::Zoom | Tool::Cut | Tool::Marker | Tool::Stretch | Tool::Spacer => {}
    }
    changed
}

/// A button showing `icon` and then `text`. Used wherever a control needs a real-world picture rather
/// than a symbol character (half of those render as tofu boxes in Segoe UI). An empty `text` gives a
/// bare icon button, centred.
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

/// Paint one tool glyph inside `rect` (a 24x22 button) in `fg`.
pub(crate) fn draw_glyph(p: &egui::Painter, rect: egui::Rect, g: Glyph, fg: Color32) {
    let c = rect.center();
    let r = 6.0; // half-extent of the drawn icon
    let stroke = Stroke::new(1.4, fg);
    let poly = |n: u32, rot: f32, rad: f32| -> Vec<egui::Pos2> {
        (0..n)
            .map(|i| {
                let a = rot + std::f32::consts::TAU * i as f32 / n as f32;
                c + egui::vec2(a.cos() * rad, a.sin() * rad)
            })
            .collect()
    };
    // isoceles triangle centred on `ctr`, `depth` from base to tip along `d`, `half` across the base
    let tri = |ctr: egui::Pos2, d: Dir, depth: f32, half: f32| -> Vec<egui::Pos2> {
        let u = d.unit();
        let n = egui::vec2(-u.y, u.x);
        vec![ctr + u * depth, ctr - u * depth + n * half, ctr - u * depth - n * half]
    };
    match g {
        Glyph::Letter(ch) => {
            p.text(c, Align2::CENTER_CENTER, ch, FontId::proportional(13.0), fg);
        }
        // a classic arrow cursor
        Glyph::Cursor => {
            let pts = vec![
                c + egui::vec2(-3.5, -6.5),
                c + egui::vec2(-3.5, 5.5),
                c + egui::vec2(-0.5, 2.5),
                c + egui::vec2(1.5, 6.5),
                c + egui::vec2(3.5, 5.5),
                c + egui::vec2(1.5, 1.8),
                c + egui::vec2(5.0, 1.5),
            ];
            p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE));
        }
        Glyph::Rect => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(12.0, 9.0)),
                CornerRadius::ZERO,
                stroke,
                StrokeKind::Inside,
            );
        }
        Glyph::Ellipse => {
            p.circle_stroke(c, r, stroke);
        }
        Glyph::Poly(n) => {
            let pts = poly(n, -std::f32::consts::FRAC_PI_2, r);
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        Glyph::Star => {
            let outer = poly(5, -std::f32::consts::FRAC_PI_2, r);
            let inner = poly(5, -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU / 10.0, r * 0.45);
            let mut pts = Vec::with_capacity(10);
            for i in 0..5 {
                pts.push(outer[i]);
                pts.push(inner[i]);
            }
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        Glyph::Line => {
            p.line_segment([c + egui::vec2(-6.0, 5.0), c + egui::vec2(6.0, -5.0)], stroke);
        }
        Glyph::Arrow => {
            p.line_segment([c + egui::vec2(-6.0, 0.0), c + egui::vec2(4.0, 0.0)], stroke);
            let head = vec![c + egui::vec2(6.5, 0.0), c + egui::vec2(2.5, -3.2), c + egui::vec2(2.5, 3.2)];
            p.add(egui::Shape::convex_polygon(head, fg, Stroke::NONE));
        }
        // pencil: a slanted body with a tip at the bottom-left
        Glyph::Pencil => {
            let body = vec![
                c + egui::vec2(1.0, -6.0),
                c + egui::vec2(5.5, -1.5),
                c + egui::vec2(-2.0, 6.0),
                c + egui::vec2(-6.0, 6.5),
                c + egui::vec2(-5.5, 2.5),
            ];
            p.add(egui::Shape::convex_polygon(body, fg, Stroke::NONE));
        }
        // mask: a circle whose left half is filled (matte in / matte out)
        Glyph::Mask => {
            p.circle_stroke(c, r, stroke);
            let half: Vec<egui::Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                    c + egui::vec2(a.cos() * r, a.sin() * r)
                })
                .collect();
            p.add(egui::Shape::convex_polygon(half, fg, Stroke::NONE));
        }
        Glyph::Zoom => {
            p.circle_stroke(c + egui::vec2(-1.0, -1.0), 4.5, stroke);
            p.line_segment([c + egui::vec2(2.2, 2.2), c + egui::vec2(6.0, 6.0)], Stroke::new(1.8, fg));
        }
        // razor blade: a rectangular blade with a toothed cutting edge along the bottom
        Glyph::Razor => {
            let body = egui::Rect::from_center_size(c + egui::vec2(0.0, -2.0), egui::vec2(12.0, 6.0));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.line_segment([c + egui::vec2(-6.0, 1.5), c + egui::vec2(6.0, 1.5)], Stroke::new(2.0, fg));
            for i in 0..4 {
                let x = -4.5 + i as f32 * 3.0;
                p.line_segment([c + egui::vec2(x, 1.5), c + egui::vec2(x, 5.0)], Stroke::new(1.0, fg));
            }
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
        // pennant on a pole
        Glyph::Flag => {
            p.line_segment([c + egui::vec2(-4.0, -6.5), c + egui::vec2(-4.0, 6.5)], Stroke::new(1.6, fg));
            let cloth = vec![c + egui::vec2(-4.0, -6.0), c + egui::vec2(5.5, -3.0), c + egui::vec2(-4.0, 0.0)];
            p.add(egui::Shape::convex_polygon(cloth, fg, Stroke::NONE));
        }
        // hourglass: two triangles meeting at the waist, between two plates
        Glyph::Hourglass => {
            p.line_segment([c + egui::vec2(-5.0, -6.5), c + egui::vec2(5.0, -6.5)], stroke);
            p.line_segment([c + egui::vec2(-5.0, 6.5), c + egui::vec2(5.0, 6.5)], stroke);
            p.add(egui::Shape::convex_polygon(
                vec![c + egui::vec2(-4.5, -6.0), c + egui::vec2(4.5, -6.0), c],
                fg,
                Stroke::NONE,
            ));
            p.add(egui::Shape::convex_polygon(
                vec![c + egui::vec2(-4.5, 6.0), c + egui::vec2(4.5, 6.0), c],
                fg,
                Stroke::NONE,
            ));
        }
        // eye: two lids and a pupil (crossed out when hidden)
        Glyph::Eye | Glyph::EyeOff => {
            let lid = |up: f32| -> Vec<egui::Pos2> {
                (0..=16)
                    .map(|i| {
                        let x = -7.0 + i as f32 * 14.0 / 16.0;
                        let k: f32 = 1.0 - (x / 7.0) * (x / 7.0);
                        c + egui::vec2(x, up * 4.2 * k.max(0.0))
                    })
                    .collect()
            };
            p.add(egui::Shape::line(lid(-1.0), stroke));
            p.add(egui::Shape::line(lid(1.0), stroke));
            p.circle_filled(c, 2.2, fg);
            if g == Glyph::EyeOff {
                p.line_segment([c + egui::vec2(-6.5, -6.0), c + egui::vec2(6.5, 6.0)], Stroke::new(1.8, fg));
            }
        }
        Glyph::Diamond => {
            let pts = vec![
                c + egui::vec2(0.0, -5.5),
                c + egui::vec2(4.5, 0.0),
                c + egui::vec2(0.0, 5.5),
                c + egui::vec2(-4.5, 0.0),
            ];
            p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE));
        }
        Glyph::Record => {
            p.circle_filled(c, 5.5, Color32::from_rgb(220, 60, 60));
        }
        // microphone: a capsule on a stand
        Glyph::Mic => {
            p.rect_filled(
                egui::Rect::from_center_size(c + egui::vec2(0.0, -2.5), egui::vec2(5.0, 8.0)),
                CornerRadius::same(2),
                fg,
            );
            p.line_segment([c + egui::vec2(0.0, 3.0), c + egui::vec2(0.0, 6.0)], Stroke::new(1.6, fg));
            p.line_segment([c + egui::vec2(-3.5, 6.0), c + egui::vec2(3.5, 6.0)], Stroke::new(1.6, fg));
        }
        // headphones: a headband over two ear cups
        Glyph::Headphone => {
            let band: Vec<egui::Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 16.0;
                    c + egui::vec2(a.cos() * 6.0, a.sin() * 6.0 + 1.0)
                })
                .collect();
            p.add(egui::Shape::line(band, stroke));
            for x in [-6.0, 6.0] {
                p.rect_filled(
                    egui::Rect::from_center_size(c + egui::vec2(x, 3.0), egui::vec2(3.5, 6.0)),
                    CornerRadius::same(1),
                    fg,
                );
            }
        }
        // speaker cone, with sound waves or crossed out
        Glyph::SpeakerOn | Glyph::SpeakerOff => {
            let cone = vec![
                c + egui::vec2(-6.0, -2.0),
                c + egui::vec2(-3.0, -2.0),
                c + egui::vec2(0.5, -5.5),
                c + egui::vec2(0.5, 5.5),
                c + egui::vec2(-3.0, 2.0),
                c + egui::vec2(-6.0, 2.0),
            ];
            p.add(egui::Shape::convex_polygon(cone, fg, Stroke::NONE));
            if g == Glyph::SpeakerOn {
                for (i, rad) in [3.0_f32, 5.5].iter().enumerate() {
                    let arc: Vec<egui::Pos2> = (0..=10)
                        .map(|k| {
                            let a = -std::f32::consts::FRAC_PI_3 + 2.0 * std::f32::consts::FRAC_PI_3 * k as f32 / 10.0;
                            c + egui::vec2(1.5 + a.cos() * rad, a.sin() * rad)
                        })
                        .collect();
                    p.add(egui::Shape::line(arc, Stroke::new(1.3 - i as f32 * 0.2, fg)));
                }
            } else {
                p.line_segment([c + egui::vec2(2.5, -3.5), c + egui::vec2(6.5, 3.5)], Stroke::new(1.6, fg));
                p.line_segment([c + egui::vec2(6.5, -3.5), c + egui::vec2(2.5, 3.5)], Stroke::new(1.6, fg));
            }
        }
        // stills camera: body, lens and a viewfinder bump
        Glyph::Camera => {
            let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 1.0), egui::vec2(13.0, 9.0));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.rect_filled(
                egui::Rect::from_center_size(c + egui::vec2(-2.0, -4.5), egui::vec2(5.0, 2.5)),
                CornerRadius::same(1),
                fg,
            );
            p.circle_stroke(c + egui::vec2(0.0, 1.0), 3.0, stroke);
        }
        // film strip: a frame with sprocket holes down both edges
        Glyph::FilmStrip => {
            let body = egui::Rect::from_center_size(c, egui::vec2(13.0, 11.0));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            for i in 0..3 {
                let y = -3.5 + i as f32 * 3.5;
                for x in [-4.8_f32, 4.8] {
                    p.rect_filled(
                        egui::Rect::from_center_size(c + egui::vec2(x, y), egui::vec2(2.2, 2.0)),
                        CornerRadius::ZERO,
                        fg,
                    );
                }
            }
        }
        // node graph: two patch boxes with a cable between them
        Glyph::Nodes => {
            let a = egui::Rect::from_center_size(c + egui::vec2(-3.5, -3.5), egui::vec2(7.0, 5.0));
            let b = egui::Rect::from_center_size(c + egui::vec2(3.5, 3.5), egui::vec2(7.0, 5.0));
            p.line_segment([a.right_center(), b.left_center()], stroke);
            p.rect_stroke(a, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.rect_stroke(b, CornerRadius::same(1), stroke, StrokeKind::Inside);
        }
        // adjustment layer: a stack of sheets
        Glyph::Layers => {
            for dy in [-4.0_f32, 0.0, 4.0] {
                let sheet = egui::Rect::from_center_size(c + egui::vec2(0.0, dy), egui::vec2(12.0, 3.0));
                p.rect_stroke(sheet, CornerRadius::same(1), stroke, StrokeKind::Inside);
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
        // tracking: a shooting target — two rings and four cross ticks
        Glyph::Target => {
            p.circle_stroke(c, r, stroke);
            p.circle_stroke(c, r * 0.4, stroke);
            for (dx, dy) in [(1.0_f32, 0.0_f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                p.line_segment(
                    [c + egui::vec2(dx * r, dy * r), c + egui::vec2(dx * (r + 2.5), dy * (r + 2.5))],
                    stroke,
                );
            }
        }
        // eighth note: filled notehead, a stem and a flag
        Glyph::MusicNote => {
            let head = c + egui::vec2(-2.5, 3.5);
            p.circle_filled(head, 2.6, fg);
            let stem_top = c + egui::vec2(2.0, -6.0);
            p.line_segment([head + egui::vec2(2.4, -0.5), stem_top], Stroke::new(1.6, fg));
            let flag = vec![stem_top, stem_top + egui::vec2(4.0, 1.5), stem_top + egui::vec2(0.5, 4.5)];
            p.add(egui::Shape::convex_polygon(flag, fg, Stroke::NONE));
        }
        Glyph::Folder => {
            let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 1.5), egui::vec2(12.0, 8.0));
            let tab = egui::Rect::from_min_size(body.left_top() - egui::vec2(0.0, 2.5), egui::vec2(5.0, 2.5));
            p.rect_stroke(tab, CornerRadius::ZERO, stroke, StrokeKind::Inside);
            p.rect_stroke(body, CornerRadius::ZERO, stroke, StrokeKind::Inside);
        }
        Glyph::Cross => {
            let s = Stroke::new(1.7, fg);
            p.line_segment([c + egui::vec2(-4.2, -4.2), c + egui::vec2(4.2, 4.2)], s);
            p.line_segment([c + egui::vec2(4.2, -4.2), c + egui::vec2(-4.2, 4.2)], s);
        }
        Glyph::Dot => {
            p.circle_filled(c, 3.6, fg);
        }
        // copy: the sheet you are taking, with its original still behind it
        Glyph::Copy => {
            p.rect_stroke(
                egui::Rect::from_min_size(c + egui::vec2(-6.0, -6.5), egui::vec2(8.0, 10.0)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            p.rect_filled(
                egui::Rect::from_min_size(c + egui::vec2(-2.0, -3.0), egui::vec2(8.0, 10.0)),
                CornerRadius::same(1),
                fg,
            );
        }
        // paste: a clipboard — a board with its spring clip on top
        Glyph::Paste => {
            p.rect_stroke(
                egui::Rect::from_center_size(c + egui::vec2(0.0, 1.0), egui::vec2(11.0, 12.0)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            p.rect_filled(
                egui::Rect::from_center_size(c + egui::vec2(0.0, -5.0), egui::vec2(6.5, 3.5)),
                CornerRadius::same(1),
                fg,
            );
        }
        Glyph::Tri(d) => {
            p.add(egui::Shape::convex_polygon(tri(c, d, 4.0, 4.2), fg, Stroke::NONE));
        }
        Glyph::Skip(d) => {
            let u = d.unit();
            for k in [-3.4, 3.4] {
                p.add(egui::Shape::convex_polygon(tri(c + u * k, d, 3.4, 4.6), fg, Stroke::NONE));
            }
        }
        // |◀ / ▶| — the bar sits at the far edge, in the direction of travel
        Glyph::Jump(d) => {
            let u = d.unit();
            let n = egui::vec2(-u.y, u.x);
            p.line_segment([c + u * 5.5 - n * 5.0, c + u * 5.5 + n * 5.0], Stroke::new(2.0, fg));
            p.add(egui::Shape::convex_polygon(tri(c - u * 1.5, d, 3.8, 5.0), fg, Stroke::NONE));
        }
        Glyph::Play => {
            p.add(egui::Shape::convex_polygon(tri(c + egui::vec2(0.8, 0.0), Dir::Right, 5.5, 6.0), fg, Stroke::NONE));
        }
        Glyph::Pause => {
            for x in [-3.0_f32, 3.0] {
                p.rect_filled(
                    egui::Rect::from_center_size(c + egui::vec2(x, 0.0), egui::vec2(3.0, 11.0)),
                    CornerRadius::ZERO,
                    fg,
                );
            }
        }
        Glyph::Stop => {
            p.rect_filled(egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)), CornerRadius::same(1), fg);
        }
        // fullscreen: four corner brackets pointing outwards
        Glyph::Fullscreen => {
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = c + egui::vec2(sx * 6.5, sy * 5.5);
                p.line_segment([corner, corner - egui::vec2(sx * 4.0, 0.0)], stroke);
                p.line_segment([corner, corner - egui::vec2(0.0, sy * 3.5)], stroke);
            }
        }
        // pop out: a window frame with an arrow leaving through its top-right corner
        Glyph::PopOut => {
            p.rect_stroke(
                egui::Rect::from_center_size(c + egui::vec2(-1.0, 1.5), egui::vec2(11.0, 10.0)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            p.line_segment([c + egui::vec2(0.0, -1.0), c + egui::vec2(6.0, -6.0)], stroke);
            p.add(egui::Shape::convex_polygon(
                vec![c + egui::vec2(6.5, -6.5), c + egui::vec2(1.5, -6.0), c + egui::vec2(6.0, -1.5)],
                fg,
                Stroke::NONE,
            ));
        }
        // indent / outdent: an arrow shoved against the margin it moves towards
        Glyph::Indent(inward) => {
            let d = if inward { Dir::Right } else { Dir::Left };
            let u = d.unit();
            p.line_segment([c + u * 6.0 + egui::vec2(0.0, -5.5), c + u * 6.0 + egui::vec2(0.0, 5.5)], stroke);
            p.line_segment([c - u * 6.0, c + u * 1.0], stroke);
            p.add(egui::Shape::convex_polygon(tri(c + u * 2.5, d, 2.5, 3.2), fg, Stroke::NONE));
        }
        // nested sequence: two clip blocks standing on a timeline lane
        Glyph::Sequence => {
            p.line_segment([c + egui::vec2(-6.5, 5.5), c + egui::vec2(6.5, 5.5)], stroke);
            for (x, h) in [(-6.0_f32, 9.0_f32), (0.5, 6.0)] {
                p.rect_filled(
                    egui::Rect::from_min_size(c + egui::vec2(x, 4.0 - h), egui::vec2(5.5, h)),
                    CornerRadius::same(1),
                    fg,
                );
            }
        }
        // container / slot: a frame box with an inner block
        Glyph::Container => {
            let outer = egui::Rect::from_center_size(c, egui::vec2(12.0, 10.0));
            let inner = egui::Rect::from_center_size(c, egui::vec2(6.0, 4.5));
            p.rect_stroke(outer, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.rect_filled(inner, CornerRadius::ZERO, fg);
        }
        // template: a card with its top-right corner folded over
        Glyph::Template => {
            let fold = 4.0;
            let (l, t, rr, b) = (c.x - 5.0, c.y - 6.0, c.x + 5.0, c.y + 6.0);
            p.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(l, t),
                    egui::pos2(rr - fold, t),
                    egui::pos2(rr, t + fold),
                    egui::pos2(rr, b),
                    egui::pos2(l, b),
                ],
                stroke,
            ));
            p.add(egui::Shape::line(
                vec![egui::pos2(rr - fold, t), egui::pos2(rr - fold, t + fold), egui::pos2(rr, t + fold)],
                stroke,
            ));
        }
        // snowflake: three crossed arms, each with a pair of barbs
        Glyph::Snowflake => {
            let thin = Stroke::new(1.0, fg);
            for i in 0..3 {
                let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 3.0;
                let arm = egui::vec2(a.cos(), a.sin());
                p.line_segment([c - arm * 6.5, c + arm * 6.5], thin);
                let barb = egui::vec2(-arm.y, arm.x);
                for s in [-1.0_f32, 1.0] {
                    let tip = c + arm * (6.5 * s);
                    p.line_segment([tip, tip - arm * 2.4 * s + barb * 1.8], thin);
                    p.line_segment([tip, tip - arm * 2.4 * s - barb * 1.8], thin);
                }
            }
        }
        // movie reel: rim, hub, four spoke holes and a tape tail leaving bottom-right
        Glyph::FilmReel => {
            p.circle_stroke(c, r, stroke);
            p.circle_filled(c, 1.2, fg);
            for i in 0..4 {
                let a = std::f32::consts::FRAC_PI_2 * i as f32;
                p.circle_stroke(c + egui::vec2(a.cos(), a.sin()) * (r * 0.55), 1.3, Stroke::new(1.0, fg));
            }
            p.line_segment([c + egui::vec2(4.2, 4.2), c + egui::vec2(8.0, 6.0)], stroke);
        }
        // lightning bolt: a filled zigzag
        Glyph::Bolt => {
            let pts = vec![
                c + egui::vec2(1.5, -6.5),
                c + egui::vec2(-3.5, 1.0),
                c + egui::vec2(-0.5, 1.0),
                c + egui::vec2(-1.5, 6.5),
                c + egui::vec2(3.5, -1.0),
                c + egui::vec2(0.5, -1.0),
            ];
            p.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.0, fg)));
            p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE)); // concave, but close enough at 12px
        }
        // import / export: an open tray with an arrow dropping in or rising out
        Glyph::ImportArrow | Glyph::ExportArrow => {
            p.add(egui::Shape::line(
                vec![
                    c + egui::vec2(-6.0, 2.0),
                    c + egui::vec2(-6.0, 6.0),
                    c + egui::vec2(6.0, 6.0),
                    c + egui::vec2(6.0, 2.0),
                ],
                stroke,
            ));
            let d = if g == Glyph::ImportArrow { Dir::Down } else { Dir::Up };
            let tip_y = if g == Glyph::ImportArrow { 3.0 } else { -6.5 };
            let tail_y = if g == Glyph::ImportArrow { -6.5 } else { 3.0 };
            p.line_segment([c + egui::vec2(0.0, tail_y), c + egui::vec2(0.0, tip_y)], stroke);
            p.add(egui::Shape::convex_polygon(tri(c + egui::vec2(0.0, tip_y), d, 2.8, 3.2), fg, Stroke::NONE));
        }
        // transition: two overlapping frames with a diagonal cut across the overlap
        Glyph::Transition => {
            let a = egui::Rect::from_center_size(c + egui::vec2(-2.5, -2.0), egui::vec2(9.0, 8.0));
            let b = egui::Rect::from_center_size(c + egui::vec2(2.5, 2.0), egui::vec2(9.0, 8.0));
            p.rect_stroke(a, CornerRadius::ZERO, stroke, StrokeKind::Inside);
            p.rect_stroke(b, CornerRadius::ZERO, stroke, StrokeKind::Inside);
            p.line_segment([a.left_bottom() + egui::vec2(2.0, 0.0), a.right_top() + egui::vec2(0.0, 2.0)], stroke);
        }
        // subtitles: a caption box with two text bars in its lower half
        Glyph::Subtitles => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(13.0, 10.0)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            p.line_segment([c + egui::vec2(-4.5, 1.0), c + egui::vec2(2.5, 1.0)], Stroke::new(1.6, fg));
            p.line_segment([c + egui::vec2(-4.5, 3.2), c + egui::vec2(4.5, 3.2)], Stroke::new(1.6, fg));
        }
        // gear: a ring with eight tooth stubs and a hub
        Glyph::Gear => {
            p.circle_stroke(c, 4.2, stroke);
            p.circle_filled(c, 1.4, fg);
            for i in 0..8 {
                let a = std::f32::consts::FRAC_PI_4 * i as f32;
                let u = egui::vec2(a.cos(), a.sin());
                p.line_segment([c + u * 4.2, c + u * 6.5], stroke);
            }
        }
        // sliders: three tracks, each with its knob somewhere else
        Glyph::Sliders => {
            for (i, kx) in [(-1.0_f32, -2.5_f32), (0.0, 3.0), (1.0, -0.5)] {
                let y = i * 4.5;
                p.line_segment([c + egui::vec2(-6.5, y), c + egui::vec2(6.5, y)], stroke);
                p.circle_filled(c + egui::vec2(kx, y), 1.8, fg);
            }
        }
        // wrench: an open jaw (circle with a notch) and a diagonal handle
        Glyph::Wrench => {
            let jaw = c + egui::vec2(-3.5, -3.5);
            p.circle_stroke(jaw, 3.0, stroke);
            // notch: paint over the rim towards the top-left, opening the jaw
            p.line_segment([jaw, jaw + egui::vec2(-3.5, -3.5)], Stroke::new(2.6, fg));
            p.line_segment([jaw + egui::vec2(2.0, 2.0), c + egui::vec2(6.0, 6.0)], Stroke::new(2.2, fg));
        }
        // clapperboard: the body and a slanted top bar with two hatch strokes
        Glyph::Clapperboard => {
            let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 2.0), egui::vec2(13.0, 7.5));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.add(egui::Shape::closed_line(
                vec![
                    body.left_top(),
                    body.left_top() + egui::vec2(1.0, -3.8),
                    body.right_top() + egui::vec2(0.0, -2.8),
                    body.right_top(),
                ],
                stroke,
            ));
            for x in [-2.5_f32, 2.0] {
                p.line_segment([c + egui::vec2(x, -1.7), c + egui::vec2(x + 2.0, -4.8)], Stroke::new(1.0, fg));
            }
        }
        // waveform: five bars mirrored about the midline
        Glyph::Waveform => {
            for (i, h) in [3.0_f32, 6.0, 4.0, 6.5, 2.5].iter().enumerate() {
                let x = -6.0 + i as f32 * 3.0;
                p.line_segment([c + egui::vec2(x, -h / 2.0), c + egui::vec2(x, h / 2.0)], Stroke::new(1.8, fg));
            }
        }
        // value curve: L axes, an easing curve and two square handles
        Glyph::CurveIcon => {
            let o = c + egui::vec2(-6.0, 6.0);
            p.line_segment([o, o + egui::vec2(0.0, -12.0)], stroke);
            p.line_segment([o, o + egui::vec2(12.0, 0.0)], stroke);
            p.add(egui::Shape::line(
                vec![
                    o + egui::vec2(1.0, -1.0),
                    o + egui::vec2(5.0, -2.5),
                    o + egui::vec2(8.0, -6.5),
                    o + egui::vec2(11.0, -11.0),
                ],
                stroke,
            ));
            for d in [egui::vec2(5.0, -2.5), egui::vec2(8.0, -6.5)] {
                p.rect_filled(egui::Rect::from_center_size(o + d, egui::vec2(2.4, 2.4)), CornerRadius::ZERO, fg);
            }
        }
        Glyph::Clock => {
            p.circle_stroke(c, r, stroke);
            p.line_segment([c, c + egui::vec2(0.0, -4.0)], stroke);
            p.line_segment([c, c + egui::vec2(3.0, 1.5)], stroke);
        }
        Glyph::Notepad => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(10.0, 12.0)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            for dy in [-3.0_f32, 0.0, 3.0] {
                p.line_segment([c + egui::vec2(-3.0, dy), c + egui::vec2(3.0, dy)], Stroke::new(1.0, fg));
            }
        }
        // bookmark: a ribbon whose bottom edge is notched into a V
        Glyph::Bookmark => {
            p.add(egui::Shape::closed_line(
                vec![
                    c + egui::vec2(-3.5, -6.5),
                    c + egui::vec2(3.5, -6.5),
                    c + egui::vec2(3.5, 6.5),
                    c + egui::vec2(0.0, 3.0),
                    c + egui::vec2(-3.5, 6.5),
                ],
                stroke,
            ));
        }
        // undo / redo: an arc curving over the top, arrowhead at the `Dir::Left` / `Dir::Right` end
        Glyph::UndoArrow(d) => {
            let sx = if d == Dir::Left { 1.0 } else { -1.0 };
            let arc: Vec<egui::Pos2> = (0..=8)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * 0.85 * i as f32 / 8.0;
                    c + egui::vec2(a.cos() * 5.0 * sx, a.sin() * 5.0 + 1.5)
                })
                .collect();
            let tip = arc[0];
            p.add(egui::Shape::line(arc, stroke));
            p.add(egui::Shape::convex_polygon(
                vec![
                    tip + egui::vec2(-2.5 * sx, 0.5),
                    tip + egui::vec2(2.0 * sx, -1.5),
                    tip + egui::vec2(1.5 * sx, 3.0),
                ],
                fg,
                Stroke::NONE,
            ));
        }
        // floppy disk: a square with its top-right corner cut, a shutter and a label
        Glyph::FloppyDisk => {
            let (l, t, rr, b) = (c.x - 6.0, c.y - 6.0, c.x + 6.0, c.y + 6.0);
            p.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(l, t),
                    egui::pos2(rr - 2.5, t),
                    egui::pos2(rr, t + 2.5),
                    egui::pos2(rr, b),
                    egui::pos2(l, b),
                ],
                stroke,
            ));
            p.rect_filled(
                egui::Rect::from_min_size(egui::pos2(l + 2.5, t + 0.7), egui::vec2(5.5, 3.0)),
                CornerRadius::ZERO,
                fg,
            );
            p.rect_stroke(
                egui::Rect::from_min_size(egui::pos2(l + 2.0, b - 4.5), egui::vec2(8.0, 3.8)),
                CornerRadius::ZERO,
                Stroke::new(1.0, fg),
                StrokeKind::Inside,
            );
        }
        // terminal: a window with a '>' prompt and an underscore cursor
        Glyph::Terminal => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(13.0, 10.0)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Inside,
            );
            p.line_segment([c + egui::vec2(-4.5, -2.5), c + egui::vec2(-2.0, 0.0)], stroke);
            p.line_segment([c + egui::vec2(-2.0, 0.0), c + egui::vec2(-4.5, 2.5)], stroke);
            p.line_segment([c + egui::vec2(0.0, 2.5), c + egui::vec2(3.5, 2.5)], stroke);
        }
    }
}

/// Built-in icon for a menu action (None = text-only). The user's Settings → Appearance → Icons
/// override wins over these; abstract actions stay text-only on purpose.
pub(crate) fn action_glyph(a: crate::hotkeys::Action) -> Option<Glyph> {
    use crate::hotkeys::Action::*;
    Some(match a {
        NewProject => Glyph::Clapperboard,
        OpenFile | OpenProject => Glyph::Folder,
        Save | SaveProjectAs => Glyph::FloppyDisk,
        ExportVideo | ExportLossless | ExportXml => Glyph::ExportArrow,
        ImportMedia => Glyph::FilmReel,
        Settings => Glyph::Gear,
        Undo => Glyph::UndoArrow(Dir::Left),
        Redo => Glyph::UndoArrow(Dir::Right),
        PlayPause => Glyph::Play,
        Stop => Glyph::Stop,
        Split => Glyph::Razor,
        AddText => Glyph::Letter('T'),
        AddMarker => Glyph::Flag,
        Retime => Glyph::Clock,
        Fullscreen => Glyph::Fullscreen,
        ScreenCapture => Glyph::Camera,
        _ => return None,
    })
}

/// Paint `icon` where a plain label would go — no button chrome, no hit area.
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
fn on_accent(c: Color32) -> Color32 {
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
        snap: bool,
        base: egui::Id,
        time: f64,
        changed: bool,
    }

    impl Harness {
        fn new() -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                state: ToolsState::default(),
                snap: false,
                base: egui::Id::NULL,
                time: 0.0,
                changed: false,
            };
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 60.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
            let Harness { ctx, state, snap, base, changed, .. } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    *base = ui.id();
                    *changed = show(ui, state, &pal, snap);
                });
            });
        }
        /// Centre of a strip button by its name, from the previous frame's layout.
        fn button(&self, name: &str) -> Pos2 {
            self.ctx
                .read_response(self.base.with(("tool", name)))
                .unwrap_or_else(|| panic!("no button {name}"))
                .rect
                .center()
        }
        fn click(&mut self, name: &str) {
            let pos = self.button(name);
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
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
    /// like — and without giving it a name for the icon picker.
    const ALL_GLYPHS: &[Glyph] = Glyph::ALL;

    /// Tessellated vertices produced by `paint`, on a throwaway context.
    fn painted(paint: impl Fn(&egui::Painter, Rect)) -> usize {
        let ctx = egui::Context::default();
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

    #[test]
    fn every_glyph_paints_a_picture() {
        // the point of a Glyph is that nothing is typed — a variant that paints nothing would be a
        // blank button, no better than the tofu box it replaced
        let empty = painted(|_, _| {});
        for g in ALL_GLYPHS {
            let n = painted(|p, rect| draw_glyph(p, rect, *g, Color32::WHITE));
            assert!(n > empty, "{g:?} painted nothing ({n} vs {empty} vertices)");
        }
    }

    #[test]
    fn strip_lays_out_every_tool() {
        let h = Harness::new();
        for (_, _, name) in STRIP {
            let c = h.button(name);
            assert!(c.x > 0.0 && c.x < 900.0, "{name} off-strip at {c:?}");
        }
    }

    #[test]
    fn clicking_switches_the_tool() {
        let mut h = Harness::new();
        assert_eq!(h.state.tool, Tool::Select);
        h.click("Ellipse");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Ellipse));
        assert!(h.changed, "a switch is reported as a change");
        h.click("Draw");
        assert_eq!(h.state.tool, Tool::Draw);
        h.click("Zoom");
        assert_eq!(h.state.tool, Tool::Zoom);
        h.click("Select");
        assert_eq!(h.state.tool, Tool::Select);
    }

    #[test]
    fn clicking_the_active_tool_reports_nothing() {
        let mut h = Harness::new();
        h.click("Star");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Star));
        h.click("Star");
        assert!(!h.changed, "re-clicking the active tool is not a change");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Star));
    }

    #[test]
    fn mask_button_lights_up_for_every_mask_shape() {
        assert!(same_tool(Tool::Mask(MaskShape::Rect), Tool::Mask(MaskShape::Path)));
        assert!(!same_tool(Tool::Mask(MaskShape::Rect), Tool::Select));
        assert!(!same_tool(Tool::Shape(ShapeKind::Rect), Tool::Shape(ShapeKind::Star)));
        let mut h = Harness::new();
        h.state.tool = Tool::Mask(MaskShape::Path);
        h.frame(vec![]);
        h.click("Mask");
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Path), "the active mask shape survives a re-click");
        h.click("Rectangle");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Rect));
        h.click("Mask");
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Rect));
    }

    #[test]
    fn single_key_shortcuts_switch_tools() {
        let mut h = Harness::new();
        h.key(Key::T);
        assert_eq!(h.state.tool, Tool::Text);
        h.key(Key::D);
        assert_eq!(h.state.tool, Tool::Draw);
        h.key(Key::V);
        assert_eq!(h.state.tool, Tool::Select);
        h.key_mod(Key::S, Modifiers::SHIFT);
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Rect));
        h.key_mod(Key::S, Modifiers::SHIFT);
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Ellipse), "Shift+S steps through the shapes");
        h.key(Key::M);
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Rect));
        h.key(Key::M);
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Ellipse));
    }

    #[test]
    fn bare_s_toggles_snapping_not_the_tool() {
        let mut h = Harness::new();
        h.state.tool = Tool::Draw;
        assert!(!h.snap);
        h.key(Key::S);
        assert!(h.snap, "bare S toggles snapping");
        assert_eq!(h.state.tool, Tool::Draw, "bare S must not touch the active tool");
        assert!(h.changed);
        h.key(Key::S);
        assert!(!h.snap, "S toggles back off");
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
        assert!(!h.snap, "Ctrl+S must not toggle snapping either");
    }

    #[test]
    fn hotkeys_are_consumed_once() {
        // `show` handles the keys itself, so a second call in the same frame must be a no-op
        let mut state = ToolsState::default();
        let mut snap = false;
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 60.0))),
            events: vec![Event::Key {
                key: Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            }],
            ..Default::default()
        };
        let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &pal, &mut snap);
                assert!(handle_hotkeys(ui.ctx(), &mut state).is_none(), "key already consumed");
            });
        });
        assert_eq!(state.tool, Tool::Shape(ShapeKind::Rect));
        assert!(!snap, "Shift+S is the shape cycle, not the snap toggle");
    }

    #[test]
    fn tool_hotkey_covers_the_documented_keys() {
        assert_eq!(tool_hotkey(Tool::Select), Some(Key::V));
        assert_eq!(tool_hotkey(Tool::Text), Some(Key::T));
        assert_eq!(tool_hotkey(Tool::Shape(ShapeKind::Star)), None, "shape tools cycle on Shift+S");
        assert_eq!(tool_hotkey(Tool::Draw), Some(Key::D));
        assert_eq!(tool_hotkey(Tool::Mask(MaskShape::Path)), Some(Key::M));
        assert_eq!(tool_hotkey(Tool::Zoom), None);
    }

    #[test]
    fn clicking_the_magnet_toggles_snapping() {
        let mut h = Harness::new();
        assert!(!h.snap);
        let pos = h.ctx.read_response(h.base.with("snap")).unwrap().rect.center();
        h.frame(vec![Event::PointerMoved(pos)]);
        h.frame(vec![Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(h.snap, "clicking the magnet turns snapping on");
        assert!(h.changed);
    }

    #[test]
    fn every_tool_renders_its_style_controls() {
        let mut h = Harness::new();
        let mut tools: Vec<Tool> = STRIP.iter().map(|(t, ..)| *t).collect();
        tools.extend(MaskShape::ALL.map(Tool::Mask));
        tools.push(Tool::Shape(ShapeKind::Draw));
        for t in tools {
            h.state.tool = t;
            h.frame(vec![]);
            assert_eq!(h.state.tool, t, "{t:?} controls must not change the tool on their own");
            assert!(!h.changed, "{t:?} reports no change when nothing is touched");
        }
    }
}
