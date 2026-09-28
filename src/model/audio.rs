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
}

impl FilterKind {
    pub const ALL: [FilterKind; 13] = [
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
        }
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
        assert_eq!(FilterKind::ALL.len(), 13);
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
}
