//! `simple-editor --selftest [dir]` — headless end-to-end check (debug builds print to the console).
//! Generates synthetic media with ffmpeg (solid colour segments + two sine audio streams), then verifies:
//! probe → project layout; video decode at known times (colour & seek accuracy) for both backends;
//! audio decode RMS; compositor (blend/opacity/text layer); mixer; waveform peaks; export (mp4) →
//! ffprobe duration; lossless cut; xmeml is well-formed. Prints PASS/FAIL lines, returns 0 on success.

pub fn run(_args: &[String]) -> i32 {
    eprintln!("selftest not implemented");
    1
}
