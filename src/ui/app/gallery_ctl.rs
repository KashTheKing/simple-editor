//! ---- ws:inspector-gallery ----
//! PANE_DRAWERS entry for `Pane::Presets` (the Gallery): supersedes size-diet's `library::reuse_ui`
//! placeholder that still lives inline in `panes.rs`'s `Pane::Presets` match arm - PANE_DRAWERS is tried
//! BEFORE that legacy match (registry protocol, `plans/ui-overhaul/README.md`), so returning `true` here
//! makes that old arm unreachable dead code by design, not by an edit to a file this workstream doesn't
//! own. Every mutation is routed through `App::run_tool_undoable("gallery.apply", …)` so the UI path and
//! the MCP tool share one code path (`tools_gallery::apply_card`) and one undo.

use super::*;
use crate::ui::gallery;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Presets {
        return false;
    }
    app.build_gallery_thumbnails(ui.ctx());
    // one scroll area (inside `gallery::show`, under the pinned tab strip) - no outer one around it
    let resp = {
        let App { gallery: st, settings, project, selection, palette, .. } = app;
        gallery::show(ui, st, settings, project, selection, palette)
    };
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
    let want = resp.hover.map(|(tab, name)| monitor::AltRequest::Gallery(tab.name().to_string(), name));
    app.alt_render.request(want);
    true
}
