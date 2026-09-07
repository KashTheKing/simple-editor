use super::*;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    let resp = {
        let live = app.lib_preview_live;
        let App { project, settings, library: lib, ytdlp_available, thumbs, undo, redo, palette, .. } = app;
        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
        let ytdlp = ytdlp_available.load(std::sync::atomic::Ordering::Relaxed);
        library::show(ui, lib, project, settings, Some(thumbs), live.as_ref(), palette, ytdlp, &mut push)
    };
    if resp.edited {
        app.after_edit();
    }
    if resp.settings_changed {
        app.settings.save();
    }
    if resp.import {
        app.act_import();
    }
    // single-clicked in the Library: the file the preview pane should show (source viewer)
    if let Some(p) = resp.preview.clone() {
        app.start_lib_preview(ui.ctx(), p);
    }
    if !resp.add_to_timeline.is_empty() {
        app.push_undo();
        app.insert_at(resp.add_to_timeline, app.playhead, None);
        app.after_edit();
    }
    // both used to sit inside the open_paths branch, so the Library's "New ▸ Adjustment layer"
    // and "Open…" only ever fired on a frame that also opened a file — i.e. never
    if resp.new_adjustment {
        app.pending_actions.push(Action::AddAdjustment);
    }
    if resp.open_dialog {
        app.pending_actions.push(Action::OpenFile);
    }
    if !resp.open_paths.is_empty() {
        let ids = app.open_or_import(&resp.open_paths);
        if !ids.is_empty() {
            app.library.selected = ids.last().copied();
            app.library.tab = 0;
        }
    }
    if !resp.remove.is_empty() {
        app.push_undo();
        for id in resp.remove {
            app.project.remove_asset(id);
        }
        app.after_edit();
    }
    // ---- ws:forgiveness ----
    // "Clear recent" now goes through confirm::ask(ConfirmAction::ClearRecent) -> App::resolve_confirm
    // directly (library.rs), not this response field — the old clear_recent bool/recent_clear() fn
    // were deleted as dead code once that landed.
    if let Some(n) = resp.removed_unused {
        let s = if n == 1 { "" } else { "s" };
        app.toast_undo(format!("Removed {n} unused asset{s}"), Action::Undo);
    }
    for (id, ext) in resp.convert {
        app.start_asset_convert(id, &ext);
    }
    if !resp.regen_proxy.is_empty() {
        // decoders hold the proxy files open on Windows: release them before deleting
        app.player.set_proxies(std::collections::HashMap::new());
        app.player.release_files();
        for src in resp.regen_proxy {
            let _ = std::fs::remove_file(crate::media::proxy::proxy_path(&src, app.settings.proxy_height));
        }
        app.proxy_map.clear();
        crate::media::proxy::set_ready(Vec::new()); // regen: nothing is proxied right now
        app.proxy_scan_at = None; // rebuild + re-push on the next update
    }
    if let Some(id) = resp.convert_dialog {
        app.convert_dialog = Some((id, "mp4".into()));
    }
    if let Some(p) = resp.compress.and_then(|id| app.project.asset(id)).map(|a| a.path.clone()) {
        app.compress = Some(Compress::new(PathBuf::from(p), app.settings.crf));
    }
    if let Some(id) = resp.open_sequence {
        app.enter_sequence(id);
    }
    if resp.import_url {
        app.url_dialog = Some((String::new(), false));
    }
    for name in resp.place_template {
        app.place_template(&name, app.playhead);
    }
    // Recent tab, reusable sections: an effect / saved preset / node graph goes onto the
    // selection (a clip rendering from a graph ignores its linear stack, so it is skipped).
    if let Some(kind) = resp.add_effect {
        let targets: Vec<Id> = app
            .selection
            .iter()
            .copied()
            .filter(|&id| app.project.clip(id).is_some_and(|c| c.is_visual() && !c.uses_graph()))
            .collect();
        if targets.is_empty() {
            app.toast("Select a clip first");
        } else {
            app.push_undo();
            for id in targets {
                if let Some(c) = app.project.clip_mut(id) {
                    c.effects.push(Effect::new(kind));
                }
            }
            app.after_edit();
        }
    }
    if let Some(i) = resp.apply_preset {
        app.apply_effect_preset(i);
    }
    if let Some(from) = resp.copy_graph {
        let graph = app.project.clip(from).and_then(|c| c.graph.clone());
        let targets: Vec<Id> = app
            .selection
            .iter()
            .copied()
            .filter(|&id| id != from && app.project.clip(id).is_some_and(|c| c.is_visual()))
            .collect();
        match graph {
            Some(g) if !targets.is_empty() => {
                app.push_undo();
                for id in targets {
                    if let Some(c) = app.project.clip_mut(id) {
                        c.graph = Some(g.clone());
                    }
                }
                app.after_edit();
            }
            _ => app.toast("Select another clip to copy this node graph onto"),
        }
    }
    // ---- ws:media-library ----
    if !resp.relink.is_empty() {
        if let Some(dir) = rfd::FileDialog::new().set_title("Relink media: pick the folder").pick_folder() {
            media_sync::start_relink(app, &resp.relink, &dir);
        }
    }
    if resp.consolidate {
        media_sync::ask_consolidate(app);
    }
    if !resp.new_subclip.is_empty() {
        // one undo snapshot BEFORE the first row is added, then add_subclip per id — see new_subclips
        app.new_subclips(&resp.new_subclip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// New Subclip must push its undo snapshot BEFORE mutating, so one Ctrl+Z restores the exact
    /// pre-subclip project. `App::new_subclips` needs a live `App` (none is buildable headlessly —
    /// see tools_registry_tests.rs), so this pins both halves separately: the project-level round
    /// trip through the same `Project::add_subclip` + JSON snapshot the method uses, and the method's
    /// own source order (push before the first `add_subclip`, never after).
    #[test]
    fn new_subclip_undo_restores_pre_subclip_project() {
        let mut p = Project::new();
        let a = p.add_asset(crate::engine::import::placeholder("C:/x.mp4"));
        p.asset_mut(a).unwrap().duration = 8.0;
        p.in_point = Some(1.0);
        p.out_point = Some(3.0);
        let before = p.to_json();
        let sub = p.add_subclip(a, 1.0, 3.0, Some("x".into())).unwrap();
        assert_ne!(p.to_json(), before);
        assert_eq!(p.asset(sub).unwrap().range, Some((1.0, 3.0)));
        // Ctrl+Z = restore the snapshot taken before the subclip existed
        let restored = Project::from_json(&before).unwrap();
        assert_eq!(restored.to_json(), before);
        assert!(restored.asset(sub).is_none() && restored.assets.len() == 1);

        let src = include_str!("media_sync.rs");
        let start = src.find("pub(crate) fn new_subclips").expect("new_subclips exists");
        let body = &src[start..start + src[start..].find("\n    }\n").unwrap()];
        let push = body.find("push_undo_labeled").expect("pushes a labelled undo");
        let add = body.find("add_subclip(").expect("adds subclips");
        assert!(push < add, "the undo snapshot must be pushed BEFORE the first subclip is added");
        assert_eq!(body.matches("push_undo").count(), 1, "exactly one undo step for the whole batch");
    }
}
