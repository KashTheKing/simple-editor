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
}

impl SlotWant {
    pub const ALL: [SlotWant; 3] = [SlotWant::Any, SlotWant::Image, SlotWant::Video];
    pub fn name(self) -> &'static str {
        match self {
            SlotWant::Any => "any",
            SlotWant::Image => "image",
            SlotWant::Video => "video",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SlotWant::Any => "Any",
            SlotWant::Image => "Image",
            SlotWant::Video => "Video",
        }
    }
    pub fn parse(s: &str) -> Option<SlotWant> {
        Self::ALL.into_iter().find(|w| w.name().eq_ignore_ascii_case(s.trim()))
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
            .map(|(i, c)| Slot {
                name: if c.container_label.is_empty() { format!("Slot {}", i + 1) } else { c.container_label.clone() },
                clip: c.id,
                want: SlotWant::Any,
                fit: SlotFit::Trim,
                card: 0,
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
            None => self.slots.push(Slot { name: name.to_string(), clip, want, fit, card: 0 }),
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
                slots.push(Slot { name, clip: c.id, want: SlotWant::Any, fit: SlotFit::Trim, card: 0 });
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
            let Some(i) = self.clips.iter().position(|c| c.id == clip && c.is_empty_container()) else { continue };
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
    let slots = d.slot_list();
    let before: Vec<Id> = p.tracks.iter().map(|t| t.id).collect();
    let (mut ids, map) = p.place_template(d.clips, d.assets, d.transitions, &d.layers, at);
    for &(i, asset) in fills {
        let Some(s) = slots.get(i) else { continue };
        let Some(&clip) = map.get(&s.clip) else { continue };
        if p.fill_slot(clip, asset, s.fit == SlotFit::Stretch, &ids) {
            if let Some(card) = map.get(&s.card) {
                for t in &mut p.tracks {
                    t.clips.retain(|c| c.id != *card);
                }
            }
        }
    }
    while let Some(i) = p.tracks.iter().position(|t| t.clips.is_empty() && !before.contains(&t.id)) {
        p.remove_track(i);
    }
    ids.retain(|id| p.clip(*id).is_some());
    ids
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
        self.d.slots.push(Slot { name: name.to_string(), clip: id, want, fit: SlotFit::Trim, card: 0 });
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
    fn done(mut self) -> TemplateData {
        self.d.add_cards(self.w as u32, self.h as u32);
        self.d
    }
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const INK: [u8; 4] = [16, 18, 24, 235];
const ACCENT: [u8; 4] = [255, 196, 46, 255];

pub const BUILTIN_NAMES: [&str; 10] = [
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

/// Write template `d` (called `name`) to `out` as one file. `footage`: the files its clips use ride along;
/// without it - or for a file that is gone - those clips become empty slots. `thumb`: its Gallery picture.
pub fn export_package(name: &str, d: &TemplateData, footage: bool, thumb: Option<&Path>, out: &Path) -> Result<Exported, String> {
    let mut d = d.clone();
    let mut files: Vec<(String, PathBuf, u64, bool)> = Vec::new();
    let mut gone: Vec<Id> = Vec::new();
    for (i, a) in d.assets.iter_mut().enumerate() {
        a.rel_path = None;
        if a.html.is_some() {
            // (its picture is re-made from the source inside the template; the path is only a cache name)
            a.path = format!("asset-{}", a.id);
            continue;
        }
        let src = PathBuf::from(&a.path);
        match std::fs::metadata(&src) {
            Ok(m) if footage && m.is_file() => {
                let n = format!("{i}-{}", base_name(&a.path));
                files.push((n.clone(), src, m.len(), false));
                a.path = n;
            }
            _ => gone.push(a.id),
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
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", out.display()));
    }
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(Exported { files: files.iter().filter(|f| !f.3).count(), empty })
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

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
