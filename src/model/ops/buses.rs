use crate::model::*;

impl Project {
    // ---------- buses ----------
    /// The Main bus id, creating the default bus set on first use.
    pub fn main_bus(&mut self) -> Id {
        if self.buses.is_empty() {
            let id = self.new_id();
            self.buses.push(Bus { id, name: "Main".into(), output: 0, ..Default::default() });
        }
        self.buses[0].id
    }
    pub fn bus(&self, id: Id) -> Option<&Bus> {
        self.buses.iter().find(|b| b.id == id)
    }
    pub fn bus_mut(&mut self, id: Id) -> Option<&mut Bus> {
        self.buses.iter_mut().find(|b| b.id == id)
    }
    pub fn add_bus(&mut self, name: impl Into<String>) -> Id {
        let main = self.main_bus();
        let id = self.new_id();
        self.buses.push(Bus { id, name: name.into(), output: main, ..Default::default() });
        id
    }
    /// Remove a bus (never Main); tracks/clips and sends fall back to Main.
    pub fn remove_bus(&mut self, id: Id) {
        let main = self.main_bus();
        if id == main {
            return;
        }
        self.buses.retain(|b| b.id != id);
        for b in &mut self.buses {
            if b.output == id {
                b.output = main;
            }
        }
        for t in &mut self.tracks {
            if t.bus == id {
                t.bus = 0;
            }
            for c in &mut t.clips {
                if c.bus == id {
                    c.bus = 0;
                }
            }
        }
    }
    /// Which bus a clip feeds: its own override, else its track's, else Main.
    pub fn bus_of(&self, track: usize, clip: &Clip) -> Id {
        let main = self.buses.first().map(|b| b.id).unwrap_or(0);
        if clip.bus != 0 && self.bus(clip.bus).is_some() {
            return clip.bus;
        }
        let t = self.tracks.get(track).map(|t| t.bus).unwrap_or(0);
        if t != 0 && self.bus(t).is_some() {
            t
        } else {
            main
        }
    }

    // ---- ws:audio-dsp-automation ----
    /// Essential-Sound "Repair"/"Clarity": route `ids` through a bus carrying that preset's filter
    /// chain (`mixer_fx::REPAIR_PRESETS`), creating the bus on first use and reusing it (by label)
    /// after — the chain stays a plain, editable Mixer bus. Returns the bus id (0 = unknown preset,
    /// nothing changed) so the inspector can pre-select it in the Mixer.
    pub fn apply_repair(&mut self, ids: &[Id], preset: &str) -> Id {
        let Some((label, chain)) = crate::engine::mixer_fx::repair_chain(preset) else { return 0 };
        let bus = match self.buses.iter().find(|b| b.name == label).map(|b| b.id) {
            Some(id) => id,
            None => {
                let id = self.add_bus(label);
                self.bus_mut(id).expect("just added").filters = chain;
                id
            }
        };
        for &id in ids {
            if let Some(c) = self.clip_mut(id) {
                c.bus = bus;
            }
        }
        bus
    }
}

// ---- ws:audio-dsp-automation ----
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_repair_creates_bus_and_routes_clips() {
        let mut p = Project::new();
        let ai = p.audio_tracks()[0];
        p.tracks[ai].clips.push(Clip::new(7, ClipKind::Audio, "a", 0.0, 4.0));
        p.tracks[ai].clips.push(Clip::new(8, ClipKind::Audio, "b", 4.0, 4.0));
        assert!(p.buses.is_empty());
        let bus = p.apply_repair(&[7], "repair");
        assert_ne!(bus, 0);
        assert_eq!(p.buses.len(), 2, "Main + the repair bus");
        let b = p.bus(bus).unwrap();
        assert_eq!(b.name, "Repair");
        assert_eq!(b.output, p.buses[0].id, "sends to Main");
        let kinds: Vec<FilterKind> = b.filters.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            [FilterKind::HighPass, FilterKind::DeHum, FilterKind::NoiseGate, FilterKind::Compressor, FilterKind::Limiter]
        );
        assert_eq!(p.clip(7).unwrap().bus, bus);
        assert_eq!(p.clip(8).unwrap().bus, 0, "untouched clip keeps its track default");
        assert_eq!(p.bus_of(ai, p.clip(7).unwrap()), bus);
        // re-applying the same preset reuses the bus (no second "Repair" strip)
        let again = p.apply_repair(&[8], "repair");
        assert_eq!(again, bus);
        assert_eq!(p.buses.len(), 2);
        assert_eq!(p.clip(8).unwrap().bus, bus);
        // a different preset gets its own bus; an unknown one is a no-op
        let clarity = p.apply_repair(&[7], "clarity");
        assert_ne!(clarity, bus);
        assert_eq!(p.buses.len(), 3);
        assert_eq!(p.bus(clarity).unwrap().name, "Clarity");
        assert_eq!(p.clip(7).unwrap().bus, clarity);
        let before = p.to_json();
        assert_eq!(p.apply_repair(&[7], "nope"), 0);
        assert_eq!(p.to_json(), before);
    }
}
