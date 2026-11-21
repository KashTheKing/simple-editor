//! ---- ws:visualizer-helper ----
//! Visualizer Helper's native side: what the Python scripts of the hardstyle visualizer build did
//! (`plans/visualizer-helper/`), in Rust so users need no Python. The plugin's panel
//! (`library/plugins/visualizer-helper/`) drives it through the `audio.analyze`, `visualizer.template`,
//! `lyrics.karaoke`, `visualizer.drop_fx` and `media.loop` tools (`ui::app::vizhelp_ctl`).

pub mod analyze;
pub mod dsp;
pub mod keys;
pub mod loopbg;
pub mod lyrics;
pub mod shader;
