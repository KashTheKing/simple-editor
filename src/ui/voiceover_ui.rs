//! ---- ws:panels-multiscreen ----
//! The Voiceover pane (was the Voiceover window of `capture_ui`): input device and channels, a live level
//! meter, Record / Stop, "Punch in at the playhead" (playback rolls from the playhead while recording, so
//! the take lands under what you hear), Retake, and the takes already on the timeline (every clip of a
//! `voice-*.wav` recording): click a take's time to seek to it, the cross deletes its clips.
//!
//! App-free like the other pane bodies: it returns what was asked for and `ui::app::voiceover_pane` does it.

use crate::engine::capture::VoiceoverOptions;
use crate::model::{Id, Project};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::capture_ui::{self, CaptureUi};
use crate::ui::combo;
use crate::ui::tools::{glyph_label, glyph_text_button, Glyph};
use eframe::egui;
use std::path::Path;

/// One take on the timeline: a voiceover recording's clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    pub clip: Id,
    pub file: String,
    pub at: f64,
    pub duration: f64,
}

/// What the pane asked for this frame.
#[derive(Default)]
pub struct VoResponse {
    /// Start a take with these options (Record or Retake).
    pub start: Option<VoiceoverOptions>,
    pub stop: bool,
    /// Throw the last take away first (Retake).
    pub retake: bool,
    pub seek: Option<f64>,
    /// Delete this take's clip.
    pub delete: Option<Id>,
}

/// A voiceover recording's file name: what `capture_ui::voice_options` writes (`voice-<time>.wav`).
pub fn is_take_file(path: &str) -> bool {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.starts_with("voice-") && name.ends_with(".wav")
}

/// The takes on the timeline, in time order: every audio clip of a voiceover recording.
pub fn takes(project: &Project) -> Vec<Take> {
    let mut out: Vec<Take> = project
        .all_clips()
        .filter_map(|(_, c)| {
            let a = project.asset(c.asset)?;
            (is_take_file(&a.path) && !a.has_video()).then(|| Take {
                clip: c.id,
                file: a.name(),
                at: c.start,
                duration: c.duration,
            })
        })
        .collect();
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
}

/// Peak level 0..1 of the last ~50 ms written to a recording 16-bit PCM WAV (ffmpeg writes it as it
/// records). Reads ~10 KB off the end of the file; `None` while it is too short or unreadable.
pub fn wav_tail_peak(path: &Path) -> Option<f32> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 9600; // 50 ms of 48 kHz stereo s16
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len < 44 + 64 {
        return None;
    }
    let from = (len - TAIL.min(len - 44)) & !1; // sample-aligned: the header is an even number of bytes
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity(TAIL as usize);
    f.take(TAIL).read_to_end(&mut buf).ok()?;
    let peak = buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max()?;
    Some(peak as f32 / 32768.0)
}

/// The pane. `devices` = the dshow inputs (`App::audio_inputs`, probed once), `recording` = a take is
/// running.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    st: &mut CaptureUi,
    settings: &mut Settings,
    devices: &[(String, bool)],
    recording: bool,
    playhead: f64,
    takes: &[Take],
    palette: &Palette,
) -> VoResponse {
    let mut r = VoResponse::default();
    ui.add_enabled_ui(!recording, |ui| {
        egui::Grid::new("vo_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Input");
            let opts: Vec<(&str, &str)> = devices.iter().map(|(n, _)| (n.as_str(), n.as_str())).collect();
            combo(ui, "vo_dev", &mut settings.voice_device, &opts, Some(190.0));
            ui.end_row();
            ui.label("Channels");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut settings.voice_channels, 1, "Mono");
                ui.selectable_value(&mut settings.voice_channels, 2, "Stereo");
            });
            ui.end_row();
        });
        ui.checkbox(&mut st.from_playhead, "Punch in at the playhead")
            .on_hover_text("Playback rolls from the playhead while you record, and the take lands where you started");
    });
    capture_ui::meter(ui, if recording { st.level } else { 0.0 }, palette);
    ui.horizontal(|ui| {
        if recording {
            glyph_label(ui, Glyph::Record, ui.visuals().error_fg_color);
            ui.colored_label(ui.visuals().error_fg_color, "REC");
            ui.label(capture_ui::clock(st.elapsed));
            if ui.button("Stop").clicked() {
                r.stop = true;
            }
        } else {
            let can = !settings.voice_device.is_empty();
            let rec = ui.add_enabled_ui(can, |ui| glyph_text_button(ui, Glyph::Mic, "Record")).inner;
            if rec.on_disabled_hover_text("Pick an input").clicked() {
                r.start = Some(capture_ui::voice_options(settings));
                settings.save();
            }
            if st.last_take.is_some() && ui.add_enabled(can, egui::Button::new("Retake")).clicked() {
                r.retake = true;
                r.start = Some(capture_ui::voice_options(settings));
            }
            ui.weak(format!("at {}", crate::ui::duration_text(playhead)));
        }
    });
    ui.separator();
    if takes.is_empty() {
        ui.weak("No takes yet");
        return r;
    }
    // aligned rows: [time][name, truncates][length][×]
    egui::ScrollArea::vertical().id_salt("vo_takes").auto_shrink([false, true]).show(ui, |ui| {
        for t in takes {
            ui.horizontal(|ui| {
                if ui.add_sized([64.0, 18.0], egui::Button::new(crate::ui::duration_text(t.at))).clicked() {
                    r.seek = Some(t.at);
                }
                let right = 64.0 + 26.0;
                let w = (ui.available_width() - right).max(40.0);
                ui.add_sized([w, 18.0], egui::Label::new(&t.file).truncate());
                ui.add_sized([64.0, 18.0], egui::Label::new(crate::ui::duration_text(t.duration)));
                if ui.small_button("×").on_hover_text("Delete this take").clicked() {
                    r.delete = Some(t.clip);
                }
            });
        }
    });
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_files_are_the_recorder_names() {
        assert!(is_take_file(r"C:\Videos\voice-20261117-101500.wav"));
        assert!(is_take_file("VOICE-1.WAV"));
        assert!(!is_take_file(r"C:\Videos\screen-1.mp4"));
        assert!(!is_take_file("myvoice-1.wav"));
    }

    #[test]
    fn wav_tail_peak_reads_the_last_samples() {
        let p = std::env::temp_dir().join(format!("se-vo-peak-{}.wav", std::process::id()));
        let mut bytes = vec![0u8; 44];
        for i in 0..4800i32 {
            let v: i16 = if i == 4000 { 16384 } else { 0 };
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&p, &bytes).unwrap();
        let peak = wav_tail_peak(&p).unwrap();
        assert!((peak - 0.5).abs() < 1e-3, "{peak}");
        std::fs::write(&p, [0u8; 50]).unwrap();
        assert_eq!(wav_tail_peak(&p), None, "too short");
        let _ = std::fs::remove_file(&p);
    }

    fn run(st: &mut CaptureUi, rec: bool, takes: &[Take]) -> (VoResponse, bool) {
        let ctx = egui::Context::default();
        let palette = Palette::new(false, egui::Color32::BLUE);
        let mut s = Settings::default();
        s.voice_device = "Mic".into();
        let devices = vec![("Mic".to_string(), false)];
        let mut r = VoResponse::default();
        let mut repaint = false;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    r = show(ui, st, &mut s, &devices, rec, 1.5, takes, &palette);
                });
            });
            repaint = ctx.has_requested_repaint();
        }
        (r, repaint)
    }

    /// Idle, recording and with takes: the pane lays out, asks for nothing and never asks to repaint
    /// (the app's own 100 ms tick drives the meter only while a take runs).
    #[test]
    fn assert_no_idle_repaint_voiceover_pane() {
        let takes = vec![Take { clip: 1, file: "voice-1.wav".into(), at: 2.0, duration: 3.0 }];
        for rec in [false, true] {
            let mut st = CaptureUi { level: 0.4, elapsed: 2.0, ..Default::default() };
            let (r, repaint) = run(&mut st, rec, &takes);
            assert!(r.start.is_none() && !r.stop && !r.retake && r.seek.is_none() && r.delete.is_none());
            assert!(!repaint, "idle Voiceover pane requested a repaint (recording: {rec})");
        }
    }
}
