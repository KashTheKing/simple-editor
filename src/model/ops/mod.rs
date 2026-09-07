mod assets;
mod attrs;
mod autocut;
mod buses;
mod editing;
// ---- ws:inspector-gallery ----
mod effects;
mod graph;
mod markers;
// ---- ws:pro-monitor ----
mod multicam;
mod paths;
mod planner;
mod queries;
mod sequences;
mod shapes;
// ws:transcript-captions: `transcript_hits` is a free fn (search over `&[Transcript]`, no `Project`
// method), so — same reasoning as `trim` below — the module is `pub` for `ui::transcript_ui` and
// `ui::app::tools_transcript` to name it.
pub mod subtitles;
mod templates;
// ws:trim-model: `TrackFlag`/`EditPoint`/`Side` are free-standing types (not `impl Project` methods),
// so — unlike every other ops file, which never needs a module-path to be usable from outside
// `model::ops` (method calls resolve through the already-`pub` `Project` type regardless of the
// defining module's own visibility) — these two need `pub` so `ui::app::tools_trim`/`trim_actions`
// and snap-engine's `TimelineState.edit_point: Option<crate::model::ops::trim::EditPoint>` can name them.
pub mod tracks;
mod transitions;
pub mod trim;
