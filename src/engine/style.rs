//! Style summary: a deterministic Markdown description of how a project was edited — statistics an AI (or a
//! human) can turn into a reusable style guide: format, duration, tracks, cut cadence (count, mean/median
//! clip length, shortest/longest), transitions used (kinds, durations), effects (kinds + typical params),
//! text styles (fonts, sizes, colours, outlines/shadows), colour labels, retimes, audio (levels, fades,
//! music/SFX split by length), subtitles usage, the planner (done/todo), and the free-form notes verbatim.
//! Also served by the MCP tool `style.summary` and exported via File ▸ Export Style Summary (.md).

use crate::model::Project;

pub fn style_summary(_project: &Project) -> String {
    todo!("engine::style::style_summary")
}
