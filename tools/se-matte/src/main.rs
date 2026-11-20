//! se-matte: Simple Editor's background-removal helper.
//!
//! Usage:
//!   se-matte --model <file.onnx> --kind modnet|birefnet --width W --height H [--ep auto|dml|cpu] [--threads N]
//!   se-matte --model <file.onnx> --kind modnet|birefnet --probe [--ep ...]
//!
//! Frame protocol (binary, no headers): the app writes frames of exactly W*H*3 bytes (RGB24, top-down) to
//! stdin; for every frame the helper writes exactly W*H bytes (an 8-bit matte, 255 = subject) to stdout,
//! in order. EOF on stdin ends the run (exit 0). The first stderr line is `ep dml` or `ep cpu` (which
//! execution provider the session got); a failure is one `error: ...` line and exit code 1.
//! `--probe` loads the model, runs one frame of grey and prints `{"ep":"dml","ms":12.3}` on stdout.
//!
//! onnxruntime.dll is loaded from beside this exe. DirectML is the GPU path (any DirectX 12 GPU; the
//! DirectML.dll that Windows 10 1903+ ships in System32 is used unless one sits beside the exe); when it
//! cannot be created (`--ep auto`) the CPU provider runs instead.

use ort::execution_providers::{CPUExecutionProvider, DirectMLExecutionProvider};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    /// MODNet (portrait matting): dynamic input, long side 512 in multiples of 32, x/127.5 - 1, output 0..1.
    Modnet,
    /// BiRefNet (general subjects): fixed 1024x1024 input, ImageNet mean/std, output logits (sigmoid).
    Birefnet,
}

struct Args {
    model: PathBuf,
    kind: Kind,
    w: usize,
    h: usize,
    ep: String,
    threads: usize,
    probe: bool,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        model: PathBuf::new(),
        kind: Kind::Modnet,
        w: 0,
        h: 0,
        ep: "auto".into(),
        threads: 0,
        probe: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().ok_or_else(|| format!("{k} needs a value"));
        match k.as_str() {
            "--model" => a.model = PathBuf::from(v()?),
            "--kind" => {
                a.kind = match v()?.as_str() {
                    "modnet" => Kind::Modnet,
                    "birefnet" => Kind::Birefnet,
                    o => return Err(format!("unknown kind {o}")),
                }
            }
            "--width" => a.w = v()?.parse().map_err(|_| "bad --width")?,
            "--height" => a.h = v()?.parse().map_err(|_| "bad --height")?,
            "--ep" => a.ep = v()?,
            "--threads" => a.threads = v()?.parse().map_err(|_| "bad --threads")?,
            "--probe" => a.probe = true,
            "--version" => {
                println!("se-matte {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            o => return Err(format!("unknown argument {o}")),
        }
    }
    if a.model.as_os_str().is_empty() {
        return Err("--model is required".into());
    }
    if a.probe && (a.w == 0 || a.h == 0) {
        (a.w, a.h) = (640, 360);
    }
    if a.w == 0 || a.h == 0 {
        return Err("--width and --height are required".into());
    }
    Ok(a)
}

fn session(a: &Args) -> Result<(Session, &'static str), String> {
    let e = |e: ort::Error| e.to_string();
    let dml = || -> Result<Session, String> {
        Session::builder()
            .map_err(e)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(e)?
            // DirectML wants no memory pattern and sequential execution
            .with_memory_pattern(false)
            .map_err(e)?
            .with_parallel_execution(false)
            .map_err(e)?
            .with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])
            .map_err(e)?
            .commit_from_file(&a.model)
            .map_err(e)
    };
    let cpu = || -> Result<Session, String> {
        let mut b = Session::builder().map_err(e)?.with_optimization_level(GraphOptimizationLevel::Level3).map_err(e)?;
        if a.threads > 0 {
            b = b.with_intra_threads(a.threads).map_err(e)?;
        }
        b.with_execution_providers([CPUExecutionProvider::default().build()]).map_err(e)?.commit_from_file(&a.model).map_err(e)
    };
    match a.ep.as_str() {
        "cpu" => cpu().map(|s| (s, "cpu")),
        "dml" => dml().map(|s| (s, "dml")),
        _ => match dml() {
            Ok(s) => Ok((s, "dml")),
            Err(d) => cpu().map(|s| (s, "cpu")).map_err(|c| format!("DirectML: {d}; CPU: {c}")),
        },
    }
}

/// The model's input size for a `w`x`h` picture: its fixed size, or (dynamic) long side `long`, both
/// sides multiples of 32.
fn input_size(fixed: (i64, i64), w: usize, h: usize, long: usize) -> (usize, usize) {
    if fixed.0 > 0 && fixed.1 > 0 {
        return (fixed.1 as usize, fixed.0 as usize);
    }
    let s = long as f64 / w.max(h) as f64;
    let r = |v: usize| (((v as f64 * s) / 32.0).round() as usize).max(1) * 32;
    (r(w), r(h))
}

/// Bilinear sample of a `w`x`h` RGB picture into planar CHW floats at `mw`x`mh`, normalised per kind.
fn to_tensor(rgb: &[u8], w: usize, h: usize, mw: usize, mh: usize, kind: Kind, out: &mut Vec<f32>) {
    let (mean, std) = match kind {
        Kind::Modnet => ([0.5f32; 3], [0.5f32; 3]),
        Kind::Birefnet => ([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]),
    };
    out.clear();
    out.resize(3 * mw * mh, 0.0);
    let plane = mw * mh;
    let sx = w as f32 / mw as f32;
    let sy = h as f32 / mh as f32;
    let k: [f32; 3] = [1.0 / (255.0 * std[0]), 1.0 / (255.0 * std[1]), 1.0 / (255.0 * std[2])];
    let b: [f32; 3] = [mean[0] / std[0], mean[1] / std[1], mean[2] / std[2]];
    for y in 0..mh {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(h - 1);
        let wy = fy - y0 as f32;
        for x in 0..mw {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let x0 = fx as usize;
            let x1 = (x0 + 1).min(w - 1);
            let wx = fx - x0 as f32;
            let (a, bb, c, d) = ((y0 * w + x0) * 3, (y0 * w + x1) * 3, (y1 * w + x0) * 3, (y1 * w + x1) * 3);
            for ch in 0..3 {
                let top = rgb[a + ch] as f32 * (1.0 - wx) + rgb[bb + ch] as f32 * wx;
                let bot = rgb[c + ch] as f32 * (1.0 - wx) + rgb[d + ch] as f32 * wx;
                let v = top * (1.0 - wy) + bot * wy;
                out[ch * plane + y * mw + x] = v * k[ch] - b[ch];
            }
        }
    }
}

/// Bilinear sample of the model's `ow`x`oh` output into a `w`x`h` 8-bit matte.
fn to_matte(m: &[f32], ow: usize, oh: usize, sigmoid: bool, w: usize, h: usize, out: &mut [u8]) {
    let sx = ow as f32 / w as f32;
    let sy = oh as f32 / h as f32;
    let at = |x: usize, y: usize| {
        let v = m[y * ow + x];
        if sigmoid {
            1.0 / (1.0 + (-v).exp())
        } else {
            v
        }
    };
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (oh - 1) as f32);
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(oh - 1);
        let wy = fy - y0 as f32;
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (ow - 1) as f32);
            let x0 = fx as usize;
            let x1 = (x0 + 1).min(ow - 1);
            let wx = fx - x0 as f32;
            let top = at(x0, y0) * (1.0 - wx) + at(x1, y0) * wx;
            let bot = at(x0, y1) * (1.0 - wx) + at(x1, y1) * wx;
            out[y * w + x] = ((top * (1.0 - wy) + bot * wy) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
}

fn run() -> Result<(), String> {
    let a = parse()?;
    let dir = std::env::current_exe().map_err(|e| e.to_string())?.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let dll = std::env::var_os("SE_ORT_DLL").map(PathBuf::from).unwrap_or_else(|| dir.join("onnxruntime.dll"));
    if !dll.is_file() {
        return Err(format!("{} not found", dll.display()));
    }
    ort::init_from(dll.to_string_lossy()).with_name("se-matte").commit().map_err(|e| e.to_string())?;
    let (mut session, mut ep) = session(&a)?;
    let shape = session.inputs.first().and_then(|i| i.input_type.tensor_shape().map(|s| s.to_vec())).unwrap_or_default();
    let fixed = if shape.len() == 4 { (shape[2], shape[3]) } else { (-1, -1) };
    let (mw, mh) = input_size(fixed, a.w, a.h, 512);
    let sigmoid = a.kind == Kind::Birefnet;
    let (w, h) = (a.w, a.h);
    let mut rgb = vec![0u8; w * h * 3];
    let mut input = Vec::new();
    let mut matte = vec![0u8; w * h];
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let infer = |session: &mut Session, rgb: &[u8], matte: &mut [u8], input: &mut Vec<f32>| -> Result<(), String> {
        to_tensor(rgb, w, h, mw, mh, a.kind, input);
        let t = Tensor::from_array(([1usize, 3, mh, mw], std::mem::take(input))).map_err(|e| e.to_string())?;
        let out = session.run(ort::inputs![t]).map_err(|e| e.to_string())?;
        let (s, m) = out[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let n = s.len();
        if n < 2 {
            return Err(format!("unexpected output shape {:?}", &**s));
        }
        let (oh, ow) = (s[n - 2] as usize, s[n - 1] as usize);
        if ow * oh == 0 || m.len() < ow * oh {
            return Err(format!("unexpected output shape {:?}", &**s));
        }
        to_matte(&m[..ow * oh], ow, oh, sigmoid, w, h, matte);
        Ok(())
    };
    // the first run compiles the graph, and is where DirectML finds out it has too little GPU memory
    // (a big model next to other GPU work): `auto` then goes on on the CPU
    rgb.fill(128);
    if let Err(e) = infer(&mut session, &rgb, &mut matte, &mut input) {
        if ep != "dml" || a.ep != "auto" {
            return Err(e);
        }
        eprintln!("DirectML failed on the first frame, using the CPU: {e}");
        let cpu = Args { ep: "cpu".into(), model: a.model.clone(), ..a };
        session = self::session(&cpu)?.0;
        ep = "cpu";
        infer(&mut session, &rgb, &mut matte, &mut input)?;
    }
    eprintln!("ep {ep}");
    if a.probe {
        let t0 = Instant::now();
        infer(&mut session, &rgb, &mut matte, &mut input)?;
        println!("{{\"ep\":\"{ep}\",\"ms\":{:.1},\"input\":[{mw},{mh}]}}", t0.elapsed().as_secs_f64() * 1000.0);
        return Ok(());
    }
    loop {
        match stdin.read_exact(&mut rgb) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
        infer(&mut session, &rgb, &mut matte, &mut input)?;
        if stdout.write_all(&matte).and_then(|_| stdout.flush()).is_err() {
            return Ok(()); // the app stopped reading: cancelled
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_inputs_keep_the_aspect_in_multiples_of_32() {
        assert_eq!(input_size((-1, -1), 1920, 1080, 512), (512, 288));
        assert_eq!(input_size((-1, -1), 1080, 1920, 512), (288, 512));
        assert_eq!(input_size((1024, 1024), 640, 360, 512), (1024, 1024));
    }

    #[test]
    fn a_flat_output_maps_to_a_flat_matte() {
        let mut out = vec![0u8; 8 * 4];
        to_matte(&[1.0; 4], 2, 2, false, 8, 4, &mut out);
        assert!(out.iter().all(|&v| v == 255));
        to_matte(&[0.0; 4], 2, 2, true, 8, 4, &mut out);
        assert!(out.iter().all(|&v| v == 128));
    }

    #[test]
    fn grey_normalises_to_zero_for_modnet() {
        let rgb = vec![128u8; 4 * 4 * 3];
        let mut t = Vec::new();
        to_tensor(&rgb, 4, 4, 2, 2, Kind::Modnet, &mut t);
        assert_eq!(t.len(), 12);
        assert!(t.iter().all(|v| v.abs() < 0.01));
    }
}
