//! ---- ws:inspector-gallery ----
//! PANE_DRAWERS entry for `Pane::Presets` (the Gallery): supersedes size-diet's `library::reuse_ui`
//! placeholder that still lives inline in `panes.rs`'s `Pane::Presets` match arm - PANE_DRAWERS is tried
//! BEFORE that legacy match (registry protocol, `plans/ui-overhaul/README.md`), so returning `true` here
//! makes that old arm unreachable dead code by design, not by an edit to a file this workstream doesn't
//! own. Every mutation is routed through `App::run_tool_undoable("gallery.apply", …)` so the UI path and
//! the MCP tool share one code path (`tools_gallery::apply_card`) and one undo.

use super::*;
use crate::engine::library::{self, Kind, Source};
use crate::ui::gallery;

// ---- ws:online-library ----
/// A library cell click / menu choice from the Gallery.
fn library_act(app: &mut App, act: gallery::LibAct) {
    use gallery::LibAct;
    match act {
        LibAct::Apply(id) => library_ctl::click(app, &id, Default::default()),
        LibAct::ApplyAt(id, at) => {
            library_ctl::click(app, &id, library_ctl::Target { place: Some(at.to_string()), ..Default::default() })
        }
        LibAct::Install(id) => {
            library_ctl::start_download(app, &id, library_ctl::Then::Install);
        }
        LibAct::View(id) => library_ctl::view(app, &id),
        LibAct::Remove(id) => {
            let Some((kind, short)) = library::split_id(&id) else { return };
            if library::remove(kind, &short) {
                app.online.installed.remove(&id);
                gallery::forget_lut_listing();
                let msg = format!("Removed {} from this PC", app.online.name_of(&id));
                app.toast(msg);
            }
        }
        LibAct::Reveal(id) => {
            let Some((kind, short)) = library::split_id(&id) else { return };
            let mut p = library::path_of(kind, &short);
            if !p.is_file() {
                p = Source::of(&app.settings).local(&library::file_of(&id).unwrap_or_default());
            }
            if p.is_file() {
                let _ = std::process::Command::new("explorer").arg("/select,").arg(&p).spawn();
            } else {
                app.toast("Not on this PC yet: Install it first");
            }
        }
        LibAct::Refresh => library_ctl::ensure(app, true),
        LibAct::Connect => library_ctl::set_connected(app, true),
        LibAct::Disconnect => library_ctl::set_connected(app, false),
        LibAct::UseFolder => {
            if let Some(dir) = rfd::FileDialog::new().set_title("Library folder (has index.json)").pick_folder() {
                app.settings.library_dir = dir.to_string_lossy().into_owned();
                app.settings.save();
                library_ctl::ensure(app, true);
            }
        }
        LibAct::UseOnline => {
            app.settings.library_dir.clear();
            library_ctl::set_connected(app, true);
        }
    }
}

/// One more use of a Gallery card (the "Most used" order): `tab` is a `GalleryTab::name()` or "lib".
pub(super) fn count_use(app: &mut App, tab: &str, name: &str) {
    *app.settings.gallery_used.entry(gallery::used_key(tab, name)).or_insert(0) += 1;
    app.settings.save();
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Presets {
        return false;
    }
    // ---- ws:online-library ---- GPU thumbnails only where they are drawn (Looks / folder LUTs)
    // (Effects and Transitions list their library items beside the built-in cards)
    let lib_kind = app.gallery.lib.or(gallery::lib_kind_of(app.gallery.tab));
    if app.gallery.lib.is_none() && matches!(app.gallery.tab, gallery::GalleryTab::Looks | gallery::GalleryTab::Luts) {
        app.build_gallery_thumbnails(ui.ctx());
    }
    // ws:animator-templates: the pictures of the saved templates' cards (a `.png` beside each one)
    if app.gallery.lib.is_none() && app.gallery.tab == gallery::GalleryTab::Templates {
        let ctx = ui.ctx().clone();
        let mut m = std::collections::HashMap::new();
        for i in gallery::mine_items(crate::engine::gallery::Kind::Template, ctx.input(|i| i.time)).iter() {
            let png = crate::engine::templates::thumb_path(&i.path);
            if png.is_file() {
                if let Some(t) = app.thumbs.texture(&ctx, &png.to_string_lossy(), 0.0, 54) {
                    m.insert(i.name.clone(), t);
                }
            }
        }
        gallery::set_template_thumbs(m);
    }
    // ---- ws:gallery-audio ---- Titles / Templates pictures, rendered off the UI thread
    if app.gallery.lib.is_none() && matches!(app.gallery.tab, gallery::GalleryTab::Titles | gallery::GalleryTab::Templates) {
        template_thumbs::poll(app, ui.ctx());
    }
    if lib_kind.is_some() {
        // not connected: reads what is installed and nothing else (no network until "Browse online library")
        library_ctl::ensure(app, false);
    }
    // ---- ws:html-layer ---- browsing HTML layers / icons: start Edge now, so the first one placed shows at once
    if matches!(lib_kind, Some(Kind::Html | Kind::Icon)) {
        crate::engine::html::prewarm();
    }
    // ws:gallery-audio: the side / tab the user picks is saved at once (restored by `GalleryState::from_settings`)
    let picked = |s: &crate::settings::Settings| (s.gallery_audio, s.gallery_tab.clone(), s.gallery_audio_tab.clone());
    let before = picked(&app.settings);
    // one scroll area (inside `gallery::show`, under the pinned tab strip) - no outer one around it
    let resp = {
        let src = Source::of(&app.settings);
        let App { gallery: st, settings, project, selection, palette, online, thumbs, transitions_ui: tr, .. } = app;
        // every row: the Gallery's header filters them (search, chips, category, sort)
        let cells = lib_kind
            .map(|k| online.rows(Some(k), ""))
            .unwrap_or_default()
            .into_iter()
            .map(|r| gallery::LibCell {
                busy: online.busy(&r.id),
                skill: r.kind == Kind::Skill,
                id: r.id,
                kind: r.kind,
                name: r.name,
                desc: r.desc,
                installed: r.installed,
                tags: r.tags,
                yours: r.yours,
            })
            .collect();
        let (note, loading, connected) = (online.note.clone(), online.loading(), online.connected);
        let ctx = ui.ctx().clone();
        let mut thumb = |id: &str| {
            let p = online.thumb_path(id).or_else(|| {
                // a skill's own `thumb:` (installed or yours); none: the Gallery draws its card
                let (_, name) = crate::engine::library::split_id(id).filter(|(k, _)| *k == Kind::Skill)?;
                crate::engine::skills::get(&name).and_then(|s| crate::engine::skills::thumb_path(&s))
            })?;
            thumbs.texture(&ctx, &p.to_string_lossy(), 0.0, 54)
        };
        let lib = lib_kind.map(|_| gallery::LibView {
            cells,
            local: matches!(src, Source::Dir(_)),
            note,
            loading,
            connected,
            thumb: &mut thumb,
        });
        gallery::show(ui, st, settings, project, selection, palette, tr, lib)
    };
    if picked(&app.settings) != before {
        app.settings.save();
    }
    if let Some(act) = resp.lib {
        library_act(app, act);
    }
    if let Some((tab, name, intensity)) = resp.apply {
        // (a transition / effect mirrors onto linked clips itself: the plain selection, not its links)
        let ids = match tab {
            gallery::GalleryTab::Effects
            | gallery::GalleryTab::Transitions
            | gallery::GalleryTab::AudioTransitions => app.selection.clone(),
            _ => app.project.expand_links(&app.selection),
        };
        let args = json!({"tab": tab.name(), "name": name, "clip_ids": ids, "intensity": intensity, "where": resp.apply_where});
        match app.run_tool_undoable("gallery.apply", &args) {
            Ok(_) => count_use(app, tab.name(), &name),
            Err(e) => app.toast(e),
        }
    }
    // ws:animator-templates: a template with slots asks for footage first (`template_ctl` counts the use)
    if let Some(name) = resp.place {
        template_ctl::card_clicked(app, &name);
    }
    if resp.builder {
        template_ctl::open_builder(app);
    }
    // ---- ws:text-titles ----
    if let Some(name) = resp.place_title {
        match app.place_title_template(&name, app.playhead) {
            Ok((_, customize)) => {
                count_use(app, "Titles", &name);
                app.gallery.customize = customize;
                if app.gallery.customize.is_empty() {
                    app.toast(format!("Placed \"{name}\" (nothing to customize)"));
                }
            }
            Err(e) => app.toast(e),
        }
    }
    // the Customize rows edit clones (at most one changes per frame from a mouse/keyboard): one undo,
    // then write each back
    if !resp.customized.is_empty() {
        app.push_undo();
        for (cid, clone) in resp.customized {
            if let Some(c) = app.project.clip_mut(cid) {
                *c = clone;
            }
        }
        app.after_edit();
    }
    // ---- ws:my-gallery ----: Save to Gallery…, the Mine menu, Import…, Folder, a clicked sound
    if resp.save_dialog {
        gallery_mine::open_dialog(app, None);
    }
    if let Some((act, r)) = resp.mine {
        gallery_mine::act(app, act, r);
    }
    if let Some(folder) = resp.import {
        gallery_mine::import_dialog(app, folder);
    }
    if let Some(k) = resp.open_folder {
        gallery_mine::open_folder(k);
    }
    if let Some(name) = resp.sound {
        app.push_undo();
        let at = app.playhead;
        match gallery_mine::place_sound(app, &name, at, None, false) {
            Ok(ids) => {
                count_use(app, "Sounds", &name);
                app.selection = ids;
                app.after_edit();
                app.toast(format!("{name} added at the playhead"));
            }
            Err(e) => app.toast(e),
        }
    }
    // waveforms for the sound cards on screen: a few per frame, decoded lazily (nothing at start-up)
    for path in gallery::take_sound_wants().into_iter().take(6) {
        if let Some(pk) = app.waveforms.get(&path.to_string_lossy(), 0) {
            gallery::set_sound_peaks(path, pk);
            ui.ctx().request_repaint();
        }
    }
    // ---- ws:gallery-audio ---- hear an Audio card on the selected clip while the pointer rests on it
    gallery_audition::want(app, ui.ctx(), resp.audition);
    // ---- ws:keyframe-blocks ----
    if let Some(name) = resp.forget_block {
        if let Err(e) = app.run_tool("keyblocks.forget", &json!({"name": name})) {
            app.toast(e);
        }
    }
    // a transition card previews in the monitor (a library one: its own source); the other tabs' requests are
    // the reserved `AltRequest::Gallery`
    let want = resp.hover.map(|(tab, name)| match (tab, crate::model::TransitionKind::from_name(&name)) {
        (gallery::GalleryTab::Transitions, Some(kind)) => monitor::AltRequest::Transition(kind),
        _ => monitor::AltRequest::Gallery(tab.name().to_string(), name),
    });
    let want = resp.hover_shader.map(monitor::AltRequest::ShaderTransition).or(want);
    app.alt_render.request(want);
    true
}
