use super::*;

pub(super) fn effect_thumb_key(kind: EffectKind, size: (u32, u32), image: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a
    let mut eat = |b: &[u8]| {
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
    };
    eat(kind.name().as_bytes());
    eat(&size.0.to_le_bytes());
    eat(&size.1.to_le_bytes());
    eat(image.as_bytes());
    h
}

/// The embedded stock picture, already decoded: `assets/bubble.webp` converted once to raw RGBA at card
/// size, because nothing in the binary can decode a webp (no image crate, and ffmpeg may be missing).
pub(crate) const STOCK: &[u8] = include_bytes!("../../../assets/bubble_96x54.rgba");
pub(crate) const STOCK_W: u32 = 96;
pub(crate) const STOCK_H: u32 = 54;

#[allow(dead_code)]
/// The picture the effect thumbnails are rendered from: the user's stock image, else the embedded one.
pub(super) fn effect_thumb_source(image: &str, w: u32, h: u32, backend: Backend) -> Frame {
    let (w, h) = (w.max(1), h.max(1));
    if !image.is_empty() {
        if let Ok(mut src) = media::open_video(image, backend) {
            let mut f = Frame::default();
            if src.frame_at(0.0, w, h, &mut f) && !f.is_empty() {
                return f;
            }
        }
    }
    // ponytail: nearest rescale — the catalogue asks for exactly STOCK_W x STOCK_H, so it is a plain copy
    let mut f = Frame::new(w, h);
    for y in 0..h {
        let sy = y * STOCK_H / h;
        for x in 0..w {
            let s = ((sy * STOCK_W + x * STOCK_W / w) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            f.rgba[d..d + 4].copy_from_slice(&STOCK[s..s + 4]);
        }
    }
    f
}

pub(super) fn box_blur(rgba: &mut [u8], w: usize, h: usize, radius: usize) {
    if w == 0 || h == 0 || radius == 0 {
        return;
    }
    let mut tmp = vec![0u8; rgba.len()];
    // one horizontal sliding-window pass over `src` into `dst`; the vertical pass reuses it transposed
    // by swapping the stride arguments.
    let pass = |src: &[u8], dst: &mut [u8], cols: usize, rows: usize, col_stride: usize, row_stride: usize| {
        for row in 0..rows {
            let at = |col: usize| row * row_stride + col * col_stride;
            let mut sum = [0usize; 4];
            for col in 0..=radius.min(cols - 1) {
                let p = at(col);
                for c in 0..4 {
                    sum[c] += src[p + c] as usize;
                }
            }
            let mut count = radius.min(cols - 1) + 1;
            for col in 0..cols {
                let p = at(col);
                for c in 0..4 {
                    dst[p + c] = (sum[c] / count) as u8;
                }
                if col + radius + 1 < cols {
                    let q = at(col + radius + 1);
                    for c in 0..4 {
                        sum[c] += src[q + c] as usize;
                    }
                    count += 1;
                }
                if col >= radius {
                    let q = at(col - radius);
                    for c in 0..4 {
                        sum[c] -= src[q + c] as usize;
                    }
                    count -= 1;
                }
            }
        }
    };
    for _ in 0..3 {
        pass(rgba, &mut tmp, w, h, 4, w * 4); // horizontal
        pass(&tmp, rgba, h, w, w * 4, 4); // vertical
    }
}

/// Write one rendered frame as PNG / JPG / WebP through ffmpeg (raw RGBA on stdin), resizing to
/// `opts.size` with `opts.resize` when the render came out at a different size.

pub(super) fn write_image(frame: &Frame, opts: &frame_ui::FrameExport) -> Result<(), String> {
    if frame.is_empty() {
        return Err("empty frame".into());
    }
    let exe = media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let ext = opts.out.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "png".into());
    let mut cmd = media::ffpipe::command(&exe);
    cmd.args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba"]);
    cmd.args(["-s", &format!("{}x{}", frame.width, frame.height), "-i", "-"]);
    if (frame.width, frame.height) != opts.size {
        let flags = if opts.resize.is_empty() { "lanczos" } else { opts.resize.as_str() };
        cmd.args(["-vf", &format!("scale={}:{}:flags={flags}", opts.size.0, opts.size.1)]);
    }
    let q = opts.quality.clamp(1, 100);
    match ext.as_str() {
        // ffmpeg's mjpeg qscale is 2 (best) .. 31 (worst)
        "jpg" | "jpeg" => cmd.args(["-q:v", &format!("{}", 2 + (100 - q) * 29 / 100)]),
        "webp" => cmd.args(["-quality", &format!("{q}")]),
        _ => cmd.args(["-compression_level", "9"]),
    };
    cmd.args(["-frames:v", "1"]).arg(&opts.out);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(&frame.rgba); // a broken pipe shows up as a non-zero exit below
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Standard base64 (RFC 4648, with padding) — for the `render.frame` PNG data url. Tool path, not hot.
pub(super) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------- MCP tool argument helpers ----------------

impl App {
    pub(super) fn build_effect_thumbnails(&mut self, ctx: &egui::Context) {
        const TW: u32 = 96;
        const TH: u32 = 54;
        let key = (self.settings.effect_thumb_image.clone(), TW);
        if self.gpu.is_none() || self.effect_thumbs_key.as_ref() == Some(&key) {
            return;
        }
        let src = effect_thumb_source(&key.0, TW, TH, self.backend());
        if src.is_empty() {
            return;
        }
        effects_ui::clear_thumbnails();
        self.effect_thumbs.clear();
        let mut out = Frame::default();
        for kind in EffectKind::ALL {
            // geometric kinds move the layer instead of touching pixels: show the plain source
            let effect = crate::model::Effect::new(kind);
            let rendered = {
                let Some(gpu) = self.gpu.as_mut() else { break };
                guarded(|| gpu.effect_preview(&src, &effect, 0.35, &mut out)).unwrap_or(false)
            };
            let frame = if rendered { &out } else { &src };
            if frame.is_empty() || frame.rgba.len() != (frame.width * frame.height * 4) as usize {
                continue;
            }
            let img =
                egui::ColorImage::from_rgba_premultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
            let tex = ctx.load_texture(format!("fxthumb_{}", kind.name()), img, egui::TextureOptions::LINEAR);
            effects_ui::set_thumbnail(kind, tex.id(), [frame.width, frame.height]);
            self.effect_thumbs.push(tex);
        }
        // the transitions catalogue previews its wipes over the same picture (painted, no GPU pass)
        let img = egui::ColorImage::from_rgba_premultiplied([src.width as usize, src.height as usize], &src.rgba);
        transitions_ui::set_stock(ctx.load_texture("tr_stock", img, egui::TextureOptions::LINEAR));
        self.effect_thumbs_key = Some(key);
    }
}
