//! Timeline context menus: label/transition/clip right-click menus.
use super::*;

/// Colour submenu: the project's own labels (name + colour), not the built-in defaults.
pub(super) fn label_menu(ui: &mut egui::Ui, labels: &[Label], act: &mut Option<Act>, edit_labels: &mut bool) {
    if ui.button("None").clicked() {
        *act = Some(Act::Label(0));
    }
    for (i, l) in labels.iter().enumerate() {
        let color = Color32::from_rgb(l.color[0], l.color[1], l.color[2]);
        if ui.button(egui::RichText::new(l.name.clone()).color(color)).clicked() {
            *act = Some(Act::Label(i as u8 + 1));
        }
    }
    ui.separator();
    if ui.button("Apply label to asset").clicked() {
        *act = Some(Act::LabelToAsset);
    }
    if ui.button("Edit labels…").clicked() {
        *edit_labels = true;
    }
}

/// "Change Type" entries for the transition band's own right-click menu: absolute-overwrite every id
/// in `sel` to the clicked kind (works for a single selected transition or a bulk one — a menu pick
/// is the new value, there's nothing to diff against).
pub(super) fn transition_kind_menu(ui: &mut egui::Ui, sel: &[Id], act: &mut Option<Act>) {
    for k in TransitionKind::ALL {
        if ui.button(k.name()).clicked() {
            *act = Some(Act::SetTransitionsKind(sel.to_vec(), k));
        }
    }
}

/// "Change Easing" entries (see `transition_kind_menu`).
pub(super) fn transition_ease_menu(ui: &mut egui::Ui, sel: &[Id], act: &mut Option<Act>) {
    for e in Ease::ALL {
        if ui.button(e.name()).clicked() {
            *act = Some(Act::SetTransitionsEase(sel.to_vec(), e));
        }
    }
}

/// Effect kinds present on 2+ of the given clips (deduped per clip, so a clip carrying two Blurs only
/// counts once) — what the timeline's multi-clip right-click "Effects" quick-menu offers to toggle.
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

#[allow(clippy::too_many_arguments)]
pub(super) fn clip_menu(
    ui: &mut egui::Ui,
    clip_id: Id,
    is_container: bool,
    linked: bool,
    enabled: bool,
    audio: bool,
    has_native_size: bool,
    // ws:timeline-trim-gestures: a Sequence clip can be un-nested; the one Library-selected asset (if
    // exactly one) is what "Replace with Library Selection" swaps in.
    is_sequence: bool,
    library_selected: Option<Id>,
    // Some(currently open) when the clip has curves the inline mini graph could plot; None hides the
    // entry. Toggling is UI state, not a project edit, so it reports through `toggle_graph`, not `Act`
    // (every Act pushes an undo step).
    graph_open: Option<bool>,
    toggle_graph: &mut bool,
    labels: &[Label],
    buses: &[crate::model::Bus],
    shared_effects: &[EffectKind],
    act: &mut Option<Act>,
    actions: &mut Vec<crate::hotkeys::Action>,
    edit_labels: &mut bool,
) {
    use crate::hotkeys::Action;
    if ui.button("Copy").clicked() {
        actions.push(Action::CopyClips);
    }
    if ui.button("Cut").clicked() {
        actions.push(Action::CutClips);
    }
    if ui.button("Paste").clicked() {
        actions.push(Action::PasteClips);
    }
    ui.separator();
    if ui.button("Split at Playhead").clicked() {
        *act = Some(Act::Split);
    }
    if ui.button("Delete").clicked() {
        *act = Some(Act::Delete(false));
    }
    if ui.button("Ripple Delete").clicked() {
        *act = Some(Act::Delete(true));
    }
    ui.separator();
    if is_container {
        ui.menu_button("Container", |ui| {
            if ui.button("Replace Media…").clicked() {
                *act = Some(Act::ReplaceContainerMedia(clip_id));
                ui.close_menu();
            }
            if !audio && ui.button("Replace Pair…").clicked() {
                *act = Some(Act::ReplaceContainerPair(clip_id));
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Remove Container").clicked() {
                *act = Some(Act::UnmakeContainer);
                ui.close_menu();
            }
        });
    } else if ui.button("Convert to Container").clicked() {
        *act = Some(Act::MakeContainer);
    }
    ui.separator();
    if ui.button("Retime…").clicked() {
        actions.push(Action::Retime);
    }
    if ui.button("Freeze Frame at Playhead").clicked() {
        actions.push(Action::FreezeFrame);
    }
    // ponytail: one entry — Action::AddTransition always targets the cut on the selected clip's left.
    // "at End" pushed the same action, so it either duplicated this or toasted "no left neighbour".
    // The Transitions pane covers the right-hand cut (transitions_ui::right_neighbor).
    if ui.button("Add Transition at Start").clicked() {
        actions.push(Action::AddTransition);
    }

    if ui.button("Add Transition at End").clicked() {
        actions.push(Action::AddTransitionEnd);
    }
    if ui.button("Auto-cut…").clicked() {
        actions.push(Action::AutoCut);
    }
    ui.separator();
    if ui.button("Add Marker").clicked() {
        actions.push(Action::AddMarker);
    }
    if ui.button("Copy Attributes").clicked() {
        actions.push(Action::CopyAttributes);
    }
    if ui.button("Paste Attributes…").clicked() {
        actions.push(Action::PasteAttributes);
    }
    // a mask means nothing on audio; that clip wants its mixer routing instead
    if audio {
        ui.menu_button("Bus", |ui| {
            if ui.button("(track)").clicked() {
                *act = Some(Act::Bus(0));
            }
            for b in buses {
                if ui.button(&b.name).clicked() {
                    *act = Some(Act::Bus(b.id));
                }
            }
        });
    } else if ui.button("Add Mask").clicked() {
        actions.push(Action::AddMask);
    }
    if !shared_effects.is_empty() {
        ui.menu_button("Effects", |ui| {
            for k in shared_effects.iter().copied() {
                if ui.button(format!("Toggle {}", k.name())).clicked() {
                    *act = Some(Act::ToggleEffect(k));
                }
            }
        });
    }
    // video/image/sequence only — resets the transform of every selected clip with a native size
    // (Project::fit_clip_to_screen skips the rest, so this is safe on a mixed selection too).
    if has_native_size {
        ui.menu_button("Transform", |ui| {
            if ui.button("Stretch to Screen").clicked() {
                *act = Some(Act::StretchToScreen);
            }
            if ui.button("Fit to Screen").clicked() {
                *act = Some(Act::FitToScreen);
            }
        });
    }
    // second way into the inline mini keyframe graph, per the original ask ("if I zoom in on a clip OR
    // right click it") — the zoomed-in corner icon stays the first
    if let Some(open) = graph_open {
        if ui.button(if open { "Hide Inline Keyframe Graph" } else { "Show Inline Keyframe Graph" }).clicked() {
            *toggle_graph = true;
        }
    }
    ui.separator();
    if ui.button("Nest into Sequence…").clicked() {
        actions.push(Action::NestSequence);
    }
    // ---- ws:timeline-trim-gestures: trim-model verbs (Ctrl+J / Ctrl+D twins, plus two menu-only ones) ----
    if ui.add_enabled(is_sequence, egui::Button::new("Un-nest")).clicked() {
        *act = Some(Act::Unnest(clip_id));
    }
    if ui.button("Join Through Edit").clicked() {
        actions.push(Action::JoinThroughEdit);
    }
    if ui.button("Duplicate").clicked() {
        actions.push(Action::DuplicateClips);
    }
    if ui.add_enabled(library_selected.is_some(), egui::Button::new("Replace with Library Selection")).clicked() {
        *act = Some(Act::ReplaceClip(clip_id));
    }
    if ui.button("Convert to Adjustment Layer").clicked() {
        actions.push(Action::AddAdjustment);
    }
    if ui.button("Save as Template…").clicked() {
        actions.push(Action::SaveTemplate);
    }
    // ---- ws:transcript-captions ----
    // Speech → text for this clip (video/audio only): whisper as a background job, the words into
    // Project.transcripts. Every entry is an unbound Action handled by ui::app::transcript_ctl::act
    // on the selection — a right-click on an unselected clip selects it first (see the caller).
    if audio || has_native_size {
        ui.menu_button("Transcript", |ui| {
            if ui.button("Transcribe…").on_hover_text("whisper, in the background; downloads the model first if needed").clicked() {
                actions.push(Action::TranscribeClip);
                ui.close_menu();
            }
            if ui.button("View transcript").clicked() {
                actions.push(Action::ViewTranscript);
                ui.close_menu();
            }
            if ui.button("Export transcript…").on_hover_text(".txt, .srt or .json").clicked() {
                actions.push(Action::ExportTranscript);
                ui.close_menu();
            }
        });
    }
    ui.menu_button("Color Label", |ui| label_menu(ui, labels, act, edit_labels));
    ui.separator();
    if ui.button(if linked { "Unlink" } else { "Link" }).clicked() {
        *act = Some(Act::Link);
    }
    if ui.button(if enabled { "Disable" } else { "Enable" }).clicked() {
        *act = Some(Act::Enable(!enabled));
    }
}
