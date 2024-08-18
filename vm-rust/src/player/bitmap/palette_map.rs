use fxhash::FxHashMap;

use crate::player::cast_member::PaletteMember;

struct PaletteEntry {
    number: u32,
    member: PaletteMember,
}

pub struct PaletteMap {
    palettes: FxHashMap<u32, PaletteEntry>,
}

impl PaletteMap {
    pub fn new() -> Self {
        Self {
            palettes: FxHashMap::default(),
        }
    }

    pub fn insert(&mut self, number: u32, palette: PaletteMember) {
        self.palettes.insert(number, PaletteEntry {
            number: number as u32,
            member: palette,
        });
    }

    pub fn get(&self, number: usize) -> Option<&PaletteMember> {
        self.palettes.get(&(number as u32)).map(|entry| &entry.member)
    }
}
