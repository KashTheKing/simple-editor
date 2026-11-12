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
        LibAct::Apply(id) => library_ctl::click(app, &id),
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
        LibAct::UseFolder => {
            if let Some(dir) = rfd::FileDialog::new().set_title("Library folder (has index.json)").pick_folder() {
                app.settings.library_dir = dir.to_string_lossy().into_owned();
                app.settings.save();
                library_ctl::ensure(app, true);
            }
        }
        LibAct::UseOnline => {
            app.settings.library_dir.clear();
            app.settings.save();
            library_ctl::ensure(app, true);
        }
    }
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Presets {
        return false;
    }
    // ---- ws:online-library ---- GPU thumbnails only where they are drawn (Looks / folder LUTs)
    let lib_kind = app.gallery.lib.or((app.gallery.tab == gallery::GalleryTab::Luts).then_some(Kind::Lut));
    if app.gallery.lib.is_none() && matches!(app.gallery.tab, gallery::GalleryTab::Looks | gallery::GalleryTab::Luts) {
        app.build_gallery_thumbnails(ui.ctx());
    }
    if lib_kind.is_some() {
        library_ctl::ensure(app, false); // the first library tab opened: no network before this
    }
    // one scroll area (inside `gallery::show`, under the pinned tab strip) - no outer one around it
    let resp = {
        let src = Source::of(&app.settings);
        let App { gallery: st, settings, project, selection, palette, online, thumbs, .. } = app;
        let cells = lib_kind
            .map(|k| online.rows(Some(k), &st.lib_query))
            .unwrap_or_default()
            .into_iter()
            .map(|r| gallery::LibCell {
                busy: online.busy(&r.id),
                skill: r.kind == Kind::Skill,
                id: r.id,
                name: r.name,
                desc: r.desc,
                installed: r.installed,
            })
            .collect();
        let (note, loading) = (online.note.clone(), online.loading());
        let ctx = ui.ctx().clone();
        let mut thumb = |id: &str| {
            let p = online.thumb_path(id)?;
            thumbs.texture(&ctx, &p.to_string_lossy(), 0.0, 54)
        };
        let lib = lib_kind.map(|_| gallery::LibView {
            cells,
            source: src.label(),
            local: matches!(src, Source::Dir(_)),
            note,
            loading,
            thumb: &mut thumb,
        });
        gallery::show(ui, st, settings, project, selection, palette, lib)
    };
    if let Some(act) = resp.lib {
        library_act(app, act);
    }
    if let Some((tab, name, intensity)) = resp.apply {
        let ids = app.project.expand_links(&app.selection);
        let args = json!({"tab": tab.name(), "name": name, "clip_ids": ids, "intensity": intensity});
        if let Err(e) = app.run_tool_undoable("gallery.apply", &args) {
            app.toast(e);
        }
    }
    if let Some(name) = resp.place {
        app.place_template(&name, app.playhead);
    }
    // ---- ws:text-titles ----
    if let Some(name) = resp.place_title {
        match app.place_title_template(&name, app.playhead) {
            Ok((_, customize)) => {
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
    // "Save look from selection…" (the pane's right-click): capture the first selected clip's effect
    // stack as a reusable EffectPreset - same shape `engine::presets::capture_template`'s siblings use
    if let Some(name) = App::name_window(ui.ctx(), "Save look from selection", &mut app.gallery.save_name) {
        if let Some(c) = app.selection.first().and_then(|&id| app.project.clip(id)) {
            let preset =
                crate::settings::EffectPreset { name, json: serde_json::to_string(&c.effects).unwrap_or_default() };
            app.settings.effect_presets.retain(|p| p.name != preset.name);
            app.settings.effect_presets.push(preset);
            app.settings.save();
            app.toast("Look saved");
        } else {
            app.toast("Select a clip first");
        }
    }
    // ---- ws:keyframe-blocks ----
    if let Some(name) = resp.forget_block {
        if let Err(e) = app.run_tool("keyblocks.forget", &json!({"name": name})) {
            app.toast(e);
        }
    }
    if let Some(name) = App::name_window(ui.ctx(), "Save keyframe block", &mut app.gallery.save_block) {
        match app.selection.first().copied() {
            Some(id) => match super::tools_keyblocks::save_block(app, id, &name, None, None) {
                Ok(()) => app.toast(format!("Saved \"{name}\" to Gallery ▸ Animate")),
                Err(e) => app.toast(e),
            },
            None => app.toast("Select a clip first"),
        }
    }
    let want = resp.hover.map(|(tab, name)| monitor::AltRequest::Gallery(tab.name().to_string(), name));
    app.alt_render.request(want);
    true
}
