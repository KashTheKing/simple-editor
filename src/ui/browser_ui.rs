//! ---- ws:browser-layer ----
//! The Inspector's Browser and Mouse sections for a `ClipKind::Browser` clip. Edits go to the
//! inspector's per-frame clone of the clip through its shared `Gesture` (one undo per drag / text
//! entry); buttons that need the whole project or the viewer (Record mouse, Fit pages...) are
//! Actions handed back through `inspector::set_pending_action`.

use crate::hotkeys::Action;
use crate::model::{Animated, BrowserDisplay, BrowserStyle, BrowserTab, BrowserTheme, Cursor, CursorEvent, CursorEventKind, CursorShape};
use crate::theme::Palette;
use crate::ui::inspector::{fold, set_pending_action, Folds};
use crate::ui::list_rows::{Ctrl, Moved, RowSpec, Rows};
use crate::ui::{key_buttons, key_menu, menu, Gesture};
use eframe::egui::{self, DragValue, Grid};

/// Height of a list row.
const ROW: f32 = 20.0;

type Range = std::ops::RangeInclusive<f64>;

// The sections are tables of rows drawn by a few small functions (kept out of line: each widget call
// spelled out per row compiled to its own copy of the same code).

/// A number that may be keyframed: the value (a drag) with its context menu.
#[inline(never)]
fn anim_drag(ui: &mut egui::Ui, g: &mut Gesture, a: &mut Animated, lt: f64, range: Range, speed: f64, label: &str) {
    let mut v = a.at(lt);
    let r = ui.add_sized(
        [crate::ui::PROP_VAL_W, ui.spacing().interact_size.y],
        DragValue::new(&mut v).range(range).speed(speed),
    );
    if r.changed() {
        a.set_at(lt, v);
    }
    g.note(&r);
    menu::context(&r, |ui| key_menu(ui, a, lt, g, label, &[]));
}

/// A keyframable number row: label, the value and the ◆ button.
#[inline(never)]
fn animated(ui: &mut egui::Ui, g: &mut Gesture, palette: &Palette, lt: f64, label: &str, a: &mut Animated, range: Range, speed: f64) {
    crate::ui::prop_label(ui, label);
    ui.horizontal(|ui| {
        anim_drag(ui, g, a, lt, range, speed, label);
        key_buttons(ui, a, lt, palette, g, label, &[]);
    });
    ui.end_row();
}

/// What a button here asks the app to do with the whole project (`browser_ctl::run_request`).
pub(crate) enum Request {
    /// Zoom to this much, holding the fake mouse's position still.
    ZoomAtCursor(f64),
    /// Move the mouse to the element with this text.
    MoveTo(String),
    ClickGo { target: String, page: crate::model::Id, transition: String },
}

thread_local! {
    static REQUEST: std::cell::RefCell<Option<Request>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn take_request() -> Option<Request> {
    REQUEST.with(|r| r.borrow_mut().take())
}

fn ask(r: Request) {
    REQUEST.with(|slot| *slot.borrow_mut() = Some(r));
}

/// A button and what it says on hover, for the Actions the app runs.
type Btn = (&'static str, &'static str, Action);

/// A row of buttons, each handing its `Action` back through the inspector.
#[inline(never)]
fn act_buttons(ui: &mut egui::Ui, list: &[Btn]) {
    ui.horizontal_wrapped(|ui| {
        for (text, tip, a) in list {
            if ui.button(*text).on_hover_text(*tip).clicked() {
                set_pending_action(*a);
            }
        }
    });
}

#[inline(never)]
fn drag(ui: &mut egui::Ui, g: &mut Gesture, v: &mut f32, range: std::ops::RangeInclusive<f32>, speed: f64) {
    g.note(&ui.add(DragValue::new(v).range(range).speed(speed)));
}

#[inline(never)]
fn checks(ui: &mut egui::Ui, g: &mut Gesture, list: &mut [(&mut bool, &str, &str)]) {
    for (v, text, tip) in list {
        let r = ui.checkbox(v, *text);
        g.note(&if tip.is_empty() { r } else { r.on_hover_text(*tip) });
    }
}

#[inline(never)]
fn colours(ui: &mut egui::Ui, g: &mut Gesture, list: &mut [(&mut [u8; 4], &str)]) {
    for (c, tip) in list {
        g.note(&ui.color_edit_button_srgba_unmultiplied(c).on_hover_text(*tip));
    }
}

fn pick_image() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Image", &["png", "jpg", "jpeg", "webp", "bmp", "gif", "ico"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

/// A picture chosen from a file (right-click: none).
#[inline(never)]
fn picture(ui: &mut egui::Ui, g: &mut Gesture, path: &mut String, what: &str, tip: &str) {
    let r = ui.small_button(format!("{what}{}", if path.is_empty() { "…" } else { " ✓" })).on_hover_text(tip);
    if r.clicked() {
        if let Some(p) = pick_image() {
            *path = p;
            g.click();
        }
    }
    if r.secondary_clicked() {
        path.clear();
        g.click();
    }
}

#[inline(never)]
fn pick(ui: &mut egui::Ui, g: &mut Gesture, salt: &str, names: &[&str], cur: usize) -> usize {
    let mut now = cur;
    egui::ComboBox::from_id_salt(salt).selected_text(names[cur]).show_ui(ui, |ui| {
        for (i, n) in names.iter().enumerate() {
            ui.selectable_value(&mut now, i, *n);
        }
    });
    if now != cur {
        g.click();
    }
    now
}

/// A drop-down over every value of an enum.
fn pick_enum<T: Copy + PartialEq>(ui: &mut egui::Ui, g: &mut Gesture, salt: &str, all: &[T], name: fn(T) -> &'static str, cur: &mut T) {
    let names: Vec<&str> = all.iter().map(|&t| name(t)).collect();
    let i = all.iter().position(|t| t == cur).unwrap_or(0);
    *cur = all[pick(ui, g, salt, &names, i)];
}

/// Text kept between frames under `key` (what is typed beside a button, until it is pressed).
fn typed(ui: &mut egui::Ui, key: &str, width: f32, hint: &str) -> String {
    let id = ui.id().with(key);
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut text).desired_width(width).hint_text(hint));
    ui.data_mut(|d| d.insert_temp(id, text.clone()));
    text
}

const IMPORT_BUTTONS: [Btn; 5] = [
    ("Import web page…", "A real page by address or from a saved file, as a page under this browser", Action::ImportWebPage),
    ("Make editable", "Turn the headings and labels of an imported page into fields you can retype (on the page clip)", Action::PageMakeEditable),
    ("Sharpen zooms", "Draw the zoomed parts of an imported page again at exactly that zoom, so text stays crisp", Action::SharpenZooms),
    ("Fit pages", "Make the pictures below as wide as the window, top at the top, so a tall screenshot scrolls", Action::BrowserFitPages),
    ("Next page here", "Cut what is below at the playhead and run the load bar: the clip after the cut is the next page", Action::BrowserNextPage),
];

const MOUSE_BUTTONS: [Btn; 2] = [
    ("● Record mouse", "Plays the timeline and records your mouse over the viewer: position, clicks and the wheel. Only the part you record is replaced.", Action::RecordMouse),
    ("Place waypoints", "Click on the viewer to place the cursor at the playhead; each click moves the playhead on", Action::CursorWaypoints),
];

/// "At the playhead" buttons: what they add to the cursor's events.
const EVENT_BUTTONS: [(&str, CursorEventKind); 6] = [
    ("Click", CursorEventKind::Click),
    ("Double click", CursorEventKind::DoubleClick),
    ("Press", CursorEventKind::Down),
    ("Release", CursorEventKind::Up),
    ("Wheel ↓", CursorEventKind::Scroll(1.0)),
    ("Wheel ↑", CursorEventKind::Scroll(-1.0)),
];

const TAB_CTRLS: &[Ctrl] = &[
    Ctrl { glyph: "●", hint: "In front from the playhead on" },
    Ctrl { glyph: "▣", hint: "The tab's icon: choose a picture, or remove it" },
    Ctrl { glyph: "×", hint: "Remove this tab" },
];

/// The tabs as a table of the shared rows: [grip] [title, fills and truncates] [front] [icon] [remove],
/// reordered by dragging the grip or Alt+Up / Alt+Down on the selected row; the selected tab's title is
/// typed under the list.
#[inline(never)]
fn tabs(ui: &mut egui::Ui, g: &mut Gesture, b: &mut BrowserStyle, lt: f64) {
    ui.set_width(250.0);
    let (front, n) = (b.tab_at(lt), b.tabs.len());
    let sel_id = ui.make_persistent_id("browser_tabs").with("sel");
    let mut rows = Rows::new(ui, "browser_tabs", TAB_CTRLS, &[], false);
    let mut clicked = None;
    for (i, t) in b.tabs.iter().enumerate() {
        let label = if t.title.is_empty() { "(untitled)" } else { t.title.as_str() };
        let r = rows.row(ui, i, &RowSpec { key: i as u64 + 1, label, dim: i != front, ..Default::default() });
        clicked = r.ctrl.map(|k| (i, k)).or(clicked);
    }
    let moved = rows.finish(ui);
    match clicked {
        Some((i, 0)) => {
            crate::model::put_key(&mut b.active_tab, lt, i as f64, crate::model::Ease::Hold);
            g.click();
        }
        Some((i, 1)) => {
            if b.tabs[i].favicon.is_empty() {
                if let Some(p) = pick_image() {
                    b.tabs[i].favicon = p;
                }
            } else {
                b.tabs[i].favicon.clear();
            }
            g.click();
        }
        Some((i, _)) if n > 1 => {
            b.tabs.remove(i);
            g.click();
        }
        _ => {}
    }
    // a move, as the slot the tab ends up in
    let to = match moved {
        Some(Moved::Slot { from, slot, .. }) => Some((from, if slot > from { slot - 1 } else { slot })),
        Some(Moved::Step { from, delta }) => Some((from, (from as i32 + delta).clamp(0, n as i32 - 1) as usize)),
        _ => None,
    };
    if let Some((from, to)) = to.filter(|(f, t)| f != t && *f < n && *t < n) {
        let tab = b.tabs.remove(from);
        b.tabs.insert(to, tab);
        ui.data_mut(|d| d.insert_temp(sel_id, to as u64 + 1));
        g.click();
    }
    let sel = ui.data(|d| d.get_temp::<u64>(sel_id)).map_or(front, |k| k as usize - 1).min(b.tabs.len() - 1);
    ui.horizontal(|ui| {
        g.note_text(&ui.add(egui::TextEdit::singleline(&mut b.tabs[sel].title).desired_width(190.0).hint_text("Title of the selected tab")));
        if ui.small_button("+ Tab").clicked() {
            b.tabs.push(BrowserTab { title: "New Tab".into(), favicon: String::new() });
            g.click();
        }
    });
}

/// The cursor's events as rows: [time, fixed] [what, fills and truncates] [remove, fixed on the right].
// ponytail: the first 60 only - a long recording is edited by re-recording a part
#[inline(never)]
fn events(ui: &mut egui::Ui, g: &mut Gesture, c: &mut Cursor) {
    let mut remove = None;
    for (i, e) in c.events.iter().enumerate().take(60) {
        let what = match &e.kind {
            CursorEventKind::Key(k) => format!("Keys {k}"),
            CursorEventKind::Shape(s) => format!("→ {}", s.name()),
            CursorEventKind::Scroll(d) => EVENT_BUTTONS[if *d < 0.0 { 5 } else { 4 }].0.to_string(),
            k => EVENT_BUTTONS.iter().find(|(_, b)| b == k).map_or("", |b| b.0).to_string(),
        };
        ui.allocate_ui(egui::vec2(250.0, ROW), |ui| {
            ui.horizontal(|ui| {
                ui.push_id(("browser_event", i), |ui| {
                    ui.add_sized([52.0, ROW], egui::Label::new(egui::RichText::new(format!("{:.2} s", e.t)).weak()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Remove this event").clicked() {
                            remove = Some(i);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(what).truncate());
                        });
                    });
                });
            });
        });
    }
    if let Some(i) = remove {
        c.events.remove(i);
        g.click();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn section(
    ui: &mut egui::Ui,
    b: &mut BrowserStyle,
    lt: f64,
    fonts: &[String],
    // library pictures (id, name): what "Click and go to page" can go to
    pages: &[(crate::model::Id, String)],
    palette: &Palette,
    folds: &mut Folds,
    g: &mut Gesture,
) {
    fold(ui, folds, "browser", "Browser", None, |ui| {
        let window = b.display == BrowserDisplay::Window;
        Grid::new("inspector_browser").num_columns(2).show(ui, |ui| {
            ui.label("Display");
            pick_enum(ui, g, "browser_display", &BrowserDisplay::ALL, BrowserDisplay::name, &mut b.display);
            ui.end_row();
            ui.label("Look");
            pick_enum(ui, g, "browser_theme", &BrowserTheme::ALL, BrowserTheme::name, &mut b.theme);
            ui.end_row();
            ui.label("Address");
            ui.horizontal(|ui| {
                g.note_text(&ui.add(egui::TextEdit::singleline(&mut b.url).desired_width(170.0)));
                if ui.button("Type it").on_hover_text("The address types itself out from the playhead").clicked() {
                    b.type_url(lt, (b.url.chars().count() as f64 * 0.055).clamp(0.4, 4.0));
                    g.click();
                }
            });
            ui.end_row();
            animated(ui, g, palette, lt, "Typed", &mut b.url_typed, 0.0..=1.0, 0.01);
            ui.label("Tabs");
            ui.vertical(|ui| tabs(ui, g, b, lt));
            ui.end_row();
            animated(ui, g, palette, lt, "Scroll", &mut b.scroll, 0.0..=1.0e6, 2.0);
            animated(ui, g, palette, lt, "Window zoom", &mut b.wzoom, 0.05..=64.0, 0.01);
            ui.label("Zoom at").on_hover_text("The point the Window zoom holds still: X, then Y, from the middle of the frame");
            ui.horizontal(|ui| {
                for (a, name) in [(&mut b.wzoom_x, "Window zoom X"), (&mut b.wzoom_y, "Window zoom Y")] {
                    anim_drag(ui, g, a, lt, -1.0e9..=1.0e9, 1.0, name);
                }
                if ui.small_button("In ×2").on_hover_text("The whole browser zooms in to where the mouse cursor is, from the playhead").clicked() {
                    ask(Request::ZoomAtCursor((b.wzoom.at(lt) * 2.0).min(64.0)));
                }
                if ui.small_button("Out").on_hover_text("Zoom back out from the playhead").clicked() {
                    ask(Request::ZoomAtCursor(1.0));
                }
            });
            ui.end_row();
            animated(ui, g, palette, lt, "Page zoom", &mut b.zoom, 0.05..=64.0, 0.01);
            crate::ui::prop_label(ui, "Load bar");
            ui.horizontal(|ui| {
                anim_drag(ui, g, &mut b.load, lt, 0.0..=1.0, 0.01, "Load bar");
                key_buttons(ui, &mut b.load, lt, palette, g, "Load bar", &[]);
                if ui.button("Run").on_hover_text("The load bar runs from the playhead").clicked() {
                    b.run_load(lt, 0.6);
                    g.click();
                }
            });
            ui.end_row();
            if !window {
                return; // Page only: there is no window to size, dress or put a background around
            }
            ui.label("Size");
            ui.horizontal(|ui| {
                drag(ui, g, &mut b.w, 120.0..=16000.0, 1.0);
                ui.weak("×");
                drag(ui, g, &mut b.h, 120.0..=16000.0, 1.0);
            });
            ui.end_row();
            for (label, v, max, speed) in [("Corners", &mut b.corner, 200.0, 0.2), ("Shadow", &mut b.shadow, 400.0, 0.5)] {
                ui.label(label);
                drag(ui, g, v, 0.0..=max, speed);
                ui.end_row();
            }
            ui.label("Show");
            ui.horizontal_wrapped(|ui| {
                let list = &mut [(&mut b.show_tabs, "Tabs", ""), (&mut b.show_toolbar, "Address bar", ""), (&mut b.show_controls, "Window dots", ""), (&mut b.show_bookmarks, "Bookmarks", "")];
                checks(ui, g, list);
            });
            ui.end_row();
            if b.show_bookmarks {
                ui.label("Bookmarks");
                let mut text = b.bookmarks.join(", ");
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.0).hint_text("Home, News, Docs"));
                if r.changed() {
                    b.bookmarks = text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
                g.note_text(&r);
                ui.end_row();
            }
            ui.label("Font");
            let names: Vec<&str> = std::iter::once("Default").chain(fonts.iter().map(|f| f.as_str())).collect();
            let cur = fonts.iter().position(|f| *f == b.font).map_or(0, |i| i + 1);
            match pick(ui, g, "browser_font", &names, cur) {
                i if i == cur => {}
                0 => b.font.clear(),
                i => b.font = names[i].to_string(),
            }
            ui.end_row();
            ui.label("Around it");
            ui.horizontal(|ui| {
                colours(ui, g, &mut [(&mut b.bg, "Colour around the window"), (&mut b.bg2, "Bottom colour (same as the first = flat)")]);
                picture(ui, g, &mut b.bg_image, "Picture", "A picture behind the window (right-click: none)");
            });
            ui.end_row();
        });
        g.note(&ui.checkbox(&mut b.auto_fit, "Fit pages automatically").on_hover_text(
            "A picture that becomes a page under this browser (a next page, an imported page) is made as wide as the window",
        ));
        act_buttons(ui, &IMPORT_BUTTONS);
        if window {
            ui.weak("Move, scale and rotate the window in Transform or on the viewer.");
        }
    });

    fold(ui, folds, "browser_mouse", "Mouse", None, |ui| {
        let mut on = b.cursor.as_ref().is_some_and(|c| c.visible);
        if ui.checkbox(&mut on, "Show a mouse cursor").changed() {
            b.cursor.get_or_insert_with(Cursor::default).visible = on;
            g.click();
        }
        act_buttons(ui, &MOUSE_BUTTONS);
        let Some(c) = b.cursor.as_mut().filter(|c| c.visible) else { return };
        Grid::new("inspector_browser_mouse").num_columns(2).show(ui, |ui| {
            animated(ui, g, palette, lt, "X", &mut c.x, -1.0e5..=1.0e5, 1.0);
            animated(ui, g, palette, lt, "Y", &mut c.y, -1.0e5..=1.0e5, 1.0);
            ui.label("Pointer");
            ui.horizontal(|ui| {
                pick_enum(ui, g, "browser_cursor_shape", &CursorShape::ALL, CursorShape::name, &mut c.shape);
                g.note(&ui.add(DragValue::new(&mut c.size).range(0.2..=8.0).speed(0.02).prefix("× ")));
            });
            ui.end_row();
            ui.label("Colours");
            ui.horizontal(|ui| colours(ui, g, &mut [(&mut c.color, "Pointer"), (&mut c.outline, "Outline"), (&mut c.ripple, "Click ripple (transparent = none)")]));
            ui.end_row();
            ui.label("Path");
            ui.horizontal(|ui| {
                checks(ui, g, &mut [(&mut c.smooth, "Curved", "Curve through the keys instead of straight lines")]);
                ui.weak(format!("{} keys", c.x.keys.len()));
            });
            ui.end_row();
            ui.label("Smoothing");
            ui.horizontal(|ui| {
                g.note(
                    &ui.add(egui::Slider::new(&mut c.smoothing, 0.0..=1.0).show_value(false))
                        .on_hover_text("How much a recording is simplified: left keeps every wiggle, right only the big moves"),
                );
                if ui.button("Tidy path").on_hover_text("Remove jitter: keep only the keys the path needs").clicked() {
                    // the same reduction the recorder runs, over the keys already there
                    let times: Vec<f64> = c.x.keys.iter().map(|k| k.t).collect();
                    let pts: Vec<(f64, f64, f64)> = times.iter().map(|&t| (c.x.at(t), c.y.at(t), t)).collect();
                    let keep = crate::model::simplify_path(&pts, 1.0 + c.smoothing as f64 * 23.0);
                    let stays = |t: f64| keep.iter().any(|&i| (times[i] - t).abs() < 1e-4) || c.events.iter().any(|e| (e.t - t).abs() < 1e-3);
                    let drop: Vec<f64> = times.iter().copied().filter(|&t| !stays(t)).collect();
                    for a in [&mut c.x, &mut c.y] {
                        a.keys.retain(|k| !drop.iter().any(|t| (k.t - t).abs() < 1e-4));
                    }
                    g.click();
                }
            });
            ui.end_row();
        });
        ui.label("At the playhead:");
        ui.horizontal_wrapped(|ui| {
            let shapes = CursorShape::ALL.map(|s| (format!("→ {}", s.name()), CursorEventKind::Shape(s)));
            let all = EVENT_BUTTONS.iter().map(|(t, k)| (t.to_string(), k.clone())).chain(shapes);
            for (text, kind) in all {
                if ui.small_button(text).clicked() {
                    c.events.push(CursorEvent { t: lt, kind });
                    c.sort();
                    g.click();
                }
            }
        });
        ui.horizontal(|ui| {
            let text = typed(ui, "browser_keys_text", 110.0, "Ctrl+L");
            if ui.small_button("Show keys").on_hover_text("Shows these keys in an overlay from the playhead").clicked() && !text.trim().is_empty() {
                c.events.push(CursorEvent { t: lt, kind: CursorEventKind::Key(text.trim().to_string()) });
                c.sort();
                g.click();
            }
            checks(ui, g, &mut [(&mut c.keys, "Overlay", "")]);
        });
        // an imported page knows its links and buttons by name
        ui.horizontal(|ui| {
            let target = typed(ui, "browser_target", 110.0, "Play");
            let target = target.trim().to_string();
            let named = !target.is_empty();
            if ui.add_enabled(named, egui::Button::new("Go to it").small()).on_hover_text("The mouse moves there, arriving at the playhead").clicked() {
                ask(Request::MoveTo(target.clone()));
            }
            let pid = ui.id().with("browser_next_page");
            let mut page = ui.data_mut(|d| d.get_temp::<crate::model::Id>(pid).unwrap_or(0));
            let name = pages.iter().find(|p| p.0 == page).map_or("Next page…", |p| p.1.as_str()).to_string();
            egui::ComboBox::from_id_salt("browser_next_page").selected_text(name).width(90.0).show_ui(ui, |ui| {
                for (id, name) in pages {
                    ui.selectable_value(&mut page, *id, name);
                }
            });
            let hint = "One step: the mouse goes there, clicks at the playhead, and the page changes to the chosen one with a load bar";
            if ui.add_enabled(named && page != 0, egui::Button::new("Click and go").small()).on_hover_text(hint).clicked() {
                ask(Request::ClickGo { target, page, transition: "fade".into() });
            }
            ui.data_mut(|d| d.insert_temp(pid, page));
        });
        act_buttons(ui, &[("Press what is under it", "The clip under the cursor at the playhead dips and springs back, like a pressed button", Action::CursorPress)]);
        if !c.events.is_empty() {
            ui.collapsing(format!("{} events", c.events.len()), |ui| events(ui, g, c));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sections ask for no repaint of their own: drawn over several idle frames they request none.
    #[test]
    fn assert_no_idle_repaint_browser_sections() {
        let ctx = egui::Context::default();
        let mut b = BrowserStyle::default();
        b.cursor.as_mut().unwrap().events.push(CursorEvent { t: 1.0, kind: CursorEventKind::Click });
        let mut map = std::collections::BTreeMap::new();
        map.insert("browser".to_string(), true);
        map.insert("browser_mouse".to_string(), true);
        let before = b.clone();
        for frame in 0..6 {
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut folds = Folds { primary: "browser", map: &mut map };
                    let mut g = Gesture::default();
                    let pages = [(7, "page.png".to_string())];
                    section(ui, &mut b, 0.5, &[], &pages, &Palette::new(true, egui::Color32::LIGHT_BLUE), &mut folds, &mut g);
                    assert!(!g.start && !g.changed, "drawing edits nothing");
                });
            });
            if frame >= 3 {
                let delay = out.viewport_output.values().map(|v| v.repaint_delay).min().unwrap_or(std::time::Duration::MAX);
                assert!(delay > std::time::Duration::from_secs(1), "idle browser sections asked for a repaint: {delay:?}");
            }
        }
        assert_eq!(b, before);
        assert!(take_request().is_none(), "nothing is asked for without a click");
    }
}
