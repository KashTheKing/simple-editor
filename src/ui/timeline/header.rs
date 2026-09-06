//! Track-header cell drawing, extracted from `show()`'s row loop.
use super::*;

/// Draws one track's header cell (name, mute/solo toggles, add/remove-track context menu).
/// Returns a deferred `Act` when a toggle or menu item was clicked.
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
) -> Option<Act> {
    let mut act: Option<Act> = None;
    // header cell
    let hr = Rect::from_min_max(pos2(header.left(), row.top()), pos2(header.right(), row.bottom()));
    let tid = id.with(track.id);
    let hresp = ui.interact(hr, tid.with("hdr"), Sense::click());
    let bw = vec2(18.0, 16.0);
    let sb = Rect::from_center_size(pos2(hr.right() - 4.0 - bw.x * 0.5, hr.center().y), bw);
    let mb = sb.translate(vec2(-(bw.x + 3.0), 0.0));
    bp.with_clip_rect(Rect::from_min_max(hr.min, pos2(mb.left() - 2.0, hr.bottom()))).text(
        pos2(hr.left() + 6.0, hr.center().y),
        Align2::LEFT_CENTER,
        &track.name,
        font.clone(),
        if active { pal.text } else { pal.text_dim },
    );
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
    });
    act
}
