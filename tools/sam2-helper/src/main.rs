//! se-sam2: Simple Editor's object-mask helper. SAM 2.1 tiny (Meta, Apache-2.0) as five ONNX graphs
//! (the square-zero-labs export: vision encoder, mask decoder, memory encoder, memory attention,
//! object-pointer positions) run by ONNX Runtime (MIT) on the GPU through DirectML, or on the CPU.
//!
//! A child process of the editor, never linked into it. It reads frames itself through ffmpeg.exe
//! (`-f rawvideo rgb24` at 1024×1024) and writes one 8-bit greyscale PNG per frame. Protocol: one JSON
//! object per line on stdin, one per line on stdout (see `serve`). Every reply carries `"ok"`.
//!
//!   se-sam2 info  --models DIR [--cpu]     load the graphs, print their inputs / outputs and the device
//!   se-sam2 serve --models DIR [--cpu]     the editor's session (below)
//!
//! Commands (stdin):
//!   {"cmd":"frame","ffmpeg":F,"video":V,"time":T}           encode one frame, keep it as the seed
//!   {"cmd":"points","points":[[x,y,l],..],"out":P,"w":W,"h":H} mask the seed frame; x, y in 0..1, l 1 = add, 0 = remove
//!   {"cmd":"track","ffmpeg":F,"video":V,"start":T0,"fps":R,"count":N,"seed":K,
//!    "points":[[x,y,l],..],"dir":D,"w":W,"h":H}             propagate from frame K both ways; D/NNNNNN.png
//!   {"cmd":"quit"}
//! Track prints {"progress":i,"of":N} lines as it goes, then the reply. Cancel = kill the process.

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const S: usize = 1024; // model input size
const F: usize = 64; // feature map size
const HW: usize = F * F;
const C: usize = 256; // hidden dim
const M: usize = 64; // memory dim
const NUM_MASKMEM: usize = 7;
const MAX_PTRS: usize = 16;

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).cloned().unwrap_or_default();
    let flag = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let cpu = args.iter().any(|a| a == "--cpu");
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let models = flag("--models").map(PathBuf::from).unwrap_or_else(|| exe_dir.join("models"));
    let r = match mode.as_str() {
        "info" => Sam::load(&exe_dir, &models, cpu).map(|s| {
            println!("{}", json!({"ok": true, "device": s.device, "load_ms": s.load_ms, "graphs": s.describe()}));
        }),
        "serve" => Sam::load(&exe_dir, &models, cpu).and_then(serve),
        "--version" | "version" => {
            println!("se-sam2 {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("usage: se-sam2 info|serve [--models DIR] [--cpu]");
            std::process::exit(2);
        }
    };
    if let Err(err) = r {
        println!("{}", json!({"ok": false, "error": err}));
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------------------------------
// the model

struct Consts {
    mean: [f32; 3],
    std: [f32; 3],
    /// memory temporal positional encoding, NUM_MASKMEM rows of M
    tpe: Vec<[f32; M]>,
}

struct Sam {
    enc: Session,
    dec: Session,
    menc: Session,
    mattn: Session,
    ptpos: Session,
    k: Consts,
    device: &'static str,
    load_ms: u128,
    /// the seed frame's encoder outputs (`frame`)
    seed: Option<Enc>,
}

/// One frame's vision-encoder outputs.
struct Enc {
    feats0: Vec<f32>,
    feats1: Vec<f32>,
    feats2: Vec<f32>,
    feats2_no_mem: Vec<f32>,
    pos: Vec<f32>,
}

/// One decoded frame.
struct Dec {
    high: Vec<f32>, // [1024*1024] logits
    score: f32,     // object score logit
    iou: f32,
    ptr: Vec<f32>, // [256]
}

/// One frame's memory: memory tokens + spatial positions ([4096·64] each) and its object pointer.
struct Mem {
    tok: Vec<f32>,
    pos: Vec<f32>,
    ptr: Vec<f32>,
}

fn session(path: &Path, gpu: bool) -> R<Session> {
    let mut b = Session::builder().map_err(e)?;
    let lvl = match std::env::var("SE_SAM2_OPT").ok().as_deref() {
        Some("0") => GraphOptimizationLevel::Disable,
        Some("1") => GraphOptimizationLevel::Level1,
        Some("2") => GraphOptimizationLevel::Level2,
        _ => GraphOptimizationLevel::Level3,
    };
    b = b.with_optimization_level(lvl).map_err(e)?;
    if gpu {
        // DirectML wants sequential execution and no memory pattern
        b = b
            .with_parallel_execution(false)
            .map_err(e)?
            .with_memory_pattern(false)
            .map_err(e)?
            .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
            .map_err(e)?;
    } else {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        b = b.with_intra_threads(n.saturating_sub(1).max(1)).map_err(e)?;
    }
    let t = Instant::now();
    let s = b.commit_from_file(path).map_err(|x| format!("{}: {x}", path.display()))?;
    eprintln!("se-sam2: {} on the {} in {} ms", path.file_name().unwrap_or_default().to_string_lossy(), if gpu { "GPU" } else { "CPU" }, t.elapsed().as_millis());
    Ok(s)
}

impl Sam {
    fn load(exe_dir: &Path, models: &Path, cpu: bool) -> R<Sam> {
        let t = Instant::now();
        let dll = exe_dir.join("onnxruntime.dll");
        ort::init_from(&dll).map_err(|x| format!("{}: {x}", dll.display()))?.with_name("se-sam2").commit();
        let k = consts(&models.join("constants.json"))?;
        // The mask decoder stays on the CPU by default: on DirectML it recompiles for every new number of
        // click points (48 s for the first click on an RTX 2060 Super, 1.4 s on the CPU) and is no faster
        // once warm. SE_SAM2_CPU_GRAPHS=enc,dec,menc,mattn picks the CPU graphs by hand ("" = all on the GPU).
        let cpu_only = std::env::var("SE_SAM2_CPU_GRAPHS").unwrap_or_else(|_| "dec".into());
        let on = |g: &str, gpu: bool| gpu && !cpu_only.split(',').any(|x| x.trim() == g);
        let open = |gpu: bool| -> R<[Session; 5]> {
            Ok([
                session(&models.join("vision_encoder.onnx"), on("enc", gpu))?,
                session(&models.join("mask_decoder.onnx"), on("dec", gpu))?,
                session(&models.join("memory_encoder.onnx"), on("menc", gpu))?,
                session(&models.join("memory_attention.onnx"), on("mattn", gpu))?,
                // a tiny graph: the CPU is faster than a GPU round trip
                session(&models.join("pointer_tpos.onnx"), false)?,
            ])
        };
        let (s, device) = if cpu {
            (open(false)?, "cpu")
        } else {
            match open(true) {
                Ok(s) => (s, "gpu"),
                Err(err) => {
                    eprintln!("se-sam2: DirectML unavailable ({err}); using the CPU");
                    (open(false)?, "cpu")
                }
            }
        };
        let [enc, dec, menc, mattn, ptpos] = s;
        Ok(Sam { enc, dec, menc, mattn, ptpos, k, device, load_ms: t.elapsed().as_millis(), seed: None })
    }

    fn describe(&self) -> Value {
        let d = |s: &Session| {
            let io = |v: &[ort::value::Outlet]| {
                v.iter().map(|o| format!("{}: {:?}", o.name(), o.dtype())).collect::<Vec<_>>()
            };
            json!({"inputs": io(s.inputs()), "outputs": io(s.outputs())})
        };
        json!({"vision_encoder": d(&self.enc), "mask_decoder": d(&self.dec), "memory_encoder": d(&self.menc),
               "memory_attention": d(&self.mattn), "pointer_tpos": d(&self.ptpos)})
    }

    /// `rgb` = 1024×1024 RGB24.
    fn encode(&mut self, rgb: &[u8]) -> R<Enc> {
        let mut px = vec![0f32; 3 * S * S];
        for (i, p) in rgb.chunks_exact(3).enumerate() {
            for c in 0..3 {
                px[c * S * S + i] = (p[c] as f32 / 255.0 - self.k.mean[c]) / self.k.std[c];
            }
        }
        let t = Tensor::from_array(([1usize, 3, S, S], px)).map_err(e)?;
        let o = self.enc.run(ort::inputs!["pixel_values" => t]).map_err(e)?;
        let get = |n: &str| -> R<Vec<f32>> {
            let v = o.get(n).ok_or(format!("vision_encoder has no output {n}"))?;
            Ok(v.try_extract_tensor::<f32>().map_err(e)?.1.to_vec())
        };
        Ok(Enc {
            feats0: get("feats0")?,
            feats1: get("feats1")?,
            feats2: get("feats2")?,
            feats2_no_mem: get("feats2_no_mem")?,
            pos: get("vision_pos_embed")?,
        })
    }

    /// The mask decoder on `cond` (feats2_no_mem for a prompted first frame, else the memory-conditioned
    /// features). `points` in 1024 px; an empty list sends one padding point (label -1).
    fn decode(&mut self, enc: &Enc, cond: &[f32], points: &[(f32, f32, i32)]) -> R<Dec> {
        let pts: Vec<(f32, f32, i32)> = if points.is_empty() { vec![(0.0, 0.0, -1)] } else { points.to_vec() };
        let n = pts.len();
        let xy: Vec<f32> = pts.iter().flat_map(|p| [p.0, p.1]).collect();
        let lab: Vec<i32> = pts.iter().map(|p| p.2).collect();
        let o = self
            .dec
            .run(ort::inputs![
                "feats0" => Tensor::from_array(([1usize, 32, 256, 256], enc.feats0.clone())).map_err(e)?,
                "feats1" => Tensor::from_array(([1usize, 64, 128, 128], enc.feats1.clone())).map_err(e)?,
                "feats2_cond" => Tensor::from_array(([1usize, C, F, F], cond.to_vec())).map_err(e)?,
                "input_points" => Tensor::from_array(([1usize, 1, n, 2], xy)).map_err(e)?,
                "input_labels" => Tensor::from_array(([1usize, 1, n], lab)).map_err(e)?,
            ])
            .map_err(e)?;
        let get = |k: &str| -> R<Vec<f32>> {
            let v = o.get(k).ok_or(format!("mask_decoder has no output {k}"))?;
            Ok(v.try_extract_tensor::<f32>().map_err(e)?.1.to_vec())
        };
        Ok(Dec {
            high: get("high_res_mask")?,
            score: get("object_score_logits")?.first().copied().unwrap_or(0.0),
            iou: get("iou")?.first().copied().unwrap_or(0.0),
            ptr: get("object_pointer")?,
        })
    }

    fn memory(&mut self, enc: &Enc, d: &Dec, binarize: bool) -> R<Mem> {
        let o = self
            .menc
            .run(ort::inputs![
                "feats2" => Tensor::from_array(([1usize, C, F, F], enc.feats2.clone())).map_err(e)?,
                "high_res_mask" => Tensor::from_array(([1usize, 1, S, S], d.high.clone())).map_err(e)?,
                "object_score_logits" => Tensor::from_array(([1usize, 1], vec![d.score])).map_err(e)?,
                "binarize" => Tensor::from_array(((), vec![if binarize { 1.0f32 } else { 0.0 }])).map_err(e)?,
            ])
            .map_err(e)?;
        let get = |k: &str| -> R<Vec<f32>> {
            let v = o.get(k).ok_or(format!("memory_encoder has no output {k}"))?;
            Ok(v.try_extract_tensor::<f32>().map_err(e)?.1.to_vec())
        };
        Ok(Mem { tok: get("memory_tokens")?, pos: get("memory_pos")?, ptr: d.ptr.clone() })
    }

    /// Memory attention over the bank: `blocks` = (temporal offset, memory) with offset 0 = the prompted
    /// frame, oldest-first otherwise; `ptrs` = (temporal offset, pointer), prompted first. `max_ptr` =
    /// min(frames, 16) (the normaliser).
    fn attend(&mut self, enc: &Enc, blocks: &[(usize, &Mem)], ptrs: &[(usize, &Mem)], max_ptr: usize) -> R<Vec<f32>> {
        // (C, H·W) -> (H·W, 1, C)
        let tok = |v: &[f32], ch: usize| -> Vec<f32> {
            let mut o = vec![0f32; v.len()];
            for c in 0..ch {
                for i in 0..HW {
                    o[i * ch + c] = v[c * HW + i];
                }
            }
            o
        };
        let cur = tok(&enc.feats2, C);
        let cur_pos = tok(&enc.pos, C);
        // the bank: NUM_MASKMEM blocks, padded by repeating the most recent one
        let mut blocks: Vec<(usize, &Mem)> = blocks.to_vec();
        while blocks.len() < NUM_MASKMEM {
            let last = *blocks.last().ok_or("an empty memory bank")?;
            blocks.push(last);
        }
        let mut mem = Vec::with_capacity((NUM_MASKMEM * HW + MAX_PTRS * 4) * M);
        let mut mpos = Vec::with_capacity(mem.capacity());
        for (off, m) in &blocks {
            mem.extend_from_slice(&m.tok);
            let row = &self.k.tpe[(off + NUM_MASKMEM - 1) % NUM_MASKMEM]; // offset 0 -> the last row
            for (i, p) in m.pos.iter().enumerate() {
                mpos.push(p + row[i % M]);
            }
        }
        let mut ptrs: Vec<(usize, &Mem)> = ptrs.to_vec();
        while ptrs.len() < MAX_PTRS {
            let last = *ptrs.last().ok_or("no object pointers")?;
            ptrs.push(last);
        }
        let denom = (max_ptr.max(2) - 1) as f32;
        let diffs: Vec<f32> = ptrs.iter().map(|(o, _)| *o as f32 / denom).collect();
        let o = self
            .ptpos
            .run(ort::inputs!["normalized_diffs" => Tensor::from_array(([MAX_PTRS], diffs)).map_err(e)?])
            .map_err(e)?;
        let ppos = o[0].try_extract_tensor::<f32>().map_err(e)?.1.to_vec(); // [16, 64]
        drop(o);
        for (i, (_, m)) in ptrs.iter().enumerate() {
            // a 256-wide pointer is 4 tokens of 64, each with the pointer's position
            for s in 0..C / M {
                mem.extend_from_slice(&m.ptr[s * M..(s + 1) * M]);
                mpos.extend_from_slice(&ppos[i * M..(i + 1) * M]);
            }
        }
        let n = mem.len() / M;
        let o = self
            .mattn
            .run(ort::inputs![
                "current_vision_features" => Tensor::from_array(([HW, 1, C], cur)).map_err(e)?,
                "current_vision_position_embeddings" => Tensor::from_array(([HW, 1, C], cur_pos)).map_err(e)?,
                "memory" => Tensor::from_array(([n, 1, M], mem)).map_err(e)?,
                "memory_pos" => Tensor::from_array(([n, 1, M], mpos)).map_err(e)?,
            ])
            .map_err(e)?;
        Ok(o[0].try_extract_tensor::<f32>().map_err(e)?.1.to_vec())
    }
}

fn consts(path: &Path) -> R<Consts> {
    let text = std::fs::read_to_string(path).map_err(|x| format!("{}: {x}", path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(e)?;
    let three = |k: &str| -> R<[f32; 3]> {
        let a = v[k].as_array().ok_or(format!("constants.json: no {k}"))?;
        Ok([0, 1, 2].map(|i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32))
    };
    let rows = v["memory_temporal_positional_encoding"].as_array().ok_or("constants.json: no temporal encoding")?;
    let tpe: Vec<[f32; M]> = rows
        .iter()
        .map(|r| {
            let mut o = [0f32; M];
            for (i, x) in r.as_array().into_iter().flatten().take(M).enumerate() {
                o[i] = x.as_f64().unwrap_or(0.0) as f32;
            }
            o
        })
        .collect();
    if tpe.len() != NUM_MASKMEM {
        return Err("constants.json: the temporal encoding is not 7 rows".into());
    }
    Ok(Consts { mean: three("image_mean")?, std: three("image_std")?, tpe })
}

// ---------------------------------------------------------------------------------------------------
// frames (ffmpeg) and masks (png)

/// Frames of `video` from `start` at `fps` (`count` of them), scaled to 1024×1024 RGB24.
struct Frames {
    child: std::process::Child,
    out: std::process::ChildStdout,
}

impl Frames {
    fn open(ffmpeg: &str, video: &str, start: f64, fps: f64, count: usize) -> R<Frames> {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-nostdin"]);
        if start > 0.0 {
            cmd.args(["-ss", &format!("{start:.6}")]);
        }
        cmd.arg("-i").arg(video);
        let vf = if fps > 0.0 {
            format!("fps={fps:.6},scale={S}:{S}:flags=bilinear")
        } else {
            format!("scale={S}:{S}:flags=bilinear")
        };
        cmd.args(["-an", "-sn", "-vf", &vf, "-frames:v", &count.max(1).to_string(), "-pix_fmt", "rgb24", "-f", "rawvideo", "-"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|x| format!("ffmpeg: {x}"))?;
        let out = child.stdout.take().ok_or("ffmpeg: no stdout")?;
        Ok(Frames { child, out })
    }
    /// The next frame, None at the end.
    fn next(&mut self) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; S * S * 3];
        self.out.read_exact(&mut buf).ok().map(|_| buf)
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The 1024² logits resized to w×h (bilinear), as sigmoid coverage bytes.
fn mask_bytes(high: &[f32], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    let (sx, sy) = (S as f32 / w as f32, S as f32 / h as f32);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (S - 1) as f32);
        let (y0, ty) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(S - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (S - 1) as f32);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(S - 1);
            let a = high[y0 * S + x0] * (1.0 - tx) + high[y0 * S + x1] * tx;
            let b = high[y1 * S + x0] * (1.0 - tx) + high[y1 * S + x1] * tx;
            let l = a * (1.0 - ty) + b * ty;
            out[y * w + x] = (255.0 / (1.0 + (-l).exp()) + 0.5) as u8;
        }
    }
    out
}

fn write_png(path: &Path, w: usize, h: usize, px: &[u8]) -> R<()> {
    let tmp = path.with_extension("png.part");
    {
        let f = std::fs::File::create(&tmp).map_err(|x| format!("{}: {x}", tmp.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().map_err(e)?;
        wr.write_image_data(px).map_err(e)?;
    }
    std::fs::rename(&tmp, path).map_err(|x| format!("{}: {x}", path.display()))
}

/// Fraction of the frame covered (coverage ≥ 50%).
fn area(px: &[u8]) -> f64 {
    px.iter().filter(|&&v| v >= 128).count() as f64 / px.len().max(1) as f64
}

// ---------------------------------------------------------------------------------------------------
// the session

fn points_of(v: &Value) -> Vec<(f32, f32, i32)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let a = p.as_array()?;
            let x = a.first()?.as_f64()? as f32;
            let y = a.get(1)?.as_f64()? as f32;
            let l = a.get(2).and_then(Value::as_i64).unwrap_or(1) as i32;
            Some((x.clamp(0.0, 1.0) * S as f32, y.clamp(0.0, 1.0) * S as f32, if l > 0 { 1 } else { 0 }))
        })
        .collect()
}

fn say(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn serve(mut sam: Sam) -> R<()> {
    say(json!({"ok": true, "ready": true, "device": sam.device, "load_ms": sam.load_ms}));
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(err) => {
                say(json!({"ok": false, "error": format!("bad command: {err}")}));
                continue;
            }
        };
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let t = Instant::now();
        let r: R<Value> = match v["cmd"].as_str().unwrap_or("") {
            "quit" => break,
            "frame" => (|| {
                let mut fr = Frames::open(&s("ffmpeg"), &s("video"), v["time"].as_f64().unwrap_or(0.0), 0.0, 1)?;
                let rgb = fr.next().ok_or("ffmpeg returned no frame")?;
                let read_ms = t.elapsed().as_millis();
                sam.seed = Some(sam.encode(&rgb)?);
                Ok(json!({"read_ms": read_ms}))
            })(),
            "points" => (|| {
                let enc = sam.seed.take().ok_or("no frame (send a frame command first)")?;
                let pts = points_of(&v["points"]);
                let r = (|| {
                    let d = sam.decode(&enc, &enc.feats2_no_mem.clone(), &pts)?;
                    let (w, h) = (v["w"].as_u64().unwrap_or(512) as usize, v["h"].as_u64().unwrap_or(512) as usize);
                    let px = mask_bytes(&d.high, w.max(1), h.max(1));
                    write_png(Path::new(&s("out")), w.max(1), h.max(1), &px)?;
                    Ok(json!({"iou": d.iou, "score": d.score, "area": area(&px)}))
                })();
                sam.seed = Some(enc);
                r
            })(),
            "track" => track(&mut sam, &v),
            other => Err(format!("unknown command {other:?}")),
        };
        say(match r {
            Ok(mut o) => {
                o["ok"] = json!(true);
                o["ms"] = json!(t.elapsed().as_millis());
                o
            }
            Err(err) => json!({"ok": false, "error": err}),
        });
    }
    Ok(())
}

/// Propagate from the prompted frame `seed` to both ends of `count` frames.
fn track(sam: &mut Sam, v: &Value) -> R<Value> {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let (ffmpeg, video) = (s("ffmpeg"), s("video"));
    let start = v["start"].as_f64().unwrap_or(0.0);
    let fps = v["fps"].as_f64().filter(|f| *f > 0.0).ok_or("no fps")?;
    let count = v["count"].as_u64().unwrap_or(0) as usize;
    if count == 0 {
        return Err("no frames to track".into());
    }
    let seed = (v["seed"].as_u64().unwrap_or(0) as usize).min(count - 1);
    let pts = points_of(&v["points"]);
    if !pts.iter().any(|p| p.2 == 1) {
        return Err("no points on the object".into());
    }
    let dir = PathBuf::from(s("dir"));
    std::fs::create_dir_all(&dir).map_err(|x| format!("{}: {x}", dir.display()))?;
    let (w, h) = (v["w"].as_u64().unwrap_or(512).max(1) as usize, v["h"].as_u64().unwrap_or(512).max(1) as usize);
    let max_ptr = count.min(MAX_PTRS);
    let mut done = 0usize;
    let mut stats = Stats::default();

    // the prompted frame
    let t0 = Instant::now();
    let rgb = Frames::open(&ffmpeg, &video, start + seed as f64 / fps, 0.0, 1)?.next().ok_or("ffmpeg returned no frame")?;
    stats.read += t0.elapsed().as_secs_f64();
    let enc = sam.encode(&rgb)?;
    let d = sam.decode(&enc, &enc.feats2_no_mem.clone(), &pts)?;
    let cond = sam.memory(&enc, &d, true)?;
    let px = mask_bytes(&d.high, w, h);
    write_png(&dir.join(format!("{seed:06}.png")), w, h, &px)?;
    let mut areas = BTreeMap::new();
    areas.insert(seed, area(&px));
    done += 1;
    say(json!({"progress": done, "of": count}));

    // forward: one ffmpeg stream from the frame after the seed
    if seed + 1 < count {
        let mut fr = Frames::open(&ffmpeg, &video, start + (seed + 1) as f64 / fps, fps, count - seed - 1)?;
        let mut recent: Vec<Mem> = Vec::new(); // nearest last
        for i in seed + 1..count {
            let t = Instant::now();
            let Some(rgb) = fr.next() else { break };
            stats.read += t.elapsed().as_secs_f64();
            let px = step(sam, &rgb, &cond, i - seed, &mut recent, max_ptr, w, h, &mut stats)?;
            write_png(&dir.join(format!("{i:06}.png")), w, h, &px)?;
            areas.insert(i, area(&px));
            done += 1;
            say(json!({"progress": done, "of": count}));
        }
    }
    // backward: chunks read forward, worked through in reverse
    if seed > 0 {
        let mut recent: Vec<Mem> = Vec::new();
        let mut hi = seed; // exclusive
        while hi > 0 {
            let lo = hi.saturating_sub(24);
            let t = Instant::now();
            let mut fr = Frames::open(&ffmpeg, &video, start + lo as f64 / fps, fps, hi - lo)?;
            let mut chunk = Vec::with_capacity(hi - lo);
            while chunk.len() < hi - lo {
                match fr.next() {
                    Some(f) => chunk.push(f),
                    None => break,
                }
            }
            drop(fr);
            stats.read += t.elapsed().as_secs_f64();
            for (k, rgb) in chunk.iter().enumerate().rev() {
                let i = lo + k;
                let px = step(sam, rgb, &cond, seed - i, &mut recent, max_ptr, w, h, &mut stats)?;
                write_png(&dir.join(format!("{i:06}.png")), w, h, &px)?;
                areas.insert(i, area(&px));
                done += 1;
                say(json!({"progress": done, "of": count}));
            }
            hi = lo;
        }
    }
    let n = stats.frames.max(1) as f64;
    Ok(json!({
        "frames": done,
        "device": sam.device,
        "per_frame_ms": {
            "read": stats.read * 1000.0 / done.max(1) as f64,
            "encode": stats.enc * 1000.0 / n, "attend": stats.att * 1000.0 / n,
            "decode": stats.dec * 1000.0 / n, "memory": stats.mem * 1000.0 / n,
            "total": (stats.enc + stats.att + stats.dec + stats.mem) * 1000.0 / n,
        },
        "areas": areas.values().collect::<Vec<_>>(),
    }))
}

#[derive(Default)]
struct Stats {
    frames: usize,
    read: f64,
    enc: f64,
    att: f64,
    dec: f64,
    mem: f64,
}

/// One tracked frame `dist` frames away from the prompted one. `recent` holds the last frames'
/// memories, nearest last (at most NUM_MASKMEM - 1 used; pointers reach 15 back).
#[allow(clippy::too_many_arguments)]
fn step(
    sam: &mut Sam,
    rgb: &[u8],
    cond: &Mem,
    dist: usize,
    recent: &mut Vec<Mem>,
    max_ptr: usize,
    w: usize,
    h: usize,
    st: &mut Stats,
) -> R<Vec<u8>> {
    let t = Instant::now();
    let enc = sam.encode(rgb)?;
    st.enc += t.elapsed().as_secs_f64();
    // blocks: the prompted frame (offset 0), then offsets 6..1 oldest first (only those that exist)
    let mut blocks: Vec<(usize, &Mem)> = vec![(0, cond)];
    let n = recent.len();
    for off in (1..NUM_MASKMEM).rev() {
        if off <= n && off < dist + 1 {
            blocks.push((off, &recent[n - off]));
        }
    }
    // pointers: the prompted frame at its distance, then 1.. back (up to max_ptr - 1)
    let mut ptrs: Vec<(usize, &Mem)> = vec![(dist, cond)];
    for off in 1..max_ptr {
        if off <= n && off < dist {
            ptrs.push((off, &recent[n - off]));
        }
    }
    let t = Instant::now();
    let feats = sam.attend(&enc, &blocks, &ptrs, max_ptr)?;
    st.att += t.elapsed().as_secs_f64();
    let t = Instant::now();
    let d = sam.decode(&enc, &feats, &[])?;
    st.dec += t.elapsed().as_secs_f64();
    let t = Instant::now();
    let m = sam.memory(&enc, &d, false)?;
    st.mem += t.elapsed().as_secs_f64();
    st.frames += 1;
    let px = mask_bytes(&d.high, w, h);
    recent.push(m);
    if recent.len() > MAX_PTRS {
        recent.remove(0);
    }
    Ok(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Positive logits on the left half, negative on the right: the resized mask keeps the split, at any size.
    #[test]
    fn mask_bytes_resizes_logits_into_coverage() {
        let high: Vec<f32> = (0..S * S).map(|i| if i % S < S / 2 { 8.0 } else { -8.0 }).collect();
        for (w, h) in [(1024, 576), (320, 180), (7, 3)] {
            let m = mask_bytes(&high, w, h);
            assert_eq!(m.len(), w * h);
            assert!(m[0] >= 250 && m[w - 1] <= 5, "{w}x{h}: left inside, right outside");
            assert!((area(&m) - 0.5).abs() < 1.0 / w as f64 + 1e-9, "{w}x{h}: half the frame");
        }
        assert_eq!(mask_bytes(&vec![0.0; S * S], 2, 2), vec![128; 4], "a zero logit is half coverage");
    }

    #[test]
    fn points_scale_to_the_model_input_and_clamp() {
        let p = points_of(&serde_json::json!([[0.5, 0.25, 1], [1.5, -0.2, 0], [0.1, 0.1], "junk", [0.2]]));
        assert_eq!(p, vec![(512.0, 256.0, 1), (1024.0, 0.0, 0), (102.4, 102.4, 1)]);
        assert!(points_of(&serde_json::json!(null)).is_empty());
    }

    #[test]
    fn png_frames_land_whole_and_area_counts_half_coverage() {
        let dir = std::env::temp_dir().join(format!("se-sam2-helper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("000000.png");
        let px = [0u8, 127, 128, 255, 200, 10];
        write_png(&path, 3, 2, &px).unwrap();
        assert!(!path.with_extension("png.part").exists(), "written to a .part, then renamed in");
        let mut r = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap())).read_info().unwrap();
        let mut buf = vec![0u8; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height, &buf[..6]), (3, 2, &px[..]));
        assert_eq!(area(&px), 0.5);
        assert_eq!(area(&[]), 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
