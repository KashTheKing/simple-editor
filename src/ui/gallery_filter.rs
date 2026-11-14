//! The Gallery's shared view state: one search box, the source chips (All / Built-in / Mine / Installed /
//! Online), a category, a sort order and a card size - the same for every tab - plus the grid helper that
//! makes every tab one wrapping grid filling the panel's width. Pure (no egui widgets but `grid`), so the
//! rules are unit-tested.

use eframe::egui;

/// Where an item comes from; also the chip the user filters by (`All` = no filter).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Src {
    #[default]
    All,
    BuiltIn,
    Mine,
    Installed,
    Online,
}

impl Src {
    /// "all", "built-in" / "builtin", "mine", "installed", "online" (any case).
    pub fn from_name(s: &str) -> Option<Src> {
        [Src::All, Src::BuiltIn, Src::Mine, Src::Installed, Src::Online]
            .into_iter()
            .find(|k| k.label().replace('-', "").eq_ignore_ascii_case(&s.replace('-', "")))
    }
    pub fn label(self) -> &'static str {
        match self {
            Src::All => "All",
            Src::BuiltIn => "Built-in",
            Src::Mine => "Mine",
            Src::Installed => "Installed",
            Src::Online => "Online",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Sort {
    /// The catalogue's own order: built-ins as designed, then yours, then the library's.
    #[default]
    Catalogue,
    Name,
    /// Your newest saves first (then by name).
    Recent,
    /// What you apply most, first.
    Used,
}

impl Sort {
    /// "default", "name", "recent", "used" / "most used" (any case).
    pub fn from_name(s: &str) -> Option<Sort> {
        let s = s.trim().to_lowercase();
        Sort::ALL.into_iter().find(|k| k.label().to_lowercase() == s || (*k == Sort::Used && s == "used"))
    }
    pub const ALL: [Sort; 4] = [Sort::Catalogue, Sort::Name, Sort::Recent, Sort::Used];
    pub fn label(self) -> &'static str {
        match self {
            Sort::Catalogue => "Default",
            Sort::Name => "Name",
            Sort::Recent => "Recent",
            Sort::Used => "Most used",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Size {
    S,
    #[default]
    M,
    L,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::S, Size::M, Size::L];
    pub fn label(self) -> &'static str {
        match self {
            Size::S => "S",
            Size::M => "M",
            Size::L => "L",
        }
    }
    /// The card width the grid aims for (it stretches a little so the row fills the panel).
    pub fn base_w(self) -> f32 {
        match self {
            Size::S => 70.0,
            Size::M => 96.0,
            Size::L => 140.0,
        }
    }
}

/// What the header edits and every tab reads.
#[derive(Default, Clone, Debug)]
pub struct Filters {
    pub q: String,
    pub src: Src,
    /// A category (effect category, transition kind, sound / library tag); empty = every one.
    pub cat: String,
    pub sort: Sort,
    pub size: Size,
}

impl Filters {
    /// Does an item pass? `origin`: where it comes from; `cats`: its categories / tags; `hay`: the text the
    /// search looks in (name, tags, description). Every word of the query must be in `hay`.
    pub fn passes(&self, origin: Src, cats: &[&str], hay: &str) -> bool {
        if self.src != Src::All && self.src != origin {
            return false;
        }
        if !self.cat.is_empty() && !cats.iter().any(|c| c.eq_ignore_ascii_case(&self.cat)) {
            return false;
        }
        let hay = hay.to_lowercase();
        self.q.to_lowercase().split_whitespace().all(|w| hay.contains(w))
    }

    /// Is any filter on (the header shows a "clear")?
    pub fn active(&self) -> bool {
        !self.q.is_empty() || self.src != Src::All || !self.cat.is_empty()
    }

    pub fn clear(&mut self) {
        self.q.clear();
        self.src = Src::All;
        self.cat.clear();
    }
}

/// Order items in place: by name, newest saves first, or most used first (`used` = apply count, `created`
/// = the save time, 0 for what is not yours).
pub fn sort_by<T>(items: &mut [T], sort: Sort, key: impl Fn(&T) -> (String, u32, u64)) {
    items.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        let by_name = ka.0.to_lowercase().cmp(&kb.0.to_lowercase());
        match sort {
            Sort::Catalogue => std::cmp::Ordering::Equal,
            Sort::Name => by_name,
            Sort::Used => kb.1.cmp(&ka.1).then(by_name),
            Sort::Recent => kb.2.cmp(&ka.2).then(by_name),
        }
    });
}

/// How many columns of cards `base` wide fit `avail`, and the width each then gets so the row fills it
/// (at most 1.5x `base`; a pane narrower than one card gets one card as wide as the pane).
pub fn columns(avail: f32, gap: f32, base: f32) -> (usize, f32) {
    let cols = (((avail + gap) / (base + gap)).floor() as usize).max(1);
    // a hair under the exact share: a row that fits to the last pixel would wrap its last card
    let w = (avail - gap * (cols - 1) as f32) / cols as f32 - 0.05;
    (cols, w.clamp(40.0_f32.min(avail.max(1.0)), (base * 1.5).max(avail.min(base))).min(avail.max(1.0)))
}

/// A card's tile size (width, height = 16:9) in the grid being drawn.
pub fn dims(ctx: &egui::Context) -> (f32, f32) {
    ctx.data(|d| d.get_temp::<(f32, f32)>(egui::Id::new("gallery_dims"))).unwrap_or((96.0, 54.0))
}

/// The card width the user chose (set once a frame by `show`).
pub fn set_size(ctx: &egui::Context, size: Size) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("gallery_base"), size.base_w()));
}

/// One wrapping grid of cards: measure the room it has, size the cards so whole columns fill it, then lay
/// them out left to right and wrap. Every card of every tab is drawn inside one of these, so a narrow pane,
/// a wide one and a resize all reflow the same way.
pub fn grid(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let base = ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::new("gallery_base"))).unwrap_or(96.0);
    let (_, w) = columns(ui.available_width(), ui.spacing().item_spacing.x, base);
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("gallery_dims"), (w, (w * 9.0 / 16.0).round())));
    ui.horizontal_wrapped(add);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(q: &str, src: Src, cat: &str) -> Filters {
        Filters { q: q.into(), src, cat: cat.into(), ..Default::default() }
    }

    #[test]
    fn search_needs_every_word_and_looks_in_the_haystack() {
        let hay = "Blur Gaussian soft Stylize";
        assert!(f("", Src::All, "").passes(Src::BuiltIn, &[], hay));
        assert!(f("soft blur", Src::All, "").passes(Src::BuiltIn, &[], hay), "any order, any case");
        assert!(!f("blur sharp", Src::All, "").passes(Src::BuiltIn, &[], hay));
    }

    #[test]
    fn the_source_chip_and_the_category_both_filter() {
        let blur = ["Stylize"];
        assert!(f("", Src::Mine, "").passes(Src::Mine, &blur, "x"));
        assert!(!f("", Src::Mine, "").passes(Src::BuiltIn, &blur, "x"));
        assert!(!f("", Src::Online, "").passes(Src::Installed, &blur, "x"));
        assert!(f("", Src::All, "stylize").passes(Src::BuiltIn, &blur, "x"), "case-insensitive");
        assert!(!f("", Src::All, "Color").passes(Src::BuiltIn, &blur, "x"));
        assert!(!f("", Src::All, "Color").passes(Src::Mine, &[], "x"), "no category: not in one");
        let mut fl = f("a", Src::Mine, "c");
        assert!(fl.active());
        fl.clear();
        assert!(!fl.active());
    }

    #[test]
    fn sorting_by_name_recency_and_use() {
        let mut v = vec![("b", 1, 5), ("a", 9, 1), ("C", 1, 9)];
        let key = |t: &(&str, u32, u64)| (t.0.to_string(), t.1, t.2);
        sort_by(&mut v, Sort::Name, key);
        assert_eq!(v.iter().map(|t| t.0).collect::<Vec<_>>(), ["a", "b", "C"]);
        sort_by(&mut v, Sort::Used, key);
        assert_eq!(v[0].0, "a", "9 uses");
        sort_by(&mut v, Sort::Recent, key);
        assert_eq!(v[0].0, "C", "newest save");
    }

    #[test]
    fn columns_fill_the_row_at_any_width() {
        for avail in [60.0_f32, 120.0, 205.0, 350.0, 700.0, 1400.0] {
            for size in Size::ALL {
                let (cols, w) = columns(avail, 8.0, size.base_w());
                assert!(cols >= 1);
                assert!(w * cols as f32 + 8.0 * (cols - 1) as f32 <= avail + 0.01, "{avail}/{size:?}: {cols} x {w}");
                if cols > 1 {
                    assert!(w >= size.base_w() - 0.1 && w <= size.base_w() * 1.5, "{avail}/{size:?}: {w}");
                }
            }
        }
        assert_eq!(columns(205.0, 8.0, 96.0).0, 2);
        assert!(columns(1400.0, 8.0, 96.0).0 > columns(700.0, 8.0, 96.0).0, "wider panel, more columns");
        assert!(columns(205.0, 8.0, 70.0).0 > columns(205.0, 8.0, 140.0).0, "S fits more than L");
    }
}
