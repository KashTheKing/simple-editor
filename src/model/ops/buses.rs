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
}
