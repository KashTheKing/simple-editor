//! Tool bar: a thin, movable strip that sits between the viewport and the timeline (its own dockable
//! pane, so it can also be popped out). One row of icon buttons:
//!   Select (V) · Text (T) · Rectangle · Ellipse · Triangle · Polygon · Star · Line · Arrow · Draw (D) ·
//!   Mask (rect/ellipse/polygon/path) · Zoom
//! plus, for shape tools, fill and stroke colour buttons and a stroke-width DragValue; for Draw, the
//! brush colour/width, the record rate (0.5x / 1x / 2x) and a page-colour toggle.
//!
//! The active tool changes what a click-drag in the Preview does (see `ui::preview`): Select edits the
//! selected clip, a shape tool drags out a new Shape clip at the playhead, Draw records strokes while the
//! mouse is down (timed, so the sketch replays), Mask edits the selected effect's / clip's mask.

use crate::model::{MaskShape, ShapeKind};
use crate::theme::Palette;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Text,
    Shape(ShapeKind),
    Draw,
    Mask(MaskShape),
    Zoom,
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
        }
    }
}

/// Returns true when the tool or its style changed (the app may want to repaint the preview overlay).
pub fn show(_ui: &mut egui::Ui, _state: &mut ToolsState, _palette: &Palette) -> bool {
    todo!("ui::tools::show")
}
