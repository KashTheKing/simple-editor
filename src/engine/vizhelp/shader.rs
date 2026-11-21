//! ---- ws:visualizer-helper ----
//! The Olympus Storm shader per song: its `const` lines (drop times, colours, spines per hit, which layers
//! draw) filled from the analysis and the knobs, and the Difference-blend invert layer for the shatter into
//! the biggest drop cut out of it (a port of `hsv_shaders.py`). The shader ships with the plugin as a
//! library effect that compiles as it is (no drop, every layer on); only its `const` lines change here.

use super::keys::Knobs;
use crate::model::SongViz;

/// The shatter build into the biggest drop, in clip seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shatter {
    pub big: f64,
    pub rift0: f64,
    /// Hits before the build starts, and in it (the shards' progress counts hits).
    pub hit0: f64,
    pub hits: f64,
}

impl Shatter {
    /// `build` seconds into the biggest drop (never before 2 s after the drop ahead of it), for a clip
    /// that starts at song second `from`. None without a drop in the clip.
    pub fn of(v: &SongViz, from: f64, len: f64, build: f64) -> Option<Shatter> {
        let big = v.big_drop()?;
        let prev = if v.drops.len() > 1 { v.drops[v.drops.len() - 2] as f64 + 2.0 } else { 2.0 };
        let rift0 = (big - build.clamp(1.0, 60.0)).max(prev).min(big - 0.5);
        if big - from <= 0.0 || big - from > len {
            return None;
        }
        let hit0 = v.hits.iter().filter(|h| (h[0] as f64) < rift0.max(from)).count() - v.hits.iter().filter(|h| (h[0] as f64) < from).count();
        let hits = v.hits.iter().filter(|h| (h[0] as f64) >= rift0 && (h[0] as f64) < big).count().max(1);
        Some(Shatter { big: big - from, rift0: (rift0 - from).max(0.0), hit0: hit0 as f64, hits: hits as f64 })
    }
}

/// `src` with `const <type> <name> = ...;` set to `value` (unchanged when the line is missing).
pub fn set_const(src: &str, name: &str, value: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    let mut done = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        let hit = !done
            && t.starts_with("const ")
            && t.split_whitespace().nth(2) == Some(name)
            && t.split_whitespace().nth(3) == Some("=");
        if hit {
            let ty = t.split_whitespace().nth(1).unwrap_or("float");
            let nl = if line.ends_with('\n') { "\n" } else { "" };
            out.push_str(&format!("const {ty} {name} = {value};{nl}"));
            done = true;
        } else {
            out.push_str(line);
        }
    }
    out
}

fn f(v: f64) -> String {
    let s = format!("{:.3}", v);
    if s.contains('.') {
        s
    } else {
        format!("{s}.0")
    }
}

fn rgb(c: [u8; 3]) -> String {
    format!("vec3({}, {}, {})", f(c[0] as f64 / 255.0), f(c[1] as f64 / 255.0), f(c[2] as f64 / 255.0))
}

/// The template with this song's shatter times (None = no shatter) and the knobs' colours, spines and
/// layers.
pub fn specialize(src: &str, k: &Knobs, sh: Option<Shatter>) -> String {
    let mut s = src.to_string();
    let (big, rift0, hit0, hits) = match sh {
        Some(x) => (x.big, x.rift0, x.hit0, x.hits),
        None => (1_000_000.0, 999_999.0, 0.0, 1.0),
    };
    for (n, v) in [
        ("BIGDROP", big),
        ("RIFT0", rift0),
        ("SUCK0", rift0 - 2.0),
        ("SUCK1", rift0 + 6.0),
        ("RIFTHIT0", hit0),
        ("RIFTHITS", hits),
        ("SPINES", k.spikes.clamp(1.0, 6.0)),
        ("SHOW_BALL", k.ball as u8 as f64),
        ("SHOW_WAVE", k.wave as u8 as f64),
        ("SHOW_BOLTS", k.bolts as u8 as f64),
        ("SHOW_BEAM", k.beam as u8 as f64),
    ] {
        s = set_const(&s, n, &f(v));
    }
    for (n, c) in [("METAL0", k.metal_pre), ("METAL1", k.metal_post), ("ELEC0", k.elec_pre), ("ELEC1", k.elec_post)] {
        s = set_const(&s, n, &rgb(c));
    }
    s
}

/// The invert layer: the visualizer shader up to the ball size, then only the shard mask (white = invert
/// what is below; the clip uses the Difference blend). Its clip starts at RIFT0, so its time is shifted.
pub fn invert_layer(visualizer: &str) -> Result<String, String> {
    let s = visualizer;
    let missing = |what: &str| format!("not an Olympus Storm shader ({what} missing)");
    let g0 = s.find("        float prog = clamp((beats - ").ok_or_else(|| missing("the shatter"))?;
    let g1 = s[g0..].find("        vec3 goldL").map(|i| g0 + i).ok_or_else(|| missing("the shatter's end"))?;
    let geom = &s[g0..g1];
    let mask = format!(
        "\n    // INVERT MASK: white where the colours below get inverted (this clip uses the Difference blend)\n    {{\n{geom}        float m = inv * step(RIFT0, u_time) * step(u_time, BIGDROP);\n        return vec4(vec3(m), m);\n    }}\n"
    );
    let anchor = "float rb = 0.75 * 0.66 * 0.4 * 0.85 * gs;";
    let j = s.find(anchor).ok_or_else(|| missing("the ball size"))? + anchor.len();
    let mut m = format!("{}{}{}", &s[..j], mask, &s[j..]);
    // the whole ball inverts once it has shattered
    let inv = "float inv = max(max(first, step(sc.z / 7.0, ballR)),";
    if !m.contains(inv) {
        return Err(missing("the shard mask"));
    }
    m = m.replacen(inv, "float inv = max(max(max(first, step(sc.z / 7.0, ballR)), step(0.28, prog) * step(cr, r0 * 1.03)),", 1);
    m = m.replace("u_time", "(u_time + RIFT0)");
    m = m.replacen("@name Olympus Storm", "@name Olympus Rift Invert", 1);
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = include_str!("../../../library/plugins/visualizer-helper/effects/olympus-storm.glsl");

    #[test]
    fn consts_are_set_and_nothing_else_moves() {
        let k = Knobs { spikes: 6.0, ball: false, metal_post: [255, 0, 0], ..Default::default() };
        let sh = Shatter { big: 52.0, rift0: 39.3, hit0: 210.0, hits: 70.0 };
        let s = specialize(SRC, &k, Some(sh));
        assert!(s.contains("const float BIGDROP = 52.000;\n"));
        assert!(s.contains("const float SUCK0 = 37.300;\n"));
        assert!(s.contains("const float SPINES = 6.000;\n"));
        assert!(s.contains("const float SHOW_BALL = 0.000;\n"));
        assert!(s.contains("const vec3 METAL1 = vec3(1.000, 0.000, 0.000);\n"));
        assert_eq!(s.lines().count(), SRC.lines().count());
        assert_eq!(specialize(SRC, &k, Some(sh)), s, "deterministic");
        // defaults reproduce the library copy's look
        let d = specialize(SRC, &Knobs::default(), None);
        assert!(d.contains("const vec3 METAL0 = vec3(0.620, 0.639, 0.678);"));
    }

    #[test]
    fn the_invert_layer_is_the_shard_mask_on_shifted_time() {
        let s = specialize(SRC, &Knobs::default(), Some(Shatter { big: 52.0, rift0: 39.3, hit0: 210.0, hits: 70.0 }));
        let m = invert_layer(&s).unwrap();
        assert!(m.contains("@name Olympus Rift Invert"));
        assert!(m.contains("INVERT MASK"));
        assert!(m.contains("step(0.28, prog) * step(cr, r0 * 1.03)"));
        assert!(!m.contains("u_time)") || m.contains("(u_time + RIFT0)"));
        assert!(!m.replace("(u_time + RIFT0)", "").contains("u_time"));
        assert!(invert_layer("vec4 effect(vec4 s, vec2 uv) { return s; }").is_err());
    }

    #[test]
    fn shatter_times_count_hits_from_the_clip_start() {
        let v = SongViz { drops: vec![20.0, 52.0], hits: (0..200).map(|i| [0.3 * i as f32, 1.0]).collect(), ..Default::default() };
        let sh = Shatter::of(&v, 10.0, 60.0, 12.7).unwrap();
        assert_eq!(sh.big, 42.0);
        assert!((sh.rift0 - 29.3).abs() < 1e-9);
        // hits in 10..39.3 s of the song
        assert_eq!(sh.hit0 as usize, (0..200).filter(|&i| (0.3 * i as f32) < 39.3 && (0.3 * i as f32) >= 10.0).count());
        assert!(Shatter::of(&v, 60.0, 10.0, 12.7).is_none());
    }
}
