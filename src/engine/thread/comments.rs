//! The video comments page: count and sort bar, the comment box (it opens while a comment is typed,
//! with a Comment button that lights up and is pressed), the comments with replies, hearts and pins. The
//! geometry of the header and box is fixed in `em`, so `targets()` can say where the button is and
//! `mouse()` can aim the fake mouse at it.

use super::*;
use crate::model::{Animated, Ease, Keyframe, Mouse};

/// 12,345
pub fn comma(n: i64) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Header, comment box row and the Comment button's row, in em: the layout `targets` relies on.
const HEAD_EM: f64 = 2.6;
const ROW_EM: f64 = 3.8;
const ACT_EM: f64 = 3.0;
const BTN_W_EM: f64 = 7.0;
const BTN_H_EM: f64 = 2.1;

impl Thread {
    fn creator(&self) -> String {
        if self.post.who.is_empty() { "the creator".into() } else { self.post.who.clone() }
    }

    /// The Comment button of the first comment that is typed: (x, y, w, h) in CSS px of the page, and
    /// the second it is pressed.
    pub fn comment_button(&self) -> Option<([f64; 4], f64)> {
        if self.kind != Kind::Comments {
            return None;
        }
        let slots = self.schedule();
        let (_, s) = self.items.iter().zip(&slots).find(|(it, s)| self.is_typed(it) && it.depth == 0 && s.compose)?;
        let em = self.size as f64;
        let (w, h) = (BTN_W_EM * em, BTN_H_EM * em);
        let x = self.width as f64 - em * (1.0 + BTN_W_EM);
        let y = (HEAD_EM + ROW_EM + (ACT_EM - BTN_H_EM) / 2.0) * em;
        Some(([x, y, w, h], s.press))
    }

    /// A fake mouse that rests by the comment box, glides to the Comment button, and clicks it when the
    /// typing is done.
    pub fn mouse(&self) -> Option<Mouse> {
        let ([x, y, w, h], press) = self.comment_button()?;
        let slots = self.schedule();
        let start = self.items.iter().zip(&slots).find(|(it, s)| self.is_typed(it) && it.depth == 0 && s.compose).map_or(0.0, |(_, s)| s.type_start);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (sx, sy) = (self.width as f64 * 0.55, (HEAD_EM + ROW_EM * 0.5) * self.size as f64);
        let glide = (press - 0.45).max(start + 0.2);
        let track = |a: f64, b: f64| {
            let mut t = Animated::new(a);
            t.keys = vec![Keyframe { t: 0.0, v: a, ease: Ease::Linear }, Keyframe { t: start, v: a, ease: Ease::EaseInOut }, Keyframe { t: glide, v: b, ease: Ease::Linear }];
            t
        };
        let mut down = Animated::new(0.0);
        down.keys = [(press - 0.02, 0.0), (press, 1.0), (press + 0.14, 1.0), (press + 0.16, 0.0)].iter().map(|&(t, v)| Keyframe { t: t.max(0.0), v, ease: Ease::Hold }).collect();
        Some(Mouse { x: track(sx, cx), y: track(sy, cy), down })
    }

    /// The comment box of a typed reply: opens under its comment while typing, then closes as the reply shows.
    fn reply_box(&self, it: &Item, slot: &Slot) -> String {
        let open = secs((slot.type_start - 0.1).max(0.0));
        format!(
            "<div class=\"gx\" style=\"animation-delay:{open}\"><div><div class=\"gy\" style=\"animation-delay:{}\"><div><div class=\"rbx\" style=\"--dp:{}\">{}<div class=\"rcol\"><div class=\"cfield\"><span class=\"ph\">Add a reply…</span>{}</div><div class=\"rbtns\"><span class=\"cx\">Cancel</span><span class=\"cbtn sm\" style=\"animation-delay:{},{}\">Reply</span></div></div></div></div></div></div></div>",
            secs(slot.appear), it.depth, self.avatar(&it.who, " sm"), self.tbox(it, slot), secs(slot.type_start), secs(slot.press)
        )
    }

    fn comment_item(&self, i: usize, slot: &Slot, hidden: &[bool]) -> String {
        let it = &self.items[i];
        let kids = self.items[i + 1..].iter().take_while(|k| k.depth > it.depth).count();
        let creator = self.creator();
        let pin = if it.pinned { format!("<div class=\"pnb\">📌 Pinned by {}</div>", esc(&creator)) } else { String::new() };
        let edited = if it.edited { "<span class=\"mt\">(edited)</span>" } else { "" };
        let heart = if it.hearted {
            format!("<span class=\"hr\">{}<i>♥</i></span>", self.avatar(&creator, " sm"))
        } else {
            String::new()
        };
        let toggle = if it.depth == 0 && kids > 0 {
            format!("<div class=\"rt\">{} {kids} {}</div>", if it.collapsed { "▸" } else { "▾" }, if kids == 1 { "reply" } else { "replies" })
        } else {
            String::new()
        };
        let body = self.body_html(it, slot);
        let inner = format!(
            "<div class=\"bx\">{}<div class=\"cb2\">{pin}<div class=\"hh\">{}<span class=\"mt\">{}</span>{edited}</div>{body}<div class=\"ca2\"><span class=\"lk2\">👍 {}</span><span>👎</span><span class=\"rp2\">Reply</span>{heart}</div>{toggle}</div></div>",
            self.avatar(&it.who, if it.depth > 0 { " sm" } else { "" }),
            self.name_html(&it.who),
            esc(&it.age),
            self.score_html(it.score, slot.appear + 0.25)
        );
        let _ = hidden;
        let pre = if slot.compose && it.depth > 0 { self.reply_box(it, slot) } else { String::new() };
        format!("{pre}{}", self.wrap("c2", slot, it, &inner, &format!("--dp:{};", it.depth)))
    }

    pub(super) fn comments_body(&self) -> String {
        let (slots, hidden) = (self.schedule(), self.hidden());
        let n = if self.post.comments > 0 { self.post.comments as usize } else { self.items.len() };
        let sort = if self.post.sort.is_empty() { "Sort by" } else { &self.post.sort };
        let me = if self.me.is_empty() { self.cast.first().map(|m| m.name.clone()).unwrap_or_default() } else { self.me.clone() };
        let mut field = String::new();
        let mut acts = String::new();
        for (it, s) in self.items.iter().zip(&slots) {
            if !s.compose || it.depth > 0 {
                continue;
            }
            field.push_str(&self.tbox(it, s));
            acts.push_str(&format!(
                "<div class=\"gx\" style=\"animation-delay:{}\"><div><div class=\"gy\" style=\"animation-delay:{}\"><div><div class=\"cai\"><span class=\"cx\">Cancel</span><span class=\"cbtn\" style=\"animation-delay:{},{}\">Comment</span></div></div></div></div></div>",
                secs((s.type_start - 0.1).max(0.0)), secs(s.appear), secs(s.type_start), secs(s.press)
            ));
        }
        let mut out = format!(
            "<div class=\"chd\"><b>{} Comments</b><span class=\"so2\">☰ {}</span></div><div class=\"cmx\"><div class=\"crow\">{}<div class=\"cfield\"><span class=\"ph\">Add a comment…</span>{field}</div></div>{acts}</div><div class=\"main\"><div class=\"vp\"><div class=\"col\">",
            comma(n as i64), esc(sort), self.avatar(&me, "")
        );
        for i in 0..self.items.len() {
            if !hidden[i] {
                out.push_str(&self.comment_item(i, &slots[i], &hidden));
            }
        }
        out.push_str("</div></div></div>");
        out
    }
}
