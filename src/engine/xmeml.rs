//! Final Cut Pro 7 XML (xmeml v5) exporter — the interchange format both Premiere Pro
//! (File → Import) and DaVinci Resolve (File → Import → Timeline) read.
//! Video tracks (with clipitems: file path, in/out/start/end in frames, opacity/position if animated)
//! and audio tracks (one per project audio track, channel-source by stream). Text clips are exported as
//! a generator-less gap (Premiere/Resolve text isn't interchangeable) — mention it in a comment.
//! Paths as `file://localhost/C:/...` pathurls. Timebase = round(fps), NTSC flag when fps is fractional.

use crate::model::Project;

pub fn export_xmeml(_project: &Project) -> String {
    todo!("engine::xmeml::export_xmeml")
}
