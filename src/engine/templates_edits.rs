//! ---- ws:edit-templates ----
//! Ready-made beat edits in the CapCut-template style: drop in a song and your clips, and the cuts land on
//! the song's beats (`BeatSync`: each template is laid out on a 120 BPM grid and retimed to the song when
//! placed, its drop on the song's drop). Everything here is native - slots, animation blocks, keyframed
//! speed, effect windows, transitions (the white flash is a Fade to Color, the zoom one the library's
//! Zoom Momentum shader) and a low-pass bus whose cutoff opens on the drop. Text cards are text clips.
//!
//! Every shot keeps moving: a slow push under it all, a punch / shake / speed ramp on the hits.

use super::*;

/// One beat of the grid every edit template is laid out on (120 BPM).
const BEAT: f64 = 0.5;
const BPM: f64 = 120.0;

/// The Zoom Momentum transition (also in the online library), built into the edit templates.
const ZOOM_MOMENTUM: &str = include_str!("../../library/transitions/zoom-momentum.glsl");

const SNAP: Ease = Ease::Bezier { x1: 0.9, y1: 0.0, x2: 0.1, y2: 1.0 };
const RED: [u8; 4] = [214, 24, 24, 255];
const YELLOW: [u8; 4] = [255, 210, 40, 255];

fn key(f: f64, v: BlockVal, ease: Ease) -> BlockKey {
    BlockKey { f, v, ease }
}
fn track(prop: &str, keys: Vec<BlockKey>) -> BlockTrack {
    BlockTrack { prop: prop.into(), keys }
}
fn kb(name: &str, dur: f64, tracks: Vec<BlockTrack>) -> KeyBlock {
    KeyBlock { name: name.into(), dur, tracks, gen: None }
}

/// A slow zoom that never stops: scale x1 -> x`by` over the block.
fn push_in(by: f64) -> KeyBlock {
    kb("Push In", 2.0, vec![track("scale", vec![key(0.0, BlockVal::Mul(1.0), Ease::Linear), key(1.0, BlockVal::Mul(by), Ease::Linear)])])
}
/// Accelerating into the cut: the zoom speeds up toward the end (feeds a zoom transition).
fn rush_in(by: f64) -> KeyBlock {
    kb("Rush In", 1.0, vec![track("scale", vec![key(0.0, BlockVal::Mul(1.0), Ease::EaseIn), key(1.0, BlockVal::Mul(by), Ease::Linear)])])
}
/// A title slamming in: big and see-through to its size.
fn slam() -> KeyBlock {
    kb(
        "Slam",
        0.3,
        vec![
            track("scale", vec![key(0.0, BlockVal::Mul(1.7), Ease::EaseOut), key(1.0, BlockVal::Mul(1.0), Ease::Linear)]),
            track("opacity", vec![key(0.0, BlockVal::Abs(0.0), Ease::EaseOut), key(0.5, BlockVal::Abs(1.0), Ease::Linear)]),
        ],
    )
}
/// Slow motion from the hit on: x1 to x0.25 at once, then held.
fn slow_mo() -> KeyBlock {
    kb(
        "Slow-mo",
        1.5,
        vec![track("speed", vec![key(0.0, BlockVal::Mul(1.0), Ease::EaseOut), key(0.08, BlockVal::Mul(0.25), Ease::Hold), key(1.0, BlockVal::Mul(0.25), Ease::Linear)])],
    )
}
/// A velocity ramp: slow on the hit, then speeding up into the next cut.
fn velocity() -> KeyBlock {
    kb(
        "Velocity",
        1.0,
        vec![track(
            "speed",
            vec![key(0.0, BlockVal::Mul(0.35), Ease::Linear), key(0.42, BlockVal::Mul(0.35), Ease::EaseIn), key(1.0, BlockVal::Mul(2.8), Ease::Linear)],
        )],
    )
}
/// A freeze-frame: plays, then stops dead (0.01x) and holds.
fn freeze_at(f: f64) -> KeyBlock {
    kb(
        "Freeze Frame",
        2.0,
        vec![track(
            "speed",
            vec![key(0.0, BlockVal::Mul(1.0), Ease::Hold), key(f, BlockVal::Abs(0.01), Ease::Hold), key(1.0, BlockVal::Abs(0.01), Ease::Linear)],
        )],
    )
}

/// How a cut changes picture.
#[derive(Clone, Copy)]
enum Cut {
    /// A white flash.
    Flash(f64),
    /// The Zoom Momentum shader.
    Zoom(f64),
    /// A whip pan: a fast push with a snap ease (direction 0 left, 1 right).
    Whip(f64, u8),
    Hard,
}

impl Build {
    fn anim(&mut self, id: Id, b: &KeyBlock, t: f64, dur: f64) {
        let (w, h) = (self.w, self.h);
        self.clip_mut(id).place_block(b, t, Some(dur), w, h);
    }
    fn named(&mut self, id: Id, name: &str, t: f64, dur: f64) {
        if let Some(b) = find_block(&[], name) {
            self.anim(id, &b, t, dur);
        }
    }
    /// An effect switched on for clip-local [start, start + len) (`len` 0 = to the end).
    fn fx(&mut self, id: Id, kind: EffectKind, params: &[(usize, f64)], start: f64, len: f64) {
        let mut e = Effect::new(kind);
        for &(i, v) in params {
            if let Some(p) = e.params.get_mut(i) {
                *p = Animated::new(v);
            }
        }
        (e.start, e.len) = (start, len);
        self.clip_mut(id).effects.push(e);
    }
    fn trans(&mut self, right: Id, cut: Cut) {
        let (kind, dur, color, dir, ease, shader) = match cut {
            Cut::Hard => return,
            Cut::Flash(d) => (TransitionKind::FadeToColor, d, [255, 255, 255, 255], 0, Ease::Linear, String::new()),
            Cut::Zoom(d) => (TransitionKind::Shader, d, [0, 0, 0, 255], 0, Ease::Linear, ZOOM_MOMENTUM.to_string()),
            Cut::Whip(d, dir) => (TransitionKind::Push, d, [0, 0, 0, 255], dir, SNAP, String::new()),
        };
        self.d.transitions.push(Transition {
            id: 0,
            right,
            kind,
            duration: dur,
            color,
            direction: dir,
            ease,
            edge: TransitionEdge::Cut,
            shader,
            params: Vec::new(),
            curve: Default::default(),
        });
    }
    /// A shot slot from beat `at` for `len` beats, always moving: a punch and / or a shake on the cut,
    /// then a slow push to its end. Scaled up a touch so a shake never shows an edge.
    fn shot(&mut self, n: usize, at: f64, len: f64, punch: bool, shake: bool) -> Id {
        let (t, d) = (at * BEAT, len * BEAT);
        let id = self.slot(&format!("Clip {n}"), SlotWant::Video, 0, t, d, (0.0, 0.0), 1.1, &[]);
        let hit = if punch { (0.3_f64).min(d * 0.45) } else { 0.0 };
        if punch {
            self.named(id, "Zoom Punch", 0.0, hit);
        }
        if shake {
            self.named(id, "Shake", 0.0, (0.35_f64).min(d));
        }
        self.anim(id, &push_in(if len >= 4.0 { 1.14 } else { 1.08 }), hit, d - hit);
        id
    }
    /// A sound slot (the song when `song`): an audio container over [start, start + dur).
    fn sound(&mut self, name: &str, hint: &str, layer: u32, start: f64, dur: f64, hit: f64) -> Id {
        let mut c = Clip::new(0, ClipKind::Audio, name, start, dur);
        c.container = true;
        c.container_label = name.to_string();
        let id = self.push(c, layer, (0.0, 0.0), 1.0, &[]);
        let mut s = Slot::new(name, id, SlotWant::Audio, SlotFit::Trim);
        (s.hint, s.hit) = (hint.to_string(), hit);
        self.d.slots.push(s);
        id
    }
    /// The song slot over the whole template, `drop` seconds in, fading out over its last second.
    fn song(&mut self, len: f64, drop: f64, bus: Id) -> Id {
        let id = self.sound("Song", "", 0, 0.0, len, 0.0);
        // the song is the first slot you fill
        if let Some(s) = self.d.slots.pop() {
            self.d.slots.insert(0, s);
        }
        let c = self.clip_mut(id);
        c.fade_out = 1.0;
        c.bus = bus;
        c.audio_role = AudioRole::Music;
        self.d.sync = Some(BeatSync { bpm: BPM, drop, song: id });
        id
    }
    /// A bus whose low-pass muffles the song until `drop`, opening over the last beat before it.
    fn muffle_bus(&mut self, drop: f64) -> Id {
        let id = 1_000_000 + self.next;
        self.next += 1;
        let mut f = AudioFilter::new(FilterKind::LowPass);
        let k = |t: f64, v: f64, ease: Ease| Keyframe { t, v, ease };
        f.params[0].keys = vec![
            k(0.0, 380.0, Ease::Hold),
            k(drop - BEAT, 380.0, Ease::EaseIn),
            k(drop - 0.03, 1600.0, Ease::Hold),
            k(drop, 20000.0, Ease::Hold),
        ];
        f.params[1] = Animated::new(1.4); // a little resonance: the "through the wall" phonk sound
        self.d.buses.push(Bus { id, name: "Muffle until the drop".into(), filters: vec![f], ..Default::default() });
        id
    }
    /// A full-width title with an outline, at `y`, from `start` for `dur`.
    #[allow(clippy::too_many_arguments)]
    fn title(&mut self, text: &str, font: &str, size: f64, color: [u8; 4], layer: u32, start: f64, dur: f64, y: f64) -> Id {
        let id = self.text(text, text, size, layer, start, dur, (0.0, y), &[]);
        let u = self.u;
        if let Some(t) = self.clip_mut(id).text.as_mut() {
            t.font = font.to_string();
            t.color = color;
            t.outline_color = [0, 0, 0, 255];
            t.outline_width = Animated::new(6.0 * u);
        }
        id
    }
}

/// Build edit template `i` (0 = Phonk Synced Edit, then Velocity Edit, Hype Montage, Meme Cut).
pub(super) fn build(b: &mut Build, i: usize) {
    match i {
        0 => phonk(b),
        1 => velocity_edit(b),
        2 => hype(b),
        _ => meme(b),
    }
}

/// Muffled build-up, then on the drop: the muffle opens, the shot goes grey and slow under a WASTED card
/// with the real sound, and a flash into fast cuts with punches, shakes and zoom transitions.
fn phonk(b: &mut Build) {
    let (fw, u) = (b.w, b.u);
    let drop_beat = 16.0;
    let drop = drop_beat * BEAT;
    // before the drop: moody, longer shots that push in, zoom cuts
    let pre: [(f64, f64, Cut); 5] =
        [(0.0, 4.0, Cut::Hard), (4.0, 4.0, Cut::Zoom(0.35)), (8.0, 2.0, Cut::Zoom(0.3)), (10.0, 2.0, Cut::Flash(0.16)), (12.0, 2.0, Cut::Zoom(0.3))];
    let mut n = 1;
    for (k, &(at, len, cut)) in pre.iter().enumerate() {
        let id = b.shot(n, at, len, k >= 2, false);
        if k == 0 {
            b.named(id, "Fade In", 0.0, 0.4);
        }
        b.trans(id, cut);
        n += 1;
    }
    // the wasted shot: 2 beats of action, then the drop hits - slow-mo, grey and red, a shake, a slow push
    let (ws, wl) = (14.0, 6.0);
    let w = b.slot("Wasted shot", SlotWant::Video, 0, ws * BEAT, wl * BEAT, (0.0, 0.0), 1.1, &[]);
    let hit = drop - ws * BEAT;
    b.anim(w, &push_in(1.06), 0.0, hit);
    b.anim(w, &slow_mo(), hit, wl * BEAT - hit);
    b.named(w, "Shake", hit, 0.45);
    b.anim(w, &push_in(1.16), hit + 0.45, wl * BEAT - hit - 0.45);
    b.fx(w, EffectKind::Color, &[(0, -0.05), (1, 1.3), (2, 0.1)], hit, 0.0);
    b.fx(w, EffectKind::Tint, &[(0, 170.0), (1, 20.0), (2, 20.0), (3, 0.2)], hit, 0.0);
    b.fx(w, EffectKind::Vignette, &[(0, 0.7), (2, 0.8)], hit, 0.0);
    b.trans(w, Cut::Zoom(0.3));
    // the WASTED card: a dark band across the frame and the word, tilted like the game's
    let card = (wl - (drop_beat - ws)) * BEAT;
    let band = b.rect("Wasted band", [0, 0, 0, 165], (fw * 0.55, 74.0 * u), 1, drop, card, (0.0, 0.0), &[("Fade In", 0.0, 0.2)]);
    let word = b.title("wasted", "Montserrat", 150.0, RED, 2, drop, card, -6.0 * u);
    let s = slam();
    b.anim(word, &s, 0.0, 0.3);
    b.anim(word, &push_in(1.06), 0.3, card - 0.3);
    for id in [band, word] {
        b.clip_mut(id).rotation = Animated::new(-2.0);
    }
    b.clip_mut(band).shape.as_mut().expect("a rect").corner = 0.0;
    // after the drop: a flash out of the card, then fast cuts on every beat
    let post: [f64; 13] = [2.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 4.0];
    let mut at = ws + wl;
    for (k, &len) in post.iter().enumerate() {
        let last = k == post.len() - 1;
        let id = b.shot(n, at, len, true, k % 2 == 0 || (len < 2.0 && k > 5));
        let cut = match k {
            0 => Cut::Flash(0.4),
            _ if last => Cut::Zoom(0.35),
            _ if len >= 2.0 => if k % 2 == 1 { Cut::Zoom(0.3) } else { Cut::Flash(0.18) },
            _ if k % 2 == 0 => Cut::Flash(0.12),
            _ => Cut::Hard,
        };
        b.trans(id, cut);
        if len >= 2.0 || k % 2 == 0 {
            b.fx(id, EffectKind::Color, &[(1, 1.12), (2, 1.2)], 0.0, 0.0);
        }
        if last {
            b.named(id, "Fade Out", len * BEAT - 0.6, 0.6);
        }
        at += len;
        n += 1;
    }
    let end = at * BEAT;
    let bus = b.muffle_bus(drop);
    b.song(end, drop, bus);
    // the real sound: its hit on the drop (the slot finds a file called "wasted" in your sounds)
    b.sound("Wasted sound", "wasted", 1, drop - 0.6, 4.5, 0.6);
    b.d.thumb_t = drop + 0.5;
}

/// Speed ramps on every shot: slow on the hit, rushing into a zoom cut; a flash every bar.
fn velocity_edit(b: &mut Build) {
    let drop = 8.0 * BEAT;
    let mut n = 1;
    let mut at = 0.0;
    // two slow-motion shots to set it up
    for k in 0..2 {
        let id = b.slot(&format!("Clip {n}"), SlotWant::Video, 0, at * BEAT, 4.0 * BEAT, (0.0, 0.0), 1.1, &[]);
        b.anim(id, &slow_mo(), 0.0, 0.6);
        b.anim(id, &push_in(1.1), 0.0, 4.0 * BEAT - 0.5);
        b.anim(id, &rush_in(1.25), 4.0 * BEAT - 0.5, 0.5);
        if k == 0 {
            b.named(id, "Fade In", 0.0, 0.4);
        } else {
            b.trans(id, Cut::Zoom(0.3));
        }
        at += 4.0;
        n += 1;
    }
    for k in 0..12 {
        let d = 2.0 * BEAT;
        let id = b.slot(&format!("Clip {n}"), SlotWant::Video, 0, at * BEAT, d, (0.0, 0.0), 1.1, &[]);
        b.anim(id, &velocity(), 0.0, d);
        b.named(id, "Zoom Punch", 0.0, 0.25);
        b.anim(id, &push_in(1.05), 0.25, d * 0.6 - 0.25);
        b.anim(id, &rush_in(1.22), d * 0.6, d * 0.4);
        if k % 3 == 0 {
            b.named(id, "Shake", 0.0, 0.3);
        }
        b.trans(id, if k % 4 == 0 { Cut::Flash(if k == 0 { 0.35 } else { 0.18 }) } else { Cut::Zoom(0.28) });
        b.fx(id, EffectKind::Color, &[(1, 1.1), (2, 1.15)], 0.0, 0.0);
        if k == 11 {
            b.named(id, "Fade Out", d - 0.4, 0.4);
        }
        at += 2.0;
        n += 1;
    }
    b.song(at * BEAT, drop, 0);
    b.d.thumb_t = drop + 0.3;
}

/// Whip pans between shots, a zoom punch on every beat, a shake on every bar, a big title on the drop.
fn hype(b: &mut Build) {
    let u = b.u;
    let drop = 8.0 * BEAT;
    let mut n = 1;
    let mut at = 0.0;
    let lens: [f64; 14] = [4.0, 4.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 4.0];
    for (k, &len) in lens.iter().enumerate() {
        let id = b.shot(n, at, len, true, at >= 8.0 && (at as i64) % 4 == 0);
        // a second punch on the off beat of every two-beat shot
        if len == 2.0 {
            b.named(id, "Zoom Punch", BEAT, 0.3);
        }
        match k {
            0 => b.named(id, "Fade In", 0.0, 0.3),
            2 => b.trans(id, Cut::Flash(0.3)),
            _ => b.trans(id, Cut::Whip(0.24, (k % 2) as u8)),
        }
        if k == lens.len() - 1 {
            b.named(id, "Fade Out", len * BEAT - 0.5, 0.5);
        }
        at += len;
        n += 1;
    }
    let ready = b.title("READY?", "Montserrat", 110.0, [255, 255, 255, 255], 1, 0.4, drop - 0.4, 0.0);
    b.named(ready, "Pop In", 0.0, 0.3);
    b.named(ready, "Pulse", 1.6, 0.5);
    b.named(ready, "Pop Out", drop - 0.4 - 0.25, 0.25);
    let go = b.title("LET'S GO", "Montserrat", 170.0, [20, 20, 20, 255], 1, drop, 4.0 * BEAT, 0.0);
    if let Some(t) = b.clip_mut(go).text.as_mut() {
        t.box_color = YELLOW;
        t.box_padding = (26.0 * u) as f32;
        t.outline_width = Animated::new(0.0);
    }
    b.named(go, "Pop In", 0.0, 0.25);
    b.named(go, "Shake", 0.25, 0.4);
    b.named(go, "Pop Out", 4.0 * BEAT - 0.25, 0.25);
    b.song(at * BEAT, drop, 0);
    b.d.thumb_t = drop + 0.4;
}

/// Each shot plays, then freezes on a flash with a zoom, a shake and a caption that pops.
fn meme(b: &mut Build) {
    let (fh, u) = (b.h, b.u);
    let words = ["WAIT...", "BRUH", "NAH", "OH NO", "WHAT", "LOL"];
    let drop = 8.0 * BEAT;
    for (k, word) in words.iter().enumerate() {
        let (at, len) = (k as f64 * 4.0, 4.0);
        let (t, d) = (at * BEAT, len * BEAT);
        let id = b.slot(&format!("Clip {}", k + 1), SlotWant::Video, 0, t, d, (0.0, 0.0), 1.05, &[]);
        // plays for a beat and a half, then stops dead on the beat
        let f = 1.5 * BEAT;
        b.anim(id, &freeze_at(f / d), 0.0, d);
        b.anim(id, &push_in(1.04), 0.0, f);
        b.named(id, "Zoom Punch", f, 0.12);
        b.anim(id, &kb("Zoom In", 0.3, vec![track("scale", vec![key(0.0, BlockVal::Mul(1.0), Ease::EaseOut), key(1.0, BlockVal::Mul(1.35), Ease::Linear)])]), f + 0.12, 0.3);
        b.named(id, "Shake", f + 0.12, 0.4);
        b.anim(id, &push_in(1.06), f + 0.42, d - f - 0.42);
        // the frozen frame, deep-fried
        b.fx(id, EffectKind::Color, &[(1, 1.4), (2, 1.6)], f, 0.0);
        b.trans(id, if k == 0 { Cut::Hard } else { Cut::Whip(0.22, (k % 2) as u8) });
        // a camera flash on the freeze
        let flash = b.rect("Flash", [255, 255, 255, 255], (b.w / 2.0, fh / 2.0), 1, t + f, 0.25, (0.0, 0.0), &[("Fade Out", 0.0, 0.25)]);
        b.clip_mut(flash).shape.as_mut().expect("a rect").corner = 0.0;
        let cap = b.title(word, "Impact", 150.0, [255, 255, 255, 255], 2, t + f, d - f, fh * 0.3);
        if let Some(s) = b.clip_mut(cap).text.as_mut() {
            s.outline_width = Animated::new(9.0 * u);
            s.bold = false;
        }
        b.named(cap, "Pop In", 0.0, 0.2);
        b.named(cap, "Wiggle", 0.2, d - f - 0.2);
    }
    b.song(6.0 * 4.0 * BEAT, drop, 0);
    b.d.thumb_t = 1.5 * BEAT + 0.4;
}

#[cfg(test)]
#[path = "templates_edits_tests.rs"]
mod tests;
