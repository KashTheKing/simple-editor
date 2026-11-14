//! ---- ws:browser-layer ----
//! A browser window drawn by the editor: `ClipKind::Browser` shows everything on the tracks below it
//! (for as long as the clip lasts) inside a window with tabs, an address bar and a fake mouse cursor.
//! The window is placed by the clip's own Position / Scale / Rotation; everything else lives here.
//! Rendering: `engine::browser`.

use crate::model::*;
use serde::{Deserialize, Serialize};

/// Looks are named by what they are, not after a product; the art is generic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum BrowserTheme {
    #[default]
    Light,
    Dark,
    /// One slim bar: window dots and the address, no tab strip.
    Minimal,
    /// A phone-shaped window: address on top, a home bar below.
    Mobile,
}

impl BrowserTheme {
    pub const ALL: [BrowserTheme; 4] =
        [BrowserTheme::Light, BrowserTheme::Dark, BrowserTheme::Minimal, BrowserTheme::Mobile];
    pub fn name(self) -> &'static str {
        match self {
            BrowserTheme::Light => "Light",
            BrowserTheme::Dark => "Dark",
            BrowserTheme::Minimal => "Minimal",
            BrowserTheme::Mobile => "Mobile",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

/// What the layer draws around the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum BrowserDisplay {
    /// A browser window: tabs, address bar, frame, shadow, background.
    #[default]
    Window,
    /// The page and nothing else, filling the frame edge to edge (a full-screen viewer). Scroll,
    /// zoom, the mouse and page changes work as in a window.
    Page,
}

impl BrowserDisplay {
    pub const ALL: [BrowserDisplay; 2] = [BrowserDisplay::Window, BrowserDisplay::Page];
    pub fn name(self) -> &'static str {
        match self {
            BrowserDisplay::Window => "Browser window",
            BrowserDisplay::Page => "Page only",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "window" | "browser" | "browserwindow" => BrowserDisplay::Window,
            "page" | "pageonly" | "viewer" | "fullscreen" => BrowserDisplay::Page,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BrowserTab {
    pub title: String,
    /// Optional image file shown before the title (any still the app can open).
    pub favicon: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum CursorShape {
    #[default]
    Arrow,
    Hand,
    /// The I-beam shown over text.
    Text,
}

impl CursorShape {
    pub const ALL: [CursorShape; 3] = [CursorShape::Arrow, CursorShape::Hand, CursorShape::Text];
    pub fn name(self) -> &'static str {
        match self {
            CursorShape::Arrow => "Arrow",
            CursorShape::Hand => "Hand",
            CursorShape::Text => "Text",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        let s = if s.eq_ignore_ascii_case("ibeam") { "Text" } else { s };
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CursorEventKind {
    Click,
    DoubleClick,
    /// Button held from here (a drag) until the next `Up`.
    Down,
    Up,
    /// Wheel tick; the number is its direction (+ = down), drawn as a small indicator.
    Scroll(f32),
    /// Keys shown in the keystroke overlay ("Ctrl+C", "hello").
    Key(String),
    /// The pointer changes shape from here on (hovering a link: Hand).
    Shape(CursorShape),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorEvent {
    /// Clip-local seconds.
    pub t: f64,
    pub kind: CursorEventKind,
}

/// The fake mouse. Position is in project px relative to the centre of the page (the picture below
/// the browser at zoom 1), so it stays put on the page while the window moves.
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub visible: bool,
    pub x: Animated,
    pub y: Animated,
    /// The shape before any `Shape` event.
    pub shape: CursorShape,
    /// 1 = a 24 px pointer.
    pub size: f32,
    pub color: [u8; 4],
    pub outline: [u8; 4],
    /// Click ripple colour (alpha 0 = no ripples).
    pub ripple: [u8; 4],
    /// Show `Key` events as an overlay at the bottom of the page.
    pub keys: bool,
    /// Curve through the keys instead of straight lines between them.
    pub smooth: bool,
    /// How much a recording is simplified, 0..1 (Record mouse and Tidy path use it).
    pub smoothing: f32,
    pub events: Vec<CursorEvent>,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            visible: true,
            x: a0(),
            y: a0(),
            shape: CursorShape::Arrow,
            size: 1.0,
            color: [255, 255, 255, 255],
            outline: [20, 20, 24, 255],
            ripple: [80, 150, 255, 200],
            keys: true,
            smooth: true,
            smoothing: 0.35,
            events: Vec::new(),
        }
    }
}

/// Seconds a click ripple / scroll indicator stays on screen.
pub const RIPPLE_SECS: f64 = 0.45;
/// Seconds a keystroke stays in the overlay.
pub const KEY_SECS: f64 = 1.2;

impl Cursor {
    /// Position at clip-local `t`. With `smooth`, Linear segments bend through their neighbours
    /// (a Catmull-Rom curve, i.e. a cubic Bezier per segment); an eased segment stays a straight,
    /// eased move so a waypoint the pointer should stop on is really stopped on.
    pub fn pos(&self, t: f64) -> (f64, f64) {
        if self.smooth {
            (spline_at(&self.x, t), spline_at(&self.y, t))
        } else {
            (self.x.at(t), self.y.at(t))
        }
    }
    pub fn shape_at(&self, t: f64) -> CursorShape {
        let mut s = self.shape;
        for e in self.events.iter().filter(|e| e.t <= t + 1e-9) {
            if let CursorEventKind::Shape(k) = e.kind {
                s = k;
            }
        }
        s
    }
    /// Is the button down at `t` (inside a Down..Up pair, or the first moments of a click)?
    pub fn pressed(&self, t: f64) -> bool {
        let mut down = false;
        for e in self.events.iter().filter(|e| e.t <= t + 1e-9) {
            match e.kind {
                CursorEventKind::Down => down = true,
                CursorEventKind::Up => down = false,
                CursorEventKind::Click | CursorEventKind::DoubleClick => down = t - e.t < 0.09,
                _ => {}
            }
        }
        down
    }
    /// Events still showing at `t`: (age 0..1, event).
    pub fn live(&self, t: f64) -> impl Iterator<Item = (f32, &CursorEvent)> {
        self.events.iter().filter_map(move |e| {
            let life = if matches!(e.kind, CursorEventKind::Key(_)) { KEY_SECS } else { RIPPLE_SECS };
            let age = (t - e.t) / life;
            (0.0..1.0).contains(&age).then_some((age as f32, e))
        })
    }
    pub fn sort(&mut self) {
        sort_by_time(&mut self.events, |e| e.t);
    }
}

/// Value at `t` with Linear segments curved through the neighbouring keys.
pub fn spline_at(a: &Animated, t: f64) -> f64 {
    let k = &a.keys;
    if k.len() < 3 || !a.baked.is_empty() || t <= k[0].t || t >= k[k.len() - 1].t {
        return a.at(t);
    }
    let i = k.partition_point(|x| x.t <= t) - 1;
    if k[i].ease != Ease::Linear {
        return a.at(t);
    }
    let (p1, p2) = (k[i], k[i + 1]);
    let span = (p2.t - p1.t).max(1e-9);
    // tangents (value per second): the slope across the two neighbours; flat at the ends of the run
    // and next to an eased segment, where the pointer arrives or leaves at rest
    let slope = |j: usize| -> f64 {
        if j == 0 || j + 1 >= k.len() || k[j - 1].ease != Ease::Linear || k[j].ease != Ease::Linear {
            return 0.0;
        }
        (k[j + 1].v - k[j - 1].v) / (k[j + 1].t - k[j - 1].t).max(1e-9)
    };
    let (m1, m2) = (slope(i) * span, slope(i + 1) * span);
    let f = (t - p1.t) / span;
    let (f2, f3) = (f * f, f * f * f);
    (2.0 * f3 - 3.0 * f2 + 1.0) * p1.v + (f3 - 2.0 * f2 + f) * m1 + (-2.0 * f3 + 3.0 * f2) * p2.v + (f3 - f2) * m2
}

/// Going to another page in the same tab: from clip-local `t` on, the address is `url` and (when
/// not empty) the front tab is called `title`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Nav {
    pub t: f64,
    pub url: String,
    pub title: String,
    /// The front tab's icon picture from here on (empty = unchanged).
    pub favicon: String,
}

/// A stable sort by time for the short, nearly sorted lists of this feature: an insertion sort is a few
/// bytes of code, `sort_by` a few KB of exe per element type.
pub fn sort_by_time<T>(v: &mut [T], t: fn(&T) -> f64) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && t(&v[j - 1]) > t(&v[j]) {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Upsert a key at `t` (a constant first becomes a held key at 0, so earlier times keep its value).
pub fn put_key(a: &mut Animated, t: f64, v: f64, ease: Ease) {
    a.unlink();
    if !a.is_animated() && t > 1e-4 {
        a.keys.push(Keyframe { t: 0.0, v: a.value, ease: Ease::Hold });
    }
    a.keys.retain(|k| (k.t - t).abs() > 1e-4);
    let i = a.keys.partition_point(|k| k.t < t);
    a.keys.insert(i, Keyframe { t, v, ease });
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrowserStyle {
    pub display: BrowserDisplay,
    pub theme: BrowserTheme,
    pub url: String,
    /// Later pages of the same tab, by time.
    pub nav: Vec<Nav>,
    pub tabs: Vec<BrowserTab>,
    /// Index into `tabs` of the tab in front; key it with Hold keys to switch tabs.
    pub active_tab: Animated,
    /// Window size in project px (the clip's Scale multiplies it).
    pub w: f32,
    pub h: f32,
    pub corner: f32,
    /// Shadow size in project px (0 = none).
    pub shadow: f32,
    pub show_tabs: bool,
    pub show_toolbar: bool,
    pub show_controls: bool,
    pub show_bookmarks: bool,
    pub bookmarks: Vec<String>,
    pub font: String,
    /// Around the window: one colour, or top -> bottom when `bg2` differs.
    pub bg: [u8; 4],
    pub bg2: [u8; 4],
    /// Optional picture around the window (stretched over the frame).
    pub bg_image: String,
    /// The page moves up by this many project px.
    pub scroll: Animated,
    /// 1 = the page fits the window's width. Zooming enlarges the layers below (not a picture of
    /// them), so a page with more pixels than the frame stays sharp.
    pub zoom: Animated,
    /// The point the zoom holds still, project px from the middle of the page area (0, 0 = the
    /// middle). "Zoom into where the mouse is" puts the cursor's position here.
    pub zoom_x: Animated,
    pub zoom_y: Animated,
    /// Fit the pictures below as pages when this browser or a next page is added.
    pub auto_fit: bool,
    /// Window zoom: the whole browser - frame, tabs, page and mouse - this many times bigger, like a
    /// camera moving closer to the screen. (`zoom` enlarges the page inside an unchanged window.)
    pub wzoom: Animated,
    /// The point the Window zoom holds still, project px from the frame's middle.
    pub wzoom_x: Animated,
    pub wzoom_y: Animated,
    /// How much of the address is typed, 0..1.
    pub url_typed: Animated,
    /// Page-load bar under the address, 0..1 (hidden at 0 and at 1).
    pub load: Animated,
    pub cursor: Option<Cursor>,
}

impl Default for BrowserStyle {
    fn default() -> Self {
        Self {
            display: BrowserDisplay::Window,
            theme: BrowserTheme::Light,
            url: "https://example.com".into(),
            nav: Vec::new(),
            tabs: vec![BrowserTab { title: "New Tab".into(), favicon: String::new() }],
            active_tab: a0(),
            w: 1600.0,
            h: 940.0,
            corner: 12.0,
            shadow: 40.0,
            show_tabs: true,
            show_toolbar: true,
            show_controls: true,
            show_bookmarks: false,
            bookmarks: Vec::new(),
            font: String::new(),
            bg: [36, 40, 52, 255],
            bg2: [88, 96, 124, 255],
            bg_image: String::new(),
            scroll: a0(),
            zoom: a1(),
            zoom_x: a0(),
            zoom_y: a0(),
            auto_fit: true,
            wzoom: a1(),
            wzoom_x: a0(),
            wzoom_y: a0(),
            url_typed: a1(),
            load: a0(),
            cursor: Some(Cursor::default()),
        }
    }
}

impl BrowserStyle {
    pub fn animated(&self) -> Vec<&Animated> {
        let mut v = vec![
            &self.active_tab,
            &self.scroll,
            &self.zoom,
            &self.zoom_x,
            &self.zoom_y,
            &self.url_typed,
            &self.load,
            &self.wzoom,
            &self.wzoom_x,
            &self.wzoom_y,
        ];
        if let Some(c) = &self.cursor {
            v.extend([&c.x, &c.y]);
        }
        v
    }
    pub fn animated_mut(&mut self) -> Vec<&mut Animated> {
        let mut v = vec![
            &mut self.active_tab,
            &mut self.scroll,
            &mut self.zoom,
            &mut self.zoom_x,
            &mut self.zoom_y,
            &mut self.url_typed,
            &mut self.load,
            &mut self.wzoom,
            &mut self.wzoom_x,
            &mut self.wzoom_y,
        ];
        if let Some(c) = &mut self.cursor {
            v.extend([&mut c.x, &mut c.y]);
        }
        v
    }
    fn nav_at(&self, t: f64) -> Option<&Nav> {
        self.nav.iter().filter(|n| n.t <= t + 1e-9).last()
    }
    /// The address as far as it is typed at clip-local `t`.
    pub fn url_at(&self, t: f64) -> &str {
        let url = self.nav_at(t).map_or(self.url.as_str(), |n| n.url.as_str());
        let n = url.chars().count();
        let k = (self.url_typed.at(t).clamp(0.0, 1.0) * n as f64).round() as usize;
        match url.char_indices().nth(k) {
            Some((i, _)) => &url[..i],
            None => url,
        }
    }
    /// Title of tab `i` at clip-local `t` (the front tab follows `nav`).
    pub fn title_at(&self, i: usize, t: f64) -> &str {
        let own = self.tabs.get(i).map_or("", |t| t.title.as_str());
        if i != self.tab_at(t) {
            return own;
        }
        self.nav_at(t).map(|n| n.title.as_str()).filter(|s| !s.is_empty()).unwrap_or(own)
    }
    /// Go to `url` at `t` (same tab). An empty `url` keeps the address, an empty `title` the title.
    pub fn go(&mut self, t: f64, url: &str, title: &str) {
        let url = if url.is_empty() { self.url_at(f64::MAX).to_string() } else { url.to_string() };
        self.nav.retain(|n| (n.t - t).abs() > 1e-4);
        self.nav.push(Nav { t, url, title: title.to_string(), favicon: String::new() });
        sort_by_time(&mut self.nav, |n| n.t);
    }
    /// Preset: the address types itself out over `secs` from `t`.
    pub fn type_url(&mut self, t: f64, secs: f64) {
        if t > 1e-3 {
            put_key(&mut self.url_typed, t - 1e-3, 1.0, Ease::Hold);
        }
        put_key(&mut self.url_typed, t, 0.0, Ease::Linear);
        put_key(&mut self.url_typed, t + secs.max(0.05), 1.0, Ease::Hold);
    }
    /// Preset: the load bar runs for `secs` from `t` (fast at first, like a page arriving).
    pub fn run_load(&mut self, t: f64, secs: f64) {
        if t > 1e-3 {
            put_key(&mut self.load, t - 1e-3, 0.0, Ease::Hold);
        }
        put_key(&mut self.load, t, 0.08, Ease::EaseOut);
        put_key(&mut self.load, t + secs.max(0.05), 1.0, Ease::Hold);
    }
    /// Preset: bring tab `i` to the front at `t` and run the load bar.
    pub fn switch_tab(&mut self, t: f64, i: usize, load: f64) {
        put_key(&mut self.active_tab, t, i.min(self.tabs.len().saturating_sub(1)) as f64, Ease::Hold);
        if load > 0.0 {
            self.run_load(t, load);
        }
    }
    /// Icon picture of tab `i` at clip-local `t` (the front tab follows `nav`).
    pub fn favicon_at(&self, i: usize, t: f64) -> &str {
        let own = self.tabs.get(i).map_or("", |t| t.favicon.as_str());
        if i != self.tab_at(t) {
            return own;
        }
        let nav = self.nav.iter().filter(|n| n.t <= t + 1e-9 && !n.favicon.is_empty()).last();
        nav.map_or(own, |n| n.favicon.as_str())
    }
    /// The zoom at `t` and the point it holds still (project px from the frame's middle).
    pub fn zoom_at(&self, t: f64) -> (f64, f64, f64) {
        (self.zoom.at(t).clamp(0.05, 64.0), self.zoom_x.at(t), self.zoom_y.at(t))
    }
    /// Where a point of the page (project px from the frame's middle, before scroll and zoom) is
    /// shown at `t` - the same sum `engine::browser::LayerView` does for the layers.
    pub fn shown(&self, t: f64, p: (f64, f64)) -> (f64, f64) {
        let (z, ax, ay) = self.zoom_at(t);
        (ax + (p.0 - ax) * z, ay + (p.1 - self.scroll.at(t) - ay) * z)
    }
    /// The point of the page shown at `p` at `t` (the inverse of `shown`).
    pub fn page_point(&self, t: f64, p: (f64, f64)) -> (f64, f64) {
        let (z, ax, ay) = self.zoom_at(t);
        (ax + (p.0 - ax) / z, ay + (p.1 - ay) / z + self.scroll.at(t))
    }
    /// Preset: zoom glides to `z` over `secs` from `t`, holding the point (x, y) still. The point is
    /// set for the whole move (it is not animated: a moving anchor would slide the page).
    pub fn zoom_to(&mut self, t: f64, secs: f64, z: f64, at: Option<(f64, f64)>) {
        // ponytail: a new point only when not zoomed yet - moving the point of a page that is already
        // zoomed would make it jump; zoom out first (or keyframe Zoom X / Y by hand)
        if let Some((x, y)) = at.filter(|_| (self.zoom.at(t) - 1.0).abs() < 1e-6) {
            put_key(&mut self.zoom_x, t, x, Ease::Hold);
            put_key(&mut self.zoom_y, t, y, Ease::Hold);
        }
        self.glide(true, t, secs, z.clamp(0.05, 64.0));
    }
    /// Preset: the Window zoom glides to `z` over `secs` from `t`, holding the frame point (x, y)
    /// still (project px from the frame's middle). As with `zoom_to`, the point is only set while
    /// not zoomed.
    pub fn window_zoom_to(&mut self, t: f64, secs: f64, z: f64, at: Option<(f64, f64)>) {
        if let Some((x, y)) = at.filter(|_| (self.wzoom.at(t) - 1.0).abs() < 1e-6) {
            put_key(&mut self.wzoom_x, t, x, Ease::Hold);
            put_key(&mut self.wzoom_y, t, y, Ease::Hold);
        }
        let now = self.wzoom.at(t);
        put_key(&mut self.wzoom, t, now, Ease::PRESETS[0].1);
        put_key(&mut self.wzoom, t + secs.max(0.05), z.clamp(0.05, 64.0), Ease::Hold);
    }
    /// Preset: scroll (or zoom) glides to `v` over `secs` from `t` and stays there.
    pub fn glide(&mut self, zoom: bool, t: f64, secs: f64, v: f64) {
        let a = if zoom { &mut self.zoom } else { &mut self.scroll };
        let now = a.at(t);
        put_key(a, t, now, Ease::PRESETS[0].1);
        put_key(a, t + secs.max(0.05), v, Ease::Hold);
    }
    pub fn tab_at(&self, t: f64) -> usize {
        (self.active_tab.at(t).round().max(0.0) as usize).min(self.tabs.len().saturating_sub(1))
    }
    /// Height of the bars above the page, project px.
    pub fn chrome_h(&self) -> f32 {
        if self.display == BrowserDisplay::Page {
            return 0.0;
        }
        let (tabs, bar, marks) = match self.theme {
            BrowserTheme::Minimal | BrowserTheme::Mobile => (0.0, 48.0, 0.0),
            _ => (40.0, 46.0, 30.0),
        };
        (if self.show_tabs { tabs } else { 0.0 })
            + (if self.show_toolbar { bar } else { 0.0 })
            + (if self.show_bookmarks { marks } else { 0.0 })
    }
    /// Height of the bar under the page (Mobile's home bar).
    pub fn foot_h(&self) -> f32 {
        if self.theme == BrowserTheme::Mobile && self.display == BrowserDisplay::Window {
            28.0
        } else {
            0.0
        }
    }
    /// How much of the page's height is seen at once, in project px, in a `pw` x `ph` frame: all of the
    /// frame in Page-only display, else what the window's page area holds once the page fits its width.
    pub fn view_h(&self, pw: f64, ph: f64) -> f64 {
        if self.display == BrowserDisplay::Page {
            return ph;
        }
        let (vw, vh) = self.viewport();
        vh as f64 * pw / vw as f64
    }
    /// The page area's size, project px.
    pub fn viewport(&self) -> (f32, f32) {
        (self.w.max(16.0), (self.h - self.chrome_h() - self.foot_h()).max(16.0))
    }
}

/// Reduce a timed polyline to the points that matter (Ramer-Douglas-Peucker over x, y with the
/// distance in px): a point survives when the path would miss it by more than `tol`. The first and
/// last point always survive. Returns the kept indices, in order.
pub fn simplify_path(pts: &[(f64, f64, f64)], tol: f64) -> Vec<usize> {
    if pts.len() < 3 {
        return (0..pts.len()).collect();
    }
    let mut keep = vec![false; pts.len()];
    (keep[0], keep[pts.len() - 1]) = (true, true);
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (ax, ay, at) = pts[a];
        let (bx, by, bt) = pts[b];
        let (mut worst, mut wi) = (0.0f64, a);
        for (i, &(x, y, t)) in pts.iter().enumerate().take(b).skip(a + 1) {
            // distance from where a straight, evenly timed move a->b would be at this moment: a pause
            // or a change of speed is kept as well as a change of direction
            let f = if bt > at { (t - at) / (bt - at) } else { 0.5 };
            let d = (x - (ax + (bx - ax) * f)).hypot(y - (ay + (by - ay) * f));
            if d > worst {
                (worst, wi) = (d, i);
            }
        }
        if worst > tol {
            keep[wi] = true;
            stack.push((a, wi));
            stack.push((wi, b));
        }
    }
    (0..pts.len()).filter(|&i| keep[i]).collect()
}

/// One raw sample of a recorded take: clip-local time, page position, button down, wheel since the
/// last sample (project px, + = down).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CursorSample {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub down: bool,
    pub wheel: f64,
}

/// What a take becomes: position keys, click / drag / wheel events and scroll keys (absolute scroll,
/// starting from `scroll0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Take {
    pub keys: Vec<(f64, f64, f64)>,
    pub events: Vec<CursorEvent>,
    pub scroll: Vec<(f64, f64)>,
}

/// Turn raw samples into a few keys and events. `smoothing` 0..1: 0 keeps almost every wiggle,
/// 1 keeps only the big moves (the tolerance runs from 1 to 24 px).
pub fn reduce_take(samples: &[CursorSample], smoothing: f64, scroll0: f64) -> Take {
    let mut take = Take::default();
    if samples.is_empty() {
        return take;
    }
    // jitter filter: a short moving average, never across a button edge so a click stays where it was
    let n = samples.len();
    let r = (smoothing.clamp(0.0, 1.0) * 3.0).round() as usize;
    let pts: Vec<(f64, f64, f64)> = (0..n)
        .map(|i| {
            let (mut a, mut b) = (i.saturating_sub(r), (i + r).min(n - 1));
            while a < i && samples[a..=i].iter().any(|s| s.down != samples[i].down) {
                a += 1;
            }
            while b > i && samples[i..=b].iter().any(|s| s.down != samples[i].down) {
                b -= 1;
            }
            let w = (b - a + 1) as f64;
            let (sx, sy) = samples[a..=b].iter().fold((0.0, 0.0), |acc, s| (acc.0 + s.x, acc.1 + s.y));
            (sx / w, sy / w, samples[i].t)
        })
        .collect();
    let tol = 1.0 + smoothing.clamp(0.0, 1.0) * 23.0;
    let mut keep = simplify_path(&pts, tol);
    // button edges: a press and release close together in time and place is a click (two in a row a
    // double click); anything else is a drag. The position at an edge is always a key.
    let mut edges: Vec<usize> = (1..n).filter(|&i| samples[i].down != samples[i - 1].down).collect();
    if samples[0].down {
        edges.insert(0, 0);
    }
    let mut i = 0;
    while i < edges.len() {
        let d = edges[i];
        keep.push(d);
        let up = edges.get(i + 1).copied();
        let quick = up.is_some_and(|u| {
            samples[u].t - samples[d].t < 0.35 && (pts[u].0 - pts[d].0).hypot(pts[u].1 - pts[d].1) < 12.0
        });
        if quick {
            match take.events.last_mut() {
                Some(e) if e.kind == CursorEventKind::Click && samples[d].t - e.t < 0.45 => {
                    e.kind = CursorEventKind::DoubleClick
                }
                _ => take.events.push(CursorEvent { t: samples[d].t, kind: CursorEventKind::Click }),
            }
        } else {
            take.events.push(CursorEvent { t: samples[d].t, kind: CursorEventKind::Down });
            if let Some(u) = up {
                keep.push(u);
                take.events.push(CursorEvent { t: samples[u].t, kind: CursorEventKind::Up });
            }
        }
        i += 2;
    }
    keep.sort_unstable();
    keep.dedup();
    for i in keep {
        // two keys at one instant would swallow each other
        if take.keys.last().is_none_or(|k| pts[i].2 > k.2 + 1e-3) {
            take.keys.push((pts[i].2, pts[i].0, pts[i].1));
        }
    }
    // keys are (t, x, y) from here on
    // wheel: bursts become one eased scroll move each, plus an indicator event at the start
    let mut scroll = scroll0;
    let mut last: Option<f64> = None;
    for s in samples.iter().filter(|s| s.wheel != 0.0) {
        if last.is_none_or(|l| s.t - l > 0.25) {
            take.scroll.push((s.t, scroll));
            take.events.push(CursorEvent { t: s.t, kind: CursorEventKind::Scroll(s.wheel.signum() as f32) });
        } else {
            take.scroll.pop();
        }
        scroll = (scroll + s.wheel).max(0.0);
        take.scroll.push((s.t + 0.18, scroll));
        last = Some(s.t);
    }
    sort_by_time(&mut take.events, |e| e.t);
    take
}

// ---------- JSON ----------
// Written by hand through `serde_json::Value`, not derived: the derived (de)serialisers of these few
// structs were over 100 KB of the exe (one copy per JSON reader / writer in the app, measured with
// cargo-bloat), while a `Value` is read and written everywhere already. The shape is what the
// derives made: field names as keys, enum variants by name, a cursor event as {"t", "kind"}.
// Table-driven: `fields` lists the plain fields of a struct once (name, kind, reference) and reading
// and writing go through that list: one small function per kind of value instead of a line of code
// per field and per direction.

use serde_json::{json, Map, Value};

/// A plain field, by reference.
enum F<'a> {
    N(&'a mut f32),
    B(&'a mut bool),
    S(&'a mut String),
    C(&'a mut [u8; 4]),
    /// Keyframable: a number sets a constant, an object is the whole `Animated`.
    A(&'a mut Animated),
}

type Fields<'a, const N: usize> = [(&'static str, F<'a>); N];

impl F<'_> {
    #[inline(never)]
    fn get(&self) -> Value {
        match self {
            F::N(x) => Value::from(**x),
            F::B(x) => Value::Bool(**x),
            F::S(x) => Value::String((**x).clone()),
            F::C(x) => Value::Array(x.iter().map(|&c| Value::from(c)).collect()),
            // through text: `Animated`'s text (de)serialisers are in the app already (every project
            // file); going through `Value` directly compiled a second set of them
            F::A(x) => serde_json::to_string(&**x).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null),
        }
    }
    #[inline(never)]
    fn put(&mut self, v: &Value) {
        match self {
            F::N(x) => **x = v.as_f64().map_or(**x, |n| n as f32),
            F::B(x) => **x = v.as_bool().unwrap_or(**x),
            F::S(x) => {
                if let Some(s) = v.as_str() {
                    **x = s.to_string();
                }
            }
            F::C(x) => {
                if let Some(a) = v.as_array().filter(|a| a.len() >= 3) {
                    for (c, n) in x.iter_mut().zip(a) {
                        *c = n.as_u64().unwrap_or(0).min(255) as u8;
                    }
                }
            }
            F::A(a) => match v {
                Value::Number(n) => **a = Animated::new(n.as_f64().unwrap_or(0.0)),
                Value::Object(_) => {
                    if let Ok(n) = serde_json::from_str(&v.to_string()) {
                        **a = n;
                    }
                }
                _ => {}
            },
        }
    }
}

/// Every field of `fs` that `v` has.
#[inline(never)]
fn load(fs: &mut [(&'static str, F)], v: &Value) {
    for (k, f) in fs {
        if let Some(x) = v.get(*k) {
            f.put(x);
        }
    }
}

/// `fs` as a JSON object.
#[inline(never)]
fn dump(fs: &[(&'static str, F)]) -> Map<String, Value> {
    // (inserted one by one: `collect` into a map sorts first, a second copy of the sort for these pairs)
    let mut o = Map::new();
    for (k, f) in fs {
        o.insert(k.to_string(), f.get());
    }
    o
}

/// A JSON object of text values.
#[inline(never)]
fn texts(kv: &[(&str, &str)]) -> Value {
    let mut o = Map::new();
    for (k, v) in kv {
        o.insert(k.to_string(), Value::from(*v));
    }
    Value::Object(o)
}

fn items<'a>(v: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> {
    v[k].as_array().into_iter().flatten()
}

fn str_of(x: &Value, k: &str) -> String {
    x[k].as_str().unwrap_or("").to_string()
}

impl CursorEvent {
    fn to_json(&self) -> Value {
        let kind = match &self.kind {
            CursorEventKind::Click => json!("Click"),
            CursorEventKind::DoubleClick => json!("DoubleClick"),
            CursorEventKind::Down => json!("Down"),
            CursorEventKind::Up => json!("Up"),
            CursorEventKind::Scroll(d) => json!({ "Scroll": d }),
            CursorEventKind::Key(k) => json!({ "Key": k }),
            CursorEventKind::Shape(s) => json!({ "Shape": s.name() }),
        };
        json!({"t": self.t, "kind": kind})
    }
    fn from_json(v: &Value) -> Option<CursorEvent> {
        let k = &v["kind"];
        let kind = match k.as_str() {
            Some("Click") => CursorEventKind::Click,
            Some("DoubleClick") => CursorEventKind::DoubleClick,
            Some("Down") => CursorEventKind::Down,
            Some("Up") => CursorEventKind::Up,
            Some(_) => return None,
            None => {
                if let Some(d) = k["Scroll"].as_f64() {
                    CursorEventKind::Scroll(d as f32)
                } else if let Some(s) = k["Key"].as_str() {
                    CursorEventKind::Key(s.to_string())
                } else {
                    CursorEventKind::Shape(CursorShape::from_name(k["Shape"].as_str()?)?)
                }
            }
        };
        Some(CursorEvent { t: v["t"].as_f64()?, kind })
    }
}

impl Cursor {
    fn fields(&mut self) -> Fields<'_, 10> {
        [
            ("visible", F::B(&mut self.visible)),
            ("x", F::A(&mut self.x)),
            ("y", F::A(&mut self.y)),
            ("size", F::N(&mut self.size)),
            ("color", F::C(&mut self.color)),
            ("outline", F::C(&mut self.outline)),
            ("ripple", F::C(&mut self.ripple)),
            ("keys", F::B(&mut self.keys)),
            ("smooth", F::B(&mut self.smooth)),
            ("smoothing", F::N(&mut self.smoothing)),
        ]
    }
    pub fn to_json(&self) -> Value {
        let mut o = dump(&self.clone().fields());
        o.insert("shape".into(), Value::from(self.shape.name()));
        o.insert("events".into(), Value::Array(self.events.iter().map(CursorEvent::to_json).collect()));
        Value::Object(o)
    }
    /// Take every field `v` has (the others stay).
    pub fn set_json(&mut self, v: &Value) {
        load(&mut self.fields(), v);
        if let Some(s) = v["shape"].as_str().and_then(CursorShape::from_name) {
            self.shape = s;
        }
        if v["events"].is_array() {
            self.events = items(v, "events").filter_map(CursorEvent::from_json).collect();
            self.sort();
        }
    }
}

impl BrowserStyle {
    fn fields(&mut self) -> Fields<'_, 24> {
        [
            ("url", F::S(&mut self.url)),
            ("active_tab", F::A(&mut self.active_tab)),
            ("w", F::N(&mut self.w)),
            ("h", F::N(&mut self.h)),
            ("corner", F::N(&mut self.corner)),
            ("shadow", F::N(&mut self.shadow)),
            ("show_tabs", F::B(&mut self.show_tabs)),
            ("show_toolbar", F::B(&mut self.show_toolbar)),
            ("show_controls", F::B(&mut self.show_controls)),
            ("show_bookmarks", F::B(&mut self.show_bookmarks)),
            ("font", F::S(&mut self.font)),
            ("bg", F::C(&mut self.bg)),
            ("bg2", F::C(&mut self.bg2)),
            ("bg_image", F::S(&mut self.bg_image)),
            ("scroll", F::A(&mut self.scroll)),
            ("zoom", F::A(&mut self.zoom)),
            ("zoom_x", F::A(&mut self.zoom_x)),
            ("zoom_y", F::A(&mut self.zoom_y)),
            ("auto_fit", F::B(&mut self.auto_fit)),
            ("wzoom", F::A(&mut self.wzoom)),
            ("wzoom_x", F::A(&mut self.wzoom_x)),
            ("wzoom_y", F::A(&mut self.wzoom_y)),
            ("url_typed", F::A(&mut self.url_typed)),
            ("load", F::A(&mut self.load)),
        ]
    }
    /// Every field, by name (what a project file holds, and what `browser.set` returns).
    pub fn to_json(&self) -> Value {
        let mut o = dump(&self.clone().fields());
        o.insert("display".into(), Value::from(if self.display == BrowserDisplay::Page { "Page" } else { "Window" }));
        o.insert("theme".into(), Value::from(self.theme.name()));
        let nav = self.nav.iter().map(|n| {
            let mut v = texts(&[("url", &n.url), ("title", &n.title), ("favicon", &n.favicon)]);
            v["t"] = Value::from(n.t);
            v
        });
        o.insert("nav".into(), Value::Array(nav.collect()));
        let tabs = self.tabs.iter().map(|t| texts(&[("title", &t.title), ("favicon", &t.favicon)]));
        o.insert("tabs".into(), Value::Array(tabs.collect()));
        o.insert("bookmarks".into(), Value::Array(self.bookmarks.iter().map(|b| Value::from(b.as_str())).collect()));
        o.insert("cursor".into(), self.cursor.as_ref().map_or(Value::Null, Cursor::to_json));
        Value::Object(o)
    }
    /// Take every field `v` has (the others stay): how a file is read (onto the defaults) and how
    /// `browser.set` changes a style. Names of looks / displays / pointers in any case; a number for a
    /// keyframable field is a constant; a tab may be just its title.
    pub fn set_json(&mut self, v: &Value) {
        if let Some(d) = v["display"].as_str().and_then(BrowserDisplay::from_name) {
            self.display = d;
        }
        if let Some(t) = v["theme"].as_str().and_then(BrowserTheme::from_name) {
            self.theme = t;
        }
        if v["nav"].is_array() {
            self.nav = items(v, "nav")
                .map(|n| Nav { t: n["t"].as_f64().unwrap_or(0.0), url: str_of(n, "url"), title: str_of(n, "title"), favicon: str_of(n, "favicon") })
                .collect();
        }
        if v["tabs"].as_array().is_some_and(|a| !a.is_empty()) {
            self.tabs = items(v, "tabs")
                .map(|t| match t.as_str() {
                    Some(title) => BrowserTab { title: title.to_string(), favicon: String::new() },
                    None => BrowserTab { title: str_of(t, "title"), favicon: str_of(t, "favicon") },
                })
                .collect();
        }
        if v["bookmarks"].is_array() {
            self.bookmarks = items(v, "bookmarks").filter_map(|b| b.as_str().map(String::from)).collect();
        }
        load(&mut self.fields(), v);
        (self.w, self.h) = (self.w.clamp(120.0, 16000.0), self.h.clamp(120.0, 16000.0));
        match v.get("cursor") {
            Some(Value::Null) => self.cursor = None,
            Some(c @ Value::Object(_)) => self.cursor.get_or_insert_with(Cursor::default).set_json(c),
            _ => {}
        }
    }
}

impl Serialize for BrowserStyle {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for BrowserStyle {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let mut b = BrowserStyle::default();
        b.set_json(&v);
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_corners_and_drops_jitter() {
        // an L: 50 points right, 50 points down, with half a pixel of noise
        let mut pts = Vec::new();
        for i in 0..100 {
            let n = if i % 2 == 0 { 0.4 } else { -0.4 };
            let (x, y) = if i < 50 { (i as f64 * 4.0, n) } else { (196.0 + n, (i - 49) as f64 * 4.0) };
            pts.push((x, y, i as f64 / 60.0));
        }
        let kept = simplify_path(&pts, 3.0);
        assert!(kept.len() <= 5, "an L needs its ends and its corner, got {}", kept.len());
        assert_eq!((kept[0], *kept.last().unwrap()), (0, 99));
        assert!(kept.iter().any(|&i| (48..=51).contains(&i)), "the corner survives: {kept:?}");
        // a tolerance of nothing keeps everything that is off the line
        assert!(simplify_path(&pts, 0.01).len() > 50);
    }

    #[test]
    fn simplify_keeps_a_pause() {
        // straight line, but the pointer waits in the middle: the wait must not be averaged away
        let mut pts: Vec<(f64, f64, f64)> = (0..30).map(|i| (i as f64 * 10.0, 0.0, i as f64 / 60.0)).collect();
        pts.extend((0..60).map(|i| (290.0, 0.0, 0.5 + i as f64 / 60.0)));
        pts.extend((0..30).map(|i| (290.0 + i as f64 * 10.0, 0.0, 1.5 + i as f64 / 60.0)));
        assert!(simplify_path(&pts, 4.0).len() >= 4);
    }

    #[test]
    fn a_take_becomes_a_few_keys_and_events() {
        // 2 s at 120 Hz: move right, click, drag down, release; one wheel burst
        let mut s = Vec::new();
        for i in 0..240 {
            let t = i as f64 / 120.0;
            let x = (t * 300.0).min(300.0) + if i % 2 == 0 { 0.3 } else { -0.3 };
            let y = if t > 1.4 { (t - 1.4) * 200.0 } else { 0.0 };
            let down = (1.1..1.2).contains(&t) || (1.4..1.9).contains(&t);
            let wheel = if (0.5..0.6).contains(&t) { 20.0 } else { 0.0 };
            s.push(CursorSample { t, x, y, down, wheel });
        }
        let take = reduce_take(&s, 0.3, 100.0);
        assert!(take.keys.len() < 16, "240 samples must not be 240 keys: {}", take.keys.len());
        assert!(take.keys.windows(2).all(|w| w[1].0 > w[0].0), "key times strictly increase");
        let kinds: Vec<_> = take.events.iter().map(|e| e.kind.clone()).collect();
        assert!(kinds.contains(&CursorEventKind::Click), "{kinds:?}");
        assert!(kinds.contains(&CursorEventKind::Down) && kinds.contains(&CursorEventKind::Up), "{kinds:?}");
        assert!(kinds.contains(&CursorEventKind::Scroll(1.0)), "{kinds:?}");
        // the click lands where the pointer was (300, 0), not on a smoothed-away spot
        let click = take.events.iter().find(|e| e.kind == CursorEventKind::Click).unwrap();
        let key = take.keys.iter().find(|k| (k.0 - click.t).abs() < 1e-6).expect("a key at the click");
        assert!((key.1 - 300.0).abs() < 2.0 && key.2.abs() < 2.0, "{key:?}");
        // the wheel burst: 12 ticks of 20 px from 100
        assert_eq!(take.scroll.first().map(|s| s.1), Some(100.0));
        assert_eq!(take.scroll.last().map(|s| s.1), Some(340.0));
        assert_eq!(take.scroll.len(), 2, "one burst is one move");
    }

    #[test]
    fn two_quick_clicks_are_a_double_click() {
        let s: Vec<CursorSample> = (0..60)
            .map(|i| CursorSample { t: i as f64 / 60.0, x: 5.0, y: 5.0, down: matches!(i, 10..=13 | 22..=25), wheel: 0.0 })
            .collect();
        let take = reduce_take(&s, 0.5, 0.0);
        assert_eq!(take.events.len(), 1);
        assert_eq!(take.events[0].kind, CursorEventKind::DoubleClick);
    }

    #[test]
    fn smooth_cursor_curves_between_linear_keys_and_stops_on_eased_ones() {
        let mut c = Cursor::default();
        for (t, x, y) in [(0.0, 0.0, 0.0), (1.0, 100.0, 0.0), (2.0, 100.0, 100.0), (3.0, 0.0, 100.0)] {
            c.x.keys.push(Keyframe { t, v: x, ease: Ease::Linear });
            c.y.keys.push(Keyframe { t, v: y, ease: Ease::Linear });
        }
        // through every key
        for (t, x, y) in [(0.0, 0.0, 0.0), (1.0, 100.0, 0.0), (2.0, 100.0, 100.0)] {
            let p = c.pos(t);
            assert!((p.0 - x).abs() < 1e-6 && (p.1 - y).abs() < 1e-6, "{p:?} at {t}");
        }
        // rounding the corner: at 1.5 the straight path is at x = 100; the curve swings outside it
        assert!(c.pos(1.5).0 > 100.5, "{:?}", c.pos(1.5));
        c.smooth = false;
        assert!((c.pos(1.5).0 - 100.0).abs() < 1e-9);
        // an eased segment is a straight move
        c.smooth = true;
        c.x.keys[1].ease = Ease::EaseInOut;
        c.y.keys[1].ease = Ease::EaseInOut;
        assert!((c.pos(1.5).0 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn url_types_out_and_events_report_state() {
        let mut b = BrowserStyle { url: "https://a.b".into(), ..Default::default() };
        b.url_typed = Animated::new(0.0);
        b.url_typed.keys = vec![Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear }, Keyframe { t: 1.0, v: 1.0, ease: Ease::Linear }];
        assert_eq!(b.url_at(0.0), "");
        assert_eq!(b.url_at(0.5), "https:");
        assert_eq!(b.url_at(2.0), "https://a.b");
        // display: a window by default (also for a project saved before the setting existed); Page
        // only has no bars and sees the whole frame's height
        let old: BrowserStyle = serde_json::from_str(r#"{"url":"https://a.b","theme":"Dark"}"#).unwrap();
        assert_eq!((old.display, old.chrome_h()), (BrowserDisplay::Window, 86.0));
        assert!(old.view_h(1920.0, 1080.0) < 1080.0);
        let page = BrowserStyle { display: BrowserDisplay::Page, ..old.clone() };
        assert_eq!((page.chrome_h(), page.foot_h(), page.view_h(1920.0, 1080.0)), (0.0, 0.0, 1080.0));
        let back: BrowserStyle = serde_json::from_str(&serde_json::to_string(&page).unwrap()).unwrap();
        assert_eq!(back, page);
        assert_eq!(BrowserDisplay::from_name("Page only"), Some(BrowserDisplay::Page));
        // going to another page changes the address and the front tab's title from then on
        b.go(3.0, "https://c.d/e", "Next");
        assert_eq!((b.url_at(2.9), b.title_at(0, 2.9)), ("https://a.b", "New Tab"));
        assert_eq!((b.url_at(3.0), b.title_at(0, 3.0)), ("https://c.d/e", "Next"));
        // presets: keys that leave the time before them alone
        b.url_typed = Animated::new(1.0);
        b.type_url(5.0, 1.0);
        assert_eq!((b.url_typed.at(4.0), b.url_typed.at(5.5), b.url_typed.at(7.0)), (1.0, 0.5, 1.0));
        b.run_load(5.0, 0.5);
        assert!(b.load.at(4.0) == 0.0 && b.load.at(5.2) > 0.08 && b.load.at(6.0) == 1.0);
        b.tabs.push(BrowserTab { title: "Two".into(), favicon: String::new() });
        b.switch_tab(6.0, 1, 0.0);
        assert_eq!((b.tab_at(5.9), b.tab_at(6.0)), (0, 1));
        // a page point and where it is shown are inverses, zoomed and scrolled
        b.zoom_to(7.0, 1.0, 4.0, Some((100.0, -50.0)));
        assert_eq!(b.zoom_at(9.0), (4.0, 100.0, -50.0));
        assert_eq!(b.shown(9.0, (100.0, -50.0)), (100.0, -50.0), "the zoom's point stays put");
        assert_eq!(b.shown(9.0, (110.0, -50.0)), (140.0, -50.0));
        let back = b.page_point(9.0, b.shown(9.0, (333.0, 21.0)));
        assert!((back.0 - 333.0).abs() < 1e-9 && (back.1 - 21.0).abs() < 1e-9);
        // already zoomed: a later zoom keeps the point (changing it would make the page jump)
        b.zoom_to(9.0, 1.0, 8.0, Some((-300.0, 0.0)));
        assert_eq!(b.zoom_at(11.0), (8.0, 100.0, -50.0));
        b.zoom = Animated::new(1.0);
        b.glide(false, 2.0, 1.0, 500.0);
        assert_eq!((b.scroll.at(1.0), b.scroll.at(9.0)), (0.0, 500.0));
        assert!((b.scroll.at(2.5) - 250.0).abs() < 0.01, "eased: half way at half time");
        let c = b.cursor.as_mut().unwrap();
        c.events = vec![
            CursorEvent { t: 1.0, kind: CursorEventKind::Shape(CursorShape::Hand) },
            CursorEvent { t: 2.0, kind: CursorEventKind::Down },
            CursorEvent { t: 3.0, kind: CursorEventKind::Up },
            CursorEvent { t: 4.0, kind: CursorEventKind::Click },
        ];
        assert_eq!((c.shape_at(0.5), c.shape_at(1.5)), (CursorShape::Arrow, CursorShape::Hand));
        assert!(!c.pressed(1.9) && c.pressed(2.5) && !c.pressed(3.5) && c.pressed(4.05) && !c.pressed(4.2));
        assert_eq!(c.live(4.1).count(), 1);
        assert_eq!(c.live(5.0).count(), 0);
    }
}
