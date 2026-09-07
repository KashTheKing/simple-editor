//! Track-header cell drawing, extracted from `show()`'s row loop.
use super::*;
use crate::model::LABEL_COLORS;

/// Next colour in the header swatch's click-cycle: `None` -> `LABEL_COLORS[0]` -> … ->
/// `LABEL_COLORS[7]` -> `None`, the same palette Asset/Clip labels use.
fn next_swatch_color(cur: Option<[u8; 3]>) -> Option<[u8; 3]> {
    match cur {
        None => Some(LABEL_COLORS[0].1),
        Some(c) => match LABEL_COLORS.iter().position(|&(_, lc)| lc == c) {
            Some(i) if i + 1 < LABEL_COLORS.len() => Some(LABEL_COLORS[i + 1].1),
            _ => None,
        },
    }
}

/// Draws one track's header cell (drag-reorder grip, colour swatch, name/inline rename, lock/ripple/
/// mute/solo toggles, add/remove-track + Rename/Colour context menu). Returns a deferred `Act` when a
/// toggle or menu item was clicked. Lock / Ripple / Magnetic report through `track_toggle` instead
/// (ws:timeline-trim-gestures): they are undo-free track state, and every `Act` pushes one undo
/// unconditionally.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_header(
    ui: &mut egui::Ui,
    bp: &egui::Painter,
    header: Rect,
    row: Rect,
    id: egui::Id,
    pal: &Palette,
    font: &FontId,
    small: &FontId,
    track: &crate::model::Track,
    ti: usize,
    active: bool,
    track_toggle: &mut Option<(usize, TrackFlag)>,
    track_rename: &mut Option<(usize, String)>,
) -> Option<Act> {
    let mut act: Option<Act> = None;
    // header cell
    let hr = Rect::from_min_max(pos2(header.left(), row.top()), pos2(header.right(), row.bottom()));
    let tid = id.with(track.id);
    let hresp = ui.interact(hr, tid.with("hdr"), Sense::click());
    let bw = vec2(18.0, 16.0);
    let sb = Rect::from_center_size(pos2(hr.right() - 4.0 - bw.x * 0.5, hr.center().y), bw);
    let mb = sb.translate(vec2(-(bw.x + 3.0), 0.0));
    // ripple (chain link) and lock (padlock) sit left of the mute/solo pair
    let rb = mb.translate(vec2(-(bw.x + 3.0), 0.0));
    let lb = rb.translate(vec2(-(bw.x + 3.0), 0.0));

    // ---- ws:pro-timeline: drag-reorder grip + colour swatch, left of the name (previously-unused
    // strip per the plan's own risk note — bw/sb/mb/rb/lb already reserve the right side) ----
    let grip = Rect::from_min_max(pos2(hr.left(), hr.top() + 2.0), pos2(hr.left() + 9.0, hr.bottom() - 2.0));
    let gresp = ui.interact(grip, tid.with("grip"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
    for dy in [-3.0_f32, 0.0, 3.0] {
        bp.hline(
            Rangef::new(grip.left() + 1.0, grip.right() - 1.0),
            grip.center().y + dy,
            Stroke::new(1.0, pal.border),
        );
    }
    if gresp.drag_stopped() {
        if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
            let visual_up = if pos.y < row.top() {
                Some(true)
            } else if pos.y > row.bottom() {
                Some(false)
            } else {
                None
            };
            if let Some(visual_up) = visual_up {
                // Video tracks display in REVERSED index order (row_order shows the highest index on
                // top, so V2 sits above V1) — `Project::move_track`'s `up` walks `video_tracks()` in
                // ascending index order, the opposite of what's on screen. Audio tracks display in
                // plain ascending order, so their visual direction already matches `move_track`'s.
                let list_up = if track.kind == TrackKind::Video { !visual_up } else { visual_up };
                act = Some(Act::ReorderTrack(ti, list_up));
            }
        }
    }
    let swatch = Rect::from_center_size(pos2(grip.right() + 9.0, hr.center().y), vec2(12.0, 12.0));
    let sw_resp = ui.interact(swatch, tid.with("swatch"), Sense::click());
    let sw_fill = track.color.map(|[r, g, b]| Color32::from_rgb(r, g, b)).unwrap_or(pal.panel);
    bp.rect_filled(swatch, CornerRadius::same(2), sw_fill);
    bp.rect_stroke(
        swatch,
        CornerRadius::same(2),
        Stroke::new(1.0, if track.color.is_some() { pal.border } else { pal.text_dim }),
        StrokeKind::Inside,
    );
    if sw_resp.clicked() {
        act = Some(Act::SetTrackColor(ti, next_swatch_color(track.color)));
    }
    let name_x0 = swatch.right() + 6.0;

    if let Some((_, buf)) = track_rename.as_mut().filter(|(rti, _)| *rti == ti) {
        let name_rect = Rect::from_min_max(pos2(name_x0, hr.top() + 2.0), pos2(lb.left() - 2.0, hr.bottom() - 2.0));
        let te = ui.put(name_rect, egui::TextEdit::singleline(buf).font(font.clone()));
        te.request_focus();
        let commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if commit {
            act = Some(Act::RenameTrack(ti, buf.clone()));
        }
        if commit || te.clicked_elsewhere() {
            *track_rename = None;
        }
    } else {
        bp.with_clip_rect(Rect::from_min_max(pos2(name_x0, hr.top()), pos2(lb.left() - 2.0, hr.bottom()))).text(
            pos2(name_x0, hr.center().y),
            Align2::LEFT_CENTER,
            &track.name,
            font.clone(),
            if active { pal.text } else { pal.text_dim },
        );
        if hresp.double_clicked() {
            *track_rename = Some((ti, track.name.clone()));
        }
    }
    // audio: M = muted; video: V = visible (= !muted)
    let is_video = track.kind == TrackKind::Video;
    // real-world icons: an eye for video visibility, a speaker for audio mute
    let (m_label, m_on) = if is_video {
        (Cap::Icon(if track.muted { Glyph::EyeOff } else { Glyph::Eye }), !track.muted)
    } else {
        (Cap::Icon(if track.muted { Glyph::SpeakerOff } else { Glyph::SpeakerOn }), track.muted)
    };
    if toggle_button(ui, bp, mb, tid.with("m"), m_label, m_on, pal, small) {
        act = Some(Act::Mute(ti));
    }
    if toggle_button(ui, bp, sb, tid.with("s"), Cap::Text("S"), track.solo, pal, small) {
        act = Some(Act::Solo(ti));
    }
    let (locked, ripple, magnetic) = (track.locked, track.ripple.unwrap_or(false), track.magnetic);
    if toggle_button(ui, bp, lb, tid.with("lock"), Cap::Icon(Glyph::Lock), locked, pal, small) {
        *track_toggle = Some((ti, TrackFlag::Locked));
    }
    if toggle_button(ui, bp, rb, tid.with("ripple"), Cap::Icon(Glyph::Link), ripple, pal, small) {
        *track_toggle = Some((ti, TrackFlag::Ripple));
    }
    let (empty, muted, solo) = (track.clips.is_empty(), track.muted, track.solo);
    hresp.context_menu(|ui| {
        if ui.button("Add Video Track").clicked() {
            act = Some(Act::AddTrack(TrackKind::Video));
        }
        if ui.button("Add Audio Track").clicked() {
            act = Some(Act::AddTrack(TrackKind::Audio));
        }
        if ui.add_enabled(empty, egui::Button::new("Remove Track")).clicked() {
            act = Some(Act::RemoveTrack(ti));
        }
        ui.separator();
        if ui.button(if muted { "Unmute" } else { "Mute" }).clicked() {
            act = Some(Act::Mute(ti));
        }
        if ui.button(if solo { "Unsolo" } else { "Solo" }).clicked() {
            act = Some(Act::Solo(ti));
        }
        ui.separator();
        // the three trim-model flags as checkboxes (same undo-free path as the header glyphs)
        let (mut l, mut r, mut m) = (locked, ripple, magnetic);
        if ui.checkbox(&mut l, "Locked").changed() {
            *track_toggle = Some((ti, TrackFlag::Locked));
        }
        if ui.checkbox(&mut r, "Ripple (sync)").changed() {
            *track_toggle = Some((ti, TrackFlag::Ripple));
        }
        if ui
            .checkbox(&mut m, "Magnetic Track")
            .on_hover_text("Gapless: edge drags ripple and Delete closes the gap on this track")
            .changed()
        {
            *track_toggle = Some((ti, TrackFlag::Magnetic));
        }
        // ---- ws:pro-timeline ----
        ui.separator();
        if ui.button("Rename").clicked() {
            *track_rename = Some((ti, track.name.clone()));
        }
        ui.menu_button("Colour", |ui| {
            if ui.button("None").clicked() {
                act = Some(Act::SetTrackColor(ti, None));
            }
            for &(name, [r, g, b]) in LABEL_COLORS.iter() {
                if ui.button(egui::RichText::new(name).color(Color32::from_rgb(r, g, b))).clicked() {
                    act = Some(Act::SetTrackColor(ti, Some([r, g, b])));
                }
            }
        });
    });
    act
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_swatch_color_cycles_through_none_and_all_eight() {
        let mut cur = None;
        for &(_, color) in LABEL_COLORS.iter() {
            cur = next_swatch_color(cur);
            assert_eq!(cur, Some(color));
        }
        cur = next_swatch_color(cur);
        assert_eq!(cur, None, "wraps back to None after the last label colour");
    }
}
