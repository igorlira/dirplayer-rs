/// Shockwave 3D (W3D / IFX) file format parser.
///
/// Parses the IFX container format used by Macromedia/Adobe Shockwave 3D,
/// including CLOD (Continuous Level of Detail) compressed progressive meshes,
/// materials, textures, shaders, scene graph nodes, lights, skeletons, and animations.

pub mod types;
pub mod block_types;
pub mod block_reader;
pub mod bitstream;
pub mod clod_types;
pub mod clod_decoder;
pub mod primitives;
pub mod parser;
pub mod raycast;
pub mod skeleton;

pub use types::W3dScene;

/// Parse W3D data (IFX container) and return the scene.
pub fn parse_w3d(data: &[u8]) -> Result<W3dScene, String> {
    let actual_data = find_ifx_start(data)
        .ok_or_else(|| format!(
            "No IFX magic found in {} bytes (first bytes: {:02X?})",
            data.len(),
            &data[..data.len().min(16)]
        ))?;

    let mut parser = parser::W3dFileParser::new(actual_data.to_vec());
    parser.parse()?;
    Ok(parser.scene)
}

/// Return the byte offset where IFX magic starts, or None.
pub fn find_ifx_start_offset(data: &[u8]) -> Option<usize> {
    let ifx_magic = [0x49u8, 0x46, 0x58, 0x00];
    for offset in 0..data.len().min(256) {
        if offset + 4 <= data.len() && data[offset..offset + 4] == ifx_magic {
            return Some(offset);
        }
    }
    None
}

/// Find the start of the IFX container within potentially wrapped data.
fn find_ifx_start(data: &[u8]) -> Option<&[u8]> {
    let ifx_magic = [0x49u8, 0x46, 0x58, 0x00]; // "IFX\0"

    // Check bare IFX
    if data.len() >= 4 && data[0..4] == ifx_magic {
        return Some(data);
    }

    // Check after "3DEM" (4 bytes)
    if data.len() >= 8 && &data[0..4] == b"3DEM" {
        // Try right after 3DEM
        if data[4..8] == ifx_magic {
            return Some(&data[4..]);
        }

        // 3DEM may be followed by a size field (4 bytes BE) then IFX
        if data.len() >= 12 && data[8..12] == ifx_magic {
            return Some(&data[8..]);
        }

        // Scan within first 64 bytes after 3DEM for IFX magic
        for offset in 4..data.len().min(64) {
            if offset + 4 <= data.len() && data[offset..offset + 4] == ifx_magic {
                return Some(&data[offset..]);
            }
        }
    }

    // Generic scan: search for IFX\0 within first 256 bytes
    for offset in 0..data.len().min(256) {
        if offset + 4 <= data.len() && data[offset..offset + 4] == ifx_magic {
            return Some(&data[offset..]);
        }
    }

    None
}
