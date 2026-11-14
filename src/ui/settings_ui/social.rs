//! ---- ws:social-guides ----
//! Settings ▸ Social: the fake accounts the Social Guide preview shows (handle, name, picture, caption,
//! counts, sample comments). Fill them by hand, or press "Fetch from my public profile": that downloads ONE
//! public page without logging in, shows exactly what it found and where from, and only then applies it.
//! Nothing here asks for a password, a token or a cookie, and nothing touches the network until that click.

use crate::settings::Settings;
use crate::ui::social::profile::{self, Fetched, SampleComment, SocialProfile, NETS};
use eframe::egui;
use std::sync::mpsc::{channel, Receiver};

#[derive(Default)]
pub struct SocialUi {
    net: String,
    /// The profile being edited (id).
    sel: String,
    /// Text of the fetch box ("" = the profile's own link / handle).
    url: String,
    pending: Option<Receiver<Result<Fetched, String>>>,
    /// What the last fetch found, for which profile, waiting for "Apply".
    found: Option<(String, Fetched)>,
    message: String,
    avatar: Option<(String, egui::TextureHandle)>,
    /// The selected sample comment.
    csel: usize,
    /// guides.json update in flight / its result.
    update: Option<Receiver<Result<String, String>>>,
}

pub(super) fn social_tab(ui: &mut egui::Ui, st: &mut SocialUi, s: &mut Settings) -> bool {
    let mut changed = false;
    if st.net.is_empty() {
        st.net = "youtube".into();
    }
    ui.weak("Fake accounts for the Social Guide preview (viewer right-click ▸ Social Guides). No logins, no passwords, no tokens are ever asked for or stored.");
    ui.horizontal(|ui| {
        for (id, name) in NETS {
            if ui.selectable_label(st.net == id, name).clicked() {
                st.net = id.to_string();
                st.sel.clear();
                st.found = None;
            }
        }
    });
    ui.separator();
    let net = st.net.clone();
    // list: a table - [grip][arrow][name, fills][handle][followers][active] and fixed right controls
    let gidx: Vec<usize> = s.social_profiles.iter().enumerate().filter(|(_, p)| p.net == net).map(|(i, _)| i).collect();
    if !gidx.iter().any(|&i| s.social_profiles[i].id == st.sel) {
        st.sel = gidx.first().map(|&i| s.social_profiles[i].id.clone()).unwrap_or_default();
    }
    let active = s.active_profile(&net).map(|p| p.id.clone());
    let rows: Vec<Row> = gidx
        .iter()
        .map(|&i| {
            let p = &s.social_profiles[i];
            Row {
                label: p.label(),
                cols: vec![(format!("@{}", p.handle.trim_start_matches('@')), 110.0), (p.followers.clone(), 64.0), (if active.as_deref() == Some(p.id.as_str()) { "● active".into() } else { String::new() }, 64.0)],
                selected: p.id == st.sel,
            }
        })
        .collect();
    let out = list_table(ui, "social_profiles", &["Name", "Handle", "Followers", ""], &rows, true);
    if let Some(i) = out.click {
        st.sel = s.social_profiles[gidx[i]].id.clone();
        st.found = None;
    }
    let mut mv = out.moved;
    if mv.is_none() && ui.ctx().memory(|m| m.focused().is_none()) {
        if let Some(i) = gidx.iter().position(|&g| s.social_profiles[g].id == st.sel) {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp)) && i > 0 {
                mv = Some((i, i - 1));
            } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown)) && i + 1 < gidx.len() {
                mv = Some((i, i + 1));
            }
        }
    }
    if let Some((from, to)) = mv {
        let mut items: Vec<SocialProfile> = gidx.iter().map(|&g| s.social_profiles[g].clone()).collect();
        let it = items.remove(from);
        items.insert(to, it);
        for (&g, it) in gidx.iter().zip(items) {
            s.social_profiles[g] = it;
        }
        changed = true;
    }
    if let Some(i) = out.delete {
        let id = s.social_profiles[gidx[i]].id.clone();
        s.social_profiles.remove(gidx[i]);
        s.social_active.retain(|_, v| *v != id);
        changed = true;
    }
    if let Some(i) = out.make_active {
        s.social_active.insert(net.clone(), s.social_profiles[gidx[i]].id.clone());
        changed = true;
    }
    if ui.button("+ New profile").clicked() {
        let mut p = SocialProfile::sample(&net);
        p.id = s.new_profile_id();
        p.name = format!("Profile {}", gidx.len() + 1);
        st.sel = p.id.clone();
        s.social_profiles.push(p);
        changed = true;
    }
    let Some(idx) = s.social_profiles.iter().position(|p| p.id == st.sel && p.net == net) else {
        ui.add_space(8.0);
        ui.label("No profile for this network yet. The preview uses a made-up sample until you add one.");
        return changed | updates_row(ui, st, s);
    };
    // avatar texture
    let p = &s.social_profiles[idx];
    if st.avatar.as_ref().is_none_or(|(k, _)| *k != p.avatar) {
        st.avatar = None;
        if let Some(f) = profile::avatar(p, false) {
            let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
            st.avatar = Some((p.avatar.clone(), ui.ctx().load_texture("social_settings_avatar", img, egui::TextureOptions::LINEAR)));
        } else if profile::avatar_pending(p) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    let mut picked: Option<std::path::PathBuf> = None;
    let mut clear_avatar = false;
    let p = &mut s.social_profiles[idx];
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
        match &st.avatar {
            Some((_, t)) => {
                let mut m = egui::Mesh::with_texture(t.id());
                let n = 40u32;
                m.vertices.push(egui::epaint::Vertex { pos: r.center(), uv: egui::pos2(0.5, 0.5), color: egui::Color32::WHITE });
                for i in 0..=n {
                    let a = std::f32::consts::TAU * i as f32 / n as f32;
                    let pos = r.center() + egui::vec2(a.cos(), a.sin()) * 32.0;
                    m.vertices.push(egui::epaint::Vertex { pos, uv: egui::pos2(a.cos() * 0.5 + 0.5, a.sin() * 0.5 + 0.5), color: egui::Color32::WHITE });
                    if i > 0 {
                        m.add_triangle(0, i, i + 1);
                    }
                }
                ui.painter().add(egui::Shape::mesh(m));
            }
            None => {
                ui.painter().circle_filled(r.center(), 32.0, egui::Color32::from_gray(70));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "No picture", egui::FontId::proportional(10.0), egui::Color32::GRAY);
            }
        }
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui.button("Choose picture…").clicked() {
                    picked = rfd::FileDialog::new().add_filter("Picture", &["png", "jpg", "jpeg", "webp", "bmp", "gif"]).pick_file();
                }
                if ui.add_enabled(!p.avatar.is_empty(), egui::Button::new("Remove")).clicked() {
                    clear_avatar = true;
                }
            });
        });
    });
    egui::Grid::new("social_fields").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, label: &str, v: &mut String, hint: &str| -> bool {
            ui.label(label);
            let c = ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(360.0)).changed();
            ui.end_row();
            c
        };
        changed |= row(ui, "Profile name", &mut p.name, "only for this list");
        changed |= row(ui, "Handle", &mut p.handle, "yourchannel (no @)");
        changed |= row(ui, "Display name", &mut p.display, "Your Channel");
        ui.label("Verified badge");
        changed |= ui.checkbox(&mut p.verified, "").changed();
        ui.end_row();
        changed |= row(ui, if net == "youtube" { "Subscribers" } else { "Followers" }, &mut p.followers, "12.4K");
        changed |= row(ui, "Hashtags", &mut p.tags, "#roblox #gaming");
        changed |= row(ui, "Sound / music", &mut p.sound, "Original sound - yourchannel");
        changed |= row(ui, "Likes", &mut p.likes, "12K");
        changed |= row(ui, "Comments", &mut p.comments, "482");
        changed |= row(ui, "Shares", &mut p.shares, "1.2K");
        changed |= row(ui, "Saves", &mut p.saves, "3.1K");
        changed |= row(ui, "Views", &mut p.views, "184K");
        changed |= row(ui, "Channel link", &mut p.link, "https://...");
        ui.label("Caption / description");
        changed |= ui.add(egui::TextEdit::multiline(&mut p.caption).desired_rows(3).desired_width(360.0)).changed();
        ui.end_row();
    });
    if !p.note.is_empty() {
        ui.weak(&p.note);
    }

    // sample comments: a table, and the selected one in a two-column form
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Sample comments").strong());
    if st.csel >= p.comment_list.len() {
        st.csel = p.comment_list.len().saturating_sub(1);
    }
    let crow: Vec<Row> = p
        .comment_list
        .iter()
        .enumerate()
        .map(|(i, c)| Row { label: c.text.clone(), cols: vec![(c.name.clone(), 110.0), (c.likes.clone(), 64.0), (c.when.clone(), 64.0)], selected: i == st.csel })
        .collect();
    let co = list_table(ui, "social_comments", &["Comment", "Name", "Likes", "When"], &crow, false);
    if let Some(i) = co.click {
        st.csel = i;
    }
    let mut cmv = co.moved;
    if cmv.is_none() && !p.comment_list.is_empty() && ui.ctx().memory(|m| m.focused().is_none()) {
        let i = st.csel;
        if ui.input_mut(|k| k.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp)) && i > 0 {
            cmv = Some((i, i - 1));
        } else if ui.input_mut(|k| k.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown)) && i + 1 < p.comment_list.len() {
            cmv = Some((i, i + 1));
        }
    }
    if let Some((from, to)) = cmv {
        let c = p.comment_list.remove(from);
        p.comment_list.insert(to, c);
        st.csel = to;
        changed = true;
    }
    if let Some(i) = co.delete {
        p.comment_list.remove(i);
        changed = true;
    }
    if ui.button("+ Add comment").clicked() {
        p.comment_list.push(SampleComment { name: "viewer".into(), text: "great video".into(), likes: "3".into(), when: "1h".into() });
        st.csel = p.comment_list.len() - 1;
        changed = true;
    }
    if let Some(c) = p.comment_list.get_mut(st.csel) {
        egui::Grid::new("social_comment_fields").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
            for (label, v, w) in [("Name", &mut c.name, 160.0), ("Comment", &mut c.text, 360.0), ("Likes", &mut c.likes, 80.0), ("When", &mut c.when, 80.0)] {
                ui.label(label);
                changed |= ui.add(egui::TextEdit::singleline(v).desired_width(w)).changed();
                ui.end_row();
            }
        });
    }

    // fetch
    ui.add_space(6.0);
    ui.separator();
    ui.label(egui::RichText::new("Fetch from my public profile").strong());
    ui.weak("Downloads the public page (no login, no cookies) and reads its name, picture and follower text. Only when you press the button.");
    let hint = if p.link.is_empty() { format!("{} URL or @handle", profile::net_name(&net)) } else { p.link.clone() };
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.url).hint_text(hint).desired_width(360.0));
        let busy = st.pending.is_some();
        if ui.add_enabled(!busy, egui::Button::new(if busy { "Fetching…" } else { "Fetch" })).clicked() {
            let input = if st.url.trim().is_empty() { if p.link.is_empty() { p.handle.clone() } else { p.link.clone() } } else { st.url.clone() };
            let (tx, rx) = channel();
            let net2 = net.clone();
            std::thread::spawn(move || {
                let _ = tx.send(profile::fetch(&net2, &input));
            });
            st.pending = Some(rx);
            st.message.clear();
            st.found = None;
        }
    });
    if let Some(rx) = &st.pending {
        match rx.try_recv() {
            Ok(Ok(f)) => {
                st.found = Some((p.id.clone(), f));
                st.pending = None;
            }
            Ok(Err(e)) => {
                st.message = e;
                st.pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(150)),
            Err(_) => st.pending = None,
        }
    }
    if !st.message.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, &st.message);
    }
    if let Some((id, f)) = st.found.clone() {
        if id == p.id {
            ui.group(|ui| {
                ui.label("This is exactly what was fetched:");
                for l in f.report() {
                    ui.label(l);
                }
                if f.title.is_empty() && f.description.is_empty() && f.image_url.is_empty() {
                    ui.colored_label(ui.visuals().warn_fg_color, "The page has no public name or picture (many sites hide them without a login). Fill the fields by hand.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Apply to this profile").clicked() {
                        f.apply(p);
                        changed = true;
                        st.found = None;
                    }
                    if ui.button("Discard").clicked() {
                        st.found = None;
                    }
                });
            });
        }
    }

    // import / export
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Export profiles…").on_hover_text("Every profile, pictures included, in one file you can move to another PC").clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Social profiles", &["json"]).set_file_name("social-profiles.json").save_file() {
                st.message = match serde_json::to_string_pretty(&s.social_profiles).map_err(|e| e.to_string()).and_then(|t| std::fs::write(&path, t).map_err(|e| e.to_string())) {
                    Ok(()) => format!("Saved {} profiles to {}", s.social_profiles.len(), path.display()),
                    Err(e) => e,
                };
            }
        }
        if ui.button("Import profiles…").clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("Social profiles", &["json"]).pick_file() {
                st.message = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Vec<SocialProfile>>(&t).map_err(|e| e.to_string())) {
                    Ok(list) => {
                        let n = list.len();
                        for mut p in list {
                            if s.social_profiles.iter().any(|q| q.id == p.id) {
                                p.id = s.new_profile_id();
                            }
                            s.social_profiles.push(p);
                        }
                        changed = true;
                        format!("Imported {n} profiles")
                    }
                    Err(e) => format!("Not a profiles file: {e}"),
                };
            }
        }
    });
    // apply the deferred edits
    let Some(idx) = s.social_profiles.iter().position(|q| q.id == st.sel) else { return changed };
    if let Some(path) = picked {
        match profile::avatar_from_file(&path) {
            Ok(b64) => {
                s.social_profiles[idx].avatar = b64;
                changed = true;
            }
            Err(e) => st.message = e,
        }
    }
    if clear_avatar {
        s.social_profiles[idx].avatar.clear();
        changed = true;
    }
    changed | updates_row(ui, st, s)
}

/// Guides data: the verified date, and the explicit "update from the library" click.
fn updates_row(ui: &mut egui::Ui, st: &mut SocialUi, s: &Settings) -> bool {
    ui.add_space(6.0);
    ui.separator();
    let d = crate::ui::social::data();
    ui.label(egui::RichText::new("Guide data").strong());
    ui.weak(format!(
        "{} platforms, measured on the real apps' public pages; file verified {}. Platforms change their UI: if a guide looks out of date, update it or drop a newer guides.json in {}.",
        d.platforms.len(),
        d.verified,
        crate::ui::social::user_path().parent().map(|p| p.display().to_string()).unwrap_or_default()
    ));
    ui.horizontal(|ui| {
        let busy = st.update.is_some();
        if ui.add_enabled(!busy, egui::Button::new("Check the library for updated guides")).clicked() {
            let src = crate::engine::library::Source::of(s);
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::engine::library::fetch_guides(&src));
            });
            st.update = Some(rx);
        }
        if ui.button("Open folder").clicked() {
            let dir = crate::ui::social::user_path().parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
    });
    if let Some(rx) = &st.update {
        match rx.try_recv() {
            Ok(r) => {
                st.message = match r {
                    Ok(t) => match crate::ui::social::install_user_file(&t) {
                        Ok(n) => format!("Guides updated: {n} platforms, verified {}", crate::ui::social::data().verified),
                        Err(e) => e,
                    },
                    Err(e) => format!("Could not fetch guides: {e}"),
                };
                st.update = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(150)),
            Err(_) => st.update = None,
        }
    }
    if !st.message.is_empty() {
        ui.label(&st.message);
    }
    false
}

// ---------------------------------------------------------------- list rows as a table

const ROW_H: f32 = 24.0;

struct Row {
    label: String,
    /// Fixed value columns: (text, width).
    cols: Vec<(String, f32)>,
    selected: bool,
}

#[derive(Default)]
struct ListOut {
    click: Option<usize>,
    /// (from, to) after a grip drag or the up / down buttons.
    moved: Option<(usize, usize)>,
    delete: Option<usize>,
    make_active: Option<usize>,
}

/// Rows with aligned columns: [grip][arrow][label: fills, ellipsis][fixed value columns][fixed right controls
/// up / down / delete (and Use when `use_btn`)]. Controls never follow variable-length text: they sit at the
/// same x in every row. Reorder: drag the grip, the arrows, or Alt+Up/Down on the selected row.
/// ponytail: replace with the shared `ui::list_rows` once it lands.
fn list_table(ui: &mut egui::Ui, id: &str, heads: &[&str], rows: &[Row], use_btn: bool) -> ListOut {
    let mut out = ListOut::default();
    let w = ui.available_width().min(640.0);
    let ctl_w = if use_btn { 24.0 * 3.0 + 40.0 } else { 24.0 * 3.0 };
    let val_w: f32 = rows.first().map_or(0.0, |r| r.cols.iter().map(|c| c.1).sum());
    let label_w = (w - 40.0 - val_w - ctl_w).max(60.0);
    let font = egui::FontId::proportional(13.0);
    let weak = ui.visuals().weak_text_color();
    let text = ui.visuals().text_color();
    // header: the same columns
    let (hr, _) = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover());
    let mut x = hr.min.x + 40.0;
    ui.painter().text(egui::pos2(x, hr.center().y), egui::Align2::LEFT_CENTER, heads.first().copied().unwrap_or(""), egui::FontId::proportional(11.0), weak);
    x += label_w;
    for (i, c) in rows.first().map(|r| r.cols.as_slice()).unwrap_or(&[]).iter().enumerate() {
        ui.painter().text(egui::pos2(x, hr.center().y), egui::Align2::LEFT_CENTER, heads.get(i + 1).copied().unwrap_or(""), egui::FontId::proportional(11.0), weak);
        x += c.1;
    }
    let top = ui.cursor().min.y;
    let n = rows.len();
    for (i, r) in rows.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, ROW_H), egui::Sense::click());
        let p = ui.painter_at(rect);
        if r.selected {
            p.rect_filled(rect, 3.0, ui.visuals().selection.bg_fill.gamma_multiply(0.5));
        } else if resp.hovered() {
            p.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        if resp.clicked() {
            out.click = Some(i);
        }
        // grip (drag to reorder)
        let grip = egui::Rect::from_min_size(rect.min, egui::vec2(20.0, ROW_H));
        let gr = ui.interact(grip, ui.id().with((id, "grip", i)), egui::Sense::drag());
        for dy in [-4.0, 0.0, 4.0] {
            for dx in [-2.0, 2.0] {
                p.circle_filled(grip.center() + egui::vec2(dx, dy), 1.2, if gr.hovered() { text } else { weak });
            }
        }
        if gr.hovered() || gr.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if gr.drag_stopped() {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                let to = (((pos.y - top - 18.0) / ROW_H).floor().max(0.0) as usize).min(n - 1);
                if to != i {
                    out.moved = Some((i, to));
                }
            }
        }
        if r.selected {
            p.text(egui::pos2(rect.min.x + 22.0, rect.center().y), egui::Align2::LEFT_CENTER, "▸", font.clone(), text);
        }
        // label: fills, ellipsis
        let mut job = egui::text::LayoutJob::simple_singleline(r.label.clone(), font.clone(), text);
        job.wrap.max_width = label_w - 8.0;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        p.galley(egui::pos2(rect.min.x + 40.0, rect.center().y - galley.size().y / 2.0), galley, text);
        // fixed value columns
        let mut x = rect.min.x + 40.0 + label_w;
        for (t, cw) in &r.cols {
            let mut job = egui::text::LayoutJob::simple_singleline(t.clone(), font.clone(), weak);
            job.wrap.max_width = cw - 6.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, weak);
            x += cw;
        }
        // fixed controls, right aligned
        let mut cx = rect.max.x - ctl_w;
        let mut btn = |ui: &mut egui::Ui, label: &str, w: f32, on: bool, tip: &str| -> bool {
            let br = egui::Rect::from_min_size(egui::pos2(cx, rect.min.y + 2.0), egui::vec2(w, ROW_H - 4.0));
            cx += w;
            let resp = ui.interact(br, ui.id().with((id, label, i)), if on { egui::Sense::click() } else { egui::Sense::hover() });
            let col = if !on { weak.gamma_multiply(0.4) } else if resp.hovered() { text } else { weak };
            ui.painter().text(br.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.5), col);
            resp.on_hover_text(tip).clicked() && on
        };
        if use_btn && btn(ui, "Use", 40.0, !r.cols.last().is_some_and(|c| !c.0.is_empty()), "Use this profile in the guide preview") {
            out.make_active = Some(i);
        }
        if btn(ui, "▲", 24.0, i > 0, "Move up (Alt+Up)") {
            out.moved = Some((i, i - 1));
        }
        if btn(ui, "▼", 24.0, i + 1 < n, "Move down (Alt+Down)") {
            out.moved = Some((i, i + 1));
        }
        if btn(ui, "✕", 24.0, true, "Delete") {
            out.delete = Some(i);
        }
    }
    if rows.is_empty() {
        ui.weak("(none yet)");
    }
    out
}
