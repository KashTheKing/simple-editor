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
}

impl FilterKind {
    pub const ALL: [FilterKind; 10] = [
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
        }
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
