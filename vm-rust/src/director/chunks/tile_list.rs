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
        for _ in 0..8 {
            // 4 unused bytes
            let _unused = reader.read_u32().map_err(|e| format!("VWTL unused: {:?}", e))?;
            // castLib only present in D5+; pre-D5 uses the default (internal) cast.
            let cast_lib = if dir_version >= 500 {
                reader.read_u16().map_err(|e| format!("VWTL castLib: {:?}", e))? as i32
            } else {
                1
            };
            let member = reader.read_u16().map_err(|e| format!("VWTL member: {:?}", e))? as i32;
            // Mac rect order: top, left, bottom, right.
            let top = reader.read_u16().map_err(|e| format!("VWTL top: {:?}", e))? as i16;
            let left = reader.read_u16().map_err(|e| format!("VWTL left: {:?}", e))? as i16;
            let bottom = reader.read_u16().map_err(|e| format!("VWTL bottom: {:?}", e))? as i16;
            let right = reader.read_u16().map_err(|e| format!("VWTL right: {:?}", e))? as i16;
            tiles.push(TilePatternEntry { cast_lib, member, left, top, right, bottom });
        }
        Ok(TileListChunk { tiles })
    }
}
