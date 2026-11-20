//! ---- ws:mixed-footage ----
//! The App side of mixed camera footage: small cards above "Match project" for heavy footage ("make
//! proxies?", the answer remembered in `Settings.footage_proxies`), S-Log3 footage (add the conversion LUT
//! to those sources) and clips from two or more cameras on the timeline ("Match colours between cameras",
//! never automatic); plus the Inspector's and the timeline menu's requests (`inspector_footage::request`).
//! Everything here runs on a click or when an import lands; the per-frame part is one hash over the
//! timeline's video clips.

use super::*;
use crate::media::footage;
use crate::ui::inspector_footage::{self, FootageReq};
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub(crate) struct State {
    /// Heavy sources waiting on "make proxies?".
    proxies: Vec<Id>,
    /// Log / untagged-HLG sources waiting on "add the LUT?", with their gamma ("S-Log3", "S-Log2", "HLG").
    log: Vec<(Id, &'static str)>,
    /// Cameras on the timeline and their clips, while the camera card is up (2+ cameras).
    cams: Vec<(String, Vec<Id>)>,
    /// The reference camera (index into `cams`).
    reference: usize,
    /// Camera sets already offered this session, and the timeline fingerprint last scanned.
    offered: BTreeSet<String>,
    scanned: u64,
}

pub(super) fn has_card(app: &App) -> bool {
    let s = &app.footage;
    !s.proxies.is_empty() || !s.log.is_empty() || !s.cams.is_empty()
}

/// Footage that stutters without a proxy: 4K long-GOP or 10-bit HEVC.
pub(crate) fn heavy(a: &crate::model::Asset) -> bool {
    let long_gop = matches!(a.codec.as_str(), "h264" | "hevc" | "av1" | "vp9" | "mpeg2video" | "mpeg4");
    let ten_bit = footage::of(&a.path).is_some_and(|m| m.ten_bit);
    (long_gop && a.width as u64 * a.height as u64 > 1920 * 1088) || (a.codec == "hevc" && ten_bit)
}

impl App {
    /// An import landed (`videos` carry their library ids): offer proxies for heavy footage that would not
    /// get one anyway, and the conversion LUT for S-Log3, S-Log2 and untagged HLG.
    pub(super) fn footage_landed(&mut self, videos: &[crate::model::Asset]) {
        let auto = self.settings.use_proxies;
        let slow: Vec<Id> = videos
            .iter()
            .filter(|a| a.proxy.is_none() && heavy(a) && !(auto && crate::media::proxy::needs_proxy(a)))
            .map(|a| a.id)
            .collect();
        self.offer_proxies(&slow);
        let log =
            videos.iter().filter_map(|a| footage::of(&a.path).filter(footage::wants_lut).map(|m| (a.id, m.gamma)));
        self.footage.log.extend(log);
    }

    /// The "make proxies?" offer for these sources, honouring the remembered answer
    /// (`Settings.footage_proxies`). Other checks that find slow footage (phone VFR, odd codecs) call this
    /// rather than drawing their own card. Sources already proxied or already waiting are skipped.
    pub(crate) fn offer_proxies(&mut self, ids: &[Id]) {
        let ids: Vec<Id> = ids
            .iter()
            .copied()
            .filter(|id| !self.footage.proxies.contains(id))
            .filter(|&id| self.project.asset(id).is_some_and(|a| a.proxy.is_none()))
            .collect();
        match self.settings.footage_proxies {
            _ if ids.is_empty() => {}
            1 => self.make_proxies(&ids),
            2 => {}
            _ => self.footage.proxies.extend(ids),
        }
    }

    fn make_proxies(&mut self, ids: &[Id]) {
        let before = self.project.to_json();
        for &id in ids {
            if let Some(a) = self.project.asset_mut(id) {
                a.proxy = Some(true);
            }
        }
        if !self.settings.use_proxies {
            self.settings.use_proxies = true;
            self.settings.save();
        }
        self.push_undo_labeled(before, "Make proxies");
        self.after_edit();
        self.toast(format!(
            "Making proxies for {} clip{} - see Jobs",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        ));
    }

    /// Each source's conversion LUT (installed into the library on first use) on its own effects, so every
    /// clip from it - now and later - is converted. One undo step.
    fn add_log_lut(&mut self, ids: &[(Id, &'static str)]) {
        use crate::engine::library::{self, Kind};
        let mut luts: Vec<(&'static str, String)> = Vec::new();
        for &(_, gamma) in ids {
            if luts.iter().any(|(g, _)| *g == gamma) {
                continue;
            }
            let Some((lib_id, cube)) = footage::lut_for(gamma) else { continue };
            let mut path = library::path_of(Kind::Lut, lib_id);
            if !path.exists() {
                match library::save(Kind::Lut, lib_id, &cube) {
                    Ok(item) => path = item.path,
                    Err(e) => return self.toast(format!("Can't add the LUT: {e}")),
                }
            }
            luts.push((gamma, path.to_string_lossy().into_owned()));
        }
        let before = self.project.to_json();
        for &(id, gamma) in ids {
            let Some((_, lut)) = luts.iter().find(|(g, _)| *g == gamma) else { continue };
            add_lut_first(&mut self.project, id, lut);
        }
        let label = match luts.as_slice() {
            [("S-Log3", _)] => "S-Log3 to Rec.709",
            [("S-Log2", _)] => "S-Log2 to Rec.709",
            [("HLG", _)] => "HLG to Rec.709",
            _ => "Convert to Rec.709",
        };
        self.push_undo_labeled(before, label);
        self.after_edit();
    }

    /// Colour Match each other camera's clips to the reference camera's first clip. One undo step.
    fn match_cameras(&mut self) {
        let cams = std::mem::take(&mut self.footage.cams);
        let Some((_, refs)) = cams.get(self.footage.reference) else { return };
        let Some(&rid) = refs.first() else { return };
        let Some(def) = crate::mcp::tools::find("color.match") else { return };
        let before = self.project.to_json();
        let mut failed = None;
        for (i, (_, ids)) in cams.iter().enumerate() {
            if i == self.footage.reference {
                continue;
            }
            for &id in ids {
                if let Err(e) = (def.run)(self, &json!({"clip_id": id, "reference_clip_id": rid})) {
                    failed = Some(e);
                }
            }
        }
        self.push_undo_labeled(before, "Match colours between cameras");
        self.after_edit();
        if let Some(e) = failed {
            self.toast(format!("Some clips could not be matched: {e}"));
        }
    }
}

/// Put the LUT at `lut` first on source `id`'s effects (the conversion comes before any grade), once.
pub(crate) fn add_lut_first(project: &mut Project, id: Id, lut: &str) {
    let Some(a) = project.asset_mut(id) else { return };
    if a.effects.iter().any(|e| e.kind == crate::model::EffectKind::Lut && e.lut == lut) {
        return;
    }
    let mut e = crate::model::Effect::new(crate::model::EffectKind::Lut);
    e.lut = lut.to_string();
    a.effects.insert(0, e);
}

/// The card's title for the waiting sources: "S-Log3 footage", "S-Log3 and HLG footage".
pub(crate) fn log_title(log: &[(Id, &'static str)]) -> String {
    let mut gammas: Vec<&str> = Vec::new();
    for &(_, g) in log {
        if !gammas.contains(&g) {
            gammas.push(g);
        }
    }
    format!("{} footage", gammas.join(" and "))
}

/// Group the timeline's video clips by camera.
pub(crate) fn cameras(project: &Project, camera_of: &dyn Fn(&str) -> Option<String>) -> Vec<(String, Vec<Id>)> {
    let mut out: Vec<(String, Vec<Id>)> = Vec::new();
    for t in &project.tracks {
        for c in t.clips.iter().filter(|c| c.kind == ClipKind::Video) {
            let Some(cam) = project.asset(c.asset).and_then(|a| camera_of(&a.path)).filter(|s| !s.is_empty()) else {
                continue;
            };
            match out.iter_mut().find(|(n, _)| *n == cam) {
                Some(e) => e.1.push(c.id),
                None => out.push((cam, vec![c.id])),
            }
        }
    }
    out
}

/// Every frame, from `project_match::window`: run the requests, and look for a second camera when the
/// timeline's video clips changed.
pub(super) fn poll(app: &mut App) {
    for r in inspector_footage::take_requests() {
        match r {
            FootageReq::Speed(id, s) => {
                let Some(rev) = app.project.clip(id).map(|c| c.reverse) else { continue };
                let before = app.project.to_json();
                let grew = inspector_footage::set_speed_fit(&mut app.project, &[id], s, rev);
                app.push_undo_labeled(before, "Slow motion (every frame)");
                app.after_edit();
                if !grew {
                    app.toast("No room after the clip: it kept its length - drag its end to show the rest");
                }
            }
            FootageReq::Flow(id) => {
                let speed = app.project.clip(id).map_or(1.0, |c| c.speed);
                match super::tools_export::start_bake(app, &[id], super::tools_export::BakeKind::Flow(speed)) {
                    Ok(_) => app.toast("Optical flow: rendering in the background - see Jobs"),
                    Err(e) => app.toast(e),
                }
            }
            FootageReq::Upscale(id, n) => {
                app.selection = vec![id];
                app.act(if n >= 4 { Action::AiUpscale4Real } else { Action::AiUpscale2Real });
            }
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for t in &app.project.tracks {
        for c in t.clips.iter().filter(|c| c.kind == ClipKind::Video) {
            (c.id, c.asset).hash(&mut h);
        }
    }
    let key = h.finish();
    if key == app.footage.scanned {
        return;
    }
    app.footage.scanned = key;
    let cams = cameras(&app.project, &|p| footage::of(p).map(|m| m.camera));
    if cams.len() < 2 {
        app.footage.cams.clear();
        return;
    }
    let mut names: Vec<&str> = cams.iter().map(|c| c.0.as_str()).collect();
    names.sort();
    let set = names.join("|");
    if app.footage.offered.insert(set) {
        app.footage.reference = 0;
        app.footage.cams = cams;
    } else if !app.footage.cams.is_empty() {
        app.footage.cams = cams; // the card is up: keep its clip lists current
    }
}

/// The cards, drawn above the Match card in its area.
pub(super) fn cards(app: &mut App, ui: &mut egui::Ui) {
    let mut act: Option<u8> = None;
    let s = &app.footage;
    if !s.proxies.is_empty() {
        let four_k = s.proxies.iter().filter_map(|&id| app.project.asset(id)).any(|a| a.width.min(a.height) >= 2160);
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let what = if four_k { "This 4K footage" } else { "This 10-bit HEVC footage" };
            ui.label(egui::RichText::new(format!("{what} may stutter — make proxies?")).strong());
            ui.horizontal(|ui| {
                if ui
                    .button("Make proxies")
                    .on_hover_text("Small edit copies for smooth playback; export still uses the original files. Remembered for later imports")
                    .clicked()
                {
                    act = Some(1);
                }
                if ui.button("No").on_hover_text("Remembered: heavy footage plays from the original files").clicked() {
                    act = Some(2);
                }
            });
        });
    }
    if !s.log.is_empty() {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new(log_title(&s.log)).strong()).on_hover_text(
                "Log and HLG footage looks flat or washed out until it is converted to normal (Rec.709) colour",
            );
            ui.horizontal(|ui| {
                if ui
                    .button("Add LUT")
                    .on_hover_text("Adds the matching conversion LUT (to Rec.709) from the library to these clips' sources, before any grade")
                    .clicked()
                {
                    act = Some(3);
                }
                if ui.button("Not now").clicked() {
                    act = Some(4);
                }
            });
        });
    }
    if !s.cams.is_empty() {
        let mut reference = s.reference;
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new(format!("Clips from {} cameras", s.cams.len())).strong()).on_hover_text(
                s.cams.iter().map(|(n, ids)| format!("{n}: {} clips", ids.len())).collect::<Vec<_>>().join("\n"),
            );
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("footage_ref_cam")
                    .selected_text(s.cams[reference.min(s.cams.len() - 1)].0.clone())
                    .show_ui(ui, |ui| {
                        for (i, (n, _)) in s.cams.iter().enumerate() {
                            ui.selectable_value(&mut reference, i, n);
                        }
                    })
                    .response
                    .on_hover_text("Reference camera: the others are matched to its look");
                if ui
                    .button("Match colours between cameras")
                    .on_hover_text("Colour Match: each clip from the other cameras is matched to a clip from the reference camera. One undo step")
                    .clicked()
                {
                    act = Some(5);
                }
                if ui.button("Not now").clicked() {
                    act = Some(6);
                }
            });
        });
        app.footage.reference = reference;
    }
    match act {
        Some(1) => {
            app.settings.footage_proxies = 1;
            app.settings.save();
            let ids = std::mem::take(&mut app.footage.proxies);
            app.make_proxies(&ids);
        }
        Some(2) => {
            app.settings.footage_proxies = 2;
            app.settings.save();
            app.footage.proxies.clear();
        }
        Some(3) => {
            let ids = std::mem::take(&mut app.footage.log);
            app.add_log_lut(&ids);
        }
        Some(4) => app.footage.log.clear(),
        Some(5) => app.match_cameras(),
        Some(6) => app.footage.cams.clear(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two cameras on the timeline group by the model their tags named.
    #[test]
    fn timeline_clips_group_by_camera() {
        let mut p = Project::new();
        let add = |p: &mut Project, path: &str| {
            let mut a = crate::engine::import::placeholder(path);
            (a.kind, a.width, a.height, a.fps, a.duration) = (ClipKind::Video, 1920, 1080, 25.0, 4.0);
            let aid = p.add_asset(a);
            let start = p.tracks[0].clips.len() as f64 * 4.0;
            let mut c = crate::model::Clip::new(p.new_id(), ClipKind::Video, path, start, 4.0);
            c.asset = aid;
            p.tracks[0].clips.push(c);
        };
        for path in ["C:/a7iv_1.mp4", "C:/a6400_1.mp4", "C:/a7iv_2.mp4", "C:/phone.mp4"] {
            add(&mut p, path);
        }
        let cam = |p: &str| match p {
            "C:/a7iv_1.mp4" | "C:/a7iv_2.mp4" => Some("Sony ILCE-7M4".to_string()),
            "C:/a6400_1.mp4" => Some("Sony ILCE-6400".to_string()),
            _ => None,
        };
        let g = cameras(&p, &cam);
        assert_eq!(
            g.iter().map(|(n, ids)| (n.as_str(), ids.len())).collect::<Vec<_>>(),
            [("Sony ILCE-7M4", 2), ("Sony ILCE-6400", 1)]
        );
    }

    /// The conversion LUT goes first on the source (before any grade) and only once.
    #[test]
    fn conversion_lut_goes_first_once() {
        let mut p = Project::new();
        let mut a = crate::engine::import::placeholder("C:/c0001.mp4");
        a.effects.push(crate::model::Effect::new(crate::model::EffectKind::Lut));
        a.effects[0].lut = "C:/grade.cube".into();
        let id = p.add_asset(a);
        add_lut_first(&mut p, id, "C:/slog3-to-rec709.cube");
        add_lut_first(&mut p, id, "C:/slog3-to-rec709.cube");
        let luts: Vec<&str> = p.asset(id).unwrap().effects.iter().map(|e| e.lut.as_str()).collect();
        assert_eq!(luts, ["C:/slog3-to-rec709.cube", "C:/grade.cube"]);
        assert_eq!(log_title(&[(1, "S-Log3"), (2, "S-Log3")]), "S-Log3 footage");
        assert_eq!(log_title(&[(1, "S-Log3"), (2, "HLG")]), "S-Log3 and HLG footage");
    }
}
