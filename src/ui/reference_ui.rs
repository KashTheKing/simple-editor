//! ---- feat/reference-clips ----
//! The Find a Reference pane's drawing: browser search buttons, the GIPHY / Tenor in-app search with its
//! attribution, "Import a Link" with "Just a part", and the project's references (searchable by quote, Copy
//! Credits). Pure UI over `RefUi` + `View`: it returns what was asked (`Asked`) and the app
//! (`ui::app::reference_ctl`) does it - no network or project access here.

use crate::engine::references::{self, GifHit, Provider};
use crate::model::Id;
use crate::ui::list_rows::{Ctrl, RowSpec, Rows};
use eframe::egui;
use std::collections::HashMap;

/// The pane's own state (kept by the app across frames).
#[derive(Default)]
pub struct RefUi {
    /// What the user is looking for (shared by the site buttons and the GIF search).
    pub query: String,
    pub provider: Provider,
    pub hits: Vec<GifHit>,
    /// The query the hits are for.
    pub hits_for: String,
    /// "" or the last search's error / "No results".
    pub status: String,
    /// A search is running (its worker sends the result to the app).
    pub searching: bool,
    /// Result stills, by their URL. A missing entry is drawn as a plain card until it arrives.
    pub thumbs: HashMap<String, egui::TextureHandle>,
    /// "Import a Link": the pasted address and the "Just a part" range.
    pub link: String,
    pub part: bool,
    pub part_text: String,
    /// Filters the project's references by quote / title.
    pub filter: String,
    /// The fair-use note is up, waiting for OK before this import runs.
    pub note: Option<Asked>,
}

/// One reference row as the pane lists it.
pub struct RefRow {
    pub asset: Id,
    pub name: String,
    pub site: String,
    pub quote: String,
    /// It is being transcribed right now.
    pub transcribing: bool,
}

/// What the app tells the pane each frame.
pub struct View<'a> {
    pub has_key: [bool; 2],
    /// yt-dlp was found ("Import a Link" needs it).
    pub ytdlp: bool,
    pub refs: &'a [RefRow],
    /// The Library pane is on screen (its button is lit while it is).
    pub library_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
    /// Open this site's search for the query in the browser.
    OpenSite(&'static str),
    /// Search GIPHY / Tenor in the app.
    Search,
    /// Import result `i` of `hits`.
    ImportGif(usize),
    /// Import the pasted link (with the part, when ticked).
    ImportLink,
    CopyCredits,
    /// Select this asset in the Library.
    Select(Id),
    /// Settings ▸ General (the keys).
    OpenSettings,
    /// The Library pane's button.
    ToggleLibrary,
    /// The fair-use note was accepted: run what it held.
    NoteOk,
}

const THUMB_H: f32 = 84.0;
const CTRLS: &[Ctrl] = &[];

pub fn show(ui: &mut egui::Ui, st: &mut RefUi, v: &View) -> Vec<Asked> {
    let mut out = Vec::new();
    if let Some(held) = st.note.clone() {
        note(ui, st, held, &mut out);
        return out;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        find(ui, st, &mut out);
        ui.separator();
        gifs(ui, st, v, &mut out);
        ui.separator();
        link(ui, st, v, &mut out);
        ui.separator();
        project_refs(ui, st, v, &mut out);
    });
    out
}

/// The query and one button per site, best match first.
fn find(ui: &mut egui::Ui, st: &mut RefUi, out: &mut Vec<Asked>) {
    let r = ui.add(
        egui::TextEdit::singleline(&mut st.query)
            .hint_text("A line, a reaction, a scene…")
            .desired_width(ui.available_width()),
    );
    // Enter runs the in-app search (the site buttons each open a browser tab - never all at once)
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !st.query.trim().is_empty() {
        out.push(Asked::Search);
    }
    let empty = references::clean_query(&st.query).is_empty();
    ui.horizontal_wrapped(|ui| {
        for s in references::suggest(&st.query) {
            let tip = format!("{}\nOpens {} in your browser", s.what, s.name);
            if ui.add_enabled(!empty, egui::Button::new(s.name)).on_hover_text(tip).on_disabled_hover_text("Type what you're looking for first").clicked() {
                out.push(Asked::OpenSite(s.id));
            }
        }
    });
}

/// GIPHY / Tenor in the app: the service, Search, the results and the attribution their terms ask for.
fn gifs(ui: &mut egui::Ui, st: &mut RefUi, v: &View, out: &mut Vec<Asked>) {
    let key = v.has_key[st.provider as usize];
    ui.horizontal(|ui| {
        for p in Provider::ALL {
            if ui.selectable_label(st.provider == p, p.name()).on_hover_text(format!("Search {} inside the app", p.name())).clicked() && st.provider != p {
                st.provider = p;
                st.hits.clear();
                st.status.clear();
            }
        }
        if !key {
            if ui.button("Settings").on_hover_text(format!("Add your free {} key in Settings ▸ General", st.provider.name())).clicked() {
                out.push(Asked::OpenSettings);
            }
        } else {
            let can = !st.searching && !references::clean_query(&st.query).is_empty();
            if ui.add_enabled(can, egui::Button::new("Search")).on_hover_text(format!("Search {} for the text above", st.provider.name())).clicked() {
                out.push(Asked::Search);
            }
        }
        if st.searching {
            ui.spinner(); // animates only while a search runs
        }
    });
    if !key {
        ui.weak(format!("Needs your own free {} key", st.provider.name()));
    }
    if !st.status.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, &st.status);
    }
    if !st.hits.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (i, h) in st.hits.iter().enumerate() {
                let w = if h.height > 0 { (THUMB_H * h.width as f32 / h.height as f32).clamp(48.0, 220.0) } else { THUMB_H };
                let size = egui::vec2(w, THUMB_H);
                let tip = format!("{}\nClick to import into the Library", if h.title.is_empty() { h.provider.name() } else { &h.title });
                let r = match st.thumbs.get(&h.still) {
                    Some(t) => ui.add(egui::Image::new((t.id(), size)).sense(egui::Sense::click())),
                    None => {
                        let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
                        ui.painter().rect_filled(rect, 4.0, ui.visuals().faint_bg_color);
                        r
                    }
                };
                if r.on_hover_text(tip).clicked() {
                    out.push(Asked::ImportGif(i));
                }
            }
        });
        // required next to the results by both services' terms
        ui.weak(st.provider.attribution());
    }
}

/// "Import a Link" (yt-dlp) with "Just a part".
fn link(ui: &mut egui::Ui, st: &mut RefUi, v: &View, out: &mut Vec<Asked>) {
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 70.0).max(80.0);
        ui.add(egui::TextEdit::singleline(&mut st.link).hint_text("Paste a link to import").desired_width(w));
        let ok = v.ytdlp && !st.link.trim().is_empty();
        if ui
            .add_enabled(ok, egui::Button::new("Import"))
            .on_hover_text("Download it into the Library as a reference")
            .on_disabled_hover_text(if v.ytdlp { "Paste a link first" } else { "Needs yt-dlp (Settings ▸ General)" })
            .clicked()
        {
            out.push(Asked::ImportLink);
        }
    });
    if v.ytdlp {
        part_controls(ui, &mut st.part, &mut st.part_text, &st.link);
    }
}

/// The "Just a part" checkbox and its range field (also on the Import URL window). A start time in the link
/// (`?t=83`) fills the field the first time it is ticked.
pub fn part_controls(ui: &mut egui::Ui, part: &mut bool, text: &mut String, link: &str) -> bool {
    let mut enter = false;
    ui.horizontal(|ui| {
        let was = *part;
        ui.checkbox(part, "Just a part").on_hover_text("Download only a time range, e.g. 1:23-1:45 (needs ffmpeg)");
        if *part && !was && text.trim().is_empty() {
            if let Some(t) = prefill_part(link) {
                *text = t;
            }
        }
        if *part {
            let r = ui.add(egui::TextEdit::singleline(text).hint_text("1:23-1:45").desired_width(110.0));
            enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !text.trim().is_empty() && references::parse_range(text).is_none() {
                ui.colored_label(ui.visuals().warn_fg_color, "start-end");
            }
        }
    });
    enter
}

/// "1:23-1:33" for a link that starts at 1:23 (`?t=83`): ten seconds from there, to edit.
pub fn prefill_part(link: &str) -> Option<String> {
    let t = references::url_start_time(link)?;
    Some(format!("{}-{}", references::fmt_time(t), references::fmt_time(t + 10.0)))
}

/// The project's references: filter by quote, Copy Credits, a row per reference (click selects it in the
/// Library).
fn project_refs(ui: &mut egui::Ui, st: &mut RefUi, v: &View, out: &mut Vec<Asked>) {
    ui.horizontal(|ui| {
        ui.strong("In This Project");
        let lib = ui.selectable_label(v.library_open, "Library").on_hover_text("Show the Library (its References bin lists these too)");
        if lib.clicked() {
            out.push(Asked::ToggleLibrary);
        }
        if ui
            .add_enabled(!v.refs.is_empty(), egui::Button::new("Copy Credits"))
            .on_hover_text("Copy where every reference came from, one line each, for a description or end card")
            .clicked()
        {
            out.push(Asked::CopyCredits);
        }
    });
    if v.refs.is_empty() {
        ui.weak("References you import show here, found by what's said in them.");
        return;
    }
    ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Search by quote").desired_width(ui.available_width()));
    let q = st.filter.trim().to_lowercase();
    let mut rows = Rows::new(ui, "reference_rows", CTRLS, &[70.0], false).fixed_order();
    let mut i = 0;
    for r in v.refs {
        if !q.is_empty() && !r.quote.to_lowercase().contains(&q) && !r.name.to_lowercase().contains(&q) {
            continue;
        }
        let value = if r.transcribing { "…" } else { r.site.as_str() };
        let resp = rows.row(ui, i, &RowSpec { key: r.asset, label: &r.name, values: &[value], ..Default::default() });
        i += 1;
        let tip = match (r.transcribing, r.quote.is_empty()) {
            (true, _) => format!("{} - transcribing…", r.site),
            (false, true) => format!("{}\n(no words transcribed)", r.site),
            (false, false) => format!("{}\n\u{201C}{}\u{201D}", r.site, references_short(&r.quote)),
        };
        if resp.body.on_hover_text(tip).clicked() {
            out.push(Asked::Select(r.asset));
        }
    }
    let _ = rows.finish(ui);
}

fn references_short(q: &str) -> String {
    let mut s: String = q.chars().take(240).collect();
    if q.chars().count() > 240 {
        s.push('…');
    }
    s
}

/// The one-time fair-use note, shown in place of the pane until OK.
fn note(ui: &mut egui::Ui, st: &mut RefUi, _held: Asked, out: &mut Vec<Asked>) {
    ui.strong("Before your first reference");
    ui.add_space(4.0);
    ui.label(references::FAIR_USE_NOTE);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("OK").on_hover_text("Import it (this note won't show again)").clicked() {
            out.push(Asked::NoteOk);
        }
        if ui.button("Cancel").clicked() {
            st.note = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &egui::Context, st: &mut RefUi, v: &View) -> Vec<Asked> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 700.0))),
            ..Default::default()
        };
        let mut out = Vec::new();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| out = show(ui, st, v));
        });
        out
    }

    fn rows() -> Vec<RefRow> {
        vec![
            RefRow { asset: 1, name: "The scene".into(), site: "YouTube".into(), quote: "you shall not pass".into(), transcribing: false },
            RefRow { asset: 2, name: "Shrug".into(), site: "Tenor".into(), quote: String::new(), transcribing: false },
        ]
    }

    #[test]
    fn assert_no_idle_repaint_find_a_reference() {
        let ctx = egui::Context::default();
        let mut st = RefUi { query: "mind blown".into(), ..Default::default() };
        st.hits = vec![GifHit { title: "Mind blown".into(), width: 480, height: 270, ..Default::default() }];
        let refs = rows();
        let v = View { has_key: [true, false], ytdlp: true, refs: &refs, library_open: false };
        for _ in 0..30 {
            assert!(run(&ctx, &mut st, &v).is_empty(), "nothing asked without input");
        }
        assert!(!ctx.has_requested_repaint(), "an idle Find a Reference pane must not repaint");
    }

    #[test]
    fn assert_no_idle_repaint_find_a_reference_note() {
        let ctx = egui::Context::default();
        let mut st = RefUi { note: Some(Asked::ImportLink), ..Default::default() };
        let v = View { has_key: [false, false], ytdlp: false, refs: &[], library_open: false };
        for _ in 0..30 {
            let _ = run(&ctx, &mut st, &v);
        }
        assert!(!ctx.has_requested_repaint(), "the fair-use note must not repaint while idle");
    }

    #[test]
    fn part_field_takes_the_links_start_time() {
        let text = prefill_part("https://youtu.be/x?t=83").unwrap();
        assert_eq!(text, "1:23-1:33");
        assert_eq!(references::parse_range(&text), Some((83.0, 93.0)));
        assert_eq!(prefill_part("https://youtu.be/x"), None);
    }
}
