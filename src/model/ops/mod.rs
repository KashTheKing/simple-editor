mod assets;
mod attrs;
// ws:beat-aligner: `CutScope` is named by the aligner's tools and pane
pub mod beats;
// ws:browser-layer: `CursorMove` is a free-standing type the tools name, hence `pub` (as `trim`)
pub mod browser;
mod autocut;
mod buses;
mod context; // feat/styles-context
mod editing;
// ---- ws:inspector-gallery ----
mod effects;
mod graph;
// ws:forum-ui: `Layout` / `Censor` are named by the tools and menus
pub mod findability;
mod keyblocks;
mod linked_bins;
pub mod convert; // ws:animator: whole-timeline audit, keys -> blocks, transitions -> spans
mod stacks; // ws:animator
pub mod spans; // ws:animator (`MIN_SPAN`)
mod markers;
pub mod zones; // ws:zones-markers (`MIN_ZONE`)
// ---- ws:pro-monitor ----
mod multicam;
mod paths;
mod planner;
pub mod playlists; // ws:music-burn (`spaced_length`)
mod queries;
mod references; // feat/reference-clips
mod sequences;
mod shapes;
// ws:transcript-captions: `transcript_hits` is a free fn (search over `&[Transcript]`, no `Project`
// method), so - same reasoning as `trim` below - the module is `pub` for `ui::transcript_ui` and
// `ui::app::tools_transcript` to name it.
pub mod subtitles;
mod templates;
mod timeremap; // ws:time-remap
// ws:trim-model: `TrackFlag`/`EditPoint`/`Side` are free-standing types (not `impl Project` methods),
// so - unlike every other ops file, which never needs a module-path to be usable from outside
// `model::ops` (method calls resolve through the already-`pub` `Project` type regardless of the
// defining module's own visibility) - these two need `pub` so `ui::app::tools_trim`/`trim_actions`
// and snap-engine's `TimelineState.edit_point: Option<crate::model::ops::trim::EditPoint>` can name them.
pub mod tracks;
mod transitions;
pub mod trim;
pub mod montage; // ws:parity-editing: Make Montage
