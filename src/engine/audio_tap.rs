//! Screen-recording audio: a cpal (WASAPI) input stream on a background thread that writes raw
//! interleaved f32le to a side file and keeps a peak level for the meter.
//!
//! * System audio is WASAPI loopback of the default output device (cpal turns an input stream on a
//!   render device into loopback), so no "Stereo Mix" or virtual cable is needed.
//! * Raw PCM has no header to finalise: a crash or power loss keeps everything flushed so far (the
//!   thread flushes every 100 ms), and `capture::finalize` muxes it with `-f f32le -ar R -ac C`.
//! * Loopback delivers no packets while nothing plays. Each packet's capture time (QPC) says where it
//!   belongs, so gaps are filled with silence and the file stays in step with the wall clock.
//! * `t0_us` is the wall-clock time (µs since the Unix epoch) of the file's first sample - the same clock
//!   gdigrab stamps video frames with, which is how audio and video line up exactly.
//!
//! With no output file the tap only meters (the Screen Recording pane's level bars before recording).

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq)]
pub enum TapSource {
    /// Loopback of the default output device.
    System,
    /// A microphone by name ("" or not found = the default input device).
    Mic(String),
}

/// What the meter and the recorder read while the tap runs.
#[derive(Default)]
pub struct TapState {
    /// Peak of the last ~100 ms, f32 bits, 0..1.
    peak: AtomicU32,
    /// µs since the Unix epoch of the first sample written (0 until the first packet).
    pub t0_us: AtomicU64,
    pub rate: AtomicU32,
    pub channels: AtomicU32,
    /// Set when the device failed (unplugged, no default device...).
    pub error: Mutex<Option<String>>,
    pub running: AtomicBool,
}

impl TapState {
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// A running tap; dropping it stops the stream and flushes the file.
pub struct AudioTap {
    pub state: Arc<TapState>,
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioTap {
    pub fn start(source: TapSource, out: Option<PathBuf>) -> AudioTap {
        let state = Arc::new(TapState::default());
        let (stop, stop_rx) = mpsc::channel();
        let st = state.clone();
        let thread = std::thread::Builder::new()
            .name("audio-tap".into())
            .spawn(move || {
                st.running.store(true, Ordering::SeqCst);
                if let Err(e) = run(&source, out, &st, &stop_rx) {
                    *st.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
                }
                st.running.store(false, Ordering::SeqCst);
            })
            .ok();
        AudioTap { state, stop, thread }
    }

    /// Stop and wait until the file is flushed and closed.
    pub fn stop(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for AudioTap {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Microphone names (WASAPI friendly names - the same strings dshow lists).
pub fn input_names() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| it.filter_map(|d| d.description().ok().map(|x| x.name().to_string())).collect())
        .unwrap_or_default()
}

/// Shared between the cpal callback and the tap thread.
#[derive(Default)]
struct Pending {
    samples: Vec<f32>,
    /// Frames handed out so far (written + padding), the position the next packet is checked against.
    frames: u64,
    first: Option<cpal::StreamInstant>,
    peak: f32,
}

fn run(source: &TapSource, out: Option<PathBuf>, st: &TapState, stop: &Receiver<()>) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let (device, cfg) = match source {
        TapSource::System => {
            let d = host.default_output_device().ok_or("No audio output device for system audio")?;
            let c = d.default_output_config().map_err(|e| format!("System audio: {e}"))?;
            (d, c)
        }
        TapSource::Mic(name) => {
            let name = name.trim();
            let chosen = (!name.is_empty())
                .then(|| {
                    host.input_devices().ok().and_then(|mut it| {
                        it.find(|d| d.description().map(|x| x.name() == name).unwrap_or(false))
                    })
                })
                .flatten();
            let d = chosen.or_else(|| host.default_input_device()).ok_or("No microphone")?;
            let c = d.default_input_config().map_err(|e| format!("Microphone: {e}"))?;
            (d, c)
        }
    };
    let rate = cfg.sample_rate().max(1);
    let ch = cfg.channels().max(1) as usize;
    st.rate.store(rate, Ordering::SeqCst);
    st.channels.store(ch as u32, Ordering::SeqCst);
    let pending = Arc::new(Mutex::new(Pending::default()));
    let config = cfg.config();
    let p = pending.clone();
    let t0 = Arc::new(AtomicU64::new(0));
    let t0c = t0.clone();
    macro_rules! stream {
        ($t:ty) => {
            device.build_input_stream(
                config,
                move |data: &[$t], info: &cpal::InputCallbackInfo| {
                    let mut g = p.lock().unwrap_or_else(|e| e.into_inner());
                    let ts = info.timestamp();
                    let first = *g.first.get_or_insert(ts.capture);
                    if t0c.load(Ordering::Relaxed) == 0 {
                        // the wall clock of this packet's first sample: now minus how long ago it was captured
                        let lag = ts.callback.saturating_duration_since(ts.capture);
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                        t0c.store(now.saturating_sub(lag).as_micros().max(1) as u64, Ordering::Relaxed);
                    }
                    let at = ts.capture.saturating_duration_since(first).as_secs_f64();
                    let gap = gap_frames(at, rate, g.frames);
                    if gap > 0 {
                        let n = gap as usize * ch;
                        let len = g.samples.len();
                        g.samples.resize(len + n, 0.0);
                        g.frames += gap;
                    }
                    let mut peak = g.peak;
                    for &s in data {
                        let v = <f32 as cpal::Sample>::from_sample(s);
                        peak = peak.max(v.abs());
                        g.samples.push(v);
                    }
                    g.peak = peak;
                    g.frames += (data.len() / ch) as u64;
                },
                |e| eprintln!("audio tap: {e}"),
                None,
            )
        };
    }
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32),
        cpal::SampleFormat::I16 => stream!(i16),
        cpal::SampleFormat::I32 => stream!(i32),
        cpal::SampleFormat::U16 => stream!(u16),
        f => return Err(format!("Audio sample format {f:?} isn't supported")),
    }
    .map_err(|e| format!("Audio device: {e}"))?;
    stream.play().map_err(|e| format!("Audio device: {e}"))?;

    let mut file = match &out {
        Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?)),
        None => None,
    };
    let mut bytes = Vec::new();
    loop {
        let stopped = !matches!(stop.recv_timeout(Duration::from_millis(100)), Err(RecvTimeoutError::Timeout));
        let (samples, peak) = {
            let mut g = pending.lock().unwrap_or_else(|e| e.into_inner());
            (std::mem::take(&mut g.samples), std::mem::take(&mut g.peak))
        };
        st.peak.store(peak.min(1.0).to_bits(), Ordering::Relaxed);
        if st.t0_us.load(Ordering::Relaxed) == 0 {
            st.t0_us.store(t0.load(Ordering::Relaxed), Ordering::SeqCst);
        }
        if let Some(f) = file.as_mut() {
            bytes.clear();
            bytes.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
            f.write_all(&bytes).and_then(|_| f.flush()).map_err(|e| format!("Writing audio: {e}"))?;
        }
        if stopped {
            break;
        }
    }
    drop(stream);
    Ok(())
}

/// Silent frames to insert before a packet captured `at` seconds after the first one, when `have`
/// frames are already out. Jitter under 20 ms is left alone (packets are ~10 ms apart).
pub(crate) fn gap_frames(at: f64, rate: u32, have: u64) -> u64 {
    let want = (at * rate as f64).round().max(0.0) as u64;
    let slack = rate as u64 / 50;
    if want > have + slack {
        want - have
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_are_filled_only_past_jitter() {
        assert_eq!(gap_frames(0.0, 48_000, 0), 0);
        assert_eq!(gap_frames(0.010, 48_000, 0), 0, "10 ms of jitter is not a gap");
        // nothing played for 2 s: the next packet lands 2 s after the last 0.5 s of audio
        assert_eq!(gap_frames(2.5, 48_000, 24_000), 96_000);
        assert_eq!(gap_frames(1.0, 48_000, 60_000), 0, "never cut audio that ran ahead");
    }

    /// Hardware check (`cargo test -- --ignored audio_tap_writes`): ~1 s of system audio to a raw file.
    #[test]
    #[ignore]
    fn audio_tap_writes_system_audio() {
        let out = std::env::temp_dir().join(format!("se-tap-{}.f32", std::process::id()));
        let tap = AudioTap::start(TapSource::System, Some(out.clone()));
        std::thread::sleep(Duration::from_millis(1200));
        let st = tap.state.clone();
        tap.stop();
        assert!(st.error.lock().unwrap().is_none(), "{:?}", st.error.lock().unwrap());
        let len = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let _ = std::fs::remove_file(&out);
        assert!(st.rate.load(Ordering::SeqCst) > 0, "the format is known");
        // loopback may stay silent (no packets) when nothing plays; the file must still exist
        assert!(len % 4 == 0, "{len}");
    }
}
