//! ---- ws:voice-input ----
//! Push-to-talk dictation: record the microphone with a cpal input stream (no ffmpeg start-up to clip
//! the first word), write a 16 kHz mono 16-bit WAV, run whisper over it (`transcribe::plain_text`).
//!
//! The cpal `Stream` is not `Send`, so it is built, kept and dropped on the recorder's own thread;
//! the UI talks to that thread through two channels (stop in, finished WAV out).

use crate::engine::transcribe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A recording stops by itself after this long.
pub const MAX_SECS: u64 = 60;
/// What whisper wants.
pub const RATE: u32 = 16_000;

/// A running recording. `stop()` ends it; `try_finish()` yields the WAV (or why there is none) once
/// the thread has written it - also when it stopped by itself at `MAX_SECS` or on a device error.
/// Dropping it stops the recording and deletes the WAV nobody took.
pub struct Recorder {
    stop: Sender<()>,
    done: Receiver<Result<PathBuf, String>>,
}

impl Recorder {
    /// Record `mic` (a device name, e.g. Settings' voiceover microphone) or, when that is empty or not
    /// found, the default input device.
    pub fn start(mic: &str) -> Recorder {
        let (stop, stop_rx) = mpsc::channel::<()>();
        let (done_tx, done) = mpsc::channel();
        let mic = mic.trim().to_string();
        std::thread::spawn(move || {
            let r = record(&mic, &stop_rx);
            if let Err(mpsc::SendError(Ok(p))) = done_tx.send(r) {
                let _ = std::fs::remove_file(p); // the Recorder was dropped: nobody wants it
            }
        });
        Recorder { stop, done }
    }

    pub fn stop(&self) {
        let _ = self.stop.send(());
    }

    /// The finished WAV, once there is one (None while still recording or writing).
    pub fn try_finish(&self) -> Option<Result<PathBuf, String>> {
        match self.done.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the recorder stopped unexpectedly".into())),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // the thread sees the stop (or the hung-up channel), drops the stream and deletes its WAV
        let _ = self.stop.send(());
    }
}

/// The recorder thread: open the device, collect mono samples until stopped / `MAX_SECS`, write the WAV.
fn record(mic: &str, stop: &Receiver<()>) -> Result<PathBuf, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    // ponytail: Settings.capture_mic is a dshow name; WASAPI's friendly name is normally the same
    // string, so an exact match picks it and anything else falls back to the default input.
    let chosen = if mic.is_empty() {
        None
    } else {
        host.input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| d.description().map(|x| x.name() == mic).unwrap_or(false)))
    };
    let device = chosen.or_else(|| host.default_input_device()).ok_or("No microphone")?;
    let supported = device.default_input_config().map_err(|e| format!("Microphone: {e}"))?;
    let rate = supported.sample_rate().max(1);
    let config = supported.config();
    let cap = (rate as u64 * MAX_SECS) as usize;
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 10)));
    let b = buf.clone();
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, b, cap),
        cpal::SampleFormat::I16 => build::<i16>(&device, config, b, cap),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, b, cap),
        cpal::SampleFormat::U16 => build::<u16>(&device, config, b, cap),
        cpal::SampleFormat::U8 => build::<u8>(&device, config, b, cap),
        f => return Err(format!("Microphone sample format {f:?} isn't supported")),
    }
    .map_err(|e| format!("Microphone: {e}"))?;
    stream.play().map_err(|e| format!("Microphone: {e}"))?;
    // a stop, the 60 s cap, or the Recorder dropped (channel hung up) all end the take
    let _ = stop.recv_timeout(Duration::from_secs(MAX_SECS));
    drop(stream);
    let mono = std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()));
    if mono.is_empty() {
        return Err("No sound from the microphone".into());
    }
    let wav = temp_wav();
    std::fs::write(&wav, wav_bytes(&resample(&mono, rate, RATE))).map_err(|e| format!("{}: {e}", wav.display()))?;
    Ok(wav)
}

/// An input stream that downmixes each frame of `T` to one f32 sample into `buf` (at the device rate),
/// up to `cap` samples.
fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    buf: Arc<Mutex<Vec<f32>>>,
    cap: usize,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::traits::DeviceTrait;
    let ch = config.channels.max(1) as usize;
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            downmix(data, ch, &mut buf.lock().unwrap_or_else(|e| e.into_inner()), cap);
        },
        |e| eprintln!("voice: {e}"),
        None,
    )
}

/// Interleaved `ch`-channel samples → their per-frame mean as f32, appended to `out` up to `cap`.
fn downmix<T>(data: &[T], ch: usize, out: &mut Vec<f32>, cap: usize)
where
    T: cpal::Sample,
    f32: cpal::FromSample<T>,
{
    for frame in data.chunks(ch.max(1)) {
        if out.len() >= cap {
            break;
        }
        let sum: f32 = frame.iter().map(|&s| <f32 as cpal::Sample>::from_sample(s)).sum();
        out.push(sum / frame.len() as f32);
    }
}

/// Linear-interpolation resample of mono `x` from `from` Hz to `to` Hz.
/// ponytail: no anti-alias filter - fine for speech going into whisper; use a windowed-sinc if a
/// 44.1/48 kHz mic ever measurably hurts recognition.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let n = (x.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let t = i as f64 * step;
            let j = t as usize;
            let f = (t - j as f64) as f32;
            let a = x[j.min(x.len() - 1)];
            let b = x[(j + 1).min(x.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

/// A 16-bit PCM mono WAV at `RATE`.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    v.extend_from_slice(&2u16.to_le_bytes()); // block align
    v.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    v
}

fn temp_wav() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("se-voice-{}-{n}.wav", std::process::id()))
}

/// whisper's non-speech tags (`[BLANK_AUDIO]`, `(silence)`, `[MUSIC]` …) removed, whitespace collapsed.
pub fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// whisper over a recorded WAV → plain text (tags stripped). Deletes the WAV. Blocks.
pub fn transcribe_short(wav: &Path) -> Result<String, String> {
    let r = transcribe::plain_text(wav);
    let _ = std::fs::remove_file(wav);
    let text = strip_tags(&r?);
    if text.is_empty() {
        return Err("Didn't catch that".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_length_and_level() {
        // 1 s of a 440 Hz sine at 48 kHz, half scale
        let w = 440.0 * std::f32::consts::TAU / 48_000.0;
        let sine: Vec<f32> = (0..48_000).map(|i| (i as f32 * w).sin() * 0.5).collect();
        let out = resample(&sine, 48_000, RATE);
        assert_eq!(out.len(), 16_000);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
        let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
        assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.01, "{rms}");
        // upsampling (an 8 kHz mic) doubles the length; same rate is a copy
        assert_eq!(resample(&[0.0; 8000], 8000, RATE).len(), 16_000);
        assert_eq!(resample(&[0.25, 0.5], RATE, RATE), vec![0.25, 0.5]);
        assert!(resample(&[], 48_000, RATE).is_empty());
    }

    #[test]
    fn downmix_averages_frames_and_caps() {
        let mut out = Vec::new();
        downmix(&[0.5f32, -0.5, 1.0, 0.0], 2, &mut out, 100);
        assert_eq!(out, vec![0.0, 0.5]);
        // i16 full scale converts to ~1.0
        let mut out = Vec::new();
        downmix(&[i16::MAX, i16::MAX], 2, &mut out, 100);
        assert!((out[0] - 1.0).abs() < 1e-3, "{out:?}");
        // the cap stops it
        let mut out = Vec::new();
        downmix(&[0.1f32; 10], 1, &mut out, 4);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn wav_header_is_16k_mono_pcm16() {
        let b = wav_bytes(&[0.0, 1.0, -1.0]);
        assert_eq!(b.len(), 44 + 6);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([b[20], b[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([b[22], b[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 32_000);
        assert_eq!(u16::from_le_bytes([b[34], b[35]]), 16);
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 6);
        assert_eq!(i16::from_le_bytes([b[46], b[47]]), 32767);
        assert_eq!(i16::from_le_bytes([b[48], b[49]]), -32767);
    }

    #[test]
    fn tags_are_stripped() {
        assert_eq!(strip_tags("  [BLANK_AUDIO]  "), "");
        assert_eq!(strip_tags("(silence)"), "");
        assert_eq!(strip_tags(" Cut the intro [MUSIC] please. "), "Cut the intro please.");
        assert_eq!(strip_tags("Hello (laughs) there"), "Hello there");
        assert_eq!(strip_tags("plain text"), "plain text");
    }
}
