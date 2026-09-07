//! ---- ws:registries-schema-hooks ----
//! `App::place_asset` / `DropMode`: a no-op-behavior stub source-monitor (wave 2) fills in. Today every
//! drop/place path already goes through `App::insert_at` (chaining `Project::insert_asset_clips`); this
//! wraps that in the shape a future drop-modifier UI (Ctrl=Splice, Alt=Overwrite, Shift=Place on Top)
//! will dispatch through, without changing today's single always-Place behavior.

use super::*;

/// How a drop/place should interact with what's already on the track. Only `Place` is implemented —
/// the others are reachable (won't panic) but currently fall back to `Place`.
/// ponytail: Splice/Overwrite/OnTop land with source-monitor (wave 2); this stub exists so drop-site
/// call sites can already pass a `DropMode` without a breaking signature change later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Splice/Overwrite/OnTop are unused until source-monitor (wave 2)
pub(crate) enum DropMode {
    Place,
    Splice,
    Overwrite,
    OnTop,
}

impl App {
    /// Place `asset` at timeline time `at` (video on `track` if given). `mode` is accepted but ignored
    /// this wave — every mode behaves like `Place` (wraps `Project::insert_asset_clips`), matching
    /// today's only drop behavior exactly.
    #[allow(dead_code)] // unused until source-monitor (wave 2) has a caller for a non-Place mode
    pub(crate) fn place_asset(&mut self, asset: Id, at: f64, track: Option<usize>, mode: DropMode) -> Vec<Id> {
        let _ = mode; // ponytail: mode is ignored until source-monitor (wave 2)
        self.project.insert_asset_clips(asset, at, track)
    }
}
