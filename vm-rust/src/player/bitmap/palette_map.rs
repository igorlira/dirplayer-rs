use rustc_hash::FxHashMap;

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

    /// Find a palette by cast library number and stale member reference.
    /// Used as fallback when a bitmap's clutId is out of range (stale from old numbering)
    /// but there's a valid palette in the same cast library.
    /// When multiple palettes exist, picks the one with the highest member number,
    /// since a higher stale clutId corresponds to a higher CAS* index (= higher member number).
    pub fn find_by_cast_lib(&self, cast_lib: u32) -> Option<&PaletteMember> {
        self.palettes.iter()
            .filter(|entry| (entry.number >> 16) == cast_lib)
            .max_by_key(|entry| entry.number & 0xFFFF)
            .map(|entry| &entry.member)
    }

    /// Search all palettes for one with a matching member number, ignoring cast lib.
    /// Used when cast_lib is 0 (unspecified).
    pub fn find_by_member(&self, cast_member: u32) -> Option<&PaletteMember> {
        for entry in &self.palettes {
            let entry_member = entry.number & 0xFFFF;
            if entry_member == cast_member {
                return Some(&entry.member);
            }
        }
        None
    }
}
