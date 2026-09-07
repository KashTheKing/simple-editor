//! ---- ws:transcript-captions ----
//! Text-to-speech through Windows' own System.Speech (SAPI) via a powershell shell-out - no crate, no
//! bundled voices: `voices()` lists whatever the OS has installed (OS-tier voices, not neural ones),
//! `speak_to_wav` writes a WAV the app then imports as a linked asset. Same `export::spawn_job` /
//! `Progress` shape as `transcribe::download_model`, so the UI shows it like any other job.

use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

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

/// Synthesize `text` into the WAV at `out` on a worker thread. `voice` = None uses the system default.
pub fn speak_to_wav(text: &str, voice: Option<&str>, out: &Path) -> Arc<Progress> {
    let (text, voice, out) = (text.to_string(), voice.map(str::to_string), out.to_path_buf());
    export::spawn_job("tts", move |prog| {
        if text.trim().is_empty() {
            return Err("nothing to say".into());
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        prog.set(0.1, "Speaking…");
        let mut child = command(&text, voice.as_deref(), &out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("powershell: {e}"))?;
        let tail = export::stderr_tail(&mut child);
        export::wait_ffmpeg(&mut child, tail, prog)?;
        // a WAV header alone is 44 bytes: anything that small is a voice that failed silently
        if std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) <= 44 {
            let _ = std::fs::remove_file(&out);
            return Err("no speech was written (is a SAPI voice installed?)".into());
        }
        prog.set(1.0, "Done");
        Ok(())
    })
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
}
