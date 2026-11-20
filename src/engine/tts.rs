//! ---- ws:transcript-captions ----
//! Text-to-speech through Windows' own System.Speech (SAPI) via a powershell shell-out - no crate, no
//! bundled voices: `voices()` lists whatever the OS has installed (OS-tier voices, not neural ones),
//! `speak_to_wav` writes a WAV the app then imports as a linked asset. Same `export::spawn_job` /
//! `Progress` shape as `transcribe::download_model`, so the UI shows it like any other job.

use crate::engine::export::{self, Progress};
use crate::media::{self, ffpipe, Backend};
use crate::model::Asset;
use crate::engine::tts_addon;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

/// Where `speak_to_wav`'s worker leaves the probed WAV (ws:job-completion-hitches).
pub type ProbedWav = Arc<Mutex<Option<Result<Asset, String>>>>;

/// A PowerShell single-quoted literal (the only escape inside one is a doubled quote).
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The script `speak_to_wav` runs: load System.Speech, pick `voice` (when given), synthesize `text`
/// straight into `out`. Pure - see `tts_speak_builds_expected_powershell_command`.
pub fn script(text: &str, voice: Option<&str>, out: &Path) -> String {
    let mut s = String::from(
        "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; ",
    );
    if let Some(v) = voice.map(str::trim).filter(|v| !v.is_empty()) {
        s.push_str(&format!("$s.SelectVoice({}); ", ps_quote(v)));
    }
    s.push_str(&format!(
        "$s.SetOutputToWaveFile({}); $s.Speak({}); $s.Dispose()",
        ps_quote(&out.to_string_lossy()),
        ps_quote(text)
    ));
    s
}

/// The `powershell.exe` invocation for `script`, windowless (`ffpipe::command`). Not spawned here.
pub fn command(text: &str, voice: Option<&str>, out: &Path) -> Command {
    let mut c = ffpipe::command(Path::new("powershell.exe"));
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"]).arg(script(text, voice, out));
    c
}

// ---- ws:addon-voice ----
/// Which synthesiser speaks: the built-in Windows voices, or an add-on (`tts_addon`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Windows,
    Kokoro,
    Chatterbox,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Windows, Engine::Kokoro, Engine::Chatterbox];
    pub fn id(self) -> &'static str {
        match self {
            Engine::Windows => "windows",
            Engine::Kokoro => "kokoro",
            Engine::Chatterbox => "chatterbox",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Engine::Windows => "Windows",
            Engine::Kokoro => "Kokoro",
            Engine::Chatterbox => "Chatterbox",
        }
    }
    pub fn parse(s: &str) -> Option<Engine> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("sapi") {
            return Some(Engine::Windows);
        }
        Engine::ALL.into_iter().find(|e| e.id().eq_ignore_ascii_case(s))
    }
    pub fn addon(self) -> Option<tts_addon::Addon> {
        match self {
            Engine::Windows => None,
            Engine::Kokoro => Some(tts_addon::Addon::Kokoro),
            Engine::Chatterbox => Some(tts_addon::Addon::Chatterbox),
        }
    }
    pub fn available(self) -> bool {
        self.addon().is_none_or(|a| a.available())
    }
    /// The engine that will really speak: an add-on that is not installed falls back to Windows, with
    /// the reason (shown with the result, never silently).
    pub fn resolve(self) -> (Engine, Option<String>) {
        if self.available() {
            (self, None)
        } else {
            (Engine::Windows, Some(format!("{} voices are not installed; used a Windows voice", self.label())))
        }
    }
}

/// One thing to say and how. `voice` is engine-specific: a SAPI name, a Kokoro voice id ("af_heart"),
/// or for Chatterbox a cloned voice's name (None = the model's own default voice).
#[derive(Clone, Debug, PartialEq)]
pub struct Speech {
    pub text: String,
    pub engine: Engine,
    pub voice: Option<String>,
    /// 0.5..2, 1 = as spoken.
    pub speed: f32,
    /// Semitones, -12..12.
    pub pitch: f32,
    /// Chatterbox's exaggeration, 0.25..2 (0.5 = neutral).
    pub intensity: f32,
    /// Chatterbox: a reference clip to clone (any audio file; overrides `voice`).
    pub clone_ref: Option<PathBuf>,
}

impl Speech {
    pub fn new(text: impl Into<String>) -> Self {
        Speech { text: text.into(), engine: Engine::Windows, voice: None, speed: 1.0, pitch: 0.0, intensity: 0.5, clone_ref: None }
    }
    /// The cached WAV's name: the same request reuses the file instead of synthesising again.
    pub fn cache_name(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        let engine = self.engine.resolve().0;
        (self.text.trim(), engine.id(), &self.voice, self.clone_ref.as_ref().map(|p| p.to_string_lossy().into_owned())).hash(&mut h);
        ((self.speed * 100.0).round() as i32, (self.pitch * 10.0).round() as i32, (self.intensity * 100.0).round() as i32).hash(&mut h);
        // a re-recorded reference makes a new voice
        let reference = match (&self.clone_ref, &self.voice) {
            (Some(r), _) => Some(r.clone()),
            (None, Some(v)) if engine == Engine::Chatterbox => Some(voices_dir().join(format!("{v}.wav"))),
            _ => None,
        };
        reference.and_then(|r| std::fs::metadata(r).and_then(|m| m.modified()).ok()).hash(&mut h);
        format!("{}-{:016x}.wav", engine.id(), h.finish())
    }
}

/// The ffmpeg filter for speed and pitch: `asetrate` moves the pitch (and the tempo), `atempo` puts the
/// tempo where `speed` wants it. None = nothing to change. `rate` = the WAV's sample rate.
pub fn speed_pitch_filter(speed: f32, pitch: f32, rate: u32) -> Option<String> {
    let speed = speed.clamp(0.5, 2.0) as f64;
    let f = 2f64.powf(pitch.clamp(-12.0, 12.0) as f64 / 12.0);
    if (speed - 1.0).abs() < 0.005 && (f - 1.0).abs() < 0.001 {
        return None;
    }
    let mut v = Vec::new();
    if (f - 1.0).abs() >= 0.001 {
        v.push(format!("asetrate={:.0}", rate as f64 * f));
        v.push(format!("aresample={rate}"));
    }
    let tempo = speed / f;
    if (tempo - 1.0).abs() >= 0.001 {
        v.push(format!("atempo={tempo:.4}"));
    }
    Some(v.join(","))
}

/// Join `parts` (WAVs from one engine) and apply speed / pitch into `out` with one ffmpeg pass; a single
/// part with nothing to change is just moved.
fn finish_wav(parts: &[PathBuf], speed: f32, pitch: f32, rate: u32, out: &Path, prog: &Progress) -> Result<(), String> {
    let filter = speed_pitch_filter(speed, pitch, rate);
    if parts.len() == 1 && filter.is_none() {
        let _ = std::fs::remove_file(out);
        return std::fs::rename(&parts[0], out)
            .or_else(|_| std::fs::copy(&parts[0], out).map(|_| ()))
            .map_err(|e| e.to_string());
    }
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let mut c = ffpipe::command(&ffmpeg);
    c.args(["-v", "error", "-y"]);
    for p in parts {
        c.arg("-i").arg(p);
    }
    let mut graph: String = (0..parts.len()).map(|i| format!("[{i}:a]")).collect();
    graph.push_str(&format!("concat=n={}:v=0:a=1", parts.len()));
    if let Some(f) = filter {
        graph.push(',');
        graph.push_str(&f);
    }
    c.args(["-filter_complex", &graph, "-c:a", "pcm_s16le"]).arg(out);
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    export::wait_ffmpeg(&mut child, tail, prog)
}

/// `%APPDATA%\SimpleEditor\voices`: the reference clips of cloned voices (`<name>.wav`, 24 kHz mono).
pub fn voices_dir() -> PathBuf {
    crate::settings::Settings::dir().join("voices")
}

/// The cloned voices, by name.
pub fn cloned_voices() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(voices_dir())
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

/// A file-safe voice name ("My voice" stays, "a/b:c" -> "a b c").
pub fn voice_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        "Voice".into()
    } else {
        s.chars().take(60).collect()
    }
}

/// Copy `src` (any audio or video file, from `start` seconds) into the voices folder as `name`: at most
/// 15 s, 24 kHz mono 16-bit, leading silence trimmed. Blocking (one short ffmpeg run): call from a job.
pub fn save_clone_ref(src: &Path, start: f64, name: &str) -> Result<PathBuf, String> {
    let dir = voices_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let out = dir.join(format!("{}.wav", voice_file_name(name)));
    to_reference(src, start, &out)?;
    let secs = std::fs::metadata(&out).map(|m| m.len().saturating_sub(44) as f64 / 48000.0).unwrap_or(0.0);
    if secs < 3.0 {
        let _ = std::fs::remove_file(&out);
        return Err(format!("the clip has {secs:.1} s of sound; a voice needs 5 to 15 s"));
    }
    Ok(out)
}

/// Remove a cloned voice's reference.
pub fn remove_clone(name: &str) -> Result<(), String> {
    let p = voices_dir().join(format!("{}.wav", voice_file_name(name)));
    std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// ffmpeg: `src` from `start`, 15 s at most, as a 24 kHz mono 16-bit WAV (the runner's reference format).
fn to_reference(src: &Path, start: f64, out: &Path) -> Result<(), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let o = ffpipe::command(&ffmpeg)
        .args(["-v", "error", "-y", "-ss", &format!("{:.3}", start.max(0.0)), "-t", "15", "-i"])
        .arg(src)
        .args(["-vn", "-af", "silenceremove=start_periods=1:start_threshold=-50dB", "-ac", "1", "-ar", "24000"])
        .args(["-c:a", "pcm_s16le"])
        .arg(out)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if o.status.success() && out.is_file() {
        Ok(())
    } else {
        Err(format!("could not read the clip: {}", String::from_utf8_lossy(&o.stderr).trim()))
    }
}

/// Synthesize `speech` into the WAV at `out` on a worker thread. An engine whose add-on is missing falls
/// back to Windows (`Engine::resolve`; the note is the job's final status). The same worker probes the
/// finished WAV and leaves the `Asset` in the returned holder, so the UI-thread completion never spawns
/// ffprobe (ws:job-completion-hitches). A WAV already at `out` (the cache) is reused.
pub fn speak_to_wav(speech: &Speech, out: &Path, backend: Backend) -> (Arc<Progress>, ProbedWav) {
    let (speech, out) = (speech.clone(), out.to_path_buf());
    let probed: ProbedWav = Arc::default();
    let holder = probed.clone();
    let prog = export::spawn_job("tts", move |prog| {
        if speech.text.trim().is_empty() {
            return Err("nothing to say".into());
        }
        let dir = out.parent().ok_or("no output folder")?.to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let (engine, note) = speech.engine.resolve();
        if std::fs::metadata(&out).map(|m| m.len() > 44).unwrap_or(false) {
            prog.set(0.9, "Reusing the cached speech…");
        } else {
            let tmp = dir.join(format!(".{}.parts", out.file_stem().unwrap_or_default().to_string_lossy()));
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            let r = synth(&speech, engine, &tmp, &out, prog);
            let _ = std::fs::remove_dir_all(&tmp);
            if let Err(e) = r {
                let _ = std::fs::remove_file(&out);
                return Err(e);
            }
        }
        // a WAV header alone is 44 bytes: anything that small is a voice that failed silently
        if std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) <= 44 {
            let _ = std::fs::remove_file(&out);
            return Err("no speech was written".into());
        }
        prog.set(0.95, "Probing…");
        let asset = media::probe(&out.to_string_lossy(), backend);
        *holder.lock().unwrap_or_else(|e| e.into_inner()) = Some(asset);
        prog.set(1.0, note.unwrap_or_else(|| "Done".into()));
        Ok(())
    });
    (prog, probed)
}

/// The WAV's sample rate from its header (SAPI writes 22.05 kHz, the add-ons 24 kHz).
fn wav_rate(p: &Path) -> u32 {
    use std::io::Read;
    let mut h = [0u8; 28];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut h)).map(|_| u32::from_le_bytes([h[24], h[25], h[26], h[27]])).unwrap_or(24000)
}

fn synth(speech: &Speech, engine: Engine, tmp: &Path, out: &Path, prog: &Progress) -> Result<(), String> {
    let mut speed = speech.speed;
    let parts = match engine {
        Engine::Windows => {
            prog.set(0.1, "Speaking…");
            let wav = tmp.join("sapi.wav");
            let mut child = command(&speech.text, speech.voice.as_deref(), &wav)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("powershell: {e}"))?;
            let tail = export::stderr_tail(&mut child);
            export::wait_ffmpeg(&mut child, tail, prog)?;
            if std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0) <= 44 {
                return Err("no speech was written (is a SAPI voice installed?)".into());
            }
            vec![wav]
        }
        Engine::Kokoro => {
            let voice = speech
                .voice
                .as_deref()
                .filter(|v| tts_addon::KOKORO_VOICES.contains(v))
                .unwrap_or(tts_addon::KOKORO_DEFAULT);
            let parts = tts_addon::kokoro(&speech.text, voice, speed, tmp, prog)?;
            speed = 1.0; // Kokoro paced it itself
            parts
        }
        Engine::Chatterbox => {
            let named = speech.voice.as_deref().filter(|v| !v.is_empty()).map(|v| voices_dir().join(format!("{v}.wav")));
            let src = match (&speech.clone_ref, named) {
                (Some(r), _) => r.clone(),
                (None, Some(p)) if p.is_file() => p,
                (None, Some(p)) => return Err(format!("no cloned voice at {}", p.display())),
                _ => tts_addon::Addon::Chatterbox.dir().join("models").join("default_voice.wav"),
            };
            if !src.is_file() {
                return Err(format!("no voice reference at {}", src.display()));
            }
            let reference = tmp.join("reference.wav");
            to_reference(&src, 0.0, &reference)?;
            let wav = tmp.join("chatterbox.wav");
            tts_addon::chatterbox(&speech.text, &reference, speech.intensity, &wav, prog)?;
            vec![wav]
        }
    };
    prog.set(0.9, "Finishing…");
    let rate = wav_rate(&parts[0]);
    finish_wav(&parts, speed, speech.pitch, rate, out, prog)
}

/// One row of a voice list: what to store (`id`) and what to show.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Voice {
    pub id: String,
    pub label: String,
}

/// The voices `engine` offers. Windows lists SAPI (about a second of powershell the first time: call it
/// off the UI thread); Kokoro its English voices; Chatterbox its own default voice plus the cloned ones.
pub fn voice_list(engine: Engine) -> Vec<Voice> {
    match engine {
        Engine::Windows => voices().iter().map(|v| Voice { id: v.clone(), label: v.clone() }).collect(),
        Engine::Kokoro => tts_addon::KOKORO_VOICES
            .iter()
            .map(|v| Voice { id: v.to_string(), label: tts_addon::kokoro_label(v) })
            .collect(),
        Engine::Chatterbox => std::iter::once(Voice { id: String::new(), label: "Default".into() })
            .chain(cloned_voices().into_iter().map(|v| Voice { id: v.clone(), label: v }))
            .collect(),
    }
}

/// Installed SAPI voice names, queried once per process (about a second of powershell) - only ever
/// called when the Speech panel is first opened, never at startup. Empty when powershell or
/// System.Speech is unavailable; the combo then offers just "(default)".
pub fn voices() -> &'static [String] {
    static V: OnceLock<Vec<String>> = OnceLock::new();
    V.get_or_init(|| {
        let out = ffpipe::command(Path::new("powershell.exe"))
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(
                "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer)\
                 .GetInstalledVoices() | ForEach-Object { $_.VoiceInfo.Name }",
            )
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
            _ => Vec::new(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_speak_builds_expected_powershell_command() {
        let out = Path::new("C:\\out\\it's here.wav");
        let s = script("Hello, it's me", Some("Microsoft Zira Desktop"), out);
        assert!(s.contains("Add-Type -AssemblyName System.Speech"), "{s}");
        assert!(s.contains("SetOutputToWaveFile('C:\\out\\it''s here.wav')"), "path quoted and escaped: {s}");
        assert!(s.contains("$s.SelectVoice('Microsoft Zira Desktop')"), "{s}");
        assert!(s.contains("$s.Speak('Hello, it''s me')"), "the apostrophe is doubled, not a string break: {s}");
        // no voice (or a blank one): the system default, no SelectVoice call
        assert!(!script("x", None, out).contains("SelectVoice"));
        assert!(!script("x", Some("  "), out).contains("SelectVoice"));
        // the command line itself, without spawning anything
        let c = command("x", None, out);
        assert!(c.get_program().to_string_lossy().contains("powershell"));
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.iter().any(|a| a == "-NonInteractive"));
        assert_eq!(args.iter().position(|a| a == "-Command").map(|i| &args[i + 1]), Some(&script("x", None, out)));
    }

    // ---- ws:addon-voice ----
    #[test]
    fn sapi_fallback_when_no_addon_installed() {
        assert_eq!(Engine::Windows.resolve(), (Engine::Windows, None));
        for e in [Engine::Kokoro, Engine::Chatterbox] {
            if e.available() {
                continue; // installed on this machine: the ignored real runs cover it
            }
            let (got, note) = e.resolve();
            assert_eq!(got, Engine::Windows, "{e:?} without its add-on speaks with Windows");
            assert!(note.is_some_and(|n| n.contains(e.label())), "and says so");
            // the cache key follows the engine that really speaks
            let mut s = Speech::new("hi");
            s.engine = e;
            assert!(s.cache_name().starts_with("windows-"), "{}", s.cache_name());
        }
        assert_eq!(Engine::parse("SAPI"), Some(Engine::Windows));
        assert_eq!(Engine::parse(" Kokoro "), Some(Engine::Kokoro));
        assert_eq!(Engine::parse("xtts"), None);
    }

    #[test]
    fn voice_list_round_trip() {
        // Kokoro: every listed id goes back to the speaker id it was listed at
        let k = voice_list(Engine::Kokoro);
        assert_eq!(k.len(), tts_addon::KOKORO_VOICES.len());
        for (i, v) in k.iter().enumerate() {
            let c = tts_addon::kokoro_command(Path::new("k"), "x", &v.id, 1.0, None, Path::new("o.wav"));
            assert!(c.get_args().any(|a| a.to_string_lossy() == format!("--sid={i}")), "{}", v.id);
        }
        // Chatterbox: a saved reference shows up by name, and goes away with it
        let name = format!("zz se test {}", std::process::id());
        let dir = voices_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{}.wav", voice_file_name(&name)));
        std::fs::write(&file, [0u8; 64]).unwrap();
        let cb = voice_list(Engine::Chatterbox);
        assert_eq!(cb[0], Voice { id: String::new(), label: "Default".into() });
        assert!(cb.iter().any(|v| v.id == name), "{cb:?}");
        remove_clone(&name).unwrap();
        assert!(!voice_list(Engine::Chatterbox).iter().any(|v| v.id == name));
        assert_eq!(voice_file_name("a/b:c "), "a b c");
        assert_eq!(voice_file_name("///"), "Voice");
        // a different voice is a different cached file; the same request the same one
        let mut a = Speech::new("Hello");
        a.engine = Engine::Windows;
        let mut b = a.clone();
        b.voice = Some("Microsoft Zira Desktop".into());
        assert_ne!(a.cache_name(), b.cache_name());
        assert_eq!(a.cache_name(), a.clone().cache_name());
    }

    #[test]
    fn speed_and_pitch_become_one_filter() {
        assert_eq!(speed_pitch_filter(1.0, 0.0, 24000), None);
        assert_eq!(speed_pitch_filter(1.5, 0.0, 24000).as_deref(), Some("atempo=1.5000"));
        // +12 semitones doubles the rate; the tempo is put back
        assert_eq!(speed_pitch_filter(1.0, 12.0, 24000).as_deref(), Some("asetrate=48000,aresample=24000,atempo=0.5000"));
        assert_eq!(speed_pitch_filter(9.0, 0.0, 22050).as_deref(), Some("atempo=2.0000"), "clamped");
    }

    /// Real runs (ignored): one sentence through each installed engine into `SE_TTS_SCRATCH` (or the temp
    /// folder), never played. Checks the length and loudness and prints the real-time factor.
    fn real_run(engine: Engine) {
        if !engine.available() {
            eprintln!("{engine:?} is not installed; skipped");
            return;
        }
        let dir = std::env::var_os("SE_TTS_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(format!("se-tts-{}", engine.id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Speech::new("The quick brown fox jumps over the lazy dog, then naps in the sun.");
        s.engine = engine;
        let out = dir.join("out.wav");
        let t = std::time::Instant::now();
        synth(&s, engine, &dir, &out, &Progress::new()).unwrap();
        let took = t.elapsed().as_secs_f64();
        let secs = std::fs::metadata(&out).unwrap().len().saturating_sub(44) as f64 / (wav_rate(&out) as f64 * 2.0);
        let o = ffpipe::command(&ffpipe::ffmpeg_exe().unwrap())
            .args(["-hide_banner", "-i"])
            .arg(&out)
            .args(["-af", "volumedetect", "-f", "null", "-"])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&o.stderr);
        let mean: f64 = log.split("mean_volume:").nth(1).and_then(|r| r.trim().split(' ').next()).and_then(|v| v.parse().ok()).unwrap();
        eprintln!("{engine:?}: {secs:.2} s of speech in {took:.1} s (real-time factor {:.2}), mean {mean} dB -> {}", took / secs, out.display());
        assert!((2.0..10.0).contains(&secs), "{secs} s");
        assert!(mean > -35.0, "too quiet: {mean} dB");
    }

    #[test]
    #[ignore]
    fn real_run_windows() {
        real_run(Engine::Windows);
    }

    #[test]
    #[ignore]
    fn real_run_kokoro() {
        real_run(Engine::Kokoro);
    }

    #[test]
    #[ignore]
    fn real_run_chatterbox() {
        real_run(Engine::Chatterbox);
    }
}
