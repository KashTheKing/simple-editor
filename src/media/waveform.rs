//! Audio waveform peaks per (file, audio stream): computed once on a background thread (decoding the
//! whole stream through an AudioSource), cached in memory and on disk under Settings::cache_dir()
//! (key = hash of path + file size + mtime + stream). `PEAKS_PER_SEC` buckets of mono (min, max).
//! The same decode pass also makes the timeline's `WaveDetail`: RMS + 3 band levels per bucket and a
//! log-frequency spectrogram (`SPEC_BINS` rows at `SPEC_PER_SEC`), all dB-quantised to u8.

use super::{AudioSource, Backend, SAMPLE_RATE};
use crate::model::{ClipKind, Id, Project, TrackKind};
use crate::settings::Settings;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

pub const PEAKS_PER_SEC: u32 = 100;

pub struct Peaks {
    /// Per bucket: min sample in [-1,1].
    pub min: Vec<f32>,
    /// Per bucket: max sample in [-1,1].
    pub max: Vec<f32>,
}

impl Peaks {
    pub fn len(&self) -> usize {
        self.min.len()
    }
    pub fn is_empty(&self) -> bool {
        self.min.is_empty()
    }
    /// (min, max) over source time range [a, b) seconds.
    pub fn range(&self, a: f64, b: f64) -> (f32, f32) {
        let i0 = ((a * PEAKS_PER_SEC as f64).floor().max(0.0)) as usize;
        let i1 = ((b * PEAKS_PER_SEC as f64).ceil().max(0.0)) as usize;
        let i1 = i1.min(self.len()).max(i0 + 1);
        if i0 >= self.len() {
            return (0.0, 0.0);
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in i0..i1 {
            lo = lo.min(self.min[i]);
            hi = hi.max(self.max[i]);
        }
        (lo, hi)
    }
}

/// Spectrogram rows (log-spaced 30 Hz .. 16 kHz, row 0 = lowest).
pub const SPEC_BINS: usize = 64;
/// Spectrogram columns per second (every 2nd peak bucket, max of the pair).
pub const SPEC_PER_SEC: u32 = 50;
/// dB range of every u8 level: 0 = `DB_FLOOR` or quieter, 255 = 0 dBFS.
pub const DB_FLOOR: f32 = -90.0;
/// Band edges (Hz): low < 200 <= mid < 2000 <= high.
const BAND_EDGES: (f32, f32) = (200.0, 2000.0);
/// Disk format version (the `.peaks` header). Bump to rebuild every cached file.
const CACHE_VERSION: u32 = 2;
const CACHE_MAGIC: &[u8; 4] = b"SEPK";

/// Linear amplitude -> u8 on the `DB_FLOOR..0 dB` scale.
pub fn q_db(amp: f32) -> u8 {
    let db = 20.0 * amp.max(1e-9).log10();
    ((db - DB_FLOOR) / -DB_FLOOR * 255.0).round().clamp(0.0, 255.0) as u8
}
/// Inverse of `q_db` (0 -> 0.0, not the floor amplitude).
pub fn amp_of(q: u8) -> f32 {
    if q == 0 {
        0.0
    } else {
        10f32.powf((q as f32 / 255.0 * -DB_FLOOR + DB_FLOOR) / 20.0)
    }
}

/// One timeline column's worth of audio: the envelope plus loudness and band levels (u8, `q_db`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Col {
    pub lo: f32,
    pub hi: f32,
    /// Loudness: the RMS amplitude, averaged (in power) over everything merged - so a column keeps the
    /// dips between hits instead of filling up to its loudest 10 ms.
    pub rms: f32,
    /// [low, mid, high]
    pub bands: [u8; 3],
}

impl Col {
    fn merge(self, o: Col) -> Col {
        Col {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
            rms: ((self.rms * self.rms + o.rms * o.rms) * 0.5).sqrt(),
            bands: [0, 1, 2].map(|i| self.bands[i].max(o.bands[i])),
        }
    }
}

/// What the timeline draws: per-bucket `Col`s as a pyramid (`levels[l]` merges 2^l buckets) so a pixel
/// column reads a handful of entries at any zoom, the spectrogram and the onsets.
#[derive(Default)]
pub struct WaveDetail {
    pub levels: Vec<Vec<Col>>,
    /// False for a mixed sequence (envelope only: `rms`/`bands` are 0).
    pub has_bands: bool,
    pub spec: Vec<[u8; SPEC_BINS]>,
    /// Onsets (source seconds) from the spectral flux - the detailed view's ticks.
    pub onsets: Vec<f64>,
}

impl WaveDetail {
    fn new(peaks: &Peaks, rb: Option<&[[u8; 4]]>, spec: Vec<[u8; SPEC_BINS]>) -> WaveDetail {
        let base: Vec<Col> = (0..peaks.len())
            .map(|i| {
                let x = rb.and_then(|r| r.get(i)).copied().unwrap_or_default();
                Col { lo: peaks.min[i], hi: peaks.max[i], rms: amp_of(x[0]), bands: [x[1], x[2], x[3]] }
            })
            .collect();
        let mut levels = vec![base];
        while levels.last().unwrap().len() > 1 {
            let next =
                levels.last().unwrap().chunks(2).map(|c| c.iter().copied().reduce(Col::merge).unwrap()).collect();
            levels.push(next);
        }
        let onsets = spec_onsets(&spec);
        WaveDetail { levels, has_bands: rb.is_some(), spec, onsets }
    }
    /// Pyramid level for a column spanning `buckets` peak buckets: the coarsest whose entries are at
    /// most half a column wide, so a column merges 2-5 entries and bleeds at most half a column.
    pub fn level_for(&self, buckets: f64) -> usize {
        let l = (buckets * 0.5).max(1.0).log2().floor() as usize;
        l.min(self.levels.len().saturating_sub(1))
    }
    /// Everything in source range [a, b) seconds, merged (None outside the audio).
    pub fn col(&self, a: f64, b: f64) -> Option<Col> {
        let (a, b) = (a.min(b), a.max(b));
        let per = PEAKS_PER_SEC as f64;
        let l = self.level_for((b - a) * per);
        let lv = self.levels.get(l)?;
        let i0 = ((a * per).floor().max(0.0) as usize) >> l;
        let i1 = (((b * per).ceil().max(0.0) as usize + (1 << l) - 1) >> l).max(i0 + 1).min(lv.len());
        let cols = lv.get(i0..i1)?;
        let mut c = cols.iter().copied().reduce(Col::merge)?;
        c.rms = (cols.iter().map(|c| c.rms * c.rms).sum::<f32>() / cols.len() as f32).sqrt();
        Some(c)
    }
}

/// Onset times from the spectrogram's positive spectral flux, picked by Detect Beats' own `onsets`.
/// ponytail: flux on the stored 50 Hz u8 columns (so a disk load gets the same ticks), doubled to the
/// 100 Hz the picker reads.
fn spec_onsets(spec: &[[u8; SPEC_BINS]]) -> Vec<f64> {
    let mut env = Vec::with_capacity(spec.len() * 2);
    for i in 0..spec.len() {
        let f = if i == 0 {
            0.0
        } else {
            spec[i].iter().zip(&spec[i - 1]).map(|(&a, &b)| (a as f32 - b as f32).max(0.0)).sum::<f32>() / 1000.0
        };
        env.extend([f, f]);
    }
    let dur = env.len() as f64 / PEAKS_PER_SEC as f64;
    let p = Peaks { min: env.iter().map(|v| -v).collect(), max: env };
    crate::engine::analysis::onsets(&p, 0.0, dur, 0.1, 1.5)
}

#[derive(Default)]
struct State {
    /// Finished (or failed → empty) peaks, keyed by hash of (path, stream).
    ready: HashMap<u64, Arc<Peaks>>,
    detail: HashMap<u64, Arc<WaveDetail>>,
    pending: HashSet<u64>,
}

pub struct WaveformCache {
    backend: Backend,
    ctx: eframe::egui::Context,
    state: Arc<Mutex<State>>,
    /// Mixed peaks per nested sequence, with the `seq_hash` they were built from.
    seqs: HashMap<Id, (u64, Arc<Peaks>)>,
    seq_detail: HashMap<Id, (u64, Arc<WaveDetail>)>,
    /// Spectrogram heat-map tiles: (file key, pyramid level, tile index) -> texture of `TILE` columns.
    tiles: HashMap<(u64, u8, u32), eframe::egui::TextureHandle>,
}

/// Spectrogram columns per heat-map texture tile.
pub const TILE: usize = 1024;

/// ponytail: 64-bit hash as the map key so the per-frame lookup allocates nothing - collisions are astronomically unlikely.
fn mem_key(path: &str, stream: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, stream).hash(&mut h);
    h.finish()
}

impl WaveformCache {
    pub fn new(ctx: eframe::egui::Context, backend: Backend) -> Self {
        Self {
            backend,
            ctx,
            state: Arc::default(),
            seqs: HashMap::new(),
            seq_detail: HashMap::new(),
            tiles: HashMap::new(),
        }
    }
    pub fn set_backend(&mut self, b: Backend) {
        self.backend = b;
    }
    /// Forget all in-memory peaks (a file at a known path was rewritten, e.g. Save over the original).
    /// Swaps the state so an in-flight computation finishes into the orphaned map instead of re-inserting
    /// stale peaks; unchanged files just reload from the len+mtime-keyed disk cache.
    pub fn clear(&mut self) {
        self.state = Arc::default();
        self.seqs.clear();
        self.seq_detail.clear();
        self.tiles.clear();
    }
    // ---- ws:jobs-panel ----
    /// Peaks still computing. None = a worker holds the lock right now (never waits for it).
    pub fn pending_count(&self) -> Option<usize> {
        self.state.try_lock().ok().map(|st| st.pending.len())
    }
    /// Non-blocking. Returns the peaks if available; otherwise starts computing them in the background
    /// (once) and returns None. Calls `ctx.request_repaint()` when a computation finishes.
    pub fn get(&mut self, path: &str, stream: usize) -> Option<Arc<Peaks>> {
        let key = mem_key(path, stream);
        let mut st = self.state.lock().ok()?;
        if let Some(p) = st.ready.get(&key) {
            return Some(p.clone());
        }
        if st.pending.insert(key) {
            let (state, ctx, backend, path) = (self.state.clone(), self.ctx.clone(), self.backend, path.to_string());
            std::thread::spawn(move || {
                super::bg::lower_thread();
                // ponytail: a decoder panic must leave the key resolved (as empty peaks, same as a file
                // that won't open) - otherwise it stays pending forever and callers wait for good.
                let (peaks, detail) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_or_compute(&path, stream, backend)))
                        .unwrap_or_else(|_| (Peaks { min: Vec::new(), max: Vec::new() }, WaveDetail::default()));
                if let Ok(mut st) = state.lock() {
                    st.pending.remove(&key);
                    st.ready.insert(key, Arc::new(peaks));
                    st.detail.insert(key, Arc::new(detail));
                }
                ctx.request_repaint();
            });
        }
        None
    }
    /// The timeline's view of (path, stream): like `get` (starts the analysis, never blocks).
    pub fn detail(&mut self, path: &str, stream: usize) -> Option<Arc<WaveDetail>> {
        self.get(path, stream)?;
        self.state.lock().ok()?.detail.get(&mem_key(path, stream)).cloned()
    }
    /// `detail` of a nested sequence: its mixed envelope only (no bands, no spectrogram).
    pub fn sequence_detail(&mut self, p: &Project, seq: Id) -> Option<Arc<WaveDetail>> {
        let h = seq_hash(p, seq, 0);
        if let Some((k, d)) = self.seq_detail.get(&seq) {
            if *k == h {
                return Some(d.clone());
            }
        }
        let d = Arc::new(WaveDetail::new(&*self.sequence(p, seq)?, None, Vec::new()));
        self.seq_detail.insert(seq, (h, d.clone()));
        Some(d)
    }
    /// Heat-map texture of spectrogram columns `[tile * TILE, +TILE)` at pyramid `level` (each column the
    /// max of 2^level stored columns), rows high -> low. Built once per (file, level, tile), then reused.
    /// ponytail: the whole tile map is dropped past 192 tiles (~48 MB); an LRU if browsing ever thrashes it.
    pub fn spec_tile(
        &mut self,
        path: &str,
        stream: usize,
        d: &WaveDetail,
        level: u8,
        tile: u32,
    ) -> eframe::egui::TextureId {
        use eframe::egui::{ColorImage, TextureOptions};
        let key = (mem_key(path, stream), level, tile);
        if let Some(t) = self.tiles.get(&key) {
            return t.id();
        }
        if self.tiles.len() >= 192 {
            self.tiles.clear();
        }
        let step = 1usize << level;
        let mut img = ColorImage::filled([TILE, SPEC_BINS], eframe::egui::Color32::BLACK);
        for x in 0..TILE {
            let c0 = (tile as usize * TILE + x) * step;
            let Some(cols) = d.spec.get(c0..(c0 + step).min(d.spec.len())) else { break };
            for r in 0..SPEC_BINS {
                let v = cols.iter().map(|c| c[r]).max().unwrap_or(0);
                img.pixels[(SPEC_BINS - 1 - r) * TILE + x] = heat(v);
            }
        }
        let t = self.ctx.load_texture(format!("spec{key:?}"), img, TextureOptions::LINEAR);
        let id = t.id();
        self.tiles.insert(key, t);
        id
    }
}

/// Magma-like heat palette: black -> purple -> red -> orange -> pale yellow at 255.
pub fn heat(v: u8) -> eframe::egui::Color32 {
    const STOPS: [[f32; 3]; 5] =
        [[0.0, 0.0, 4.0], [81.0, 18.0, 124.0], [183.0, 55.0, 121.0], [252.0, 137.0, 97.0], [252.0, 253.0, 191.0]];
    // the bottom third of the dB range (below -60 dB) is noise floor: keep it black
    let t = ((v as f32 / 255.0 - 0.33) / 0.67).clamp(0.0, 1.0) * 4.0;
    let i = (t.floor() as usize).min(3);
    let f = t - i as f32;
    let c = [0, 1, 2].map(|k| (STOPS[i][k] + (STOPS[i + 1][k] - STOPS[i][k]) * f) as u8);
    eframe::egui::Color32::from_rgb(c[0], c[1], c[2])
}

impl WaveformCache {
    /// Peaks of a nested sequence on its own timeline: its audio clips' peaks mapped through their clip
    /// times, overlaps merged by envelope. Rebuilt only when the sequence's audio clips change (hash check per paint);
    /// None while any source's peaks are still computing (`get` repaints when they land).
    pub fn sequence(&mut self, p: &Project, seq: Id) -> Option<Arc<Peaks>> {
        let h = seq_hash(p, seq, 0);
        if let Some((k, pk)) = self.seqs.get(&seq) {
            if *k == h {
                return Some(pk.clone());
            }
        }
        let pk = Arc::new(mix_sequence(p, seq, 0, &mut |path, s| self.get(path, s))?);
        self.seqs.insert(seq, (h, pk.clone()));
        Some(pk)
    }
}

/// Audible clips of a sequence: enabled, on unmuted audio tracks.
fn audible(p: &Project, seq: Id) -> impl Iterator<Item = &crate::model::Clip> {
    p.sequence_tracks(seq)
        .unwrap_or(&[])
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && !t.muted)
        .flat_map(|t| t.clips.iter().filter(|c| c.enabled))
}

fn seq_hash(p: &Project, seq: Id, depth: u32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in audible(p, seq).filter(|_| depth < 32) {
        (c.id, c.asset, c.audio_stream, c.reverse).hash(&mut h);
        for f in [c.start, c.duration, c.src_in, c.speed, c.freeze.unwrap_or(-1.0)] {
            f.to_bits().hash(&mut h);
        }
        if c.kind == ClipKind::Sequence {
            seq_hash(p, c.sequence, depth + 1).hash(&mut h);
        }
    }
    h.finish()
}

/// Envelope of every audible clip's (min, max) per output bucket (max, not a sum: stacked full-scale
/// tracks would clip to a solid bar). `get` supplies an asset's
/// peaks (None = not ready, which makes the whole mix None).
/// ponytail: ignores volume/fades/speed ramps - the envelope shape is what the eye needs.
pub(crate) fn mix_sequence(
    p: &Project,
    seq: Id,
    depth: u32,
    get: &mut dyn FnMut(&str, usize) -> Option<Arc<Peaks>>,
) -> Option<Peaks> {
    let end = audible(p, seq).map(|c| c.end()).fold(0.0, f64::max);
    let n = (end * PEAKS_PER_SEC as f64).ceil() as usize;
    let mut out = Peaks { min: vec![0.0; n], max: vec![0.0; n] };
    let dt = 1.0 / PEAKS_PER_SEC as f64;
    for c in audible(p, seq) {
        if c.freeze.is_some() {
            continue; // a freeze frame is silent
        }
        let src = match c.kind {
            ClipKind::Sequence if depth < 32 => mix_sequence(p, c.sequence, depth + 1, get).map(Arc::new),
            _ => match p.asset(c.asset) {
                Some(a) => get(&a.path, c.audio_stream),
                None => continue, // offline: silent, not "still loading"
            },
        }?;
        let i0 = (c.start * PEAKS_PER_SEC as f64).floor().max(0.0) as usize;
        let i1 = ((c.end() * PEAKS_PER_SEC as f64).ceil() as usize).min(n);
        for i in i0..i1 {
            let t = i as f64 * dt;
            let (a, b) = (c.src_time(t), c.src_time(t + dt));
            let (lo, hi) = src.range(a.min(b), a.max(b));
            out.min[i] = out.min[i].min(lo);
            out.max[i] = out.max[i].max(hi);
        }
    }
    Some(out)
}

fn cache_file(path: &str, stream: usize) -> Option<std::path::PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, meta.len(), mtime, stream).hash(&mut h);
    Some(Settings::cache_dir().join(format!("{:016x}.peaks", h.finish())))
}

/// Bytes of a v2 cache file: header (magic, version, buckets, spectrogram columns), then per bucket
/// (min, max) f32 LE, then per bucket [rms, low, mid, high] u8, then the spectrogram columns.
fn encode(p: &Peaks, rb: &[[u8; 4]], spec: &[[u8; SPEC_BINS]]) -> Vec<u8> {
    let mut b = Vec::with_capacity(16 + p.len() * 12 + spec.len() * SPEC_BINS);
    b.extend_from_slice(CACHE_MAGIC);
    for v in [CACHE_VERSION, p.len() as u32, spec.len() as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for (lo, hi) in p.min.iter().zip(&p.max) {
        b.extend_from_slice(&lo.to_le_bytes());
        b.extend_from_slice(&hi.to_le_bytes());
    }
    b.extend(rb.iter().flatten());
    b.extend(spec.iter().flatten());
    b
}

/// `encode`'s inverse; None for any other version (an old headerless `.peaks` included) or a short file.
fn decode(b: &[u8]) -> Option<(Peaks, Vec<[u8; 4]>, Vec<[u8; SPEC_BINS]>)> {
    let u = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize);
    if b.get(..4)? != CACHE_MAGIC || u(4)? != CACHE_VERSION as usize {
        return None;
    }
    let (n, ns) = (u(8)?, u(12)?);
    if b.len() != 16 + n * 12 + ns * SPEC_BINS {
        return None;
    }
    let mut p = Peaks { min: Vec::with_capacity(n), max: Vec::with_capacity(n) };
    for c in b[16..16 + n * 8].chunks_exact(8) {
        p.min.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        p.max.push(f32::from_le_bytes([c[4], c[5], c[6], c[7]]));
    }
    let rb = b[16 + n * 8..16 + n * 12].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    let spec = b[16 + n * 12..].chunks_exact(SPEC_BINS).map(|c| c.try_into().unwrap()).collect();
    Some((p, rb, spec))
}

/// Disk cache first; else decode the whole stream (empty Peaks when the stream can't be opened).
fn load_or_compute(path: &str, stream: usize, backend: Backend) -> (Peaks, WaveDetail) {
    let file = cache_file(path, stream);
    if let Some((p, rb, spec)) = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| decode(&b)) {
        crate::engine::cache_lru::touch_if_stale(file.as_deref().unwrap_or(std::path::Path::new("")));
        let d = WaveDetail::new(&p, Some(&rb), spec);
        return (p, d);
    }
    let Ok(mut src) = super::open_audio(path, stream, backend) else {
        return (Peaks { min: Vec::new(), max: Vec::new() }, WaveDetail::default());
    };
    let duration = src.duration();
    let (peaks, rb, spec) = compute(&mut *src, duration);
    if let Some(f) = file {
        if !peaks.is_empty() {
            let tmp = f.with_extension("tmp");
            let _ = std::fs::create_dir_all(Settings::cache_dir());
            if std::fs::write(&tmp, encode(&peaks, &rb, &spec)).is_ok() {
                let _ = std::fs::rename(&tmp, &f);
            }
        }
    }
    let d = WaveDetail::new(&peaks, Some(&rb), spec);
    (peaks, d)
}

#[cfg(test)]
pub(crate) fn compute_peaks(source: &mut dyn AudioSource, duration: f64) -> Peaks {
    compute(source, duration).0
}

/// FFT size of the band / spectrogram analysis (21 ms at 48 kHz, 47 Hz bins), hopped one bucket.
const FFT_N: usize = 1024;

/// FFT bin ranges: the 3 bands, then the `SPEC_BINS` log rows (each at least one bin).
fn bin_ranges() -> ([(usize, usize); 3], Vec<(usize, usize)>) {
    let df = SAMPLE_RATE as f32 / FFT_N as f32;
    let k = |f: f32| ((f / df).round() as usize).clamp(1, FFT_N / 2);
    let bands = [(1, k(BAND_EDGES.0)), (k(BAND_EDGES.0), k(BAND_EDGES.1)), (k(BAND_EDGES.1), FFT_N / 2)];
    let (f0, f1) = (30f32, 16000f32);
    let rows = (0..SPEC_BINS)
        .map(|r| {
            let lo = k(f0 * (f1 / f0).powf(r as f32 / SPEC_BINS as f32));
            let hi = k(f0 * (f1 / f0).powf((r + 1) as f32 / SPEC_BINS as f32));
            (lo, hi.max(lo + 1))
        })
        .collect();
    (bands, rows)
}

/// Sequentially decode `source` in 4800-frame blocks into 10 ms mono (min, max) buckets, plus per bucket
/// [rms, low, mid, high] (`q_db`) from a Hann-windowed FFT of the last `FFT_N` samples, and the
/// spectrogram at `SPEC_PER_SEC`. Stops at the first all-zero block at/after `duration` (so an unknown
/// duration of 0 stops at the first silent block). Waits between blocks while something plays.
fn compute(source: &mut dyn AudioSource, duration: f64) -> (Peaks, Vec<[u8; 4]>, Vec<[u8; SPEC_BINS]>) {
    const BLOCK: usize = 4800;
    const BUCKET: usize = (SAMPLE_RATE / PEAKS_PER_SEC) as usize; // 480 frames
    let (bands, rows) = bin_ranges();
    let window: Vec<f32> =
        (0..FFT_N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_N as f32).cos()).collect();
    // a full-scale sine's Hann power, summed over its main lobe: (N/4)^2 * 1.5 -> amplitude 1.0
    let (norm_sum, norm_peak) = (1.0 / ((FFT_N as f32 / 4.0).powi(2) * 1.5), 1.0 / (FFT_N as f32 / 4.0).powi(2));
    let mut p = Peaks { min: Vec::new(), max: Vec::new() };
    let (mut rb, mut spec) = (Vec::new(), Vec::new());
    let mut buf = vec![0f32; BLOCK * 2];
    let mut hist = vec![0f32; FFT_N];
    let (mut re, mut im) = (vec![0f32; FFT_N], vec![0f32; FFT_N]);
    let mut col = [0u8; SPEC_BINS];
    let mut frame: u64 = 0;
    loop {
        let t = frame as f64 / SAMPLE_RATE as f64;
        if t > 24.0 * 3600.0 {
            break; // ponytail: hard cap - never spin forever on a broken source
        }
        super::bg::pause_point(|| false);
        source.read_at(t, &mut buf);
        if t >= duration && buf.iter().all(|&x| x == 0.0) {
            break;
        }
        for b in buf.chunks_exact(BUCKET * 2) {
            let (mut lo, mut hi, mut sq) = (f32::MAX, f32::MIN, 0f32);
            hist.copy_within(BUCKET.., 0);
            for (j, s) in b.chunks_exact(2).enumerate() {
                let m = (s[0] + s[1]) * 0.5;
                lo = lo.min(m);
                hi = hi.max(m);
                sq += m * m;
                hist[FFT_N - BUCKET + j] = m;
            }
            p.min.push(lo);
            p.max.push(hi);
            for i in 0..FFT_N {
                re[i] = hist[i] * window[i];
                im[i] = 0.0;
            }
            crate::engine::visualizer::fft(&mut re, &mut im);
            let pw = |k: usize| re[k] * re[k] + im[k] * im[k];
            let band = |(a, b): (usize, usize)| q_db(((a..b).map(pw).sum::<f32>() * norm_sum).sqrt());
            rb.push([q_db((sq / BUCKET as f32).sqrt()), band(bands[0]), band(bands[1]), band(bands[2])]);
            let odd = p.len() % 2 == 0;
            for (r, &(a, b)) in rows.iter().enumerate() {
                let v = q_db(((a..b).map(pw).fold(0.0, f32::max) * norm_peak).sqrt());
                col[r] = if odd { col[r].max(v) } else { v };
            }
            if odd {
                spec.push(col);
            }
        }
        frame += BLOCK as u64;
    }
    (p, rb, spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ffpipe;

    #[test]
    fn peaks_from_test_clip() {
        let mut src = ffpipe::open_audio(&ffpipe::tests::test_mp4(), 0).unwrap();
        let d = src.duration();
        let p = compute_peaks(&mut *src, d);
        let expect = (d * PEAKS_PER_SEC as f64) as usize;
        assert!(p.len().abs_diff(expect) <= 10, "len {} vs {expect}", p.len());
        let hi = p.max.iter().cloned().fold(f32::MIN, f32::max);
        let lo = p.min.iter().cloned().fold(f32::MAX, f32::min);
        assert!(hi > 0.1 && lo < -0.1, "{lo}..{hi}");
        let (a, b) = p.range(0.5, 0.6);
        assert!(a < -0.1 && b > 0.1);
    }

    /// get() is non-blocking, completes in the background, and the second instance hits the disk cache.
    #[test]
    fn cache_get_async_and_disk() {
        let path = ffpipe::tests::test_mp4();
        let poll = |c: &mut WaveformCache, path: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            assert!(c.get(path, 1).is_none(), "first get must not block");
            loop {
                if let Some(p) = c.get(path, 1) {
                    return p;
                }
                assert!(std::time::Instant::now() < deadline, "waveform never finished");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let mut c = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        let first = poll(&mut c, &path);
        assert!(first.len() > 350 && first.max.iter().any(|&x| x > 0.1));
        let mut c2 = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        let second = poll(&mut c2, &path);
        assert_eq!(first.len(), second.len());
        assert_eq!(first.max, second.max);
        // failure is remembered as empty peaks (no respawn loop)
        let mut c3 = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        assert!(poll(&mut c3, "Z:/definitely/missing.mp4").is_empty());
    }

    /// After the file at a path is rewritten, clear() makes get() recompute instead of serving old peaks.
    #[test]
    fn clear_forgets_rewritten_file() {
        let poll = |c: &mut WaveformCache, path: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                if let Some(p) = c.get(path, 0) {
                    return p;
                }
                assert!(std::time::Instant::now() < deadline, "waveform never finished");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let path = std::path::Path::new(&ffpipe::tests::test_mp4())
            .with_file_name(format!("{}-rewrite.mp4", std::process::id()));
        std::fs::copy(ffpipe::tests::test_mp4(), &path).unwrap();
        let path = path.to_string_lossy().into_owned();
        let mut c = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        let old = poll(&mut c, &path);
        assert!(old.len() > 350, "{}", old.len());
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-c:a", "aac"])
            .arg(&path)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        assert_eq!(poll(&mut c, &path).len(), old.len(), "stale until cleared");
        c.clear();
        let new = poll(&mut c, &path);
        assert!(new.len() < 150, "{} (old {})", new.len(), old.len());
    }

    #[test]
    fn peaks_range_bounds() {
        let p = Peaks { min: vec![-0.5, -0.2], max: vec![0.5, 0.2] };
        assert_eq!(p.range(0.0, 0.01), (-0.5, 0.5));
        assert_eq!(p.range(0.01, 0.02), (-0.2, 0.2));
        assert_eq!(p.range(5.0, 6.0), (0.0, 0.0));
    }

    /// A stereo sine (or silence) as an AudioSource.
    struct Sine(f32, f64);
    impl AudioSource for Sine {
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, s) in out.chunks_exact_mut(2).enumerate() {
                let tt = t + i as f64 / SAMPLE_RATE as f64;
                let v = if tt < self.1 { 0.5 * (std::f64::consts::TAU * self.0 as f64 * tt).sin() as f32 } else { 0.0 };
                s[0] = v;
                s[1] = v;
            }
        }
        fn duration(&self) -> f64 {
            self.1
        }
    }

    #[test]
    fn band_energies_land_in_the_right_band() {
        for (hz, band) in [(60.0, 1), (1000.0, 2), (6000.0, 3)] {
            let (p, rb, spec) = compute(&mut Sine(hz, 1.0), 1.0);
            assert!(p.len() >= 100 && rb.len() == p.len() && spec.len() == p.len() / 2);
            let x = rb[50]; // steady state
            for b in 1..4 {
                if b != band {
                    assert!(x[band] > x[b] + 40, "{hz} Hz: {x:?}");
                }
            }
            // a -6 dB sine reads about -6 dB in its band and in RMS (-9 dB)
            assert!((amp_of(x[band]) - 0.5).abs() < 0.1, "{hz} Hz: band amp {}", amp_of(x[band]));
            assert!((amp_of(x[0]) - 0.354).abs() < 0.05, "rms {}", amp_of(x[0]));
            // the spectrogram's loudest row sits at the sine's frequency
            let row = (0..SPEC_BINS).max_by_key(|&r| spec[25][r]).unwrap();
            let f = 30.0 * (16000f32 / 30.0).powf((row as f32 + 0.5) / SPEC_BINS as f32);
            assert!((f / hz as f32).log2().abs() < 0.5, "{hz} Hz -> row {row} ({f} Hz)");
        }
    }

    #[test]
    fn db_quantisation_round_trips() {
        assert_eq!(q_db(1.0), 255);
        assert_eq!(q_db(0.0), 0);
        assert!((amp_of(q_db(0.1)) - 0.1).abs() < 0.005);
        assert_eq!(amp_of(0), 0.0);
    }

    /// Each column merges a few pyramid entries and a lone spike shows in exactly the column(s) over it.
    #[test]
    fn column_picks_the_right_pyramid_level() {
        let n = 100_000;
        let mut p = Peaks { min: vec![-0.01; n], max: vec![0.01; n] };
        p.max[70_000] = 0.9;
        let d = WaveDetail::new(&p, None, Vec::new());
        assert_eq!(d.levels[0].len(), n);
        assert_eq!(d.levels.last().unwrap().len(), 1);
        for span in [0.003, 0.01, 0.37, 4.0, 60.0] {
            let l = d.level_for(span * PEAKS_PER_SEC as f64);
            let entries = span * PEAKS_PER_SEC as f64 / (1u64 << l) as f64;
            assert!(entries < 5.0, "span {span}: level {l} reads {entries} entries");
            let hits: Vec<usize> = (0..(n as f64 / PEAKS_PER_SEC as f64 / span) as usize)
                .filter(|&i| d.col(i as f64 * span, (i + 1) as f64 * span).unwrap().hi > 0.5)
                .collect();
            // zoomed in past one bucket (10 ms) a bucket spans several columns
            let most = 2 + (0.01 / span) as usize;
            assert!(!hits.is_empty() && hits.len() <= most, "span {span}: {hits:?}");
            assert!(hits.iter().all(|&i| (i as f64 * span - 700.0).abs() <= 2.0 * span + 0.01), "{span}: {hits:?}");
        }
        assert!(d.col(5000.0, 5001.0).is_none(), "past the end");
    }

    /// An old headerless `.peaks` file (and any other version) is rejected, so it gets rebuilt.
    #[test]
    fn cache_version_bump_rejects_old_files() {
        let (p, rb, spec) = compute(&mut Sine(440.0, 0.5), 0.5);
        let bytes = encode(&p, &rb, &spec);
        let (p2, rb2, spec2) = decode(&bytes).unwrap();
        assert_eq!((p2.max, rb2, spec2), (p.max.clone(), rb, spec));
        let old: Vec<u8> =
            p.min.iter().zip(&p.max).flat_map(|(a, b)| [a.to_le_bytes(), b.to_le_bytes()]).flatten().collect();
        assert!(decode(&old).is_none());
        let mut other = bytes.clone();
        other[4] = CACHE_VERSION as u8 + 1;
        assert!(decode(&other).is_none());
        assert!(decode(&bytes[..bytes.len() - 1]).is_none());
    }

    #[test]
    fn spectral_flux_finds_clicks() {
        let mut spec = vec![[0u8; SPEC_BINS]; 500];
        for i in (50..500).step_by(25) {
            spec[i] = [200; SPEC_BINS];
        }
        let on = spec_onsets(&spec);
        assert_eq!(on.len(), 18, "{on:?}");
        assert!((on[0] - 1.0).abs() < 0.03, "{on:?}");
    }

    #[test]
    fn mem_key_differs_per_stream() {
        assert_ne!(mem_key("a.mp4", 0), mem_key("a.mp4", 1));
        assert_eq!(mem_key("a.mp4", 0), mem_key("a.mp4", 0));
    }
}
