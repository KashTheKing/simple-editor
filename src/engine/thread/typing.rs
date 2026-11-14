//! Typing out a message: which key is pressed when (deterministic: typos come from a hash of the text),
//! and the page parts that show it (a span per key that appears at its moment, a caret).

use super::*;

/// One key of a typing: when it lands (seconds after the typing starts) and, for a typo, when backspace
/// takes it away again.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub ch: String,
    pub at: f64,
    pub until: Option<f64>,
}

fn hash_at(text: &str, i: usize) -> u32 {
    text.bytes().fold(2166136261u32 ^ i as u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

/// The wrong key next to `c` on the keyboard.
fn near(c: char) -> char {
    const ROW: &str = "qwertyuiopasdfghjklzxcvbnm";
    let low = c.to_ascii_lowercase();
    match ROW.find(low) {
        Some(p) => {
            let w = ROW.as_bytes()[(p + 1) % ROW.len()] as char;
            if c.is_uppercase() { w.to_ascii_uppercase() } else { w }
        }
        None => '.',
    }
}

/// The keys of `text` typed at `cps` characters a second, and the seconds the whole typing takes. With
/// `typos`, now and then a wrong key goes in, is noticed after a beat and deleted before the right one.
pub fn type_plan(text: &str, cps: f64, typos: bool) -> (Vec<Key>, f64) {
    let step = 1.0 / cps.clamp(1.0, 80.0);
    let chars: Vec<char> = text.chars().collect();
    let (mut t, mut keys) = (0.0, Vec::new());
    for (i, &c) in chars.iter().enumerate() {
        if typos && i >= 2 && i + 1 < chars.len() && c.is_alphabetic() && hash_at(text, i) % 17 == 0 {
            let gone = t + step + 0.18;
            keys.push(Key { ch: near(c).to_string(), at: t, until: Some(gone) });
            t = gone + step * 0.5;
        }
        keys.push(Key { ch: c.to_string(), at: t, until: None });
        t += step;
    }
    (keys, t)
}

/// What the box shows `t` seconds after the typing started.
pub fn typed_at(keys: &[Key], t: f64) -> String {
    keys.iter().filter(|k| k.at <= t && k.until.is_none_or(|u| t < u)).map(|k| k.ch.as_str()).collect()
}

impl Thread {
    /// Is this item typed out in a box before it shows?
    pub fn is_typed(&self, it: &Item) -> bool {
        if !self.anim.on || it.kind != ItemKind::Msg {
            return false;
        }
        match self.kind {
            Kind::Chat => self.composer && it.typed.unwrap_or(self.anim.compose && !self.me.is_empty() && it.who.eq_ignore_ascii_case(&self.me)),
            Kind::Comments => it.typed.unwrap_or(false),
            Kind::Forum => false,
        }
    }

    /// The keys of a typed item (its own speed, or the length it was given, else the thread's).
    pub fn plan_of(&self, it: &Item) -> (Vec<Key>, f64) {
        let n = it.text.chars().count().max(1) as f64;
        let cps = match it.typing.filter(|d| *d > 0.0) {
            Some(d) => n / d,
            None => it.cps.unwrap_or(self.anim.cps),
        };
        type_plan(&it.text, cps, self.anim.typos)
    }

    /// `<span>` per key: it shows at its moment (`base` + its time); a typo shows until backspace.
    pub(super) fn keys_html(keys: &[Key], base: f64) -> String {
        let mut out = String::new();
        for k in keys {
            let ch = if k.ch == " " { "&nbsp;".to_string() } else { esc(&k.ch) };
            match k.until {
                None => out.push_str(&format!("<span class=\"c\" style=\"animation-delay:{}\">{ch}</span>", secs(base + k.at))),
                Some(u) => out.push_str(&format!(
                    "<span class=\"c w\" style=\"animation-delay:{};animation-duration:{}\">{ch}</span>",
                    secs(base + k.at),
                    secs(u - k.at)
                )),
            }
        }
        out
    }

    /// The typing overlay of a box: visible from the first key until Send, the keys, a blinking caret.
    pub(super) fn tbox(&self, it: &Item, slot: &Slot) -> String {
        let (keys, _) = self.plan_of(it);
        let dur = (slot.appear - slot.type_start).max(0.05);
        format!(
            "<div class=\"tbox\" style=\"animation-delay:{};animation-duration:{}\"><div class=\"tin\">{}<i class=\"cr\" style=\"animation-delay:{};animation-iteration-count:{}\"></i></div></div>",
            secs(slot.type_start),
            secs(dur),
            Thread::keys_html(&keys, slot.type_start),
            secs(slot.type_start),
            dur.ceil().max(1.0)
        )
    }
}
