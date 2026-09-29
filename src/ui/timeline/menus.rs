//! Timeline right-click menus. Every row goes through `ui::menu` (ws:timeline-surface), so labels,
//! icons and shortcut text match the menu bar: a row that IS an Action queues it (`acts`); a row that
//! needs something only the click knows (a clip id, a time, a track) keeps its `Act`, drawn with
//! `menu::item` / `menu::row` so it still looks and reads like the rest.
use super::*;
use crate::hotkeys::Action;
use crate::ui::menu;

/// Action rows, `None` = separator (the menu bar's own `acts`).
pub(super) fn acts(ui: &mut egui::Ui, items: &[Option<Action>]) {
    for &it in items {
        match it {
            Some(a) => {
                menu::action_item(ui, a);
            }
            None => {
                ui.separator();
            }
        }
    }
}

/// Paste ▸ - shared by every timeline menu. The app decides whether the clipboard actually holds
/// anything: a menu that hid itself when empty would just look broken.
pub(super) fn paste_menu(ui: &mut egui::Ui) {
    menu::sub(ui, Some(Glyph::Paste), "Paste", |ui| {
        acts(ui, &[Action::PasteClips, Action::PasteInsert, Action::PasteAtTop, Action::PasteInPlace].map(Some))
    });
}

/// A row with a colour dot in its icon gutter (clip labels, track colours).
pub(super) fn swatch_row(ui: &mut egui::Ui, name: &str, [r, g, b]: [u8; 3]) -> egui::Response {
    let resp = menu::row(ui, None, name, "");
    let c = pos2(resp.rect.left() + ui.spacing().button_padding.x + 9.0, resp.rect.center().y);
    ui.painter().circle_filled(c, 5.0, Color32::from_rgb(r, g, b));
    resp
}

/// Colour submenu: the project's own labels (name + colour), not the built-in defaults.
pub(super) fn label_menu(ui: &mut egui::Ui, labels: &[Label], act: &mut Option<Act>, edit_labels: &mut bool) {
    if menu::row(ui, None, "None", "").clicked() {
        *act = Some(Act::Label(0));
    }
    for (i, l) in labels.iter().enumerate() {
        if swatch_row(ui, &l.name, l.color).clicked() {
            *act = Some(Act::Label(i as u8 + 1));
        }
    }
    ui.separator();
    if menu::row(ui, None, "Apply Label to Asset", "").clicked() {
        *act = Some(Act::LabelToAsset);
    }
    if menu::row(ui, Some(Glyph::Pencil), "Edit Labels…", "").clicked() {
        *edit_labels = true;
    }
}

/// The transition band's menu (body and edges): remove, then bulk "Change Type" / "Change Easing"
/// over every id in `sel` - a menu pick is the new value, there's nothing to diff against.
pub(super) fn transition_menu(ui: &mut egui::Ui, tr: &crate::model::Transition, sel: &[Id], act: &mut Option<Act>) {
    let many = sel.len() > 1 && sel.contains(&tr.id);
    let label = if many { format!("Remove {} Transitions", sel.len()) } else { "Remove Transition".into() };
    if menu::row(ui, Some(Glyph::Cross), &label, &menu::shortcut(Action::Delete)).clicked() {
        *act = Some(if many { Act::RemoveTransitions(sel.to_vec()) } else { Act::RemoveTransition(tr.id) });
    }
    ui.separator();
    menu::sub(ui, Some(Glyph::Transition), "Change Type", |ui| {
        for k in TransitionKind::ALL {
            if menu::check(ui, k == tr.kind, k.name(), "").clicked() {
                *act = Some(Act::SetTransitionsKind(sel.to_vec(), k));
            }
        }
    });
    menu::sub(ui, Some(Glyph::CurveIcon), "Change Easing", |ui| {
        for e in Ease::ALL {
            if menu::check(ui, e == tr.ease, e.name(), "").clicked() {
                *act = Some(Act::SetTransitionsEase(sel.to_vec(), e));
            }
        }
    });
}

/// A keyframe diamond: Easing ▸ (the named eases, then the curve presets), Delete.
pub(super) fn key_menu(ui: &mut egui::Ui, clip: Id, t: f64, act: &mut Option<Act>) {
    menu::sub(ui, Some(Glyph::CurveIcon), "Easing", |ui| {
        for e in Ease::ALL {
            if menu::row(ui, None, e.name(), "").clicked() {
                *act = Some(Act::SetEase(clip, t, e));
            }
        }
        ui.separator();
        for (name, e) in Ease::PRESETS {
            if menu::row(ui, None, name, "").clicked() {
                *act = Some(Act::SetEase(clip, t, e));
            }
        }
    });
    if menu::row(ui, Some(Glyph::Cross), "Delete Keyframe", "").clicked() {
        *act = Some(Act::DelKeys(clip, t));
    }
}

/// Effect kinds present on 2+ of the given clips (deduped per clip, so a clip carrying two Blurs only
/// counts once) - what the timeline's multi-clip right-click "Effects" quick-menu offers to toggle.
pub(super) fn shared_effect_kinds(p: &Project, ids: &[Id]) -> Vec<EffectKind> {
    if ids.len() < 2 {
        return Vec::new();
    }
    let mut counts: Vec<(EffectKind, u32)> = Vec::new();
    for &id in ids {
        let Some(cl) = p.clip(id) else { continue };
        let mut seen: Vec<EffectKind> = Vec::new();
        for e in &cl.effects {
            if seen.contains(&e.kind) {
                continue;
            }
            seen.push(e.kind);
            match counts.iter_mut().find(|(k, _)| *k == e.kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((e.kind, 1)),
            }
        }
    }
    counts.into_iter().filter(|&(_, n)| n >= 2).map(|(k, _)| k).collect()
}

/// What the clip menu needs to know about the right-clicked clip.
pub(super) struct ClipMenu<'a> {
    pub id: Id,
    pub container: bool,
    /// On an audio track (by track, not kind: a nested sequence's audio twin is a Sequence clip).
    pub audio: bool,
    /// Video / image / sequence: something with a frame size.
    pub native_size: bool,
    pub sequence: bool,
    /// The one Library-selected asset, if exactly one: what "Replace with Library Selection" swaps in.
    pub library_selected: Option<Id>,
    /// Some(currently open) when the clip has curves the inline mini graph could plot.
    pub graph_open: Option<bool>,
    pub labels: &'a [Label],
    pub buses: &'a [crate::model::Bus],
    pub shared_effects: &'a [EffectKind],
}

/// The clip body / edge right-click menu. Actions run on the selection (a right-click on an unselected
/// clip selects it first - see the caller); `act` carries the clicked clip where the verb needs it.
/// Toggling the mini graph is UI state, not an edit, so it reports through `toggle_graph`.
pub(super) fn clip_menu(
    ui: &mut egui::Ui,
    m: &ClipMenu,
    act: &mut Option<Act>,
    toggle_graph: &mut bool,
    edit_labels: &mut bool,
) {
    use Action::*;
    menu::scroll(ui, |ui| {
        acts(ui, &[Some(CutClips), Some(CopyClips), Some(PasteClips)]);
        menu::sub(ui, None, "Attributes", |ui| acts(ui, &[Some(CopyAttributes), Some(PasteAttributes)]));
        acts(ui, &[Some(Delete), Some(RippleDelete), None]);
        acts(
            ui,
            &[Some(Split), Some(JoinThroughEdit), Some(DuplicateClips), Some(ToggleEnabled), Some(LinkToggle), None],
        );
        menu::sub(ui, Some(Glyph::Speed), "Speed", |ui| acts(ui, &[Some(Retime), Some(FreezeFrame)]));
        // video/image/sequence only - Project::fit_clip_to_screen skips the rest of a mixed selection
        if m.native_size {
            menu::sub(ui, None, "Transform", |ui| {
                if menu::row(ui, None, "Stretch to Screen", "").clicked() {
                    *act = Some(Act::StretchToScreen);
                }
                if menu::row(ui, None, "Fit to Screen", "").clicked() {
                    *act = Some(Act::FitToScreen);
                }
            });
        }
        if !m.shared_effects.is_empty() {
            menu::sub(ui, None, "Effects", |ui| {
                for k in m.shared_effects.iter().copied() {
                    if menu::row(ui, None, &format!("Toggle {}", k.name()), "").clicked() {
                        *act = Some(Act::ToggleEffect(k));
                    }
                }
            });
        }
        menu::sub(ui, None, "Add", |ui| {
            acts(ui, &[Some(AddMarker), Some(AddTransition), Some(AddTransitionEnd)]);
            // a mask means nothing on audio
            if !m.audio {
                acts(ui, &[Some(AddMask)]);
            }
        });
        if m.audio || m.native_size {
            menu::sub(ui, Some(Glyph::Waveform), "Audio", |ui| {
                if m.audio {
                    menu::sub(ui, None, "Bus", |ui| {
                        if menu::row(ui, None, "(track)", "").clicked() {
                            *act = Some(Act::Bus(0));
                        }
                        for b in m.buses {
                            if menu::row(ui, None, &b.name, "").clicked() {
                                *act = Some(Act::Bus(b.id));
                            }
                        }
                    });
                }
                acts(ui, &[Some(Normalize), Some(MatchLoudness), Some(AutoDuck), None]);
                acts(ui, &[Some(DetectBeats), Some(SplitAtBeats), None, Some(AutoCut)]);
            });
        }
        menu::sub(ui, Some(Glyph::Sequence), "Nest", |ui| {
            acts(ui, &[Some(NestSequence)]);
            if menu::item(ui, UnnestClip, m.sequence) {
                *act = Some(Act::Unnest(m.id));
            }
            if m.container {
                menu::sub(ui, Some(Glyph::Container), "Container", |ui| {
                    if menu::item(ui, ReplaceContainerMedia, true) {
                        *act = Some(Act::ReplaceContainerMedia(m.id));
                    }
                    if !m.audio && menu::row(ui, None, "Replace Pair…", "").clicked() {
                        *act = Some(Act::ReplaceContainerPair(m.id));
                    }
                    acts(ui, &[None, Some(UnmakeContainer)]);
                });
            } else {
                acts(ui, &[Some(MakeContainer)]);
            }
        });
        // speech → text for this clip (video/audio only), whisper in the background
        if m.audio || m.native_size {
            menu::sub(ui, Some(Glyph::Transcript), "Transcript", |ui| {
                acts(ui, &[Some(TranscribeClip), Some(ViewTranscript), Some(ExportTranscript)])
            });
        }
        menu::sub(ui, Some(Glyph::Swatch), "Label", |ui| label_menu(ui, m.labels, act, edit_labels));
        // second way into the inline keyframe graph (the zoomed-in corner icon is the first)
        if let Some(open) = m.graph_open {
            if menu::check(ui, open, "Inline Keyframe Graph", "").clicked() {
                *toggle_graph = true;
            }
        }
        acts(ui, &[None, Some(MatchFrame), Some(RevealInLibrary)]);
        if menu::item(ui, ReplaceWithLibrarySelection, m.library_selected.is_some()) {
            *act = Some(Act::ReplaceClip(m.id));
        }
        acts(ui, &[None, Some(SaveTemplate)]);
    });
}
