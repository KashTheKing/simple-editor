//! ---- ws:animator-templates ----
//! Templates: a group of clips you place in one go, with their animation blocks, keyframes, transitions,
//! text / shape / HTML layers, effects and sound - and SLOTS, the clips you fill with your own footage.
//! This is the payload of a Gallery `template` item (`engine::gallery`, `Settings.templates`): the two
//! fields older templates have (`clips`, `assets`) plus optional ones, so an old template still loads and
//! places exactly as before.
//!
//! A slot is a container clip (`Clip.container`, the existing "replace media, keep everything else" clip).
//! `Slot` adds what the template wants there: a name, image / video / any, and whether the footage is cut
//! to the slot or the template stretches to the footage. An empty slot of a built-in or built template has
//! a coloured card under it that goes away when the slot is filled.
//!
//! Sharing is one `.setemplate` file (`export_package` / `import_package`): a small header, the template as
//! JSON, then the bytes of any footage that rides along. No archive tool and no new dependency; files are
//! streamed, so size is not a limit. Paths inside are file names only, never a folder of the sender's PC.

use crate::engine::gallery::{self, Collide, Item, Kind};
use crate::model::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The extension of a shared template file.
pub const EXT: &str = "setemplate";
const MAGIC: &[u8] = b"SETEMPLATE1\n";
const FORMAT: &str = "simple-editor-template";

/// What a slot would like to be filled with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotWant {
    #[default]
    Any,
    Image,
    Video,
    /// A sound: the song of a beat-synced edit, or a sound effect (an audio clip of the template).
    Audio,
}

impl SlotWant {
    /// What a picture slot can want (the Template Builder's choice); `Audio` is a sound slot's own.
    pub const ALL: [SlotWant; 3] = [SlotWant::Any, SlotWant::Image, SlotWant::Video];
    pub fn name(self) -> &'static str {
        match self {
            SlotWant::Any => "any",
            SlotWant::Image => "image",
            SlotWant::Video => "video",
            SlotWant::Audio => "audio",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SlotWant::Any => "Any",
            SlotWant::Image => "Image",
            SlotWant::Video => "Video",
            SlotWant::Audio => "Sound",
        }
    }
    pub fn parse(s: &str) -> Option<SlotWant> {
        Self::ALL.into_iter().chain([SlotWant::Audio]).find(|w| w.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// How footage and slot agree on a length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotFit {
    /// The slot keeps its length: longer footage is trimmed, shorter footage is slowed to fill it.
    #[default]
    Trim,
    /// The slot takes the footage's length and the rest of the template moves along.
    Stretch,
}

impl SlotFit {
    pub const ALL: [SlotFit; 2] = [SlotFit::Trim, SlotFit::Stretch];
    pub fn name(self) -> &'static str {
        match self {
            SlotFit::Trim => "trim",
            SlotFit::Stretch => "stretch",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SlotFit::Trim => "Trim footage",
            SlotFit::Stretch => "Stretch template",
        }
    }
    pub fn parse(s: &str) -> Option<SlotFit> {
        Self::ALL.into_iter().find(|w| w.name().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    /// The slot's picture clip, by its id inside the template.
    pub clip: Id,
    #[serde(default)]
    pub want: SlotWant,
    #[serde(default)]
    pub fit: SlotFit,
    /// The placeholder card under an empty slot (a template clip id; 0 = none): removed when it is filled.
    #[serde(default, skip_serializing_if = "is_zero_id")]
    pub card: Id,
    /// A sound slot: words its file name has, to fill it from your sounds when nothing is given
    /// (`find_sound`: the media library, then your SFX and music folders).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// A sound slot: seconds into the clip where the sound's hit (its loudest onset) lands.
    #[serde(default, skip_serializing_if = "is_zero_f")]
    pub hit: f64,
}

impl Slot {
    fn new(name: &str, clip: Id, want: SlotWant, fit: SlotFit) -> Slot {
        Slot { name: name.to_string(), clip, want, fit, card: 0, hint: String::new(), hit: 0.0 }
    }
}

/// How a beat-synced edit follows the song in its song slot. The template is laid out on a grid of `bpm`
/// (beat 0 at its start, every cut on a beat); placed with a song, it is retimed to the song's tempo and
/// the song is cut so the template's drop lands on the song's drop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeatSync {
    pub bpm: f64,
    /// Template seconds of the drop (a downbeat of the grid).
    pub drop: f64,
    /// The song slot's clip (an audio container of this template).
    pub song: Id,
}

/// What the song in a song slot does: its tempo grid and drop, in source seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongBeat {
    pub bpm: f64,
    /// A downbeat.
    pub offset: f64,
    pub drop: f64,
    /// The song's length.
    pub len: f64,
}

/// Footage for slot `slot` (index in `slot_list`). `onset`: a sound's hit, source seconds (sound slots).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub slot: usize,
    pub asset: Id,
    pub onset: f64,
}

fn is_zero_id(i: &Id) -> bool {
    *i == 0
}
fn is_zero_f(v: &f64) -> bool {
    *v == 0.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TemplateData {
    pub clips: Vec<Clip>,
    pub assets: Vec<Asset>,
    /// Transitions between / on the clips; `right` is a clip id of this template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Transition>,
    /// Per clip: its track among the template's tracks of its kind (0 = lowest video / first audio).
    /// Empty (an older template) = every clip goes to the first track with room.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<u32>,
    /// The slots, in fill order. Empty = every container picture clip is a slot (older templates).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<Slot>,
    /// Where the Gallery picture was taken (seconds into the template).
    #[serde(default, skip_serializing_if = "is_zero_f")]
    pub thumb_t: f64,
    /// Mixer buses its clips play through (`Clip.bus`), with their filters and automation (template
    /// seconds): a song muffled until the drop is a low-pass on its bus. Placed as new buses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<Bus>,
    /// A beat-synced edit: how it follows its song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<BeatSync>,
    /// Where its own sample footage is in the content library (`templates/<slug>`, files fetched on use).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media: String,
    /// The sample footage each slot comes with when it is dropped on the timeline (files in `media`).
    /// Slots naming the same file show the same footage, linked: replace it once and they all change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<Sample>,
}

/// A slot's sample footage: a file of the template's `media` folder, trimmed to what the template uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// The slot's clip (a template clip id).
    pub clip: Id,
    pub file: String,
    /// Only from your own media folder, never fetched (a sound the template may not ship): without it the
    /// clip is left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
}

impl TemplateData {
    pub fn decode(json: &str) -> Option<TemplateData> {
        serde_json::from_str(json).ok()
    }
    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }
    pub fn duration(&self) -> f64 {
        self.clips.iter().map(|c| c.end()).fold(0.0, f64::max)
    }
    /// The slots in fill order: the listed ones, else every container picture clip by start time.
    pub fn slot_list(&self) -> Vec<Slot> {
        if !self.slots.is_empty() {
            return self.slots.iter().filter(|s| self.clip(s.clip).is_some()).cloned().collect();
        }
        let mut cs: Vec<&Clip> = self.clips.iter().filter(|c| c.container && c.kind != ClipKind::Audio).collect();
        cs.sort_by(|a, b| a.start.total_cmp(&b.start));
        cs.iter()
            .enumerate()
            .map(|(i, c)| {
                let name = if c.container_label.is_empty() { format!("Slot {}", i + 1) } else { c.container_label.clone() };
                Slot::new(&name, c.id, SlotWant::Any, SlotFit::Trim)
            })
            .collect()
    }
    /// Make `clip` a slot (or change one). Its container label follows the name.
    pub fn set_slot(&mut self, clip: Id, name: &str, want: SlotWant, fit: SlotFit) {
        let link = self.clip(clip).map_or(0, |c| c.link);
        for c in &mut self.clips {
            if c.id == clip || (link != 0 && c.link == link) {
                c.container = true;
                c.container_label = name.to_string();
            }
        }
        match self.slots.iter_mut().find(|s| s.clip == clip) {
            Some(s) => (s.name, s.want, s.fit) = (name.to_string(), want, fit),
            None => self.slots.push(Slot::new(name, clip, want, fit)),
        }
    }
    /// Take the footage out of `clip` (and its linked sound): an empty slot. Assets nobody uses any more go.
    pub fn strip_footage(&mut self, clip: Id) {
        let link = self.clip(clip).map_or(0, |c| c.link);
        for c in &mut self.clips {
            if (c.id == clip || (link != 0 && c.link == link)) && c.uses_asset() {
                c.asset = 0;
                c.src_in = 0.0;
                c.container = true;
                if c.kind == ClipKind::Image {
                    c.kind = ClipKind::Video;
                }
            }
        }
        let used: Vec<Id> = self.clips.iter().filter(|c| c.uses_asset()).map(|c| c.asset).collect();
        self.assets.retain(|a| used.contains(&a.id));
    }
    /// Every empty picture container is listed as a slot (footage that went missing becomes one).
    fn list_empty_slots(&mut self) {
        let mut slots = self.slot_list();
        for c in self.clips.iter().filter(|c| c.is_empty_container() && c.kind != ClipKind::Audio) {
            if !slots.iter().any(|s| s.clip == c.id) {
                let name = if c.container_label.is_empty() { c.name.clone() } else { c.container_label.clone() };
                slots.push(Slot::new(&name, c.id, SlotWant::Any, SlotFit::Trim));
            }
        }
        self.slots = slots;
    }
    /// A coloured card under every empty slot that has none, moving as the slot moves, in a `w` x `h` frame.
    /// (A template without layers cannot say "under", so it gets none.)
    pub fn add_cards(&mut self, w: u32, h: u32) {
        if self.layers.len() != self.clips.len() {
            return;
        }
        self.slots = self.slot_list();
        let mut next = self.clips.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let mut doubled = false;
        for si in 0..self.slots.len() {
            let (clip, card) = (self.slots[si].clip, self.slots[si].card);
            let Some(i) = self.clips.iter().position(|c| c.id == clip && c.is_empty_container() && c.kind != ClipKind::Audio)
            else {
                continue;
            };
            if card != 0 {
                continue;
            }
            // (built again from a placed template: the card that came with the slot is taken, not doubled)
            let (name, at, len) = (format!("{} card", self.slots[si].name), self.clips[i].start, self.clips[i].duration);
            let old = self.clips.iter().find(|c| {
                c.kind == ClipKind::Shape && c.name == name && (c.start - at).abs() < 1e-6 && (c.duration - len).abs() < 1e-6
            });
            if let Some(old) = old {
                self.slots[si].card = old.id;
                continue;
            }
            if !doubled {
                // room for a card track under every track
                self.layers.iter_mut().for_each(|l| *l = *l * 2 + 1);
                doubled = true;
            }
            let src = &self.clips[i];
            let mut c = Clip::new(next, ClipKind::Shape, format!("{} card", self.slots[si].name), src.start, src.duration);
            if let Some(s) = c.shape.as_mut() {
                s.fill = CARD_COLORS[si % CARD_COLORS.len()];
                s.w = Animated::new(w as f64 / 2.0);
                s.h = Animated::new(h as f64 / 2.0);
            }
            for p in ["x", "y", "scale", "scale_x", "scale_y", "rotation", "opacity"] {
                if let (Some(from), Some(to)) = (prop_ref(src, p).cloned(), prop_mut(&mut c, p)) {
                    *to = from;
                }
            }
            c.blocks = src.blocks.clone();
            c.mask = src.mask.clone();
            self.layers.push(self.layers[i] - 1);
            self.clips.push(c);
            self.slots[si].card = next;
            next += 1;
        }
    }
}

impl TemplateData {
    /// Stretch the whole template in time by `k` (2 = twice as long): clip times, keys, blocks, effect
    /// windows, transitions, bus automation and the beat grid. Footage speeds are left alone.
    pub fn retime(&mut self, k: f64) {
        if !(k.is_finite() && k > 0.0) || (k - 1.0).abs() < 1e-9 {
            return;
        }
        for c in &mut self.clips {
            c.start *= k;
            c.duration *= k;
            c.fade_in *= k;
            c.fade_out *= k;
            for a in c.all_animated_mut() {
                a.keys.iter_mut().for_each(|key| key.t *= k);
            }
            for b in &mut c.blocks {
                b.t *= k;
                b.dur *= k;
            }
            for e in &mut c.effects {
                e.start *= k;
                e.len *= k;
            }
        }
        for t in &mut self.transitions {
            t.duration *= k;
        }
        for b in &mut self.buses {
            for a in bus_anims(b) {
                a.keys.iter_mut().for_each(|key| key.t *= k);
            }
        }
        for s in &mut self.slots {
            s.hit *= k;
        }
        if let Some(sy) = &mut self.sync {
            sy.bpm /= k;
            sy.drop *= k;
        }
        self.thumb_t *= k;
    }

    /// Retime a beat-synced template to `song` and say where the song starts (source seconds): the
    /// template's grid becomes the song's (the nearer of its tempo, double or half, so cuts keep their
    /// feel) and its drop lands on the song's drop. A song whose drop comes too early to fit the template
    /// before it starts a whole number of bars later, still on the beat. None for a template without sync.
    pub fn fit_to_song(&mut self, song: &SongBeat) -> Option<f64> {
        let sy = self.sync.clone()?;
        if !(song.bpm > 0.0 && sy.bpm > 0.0) {
            return Some(0.0);
        }
        let tp = 60.0 / sy.bpm;
        let mut sp = 60.0 / song.bpm;
        while sp / tp > 1.45 {
            sp /= 2.0;
        }
        while sp / tp < 0.72 {
            sp *= 2.0;
        }
        self.retime(sp / tp);
        let drop = self.sync.as_ref().map_or(0.0, |s| s.drop);
        let bar = 4.0 * sp;
        let mut src_in = song.drop - drop;
        if src_in < 0.0 {
            src_in += (-src_in / bar).ceil() * bar;
        }
        Some(src_in)
    }
}

/// Every keyframeable value of a bus: gain, pan and its filters' parameters.
fn bus_anims(b: &mut Bus) -> impl Iterator<Item = &mut Animated> {
    std::iter::once(&mut b.gain).chain(std::iter::once(&mut b.pan)).chain(b.filters.iter_mut().flat_map(|f| f.params.iter_mut()))
}

/// Where a song drops: the bar line (source seconds, at or after `min`) where the two bars after it are
/// loud and much louder than the two bars before it - the jump weighted by how loud it lands, so a drop
/// out of a breakdown into the full beat beats the intro's first lift. `low` = low-end level per analysis
/// frame at `hz`. Without a bar line that late, the best one anywhere.
pub fn find_drop(low: &[f32], hz: f64, bpm: f64, offset: f64, min: f64) -> Option<f64> {
    if !(bpm > 0.0) || low.is_empty() {
        return None;
    }
    let bar = 4.0 * 60.0 / bpm;
    let len = low.len() as f64 / hz;
    let mean = |a: f64, b: f64| {
        let (i, j) = (((a * hz) as usize).min(low.len()), ((b * hz) as usize).min(low.len()));
        if j <= i {
            return 0.0;
        }
        low[i..j].iter().map(|&v| v as f64).sum::<f64>() / (j - i) as f64
    };
    let first = offset - (offset / bar).floor() * bar;
    let mut cands: Vec<(f64, f64, f64)> = Vec::new(); // (t, jump, level after)
    let mut t = first;
    while t + 2.0 * bar <= len {
        if t >= 2.0 * bar - 1e-9 {
            let after = mean(t, t + 2.0 * bar);
            cands.push((t, after - mean(t - 2.0 * bar, t), after));
        }
        t += bar;
    }
    let loudest = cands.iter().map(|c| c.2).fold(0.0, f64::max);
    let score = |c: &(f64, f64, f64)| if loudest > 0.0 { c.1 * c.2 / loudest } else { 0.0 };
    let best = |late: bool| {
        cands.iter().filter(|c| c.1 > 0.0 && (!late || c.0 >= min - 1e-9)).max_by(|a, b| score(a).total_cmp(&score(b))).map(|c| c.0)
    };
    best(true).or_else(|| best(false))
}

/// The start of a sound's main hit (seconds): the last of its loudest peaks (within 3 dB of the loudest,
/// so a sting after a whoosh or an impact wins), walked back to where the level climbed past half of it.
/// `level` = peak level per frame at `hz`.
pub fn loudest_onset(level: &[f32], hz: f64) -> f64 {
    let top = level.iter().copied().fold(0.0f32, f32::max);
    if !(top > 0.0) {
        return 0.0;
    }
    let near = (0.15 * hz).ceil() as usize;
    let peak = (0..level.len())
        .rev()
        .find(|&i| {
            let (a, b) = (i.saturating_sub(near), (i + near + 1).min(level.len()));
            level[i] >= top * 0.7 && level[a..b].iter().all(|&v| v <= level[i])
        })
        .unwrap_or(0);
    let mut i = peak;
    while i > 0 && level[i - 1] > level[peak] * 0.5 {
        i -= 1;
    }
    i as f64 / hz
}

/// A sound file for a sound slot's `hint` under `roots` (a few folders deep): the first whose name holds
/// every word of the hint, sound files before videos. None when the hint is empty or nothing matches.
pub fn find_sound(hint: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let words: Vec<String> = hint.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return None;
    }
    const SOUND: [&str; 6] = ["wav", "mp3", "m4a", "ogg", "flac", "opus"];
    const VIDEO: [&str; 4] = ["mp4", "mov", "webm", "mkv"];
    fn walk(dir: &Path, depth: u32, f: &mut dyn FnMut(&Path)) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut es: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        es.sort();
        for p in es {
            if p.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, f);
                }
            } else {
                f(&p);
            }
        }
    }
    let mut hits: Vec<(u8, PathBuf)> = Vec::new();
    for r in roots {
        walk(r, 3, &mut |p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let rank = if SOUND.contains(&ext.as_str()) {
                0
            } else if VIDEO.contains(&ext.as_str()) {
                1
            } else {
                return;
            };
            if words.iter().all(|w| name.contains(w.as_str())) {
                hits.push((rank, p.to_path_buf()));
            }
        });
    }
    hits.sort_by_key(|h| h.0); // (stable: folder order within a rank)
    hits.into_iter().next().map(|h| h.1)
}

/// Media for slots wanting `wants`, in order: sounds (`media[i].1`) to sound slots, pictures to the
/// others. With `cycle` (a beat edit with more shots than clips) the pictures go round again until every
/// picture slot has one; otherwise a slot past the last medium stays empty.
pub fn assign(wants: &[SlotWant], media: &[(Id, bool)], cycle: bool) -> Vec<Option<Id>> {
    let mut out = vec![None; wants.len()];
    for sound in [true, false] {
        let mine: Vec<Id> = media.iter().filter(|m| m.1 == sound).map(|m| m.0).collect();
        if mine.is_empty() {
            continue;
        }
        let mut k = 0;
        for (i, w) in wants.iter().enumerate() {
            if (*w == SlotWant::Audio) != sound || (k >= mine.len() && !(cycle && !sound)) {
                continue;
            }
            out[i] = Some(mine[k % mine.len()]);
            k += 1;
        }
    }
    out
}

/// The folders a sound slot's hint is looked for in: your Video Assets SFX folder, then Music.
pub fn sound_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else { return Vec::new() };
    [home.join("Videos").join("Video Assets").join("SFX"), home.join("Music")].into_iter().filter(|p| p.is_dir()).collect()
}

/// Placeholder card colours, one per slot in turn.
const CARD_COLORS: [[u8; 4]; 6] = [
    [56, 97, 140, 255],
    [140, 76, 96, 255],
    [70, 122, 92, 255],
    [150, 112, 58, 255],
    [98, 82, 146, 255],
    [60, 120, 128, 255],
];

/// The clips `ids` of `project` as a template: times from the earliest start, tracks kept in order, and the
/// transitions that sit between two of them or on one of them.
pub fn capture(project: &Project, ids: &[Id]) -> TemplateData {
    let mut d = TemplateData::default();
    let (mut v, mut a) = (0u32, 0u32);
    for t in &project.tracks {
        let n = if t.kind == TrackKind::Video { &mut v } else { &mut a };
        let mine: Vec<&Clip> = t.clips.iter().filter(|c| ids.contains(&c.id)).collect();
        if mine.is_empty() {
            continue;
        }
        for tr in &t.transitions {
            let Some(right) = mine.iter().find(|c| c.id == tr.right) else { continue };
            let joined = tr.edge != TransitionEdge::Cut || mine.iter().any(|c| (c.end() - right.start).abs() < 1e-3);
            if joined {
                d.transitions.push(tr.clone());
            }
        }
        for c in mine {
            d.clips.push(c.clone());
            d.layers.push(*n);
        }
        *n += 1;
    }
    let start = d.clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    // the buses its clips play through (not Main), their automation moved to template time
    let main = project.buses.first().map_or(0, |b| b.id);
    let mut buses: Vec<Id> = d.clips.iter().map(|c| c.bus).filter(|&b| b != 0 && b != main).collect();
    buses.sort_unstable();
    buses.dedup();
    for b in buses {
        if let Some(mut bus) = project.bus(b).cloned() {
            let dt = if start.is_finite() { start } else { 0.0 };
            bus_anims(&mut bus).for_each(|a| a.keys.iter_mut().for_each(|k| k.t -= dt));
            bus.output = 0;
            d.buses.push(bus);
        }
    }
    for c in &mut d.clips {
        c.start -= if start.is_finite() { start } else { 0.0 };
        if c.uses_asset() && !d.assets.iter().any(|x| x.id == c.asset) {
            if let Some(x) = project.asset(c.asset) {
                d.assets.push(x.clone());
            }
        }
    }
    d
}

/// What the Template Builder decided for one footage clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub clip: Id,
    /// A slot: its name, what it wants, how it fits. None = a fixed part of the template.
    pub slot: Option<(String, SlotWant, SlotFit)>,
    /// Its footage rides along. Off: a picture becomes a placeholder slot, a sound is left out.
    pub include: bool,
}

/// Build a template from the clips `ids` of `project`: `parts` (in slot order) say which footage clips are
/// slots and whose footage stays; clips not named in `parts` are kept as they are.
pub fn build(project: &Project, ids: &[Id], parts: &[Part], thumb_t: f64) -> TemplateData {
    let mut d = capture(project, ids);
    for p in parts {
        let Some(c) = d.clip(p.clip) else { continue };
        let (name, link) = (c.name.clone(), c.link);
        if c.kind == ClipKind::Audio {
            if !p.include {
                if let Some(i) = d.clips.iter().position(|c| c.id == p.clip) {
                    d.clips.remove(i);
                    d.layers.remove(i);
                }
            }
            continue;
        }
        if c.kind != ClipKind::Video {
            continue; // (only video clips can be slots)
        }
        match &p.slot {
            Some((n, want, fit)) => d.set_slot(p.clip, if n.trim().is_empty() { &name } else { n.trim() }, *want, *fit),
            None if !p.include => d.set_slot(p.clip, &name, SlotWant::Any, SlotFit::Trim),
            // a container of the timeline that is not to be a slot here
            None => d.clips.iter_mut().filter(|c| c.id == p.clip || (link != 0 && c.link == link)).for_each(|c| {
                c.container = false;
                c.container_label.clear();
            }),
        }
        if !p.include {
            d.strip_footage(p.clip);
        }
    }
    // only video clips are slots: sound, music and pictures are the template's own (a video's sound half
    // follows its picture)
    let videos: Vec<Id> = d.clips.iter().filter(|c| c.kind == ClipKind::Video && c.container).map(|c| c.link).filter(|&l| l != 0).collect();
    for c in d.clips.iter_mut().filter(|c| c.kind != ClipKind::Video && !(c.link != 0 && videos.contains(&c.link))) {
        c.container = false;
        c.container_label.clear();
    }
    let used: Vec<Id> = d.clips.iter().filter(|c| c.uses_asset()).map(|c| c.asset).collect();
    d.assets.retain(|a| used.contains(&a.id));
    d.transitions.retain(|t| d.clips.iter().any(|c| c.id == t.right));
    if !d.slots.is_empty() {
        d.list_empty_slots();
    }
    d.thumb_t = thumb_t.clamp(0.0, d.duration());
    d.add_cards(project.width, project.height);
    d
}

/// Place template `d` at `at` and fill its slots: `fills` = (slot index in `slot_list`, project asset id).
/// Returns the new clip ids. A filled slot's placeholder card is removed, and so is a track that was added
/// only for cards.
pub fn apply(p: &mut Project, d: TemplateData, at: f64, fills: &[(usize, Id)]) -> Vec<Id> {
    let fills: Vec<Fill> = fills.iter().map(|&(slot, asset)| Fill { slot, asset, onset: 0.0 }).collect();
    apply_synced(p, d, at, &fills, None)
}

/// `apply` for every kind of slot: picture slots, the song slot and sound-effect slots. With `song` (the
/// song slot's beat grid and drop) a beat-synced template is first retimed to the song and the song cut so
/// the drops meet; the song becomes the Beat Aligner's when it has none. Footage that fills several slots
/// plays a different stretch of itself in each.
pub fn apply_synced(p: &mut Project, mut d: TemplateData, at: f64, fills: &[Fill], song: Option<SongBeat>) -> Vec<Id> {
    let song_slot = d.sync.as_ref().and_then(|sy| d.slot_list().iter().position(|s| s.clip == sy.song));
    let song_fill = song_slot.and_then(|i| fills.iter().find(|f| f.slot == i)).copied();
    let song_in = match (song_fill, song) {
        (Some(_), Some(sb)) => d.fit_to_song(&sb),
        _ => None,
    };
    let slots = d.slot_list();
    let before: Vec<Id> = p.tracks.iter().map(|t| t.id).collect();
    // the template's buses become new buses of the project, their automation moved to `at`
    let main = p.main_bus();
    let mut bus_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
    for b in &d.buses {
        let id = p.add_bus(b.name.clone());
        let mut nb = b.clone();
        (nb.id, nb.output) = (id, main);
        bus_anims(&mut nb).for_each(|a| a.keys.iter_mut().for_each(|k| k.t += at));
        if let Some(x) = p.bus_mut(id) {
            *x = nb;
        }
        bus_map.insert(b.id, id);
    }
    for c in &mut d.clips {
        c.bus = bus_map.get(&c.bus).copied().unwrap_or(0);
    }
    let sync_song = d.sync.as_ref().map(|sy| sy.song);
    let (mut ids, map) = p.place_template(d.clips, d.assets, d.transitions, &d.layers, at);
    let mut repeats: Vec<(Id, Id)> = Vec::new(); // (asset, picture clip) in slot order
    for f in fills {
        let Some(s) = slots.get(f.slot) else { continue };
        let Some(&clip) = map.get(&s.clip) else { continue };
        let ok = if p.clip(clip).is_some_and(|c| c.kind == ClipKind::Audio) {
            let src_in = if Some(f.slot) == song_slot { song_in.unwrap_or(0.0) } else { f.onset - s.hit };
            fill_sound(p, clip, f.asset, src_in)
        } else {
            let ok = p.fill_slot(clip, f.asset, s.fit == SlotFit::Stretch, &ids);
            if ok {
                repeats.push((f.asset, clip));
            }
            ok
        };
        if let (true, Some(card)) = (ok, map.get(&s.card)) {
            for t in &mut p.tracks {
                t.clips.retain(|c| c.id != *card);
            }
        }
    }
    spread_repeats(p, &repeats);
    if sync_song.is_some() {
        cover_frame(p, &repeats);
    }
    if let (Some(sb), Some(f), Some(&clip)) = (song, song_fill, sync_song.and_then(|s| map.get(&s))) {
        if p.beats.asset == 0 || p.beats.asset == f.asset {
            p.set_beat_source(Some(clip), None, false);
            p.beats.bpm = (sb.bpm * 100.0).round() / 100.0;
            p.beats.offset = sb.offset;
            if !p.beats.sections.iter().any(|s| s.name.eq_ignore_ascii_case("Drop")) {
                p.beats.add_section(sb.drop, sb.drop + 16.0 * 60.0 / sb.bpm.max(1.0), "Drop");
            }
        }
    }
    while let Some(i) = p.tracks.iter().position(|t| t.clips.is_empty() && !before.contains(&t.id)) {
        p.remove_track(i);
    }
    ids.retain(|id| p.clip(*id).is_some());
    ids
}

/// Place template `d` as ONE nested-sequence clip at `at` (video track `track` preferred), its slots filled
/// with their sample footage (`media`: sample file -> project asset). Inside, every slot is a container;
/// slots showing the same footage share a media link. Its media goes into the bin `Templates/<name>` (an
/// asset already filed elsewhere stays there). Returns the sequence clip.
pub fn place_nested(p: &mut Project, name: &str, mut d: TemplateData, at: f64, track: Option<usize>, media: &[(String, Id)]) -> Option<Id> {
    fill_fixed(&mut d, p, media);
    let slots = d.slot_list();
    let fills: Vec<Fill> = d
        .samples
        .iter()
        .filter_map(|s| {
            let i = slots.iter().position(|x| x.clip == s.clip)?;
            let asset = media.iter().find(|m| m.0 == s.file)?.1;
            // (a sample sound is cut so its hit is where the slot wants it)
            Some(Fill { slot: i, asset, onset: slots[i].hit })
        })
        .collect();
    let before: Vec<Id> = p.assets.iter().map(|a| a.id).collect();
    let n = p.sequences.iter().filter(|s| s.name == name || s.name.starts_with(&format!("{name} "))).count();
    let title = if n == 0 { name.to_string() } else { format!("{name} {}", n + 1) };
    let seq = p.new_sequence(title, p.width, p.height, p.fps);
    let mut inner = std::mem::take(&mut p.sequence_mut(seq)?.tracks);
    std::mem::swap(&mut p.tracks, &mut inner);
    apply_synced(p, d, 0.0, &fills, None);
    // (the filled slots' cards are gone: no empty track is left behind them)
    while let Some(i) = p.tracks.iter().position(|t| t.clips.is_empty() && p.tracks.iter().filter(|o| o.kind == t.kind).count() > 1) {
        p.remove_track(i);
    }
    // footage shown in several slots: one media link per file
    let mut uses: Vec<(Id, usize)> = Vec::new();
    for (_, c) in p.all_clips().filter(|(_, c)| c.container && c.kind != ClipKind::Audio && c.asset != 0) {
        match uses.iter_mut().find(|u| u.0 == c.asset) {
            Some(u) => u.1 += 1,
            None => uses.push((c.asset, 1)),
        }
    }
    for (asset, _) in uses.into_iter().filter(|u| u.1 > 1) {
        let ml = p.new_id();
        let ids: Vec<Id> = p.all_clips().filter(|(_, c)| c.container && c.asset == asset).map(|(_, c)| c.id).collect();
        for id in p.expand_links(&ids) {
            if let Some(c) = p.clip_mut(id) {
                c.media_link = ml;
            }
        }
    }
    std::mem::swap(&mut p.tracks, &mut inner);
    p.sequence_mut(seq)?.tracks = inner;
    let bin = format!("Templates/{name}");
    for a in p.assets.iter_mut().filter(|a| a.folder.is_empty() && (!before.contains(&a.id) || media.iter().any(|m| m.1 == a.id))) {
        a.folder = bin.clone();
    }
    p.insert_sequence_clip(seq, at, track)
}

/// Give the template's fixed parts (clips that are not slots: its song, sound effects, the WASTED shot)
/// their sample media, `media` = sample file -> project asset; slots are filled by `apply_synced` instead.
pub fn fill_fixed(d: &mut TemplateData, p: &Project, media: &[(String, Id)]) {
    let slots: Vec<Id> = d.slot_list().iter().map(|s| s.clip).collect();
    let mut next = d.assets.iter().map(|a| a.id).max().unwrap_or(0) + 1;
    for s in d.samples.clone().iter().filter(|s| !slots.contains(&s.clip)) {
        let Some(a) = media.iter().find(|m| m.0 == s.file).and_then(|m| p.asset(m.1)) else { continue };
        let id = match d.assets.iter().find(|x| x.path == a.path) {
            Some(x) => x.id,
            None => {
                let mut x = a.clone();
                (x.id, next) = (next, next + 1);
                d.assets.push(x);
                next - 1
            }
        };
        if let Some(c) = d.clips.iter_mut().find(|c| c.id == s.clip) {
            (c.asset, c.src_in, c.audio_stream) = (id, 0.0, 0);
        }
    }
    // a fixed part whose media is not on this PC (a local-only sound) is left out
    let empty: Vec<usize> = (0..d.clips.len())
        .filter(|&i| d.samples.iter().any(|s| s.clip == d.clips[i].id && !slots.contains(&s.clip)) && d.clips[i].asset == 0)
        .collect();
    for &i in empty.iter().rev() {
        d.clips.remove(i);
        if i < d.layers.len() {
            d.layers.remove(i);
        }
    }
}

/// The phonk edit (not a built-in: it needs a real song, which is never shipped) for a `w` x `h` frame,
/// its media in your own folder: `<gallery>/templates/media/phonk-synced-edit/`.
pub fn phonk_edit(w: u32, h: u32) -> TemplateData {
    let mut b = Build::new(w, h);
    edits::phonk(&mut b);
    b.done()
}

/// Put sound `asset` into the sound slot `clip` from source second `src_in` (a negative one starts the
/// clip that much later instead); the clip ends where the sound does.
fn fill_sound(p: &mut Project, clip: Id, asset: Id, src_in: f64) -> bool {
    let Some(a) = p.asset(asset).cloned() else { return false };
    if a.audio_streams.is_empty() && a.kind != ClipKind::Audio {
        return false;
    }
    let Some(c) = p.clip_mut(clip) else { return false };
    if !c.container || c.kind != ClipKind::Audio {
        return false;
    }
    let late = (-src_in).max(0.0);
    c.start += late;
    c.duration = (c.duration - late).max(MIN_CLIP);
    c.asset = asset;
    c.src_in = src_in.max(0.0);
    c.audio_stream = a.audio_streams.first().map_or(0, |s| s.index);
    if a.duration > 0.0 {
        c.duration = c.duration.min((a.duration - c.src_in) / c.rate(0.0).max(0.01)).max(MIN_CLIP);
    }
    c.name = if c.container_label.is_empty() { a.name() } else { format!("{} [{}]", a.name(), c.container_label) };
    if let Some(ti) = p.track_of(clip) {
        p.tracks[ti].sort();
    }
    true
}

/// A beat edit fills the frame: footage of another shape (a tall clip in a wide edit) is scaled up to cover
/// it, its animation scaled along so the motion stays the same.
fn cover_frame(p: &mut Project, fills: &[(Id, Id)]) {
    let (pw, ph) = (p.width.max(1) as f64, p.height.max(1) as f64);
    for &(asset, clip) in fills {
        let Some((aw, ah)) = p.asset(asset).filter(|a| a.width > 0 && a.height > 0).map(|a| (a.width as f64, a.height as f64)) else { continue };
        let (sx, sy) = (pw / aw, ph / ah);
        let k = sx.max(sy) / sx.min(sy);
        if k < 1.001 {
            continue;
        }
        if let Some(c) = p.clip_mut(clip) {
            c.scale.value *= k;
            c.scale.keys.iter_mut().for_each(|key| key.v *= k);
        }
    }
}

/// Footage in several slots plays a different stretch of itself in each: the k-th of n starts k/n of the
/// way into what is left after the slot's own length.
fn spread_repeats(p: &mut Project, fills: &[(Id, Id)]) {
    let mut done: Vec<Id> = Vec::new();
    for &(asset, _) in fills {
        if done.contains(&asset) {
            continue;
        }
        done.push(asset);
        let mine: Vec<Id> = fills.iter().filter(|f| f.0 == asset).map(|f| f.1).collect();
        let Some(dur) = p.asset(asset).filter(|a| a.kind == ClipKind::Video).map(|a| a.duration) else { continue };
        if mine.len() < 2 || dur <= 0.0 {
            continue;
        }
        let n = mine.len() as f64;
        for (k, id) in mine.iter().enumerate() {
            if let Some(c) = p.clip_mut(*id) {
                let room = (dur - c.src_len()).max(0.0);
                c.src_in = room * k as f64 / n;
            }
        }
    }
}

// ------------------------------------------------------------------------------------------ built-ins

/// Builds one built-in template for a `w` x `h` frame: sizes and positions follow the frame, so the same
/// template works in a wide, square or tall project.
struct Build {
    d: TemplateData,
    w: f64,
    h: f64,
    /// 1 at 1080 px on the short side.
    u: f64,
    next: Id,
}

/// (block name, start in the clip - negative = that far before its end, seconds).
type Anim<'a> = (&'a str, f64, f64);

impl Build {
    fn new(w: u32, h: u32) -> Build {
        let (w, h) = (w.max(16) as f64, h.max(16) as f64);
        Build { d: TemplateData::default(), w, h, u: w.min(h) / 1080.0, next: 1 }
    }
    fn push(&mut self, mut c: Clip, layer: u32, pos: (f64, f64), scale: f64, anims: &[Anim]) -> Id {
        c.id = self.next;
        self.next += 1;
        c.x = Animated::new(pos.0);
        c.y = Animated::new(pos.1);
        c.scale = Animated::new(scale);
        for &(name, t, dur) in anims {
            let t0 = if t < 0.0 { c.duration + t } else { t };
            if let Some(b) = find_block(&[], name) {
                c.place_block(&b, t0, Some(dur), self.w, self.h);
            }
        }
        let id = c.id;
        self.d.clips.push(c);
        self.d.layers.push(layer);
        id
    }
    #[allow(clippy::too_many_arguments)]
    fn slot(&mut self, name: &str, want: SlotWant, layer: u32, start: f64, dur: f64, pos: (f64, f64), scale: f64, anims: &[Anim]) -> Id {
        let mut c = Clip::new(0, ClipKind::Video, name, start, dur);
        c.container = true;
        c.container_label = name.to_string();
        let id = self.push(c, layer, pos, scale, anims);
        self.d.slots.push(Slot::new(name, id, want, SlotFit::Trim));
        id
    }
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, name: &str, text: &str, size: f64, layer: u32, start: f64, dur: f64, pos: (f64, f64), anims: &[Anim]) -> Id {
        let mut c = Clip::new(0, ClipKind::Text, name, start, dur);
        if let Some(t) = c.text.as_mut() {
            t.text = text.to_string();
            t.size = Animated::new(size * self.u);
            t.bold = true;
            t.align = 1;
        }
        c.exposed = vec!["text.text".into(), "text.color".into()];
        self.push(c, layer, pos, 1.0, anims)
    }
    #[allow(clippy::too_many_arguments)]
    fn rect(&mut self, name: &str, fill: [u8; 4], half: (f64, f64), layer: u32, start: f64, dur: f64, pos: (f64, f64), anims: &[Anim]) -> Id {
        let mut c = Clip::new(0, ClipKind::Shape, name, start, dur);
        if let Some(s) = c.shape.as_mut() {
            s.fill = fill;
            s.w = Animated::new(half.0);
            s.h = Animated::new(half.1);
            s.corner = (12.0 * self.u) as f32;
        }
        c.exposed = vec!["shape.fill".into()];
        self.push(c, layer, pos, 1.0, anims)
    }
    fn clip_mut(&mut self, id: Id) -> &mut Clip {
        self.d.clips.iter_mut().find(|c| c.id == id).expect("a clip this builder made")
    }
    /// A transition on the cut before `right`.
    fn cut(&mut self, right: Id, kind: TransitionKind, duration: f64, direction: u8) {
        self.d.transitions.push(Transition {
            id: 0,
            right,
            kind,
            duration,
            color: [0, 0, 0, 255],
            direction,
            ease: Ease::EaseInOut,
            edge: TransitionEdge::Cut,
            shader: String::new(),
            params: Vec::new(),
            curve: Default::default(),
        });
    }
    /// Make clip `id` a fixed part of the template (its song, a sound effect, the WASTED shot): not a slot,
    /// nothing to fill; its sample footage comes with the template.
    fn fix(&mut self, id: Id) {
        self.d.slots.retain(|s| s.clip != id);
        let c = self.clip_mut(id);
        c.container = false;
        c.container_label.clear();
    }
    /// Sample footage from the content library's `templates/<slug>` folder: `named` slots get their file,
    /// the other picture slots take `shots` in turn (a file in several slots is the same footage, linked).
    fn samples(&mut self, slug: &str, shots: &[&str], named: &[(&str, &str)]) {
        self.d.media = format!("templates/{slug}");
        let mut k = 0;
        for s in &self.d.slots {
            if s.want != SlotWant::Audio && !shots.is_empty() {
                k += 1;
                self.d.samples.push(Sample { clip: s.clip, file: shots[(k - 1) % shots.len()].to_string(), local: false });
            }
        }
        // the fixed parts, by clip name
        for &(name, file) in named {
            if let Some(c) = self.d.clips.iter().find(|c| c.name == name) {
                self.d.samples.push(Sample { clip: c.id, file: file.to_string(), local: false });
            }
        }
    }
    /// A fixed part whose media is yours only (`Sample.local`): used when it is in your media folder.
    fn local_sample(&mut self, name: &str, file: &str) {
        if let Some(c) = self.d.clips.iter().find(|c| c.name == name) {
            self.d.samples.push(Sample { clip: c.id, file: file.to_string(), local: true });
        }
    }
    fn done(mut self) -> TemplateData {
        self.d.add_cards(self.w as u32, self.h as u32);
        self.d
    }
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const INK: [u8; 4] = [16, 18, 24, 235];
const ACCENT: [u8; 4] = [255, 196, 46, 255];

pub const BUILTIN_NAMES: [&str; 14] = [
    "Photo Slideshow",
    "Beat Montage",
    "Title Intro",
    "Product Showcase",
    "Before and After",
    "Quote Card",
    "Lower Third Opener",
    "Vertical Short Opener",
    "End Card",
    "Photo Grid",
    // ---- edit templates (templates_edits.rs): beat-synced to the song they bring ----
    "Wasted",
    "Velocity Edit",
    "Hype Montage",
    "Meme Cut",
];

/// A built-in animation template for a `w` x `h` project (None = no such name).
pub fn builtin(name: &str, w: u32, h: u32) -> Option<TemplateData> {
    let mut b = Build::new(w, h);
    let (fw, fh, u) = (b.w, b.h, b.u);
    match BUILTIN_NAMES.iter().position(|n| n.eq_ignore_ascii_case(name.trim()))? {
        // photos that drift slowly; the first pops in, the next ones push in from alternating sides or dissolve
        0 => {
            let cuts = [(TransitionKind::Push, 0), (TransitionKind::Push, 1), (TransitionKind::CrossFade, 0)];
            for i in 0..4 {
                let t = i as f64 * 3.0;
                let anims: &[Anim] = if i == 0 { &[("Pop In", 0.0, 0.5), ("Ken Burns", 0.5, 2.5)] } else { &[("Ken Burns", 0.0, 3.0)] };
                let id = b.slot(&format!("Photo {}", i + 1), SlotWant::Image, 0, t, 3.0, (0.0, 0.0), 1.0, anims);
                if i > 0 {
                    b.cut(id, cuts[i - 1].0, 0.5, cuts[i - 1].1);
                }
            }
            b.text("Title", "Our Trip", 110.0, 1, 0.3, 2.7, (0.0, fh * 0.32), &[("Pop In", 0.0, 0.4), ("Fade Out", -0.4, 0.4)]);
        }
        // short shots on the beat: a punch on every cut, a shake on every second one
        1 => {
            for i in 0..6 {
                let t = i as f64 * 0.8;
                let hit: &[Anim] = if i % 2 == 0 { &[("Zoom Punch", 0.0, 0.4)] } else { &[("Shake", 0.0, 0.35)] };
                b.slot(&format!("Shot {}", i + 1), SlotWant::Video, 0, t, 0.8, (0.0, 0.0), 1.0, hit);
            }
            b.text("Hit", "GO", 220.0, 1, 4.0, 0.8, (0.0, 0.0), &[("Pop In", 0.0, 0.25), ("Shake", 0.25, 0.4)]);
        }
        // a title over slowly moving footage
        2 => {
            b.slot("Background", SlotWant::Any, 0, 0.0, 5.0, (0.0, 0.0), 1.0, &[("Fade In", 0.0, 0.6), ("Ken Burns", 0.6, 3.8), ("Fade Out", -0.6, 0.6)]);
            b.rect("Bar", ACCENT, (260.0 * u, 6.0 * u), 1, 0.4, 4.2, (0.0, 70.0 * u), &[("Slide In Left", 0.0, 0.6), ("Fade Out", -0.4, 0.4)]);
            b.text("Title", "Your Title", 150.0, 2, 0.5, 4.1, (0.0, -40.0 * u), &[("Pop In", 0.0, 0.45), ("Fade Out", -0.4, 0.4)]);
            b.text("Subtitle", "A short line under it", 54.0, 2, 1.0, 3.6, (0.0, 130.0 * u), &[("Fade In", 0.0, 0.5), ("Fade Out", -0.4, 0.4)]);
        }
        // a hero shot that drops in, two details from the sides, a name and a price tag
        3 => {
            let hero = b.slot("Hero", SlotWant::Any, 0, 0.0, 3.0, (0.0, 0.0), 1.0, &[("Drop In", 0.0, 0.6), ("Pulse", 1.2, 0.6)]);
            let d1 = b.slot("Detail 1", SlotWant::Any, 0, 3.0, 2.5, (0.0, 0.0), 1.0, &[("Slide In Right", 0.0, 0.5), ("Drift", 0.5, 2.0)]);
            let d2 = b.slot("Detail 2", SlotWant::Any, 0, 5.5, 2.5, (0.0, 0.0), 1.0, &[("Slide In Left", 0.0, 0.5), ("Drift", 0.5, 2.0)]);
            let _ = hero;
            b.cut(d1, TransitionKind::Push, 0.4, 0);
            b.cut(d2, TransitionKind::Push, 0.4, 1);
            b.text("Name", "Product Name", 96.0, 2, 0.6, 7.4, (-fw * 0.24, fh * 0.34), &[("Slide In Left", 0.0, 0.5), ("Fade Out", -0.4, 0.4)]);
            b.rect("Tag", ACCENT, (150.0 * u, 52.0 * u), 1, 1.2, 6.8, (fw * 0.3, fh * 0.34), &[("Elastic", 0.0, 0.7), ("Pop Out", -0.35, 0.35)]);
            let price = b.text("Price", "$29", 72.0, 2, 1.2, 6.8, (fw * 0.3, fh * 0.34), &[("Pop In", 0.0, 0.4), ("Fade Out", -0.35, 0.35)]);
            if let Some(t) = b.clip_mut(price).text.as_mut() {
                t.color = [20, 20, 20, 255];
            }
        }
        // two shots side by side with a divider, each under its label
        4 => {
            b.slot("Before", SlotWant::Any, 0, 0.0, 5.0, (0.0, 0.0), 1.0, &[("Fade In", 0.0, 0.5)]);
            let after = b.slot("After", SlotWant::Any, 1, 0.0, 5.0, (0.0, 0.0), 1.0, &[("Slide In Right", 0.6, 0.7)]);
            b.clip_mut(after).mask = Some(Mask {
                cx: Animated::new(fw / 4.0),
                rx: Animated::new(fw / 4.0),
                ry: Animated::new(fh / 2.0),
                enabled: true,
                ..Default::default()
            });
            b.rect("Divider", WHITE, (4.0 * u, fh / 2.0), 2, 0.6, 4.4, (0.0, 0.0), &[("Drop In", 0.0, 0.7)]);
            b.text("Before label", "BEFORE", 64.0, 3, 0.3, 4.7, (-fw / 4.0, fh * 0.38), &[("Fade In", 0.0, 0.4)]);
            b.text("After label", "AFTER", 64.0, 3, 1.3, 3.7, (fw / 4.0, fh * 0.38), &[("Pop In", 0.0, 0.4)]);
        }
        // a quote over dimmed footage, the author under a short line
        5 => {
            let bg = b.slot("Background", SlotWant::Any, 0, 0.0, 6.0, (0.0, 0.0), 1.0, &[("Ken Burns", 0.0, 6.0)]);
            b.clip_mut(bg).opacity = Animated::new(0.45);
            let q = b.text("Quote", "\u{201C}Write the quote here.\u{201D}", 84.0, 2, 0.4, 5.6, (0.0, -50.0 * u), &[("Fade In", 0.0, 0.8), ("Drift", 0.8, 4.0), ("Fade Out", -0.5, 0.5)]);
            if let Some(t) = b.clip_mut(q).text.as_mut() {
                t.italic = true;
                t.bold = false;
            }
            b.rect("Line", ACCENT, (90.0 * u, 4.0 * u), 1, 1.2, 4.8, (0.0, 80.0 * u), &[("Pop In", 0.0, 0.4), ("Fade Out", -0.5, 0.5)]);
            b.text("Author", "Someone Wise", 48.0, 2, 1.4, 4.6, (0.0, 140.0 * u), &[("Slide In Bottom", 0.0, 0.5), ("Fade Out", -0.5, 0.5)]);
        }
        // footage with a name bar that slides in and leaves again
        6 => {
            b.slot("Footage", SlotWant::Video, 0, 0.0, 6.0, (0.0, 0.0), 1.0, &[("Fade In", 0.0, 0.4)]);
            let (x, y) = (-fw * 0.26, fh * 0.34);
            b.rect("Bar", INK, (fw * 0.2, 62.0 * u), 1, 0.4, 5.0, (x, y), &[("Slide In Left", 0.0, 0.5), ("Slide Out Left", -0.5, 0.5)]);
            b.rect("Stripe", ACCENT, (6.0 * u, 62.0 * u), 2, 0.5, 4.9, (x - fw * 0.2, y), &[("Slide In Left", 0.0, 0.5), ("Slide Out Left", -0.5, 0.5)]);
            b.text("Name", "Full Name", 58.0, 3, 0.6, 4.7, (x, y - 20.0 * u), &[("Slide In Left", 0.0, 0.5), ("Fade Out", -0.4, 0.4)]);
            b.text("Role", "What they do", 36.0, 3, 0.85, 4.45, (x, y + 30.0 * u), &[("Fade In", 0.0, 0.5), ("Fade Out", -0.4, 0.4)]);
        }
        // a hook line that pops and shakes, then two fast clips with a caption
        7 => {
            b.slot("Clip 1", SlotWant::Video, 0, 0.0, 3.0, (0.0, 0.0), 1.0, &[("Zoom Punch", 0.0, 0.5)]);
            let c2 = b.slot("Clip 2", SlotWant::Video, 0, 3.0, 3.0, (0.0, 0.0), 1.0, &[("Slide In Bottom", 0.0, 0.45), ("Zoom Punch", 1.4, 0.5)]);
            b.cut(c2, TransitionKind::Push, 0.3, 2);
            let hook = b.text("Hook", "WAIT FOR IT", 130.0, 2, 0.0, 2.2, (0.0, -fh * 0.28), &[("Pop In", 0.0, 0.3), ("Shake", 0.3, 0.5), ("Pop Out", -0.3, 0.3)]);
            if let Some(t) = b.clip_mut(hook).text.as_mut() {
                t.box_color = [255, 60, 80, 255];
                t.box_padding = (22.0 * u) as f32;
            }
            let cap = b.text("Caption", "Say what happens", 64.0, 2, 3.2, 2.8, (0.0, fh * 0.3), &[("Slide In Bottom", 0.0, 0.4), ("Fade Out", -0.3, 0.3)]);
            if let Some(t) = b.clip_mut(cap).text.as_mut() {
                t.box_color = INK;
                t.box_padding = (16.0 * u) as f32;
            }
        }
        // a thank-you, two video cards for what to watch next, and a button
        8 => {
            b.rect("Backdrop", [18, 20, 30, 255], (fw / 2.0, fh / 2.0), 0, 0.0, 8.0, (0.0, 0.0), &[("Fade In", 0.0, 0.5)]);
            b.text("Thanks", "Thanks for watching", 96.0, 3, 0.3, 7.7, (0.0, -fh * 0.34), &[("Pop In", 0.0, 0.45)]);
            b.slot("Next video", SlotWant::Any, 1, 0.8, 7.2, (-fw * 0.24, fh * 0.02), 0.42, &[("Pop In", 0.0, 0.45), ("Pulse", 3.0, 0.6)]);
            b.slot("Best video", SlotWant::Any, 2, 1.1, 6.9, (fw * 0.24, fh * 0.02), 0.42, &[("Pop In", 0.0, 0.45), ("Pulse", 3.4, 0.6)]);
            b.rect("Button", [220, 40, 50, 255], (170.0 * u, 44.0 * u), 3, 1.6, 6.4, (0.0, fh * 0.36), &[("Elastic", 0.0, 0.7), ("Pulse", 2.6, 0.6)]);
            b.text("Button label", "SUBSCRIBE", 46.0, 4, 1.6, 6.4, (0.0, fh * 0.36), &[("Pop In", 0.0, 0.4)]);
        }
        10..=13 => edits::build(&mut b, BUILTIN_NAMES[10..].iter().position(|n| n.eq_ignore_ascii_case(name.trim()))?),
        // four photos that pop into a grid one after another
        _ => {
            b.rect("Backdrop", [22, 24, 30, 255], (fw / 2.0, fh / 2.0), 0, 0.0, 6.0, (0.0, 0.0), &[]);
            for i in 0..4 {
                let (cx, cy) = (if i % 2 == 0 { -0.25 } else { 0.25 }, if i < 2 { -0.25 } else { 0.25 });
                let t = 0.2 + i as f64 * 0.18;
                b.slot(&format!("Photo {}", i + 1), SlotWant::Image, 1 + i as u32, t, 6.0 - t, (fw * cx, fh * cy), 0.47, &[("Pop In", 0.0, 0.4), ("Pulse", 2.0 + i as f64 * 0.25, 0.5), ("Pop Out", -0.35, 0.35)]);
            }
            b.text("Title", "Highlights", 100.0, 5, 1.1, 4.9, (0.0, 0.0), &[("Elastic", 0.0, 0.7), ("Fade Out", -0.4, 0.4)]);
        }
    }
    Some(b.done())
}

/// The built-in templates as Gallery cards (made once, for a 1920 x 1080 frame): what their pictures are drawn from.
pub fn builtin_cards() -> &'static [crate::settings::Template] {
    static CARDS: std::sync::OnceLock<Vec<crate::settings::Template>> = std::sync::OnceLock::new();
    CARDS.get_or_init(|| {
        BUILTIN_NAMES
            .iter()
            .filter_map(|n| Some(crate::settings::Template { name: n.to_string(), json: serde_json::to_string(&builtin(n, 1920, 1080)?).ok()? }))
            .collect()
    })
}

/// A template by name: yours first (`Settings.templates`), then the built-in ones (made for `w` x `h`).
pub fn find(user: &[crate::settings::Template], name: &str, w: u32, h: u32) -> Option<TemplateData> {
    match user.iter().find(|t| t.name.eq_ignore_ascii_case(name.trim())) {
        Some(t) => TemplateData::decode(&t.json),
        None => builtin(name, w, h),
    }
}

// ------------------------------------------------------------------------------------------ sharing

/// What an export wrote.
#[derive(Debug, PartialEq)]
pub struct Exported {
    /// Footage files inside.
    pub files: usize,
    /// Clips that became empty slots (no footage asked for, or its file is gone).
    pub empty: usize,
}

fn is_abs(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || s.starts_with("\\\\")
}

fn base_name(s: &str) -> String {
    s.rsplit(['\\', '/']).next().unwrap_or(s).to_string()
}

/// No string of the header may be a path on the sender's computer (a LUT, a linked HTML file...): such a
/// string keeps its file name only.
fn scrub(v: &mut Value) {
    match v {
        Value::String(s) if is_abs(s) => *s = base_name(s),
        Value::Array(a) => a.iter_mut().for_each(scrub),
        Value::Object(o) => o.values_mut().for_each(scrub),
        _ => {}
    }
}

/// Write template `d` (called `name`) to `out` as one file. `footage`: its video files ride along; without
/// it - or for a file that is gone - those clips become empty slots. Sound and pictures always ride along. `thumb`: its Gallery picture.
pub fn export_package(name: &str, d: &TemplateData, footage: bool, thumb: Option<&Path>, out: &Path) -> Result<Exported, String> {
    let mut d = d.clone();
    let mut files: Vec<(String, PathBuf, u64, bool)> = Vec::new();
    let mut gone: Vec<Id> = Vec::new();
    let mut cut: Vec<(Id, f64)> = Vec::new(); // (asset, source seconds its trimmed copy starts at)
    let mut temps: Vec<PathBuf> = Vec::new();
    for (i, a) in d.assets.iter_mut().enumerate() {
        a.rel_path = None;
        if a.html.is_some() {
            // (its picture is re-made from the source inside the template; the path is only a cache name)
            a.path = format!("asset-{}", a.id);
            continue;
        }
        let src = PathBuf::from(&a.path);
        match std::fs::metadata(&src) {
            // (without footage, only video is left out: sound, music and pictures are the template's own)
            Ok(m) if (footage || a.kind != ClipKind::Video) && m.is_file() => {
                // only the part of the footage the template uses rides along
                let used = d.clips.iter().filter(|c| c.uses_asset() && c.asset == a.id).map(|c| (c.src_in, c.src_in + c.src_len()));
                let (lo, hi) = used.fold((f64::INFINITY, 0.0_f64), |(l, h), (a, b)| (l.min(a), h.max(b)));
                let (lo, hi) = ((lo - 0.25).max(0.0), hi + 0.25);
                let trimmed = (a.kind != ClipKind::Image && a.range.is_none() && lo.is_finite() && hi - lo < a.duration - 1.0)
                    .then(|| trim_copy(&src, lo, hi - lo, a.kind == ClipKind::Audio, i))
                    .flatten();
                let (n, src, len) = match trimmed {
                    Some((p, len)) => {
                        cut.push((a.id, lo));
                        a.duration = hi - lo;
                        temps.push(p.clone());
                        let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                        (format!("{i}-{}", base_name(&Path::new(&a.path).with_extension(ext).to_string_lossy())), p, len)
                    }
                    None => (format!("{i}-{}", base_name(&a.path)), src, m.len()),
                };
                files.push((n.clone(), src, len, false));
                a.path = n;
            }
            _ => gone.push(a.id),
        }
    }
    for c in d.clips.iter_mut().filter(|c| c.uses_asset()) {
        if let Some(&(_, lo)) = cut.iter().find(|x| x.0 == c.asset) {
            c.src_in = (c.src_in - lo).max(0.0);
        }
    }
    let stripped: Vec<Id> = d.clips.iter().filter(|c| c.uses_asset() && gone.contains(&c.asset)).map(|c| c.id).collect();
    let empty = d.clips.iter().filter(|c| stripped.contains(&c.id) && c.kind != ClipKind::Audio).count();
    for id in stripped {
        d.strip_footage(id);
    }
    d.list_empty_slots();
    if let Some(m) = thumb.and_then(|t| std::fs::metadata(t).ok().map(|m| (t, m.len()))) {
        files.push(("thumb.png".into(), m.0.to_path_buf(), m.1, true));
    }
    let list: Vec<Value> = files.iter().map(|(n, _, size, thumb)| json!({"name": n, "size": size, "thumb": thumb})).collect();
    let mut head = json!({"format": FORMAT, "version": 1, "name": name, "files": list,
                          "data": serde_json::to_value(&d).map_err(|e| e.to_string())?});
    scrub(&mut head);
    let head = serde_json::to_vec(&head).map_err(|e| e.to_string())?;
    let tmp = out.with_extension("part");
    let write = || -> std::io::Result<()> {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        f.write_all(MAGIC)?;
        f.write_all(&(head.len() as u64).to_le_bytes())?;
        f.write_all(&head)?;
        for (n, src, size, _) in &files {
            let got = std::io::copy(&mut std::fs::File::open(src)?.take(*size), &mut f)?;
            if got != *size {
                return Err(std::io::Error::other(format!("{n} changed while it was read")));
            }
        }
        f.flush()
    };
    let wrote = write();
    temps.iter().for_each(|t| drop(std::fs::remove_file(t)));
    if let Err(e) = wrote {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", out.display()));
    }
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(Exported { files: files.iter().filter(|f| !f.3).count(), empty })
}

/// `len` seconds of `src` from `from`, re-encoded into a temp file (H.264 / AAC, or AAC alone for a sound):
/// (its path, its size). None when ffmpeg is missing or fails - the whole file rides along instead.
fn trim_copy(src: &Path, from: f64, len: f64, sound: bool, i: usize) -> Option<(PathBuf, u64)> {
    let ff = crate::media::ffpipe::ffmpeg_exe()?;
    let stem = src.file_stem()?.to_string_lossy().into_owned();
    let out = std::env::temp_dir().join(format!("se-template-{}-{i}-{stem}.{}", std::process::id(), if sound { "m4a" } else { "mp4" }));
    let mut c = crate::media::ffpipe::command(&ff);
    c.args(["-v", "error", "-y", "-ss", &format!("{from:.3}"), "-i"]).arg(src).args(["-t", &format!("{len:.3}"), "-map", "0:v:0?", "-map", "0:a?"]);
    if sound {
        c.args(["-vn", "-c:a", "aac", "-b:a", "192k"]);
    } else {
        c.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k"]);
    }
    let ok = c.arg(&out).stdin(std::process::Stdio::null()).output().ok()?.status.success();
    let size = std::fs::metadata(&out).ok().filter(|m| ok && m.len() > 0)?.len();
    Some((out, size))
}

/// Read a `.setemplate` file into the gallery at `root`: its footage is unpacked into
/// `templates\footage\<stamp>-<name>\`, the template is saved (a taken name is numbered) and clips whose
/// footage is not in the file become empty slots.
pub fn import_package(root: &Path, file: &Path) -> Result<Item, String> {
    let err = |e: std::io::Error| format!("{}: {e}", file.display());
    let mut f = std::io::BufReader::new(std::fs::File::open(file).map_err(err)?);
    let mut magic = [0u8; 12];
    f.read_exact(&mut magic).map_err(err)?;
    if magic != MAGIC {
        return Err(format!("{}: not a template file", file.display()));
    }
    let mut len = [0u8; 8];
    f.read_exact(&mut len).map_err(err)?;
    let len = u64::from_le_bytes(len);
    if len > 256 << 20 {
        return Err(format!("{}: not a template file", file.display()));
    }
    let mut head = vec![0u8; len as usize];
    f.read_exact(&mut head).map_err(err)?;
    let head: Value = serde_json::from_slice(&head).map_err(|e| format!("{}: {e}", file.display()))?;
    if head.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(format!("{}: not a template file", file.display()));
    }
    let name = head.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or("Template").to_string();
    let mut d: TemplateData = serde_json::from_value(head.get("data").cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_millis()).unwrap_or(0);
    let dir = root.join(Kind::Template.dir()).join("footage").join(format!("{stamp}-{}", gallery::slug(&name)));
    let mut thumb: Option<PathBuf> = None;
    let mut got: Vec<(String, PathBuf)> = Vec::new();
    for e in head.get("files").and_then(Value::as_array).into_iter().flatten() {
        let n = base_name(e.get("name").and_then(Value::as_str).unwrap_or(""));
        let size = e.get("size").and_then(Value::as_u64).unwrap_or(0);
        if n.is_empty() || n == "." || n == ".." {
            return Err(format!("{}: a file inside has no name", file.display()));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let dest = dir.join(&n);
        let mut o = std::fs::File::create(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        if std::io::copy(&mut (&mut f).take(size), &mut o).map_err(err)? != size {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("{}: the file is cut short", file.display()));
        }
        if e.get("thumb").and_then(Value::as_bool) == Some(true) {
            thumb = Some(dest);
        } else {
            got.push((n, dest));
        }
    }
    // footage that came along gets its new place; anything else that is not on this PC is an empty slot
    let mut gone: Vec<Id> = Vec::new();
    for a in &mut d.assets {
        a.rel_path = None;
        if a.html.is_some() {
            continue;
        }
        match got.iter().find(|(n, _)| *n == a.path) {
            Some((_, p)) => a.path = p.to_string_lossy().into_owned(),
            None => gone.push(a.id),
        }
    }
    let stripped: Vec<Id> = d.clips.iter().filter(|c| c.uses_asset() && gone.contains(&c.asset)).map(|c| c.id).collect();
    for id in stripped {
        d.strip_footage(id);
    }
    d.list_empty_slots();
    let item = gallery::save(root, Kind::Template, &name, &[], serde_json::to_value(&d).map_err(|e| e.to_string())?, Collide::Number)?;
    if let Some(t) = thumb {
        let _ = std::fs::rename(t, thumb_path(&item.path));
    }
    let _ = std::fs::remove_dir(&dir); // (only when nothing but the picture was in it)
    Ok(item)
}

/// Where a saved template's Gallery picture lives: beside its `.json`.
pub fn thumb_path(item_json: &Path) -> PathBuf {
    item_json.with_extension("png")
}

#[path = "templates_edits.rs"]
mod edits;

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
