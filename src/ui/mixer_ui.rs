//! Mixer pane: one channel strip per bus (Main last), plus the strip of the selected audio clip/track.
//!
//! Strip: name (editable), a stereo peak meter fed by `BusGraph::meter`, a gain fader (dB), pan knob,
//! Mute / Solo / Mono buttons, the output-bus combo, and the filter chain — each filter as a header row
//! (enable, name, up/down, X) with its parameters below (DragValue per `FilterKind::params()`); the EQ
//! filter also draws a small response curve you can drag. "+ Filter" adds from `FilterKind::ALL`,
//! "+ Bus" creates a bus, X deletes one (never Main; its users fall back to Main).
//! Track/clip routing: the selected audio track shows a "Bus" combo (`Track.bus`), the selected clip an
//! override combo (`Clip.bus`, "(track)" = inherit).
//! Undo once per gesture; returns true when the project changed.

use crate::engine::mixer_fx::BusGraph;
use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct MixerState {
    pub selected_bus: Option<Id>,
    /// Show the clip/track routing section.
    pub show_routing: bool,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut MixerState,
    _project: &mut Project,
    _selection: &[Id],
    _buses: &BusGraph,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> bool {
    todo!("ui::mixer_ui::show")
}
