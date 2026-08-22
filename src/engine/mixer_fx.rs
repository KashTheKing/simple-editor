//! Audio filters and bus routing: the DSP behind the mixer panel.
//!
//! `BusGraph` resolves `Project.buses` (Main first) into an evaluation order and owns one `FilterState`
//! per (bus, filter) so IIR/delay state survives across blocks. Clips sum into their bus
//! (`Project::bus_of`), each bus runs its filter chain, then gain/pan/mono, then sums into its output bus.
//! Interleaved stereo f32 @ 48 kHz, processed in place, block-continuous.
//!
//! Filters (`model::FilterKind`): Eq (3-band RBJ: low shelf, peaking, high shelf), HighPass/LowPass (RBJ),
//! Reverb (Freeverb comb+allpass), Echo (delay line + feedback, optional ping-pong), Distortion (soft clip
//! + tone), Compressor (peak detector, attack/release, makeup), NoiseGate, Noise (white/pink/tone), Gain.

use crate::model::{AudioFilter, Bus, Id, Project};

/// Per-filter DSP state (biquad histories, delay lines, envelopes).
pub struct FilterState;

impl FilterState {
    pub fn new(_f: &AudioFilter, _sample_rate: u32) -> Self {
        todo!("engine::mixer_fx::FilterState::new")
    }
    /// Process one block in place. `t` = timeline seconds at the block start (keyframed params).
    pub fn process(&mut self, _f: &AudioFilter, _t: f64, _buf: &mut [f32]) {
        todo!("engine::mixer_fx::FilterState::process")
    }
}

/// Bus routing + per-bus filter state, rebuilt when the bus set changes.
#[derive(Default)]
pub struct BusGraph;

impl BusGraph {
    pub fn new() -> Self {
        Self
    }
    /// Rebuild for `project` (cheap when nothing changed). Cycles fall back to routing into Main.
    pub fn sync(&mut self, _project: &Project) {
        todo!("engine::mixer_fx::BusGraph::sync")
    }
    /// Buses in evaluation order (leaves first, Main last).
    pub fn order(&self) -> Vec<Id> {
        todo!("engine::mixer_fx::BusGraph::order")
    }
    /// Scratch buffer for a bus (created on demand, zeroed for the block).
    pub fn buffer(&mut self, _bus: Id, _frames: usize) -> &mut [f32] {
        todo!("engine::mixer_fx::BusGraph::buffer")
    }
    /// Run one bus's filters + gain/pan/mono and add the result into its output bus (or `out` for Main).
    pub fn flush(&mut self, _bus: &Bus, _t: f64, _out: &mut [f32]) {
        todo!("engine::mixer_fx::BusGraph::flush")
    }
    /// Peak level (L, R) of the last block, for the mixer meters.
    pub fn meter(&self, _bus: Id) -> (f32, f32) {
        todo!("engine::mixer_fx::BusGraph::meter")
    }
}
