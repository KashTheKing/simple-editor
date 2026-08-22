//! Node editor: the selected clip's effect chain as a graph you can wire up.
//!
//! Canvas with pan (drag empty space / middle-drag) and zoom (Ctrl+Scroll). Each node is a rounded box:
//! title (`NodeKind::title`), an enable checkbox, input ports on the left, one output port on the right,
//! and its parameters inline (collapsed to the first two when the node is small). Drag from a port to
//! another to connect (`NodeGraph::connect` refuses cycles); drag a wire off a port to disconnect;
//! right-click a node for Delete / Duplicate / Add mask; right-click the canvas for the "Add node" menu
//! (every `EffectKind` grouped by `category()`, Blend, Matte, Mask, Color, Clip, Input). Selecting a node
//! shows its full parameters in the Inspector and lets the Curves pane keyframe them.
//! `Project::ensure_graph` converts a clip's linear stack the first time this pane opens for it.
//! Every mutation calls `undo` once per gesture.

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct NodesState {
    /// Pan/zoom of the canvas.
    pub offset: egui::Vec2,
    pub zoom: f32,
    pub selected: Option<Id>,
    /// A connection being dragged: (node, port, from_output).
    pub linking: Option<(Id, usize, bool)>,
}

#[derive(Default)]
pub struct NodesResponse {
    pub edited: bool,
    /// The node whose parameters the inspector should show.
    pub selected: Option<Id>,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut NodesState,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: f64,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> NodesResponse {
    todo!("ui::nodes::show")
}
