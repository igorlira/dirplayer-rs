use fxhash::FxHashMap;

use crate::player::cast_member::PaletteMember;

pub struct PaletteEntry {
    pub number: u32,
    pub member: PaletteMember,
}

pub struct PaletteMap {
    pub palettes: Vec<PaletteEntry>,
    lookup: FxHashMap<u32, usize>,
}

impl PaletteMap {
    pub fn new() -> Self {
        Self {
            palettes: Vec::new(),
            lookup: FxHashMap::default(),
        }
    }

    pub fn insert(&mut self, number: u32, palette: PaletteMember) {
        let index = self.palettes.len();
        self.palettes.push(PaletteEntry {
            number,
            member: palette,
        });
        self.lookup.insert(number, index);
    }

    #[inline]
    pub fn get(&self, number: usize) -> Option<&PaletteMember> {
        self.lookup
            .get(&(number as u32))
            .map(|&idx| &self.palettes[idx].member)
    }

    /// Get the first available custom palette in the movie.
    /// Used when a bitmap has palette_id=0 (meaning "use default palette").
    pub fn get_first(&self) -> Option<&PaletteMember> {
        self.palettes.first().map(|entry| &entry.member)
    }
}
