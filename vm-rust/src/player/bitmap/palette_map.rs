use crate::player::cast_member::PaletteMember;

pub struct PaletteEntry {
    pub number: u32,
    pub member: PaletteMember,
}

pub struct PaletteMap {
    pub palettes: Vec<PaletteEntry>,
}

impl PaletteMap {
    pub fn new() -> Self {
        Self {
            palettes: Vec::new(),
        }
    }

    pub fn insert(&mut self, number: u32, palette: PaletteMember) {
        self.palettes.push(PaletteEntry {
            number: number as u32,
            member: palette,
        });
    }

    pub fn get(&self, number: usize) -> Option<&PaletteMember> {
        self.palettes
            .iter()
            .find(|entry| entry.number == number as u32)
            .map(|entry| &entry.member)
    }

    /// Get the first available custom palette in the movie.
    /// Used when a bitmap has palette_id=0 (meaning "use default palette").
    pub fn get_first(&self) -> Option<&PaletteMember> {
        self.palettes.first().map(|entry| &entry.member)
    }
}
