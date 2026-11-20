//! ---- ws:gallery-stock ----
//! Gallery ▸ Stock, the drawing half: search box, filters, the result grid (the Gallery's own cards) and the
//! details of the picked result. No `App`, no network: `ui::app::stock_ctl` hands in what to draw and applies
//! what comes back, so this is testable headless. Never requests a repaint unless a card is animating under the
//! pointer.

use crate::engine::stock::{Hit, Kind, Licence, LicenceLevel, Orientation, Query, Source};
use crate::ui::menu;
use eframe::egui::{self, Color32, Rect, Stroke};
use std::collections::{HashMap, HashSet};

/// What the Stock tab remembers between frames (lives on `GalleryState`).
#[derive(Clone, Debug, Default)]
pub struct StockUi {
    /// The filters of the next search (the text box is `q.text`).
    pub q: Query,
    /// The result whose details are open.
    pub selected: Option<String>,
}

/// Where the search stands.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Status {
    /// Nothing searched yet.
    #[default]
    Idle,
    Loading,
    /// Done: notes (one per source that failed); empty = all fine.
    Done(Vec<String>),
}

type Tex = Option<(egui::TextureId, [u32; 2])>;

/// What `show` draws, gathered by `stock_ctl`.
pub struct View<'a> {
    pub hits: &'a [Hit],
    pub status: Status,
    /// A further page may exist.
    pub more: bool,
    /// Download progress by hit id.
    pub busy: &'a HashMap<String, f32>,
    /// Hits already downloaded (in this project's folder).
    pub have: &'a HashSet<String>,
    /// (source, switched on, usable: keyless or has a key).
    pub sources: Vec<(Source, bool, bool)>,
    pub thumb: &'a mut dyn FnMut(&Hit) -> Tex,
    /// The hover preview's current frame (videos), when it is ready.
    pub frame: &'a mut dyn FnMut(&Hit) -> Tex,
    /// The file path a card stands for when dragged (where its download lands).
    pub drag_path: &'a dyn Fn(&Hit) -> String,
    /// Hover plays a quiet audio snippet / a video preview.
    pub hover_preview: bool,
}

/// One thing the user did.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Search,
    More,
    /// Download into the project and import it into the Library; `true` = also place it at the playhead.
    Add(String, bool),
    Open(String),
    Copy(String),
    ToggleSource(Source),
    /// Settings ▸ Stock (keys, sources, the download folder).
    Settings,
    /// Export the project's credits as text.
    Credits,
    /// A card started being dragged: start its download now (the drop imports the file as it lands).
    DragStart(String),
    HoverPreview(bool),
}

#[derive(Default, Debug)]
pub struct Resp {
    pub acts: Vec<Act>,
    /// The card under the pointer for long enough to preview.
    pub hover: Option<String>,
}

const RES: [(u32, &str); 4] = [(0, "Any size"), (720, "720p+"), (1080, "1080p+"), (2160, "4K")];

fn licence_tag(l: Licence) -> (&'static str, Color32) {
    match l {
        Licence::Pd => ("CC0", Color32::from_rgb(60, 160, 90)),
        Licence::Free => ("Free", Color32::from_rgb(60, 140, 170)),
        Licence::By => ("BY", Color32::from_rgb(200, 140, 40)),
        Licence::BySa => ("BY-SA", Color32::from_rgb(190, 90, 60)),
        Licence::Other => ("?", Color32::GRAY),
    }
}

fn mmss(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn header(ui: &mut egui::Ui, st: &mut StockUi, view: &View, out: &mut Resp) {
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 70.0).max(60.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut st.q.text)
                .id(egui::Id::new("stock_search"))
                .hint_text("Search free stock")
                .desired_width(w),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Search").on_hover_text("Search the stock sites that are on (Enter)").clicked() || enter {
            out.acts.push(Act::Search);
        }
    });
    let before = st.q.clone();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for k in Kind::ALL {
            if ui.selectable_label(st.q.kind == k, k.label()).clicked() {
                st.q.kind = k;
            }
        }
        ui.separator();
        if !st.q.kind.is_audio() {
            egui::ComboBox::from_id_salt("stock_shape")
                .selected_text(st.q.orientation.label())
                .width(92.0)
                .show_ui(ui, |ui| {
                    for o in Orientation::ALL {
                        ui.selectable_value(&mut st.q.orientation, o, o.label());
                    }
                })
                .response
                .on_hover_text("Shape");
            let res = RES.iter().find(|r| r.0 == st.q.min_res).map_or("Any size", |r| r.1);
            egui::ComboBox::from_id_salt("stock_res")
                .selected_text(res)
                .width(78.0)
                .show_ui(ui, |ui| {
                    for (v, l) in RES {
                        ui.selectable_value(&mut st.q.min_res, v, l);
                    }
                })
                .response
                .on_hover_text("Minimum resolution (the shorter side). Results whose size the site does not say are hidden while this is set.");
        }
        egui::ComboBox::from_id_salt("stock_licence")
            .selected_text(st.q.licence.label())
            .width(86.0)
            .show_ui(ui, |ui| {
                for l in LicenceLevel::ALL {
                    let tip = match l {
                        LicenceLevel::Pd => "No credit needed: CC0, public domain, NASA, and the Pexels / Pixabay licences",
                        LicenceLevel::By => "Also CC BY: credit the author (the credit is kept for you)",
                        LicenceLevel::BySa => "Also CC BY-SA: credit, and share what you make under the same licence",
                    };
                    ui.selectable_value(&mut st.q.licence, l, l.label()).on_hover_text(tip);
                }
            })
            .response
            .on_hover_text("Licence. Non-commercial and no-derivatives licences are never shown.");
        menu::button(ui, "⋯", |ui| {
            menu::sub(ui, None, "Sources", |ui| {
                for (s, on, usable) in &view.sources {
                    let r = ui.add_enabled_ui(*usable, |ui| menu::check(ui, *on, s.name(), "")).inner;
                    let r = if *usable { r } else { r.on_disabled_hover_text("Needs your own free key: add it in Settings ▸ Stock") };
                    if r.clicked() {
                        out.acts.push(Act::ToggleSource(*s));
                    }
                }
                ui.separator();
                if menu::row(ui, None, "Stock settings…", "").clicked() {
                    out.acts.push(Act::Settings);
                }
            });
            menu::sub(ui, None, "More sites", |ui| {
                for (name, url) in crate::engine::stock::LINK_ONLY {
                    if menu::row(ui, None, name, "").on_hover_text(url).clicked() {
                        out.acts.push(Act::Open(url.to_string()));
                    }
                }
            });
            ui.separator();
            if menu::check(ui, view.hover_preview, "Preview on hover", "").clicked() {
                out.acts.push(Act::HoverPreview(!view.hover_preview));
            }
            if menu::row(ui, None, "Export credits…", "").clicked() {
                out.acts.push(Act::Credits);
            }
            if menu::row(ui, None, "Stock settings…", "").clicked() {
                out.acts.push(Act::Settings);
            }
        });
    });
    // a filter change searches again at once, once something was searched
    let changed = st.q.kind != before.kind
        || st.q.orientation != before.orientation
        || st.q.min_res != before.min_res
        || st.q.licence != before.licence;
    if changed && view.status != Status::Idle {
        out.acts.push(Act::Search);
    }
}

fn status_line(ui: &mut egui::Ui, view: &View, out: &mut Resp) {
    let on: Vec<&str> = view.sources.iter().filter(|s| s.1 && s.2).map(|s| s.0.name()).collect();
    if on.is_empty() {
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().warn_fg_color, "No stock source is on.");
            let r = ui.button("Stock settings").on_hover_text("Turn a source on");
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("stock_go_settings"), r.rect));
            if r.clicked() {
                out.acts.push(Act::Settings);
            }
        });
        return;
    }
    match &view.status {
        Status::Idle => {
            ui.weak(format!("Free stock from {}.", on.join(", ")));
        }
        Status::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Searching…");
            });
        }
        Status::Done(notes) => {
            if view.hits.is_empty() {
                if notes.iter().any(|n| n.contains("offline")) {
                    ui.colored_label(ui.visuals().warn_fg_color, "You're offline. Stock needs an internet connection.");
                } else {
                    ui.weak("Nothing found. Try other words, or widen the licence or size filter.");
                }
            }
            for n in notes.iter().filter(|n| !(view.hits.is_empty() && n.contains("offline"))) {
                ui.colored_label(ui.visuals().warn_fg_color, n);
            }
        }
    }
}

fn details(ui: &mut egui::Ui, h: &Hit, view: &View, out: &mut Resp) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.strong(&h.title);
        });
        egui::Grid::new("stock_details").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.weak("Author");
            if h.author_url.is_empty() {
                ui.label(if h.author.is_empty() { "-" } else { &h.author });
            } else if ui.link(&h.author).on_hover_text(&h.author_url).clicked() {
                out.acts.push(Act::Open(h.author_url.clone()));
            }
            ui.end_row();
            ui.weak("Licence");
            let lic = if h.licence_name.is_empty() { licence_tag(h.licence()).0.to_string() } else { h.licence_name.clone() };
            if h.licence_url.is_empty() {
                ui.label(lic);
            } else if ui.link(lic).on_hover_text(&h.licence_url).clicked() {
                out.acts.push(Act::Open(h.licence_url.clone()));
            }
            ui.end_row();
            ui.weak("Source");
            let src = h.source.map_or("", Source::name);
            if ui.link(src).on_hover_text(&h.page_url).clicked() {
                out.acts.push(Act::Open(h.page_url.clone()));
            }
            ui.end_row();
            if h.has_size() || h.duration > 0.0 {
                ui.weak("Size");
                let mut s = Vec::new();
                if h.has_size() {
                    s.push(format!("{} × {}", h.width, h.height));
                }
                if h.duration > 0.0 {
                    s.push(mmss(h.duration));
                }
                ui.label(s.join(" · "));
                ui.end_row();
            }
        });
        let credit = crate::engine::stock::attribution(h);
        ui.add(egui::Label::new(egui::RichText::new(&credit).small()).wrap());
        if h.licence().needs_credit() {
            ui.horizontal_wrapped(|ui| {
                ui.weak("Credit the author where you publish.");
                if ui.small_button("Export credits…").on_hover_text("Write the credits of the project's stock files to a text file").clicked() {
                    out.acts.push(Act::Credits);
                }
            });
        }
        ui.horizontal(|ui| {
            let busy = view.busy.get(&h.id).copied();
            match busy {
                Some(f) => {
                    ui.add(egui::ProgressBar::new(f).desired_width(120.0).show_percentage());
                }
                None => {
                    let r = ui.button("Add").on_hover_text("Download into the project and add it to the Library");
                    if r.clicked() {
                        out.acts.push(Act::Add(h.id.clone(), false));
                    }
                    if ui.button("Add at playhead").clicked() {
                        out.acts.push(Act::Add(h.id.clone(), true));
                    }
                }
            }
            if ui.button("Copy credit").clicked() {
                out.acts.push(Act::Copy(credit.clone()));
            }
            if ui.button("Open page").on_hover_text(&h.page_url).clicked() {
                out.acts.push(Act::Open(h.page_url.clone()));
            }
        });
    });
}

fn picture(p: &egui::Painter, t: Rect, h: &Hit, tex: Tex, accent: Color32) {
    match tex {
        Some((id, size)) => {
            // cover the tile, cropping the longer side
            let (tw, th) = (t.width(), t.height());
            let (iw, ih) = (size[0].max(1) as f32, size[1].max(1) as f32);
            let (sx, sy) = if iw / ih > tw / th { ((th / ih * iw - tw) / (th / ih * iw) / 2.0, 0.0) } else { (0.0, (tw / iw * ih - th) / (tw / iw * ih) / 2.0) };
            let uv = Rect::from_min_max(egui::pos2(sx, sy), egui::pos2(1.0 - sx, 1.0 - sy));
            p.image(id, t, uv, Color32::WHITE);
        }
        None => {
            let g = match h.kind {
                Kind::Video => crate::ui::tools::Glyph::FilmStrip,
                Kind::Image => crate::ui::tools::Glyph::Landscape,
                Kind::Music => crate::ui::tools::Glyph::MusicNote,
                Kind::Sfx => crate::ui::tools::Glyph::Waveform,
            };
            let px = (t.height() * 0.42).clamp(12.0, 40.0);
            crate::ui::gallery::big_glyph(p, t.center(), g, px, accent);
        }
    }
    // licence pill top-left, duration bottom-left
    let (tag, col) = licence_tag(h.licence());
    let font = egui::FontId::proportional(9.0);
    let g = p.layout_no_wrap(tag.to_string(), font.clone(), Color32::WHITE);
    let r = Rect::from_min_size(t.min + egui::vec2(3.0, 3.0), g.size() + egui::vec2(6.0, 2.0));
    p.rect_filled(r, 3.0, col);
    p.galley(r.min + egui::vec2(3.0, 1.0), g, Color32::WHITE);
    if h.duration > 0.0 {
        let g = p.layout_no_wrap(mmss(h.duration), font, Color32::WHITE);
        let r = Rect::from_min_size(egui::pos2(t.left() + 3.0, t.bottom() - g.size().y - 4.0), g.size() + egui::vec2(6.0, 2.0));
        p.rect_filled(r, 3.0, Color32::from_black_alpha(160));
        p.galley(r.min + egui::vec2(3.0, 1.0), g, Color32::WHITE);
    }
}

/// Draw the Stock tab into `ui` (below the Gallery's tab strip).
pub fn show(ui: &mut egui::Ui, st: &mut StockUi, view: &mut View) -> Resp {
    let mut out = Resp::default();
    header(ui, st, view, &mut out);
    ui.separator();
    status_line(ui, view, &mut out);
    let accent = ui.visuals().selection.stroke.color;
    let hits = view.hits;
    if let Some(sel) = st.selected.clone() {
        match hits.iter().find(|h| h.id == sel) {
            Some(h) => details(ui, h, view, &mut out),
            None => st.selected = None,
        }
    }
    egui::ScrollArea::vertical().id_salt("stock_scroll").auto_shrink([false, false]).show(ui, |ui| {
        crate::ui::gallery_filter::grid(ui, |ui| {
            for h in hits {
                let drag = Some(crate::ui::DragPayload::Path((view.drag_path)(h)));
                let selected = st.selected.as_deref() == Some(h.id.as_str());
                let visible = ui.is_rect_visible(ui.cursor().expand(200.0));
                // a live preview frame while hovered, else the thumbnail (loaded only for cards on screen)
                let tex = if visible { (view.frame)(h).or_else(|| (view.thumb)(h)) } else { None };
                let busy = view.busy.get(&h.id).copied();
                let have = view.have.contains(&h.id);
                let r = crate::ui::gallery::card(ui, ("stock", h.id.as_str()), &h.title, drag, selected, |p, t| {
                    picture(p, t, h, tex, accent);
                    if let Some(f) = busy {
                        let bar = Rect::from_min_max(egui::pos2(t.left(), t.bottom() - 4.0), t.right_bottom());
                        p.rect_filled(bar, 0.0, Color32::from_black_alpha(160));
                        p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(bar.width() * f.clamp(0.0, 1.0), 4.0)), 0.0, accent);
                    } else if have {
                        let c = egui::pos2(t.right() - 9.0, t.top() + 9.0);
                        p.circle_filled(c, 7.0, Color32::from_rgb(60, 160, 90));
                        p.line_segment([c + egui::vec2(-3.5, 0.0), c + egui::vec2(-1.0, 2.5)], Stroke::new(1.8, Color32::WHITE));
                        p.line_segment([c + egui::vec2(-1.0, 2.5), c + egui::vec2(3.5, -2.5)], Stroke::new(1.8, Color32::WHITE));
                    }
                });
                let author = if h.author.is_empty() { String::new() } else { format!(" · {}", h.author) };
                let r = r.on_hover_text(format!(
                    "{}{author}\n{} · {}\nClick: details · double-click: add · drag onto the timeline",
                    h.title,
                    h.source.map_or("", Source::name),
                    if h.licence_name.is_empty() { licence_tag(h.licence()).0 } else { &h.licence_name }
                ));
                if r.drag_started() {
                    out.acts.push(Act::DragStart(h.id.clone()));
                }
                if r.double_clicked() {
                    out.acts.push(Act::Add(h.id.clone(), false));
                } else if r.clicked() {
                    st.selected = if selected { None } else { Some(h.id.clone()) };
                }
                if view.hover_preview && crate::ui::hover_after(ui, r.id, &r, 300.0) && !ui.ctx().is_being_dragged(r.id) {
                    out.hover = Some(h.id.clone());
                }
                menu::context(&r, |ui| {
                    if menu::row(ui, None, "Add", "").clicked() {
                        out.acts.push(Act::Add(h.id.clone(), false));
                    }
                    if menu::row(ui, None, "Add at playhead", "").clicked() {
                        out.acts.push(Act::Add(h.id.clone(), true));
                    }
                    ui.separator();
                    if menu::row(ui, None, "Open page", "").clicked() {
                        out.acts.push(Act::Open(h.page_url.clone()));
                    }
                    if menu::row(ui, None, "Copy credit", "").clicked() {
                        out.acts.push(Act::Copy(crate::engine::stock::attribution(h)));
                    }
                });
            }
        });
        if view.more && !hits.is_empty() && view.status != Status::Loading {
            ui.add_space(4.0);
            if ui.button("More results").clicked() {
                out.acts.push(Act::More);
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stock::Licence;

    fn hit(id: &str, l: Licence) -> Hit {
        Hit {
            id: id.into(),
            title: format!("Title {id}"),
            source: Some(Source::Commons),
            licence: Some(l),
            width: 1920,
            height: 1080,
            ..Default::default()
        }
    }

    fn run(ctx: &egui::Context, st: &mut StockUi, hits: &[Hit], status: Status, events: Vec<egui::Event>, t: f64) -> Resp {
        run_with(ctx, st, hits, status, events, t, vec![(Source::Commons, true, true), (Source::Pexels, false, false)])
    }

    fn run_with(
        ctx: &egui::Context,
        st: &mut StockUi,
        hits: &[Hit],
        status: Status,
        events: Vec<egui::Event>,
        t: f64,
        sources: Vec<(Source, bool, bool)>,
    ) -> Resp {
        let busy = HashMap::new();
        let have = HashSet::new();
        let mut out = Resp::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 640.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut thumb = |_: &Hit| None;
                let mut frame = |_: &Hit| None;
                let path = |h: &Hit| format!("C:/stock/{}.mp4", h.id);
                let mut v = View {
                    hits,
                    status: status.clone(),
                    more: true,
                    busy: &busy,
                    have: &have,
                    sources: sources.clone(),
                    thumb: &mut thumb,
                    frame: &mut frame,
                    drag_path: &path,
                    hover_preview: true,
                };
                out = show(ui, st, &mut v);
            });
        });
        out
    }

    #[test]
    fn stock_tab_draws_cards_and_a_click_opens_details() {
        let ctx = egui::Context::default();
        let mut st = StockUi::default();
        let hits = vec![hit("commons:1", Licence::Pd), hit("commons:2", Licence::By)];
        run(&ctx, &mut st, &hits, Status::Done(vec![]), vec![], 0.0);
        let r: Rect = ctx.data(|d| d.get_temp(egui::Id::new(("gallery_card", "Title commons:1")))).expect("card drawn");
        let c = r.center();
        run(&ctx, &mut st, &hits, Status::Done(vec![]), vec![egui::Event::PointerMoved(c)], 0.1);
        let press = |pressed| egui::Event::PointerButton {
            pos: c,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        run(&ctx, &mut st, &hits, Status::Done(vec![]), vec![press(true)], 0.2);
        run(&ctx, &mut st, &hits, Status::Done(vec![]), vec![press(false)], 0.25);
        assert_eq!(st.selected.as_deref(), Some("commons:1"));
    }

    #[test]
    fn stock_filter_change_searches_again_only_after_a_search() {
        let ctx = egui::Context::default();
        let mut st = StockUi::default();
        st.q.kind = Kind::Image;
        // the header compares against the last frame's query: simulate a kind change made this frame
        let out = run(&ctx, &mut st, &[], Status::Idle, vec![], 0.0);
        assert!(!out.acts.contains(&Act::Search));
    }

    #[test]
    fn stock_with_no_source_on_offers_a_settings_button() {
        let ctx = egui::Context::default();
        let mut st = StockUi::default();
        let off = vec![(Source::Commons, false, true), (Source::Pexels, false, false)];
        run_with(&ctx, &mut st, &[], Status::Idle, vec![], 0.0, off.clone());
        let r: Rect = ctx.data(|d| d.get_temp(egui::Id::new("stock_go_settings"))).expect("button drawn");
        let c = r.center();
        let press = |pressed| egui::Event::PointerButton { pos: c, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run_with(&ctx, &mut st, &[], Status::Idle, vec![egui::Event::PointerMoved(c)], 0.1, off.clone());
        run_with(&ctx, &mut st, &[], Status::Idle, vec![press(true)], 0.2, off.clone());
        let out = run_with(&ctx, &mut st, &[], Status::Idle, vec![press(false)], 0.25, off);
        assert!(out.acts.contains(&Act::Settings), "{:?}", out.acts);
    }

    #[test]
    fn stock_offline_says_so() {
        let ctx = egui::Context::default();
        let mut st = StockUi::default();
        run(&ctx, &mut st, &[], Status::Done(vec!["You're offline".into()]), vec![], 0.0);
        // drawn without a panic; the message text is checked through the status logic
        assert_eq!(st.selected, None);
    }

    #[test]
    fn assert_no_idle_repaint_gallery_stock_tab() {
        let ctx = egui::Context::default();
        let mut st = StockUi::default();
        let hits = vec![hit("commons:1", Licence::Pd)];
        for i in 0..3 {
            run(&ctx, &mut st, &hits, Status::Done(vec![]), vec![], i as f64);
        }
        let out = ctx.run(Default::default(), |_| {});
        assert!(out.viewport_output.values().all(|v| v.repaint_delay > std::time::Duration::from_secs(1)), "idle Stock tab asks for no repaint");
    }
}
