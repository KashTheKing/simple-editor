//! ---- ws:registries-schema-hooks ----
//! `AssetStatus` / `App::asset_status`: a no-op stub (always `Ready`) that ws:media-library (wave 2)
//! fills with real decode/proxy tracking. reconcile: this `ProxyBuilding(u8)` payload and the
//! `App::asset_status(&self, id)` method form (not a free fn) are the canonical shape other
//! workstreams (media-library) must converge onto, not redefine.

use super::*;

/// Per-asset media status for the library / inspector / preview badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Decoding/Offline/ProxyBuilding are unused until media-library (wave 2)
pub(crate) enum AssetStatus {
    Ready,
    Decoding,
    Offline,
    /// Percent complete (0..=100).
    ProxyBuilding(u8),
}

impl App {
    /// Always `Ready` this wave — media-library (wave 2) wires real decode/proxy-build tracking here.
    #[allow(dead_code)] // unused until media-library (wave 2) has a caller
    pub(crate) fn asset_status(&self, asset: Id) -> AssetStatus {
        let _ = asset;
        AssetStatus::Ready
    }
}
