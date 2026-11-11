//! ---- ws:html-layer ----
//! HTML layers, App side: render jobs (a finished render swaps the asset's file), the HTML editor
//! window, Add ▸ HTML Layer / Scene.

use super::*;

/// Render jobs in flight, the editor window's state.
#[derive(Default)]
pub struct HtmlState {}

pub(super) fn act(_app: &mut App, _a: Action) -> bool {
    false
}

pub(super) fn tick(_app: &mut App, _ctx: &egui::Context) {}

pub(super) fn windows(_app: &mut App, _ctx: &egui::Context) {}

impl App {
    /// CONTRACT (used by the library's "apply" and by `html.create`): add an HTML layer / scene clip
    /// from template source `html` (the format in `library/README.md`) and start rendering it.
    /// `at` = timeline second (None: the playhead), `duration` (None: the template's `se:duration`),
    /// `fields` = values overriding the template's defaults. Returns the new clip's id. Does NOT push
    /// undo itself: a `ToolKind::Mutate` tool or the caller does.
    pub(crate) fn add_html_clip(
        &mut self,
        _html: &str,
        _name: Option<&str>,
        _at: Option<f64>,
        _duration: Option<f64>,
        _fields: &[(String, String)],
    ) -> Result<Id, String> {
        Err("HTML layers are not built yet".into())
    }
}
