use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------- audio buses ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum FilterKind {
    /// 5-band parametric EQ (low shelf, three peaks, high shelf).
    Eq,
    HighPass,
    LowPass,
    Reverb,
    Echo,
    Distortion,
    Compressor,
    NoiseGate,
    /// Adds noise (white / pink / a pure tone).
    Noise,
    Gain,
    // ---- ws:audio-dsp-automation ----
    /// Mains-hum notches at the base frequency and its first two harmonics.
    DeHum,
    /// Brickwall peak limiter with a short lookahead.
    Limiter,
    /// Sibilance compressor keyed from a high-passed sidechain.
    DeEsser,
    // ---- ws:gallery-audio ----
    /// Pitch shift in semitones, length kept (two crossfaded delay-line taps).
    Pitch,
    // ---- ws:parity-audio ----
    /// RBJ band-pass (constant 0 dB peak gain) around a centre frequency.
    BandPass,
    /// RBJ notch: removes one narrow band.
    Notch,
    /// Downward expander: quiet parts below the threshold get quieter by the ratio, down to a range.
    Expander,
    /// Removes short clicks: an outlier jump in the waveform is bridged by a straight line.
    Declick,
    /// Separate gains for the middle (L+R) and the sides (L-R).
    MidSide,
    /// Narrows (0) or widens (up to 2) the stereo image.
    StereoWidth,
    /// Channel copy / swap / polarity invert (`CHANNEL_MODES`).
    Channels,
    /// Folds stereo to mono by an amount.
    Downmix,
    /// 15-band graphic EQ on the ISO 2/3-octave centres (`EQ15_HZ`).
    Eq15,
    /// Two-pass loudness normalise: an analysis pass measures the clip, the filter applies target - measured.
    Normalize,
}

impl FilterKind {
    pub const ALL: [FilterKind; 24] = [
        FilterKind::Eq,
        FilterKind::HighPass,
        FilterKind::LowPass,
        FilterKind::Reverb,
        FilterKind::Echo,
        FilterKind::Distortion,
        FilterKind::Compressor,
        FilterKind::NoiseGate,
        FilterKind::Noise,
        FilterKind::Gain,
        // ---- ws:audio-dsp-automation ----
        FilterKind::DeHum,
        FilterKind::Limiter,
        FilterKind::DeEsser,
        FilterKind::Pitch,
        // ---- ws:parity-audio ----
        FilterKind::BandPass,
        FilterKind::Notch,
        FilterKind::Expander,
        FilterKind::Declick,
        FilterKind::MidSide,
        FilterKind::StereoWidth,
        FilterKind::Channels,
        FilterKind::Downmix,
        FilterKind::Eq15,
        FilterKind::Normalize,
    ];
    pub fn name(self) -> &'static str {
        match self {
            FilterKind::Eq => "EQ (5-band)",
            FilterKind::HighPass => "High-pass",
            FilterKind::LowPass => "Low-pass",
            FilterKind::Reverb => "Reverb",
            FilterKind::Echo => "Echo / Delay",
            FilterKind::Distortion => "Distortion",
            FilterKind::Compressor => "Compressor",
            FilterKind::NoiseGate => "Noise gate",
            FilterKind::Noise => "Noise",
            FilterKind::Gain => "Gain",
            // ---- ws:audio-dsp-automation ----
            FilterKind::DeHum => "De-hum",
            FilterKind::Limiter => "Limiter",
            FilterKind::DeEsser => "De-esser",
            FilterKind::Pitch => "Pitch",
            // ---- ws:parity-audio ----
            FilterKind::BandPass => "Band-pass",
            FilterKind::Notch => "Notch",
            FilterKind::Expander => "Expander",
            FilterKind::Declick => "De-click",
            FilterKind::MidSide => "Mid-side",
            FilterKind::StereoWidth => "Stereo width",
            FilterKind::Channels => "Channels",
            FilterKind::Downmix => "Downmix",
            FilterKind::Eq15 => "EQ (15-band)",
            FilterKind::Normalize => "Normalize (two-pass)",
        }
    }
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            FilterKind::Eq => F_EQ,
            FilterKind::HighPass | FilterKind::LowPass => F_PASS,
            FilterKind::Reverb => F_REVERB,
            FilterKind::Echo => F_ECHO,
            FilterKind::Distortion => F_DIST,
            FilterKind::Compressor => F_COMP,
            FilterKind::NoiseGate => F_GATE,
            FilterKind::Noise => F_NOISE,
            FilterKind::Gain => F_GAIN,
            // ---- ws:audio-dsp-automation ----
            FilterKind::DeHum => F_DEHUM,
            FilterKind::Limiter => F_LIMITER,
            FilterKind::DeEsser => F_DEESSER,
            FilterKind::Pitch => F_PITCH,
            // ---- ws:parity-audio ----
            FilterKind::BandPass => F_BANDPASS,
            FilterKind::Notch => F_NOTCH,
            FilterKind::Expander => F_EXPANDER,
            FilterKind::Declick => F_DECLICK,
            FilterKind::MidSide => F_MIDSIDE,
            FilterKind::StereoWidth => F_WIDTH,
            FilterKind::Channels => F_CHANNELS,
            FilterKind::Downmix => F_DOWNMIX,
            FilterKind::Eq15 => F_EQ15,
            FilterKind::Normalize => F_NORMALIZE,
        }
    }
    // ---- ws:parity-audio ----
    /// A kind by its `name()` or variant name, ignoring case ("Band-pass", "bandpass", "Eq15"...).
    pub fn parse(s: &str) -> Option<FilterKind> {
        let squash = |x: &str| x.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let want = squash(s);
        Self::ALL.into_iter().find(|k| squash(k.name()) == want || squash(&format!("{k:?}")) == want)
    }
    /// The labels of a parameter that picks one of a few options (a dropdown, not a number), by param name.
    pub fn choices(self, param: &str) -> Option<&'static [&'static str]> {
        match (self, param) {
            (FilterKind::Noise, "Type") => Some(&["White", "Pink", "Tone"]),
            (FilterKind::Channels, "Mode") => Some(&CHANNEL_MODES),
            _ => None,
        }
    }
}

// ---- ws:parity-audio ----
/// The Channels filter's modes, in `Mode` parameter order.
pub const CHANNEL_MODES: [&str; 6] =
    ["Copy left to right", "Copy right to left", "Swap", "Invert left", "Invert right", "Invert both"];
/// The 15-band EQ's centre frequencies (ISO 2/3-octave), one gain parameter each.
pub const EQ15_HZ: [f32; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0, 10000.0, 16000.0,
];
/// "Measured LUFS" at this value means "not analysed yet": the Normalize filter passes the audio unchanged.
pub const NORMALIZE_UNMEASURED: f64 = 0.0;

impl AudioFilter {
    /// A filter of `kind` with `{param name: value}` overrides (names ignore case). Unknown names and
    /// non-numbers are errors, so a typo never silently does nothing.
    pub fn with_params(kind: FilterKind, params: Option<&serde_json::Map<String, serde_json::Value>>) -> Result<Self, String> {
        let mut f = AudioFilter::new(kind);
        for (name, v) in params.into_iter().flatten() {
            let i = kind
                .params()
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("unknown param '{name}' for {}", kind.name()))?;
            f.params[i].value = v.as_f64().ok_or("param values must be numbers")?;
        }
        Ok(f)
    }
}

// ---- ws:audio-dsp-automation ----
// `AudioRole`'s enum body lives in model/clip.rs (registries-schema-hooks is its sole definer); this
// inherent impl only adds the list/name helpers the Role combo and the `audio.role` tool need.
impl AudioRole {
    pub const ALL: [AudioRole; 5] =
        [AudioRole::Unset, AudioRole::Dialogue, AudioRole::Music, AudioRole::Sfx, AudioRole::Ambience];
    pub fn name(self) -> &'static str {
        match self {
            AudioRole::Unset => "Unset",
            AudioRole::Dialogue => "Dialogue",
            AudioRole::Music => "Music",
            AudioRole::Sfx => "SFX",
            AudioRole::Ambience => "Ambience",
        }
    }
    /// "Dialogue" / "sfx" / "Music" … (case-insensitive, matches the variant name or `name()`).
    pub fn parse(s: &str) -> Option<AudioRole> {
        Self::ALL.into_iter().find(|r| r.name().eq_ignore_ascii_case(s) || format!("{r:?}").eq_ignore_ascii_case(s))
    }
}

/// Five bands, but the first seven slots are frozen in the order the 3-band EQ used so old projects
/// keep their meaning; `mixer_fx::EQ_BANDS` groups them back into bands for the DSP and the UI.
const F_EQ: &[ParamSpec] = &[
    ps("Low gain dB", 0.0, -24.0, 24.0),
    ps("Low freq", 120.0, 20.0, 1000.0),
    ps("Mid gain dB", 0.0, -24.0, 24.0),
    ps("Mid freq", 1000.0, 100.0, 8000.0),
    ps("Mid Q", 1.0, 0.1, 10.0),
    ps("High gain dB", 0.0, -24.0, 24.0),
    ps("High freq", 6000.0, 1000.0, 20000.0),
    ps("Low Q", 0.707, 0.1, 10.0),
    ps("High Q", 0.707, 0.1, 10.0),
    ps("Low-mid gain dB", 0.0, -24.0, 24.0),
    ps("Low-mid freq", 400.0, 40.0, 4000.0),
    ps("Low-mid Q", 1.0, 0.1, 10.0),
    ps("High-mid gain dB", 0.0, -24.0, 24.0),
    ps("High-mid freq", 3000.0, 400.0, 16000.0),
    ps("High-mid Q", 1.0, 0.1, 10.0),
];
const F_PASS: &[ParamSpec] = &[ps("Frequency", 200.0, 20.0, 20000.0), ps("Resonance", 0.7, 0.1, 10.0)];
const F_REVERB: &[ParamSpec] = &[
    ps("Room size", 0.5, 0.0, 1.0),
    ps("Damping", 0.5, 0.0, 1.0),
    ps("Width", 1.0, 0.0, 1.0),
    ps("Mix", 0.25, 0.0, 1.0),
    ps("Pre-delay ms", 20.0, 0.0, 200.0),
];
const F_ECHO: &[ParamSpec] = &[
    ps("Delay ms", 350.0, 1.0, 2000.0),
    ps("Feedback", 0.35, 0.0, 0.95),
    ps("Mix", 0.3, 0.0, 1.0),
    ps("Ping-pong", 0.0, 0.0, 1.0),
];
const F_DIST: &[ParamSpec] = &[ps("Drive", 4.0, 1.0, 50.0), ps("Tone", 0.5, 0.0, 1.0), ps("Mix", 1.0, 0.0, 1.0)];
const F_COMP: &[ParamSpec] = &[
    ps("Threshold dB", -18.0, -60.0, 0.0),
    ps("Ratio", 4.0, 1.0, 20.0),
    ps("Attack ms", 10.0, 0.1, 200.0),
    ps("Release ms", 120.0, 5.0, 2000.0),
    ps("Makeup dB", 0.0, -12.0, 24.0),
];
const F_GATE: &[ParamSpec] =
    &[ps("Threshold dB", -45.0, -80.0, 0.0), ps("Attack ms", 2.0, 0.1, 100.0), ps("Release ms", 120.0, 5.0, 2000.0)];
const F_NOISE: &[ParamSpec] =
    &[ps("Level dB", -40.0, -80.0, 0.0), ps("Type", 0.0, 0.0, 2.0), ps("Tone Hz", 1000.0, 20.0, 18000.0)];
const F_GAIN: &[ParamSpec] = &[ps("Gain dB", 0.0, -60.0, 24.0)];
// ---- ws:audio-dsp-automation ----
const F_DEHUM: &[ParamSpec] = &[ps("Base Hz", 60.0, 40.0, 70.0), ps("Depth dB", 40.0, 6.0, 60.0)];
const F_LIMITER: &[ParamSpec] =
    &[ps("Ceiling dB", -1.0, -24.0, 0.0), ps("Lookahead ms", 5.0, 0.0, 20.0), ps("Release ms", 100.0, 5.0, 1000.0)];
const F_DEESSER: &[ParamSpec] =
    &[ps("Freq Hz", 5000.0, 2000.0, 12000.0), ps("Threshold dB", -24.0, -60.0, 0.0), ps("Ratio", 4.0, 1.0, 20.0)];

const F_PITCH: &[ParamSpec] = &[ps("Semitones", 0.0, -12.0, 12.0), ps("Mix", 1.0, 0.0, 1.0)];
// ---- ws:parity-audio ----
const F_BANDPASS: &[ParamSpec] = &[ps("Frequency", 1000.0, 20.0, 20000.0), ps("Q", 1.0, 0.1, 20.0)];
const F_NOTCH: &[ParamSpec] = &[ps("Frequency", 1000.0, 20.0, 20000.0), ps("Q", 4.0, 0.1, 20.0)];
const F_EXPANDER: &[ParamSpec] = &[
    ps("Threshold dB", -40.0, -80.0, 0.0),
    ps("Ratio", 2.0, 1.0, 10.0),
    ps("Attack ms", 2.0, 0.1, 100.0),
    ps("Release ms", 150.0, 5.0, 2000.0),
    ps("Range dB", 40.0, 0.0, 80.0),
];
const F_DECLICK: &[ParamSpec] = &[ps("Sensitivity", 0.5, 0.0, 1.0), ps("Length ms", 1.0, 0.1, 5.0)];
const F_MIDSIDE: &[ParamSpec] = &[ps("Mid gain dB", 0.0, -24.0, 24.0), ps("Side gain dB", 0.0, -24.0, 24.0)];
const F_WIDTH: &[ParamSpec] = &[ps("Width", 1.0, 0.0, 2.0)];
const F_CHANNELS: &[ParamSpec] = &[ps("Mode", 2.0, 0.0, 5.0)];
const F_DOWNMIX: &[ParamSpec] = &[ps("Amount", 1.0, 0.0, 1.0)];
const F_EQ15: &[ParamSpec] = &[
    ps("25 Hz", 0.0, -12.0, 12.0),
    ps("40 Hz", 0.0, -12.0, 12.0),
    ps("63 Hz", 0.0, -12.0, 12.0),
    ps("100 Hz", 0.0, -12.0, 12.0),
    ps("160 Hz", 0.0, -12.0, 12.0),
    ps("250 Hz", 0.0, -12.0, 12.0),
    ps("400 Hz", 0.0, -12.0, 12.0),
    ps("630 Hz", 0.0, -12.0, 12.0),
    ps("1 kHz", 0.0, -12.0, 12.0),
    ps("1.6 kHz", 0.0, -12.0, 12.0),
    ps("2.5 kHz", 0.0, -12.0, 12.0),
    ps("4 kHz", 0.0, -12.0, 12.0),
    ps("6.3 kHz", 0.0, -12.0, 12.0),
    ps("10 kHz", 0.0, -12.0, 12.0),
    ps("16 kHz", 0.0, -12.0, 12.0),
];
const F_NORMALIZE: &[ParamSpec] = &[
    ps("Target LUFS", -16.0, -40.0, -5.0),
    ps("Measured LUFS", NORMALIZE_UNMEASURED, -70.0, 0.0),
    ps("Max gain dB", 20.0, 0.0, 40.0),
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioFilter {
    pub kind: FilterKind,
    #[serde(default = "crate::model::tru")]
    pub enabled: bool,
    #[serde(default, deserialize_with = "de_params")]
    pub params: Vec<Animated>,
}

/// Filter parameters were plain numbers before they became keyframeable; those load as constants.
fn de_params<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Animated>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrAnim {
        Num(f64),
        Anim(Animated),
    }
    Ok(Vec::<NumOrAnim>::deserialize(d)?
        .into_iter()
        .map(|p| match p {
            NumOrAnim::Num(v) => Animated::new(v),
            NumOrAnim::Anim(a) => a,
        })
        .collect())
}

impl AudioFilter {
    pub fn new(kind: FilterKind) -> Self {
        Self { kind, enabled: true, params: kind.params().iter().map(|p| Animated::new(p.default)).collect() }
    }
    /// Append the parameters a shorter (older) project is missing, at their spec defaults.
    pub fn fill_params(&mut self) {
        let specs = self.kind.params();
        for s in &specs[self.params.len().min(specs.len())..] {
            self.params.push(Animated::new(s.default));
        }
    }
    /// Parameter i at time t (spec default when missing).
    pub fn at(&self, i: usize, t: f64) -> f64 {
        self.params
            .get(i)
            .map(|a| a.at(t))
            .unwrap_or_else(|| self.kind.params().get(i).map(|p| p.default).unwrap_or(0.0))
    }
}

/// A mixer bus. The first bus is always "Main"; every other bus routes into it (or into another bus).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Bus {
    pub id: Id,
    pub name: String,
    pub gain: Animated,
    pub pan: Animated,
    pub muted: bool,
    pub solo: bool,
    /// Fold to mono after the filter chain.
    pub mono: bool,
    pub filters: Vec<AudioFilter>,
    /// Where this bus sends its output (0 = Main). Main sends nowhere.
    pub output: Id,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Bus".into(),
            gain: Animated::new(1.0),
            pan: Animated::new(0.0),
            muted: false,
            solo: false,
            mono: false,
            filters: Vec::new(),
            output: 0,
        }
    }
}

// ---- ws:audio-dsp-automation ----
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filterkind_all_covers_new_variants() {
        assert_eq!(FilterKind::ALL.len(), 24); // 14 + ws:parity-audio's 10
        for k in FilterKind::ALL {
            assert!(!k.name().is_empty(), "{k:?}");
            assert!(!k.params().is_empty(), "{k:?}");
            assert_eq!(AudioFilter::new(k).params.len(), k.params().len(), "{k:?}");
        }
        for k in [FilterKind::DeHum, FilterKind::Limiter, FilterKind::DeEsser] {
            assert!(FilterKind::ALL.contains(&k));
        }
    }

    /// `AudioRole` has exactly one definition in the crate (model/clip.rs) - this module only imports
    /// it and adds helpers; a second declaration anywhere under src/ is the duplicate-definition bug
    /// the plan's audit caught. Counts only lines that are themselves a declaration (trimmed line
    /// starts with `enum AudioRole`/`pub enum AudioRole`) rather than any substring match, so this
    /// test's own comments/assert text/search string above don't inflate the count.
    #[test]
    fn audio_role_single_definition() {
        use crate::model::clip::AudioRole as _;
        fn count(dir: &std::path::Path, hits: &mut usize) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    count(&p, hits);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    for line in std::fs::read_to_string(&p).unwrap_or_default().lines() {
                        let t = line.trim_start();
                        if t.starts_with("enum AudioRole") || t.starts_with("pub enum AudioRole") {
                            *hits += 1;
                        }
                    }
                }
            }
        }
        let mut hits = 0;
        count(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
        assert_eq!(hits, 1, "`enum AudioRole` must be defined exactly once (model/clip.rs)");
        // and the helpers here round-trip every variant
        for r in AudioRole::ALL {
            assert_eq!(AudioRole::parse(r.name()), Some(r));
            assert_eq!(AudioRole::parse(&format!("{r:?}").to_lowercase()), Some(r));
        }
        assert_eq!(AudioRole::parse("nope"), None);
    }

    // ---- ws:parity-audio ----
    #[test]
    fn filter_kind_parse_and_with_params() {
        for k in FilterKind::ALL {
            assert_eq!(FilterKind::parse(k.name()), Some(k), "{k:?}");
            assert_eq!(FilterKind::parse(&format!("{k:?}").to_lowercase()), Some(k));
        }
        assert_eq!(FilterKind::parse("band pass"), Some(FilterKind::BandPass));
        assert_eq!(FilterKind::parse("nope"), None);
        assert_eq!(F_EQ15.len(), EQ15_HZ.len());
        let m = serde_json::json!({"frequency": 440.0, "Q": 8.0});
        let f = AudioFilter::with_params(FilterKind::Notch, m.as_object()).unwrap();
        assert_eq!((f.params[0].value, f.params[1].value), (440.0, 8.0));
        assert!(AudioFilter::with_params(FilterKind::Notch, serde_json::json!({"Nope": 1}).as_object()).is_err());
        assert!(AudioFilter::with_params(FilterKind::Notch, serde_json::json!({"Q": "x"}).as_object()).is_err());
        assert_eq!(FilterKind::Channels.choices("Mode").map(|c| c.len()), Some(6));
        assert_eq!(FilterKind::Channels.params()[0].max as usize + 1, CHANNEL_MODES.len());
    }
}
