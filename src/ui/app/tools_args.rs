//! ---- ws:registries-schema-hooks ----
//! Typed MCP argument extraction, wrapping the existing `arg_str`/`arg_f64`/`arg_u64`/`arg_bool`/
//! `arg_ids`/`req` free fns (`tools_helpers.rs`) verbatim so a future tool's `run` fn stays 3-6 lines.
//! Nothing in this workstream's own new tools needs the defaulting helpers below yet (`ids_or_selection`/
//! `t_or_playhead`/`track_or_hovered`) - they exist for wave-1+ tools (e.g. a detection tool that
//! defaults its target to "whatever's selected").

use super::tools_helpers::{arg_bool, arg_f64, arg_ids, arg_str, arg_u64};
use super::App;
use crate::model::Id;
use serde_json::Value;

#[allow(dead_code)] // consumed by wave-1+ tools, not this workstream's own rows
pub(super) struct Args<'a>(pub &'a Value);

#[allow(dead_code)]
impl<'a> Args<'a> {
    pub(super) fn f64(&self, k: &str) -> Option<f64> {
        arg_f64(self.0, k)
    }
    pub(super) fn id(&self, k: &str) -> Option<Id> {
        arg_u64(self.0, k)
    }
    pub(super) fn ids(&self, k: &str) -> Option<Vec<Id>> {
        arg_ids(self.0, k)
    }
    pub(super) fn str(&self, k: &str) -> Option<&'a str> {
        arg_str(self.0, k)
    }
    pub(super) fn bool(&self, k: &str) -> Option<bool> {
        arg_bool(self.0, k)
    }
    /// Clip ids at `k`, defaulting to the current selection (expanded through links) when omitted -
    /// the "act on whatever's selected" shape most detection/edit tools want.
    pub(super) fn ids_or_selection(&self, k: &str, app: &App) -> Vec<Id> {
        match self.ids(k) {
            Some(ids) => app.project.expand_links(&ids),
            None => app.project.expand_links(&app.selection),
        }
    }
    /// Timeline time at `k`, defaulting to the current playhead when omitted.
    pub(super) fn t_or_playhead(&self, k: &str, app: &App) -> f64 {
        self.f64(k).unwrap_or(app.playhead)
    }
    /// Track index at `k`; `None` when omitted (the caller picks a free/preferred track itself).
    pub(super) fn track_or_hovered(&self, k: &str, _app: &App) -> Option<usize> {
        self.id(k).map(|t| t as usize)
    }
}
