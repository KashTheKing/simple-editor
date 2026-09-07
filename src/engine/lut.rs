//! `.cube` 3D LUT (Adobe/Iridas format) parsing + a path-keyed process-wide cache, shared by the CPU
//! trilinear sampler (`engine::effects`'s `Lut` arm) and the GPU `TEXTURE_3D` upload (`engine::gpu`).
//! std-only: no dependency, matches goals.md's dependency budget.
//!
//! File shape read: `LUT_3D_SIZE N` then N^3 "r g b" rows, red changing fastest (the format's own
//! convention) — so the rows are stored in the file's own order and `sample_trilinear`'s index math
//! (`r + g*N + b*N*N`) matches it without any reshuffling. `TITLE`/`DOMAIN_MIN`/`DOMAIN_MAX`/comment
//! lines are ignored (ponytail: DOMAIN_MIN/MAX support — a non-default input domain — is rare in
//! practice; add remapping here if a real-world .cube ever needs it).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A parsed 3D LUT: `size`^3 RGB triples, row-major with red fastest (`data[r + g*size + b*size*size]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    pub size: u32,
    pub data: Vec<[f32; 3]>,
}

/// Parse a `.cube` file's text. Errors on a missing/mismatched `LUT_3D_SIZE` (a 1D LUT or a truncated
/// file); everything else (TITLE, DOMAIN_MIN/MAX, `#` comments, blank lines) is skipped.
pub fn parse_cube(src: &str) -> Result<Lut3d, String> {
    let mut size: Option<u32> = None;
    let mut data = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
            size = rest.trim().parse().ok();
            continue;
        }
        if line.starts_with("TITLE")
            || line.starts_with("DOMAIN_MIN")
            || line.starts_with("DOMAIN_MAX")
            || line.starts_with("LUT_1D_SIZE")
        {
            continue; // ponytail: domain remap unsupported, see the module doc
        }
        let vals: Vec<f32> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if vals.len() == 3 {
            data.push([vals[0], vals[1], vals[2]]);
        }
    }
    let size = size.ok_or("missing LUT_3D_SIZE")?;
    if size < 2 {
        return Err(format!("LUT_3D_SIZE {size} is too small (need >= 2)"));
    }
    let expected = (size as usize).pow(3);
    if data.len() != expected {
        return Err(format!("expected {expected} data rows for LUT_3D_SIZE {size}, found {}", data.len()));
    }
    Ok(Lut3d { size, data })
}

fn idx(n: usize, r: usize, g: usize, b: usize) -> usize {
    r.min(n - 1) + g.min(n - 1) * n + b.min(n - 1) * n * n
}

/// Trilinear-sample `lut` at `rgb` (each 0..1, clamped). Identity (returns `rgb` unchanged) for a
/// degenerate LUT (fewer than 2 samples per axis).
pub fn sample_trilinear(lut: &Lut3d, rgb: [f32; 3]) -> [f32; 3] {
    let n = lut.size as usize;
    if n < 2 || lut.data.len() < n * n * n {
        return rgb;
    }
    let scaled: [f32; 3] = std::array::from_fn(|i| rgb[i].clamp(0.0, 1.0) * (n as f32 - 1.0));
    let i0: [usize; 3] = std::array::from_fn(|i| (scaled[i].floor() as usize).min(n - 2));
    let f: [f32; 3] = std::array::from_fn(|i| scaled[i] - i0[i] as f32);
    let mut out = [0.0f32; 3];
    for dz in 0..2usize {
        for dy in 0..2usize {
            for dx in 0..2usize {
                let w = (if dx == 0 { 1.0 - f[0] } else { f[0] })
                    * (if dy == 0 { 1.0 - f[1] } else { f[1] })
                    * (if dz == 0 { 1.0 - f[2] } else { f[2] });
                let s = lut.data[idx(n, i0[0] + dx, i0[1] + dy, i0[2] + dz)];
                for c in 0..3 {
                    out[c] += s[c] * w;
                }
            }
        }
    }
    out
}

static CACHE: OnceLock<Mutex<HashMap<String, Arc<Lut3d>>>> = OnceLock::new();

/// Read + parse `path` once, cached by path for the life of the process.
/// ponytail: no mtime invalidation — re-run `clip.add_lut` (or restart) to pick up an edited `.cube`.
pub fn load(path: &str) -> Result<Arc<Lut3d>, String> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(l) = cache.lock().unwrap().get(path) {
        return Ok(l.clone());
    }
    let src = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let lut = Arc::new(parse_cube(&src)?);
    cache.lock().unwrap().insert(path.to_string(), lut.clone());
    Ok(lut)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LUT_3D_SIZE 2, corners mapped to themselves — trilinear interpolation of a truly linear function
    /// reproduces it exactly at any point.
    const IDENTITY_2: &str = "LUT_3D_SIZE 2\n\
        0.0 0.0 0.0\n1.0 0.0 0.0\n0.0 1.0 0.0\n1.0 1.0 0.0\n\
        0.0 0.0 1.0\n1.0 0.0 1.0\n0.0 1.0 1.0\n1.0 1.0 1.0\n";

    #[test]
    fn parse_cube_and_sample_identity() {
        let lut = parse_cube(IDENTITY_2).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8);
        for rgb in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.3, 0.7, 0.5], [0.9, 0.1, 0.42], [0.5, 0.5, 0.5]] {
            let out = sample_trilinear(&lut, rgb);
            for c in 0..3 {
                assert!((out[c] - rgb[c]).abs() < 1e-4, "{rgb:?} -> {out:?}");
            }
        }
        // comments / TITLE / DOMAIN lines are ignored, not counted as data rows
        let with_extras = format!("# a comment\nTITLE \"identity\"\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n{IDENTITY_2}");
        let lut2 = parse_cube(&with_extras).unwrap();
        assert_eq!(lut2, lut);
        // a mismatched row count is an error, not a silent truncation
        assert!(parse_cube("LUT_3D_SIZE 2\n0.0 0.0 0.0\n").is_err());
        assert!(parse_cube("0.0 0.0 0.0\n").is_err(), "missing LUT_3D_SIZE");
    }

    #[test]
    fn lut_cache_hits_on_second_load() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("se_lut_test_{}.cube", std::process::id()));
        std::fs::write(&path, IDENTITY_2).unwrap();
        let p = path.to_string_lossy().into_owned();
        let a = load(&p).unwrap();
        let b = load(&p).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "second load must hit the cache, not re-parse");
        std::fs::remove_file(&path).ok();
    }
}
