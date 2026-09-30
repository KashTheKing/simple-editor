//! Track-header cell drawing, extracted from `show()`'s row loop.
use super::menus::swatch_row;
use super::*;
use crate::hotkeys::Action;
use crate::model::LABEL_COLORS;
use crate::ui::menu;
use crate::ui::tools::Dir;

/// Draws one track's header cell: a colour stripe down the left edge (also the drag-to-reorder grip),
/// the name (double-click renames), lock and eye / speaker; Solo appears on hover or while it is on.
/// Everything else - add/remove track, ripple, magnetic, rename, colour, move - is in the right-click
/// menu (ws:timeline-surface). Returns a deferred `Act` when a toggle or menu item was clicked. Lock /
/// Ripple / Magnetic report through `track_toggle` instead (ws:timeline-trim-gestures): they are
/// undo-free track state, and every `Act` pushes one undo unconditionally.
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
    let hr = Rect::from_min_max(pos2(header.left(), row.top()), pos2(header.right(), row.bottom()));
    let tid = id.with(track.id);
    let hresp = ui.interact(hr, tid.with("hdr"), Sense::click());
    let is_video = track.kind == TrackKind::Video;
    // right to left: eye / speaker, lock, then solo
    let bw = vec2(18.0, 16.0);
    let mb = Rect::from_center_size(pos2(hr.right() - 4.0 - bw.x * 0.5, hr.center().y), bw);
    let lb = mb.translate(vec2(-(bw.x + 3.0), 0.0));
    let sb = lb.translate(vec2(-(bw.x + 3.0), 0.0));
    let show_solo = track.solo || ui.rect_contains_pointer(hr);

    // the track colour is a stripe down the left edge, which doubles as the drag-to-reorder grip
    let grip = Rect::from_min_max(pos2(hr.left(), hr.top() + 1.0), pos2(hr.left() + 6.0, row.bottom() - HANDLE_H));
    let gresp = ui.interact(grip, tid.with("grip"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
    if let Some([r, g, b]) = track.color {
        bp.rect_filled(
            Rect::from_min_max(grip.min, pos2(grip.left() + 3.0, grip.bottom())),
            0,
            Color32::from_rgb(r, g, b),
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
            if let Some(up) = visual_up {
                act = Some(Act::ReorderTrack(ti, list_up(is_video, up)));
            }
        }
    }

    let name_x0 = hr.left() + 10.0;
    let name_x1 = if show_solo { sb.left() } else { lb.left() } - 2.0;
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
        bp.with_clip_rect(Rect::from_min_max(pos2(name_x0, hr.top()), pos2(name_x1, hr.bottom()))).text(
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
    // video: an eye (lit = hidden); audio: a speaker (lit = muted)
    let (glyph, tip) = match (is_video, track.muted) {
        (true, false) => (Glyph::Eye, "Hide track"),
        (true, true) => (Glyph::EyeOff, "Show track"),
        (false, false) => (Glyph::SpeakerOn, "Mute"),
        (false, true) => (Glyph::SpeakerOff, "Unmute"),
    };
    if toggle_button(ui, bp, mb, tid.with("m"), Cap::Icon(glyph), track.muted, pal, small).on_hover_text(tip).clicked()
    {
        act = Some(Act::Mute(ti));
    }
    let lock = toggle_button(ui, bp, lb, tid.with("lock"), Cap::Icon(Glyph::Lock), track.locked, pal, small);
    if lock.on_hover_text(if track.locked { "Unlock track" } else { "Lock track" }).clicked() {
        *track_toggle = Some((ti, TrackFlag::Locked));
    }
    if show_solo
        && toggle_button(ui, bp, sb, tid.with("s"), Cap::Text("S"), track.solo, pal, small)
            .on_hover_text("Solo")
            .clicked()
    {
        act = Some(Act::Solo(ti));
    }

    let (locked, ripple, magnetic) = (track.locked, track.ripple.unwrap_or(false), track.magnetic);
    let (empty, muted, solo) = (track.clips.is_empty(), track.muted, track.solo);
    menu::context(&hresp, |ui| {
        super::menus::acts(ui, &[Some(Action::AddVideoTrack), Some(Action::AddAudioTrack)]);
        let rm = ui.add_enabled_ui(empty, |ui| menu::row(ui, Some(Glyph::Cross), "Remove Track", "")).inner;
        if rm.on_disabled_hover_text("Only an empty track can be removed").clicked() {
            act = Some(Act::RemoveTrack(ti));
        }
        ui.separator();
        if menu::check(ui, muted, if is_video { "Hidden" } else { "Muted" }, "").clicked() {
            act = Some(Act::Mute(ti));
        }
        if menu::check(ui, solo, "Solo", "").clicked() {
            act = Some(Act::Solo(ti));
        }
        ui.separator();
        // the three trim-model flags (same undo-free path as the header's lock)
        if menu::check(ui, locked, "Locked", &menu::shortcut(Action::ToggleTrackLock)).clicked() {
            *track_toggle = Some((ti, TrackFlag::Locked));
        }
        if menu::check(ui, ripple, "Ripple (Sync)", &menu::shortcut(Action::ToggleTrackRipple)).clicked() {
            *track_toggle = Some((ti, TrackFlag::Ripple));
        }
        if menu::check(ui, magnetic, "Magnetic Track", &menu::shortcut(Action::ToggleTrackMagnetic))
            .on_hover_text("Gapless: edge drags ripple and Delete closes the gap on this track")
            .clicked()
        {
            *track_toggle = Some((ti, TrackFlag::Magnetic));
        }
        ui.separator();
        if menu::row(ui, Some(Glyph::Pencil), "Rename…", &menu::shortcut(Action::RenameTrack)).clicked() {
            *track_rename = Some((ti, track.name.clone()));
        }
        menu::sub(ui, Some(Glyph::Swatch), "Colour", |ui| {
            if menu::row(ui, None, "None", "").clicked() {
                act = Some(Act::SetTrackColor(ti, None));
            }
            for &(name, color) in LABEL_COLORS.iter() {
                if swatch_row(ui, name, color).clicked() {
                    act = Some(Act::SetTrackColor(ti, Some(color)));
                }
            }
        });
        ui.separator();
        if menu::row(ui, Some(Glyph::Tri(Dir::Up)), "Move Track Up", "").clicked() {
            act = Some(Act::ReorderTrack(ti, list_up(is_video, true)));
        }
        if menu::row(ui, Some(Glyph::Tri(Dir::Down)), "Move Track Down", "").clicked() {
            act = Some(Act::ReorderTrack(ti, list_up(is_video, false)));
        }
    });
    act
}

/// `Project::move_track`'s `up` for an on-screen direction: video tracks display in REVERSED index
/// order (V2 sits above V1) while `move_track` walks `video_tracks()` ascending; audio already matches.
fn list_up(is_video: bool, visual_up: bool) -> bool {
    if is_video {
        !visual_up
    } else {
        visual_up
    }
}
