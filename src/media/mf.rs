//! Windows Media Foundation backend (IMFSourceReader). Native software decode, no external deps, instant seeks.
//!
//! Contract (see media/mod.rs):
//!  * `probe(path)`      -> Asset with duration, size, fps, every audio stream (language/title if available).
//!  * `open_video(path)` -> VideoSource producing top-down RGBA8 at the requested size (RGB32 output type +
//!                          MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING; handle negative stride / bottom-up;
//!                          scale to (w,h) — either via the reader's output type or a simple resize in Rust).
//!  * `open_audio(path, stream)` -> AudioSource producing stereo f32 @ 48 kHz (PCM float output type; stream
//!                          = Nth audio stream in container order; resample/upmix in Rust if MF refuses).
//! Seeking: SetCurrentPosition then decode forward until pts >= t. Sequential reads must not seek.
//! COM: MFStartup / CoInitializeEx per thread (decoders live on the thread that created them; they are `Send`
//! in the sense that a thread creates and owns them — mark types `unsafe impl Send` if needed, see ARCHITECTURE.md).

use super::{AudioSource, Frame, VideoSource, CHANNELS, SAMPLE_RATE};
use crate::model::{Asset, AudioStreamInfo, ClipKind};
use std::sync::OnceLock;
use windows::core::{Interface, BOOL, GUID, HSTRING};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// 100 ns ticks per second (MF time base).
const HNS: f64 = 1e7;
const GUID_NULL: GUID = GUID::zeroed();
/// Video: read forward (no seek) when the target is less than this far ahead of the cached frame.
const VIDEO_FWD_HNS: i64 = 15_000_000;
/// Audio: read forward (no seek) when the target is less than this many output frames past the FIFO.
const AUDIO_FWD_FRAMES: i64 = SAMPLE_RATE as i64 / 2;
/// Audio: seek this many output frames early (imprecise MP3 seeks), discard by timestamp.
const AUDIO_PREROLL_FRAMES: i64 = SAMPLE_RATE as i64 / 4;
/// Safety cap on ReadSample calls per request (no busy loops on a misbehaving source).
const MAX_READS: u32 = 4096;

fn err(e: windows::core::Error) -> String {
    format!("MF: {e}")
}

fn hns(t: f64) -> i64 {
    (t.max(0.0) * HNS).round() as i64
}

/// Process-wide MFStartup + per-thread COM init. Cheap; called on every open.
fn init() -> Result<(), String> {
    static MF: OnceLock<Result<(), String>> = OnceLock::new();
    // ponytail: CoInitializeEx on every open, never CoUninitialize — S_FALSE / RPC_E_CHANGED_MODE are fine,
    // the refcount just grows for the process lifetime.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    MF.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(err) }).clone()
}

fn open_reader(path: &str) -> Result<IMFSourceReader, String> {
    if super::is_image_path(path) {
        return Err("MF: images are decoded by ffmpeg".into());
    }
    // ponytail: MF's MPEG-2 source seeks to the *next* keyframe (EOF past the last one, verified) and reports
    // a short duration; ffmpeg seeks TS/PS exactly, so let Auto fall through to it.
    if matches!(super::ext(path).as_str(), "ts" | "m2ts" | "mts" | "m2t" | "mpg" | "mpeg" | "vob") {
        return Err("MF: MPEG-TS/PS seeks are keyframe-coarse, use ffmpeg".into());
    }
    init()?;
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1).map_err(err)?;
        let attrs = attrs.ok_or("MF: MFCreateAttributes returned null")?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1).map_err(err)?;
        // The Matroska source reports half the real frame rate; without this the video processor
        // frame-rate-converts to it and drops every second MKV/WebM frame.
        attrs.SetUINT32(&MF_XVP_DISABLE_FRC, 1).map_err(err)?;
        let url = HSTRING::from(path);
        let reader = MFCreateSourceReaderFromURL(&url, &attrs).map_err(err)?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).map_err(err)?;
        Ok(reader)
    }
}

struct StreamInfo {
    /// Source reader stream index.
    index: u32,
    major: GUID,
    ty: IMFMediaType,
    language: String,
    title: String,
}

/// Every stream in container order (ffmpeg's 0:v:N / 0:a:N). Language/title are best effort via the
/// presentation descriptor (descriptor index == reader stream index).
fn streams(reader: &IMFSourceReader) -> Vec<StreamInfo> {
    let mut v = Vec::new();
    for i in 0..64u32 {
        let Ok(ty) = (unsafe { reader.GetNativeMediaType(i, 0) }) else {
            break;
        };
        let major = unsafe { ty.GetGUID(&MF_MT_MAJOR_TYPE) }.unwrap_or(GUID_NULL);
        v.push(StreamInfo { index: i, major, ty, language: String::new(), title: String::new() });
    }
    unsafe {
        let mut p = std::ptr::null_mut();
        let r =
            reader.GetServiceForStream(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &GUID_NULL, &IMFMediaSource::IID, &mut p);
        if r.is_ok() && !p.is_null() {
            let src = IMFMediaSource::from_raw(p);
            if let Ok(pd) = src.CreatePresentationDescriptor() {
                for s in v.iter_mut() {
                    let mut sel = BOOL(0);
                    let mut sd = None;
                    let _ = pd.GetStreamDescriptorByIndex(s.index, &mut sel, &mut sd);
                    if let Some(sd) = sd {
                        s.language = get_string(&sd, &MF_SD_LANGUAGE);
                        s.title = get_string(&sd, &MF_SD_STREAM_NAME);
                    }
                }
            }
        }
    }
    // ponytail: MF's MPEG-4 source enumerates tracks in reverse `trak` order (verified); MKV/ASF/TS/AVI
    // sources are in container order. Reverse to match ffmpeg.
    if is_mp4(reader) {
        v.reverse();
    }
    v
}

/// MF's MPEG-4 source (mp4/mov/m4a: MIME */mp4, video/quicktime).
fn is_mp4(reader: &IMFSourceReader) -> bool {
    let mime = unsafe { reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_MIME_TYPE) }
        .map(|pv| pv.to_string())
        .unwrap_or_default();
    mime.ends_with("/mp4") || mime == "video/quicktime"
}

fn duration_secs(reader: &IMFSourceReader) -> f64 {
    unsafe { reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION) }
        .ok()
        .and_then(|pv| u64::try_from(&pv).ok())
        .map(|d| d as f64 / HNS)
        .unwrap_or(0.0)
}

fn get_string(a: &IMFAttributes, key: &GUID) -> String {
    unsafe {
        let Ok(len) = a.GetStringLength(key) else {
            return String::new();
        };
        let mut buf = vec![0u16; len as usize + 1];
        if a.GetString(key, &mut buf, None).is_err() {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// MF subtype -> ffmpeg-style codec name (cheap; "" when unknown).
fn codec_name(sub: GUID) -> &'static str {
    const CODECS: &[(GUID, &str)] = &[
        (MFVideoFormat_H264, "h264"),
        (MFVideoFormat_HEVC, "hevc"),
        (MFVideoFormat_H265, "hevc"),
        (MFVideoFormat_VP90, "vp9"),
        (MFVideoFormat_VP80, "vp8"),
        (MFVideoFormat_AV1, "av1"),
        (MFVideoFormat_MPEG2, "mpeg2video"),
        (MFVideoFormat_MP4V, "mpeg4"),
        (MFVideoFormat_WMV3, "wmv3"),
        (MFVideoFormat_MJPG, "mjpeg"),
        (MFAudioFormat_AAC, "aac"),
        (MFAudioFormat_MP3, "mp3"),
        (MFAudioFormat_Opus, "opus"),
        (MFAudioFormat_FLAC, "flac"),
        (MFAudioFormat_Vorbis, "vorbis"),
        (MFAudioFormat_ALAC, "alac"),
        (MFAudioFormat_Dolby_AC3, "ac3"),
        (MFAudioFormat_Dolby_DDPlus, "eac3"),
        (MFAudioFormat_WMAudioV8, "wmav2"),
        (MFAudioFormat_PCM, "pcm"),
        (MFAudioFormat_Float, "pcm"),
    ];
    CODECS.iter().find(|(g, _)| *g == sub).map_or("", |(_, n)| n)
}

pub fn probe(path: &str) -> Result<Asset, String> {
    let reader = open_reader(path)?;
    let mut asset = Asset {
        id: 0,
        path: path.to_string(),
        kind: ClipKind::Audio,
        duration: duration_secs(&reader),
        width: 0,
        height: 0,
        fps: 0.0,
        audio_streams: Vec::new(),
        codec: String::new(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
    };
    let mut has_video = false;
    for s in streams(&reader) {
        let sub = unsafe { s.ty.GetGUID(&MF_MT_SUBTYPE) }.unwrap_or(GUID_NULL);
        if s.major == MFMediaType_Video && !has_video {
            has_video = true;
            let size = unsafe { s.ty.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap_or(0);
            asset.width = (size >> 32) as u32;
            asset.height = size as u32;
            // The reader auto-rotates (ADVANCED_VIDEO_PROCESSING), so report display dims like ffmpeg does.
            if unsafe { s.ty.GetUINT32(&MF_MT_VIDEO_ROTATION) }.unwrap_or(0) % 180 == 90 {
                std::mem::swap(&mut asset.width, &mut asset.height);
            }
            asset.fps = frame_rate(&s.ty);
            asset.codec = codec_name(sub).into();
        } else if s.major == MFMediaType_Audio {
            asset.audio_streams.push(AudioStreamInfo {
                index: asset.audio_streams.len(),
                channels: unsafe { s.ty.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }.unwrap_or(0),
                sample_rate: unsafe { s.ty.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }.unwrap_or(0),
                language: s.language,
                title: s.title,
                codec: codec_name(sub).into(),
            });
        }
    }
    if has_video {
        asset.kind = ClipKind::Video;
    } else if let Some(a) = asset.audio_streams.first() {
        asset.codec = a.codec.clone();
    } else {
        return Err("MF: no audio or video streams".into());
    }
    Ok(asset)
}

fn frame_rate(ty: &IMFMediaType) -> f64 {
    let r = unsafe { ty.GetUINT64(&MF_MT_FRAME_RATE) }.unwrap_or(0);
    let (num, den) = ((r >> 32) as u32, r as u32);
    if num > 0 && den > 0 {
        num as f64 / den as f64
    } else {
        0.0
    }
}

// ---------------------------------------------------------------- video

struct MfVideo {
    reader: IMFSourceReader,
    stream: u32,
    width: u32,
    height: u32,
    /// MF_MT_DEFAULT_STRIDE of the current type (0 = unknown; negative = bottom-up).
    stride: i32,
    /// Fallback frame duration when a sample has none.
    frame_hns: i64,
    /// Native-size top-down RGBA of the cached frame.
    native: Vec<u8>,
    have: bool,
    /// Cached frame covers [pts, end) (pts may be pulled back to the requested t when the reader
    /// returned a later frame); `rpts` is the real sample time.
    pts: i64,
    end: i64,
    rpts: i64,
    /// End of the last sample seen from the reader since the last seek.
    last_end: Option<i64>,
    /// Where the stream ends (known once EOF was hit).
    eof_at: Option<i64>,
    /// MF time of the first frame = content t=0. MF keeps container time (mp4 initial empty edit, MKV
    /// offset), ffmpeg/ffprobe are content-relative; requests are shifted into MF time.
    origin: i64,
    /// MPEG-4 source: SetCurrentPosition takes content-relative time while samples carry container time.
    mp4: bool,
    // Scaled copy of the cached frame (valid when svalid && size matches).
    scaled: Vec<u8>,
    svalid: bool,
    sw: u32,
    sh: u32,
    // Box-filter tables: source x/y edges for the (native, scaled) size pair in `tab_key`.
    xs: Vec<u32>,
    ys: Vec<u32>,
    acc: Vec<u32>,
    tab_key: (u32, u32, u32, u32),
}

// SAFETY: the source reader is created on and then owned by exactly one thread at a time (DecoderPool
// per render/audio/waveform thread); MF reader objects are free-threaded and are never used concurrently.
unsafe impl Send for MfVideo {}

pub fn open_video(path: &str) -> Result<Box<dyn VideoSource>, String> {
    let reader = open_reader(path)?;
    let vs = streams(&reader).into_iter().find(|s| s.major == MFMediaType_Video).ok_or("MF: no video stream")?;
    let (stream, nat) = (vs.index, vs.ty);
    let mp4 = is_mp4(&reader);
    unsafe {
        reader.SetStreamSelection(stream, true).map_err(err)?;
        let mt = MFCreateMediaType().map_err(err)?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(err)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(err)?;
        reader.SetCurrentMediaType(stream, None, &mt).map_err(err)?;
    }
    let fps = frame_rate(&nat);
    let mut v = MfVideo {
        reader,
        stream,
        width: 0,
        height: 0,
        stride: 0,
        frame_hns: if fps > 0.0 { (HNS / fps) as i64 } else { 333_333 },
        native: Vec::new(),
        have: false,
        pts: 0,
        end: 0,
        rpts: 0,
        last_end: None,
        eof_at: None,
        origin: 0,
        mp4,
        scaled: Vec::new(),
        svalid: false,
        sw: 0,
        sh: 0,
        xs: Vec::new(),
        ys: Vec::new(),
        acc: Vec::new(),
        tab_key: (0, 0, 0, 0),
    };
    if !v.refresh_type() {
        return Err("MF: cannot read video output type".into());
    }
    // Decode the first frame now: surfaces "codec not decodable" at open (so Auto falls back to ffmpeg),
    // fixes the time origin and warms the cache for the usual first request at t=0.
    if !v.read_until(0) {
        return Err("MF: cannot decode first video frame".into());
    }
    v.origin = v.rpts.max(0);
    v.pts = v.rpts;
    Ok(Box::new(v))
}

impl MfVideo {
    /// Re-read size/stride from the current output type (after open or CURRENTMEDIATYPECHANGED).
    fn refresh_type(&mut self) -> bool {
        let Ok(cur) = (unsafe { self.reader.GetCurrentMediaType(self.stream) }) else {
            return false;
        };
        let size = unsafe { cur.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap_or(0);
        let (w, h) = ((size >> 32) as u32, size as u32);
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return false;
        }
        self.width = w;
        self.height = h;
        self.stride = unsafe { cur.GetUINT32(&MF_MT_DEFAULT_STRIDE) }.map(|s| s as i32).unwrap_or(0);
        self.native.resize((w * h * 4) as usize, 0);
        self.have = false;
        self.svalid = false;
        true
    }

    fn seek(&mut self, tt: i64) -> bool {
        let pv = PROPVARIANT::from(if self.mp4 { tt - self.origin } else { tt });
        let ok = unsafe { self.reader.SetCurrentPosition(&GUID_NULL, &pv) }.is_ok();
        self.have = false;
        self.last_end = None;
        ok
    }

    /// Read forward until a sample whose end is past `tt`; cache it. False at EOF / error.
    fn read_until(&mut self, tt: i64) -> bool {
        for _ in 0..MAX_READS {
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            let r = unsafe {
                self.reader.ReadSample(self.stream, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))
            };
            if r.is_err() || flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                return false;
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 && !self.refresh_type() {
                return false;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                let e = self.last_end.unwrap_or(tt);
                self.eof_at = Some(self.eof_at.map_or(e, |x| x.min(e)));
                return false;
            }
            let Some(sample) = sample else { continue }; // stream tick / gap
            let dur = unsafe { sample.GetSampleDuration() }.unwrap_or(0);
            let end = ts + if dur > 0 { dur } else { self.frame_hns };
            self.last_end = Some(end);
            if end <= tt {
                continue;
            }
            if !self.copy_sample(&sample) {
                return false;
            }
            self.have = true;
            self.rpts = ts;
            self.pts = ts.min(tt);
            self.end = end;
            self.svalid = false;
            return true;
        }
        false
    }

    /// BGRX sample -> top-down RGBA `native`.
    fn copy_sample(&mut self, sample: &IMFSample) -> bool {
        let (w, h) = (self.width as usize, self.height as usize);
        unsafe {
            let Ok(buf) = sample.ConvertToContiguousBuffer() else {
                return false;
            };
            if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                let mut p: *mut u8 = std::ptr::null_mut();
                let mut pitch = 0i32;
                if b2.Lock2D(&mut p, &mut pitch).is_err() || p.is_null() {
                    return false;
                }
                // Lock2D: p = top row, pitch signed.
                convert_rows(p, pitch as isize, w, h, &mut self.native);
                let _ = b2.Unlock2D();
            } else {
                let mut p: *mut u8 = std::ptr::null_mut();
                let mut len = 0u32;
                if buf.Lock(&mut p, None, Some(&mut len)).is_err() || p.is_null() {
                    return false;
                }
                let stride = if self.stride != 0 { self.stride as isize } else { (w * 4) as isize };
                if (len as usize) < h * stride.unsigned_abs() {
                    let _ = buf.Unlock();
                    return false;
                }
                // Negative stride: memory starts with the bottom row, so the top row is the last one.
                let top = if stride < 0 { p.offset((h as isize - 1) * -stride) } else { p };
                convert_rows(top, stride, w, h, &mut self.native);
                let _ = buf.Unlock();
            }
        }
        true
    }

    /// Write the cached frame at (w,h) into `out`.
    fn emit(&mut self, w: u32, h: u32, out: &mut Frame) {
        out.resize(w, h);
        out.pts = (self.rpts - self.origin) as f64 / HNS;
        if w == self.width && h == self.height {
            out.rgba.copy_from_slice(&self.native);
            return;
        }
        if !(self.svalid && self.sw == w && self.sh == h) {
            self.rescale(w, h);
        }
        out.rgba.copy_from_slice(&self.scaled);
    }

    fn rescale(&mut self, w: u32, h: u32) {
        self.scaled.resize((w * h * 4) as usize, 0);
        if w <= self.width && h <= self.height {
            let key = (self.width, self.height, w, h);
            if self.tab_key != key {
                self.tab_key = key;
                self.xs = (0..=w).map(|i| (i as u64 * self.width as u64 / w as u64) as u32).collect();
                self.ys = (0..=h).map(|j| (j as u64 * self.height as u64 / h as u64) as u32).collect();
                self.acc.resize(w as usize * 3, 0);
            }
            box_down(&self.native, self.width, &self.xs, &self.ys, &mut self.acc, &mut self.scaled);
        } else {
            // ponytail: bilinear per pixel — the compositor never asks for more than native size.
            bilinear(&self.native, self.width, self.height, w, h, &mut self.scaled);
        }
        self.svalid = true;
        self.sw = w;
        self.sh = h;
    }
}

/// Copy `h` rows of `w` BGRX pixels starting at `top` (row stride `pitch`, signed) into top-down RGBA.
unsafe fn convert_rows(top: *const u8, pitch: isize, w: usize, h: usize, dst: &mut [u8]) {
    for (y, drow) in dst.chunks_exact_mut(w * 4).take(h).enumerate() {
        let src = std::slice::from_raw_parts(top.offset(y as isize * pitch), w * 4);
        for (s, d) in src.chunks_exact(4).zip(drow.chunks_exact_mut(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
    }
}

/// Area-average downscale. `xs`/`ys` are the source edges per destination column/row (len = n+1).
fn box_down(src: &[u8], sw: u32, xs: &[u32], ys: &[u32], acc: &mut [u32], dst: &mut [u8]) {
    let (sw, w, h) = (sw as usize, xs.len() - 1, ys.len() - 1);
    for (j, drow) in dst.chunks_exact_mut(w * 4).take(h).enumerate() {
        acc.fill(0);
        let (y0, y1) = (ys[j] as usize, ys[j + 1] as usize);
        for row in src[y0 * sw * 4..y1 * sw * 4].chunks_exact(sw * 4) {
            for (i, a) in acc.chunks_exact_mut(3).enumerate() {
                let (x0, x1) = (xs[i] as usize, xs[i + 1] as usize);
                for px in row[x0 * 4..x1 * 4].chunks_exact(4) {
                    a[0] += px[0] as u32;
                    a[1] += px[1] as u32;
                    a[2] += px[2] as u32;
                }
            }
        }
        let rows = (y1 - y0) as u32;
        for (i, (d, a)) in drow.chunks_exact_mut(4).zip(acc.chunks_exact(3)).enumerate() {
            let n = (rows * (xs[i + 1] - xs[i])).max(1);
            d[0] = (a[0] / n) as u8;
            d[1] = (a[1] / n) as u8;
            d[2] = (a[2] / n) as u8;
            d[3] = 255;
        }
    }
}

fn bilinear(src: &[u8], sw: u32, sh: u32, w: u32, h: u32, dst: &mut [u8]) {
    let (sw, sh) = (sw as usize, sh as usize);
    let px = |x: usize, y: usize| &src[(y * sw + x) * 4..(y * sw + x) * 4 + 4];
    for (j, drow) in dst.chunks_exact_mut(w as usize * 4).enumerate() {
        let fy = ((j as f32 + 0.5) * sh as f32 / h as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, wy) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for (i, d) in drow.chunks_exact_mut(4).enumerate() {
            let fx = ((i as f32 + 0.5) * sw as f32 / w as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, wx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            let (a, b, c, e) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
            for k in 0..3 {
                let top = a[k] as f32 + (b[k] as f32 - a[k] as f32) * wx;
                let bot = c[k] as f32 + (e[k] as f32 - c[k] as f32) * wx;
                d[k] = (top + (bot - top) * wy + 0.5) as u8;
            }
            d[3] = 255;
        }
    }
}

impl VideoSource for MfVideo {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        if w == 0 || h == 0 {
            return false;
        }
        // +5 ms display-time tolerance: MKV pts are ms-rounded (frame n's pts can land just after n/fps),
        // raw mp4 pts jitter ±1 hns — without it every other frame repeats when stepping at n/fps.
        let tt = hns(t) + 50_000 + self.origin;
        if self.eof_at.is_some_and(|e| tt >= e) {
            return false;
        }
        if !(self.have && tt >= self.pts && tt < self.end) {
            let forward = self.have && tt >= self.end && tt < self.pts + VIDEO_FWD_HNS;
            if (!forward && !self.seek(tt)) || !self.read_until(tt) {
                return false;
            }
        }
        self.emit(w, h, out);
        true
    }
}

// ---------------------------------------------------------------- audio

/// Linear resampler state carried across decoded buffers.
#[derive(Default)]
struct Resamp {
    /// Position in "virtual source" frames where index 0 is `last` and 1.. are the buffer's frames.
    pos: f64,
    last: [f32; 2],
    primed: bool,
}

/// Append `src` (interleaved `ch` channels at `rate`) to `fifo` as interleaved stereo @ SAMPLE_RATE.
/// Mono is duplicated, >2 channels keep L/R. Non-48k input is linearly resampled.
fn push_audio(src: &[f32], ch: usize, rate: u32, rs: &mut Resamp, fifo: &mut Vec<f32>) {
    if ch == 0 || src.len() < ch {
        return;
    }
    let n = src.len() / ch;
    let rc = 1.min(ch - 1);
    let frame = |k: usize| [src[k * ch], src[k * ch + rc]];
    if rate == SAMPLE_RATE {
        fifo.reserve(n * 2);
        for k in 0..n {
            fifo.extend_from_slice(&frame(k));
        }
        return;
    }
    if !rs.primed {
        *rs = Resamp { pos: 1.0, last: frame(0), primed: true };
    }
    // ponytail: linear interpolation — a windowed-sinc resampler if aliasing ever matters.
    let step = rate as f64 / SAMPLE_RATE as f64;
    let v = |k: usize| if k == 0 { rs.last } else { frame(k - 1) };
    let mut pos = rs.pos;
    while pos < n as f64 {
        let k = pos as usize;
        let f = (pos - k as f64) as f32;
        let (a, b) = (v(k), v(k + 1));
        fifo.push(a[0] + (b[0] - a[0]) * f);
        fifo.push(a[1] + (b[1] - a[1]) * f);
        pos += step;
    }
    rs.pos = pos - n as f64;
    rs.last = frame(n - 1);
}

struct MfAudio {
    reader: IMFSourceReader,
    stream: u32,
    duration: f64,
    /// Format the reader delivers (48000/2 when MF converts for us).
    rate: u32,
    ch: u32,
    /// Decoded stereo @ SAMPLE_RATE; `fifo_pos` = output frame index of fifo[0] (valid when `located`).
    fifo: Vec<f32>,
    fifo_pos: i64,
    located: bool,
    /// Output frame index where the data ends (known once EOF was hit).
    eof_at: Option<i64>,
    /// Output frame index of the first sample = content t=0 (see MfVideo::origin).
    origin: i64,
    mp4: bool,
    rs: Resamp,
    tmp: Vec<f32>,
}

// SAFETY: see MfVideo — owned and used by one thread at a time.
unsafe impl Send for MfAudio {}

pub fn open_audio(path: &str, stream: usize) -> Result<Box<dyn AudioSource>, String> {
    let reader = open_reader(path)?;
    let s = streams(&reader)
        .into_iter()
        .filter(|s| s.major == MFMediaType_Audio)
        .nth(stream)
        .ok_or_else(|| format!("MF: no audio stream {stream}"))?;
    // ponytail: MF's FLAC decoder mis-timestamps (drifts 60-120 ms after seeks in .flac, a block late in
    // MKV/OGG, +96 ms in MP4 'fLaC'); ffmpeg seeks FLAC exactly, so let Auto fall through to it.
    const FLAC_MP4: GUID = GUID { data1: 0x664C_6143, ..MFMPEG4Format_Base }; // 'fLaC'
    let sub = unsafe { s.ty.GetGUID(&MF_MT_SUBTYPE) }.unwrap_or(GUID_NULL);
    if sub == MFAudioFormat_FLAC || sub == FLAC_MP4 {
        return Err("MF: FLAC timestamps unreliable, use ffmpeg".into());
    }
    let idx = s.index;
    let mp4 = is_mp4(&reader);
    let duration = duration_secs(&reader);
    let (rate, ch) = unsafe {
        reader.SetStreamSelection(idx, true).map_err(err)?;
        let full = MFCreateMediaType().map_err(err)?;
        full.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
        full.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).map_err(err)?;
        full.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 32).map_err(err)?;
        full.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32).map_err(err)?;
        full.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, SAMPLE_RATE).map_err(err)?;
        full.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4 * CHANNELS as u32).map_err(err)?;
        full.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 4 * CHANNELS as u32 * SAMPLE_RATE).map_err(err)?;
        if reader.SetCurrentMediaType(idx, None, &full).is_err() {
            // MF won't convert rate/channels: take native rate/channels as float, convert in Rust.
            let mt = MFCreateMediaType().map_err(err)?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).map_err(err)?;
            reader.SetCurrentMediaType(idx, None, &mt).map_err(err)?;
        }
        audio_format(&reader, idx)?
    };
    let mut a = MfAudio {
        reader,
        stream: idx,
        duration,
        rate,
        ch,
        fifo: Vec::new(),
        fifo_pos: 0,
        located: false,
        eof_at: None,
        origin: 0,
        mp4,
        rs: Resamp::default(),
        tmp: Vec::new(),
    };
    // Decode the first sample now: surfaces "not decodable" at open (so Auto falls back to ffmpeg),
    // fixes the time origin and warms the FIFO for the usual first read at t=0.
    let mut reads = 0;
    while !a.located {
        reads += 1;
        if reads > MAX_READS || !a.decode_more(0) {
            return Err("MF: cannot decode first audio sample".into());
        }
    }
    a.origin = a.fifo_pos;
    Ok(Box::new(a))
}

/// (sample rate, channels) of the reader's current output type.
fn audio_format(reader: &IMFSourceReader, stream: u32) -> Result<(u32, u32), String> {
    let cur = unsafe { reader.GetCurrentMediaType(stream) }.map_err(err)?;
    let rate = unsafe { cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }.map_err(err)?;
    let ch = unsafe { cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }.map_err(err)?;
    if rate == 0 || ch == 0 || ch > 64 {
        return Err(format!("MF: bad audio format {rate} Hz / {ch} ch"));
    }
    Ok((rate, ch))
}

impl MfAudio {
    fn fifo_end(&self) -> i64 {
        self.fifo_pos + (self.fifo.len() / CHANNELS) as i64
    }

    fn seek(&mut self, f0: i64) -> bool {
        // ponytail: MF's MP3 source lands ~60 ms *after* the requested position; pre-roll and let the
        // timestamp-based locate discard the excess (audio decode is cheap).
        let f = f0 - if self.mp4 { self.origin } else { 0 } - AUDIO_PREROLL_FRAMES;
        let pv = PROPVARIANT::from(f.max(0) * HNS as i64 / SAMPLE_RATE as i64);
        let ok = unsafe { self.reader.SetCurrentPosition(&GUID_NULL, &pv) }.is_ok();
        self.fifo.clear();
        self.located = false;
        self.rs.primed = false;
        ok
    }

    fn mark_eof(&mut self, f0: i64) {
        let e = if self.located { self.fifo_end() } else { f0 };
        self.eof_at = Some(self.eof_at.map_or(e, |x| x.min(e)));
    }

    /// Decode one more sample into the FIFO. False at EOF / error.
    fn decode_more(&mut self, f0: i64) -> bool {
        let mut flags = 0u32;
        let mut ts = 0i64;
        let mut sample = None;
        let r =
            unsafe { self.reader.ReadSample(self.stream, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)) };
        if r.is_err() || flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_ENDOFSTREAM.0) as u32 != 0 {
            // ponytail: read errors are treated as end of stream (no retry storm at audio rate).
            self.mark_eof(f0);
            return false;
        }
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
            match audio_format(&self.reader, self.stream) {
                Ok((rate, ch)) => {
                    self.rate = rate;
                    self.ch = ch;
                    self.rs.primed = false;
                }
                Err(_) => {
                    self.mark_eof(f0);
                    return false;
                }
            }
        }
        let Some(sample) = sample else { return true }; // stream tick
        unsafe {
            let Ok(buf) = sample.ConvertToContiguousBuffer() else {
                return true;
            };
            let mut p: *mut u8 = std::ptr::null_mut();
            let mut len = 0u32;
            if buf.Lock(&mut p, None, Some(&mut len)).is_err() || p.is_null() {
                return true;
            }
            let bytes = std::slice::from_raw_parts(p, len as usize);
            self.tmp.clear();
            self.tmp.extend(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            let _ = buf.Unlock();
        }
        if !self.located {
            self.fifo_pos = (ts as f64 * SAMPLE_RATE as f64 / HNS).round() as i64;
            self.located = true;
        }
        push_audio(&self.tmp, self.ch as usize, self.rate, &mut self.rs, &mut self.fifo);
        true
    }
}

impl AudioSource for MfAudio {
    fn duration(&self) -> f64 {
        self.duration
    }
    fn read_at(&mut self, t: f64, out: &mut [f32]) {
        out.fill(0.0);
        let n = (out.len() / CHANNELS) as i64;
        let f0 = (t.max(0.0) * SAMPLE_RATE as f64).round() as i64 + self.origin;
        let f1 = f0 + n;
        if n == 0 || self.eof_at.is_some_and(|e| f0 >= e) {
            return;
        }
        if (!self.located || f0 < self.fifo_pos || f0 > self.fifo_end() + AUDIO_FWD_FRAMES) && !self.seek(f0) {
            return;
        }
        let mut reads = 0;
        while !(self.located && self.fifo_end() >= f1) && reads < MAX_READS {
            reads += 1;
            if !self.decode_more(f0) {
                break;
            }
        }
        if !self.located {
            return;
        }
        let (a, b) = (f0.max(self.fifo_pos), f1.min(self.fifo_end()));
        if b > a {
            let so = ((a - self.fifo_pos) as usize) * CHANNELS;
            let len = ((b - a) as usize) * CHANNELS;
            let doff = ((a - f0) as usize) * CHANNELS;
            out[doff..doff + len].copy_from_slice(&self.fifo[so..so + len]);
        }
        // Drop everything before this block (a repeat read of the same t stays free).
        let drop = ((f0 - self.fifo_pos).max(0) as usize).min(self.fifo.len() / CHANNELS);
        if drop > 0 {
            self.fifo.drain(..drop * CHANNELS);
            self.fifo_pos += drop as i64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Instant;

    /// red 0–2 s (bottom half blue), green 2–4 s; audio 0 = 440 Hz "eng", audio 1 = 880 Hz "Music".
    fn media() -> Option<String> {
        static P: OnceLock<Option<String>> = OnceLock::new();
        P.get_or_init(|| {
            let dir = std::env::temp_dir().join("simple-editor-mf-test");
            std::fs::create_dir_all(&dir).ok()?;
            let out: PathBuf = dir.join("test.mp4");
            let st = Command::new("ffmpeg")
                .args(["-v", "error", "-y"])
                .args(["-f", "lavfi", "-i", "color=red:s=320x240:d=2,drawbox=y=120:h=120:color=blue:t=fill"])
                .args(["-f", "lavfi", "-i", "color=lime:s=320x240:d=2"])
                .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4,volume=4"])
                .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=4,volume=4"])
                .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1[v]", "-map", "[v]", "-map", "2:a", "-map", "3:a"])
                .args(["-r", "30", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-c:a", "aac"])
                .args(["-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Music"])
                .arg(&out)
                .status()
                .ok()?;
            st.success().then(|| out.to_string_lossy().into_owned())
        })
        .clone()
    }

    /// `ffmpeg <pre> -i test.mp4 <post> <name>` next to the test clip.
    fn derive(name: &str, pre: &[&str], post: &[&str]) -> Option<String> {
        let src = media()?;
        let out = PathBuf::from(&src).with_file_name(format!("{}-{name}", std::process::id()));
        let st = Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(pre)
            .args(["-i", &src])
            .args(post)
            .arg(&out)
            .status()
            .ok()?;
        st.success().then(|| out.to_string_lossy().into_owned())
    }

    fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * f.width + x) * 4) as usize;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }
    fn is_red(p: [u8; 4]) -> bool {
        p[0] > 200 && p[1] < 70 && p[2] < 70
    }
    fn is_green(p: [u8; 4]) -> bool {
        p[0] < 70 && p[1] > 200 && p[2] < 70
    }
    fn is_blue(p: [u8; 4]) -> bool {
        p[0] < 70 && p[1] < 70 && p[2] > 200
    }
    fn zero_crossings(stereo: &[f32]) -> usize {
        let l: Vec<f32> = stereo.chunks_exact(2).map(|c| c[0]).collect();
        l.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }
    fn rms(s: &[f32]) -> f64 {
        (s.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / s.len().max(1) as f64).sqrt()
    }

    #[test]
    fn probe_reports_streams() {
        let Some(p) = media() else {
            eprintln!("ffmpeg missing; skipped");
            return;
        };
        let a = probe(&p).expect("probe");
        assert_eq!(a.kind, ClipKind::Video);
        assert!((a.duration - 4.0).abs() < 0.1, "duration {}", a.duration);
        assert_eq!((a.width, a.height), (320, 240));
        assert!((a.fps - 30.0).abs() < 0.05, "fps {}", a.fps);
        assert_eq!(a.audio_streams.len(), 2);
        assert_eq!(a.audio_streams[0].index, 0);
        assert_eq!(a.audio_streams[1].index, 1);
        assert_eq!(a.codec, "h264");
        // metadata is best effort (MF exposes "en" / "Music" for this mp4 on Win10+): log, don't fail
        eprintln!("audio streams: {:?}", a.audio_streams);
        assert_eq!(a.audio_streams[0].channels, 1);
        assert_eq!(a.audio_streams[0].sample_rate, 44100);
        assert!(probe("C:/definitely/missing.mp4").is_err());
        assert!(probe("C:/x.png").is_err());
    }

    #[test]
    fn video_frames_seek_scale() {
        let Some(p) = media() else {
            eprintln!("ffmpeg missing; skipped");
            return;
        };
        let mut v = open_video(&p).expect("open_video");
        assert_eq!(v.size(), (320, 240));
        let mut f = Frame::default();
        assert!(v.frame_at(0.5, 320, 240, &mut f));
        assert_eq!((f.width, f.height), (320, 240));
        assert!(is_red(px(&f, 10, 10)), "top-left {:?}", px(&f, 10, 10));
        assert!(is_blue(px(&f, 10, 230)), "bottom-left {:?} (orientation)", px(&f, 10, 230));
        assert!(v.frame_at(2.5, 320, 240, &mut f));
        assert!(is_green(px(&f, 160, 120)), "{:?}", px(&f, 160, 120));
        assert!(v.frame_at(0.5, 320, 240, &mut f), "seek back");
        assert!(is_red(px(&f, 10, 10)));
        assert!(v.frame_at(1.0, 160, 120, &mut f));
        assert_eq!((f.width, f.height), (160, 120));
        assert!(is_red(px(&f, 5, 5)));
        assert!(is_blue(px(&f, 5, 115)));
        assert!(v.frame_at(1.0, 400, 300, &mut f), "upscale");
        assert_eq!((f.width, f.height), (400, 300));
        assert!(is_red(px(&f, 5, 5)));
        assert!(is_blue(px(&f, 5, 295)));
        assert!(!v.frame_at(10.0, 320, 240, &mut f), "past end");
        assert!(!v.frame_at(4.5, 320, 240, &mut f), "past end (cached eof)");
        assert!(v.frame_at(3.9, 320, 240, &mut f), "near end still decodes");
        assert!(is_green(px(&f, 160, 120)));
        // sequential read timing
        let t0 = Instant::now();
        let mut n = 0;
        for i in 0..100 {
            if v.frame_at(i as f64 / 30.0, 320, 240, &mut f) {
                n += 1;
            }
        }
        let dt = t0.elapsed();
        eprintln!("100 sequential 320x240 frames: {dt:?} ({n} ok)");
        assert_eq!(n, 100);
        assert!(dt.as_secs_f64() < 1.0, "too slow: {dt:?}");
        let t0 = Instant::now();
        for _ in 0..100 {
            assert!(v.frame_at(1.5, 160, 120, &mut f));
        }
        eprintln!("100 repeated frame_at(1.5) @160x120: {:?}", t0.elapsed());
    }

    #[test]
    fn audio_streams_read() {
        let Some(p) = media() else {
            eprintln!("ffmpeg missing; skipped");
            return;
        };
        let mut a = open_audio(&p, 0).expect("open_audio 0");
        assert!((a.duration() - 4.0).abs() < 0.1);
        let mut buf = vec![0f32; 4800 * 2];
        a.read_at(1.0, &mut buf);
        let r = rms(&buf);
        assert!((0.1..=1.0).contains(&r), "rms {r}");
        // L == R (mono upmixed)
        assert!(buf.chunks_exact(2).all(|c| (c[0] - c[1]).abs() < 1e-4));
        // sequential block continues without a gap: the sine must stay continuous across the boundary
        let last = buf[buf.len() - 2];
        a.read_at(1.0 + 4800.0 / 48000.0, &mut buf);
        assert!((buf[0] - last).abs() < 0.15, "discontinuity {last} -> {}", buf[0]);
        assert!((0.1..=1.0).contains(&rms(&buf)));
        // repeat read of the same block is served from the FIFO
        let mut again = vec![0f32; 4800 * 2];
        a.read_at(1.0 + 4800.0 / 48000.0, &mut again);
        assert_eq!(buf, again);
        a.read_at(10.0, &mut buf);
        assert!(buf.iter().all(|v| *v == 0.0), "past end must be silent");
        a.read_at(0.5, &mut buf);
        assert!((0.1..=1.0).contains(&rms(&buf)), "seek back after eof");
        // straddling the end: tail zero-filled
        a.read_at(3.95, &mut buf);
        assert!(buf[buf.len() - 2..].iter().all(|v| *v == 0.0));
        // container order: stream 0 = 440 Hz (~88 zero crossings / 0.1 s), stream 1 = 880 Hz (~176)
        a.read_at(1.0, &mut buf);
        let zc0 = zero_crossings(&buf);
        let mut b = open_audio(&p, 1).expect("open_audio 1");
        b.read_at(2.0, &mut buf);
        assert!((0.1..=1.0).contains(&rms(&buf)), "stream 1 rms");
        let zc1 = zero_crossings(&buf);
        eprintln!("zero crossings: stream0={zc0} stream1={zc1}");
        assert!((80..=96).contains(&zc0) && (168..=184).contains(&zc1), "stream order {zc0} {zc1}");
        assert!(open_audio(&p, 2).is_err());
        let t0 = Instant::now();
        let mut t = 0.0;
        for _ in 0..100 {
            a.read_at(t, &mut buf);
            t += 4800.0 / 48000.0;
        }
        eprintln!("100 sequential audio blocks (10 s): {:?}", t0.elapsed());
    }

    /// Stepping at n/fps must visit every frame once (MKV: FRC disabled + ms-rounded pts tolerance).
    #[test]
    fn steps_every_frame_mkv_and_mp4() {
        let Some(mkv) = derive("v30.mkv", &[], &["-c", "copy"]) else {
            eprintln!("ffmpeg missing; skipped");
            return;
        };
        for p in [mkv, media().unwrap()] {
            let mut v = open_video(&p).expect("open_video");
            let mut f = Frame::default();
            for n in 0..120 {
                let t = n as f64 / 30.0;
                assert!(v.frame_at(t, 32, 24, &mut f), "{p} frame {n}");
                assert!((f.pts - t).abs() < 0.002, "{p} frame {n}: pts {} (dup/skip)", f.pts);
                let c = px(&f, 1, 1);
                assert!(if n < 60 { is_red(c) } else { is_green(c) }, "{p} frame {n}: {c:?}");
            }
        }
    }

    /// Display-matrix rotation: both probes report display dims, both decoders deliver rotated frames.
    #[test]
    fn rotated_video_reports_display_size() {
        let Some(p) = derive("rot90.mp4", &["-display_rotation", "90"], &["-c", "copy"]) else {
            eprintln!("ffmpeg missing; skipped");
            return;
        };
        assert_eq!(probe(&p).map(|a| (a.width, a.height)), Ok((240, 320)), "mf probe");
        assert_eq!(crate::media::ffpipe::probe(&p).map(|a| (a.width, a.height)), Ok((240, 320)), "ffprobe");
        for mut v in [open_video(&p).expect("mf"), crate::media::ffpipe::open_video(&p).expect("ffpipe")] {
            assert_eq!(v.size(), (240, 320));
            let mut f = Frame::default();
            assert!(v.frame_at(0.5, 240, 320, &mut f));
            let (l, r) = (px(&f, 10, 160), px(&f, 230, 160));
            assert!((is_red(l) && is_blue(r)) || (is_blue(l) && is_red(r)), "halves {l:?} {r:?}");
        }
    }

    /// Containers whose first timestamp is not 0 (mp4 with an initial empty edit, MKV with an offset):
    /// content time 0 = first sample, like ffmpeg/ffprobe, so the clip neither starts frozen nor loses
    /// its tail. MPEG-TS (MF seeks it to the next keyframe only) is left to ffmpeg.
    #[test]
    fn offset_start_is_content_relative() {
        let (Some(ts), Some(mp4), Some(mkv)) = (
            derive("off.ts", &[], &["-c", "copy", "-f", "mpegts"]),
            derive("off10.mp4", &[], &["-c", "copy", "-output_ts_offset", "10"]),
            derive("off10.mkv", &[], &["-c", "copy", "-output_ts_offset", "10"]),
        ) else {
            eprintln!("ffmpeg missing; skipped");
            return;
        };
        assert!(open_video(&ts).is_err() && open_audio(&ts, 0).is_err(), "TS goes to ffmpeg");
        assert!(crate::media::open_video(&ts, crate::media::Backend::Auto).is_ok());
        for p in [mp4, mkv] {
            let mut v = open_video(&p).expect("open_video");
            let mut f = Frame::default();
            for (t, green) in [(0.5, false), (2.5, true), (0.5, false), (3.9, true), (0.0, false)] {
                assert!(v.frame_at(t, 32, 24, &mut f), "{p} frame_at({t})");
                let c = px(&f, 1, 1);
                assert!(if green { is_green(c) } else { is_red(c) }, "{p} t={t}: {c:?} pts {}", f.pts);
                assert!((f.pts - t).abs() < 0.05, "{p} t={t}: pts {}", f.pts);
            }
            assert!(!v.frame_at(4.5, 32, 24, &mut f), "{p} past end");
            let mut a = open_audio(&p, 0).expect("open_audio");
            let mut buf = vec![0f32; 4800 * 2];
            for t in [0.0, 1.0, 3.8, 0.5] {
                a.read_at(t, &mut buf);
                let r = rms(&buf);
                assert!((0.1..=1.0).contains(&r), "{p} audio t={t}: rms {r}");
            }
        }
    }

    #[test]
    fn push_audio_converts() {
        // 48k stereo passthrough
        let mut fifo = Vec::new();
        push_audio(&[1.0, 2.0, 3.0, 4.0], 2, 48000, &mut Resamp::default(), &mut fifo);
        assert_eq!(fifo, [1.0, 2.0, 3.0, 4.0]);
        // mono dup + 6ch keeps L/R
        fifo.clear();
        push_audio(&[0.5], 1, 48000, &mut Resamp::default(), &mut fifo);
        push_audio(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, 48000, &mut Resamp::default(), &mut fifo);
        assert_eq!(fifo, [0.5, 0.5, 1.0, 2.0]);
        // 24k mono ramp -> 48k: interpolated midpoints, continuous across buffers
        fifo.clear();
        let mut rs = Resamp::default();
        push_audio(&[0.0, 2.0, 4.0], 1, 24000, &mut rs, &mut fifo);
        push_audio(&[6.0, 8.0], 1, 24000, &mut rs, &mut fifo);
        let l: Vec<f32> = fifo.chunks_exact(2).map(|c| c[0]).collect();
        // (the last source sample waits for the next buffer: it is only an interpolation endpoint)
        assert_eq!(l, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        // 96k -> 48k: one out per two in
        fifo.clear();
        push_audio(&[0.0, 1.0, 2.0, 3.0], 1, 96000, &mut Resamp::default(), &mut fifo);
        assert_eq!(fifo.len() / 2, 2);
    }

    #[test]
    fn scalers() {
        // 4x2 source: left half red, right half blue
        let mut src = vec![0u8; 4 * 2 * 4];
        for (i, p) in src.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(if i % 4 < 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
        }
        let xs = [0u32, 2, 4];
        let ys = [0u32, 2];
        let mut acc = vec![0u32; 6];
        let mut dst = vec![0u8; 2 * 4];
        box_down(&src, 4, &xs, &ys, &mut acc, &mut dst);
        assert_eq!(dst, [255, 0, 0, 255, 0, 0, 255, 255]);
        let mut up = vec![0u8; 8 * 4 * 4];
        bilinear(&src, 4, 2, 8, 4, &mut up);
        assert_eq!(&up[..4], &[255, 0, 0, 255]);
        assert_eq!(&up[7 * 4..8 * 4], &[0, 0, 255, 255]);
        // a BGRX row with negative pitch (bottom-up memory) lands top-down
        let rows: [[u8; 8]; 2] = [[1, 2, 3, 0, 4, 5, 6, 0], [7, 8, 9, 0, 10, 11, 12, 0]]; // mem: bottom row first
        let mut out = vec![0u8; 16];
        unsafe { convert_rows(rows[1].as_ptr(), -8, 2, 2, &mut out) };
        assert_eq!(out, [9, 8, 7, 255, 12, 11, 10, 255, 3, 2, 1, 255, 6, 5, 4, 255]);
    }
}
