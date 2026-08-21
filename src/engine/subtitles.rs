//! Subtitle file formats: SubRip (.srt) and WebVTT (.vtt) — parse (auto-detected) and write.
//! Basic formatting tags (<i>, <b>, {\an8}, VTT cue settings) are stripped on import; text is kept
//! as plain lines. Times are seconds.

use crate::model::Cue;

/// Parse SRT or WebVTT (auto-detected by the "WEBVTT" header / timestamp style). Malformed blocks are
/// skipped. Returns (start, end, text) triples in file order.
pub fn parse(_text: &str) -> Vec<(f64, f64, String)> {
    todo!("engine::subtitles::parse")
}

pub fn to_srt(_cues: &[Cue]) -> String {
    todo!("engine::subtitles::to_srt")
}

pub fn to_vtt(_cues: &[Cue]) -> String {
    todo!("engine::subtitles::to_vtt")
}
