//! ---- ws:shader-library ----
//! Shader effects and shader transitions, App side: the `clip.set_shader` / `shader.check` /
//! transition-shader MCP tools and the two "apply" entry points the library uses.

use super::*;
use crate::mcp::tools::ToolDef;

#[derive(Default)]
pub struct ShaderLibState {}

pub(super) fn act(_app: &mut App, _a: Action) -> bool {
    false
}

pub(super) fn tick(_app: &mut App, _ctx: &egui::Context) {}

pub(super) fn windows(_app: &mut App, _ctx: &egui::Context) {}

impl App {
    /// CONTRACT (used by the library's "apply"): add a shader effect (source in the
    /// `library/README.md` format) to `clip`, or to every selected clip when None. Returns how many
    /// clips got it. Does NOT push undo itself.
    pub(crate) fn apply_shader_effect(&mut self, _src: &str, _clip: Option<Id>) -> Result<usize, String> {
        Err("shader effects from the library are not built yet".into())
    }

    /// CONTRACT (used by the library's "apply"): put a shader transition (source in the
    /// `library/README.md` format) on the cut at the start of `clip` - or, when None, on the selected
    /// transition, else on the selected clip's cut, else on the cut nearest the playhead - replacing
    /// what is there. Returns the transition's id. Does NOT push undo itself.
    pub(crate) fn apply_transition_shader(&mut self, _src: &str, _clip: Option<Id>) -> Result<Id, String> {
        Err("shader transitions are not built yet".into())
    }
}

pub const TOOLS: &[ToolDef] = &[];
