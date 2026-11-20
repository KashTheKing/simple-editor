//! ---- feat/reference-clips ----
//! Project ops for reference clips (`model::RefSource`).

use crate::model::*;

impl Project {
    /// The reference record of `asset`, if it is one.
    pub fn reference(&self, asset: Id) -> Option<&RefSource> {
        self.references.iter().find(|r| r.asset == asset)
    }

    /// Every reference whose asset is still in the library, in import order.
    pub fn live_references(&self) -> Vec<&RefSource> {
        self.references.iter().filter(|r| self.asset(r.asset).is_some()).collect()
    }

    /// Mark `r.asset` as a reference (replacing its old record): tags it `reference` and makes the
    /// References smart bin the first time. Records of removed assets are dropped on the way.
    pub fn set_reference(&mut self, r: RefSource) {
        let id = r.asset;
        if self.asset(id).is_none() {
            return;
        }
        let live: Vec<Id> = self.assets.iter().map(|a| a.id).collect();
        self.references.retain(|x| x.asset != id && live.contains(&x.asset));
        self.references.push(r);
        if let Some(a) = self.asset_mut(id) {
            if !a.tags.iter().any(|t| t.eq_ignore_ascii_case(REFERENCE_TAG)) {
                a.tags.push(REFERENCE_TAG.to_string());
            }
        }
        if !self.smart_bins.iter().any(|b| b.name == REFERENCES_BIN) {
            self.smart_bins.push(SmartBin { name: REFERENCES_BIN.into(), query: format!("q:{REFERENCE_TAG}") });
        }
    }

    /// Set the transcribed words of a reference (no-op for an asset that is not one).
    pub fn set_reference_quote(&mut self, asset: Id, quote: &str) {
        if let Some(r) = self.references.iter_mut().find(|r| r.asset == asset) {
            r.quote = quote.trim().to_string();
        }
    }

    /// References whose quote, title or file name contains `q` (case-insensitive), best (quote) hits first.
    pub fn find_references(&self, q: &str) -> Vec<&RefSource> {
        let q = q.trim().to_lowercase();
        let mut hits: Vec<(u8, &RefSource)> = self
            .live_references()
            .into_iter()
            .filter_map(|r| {
                if q.is_empty() {
                    return Some((0, r));
                }
                let name = self.asset(r.asset).map(|a| a.name().to_lowercase()).unwrap_or_default();
                if r.quote.to_lowercase().contains(&q) {
                    Some((0, r))
                } else if r.title.to_lowercase().contains(&q) || name.contains(&q) {
                    Some((1, r))
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by_key(|h| h.0);
        hits.into_iter().map(|h| h.1).collect()
    }

    /// "Copy credits": one line per live reference (`engine::references::credit_line`).
    pub fn reference_credits(&self) -> String {
        self.live_references()
            .iter()
            .map(|r| crate::engine::references::credit_line(&r.title, &r.site, &r.credit, r.range, &r.url))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_asset() -> (Project, Id) {
        let mut p = Project::new();
        let id = p.add_asset(Asset {
            id: 0,
            path: "C:/clips/Shrug [tenor-1].mp4".into(),
            kind: ClipKind::Video,
            duration: 2.0,
            width: 480,
            height: 270,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        });
        (p, id)
    }

    #[test]
    fn set_reference_tags_and_makes_the_bin_once() {
        let (mut p, id) = with_asset();
        let r = RefSource { asset: id, site: "Tenor".into(), url: "https://tenor.com/view/x".into(), title: "Shrug".into(), ..Default::default() };
        p.set_reference(r.clone());
        p.set_reference(r);
        assert_eq!(p.references.len(), 1, "re-setting replaces");
        assert_eq!(p.asset(id).unwrap().tags, vec![REFERENCE_TAG.to_string()]);
        assert_eq!(p.smart_bins.iter().filter(|b| b.name == REFERENCES_BIN).count(), 1);
        p.set_reference(RefSource { asset: 999, ..Default::default() });
        assert_eq!(p.references.len(), 1, "no record for a missing asset");
    }

    #[test]
    fn found_by_quote_and_credited() {
        let (mut p, id) = with_asset();
        p.set_reference(RefSource {
            asset: id,
            site: "YouTube".into(),
            url: "https://youtu.be/abc".into(),
            title: "The scene".into(),
            credit: "A Channel".into(),
            range: Some((83.0, 105.0)),
            quote: String::new(),
        });
        assert!(p.find_references("not pass").is_empty());
        p.set_reference_quote(id, "  You shall not pass!  ");
        assert_eq!(p.find_references("SHALL NOT").len(), 1);
        assert_eq!(p.find_references("scene").len(), 1, "title matches too");
        assert_eq!(p.reference_credits(), "The scene - YouTube (A Channel), 1:23-1:45: https://youtu.be/abc");
        p.remove_asset(id);
        assert!(p.live_references().is_empty() && p.reference_credits().is_empty(), "removed assets drop out");
    }

    #[test]
    fn references_round_trip_and_old_files_load() {
        let (mut p, id) = with_asset();
        p.set_reference(RefSource { asset: id, site: "GIPHY".into(), ..Default::default() });
        let back: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.references, p.references);
        let empty = serde_json::to_string(&Project::new()).unwrap();
        assert!(!empty.contains("\"references\""), "an empty list is not written");
    }
}
