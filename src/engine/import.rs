//! Import timelines from other editors, with an honest per-item report.
//!
//! Supported: Final Cut Pro 7 XML (`.xml` - what Premiere Pro exports and DaVinci Resolve reads/writes),
//! EDL (CMX 3600) and Premiere `.prproj` (gzip-wrapped XML: inflate, then the same reader). Anything the
//! reader cannot map (unknown effects, missing media, speed ramps, colour pages) becomes an `Issue`
//! instead of failing the import.

use crate::model::Project;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Ok,
    Warning,
    Skipped,
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub level: Level,
    /// What it was about ("clip 'a.mp4'", "effect 'Gaussian Blur'", "track V2").
    pub subject: String,
    pub detail: String,
}

#[derive(Debug)]
pub struct ImportReport {
    pub project: Project,
    pub issues: Vec<Issue>,
    pub clips: usize,
    pub tracks: usize,
    pub missing_media: usize,
}

impl ImportReport {
    pub fn ok(&self) -> usize {
        self.issues.iter().filter(|i| i.level == Level::Ok).count()
    }
    pub fn problems(&self) -> usize {
        self.issues.iter().filter(|i| i.level != Level::Ok).count()
    }
    /// Markdown table for the import dialog / clipboard.
    pub fn to_markdown(&self) -> String {
        todo!("engine::import::ImportReport::to_markdown")
    }
}

/// Detect the format by extension + content and import it.
pub fn import_file(_path: &std::path::Path) -> Result<ImportReport, String> {
    todo!("engine::import::import_file")
}

pub const IMPORT_EXTS: &[&str] = &["xml", "fcpxml", "edl", "prproj"];
