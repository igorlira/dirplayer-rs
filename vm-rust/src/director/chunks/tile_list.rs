use binary_reader::{BinaryReader, Endian};

/// One entry of the VWTL "tile list" — Director's 8 user-definable tile patterns
/// (shape `pattern` values 57-64). A non-zero `member` overrides the built-in
/// tile with a rectangular region of that bitmap cast member, tiled across the
/// shape. Mirrors ScummVM `Cast::loadVWTL` (engines/director/cast.cpp).
#[derive(Clone, Copy, Debug, Default)]
pub struct TilePatternEntry {
    pub cast_lib: i32,
    /// Bitmap cast member number; 0 = no custom tile (use the built-in).
    pub member: i32,
    /// Source region within the bitmap member (left, top, right, bottom).
    pub left: i16,
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
}

impl TilePatternEntry {
    pub fn is_custom(&self) -> bool {
        self.member != 0 && self.right > self.left && self.bottom > self.top
    }
}

pub struct TileListChunk {
    /// Always 8 entries (kNumBuiltinTiles).
    pub tiles: Vec<TilePatternEntry>,
}

impl TileListChunk {
    pub fn from_reader(reader: &mut BinaryReader, dir_version: u16) -> Result<TileListChunk, String> {
        reader.set_endian(Endian::Big);
        let mut tiles = Vec::with_capacity(8);
        // Director only writes entries for the tiles the movie actually defines —
        // NabiscoWorld Mini-Golf ships a 16-byte VWTL holding tile 1 alone. Reading
        // a fixed 8 entries made the short chunk fail to parse, which dropped the
        // WHOLE list and sent every pattern 57-64 to the built-in tile fallback
        // (the grass background rendered as the blue-grey built-in tile 1).
        // Stop at whatever the chunk holds and leave the rest undefined.
        for _ in 0..8 {
            // 4 unused bytes
            let Ok(_unused) = reader.read_u32() else { break };
            // castLib only present in D5+; pre-D5 uses the default (internal) cast.
            let cast_lib = if dir_version >= 500 {
                let Ok(v) = reader.read_u16() else { break };
                v as i32
            } else {
                1
            };
            let Ok(member) = reader.read_u16() else { break };
            // Mac rect order: top, left, bottom, right.
            let Ok(top) = reader.read_u16() else { break };
            let Ok(left) = reader.read_u16() else { break };
            let Ok(bottom) = reader.read_u16() else { break };
            let Ok(right) = reader.read_u16() else { break };
            tiles.push(TilePatternEntry {
                cast_lib,
                member: member as i32,
                left: left as i16,
                top: top as i16,
                right: right as i16,
                bottom: bottom as i16,
            });
        }
        // Pad to 8 so `custom_tiles[pattern - 57]` stays index-aligned; a default
        // entry has member 0, so `is_custom()` is false and the built-in tile wins.
        tiles.resize(8, TilePatternEntry::default());
        Ok(TileListChunk { tiles })
    }
}
