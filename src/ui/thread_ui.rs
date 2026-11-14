//! ---- ws:thread-viewers ----
//! The Inspector's "Thread" section of a chat / forum thread layer: look, cast, a real list of messages
//! or comments (add, duplicate, move, delete, fold; reply / indent), the animation, and a script box that
//! turns plain lines into items. The thread is read out of the clip's page every frame (so an agent's
//! edit shows up as you watch), edited as a plain struct, and written back as a regenerated page: one
//! undo step per gesture, then the usual re-render.

use crate::engine::thread::{self, Embed, Item, ItemKind, Kind, Member, Reaction, Thread};
use crate::model::{Id, Project};
use crate::ui::list_rows::{Ctrl, Moved, RowSpec, Rows};
use crate::ui::Gesture;
use eframe::egui::{self, Ui};
use std::cell::RefCell;
use std::rc::Rc;

/// (asset, thread) cache: the page is parsed again only when its text changes.
fn thread_of(aid: Id, src: &str) -> Option<Rc<Thread>> {
    use std::hash::{Hash, Hasher};
    thread_local! {
        static CACHE: RefCell<Option<(Id, u64, Option<Rc<Thread>>)>> = const { RefCell::new(None) };
    }
    if !src.contains("name=\"se:thread\"") {
        return None;
    }
    let mut h = std::hash::DefaultHasher::new();
    src.hash(&mut h);
    let key = h.finish();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((a, k, t)) = c.as_ref() {
            if *a == aid && *k == key {
                return t.clone();
            }
        }
        let t = thread::from_src(src).map(Rc::new);
        *c = Some((aid, key, t.clone()));
        t
    })
}

/// Is this clip a thread layer?
pub fn is_thread(project: &Project, clip: Id) -> bool {
    project.clip(clip).and_then(|c| project.asset(c.asset)).is_some_and(|a| a.html.as_ref().is_some_and(|h| thread_of(a.id, &h.src).is_some()))
}

fn open_id(ui: &Ui, clip: Id, item: u32) -> egui::Id {
    ui.make_persistent_id(("thread_open", clip, item))
}

fn is_open(ui: &Ui, id: egui::Id) -> bool {
    ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false)
}

fn text1(ui: &mut Ui, g: &mut Gesture, s: &mut String, w: f32, hint: &str) {
    let r = ui.add(egui::TextEdit::singleline(s).desired_width(w).hint_text(hint));
    g.note_text(&r);
}

fn text_n(ui: &mut Ui, g: &mut Gesture, s: &mut String, rows: usize) {
    let r = ui.add(egui::TextEdit::multiline(s).desired_rows(rows).desired_width(f32::INFINITY));
    g.note_text(&r);
}

fn check(ui: &mut Ui, g: &mut Gesture, v: &mut bool, label: &str) {
    let r = ui.checkbox(v, label);
    g.note(&r);
}

fn drag<N: egui::emath::Numeric>(ui: &mut Ui, g: &mut Gesture, v: &mut N, lo: N, hi: N, suffix: &str) {
    let r = ui.add(egui::DragValue::new(v).range(lo..=hi).suffix(suffix));
    g.note(&r);
}

fn colour(ui: &mut Ui, g: &mut Gesture, hex: &mut String) {
    let mut c = crate::ui::html_ui::hex_rgba(hex);
    let r = ui.color_edit_button_srgba_unmultiplied(&mut c);
    g.note(&r);
    if r.changed() {
        *hex = crate::ui::html_ui::rgba_hex(c);
    }
}

/// A combo over (value, label) options.
fn pick(ui: &mut Ui, g: &mut Gesture, salt: impl std::hash::Hash, cur: &mut String, opts: &[(String, String)]) {
    let shown = opts.iter().find(|(v, _)| v == cur).map_or(cur.clone(), |(_, l)| l.clone());
    let mut chosen = None;
    egui::ComboBox::from_id_salt(salt).selected_text(shown).width(110.0).show_ui(ui, |ui| {
        for (v, l) in opts {
            if ui.selectable_label(v == cur, l).clicked() {
                chosen = Some(v.clone());
            }
        }
    });
    if let Some(v) = chosen {
        *cur = v;
        g.click();
    }
}

fn opts(list: &[&str]) -> Vec<(String, String)> {
    list.iter().map(|s| (s.to_string(), s.to_string())).collect()
}

/// A project image (by file name) or a file: shown as its name, picked from the project's pictures.
fn image(ui: &mut Ui, g: &mut Gesture, salt: impl std::hash::Hash, cur: &mut String, images: &[String]) {
    let shown = if cur.is_empty() { "(none)".to_string() } else { std::path::Path::new(cur.as_str()).file_name().map_or(cur.clone(), |n| n.to_string_lossy().into_owned()) };
    let mut chosen: Option<String> = None;
    egui::ComboBox::from_id_salt(salt).selected_text(shown).width(130.0).show_ui(ui, |ui| {
        if ui.selectable_label(cur.is_empty(), "(none)").clicked() {
            chosen = Some(String::new());
        }
        for name in images {
            if ui.selectable_label(cur == name, name).clicked() {
                chosen = Some(name.clone());
            }
        }
        if ui.selectable_label(false, "Browse…").clicked() {
            let p = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"]).pick_file();
            chosen = p.map(|p| p.to_string_lossy().into_owned());
        }
    });
    if let Some(v) = chosen {
        *cur = v;
        g.click();
    }
}

fn opt_secs(ui: &mut Ui, g: &mut Gesture, label: &str, v: &mut Option<f64>, default: f64) {
    let mut on = v.is_some();
    let r = ui.checkbox(&mut on, label);
    g.note(&r);
    if r.changed() {
        *v = on.then_some(default);
    }
    if let Some(x) = v.as_mut() {
        drag(ui, g, x, 0.0, 60.0, " s");
    }
}

/// The people of the cast, as combo options.
fn cast_opts(t: &Thread) -> Vec<(String, String)> {
    t.cast.iter().map(|m| (m.name.clone(), m.name.clone())).collect()
}

fn who_default(t: &Thread) -> String {
    let last = t.items.iter().rev().find(|i| i.kind == ItemKind::Msg).map(|i| i.who.clone());
    let at = last.and_then(|l| t.cast.iter().position(|m| m.name.eq_ignore_ascii_case(&l)));
    t.cast.get(at.map_or(0, |a| (a + 1) % t.cast.len().max(1))).map_or_else(|| "Person".to_string(), |m| m.name.clone())
}

/// The cast as a table: one aligned column per property.
fn cast_section(ui: &mut Ui, g: &mut Gesture, t: &mut Thread, clip: Id, images: &[String]) {
    let mut remove: Option<usize> = None;
    let mut rename: Option<(String, String)> = None;
    egui::Grid::new(("thread_cast_grid", clip)).num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
        for h in ["Picture", "Name", "Colour", "Badge", ""] {
            ui.weak(h);
        }
        ui.end_row();
        for i in 0..t.cast.len() {
            let mut av = t.cast[i].avatar.clone();
            image(ui, g, ("thread_av", clip, i), &mut av, images);
            t.cast[i].avatar = av;
            // a name is renamed when you leave the box: every message follows
            let nid = ui.make_persistent_id(("thread_name", clip, i));
            let mut buf: String = ui.data(|d| d.get_temp(nid)).unwrap_or_else(|| t.cast[i].name.clone());
            if !ui.memory(|m| m.has_focus(nid)) {
                buf = t.cast[i].name.clone();
            }
            let r = ui.add(egui::TextEdit::singleline(&mut buf).id(nid).desired_width(110.0));
            ui.data_mut(|d| d.insert_temp(nid, buf.clone()));
            if r.lost_focus() && buf.trim() != t.cast[i].name {
                rename = Some((t.cast[i].name.clone(), buf.trim().to_string()));
            }
            let mut c = t.cast[i].color.clone();
            colour(ui, g, &mut c);
            t.cast[i].color = c;
            let mut b = t.cast[i].badge.clone();
            text1(ui, g, &mut b, 70.0, "BOT");
            t.cast[i].badge = b;
            if ui.small_button("✖").on_hover_text("Remove from the cast (their messages keep the name)").clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some((a, b)) = rename {
        if t.rename_member(&a, &b) {
            g.click();
        }
    }
    if let Some(i) = remove {
        t.cast.remove(i);
        g.click();
    }
    if ui.button("+ Add person").clicked() {
        let n = (1..).map(|k| format!("Person {k}")).find(|n| t.member(n).is_none()).unwrap_or_default();
        t.upsert(Member { name: n, ..Default::default() });
        g.click();
    }
}

fn item_title(t: &Thread, it: &Item) -> String {
    let body: String = it.text.chars().filter(|c| !"\n*`|".contains(*c)).take(60).collect();
    match it.kind {
        ItemKind::Msg => format!("{}: {}", it.who, body),
        ItemKind::System => format!("→ {} {}", it.who, body),
        ItemKind::Divider => format!("— {} —", if it.text.is_empty() { Thread::date_label(&it.date) } else { it.text.clone() }),
        ItemKind::Unread => "— NEW —".into(),
    }
    .replace("\u{0}", &t.title)
}

/// An open item: two aligned columns (labels, then fields).
fn item_body(ui: &mut Ui, g: &mut Gesture, t: &mut Thread, i: usize, clip: Id, images: &[String]) {
    let kind = t.kind;
    let cast = cast_opts(t);
    let replies: Vec<(String, String)> = std::iter::once((String::new(), "(none)".to_string()))
        .chain(t.items[..i].iter().filter(|p| p.kind == ItemKind::Msg).map(|p| (p.id.to_string(), item_title(t, p))))
        .collect();
    let it = &mut t.items[i];
    let id = it.id;
    let msg = it.kind == ItemKind::Msg;
    egui::Grid::new(("thread_item", clip, id)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        if matches!(it.kind, ItemKind::Msg | ItemKind::System) {
            ui.label("From");
            pick(ui, g, ("thread_who", clip, id), &mut it.who, &cast);
            ui.end_row();
        }
        if kind == Kind::Chat && matches!(it.kind, ItemKind::Msg | ItemKind::System) {
            ui.label("Sent");
            ui.horizontal(|ui| {
                text1(ui, g, &mut it.date, 86.0, "2026-11-13");
                text1(ui, g, &mut it.time, 70.0, "4:32 PM");
            });
            ui.end_row();
        }
        if kind != Kind::Chat {
            ui.label(if kind == Kind::Comments { "Age / likes" } else { "Age / votes" });
            ui.horizontal(|ui| {
                text1(ui, g, &mut it.age, 80.0, "3 hr. ago");
                drag(ui, g, &mut it.score, -999_999_999, 999_999_999, "");
            });
            ui.end_row();
        }
        if it.kind != ItemKind::Unread {
            ui.label(match it.kind {
                ItemKind::Msg => "Text",
                ItemKind::System => "After the name",
                _ => "Label",
            });
            text_n(ui, g, &mut it.text, if msg { 3 } else { 1 });
            ui.end_row();
        }
        if !msg {
            return;
        }
        ui.label("Flags");
        ui.horizontal_wrapped(|ui| {
            if kind == Kind::Chat {
                check(ui, g, &mut it.edited, "edited");
                check(ui, g, &mut it.pinned, "pinned");
            } else if kind == Kind::Comments {
                check(ui, g, &mut it.pinned, "pinned");
                check(ui, g, &mut it.hearted, "hearted");
                check(ui, g, &mut it.edited, "edited");
                check(ui, g, &mut it.collapsed, "replies folded");
            } else {
                check(ui, g, &mut it.op, "OP badge");
                check(ui, g, &mut it.collapsed, "folded");
                ui.label("more replies");
                drag(ui, g, &mut it.more, 0, 9999, "");
            }
        });
        ui.end_row();
        if kind == Kind::Chat {
            let mut r = it.reply.map(|r| r.to_string()).unwrap_or_default();
            ui.label("Reply to");
            pick(ui, g, ("thread_reply", clip, id), &mut r, &replies);
            it.reply = r.parse().ok();
            ui.end_row();
        } else if kind == Kind::Forum {
            let mut aw = it.awards.join(" ");
            ui.label("Awards");
            text1(ui, g, &mut aw, 120.0, ":gold: :trophy:");
            it.awards = aw.split_whitespace().map(String::from).collect();
            ui.end_row();
        }
        // pictures
        let mut del_img = None;
        for (k, im) in it.images.iter_mut().enumerate() {
            ui.label("Image");
            ui.horizontal(|ui| {
                image(ui, g, ("thread_img", clip, id, k), im, images);
                if ui.small_button("✖").clicked() {
                    del_img = Some(k);
                }
            });
            ui.end_row();
        }
        if let Some(k) = del_img {
            it.images.remove(k);
            g.click();
        }
        ui.label("");
        if ui.small_button("+ Image").clicked() {
            it.images.push(images.first().cloned().unwrap_or_default());
            g.click();
        }
        ui.end_row();
        if kind == Kind::Chat {
            let mut on = it.embed.is_some();
            ui.label("Embed");
            let r = ui.checkbox(&mut on, "title, text and a colour bar");
            g.note(&r);
            if r.changed() {
                it.embed = on.then(|| Embed { title: "Title".into(), desc: "Description".into(), color: "#4f6bed".into(), thumb: String::new() });
            }
            ui.end_row();
            if let Some(e) = it.embed.as_mut() {
                ui.label("  Title");
                ui.horizontal(|ui| {
                    text1(ui, g, &mut e.title, 120.0, "title");
                    colour(ui, g, &mut e.color);
                });
                ui.end_row();
                ui.label("  Picture");
                image(ui, g, ("thread_embed", clip, id), &mut e.thumb, images);
                ui.end_row();
                ui.label("  Text");
                text_n(ui, g, &mut e.desc, 2);
                ui.end_row();
            }
            let mut del = None;
            for (k, r) in it.reactions.iter_mut().enumerate() {
                ui.label("Reaction");
                ui.horizontal(|ui| {
                    text1(ui, g, &mut r.emoji, 70.0, ":fire:");
                    drag(ui, g, &mut r.count, 0, 999_999, "");
                    check(ui, g, &mut r.me, "me");
                    if ui.small_button("✖").clicked() {
                        del = Some(k);
                    }
                });
                ui.end_row();
            }
            if let Some(k) = del {
                it.reactions.remove(k);
                g.click();
            }
            ui.label("");
            if ui.small_button("+ Reaction").clicked() {
                it.reactions.push(Reaction { emoji: ":fire:".into(), count: 1, me: false });
                g.click();
            }
            ui.end_row();
        }
        ui.label("Timing");
        ui.horizontal_wrapped(|ui| {
            opt_secs(ui, g, "delay", &mut it.delay, 1.4);
            if kind != Kind::Forum {
                // typed out in the box? auto = the thread's rule (chat: the person at the screen)
                let mut t3 = match it.typed {
                    None => "auto",
                    Some(true) => "typed",
                    Some(false) => "not typed",
                }
                .to_string();
                ui.label("typing");
                pick(ui, g, ("thread_typed", clip, id), &mut t3, &opts(&["auto", "typed", "not typed"]));
                it.typed = match t3.as_str() {
                    "typed" => Some(true),
                    "not typed" => Some(false),
                    _ => None,
                };
                opt_secs(ui, g, "length", &mut it.typing, 1.0);
                let mut c = it.cps.is_some();
                let r = ui.checkbox(&mut c, "speed");
                g.note(&r);
                if r.changed() {
                    it.cps = c.then_some(14.0);
                }
                if let Some(v) = it.cps.as_mut() {
                    drag(ui, g, v, 1.0, 80.0, " keys/s");
                }
                opt_secs(ui, g, "pause", &mut it.pause, 0.5);
            }
            opt_secs(ui, g, "pulse", &mut it.pulse, 0.5);
            check(ui, g, &mut it.there, "already there");
        });
        ui.end_row();
    });
}

/// Row buttons in the same order and place on every row.
const CTRLS: &[Ctrl] = &[
    Ctrl { glyph: "↩", hint: "Reply: add a message / reply to this one" },
    Ctrl { glyph: "▲", hint: "Move up (Alt+Up)" },
    Ctrl { glyph: "▼", hint: "Move down (Alt+Down)" },
    Ctrl { glyph: "⧉", hint: "Duplicate" },
    Ctrl { glyph: "✖", hint: "Delete" },
];

/// Buttons of equal width side by side: a tidy footer row.
fn button_row(ui: &mut Ui, labels: &[&str]) -> Option<usize> {
    let n = labels.len().max(1) as f32;
    let w = ((ui.available_width() - ui.spacing().item_spacing.x * (n - 1.0)) / n).max(40.0);
    let mut hit = None;
    ui.horizontal(|ui| {
        for (k, l) in labels.iter().enumerate() {
            if ui.add_sized([w, 22.0], egui::Button::new(*l)).clicked() {
                hit = Some(k);
            }
        }
    });
    hit
}

/// The messages / comments as a table (grip, arrow, summary, time or votes, buttons), an aligned footer
/// of add buttons; drag a grip or use Alt+arrows to reorder (replies move with their comment).
fn items_section(ui: &mut Ui, g: &mut Gesture, t: &mut Thread, clip: Id, images: &[String]) {
    let nested = t.kind != Kind::Chat;
    let mut rows = Rows::new(ui, ("thread_rows", clip), CTRLS, &[64.0], nested);
    let (mut up, mut down, mut dup, mut del, mut reply) = (None, None, None, None, None);
    for i in 0..t.items.len() {
        let id = t.items[i].id;
        let oid = open_id(ui, clip, id);
        let open = is_open(ui, oid);
        let title = item_title(t, &t.items[i]);
        let value = if t.kind == Kind::Chat { t.items[i].time.clone() } else if t.items[i].kind == ItemKind::Msg { crate::engine::thread::count_text(t.items[i].score) } else { String::new() };
        let dim = nested && (0..i).rev().find(|&k| t.items[k].depth < t.items[i].depth).is_some_and(|k| t.items[k].collapsed);
        let r = rows.row(ui, i, &RowSpec { key: id as u64, label: &title, depth: t.items[i].depth, open: Some(open), dim, values: &[&value] });
        if r.toggled {
            ui.data_mut(|d| d.insert_temp(oid, !open));
        }
        match r.ctrl {
            Some(0) => reply = Some(id),
            Some(1) => up = Some(id),
            Some(2) => down = Some(id),
            Some(3) => dup = Some(id),
            Some(4) => del = Some(id),
            _ => {}
        }
        if open {
            ui.indent(("thread_ind", clip, id), |ui| item_body(ui, g, t, i, clip, images));
            ui.add_space(4.0);
        }
    }
    let moved = rows.finish(ui);
    ui.add_space(4.0);
    let mut add: Option<Item> = None;
    let who = who_default(t);
    match t.kind {
        Kind::Chat => {
            match button_row(ui, &["+ Add message", "+ System line"]) {
                Some(0) => add = Some(Item::msg(&who, "New message")),
                Some(_) => add = Some(Item { kind: ItemKind::System, who: who.clone(), text: "joined the server.".into(), ..Default::default() }),
                None => {}
            }
            match button_row(ui, &["+ Date divider", "+ New line"]) {
                Some(0) => add = Some(Item { kind: ItemKind::Divider, text: "Today".into(), ..Default::default() }),
                Some(_) => add = Some(Item { kind: ItemKind::Unread, text: "NEW".into(), ..Default::default() }),
                None => {}
            }
        }
        Kind::Forum | Kind::Comments => {
            match button_row(ui, &["+ Add comment", "+ Add reply"]) {
                Some(0) => add = Some(Item { who: who.clone(), text: "New comment".into(), age: "just now".into(), ..Default::default() }),
                Some(_) => {
                    let d = t.items.last().map_or(0, |l| l.depth + 1);
                    add = Some(Item { who: who.clone(), text: "New reply".into(), age: "just now".into(), depth: d, ..Default::default() });
                }
                None => {}
            }
            if t.kind == Kind::Comments && button_row(ui, &["+ Type a comment (typed into the box, then posted)"]).is_some() {
                let me = if t.me.is_empty() { who.clone() } else { t.me.clone() };
                add = Some(Item { who: me, text: "New comment".into(), age: "Just now".into(), typed: Some(true), ..Default::default() });
            }
        }
    }
    if let Some(it) = add {
        let top = t.kind == Kind::Comments && it.typed == Some(true);
        let id = t.add(it, None);
        if top {
            t.move_to(id, 0);
        }
        ui.data_mut(|d| d.insert_temp(open_id_of(clip, id), true));
        g.click();
    }
    // what the rows asked for: one change, one undo step
    let mut changed = true;
    let id_at = |t: &Thread, i: usize| t.items.get(i).map(|x| x.id);
    if let Some(id) = up {
        t.step_tree(id, -1);
    } else if let Some(id) = down {
        t.step_tree(id, 1);
    } else if let Some(id) = dup {
        if let Some(n) = t.duplicate(id) {
            ui.data_mut(|d| d.insert_temp(open_id_of(clip, n), true));
        }
    } else if let Some(id) = del {
        t.remove(id);
    } else if let Some(id) = reply {
        if let Some(at) = t.index_of(id) {
            let src = t.items[at].clone();
            let who = who_default(t);
            let (depth, pos) = if t.kind != Kind::Chat {
                // after the last of its replies
                let end = (at + 1..t.items.len()).find(|&k| t.items[k].depth <= src.depth).unwrap_or(t.items.len());
                (src.depth + 1, end)
            } else {
                (0, at + 1)
            };
            let it = Item { who, text: "New reply".into(), depth, reply: (t.kind == Kind::Chat).then_some(id), age: "just now".into(), ..Default::default() };
            let n = t.add(it, Some(pos));
            ui.data_mut(|d| d.insert_temp(open_id_of(clip, n), true));
        }
    } else if let Some(m) = moved {
        match m {
            Moved::Slot { from, slot, depth } => {
                if let Some(id) = id_at(t, from) {
                    t.move_tree(id, slot, depth);
                }
            }
            Moved::Step { from, delta } => {
                if let Some(id) = id_at(t, from) {
                    t.step_tree(id, delta);
                }
            }
            Moved::Indent { from, delta } => {
                if let Some(id) = id_at(t, from) {
                    t.indent(id, delta);
                }
            }
        }
    } else {
        changed = false;
    }
    if changed {
        g.click();
    }
}

fn open_id_of(clip: Id, item: u32) -> egui::Id {
    egui::Id::new(("thread_open", clip, item))
}

/// The pictures of the project, by file name (what an avatar names).
fn project_images(project: &Project) -> Vec<String> {
    let mut v: Vec<String> = project
        .assets
        .iter()
        .filter(|a| a.html.is_none() && a.kind == crate::model::ClipKind::Image)
        .filter_map(|a| std::path::Path::new(&a.path).file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Grow a clip to `secs` if nothing is in the way.
fn fit_clip(project: &mut Project, clip: Id, secs: f64) {
    let Some(ti) = project.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == clip)) else { return };
    let start = project.clip(clip).map_or(0.0, |c| c.start);
    let next = project.tracks[ti].clips.iter().filter(|c| c.id != clip && c.start > start + 1e-6).map(|c| c.start).fold(f64::INFINITY, f64::min);
    if let Some(c) = project.clip_mut(clip) {
        c.duration = secs.min(next - start).max(0.1);
    }
}

/// The section. True when the project changed.
pub fn section(ui: &mut Ui, project: &mut Project, clip: Id, fonts: &[String], undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(aid) = project.clip(clip).map(|c| c.asset) else { return false };
    let Some(h) = project.asset(aid).and_then(|a| a.html.clone()) else { return false };
    let Some(orig) = thread_of(aid, &h.src) else { return false };
    let mut t: Thread = (*orig).clone();
    let images = project_images(project);
    let mut g = Gesture::default();
    let mut detach = false;
    let mut fit = false;
    let mut aim = false;
    let mut replace_with: Option<Thread> = None;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(t.kind.label()).strong());
        let names: Vec<(String, String)> = thread::PRESETS.iter().filter(|p| p.1 == t.kind).map(|p| (p.0.to_string(), p.0.to_string())).collect();
        let mut cur = t.preset_name().unwrap_or("Custom").to_string();
        let before = cur.clone();
        pick(ui, &mut g, ("thread_preset", clip), &mut cur, &names);
        if cur != before {
            t.apply_preset(&cur);
        }
    });

    egui::CollapsingHeader::new("Look").id_salt(("thread_look", clip)).default_open(false).show(ui, |ui| {
        egui::Grid::new(("thread_look_grid", clip)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            if t.kind == Kind::Chat {
                ui.label("Layout");
                let o: Vec<(String, String)> = [("server", "Server channel"), ("dm", "DM"), ("group", "Group DM")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
                let mut m = t.mode.clone();
                pick(ui, &mut g, ("thread_mode", clip), &mut m, &o);
                t.mode = m;
                ui.end_row();
            }
            ui.label("Theme");
            ui.horizontal(|ui| {
                for th in ["dark", "light"] {
                    if ui.selectable_label(t.theme == th, th).clicked() {
                        t.theme = th.into();
                        g.click();
                    }
                }
            });
            ui.end_row();
            ui.label("Accent");
            colour(ui, &mut g, &mut t.accent);
            ui.end_row();
            ui.label("Font");
            let mut f = t.font.clone();
            let mut chosen = None;
            egui::ComboBox::from_id_salt(("thread_font", clip)).selected_text(f.clone()).height(320.0).show_ui(ui, |ui| {
                for fam in fonts {
                    if ui.selectable_label(*fam == f, fam).clicked() {
                        chosen = Some(fam.clone());
                    }
                }
            });
            if let Some(c) = chosen {
                f = c;
                g.click();
            }
            t.font = f;
            ui.end_row();
            ui.label("Corners");
            let r = ui.add(egui::Slider::new(&mut t.radius, 0..=48));
            g.note(&r);
            ui.end_row();
            ui.label("Text size");
            let r = ui.add(egui::Slider::new(&mut t.size, 14..=44));
            g.note(&r);
            ui.end_row();
            ui.label("Spacing");
            let mut d = t.density.clone();
            pick(ui, &mut g, ("thread_density", clip), &mut d, &opts(&["compact", "normal", "roomy"]));
            t.density = d;
            ui.end_row();
            ui.label("Size");
            ui.horizontal(|ui| {
                drag(ui, &mut g, &mut t.width, 320, 3840, "");
                ui.label("×");
                drag(ui, &mut g, &mut t.height, 200, 2160, "");
            });
            ui.end_row();
            if t.kind == Kind::Chat {
                ui.label("Times");
                let o: Vec<(String, String)> = [("today", "Today at 4:32 PM"), ("full", "11/13/2026 4:32 PM"), ("time", "4:32 PM"), ("24h", "16:32"), ("relative", "5 minutes ago")]
                    .iter()
                    .map(|(a, b)| (a.to_string(), b.to_string()))
                    .collect();
                let mut f = t.time_format.clone();
                pick(ui, &mut g, ("thread_times", clip), &mut f, &o);
                t.time_format = f;
                ui.end_row();
                ui.label("Now");
                text1(ui, &mut g, &mut t.now, 140.0, "2026-11-13 4:40 PM");
                ui.end_row();
            }
        });
        match t.kind {
            Kind::Chat => {
                let mut mo = t.messages_only();
                let r = ui.checkbox(&mut mo, "Messages only (just the people messaging)");
                g.note(&r);
                if r.changed() {
                    if mo {
                        t.set_messages_only();
                    } else {
                        (t.header, t.composer, t.indicator, t.dividers) = (true, true, true, true);
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    check(ui, &mut g, &mut t.rail, "server rail");
                    check(ui, &mut g, &mut t.channels, if t.mode == "server" { "channel list" } else { "conversation list" });
                    check(ui, &mut g, &mut t.sidebar, "member list");
                    check(ui, &mut g, &mut t.topbar, "top bar");
                    check(ui, &mut g, &mut t.header, "header");
                    check(ui, &mut g, &mut t.composer, "message box");
                    check(ui, &mut g, &mut t.indicator, "typing line");
                    check(ui, &mut g, &mut t.dividers, "date dividers");
                    check(ui, &mut g, &mut t.unread_line, "NEW line");
                    check(ui, &mut g, &mut t.scrollbar, "scrollbar");
                    check(ui, &mut g, &mut t.group, "group messages");
                });
            }
            Kind::Comments => {
                ui.horizontal_wrapped(|ui| {
                    check(ui, &mut g, &mut t.header, "count bar");
                    check(ui, &mut g, &mut t.composer, "comment box");
                });
            }
            Kind::Forum => {}
        }
        check(ui, &mut g, &mut t.transparent, "transparent background (over video)");
    });

    egui::CollapsingHeader::new(match t.kind { Kind::Chat => "Channel", Kind::Forum => "Community and post", Kind::Comments => "Video and creator" }).id_salt(("thread_head", clip)).default_open(true).show(ui, |ui| {
        egui::Grid::new(("thread_head_grid", clip)).num_columns(2).show(ui, |ui| {
            ui.label(match t.kind { Kind::Chat => "Channel", Kind::Forum => "Community", Kind::Comments => "Video title" });
            text1(ui, &mut g, &mut t.title, 160.0, "");
            ui.end_row();
            if t.kind != Kind::Forum {
                ui.label("You are");
                let cast = cast_opts_of(&t.cast);
                pick(ui, &mut g, ("thread_me", clip), &mut t.me, &cast);
                ui.end_row();
            }
            if t.kind == Kind::Comments {
                ui.label("Creator");
                let cast = cast_opts_of(&t.cast);
                pick(ui, &mut g, ("thread_creator", clip), &mut t.post.who, &cast);
                ui.end_row();
                ui.label("Comments");
                ui.horizontal(|ui| {
                    drag(ui, &mut g, &mut t.post.comments, 0, 99_999_999, "");
                    ui.weak("0 = count them");
                });
                ui.end_row();
                ui.label("Sort label");
                text1(ui, &mut g, &mut t.post.sort, 90.0, "Sort by");
                ui.end_row();
            }
            if t.kind == Kind::Chat {
                ui.label("Topic");
                text1(ui, &mut g, &mut t.topic, 160.0, "");
                ui.end_row();
                ui.label("Server");
                text1(ui, &mut g, &mut t.server, 160.0, "");
                ui.end_row();
            }
            if t.kind != Kind::Comments {
                ui.label("Icon");
                image(ui, &mut g, ("thread_icon", clip), &mut t.icon, &images);
                ui.end_row();
            }
            if t.kind == Kind::Forum {
                let p = &mut t.post;
                ui.label("Title");
                text1(ui, &mut g, &mut p.title, 200.0, "");
                ui.end_row();
                ui.label("Author");
                let cast = cast_opts_of(&t.cast);
                pick(ui, &mut g, ("thread_author", clip), &mut t.post.who, &cast);
                ui.end_row();
                let p = &mut t.post;
                ui.label("Flair");
                ui.horizontal(|ui| {
                    text1(ui, &mut g, &mut p.flair, 90.0, "flair");
                    colour(ui, &mut g, &mut p.flair_color);
                });
                ui.end_row();
                ui.label("Votes / age");
                ui.horizontal(|ui| {
                    drag(ui, &mut g, &mut p.score, -999_999_999, 999_999_999, "");
                    text1(ui, &mut g, &mut p.age, 80.0, "5 hr. ago");
                });
                ui.end_row();
                ui.label("Comments");
                ui.horizontal(|ui| {
                    drag(ui, &mut g, &mut p.comments, 0, 9_999_999, "");
                    ui.weak("0 = count them");
                });
                ui.end_row();
                ui.label("Sort");
                text1(ui, &mut g, &mut p.sort, 90.0, "Best");
                ui.end_row();
                ui.label("Picture");
                image(ui, &mut g, ("thread_pimg", clip), &mut p.image, &images);
                ui.end_row();
                ui.label("Thumbnail");
                image(ui, &mut g, ("thread_pthumb", clip), &mut p.thumb, &images);
                ui.end_row();
                ui.label("Link");
                text1(ui, &mut g, &mut p.link, 160.0, "example.com");
                ui.end_row();
                ui.label("Awards");
                let mut aw = p.awards.join(" ");
                text1(ui, &mut g, &mut aw, 140.0, ":gold: :trophy:");
                p.awards = aw.split_whitespace().map(String::from).collect();
                ui.end_row();
            }
        });
        if t.kind == Kind::Forum {
            ui.weak("Post text");
            text_n(ui, &mut g, &mut t.post.body, 4);
        }
    });

    egui::CollapsingHeader::new(format!("Cast ({})", t.cast.len())).id_salt(("thread_cast", clip)).default_open(false).show(ui, |ui| {
        cast_section(ui, &mut g, &mut t, clip, &images);
    });

    egui::CollapsingHeader::new(format!("{} ({})", if t.kind == Kind::Chat { "Messages" } else { "Comments" }, t.items.len()))
        .id_salt(("thread_items", clip))
        .default_open(true)
        .show(ui, |ui| items_section(ui, &mut g, &mut t, clip, &images));

    egui::CollapsingHeader::new("Animation").id_salt(("thread_anim", clip)).default_open(false).show(ui, |ui| {
        let a = &mut t.anim;
        check(ui, &mut g, &mut a.on, "Items arrive one after another");
        egui::Grid::new(("thread_anim_grid", clip)).num_columns(2).show(ui, |ui| {
            ui.label("First item at");
            drag(ui, &mut g, &mut a.start, 0.0, 60.0, " s");
            ui.end_row();
            ui.label("Gap between");
            drag(ui, &mut g, &mut a.gap, 0.0, 30.0, " s");
            ui.end_row();
            if t.kind == Kind::Chat {
                ui.label("Typing before");
                drag(ui, &mut g, &mut a.typing, 0.0, 10.0, " s");
                ui.end_row();
            }
            ui.label("Appear as");
            let mut e = a.effect.clone();
            pick(ui, &mut g, ("thread_fx", clip), &mut e, &opts(&["pop", "slide", "fade", "cut"]));
            a.effect = e;
            ui.end_row();
            ui.label("Already there");
            drag(ui, &mut g, &mut a.shown, 0, 999, " items");
            ui.end_row();
            ui.label("Hold at end");
            drag(ui, &mut g, &mut a.hold, 0.0, 60.0, " s");
            ui.end_row();
        });
        check(ui, &mut g, &mut a.autoscroll, "Keep the newest in view");
        if t.kind != Kind::Chat {
            check(ui, &mut g, &mut a.tick, if t.kind == Kind::Forum { "Vote counts count up" } else { "Like counts count up" });
        }
        if t.kind != Kind::Forum {
            ui.separator();
            check(ui, &mut g, &mut a.compose, if t.kind == Kind::Chat { "Type my messages in the message box, then send" } else { "Type my comment in the box, then post" });
            ui.horizontal_wrapped(|ui| {
                ui.label("speed");
                drag(ui, &mut g, &mut a.cps, 1.0, 80.0, " keys/s");
                ui.label("pause before send");
                drag(ui, &mut g, &mut a.pause, 0.0, 10.0, " s");
                check(ui, &mut g, &mut a.typos, "typos");
            });
        }
        if t.kind == Kind::Comments && t.comment_button().is_some() && ui.button("Aim the mouse at the Comment button").on_hover_text("Keyframes the fake mouse (Mouse section) to glide to the Comment button and click it when the typing ends").clicked() {
            aim = true;
        }
        let need = t.duration();
        let have = project.clip(clip).map_or(need, |c| c.duration);
        ui.horizontal(|ui| {
            ui.weak(format!("Plays for {need:.1} s; the clip is {have:.1} s"));
            if (need - have).abs() > 0.05 && ui.small_button("Fit clip").clicked() {
                fit = true;
            }
        });
    });

    egui::CollapsingHeader::new("Script and data").id_salt(("thread_data", clip)).default_open(false).show(ui, |ui| {
        let sid = ui.make_persistent_id(("thread_script", clip));
        let mut script: String = ui.data(|d| d.get_temp(sid)).unwrap_or_default();
        ui.weak(match t.kind { Kind::Chat => "name: message    name (4:32 PM): message    --- divider    * dave joined", Kind::Forum => "# title   name (3h, 4.2k): comment   > reply   >> deeper", Kind::Comments => "name (3 days ago, 12K): comment    > reply to the one above" });
        let r = ui.add(egui::TextEdit::multiline(&mut script).desired_rows(5).desired_width(f32::INFINITY).hint_text("Paste lines here"));
        let _ = r;
        ui.horizontal_wrapped(|ui| {
            let some = !script.trim().is_empty();
            if ui.add_enabled(some, egui::Button::new("Add lines")).clicked() {
                t.from_script(&script, false);
                script.clear();
                g.click();
            }
            if ui.add_enabled(some, egui::Button::new("Replace all")).on_hover_text("Replace every item with these lines").clicked() {
                t.from_script(&script, true);
                script.clear();
                g.click();
            }
            if ui.button("Show as lines").on_hover_text("Fill the box with the current items to edit them as text").clicked() {
                script = t.to_script();
            }
        });
        ui.data_mut(|d| d.insert_temp(sid, script));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Copy JSON").clicked() {
                ui.ctx().copy_text(t.json_pretty());
            }
            if ui.button("Export JSON…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Thread", &["json"]).set_file_name("thread.json").save_file() {
                    let _ = std::fs::write(p, t.json_pretty());
                }
            }
            if ui.button("Import JSON…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Thread", &["json"]).pick_file() {
                    if let Ok(mut n) = std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Thread>(&s).map_err(|e| e.to_string())) {
                        n.normalize();
                        replace_with = Some(n);
                    }
                }
            }
            if ui.button("Make plain HTML").on_hover_text("Forget the thread data: the page becomes ordinary HTML you can hand-edit").clicked() {
                detach = true;
            }
        });
    });

    if let Some(n) = replace_with {
        t = n;
        g.click();
    }
    let mut changed = false;
    if detach {
        undo(project);
        if let Some(x) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
            x.src = thread::detach(&x.src);
        }
        changed = true;
    } else if g.changed && t != *orig {
        if g.start {
            undo(project);
        }
        t.normalize();
        if let Some(x) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
            thread::store(x, &t);
        }
        changed = true;
    }
    if aim {
        undo(project);
        if let Some(x) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
            x.mouse = t.mouse();
        }
        changed = true;
    }
    if fit {
        undo(project);
        fit_clip(project, clip, t.duration());
        changed = true;
    }
    if changed {
        crate::ui::html_ui::mark_edited();
    }
    changed
}

fn cast_opts_of(cast: &[Member]) -> Vec<(String, String)> {
    cast.iter().map(|m| (m.name.clone(), m.name.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, ClipKind, HtmlSource};

    fn project_with(t: &Thread) -> (Project, Id, Id) {
        let mut p = Project::new();
        let aid = p.add_asset(Asset {
            kind: ClipKind::Image,
            html: Some(HtmlSource { name: "x".into(), src: t.html(), duration: t.duration(), ..Default::default() }),
            ..crate::engine::import::placeholder("C:/x/pending.png")
        });
        let cid = p.insert_asset_clips(aid, 0.0, None)[0];
        (p, aid, cid)
    }

    fn draw(ctx: &egui::Context, p: &mut Project, cid: Id, undos: &mut u32) -> bool {
        let mut changed = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed |= section(ui, p, cid, &["Segoe UI".to_string()], &mut |_| *undos += 1);
            });
        });
        changed
    }

    /// Drawing the section is not an edit and asks for no frames (chat and forum).
    #[test]
    fn assert_no_idle_repaint_thread_section() {
        for kind in [Kind::Chat, Kind::Forum, Kind::Comments] {
            let t = Thread::starter(kind, "");
            let (mut p, _, cid) = project_with(&t);
            assert!(is_thread(&p, cid));
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            let mut undos = 0;
            for _ in 0..30 {
                assert!(!draw(&ctx, &mut p, cid, &mut undos));
            }
            assert_eq!(undos, 0);
            assert!(!crate::ui::html_ui::take_fields_edited());
            assert!(!ctx.has_requested_repaint(), "idle thread section requested a repaint ({kind:?})");
        }
    }

    /// The list buttons are what the data calls are: a second page with the same thread, edited.
    #[test]
    fn an_agent_edit_shows_up_and_the_page_stays_a_thread() {
        let t = Thread::starter(Kind::Chat, "");
        let (mut p, aid, cid) = project_with(&t);
        let mut n = t.clone();
        n.add(Item::msg("Zed", "hello from outside"), None);
        thread::store(p.asset_mut(aid).unwrap().html.as_mut().unwrap(), &n);
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut undos = 0;
        assert!(!draw(&ctx, &mut p, cid, &mut undos));
        let back = thread::from_src(&p.asset(aid).unwrap().html.as_ref().unwrap().src).unwrap();
        assert_eq!(back.items.len(), 4);
        assert!(thread_of(aid, "<p>plain</p>").is_none());
    }

    #[test]
    fn a_new_speaker_follows_the_last_one_round_the_cast() {
        let t = Thread::starter(Kind::Chat, "");
        assert_eq!(who_default(&t), "Jordan", "the last message is Maya's");
        assert_eq!(who_default(&Thread::blank(Kind::Chat, "")), "Person");
        let _ = item_title(&t, &t.items[0]);
    }

    #[test]
    fn fitting_a_clip_stops_at_the_next_one() {
        let t = Thread::starter(Kind::Chat, "");
        let (mut p, _, cid) = project_with(&t);
        fit_clip(&mut p, cid, 20.0);
        assert!((p.clip(cid).unwrap().duration - 20.0).abs() < 1e-9);
        fit_clip(&mut p, cid, 2.0);
        assert!((p.clip(cid).unwrap().duration - 2.0).abs() < 1e-9);
    }
}
