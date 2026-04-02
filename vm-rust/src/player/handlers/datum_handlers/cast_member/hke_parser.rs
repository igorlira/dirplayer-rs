/// Parser for the Havok HKE binary format (collision geometry).
///
/// Binary layout (little-endian):
///
/// FILE HEADER:
///   [4 bytes]  Magic/checksum
///   [variable] Header fields until first entry marker
///
/// ENTRY MARKER (6 bytes, before every collision mesh):
///   A9 EE 9F 01 45 30
///
/// ENTRY DATA:
///   [2 bytes]   Type (u16 LE, e.g. 0x0505)
///   [N+1 bytes] Null-terminated model name (ASCII)
///   [4 bytes]   Vertex count (u32 LE)
///   [count*12]  Vertices (3 × f32 LE per vertex: x, y, z)
///   [4 bytes]   Triangle count (u32 LE)
///   [count*12]  Triangle indices (3 × u32 LE per triangle)
///
/// ENTRY SEPARATOR (8 bytes, between entries):
///   EF CD AB 12 C9 0A A3 0E

pub struct HkeCollisionMesh {
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

pub struct HkeWorld {
    pub meshes: Vec<HkeCollisionMesh>,
}

const ENTRY_MARKER: [u8; 6] = [0xA9, 0xEE, 0x9F, 0x01, 0x45, 0x30];

pub fn parse_hke(data: &[u8]) -> HkeWorld {
    let mut meshes = Vec::new();
    let mut search_pos = 0;

    while search_pos < data.len().saturating_sub(ENTRY_MARKER.len()) {
        let marker_pos = match find_bytes(data, &ENTRY_MARKER, search_pos) {
            Some(pos) => pos,
            None => break,
        };

        if let Some(mesh) = parse_entry(data, marker_pos) {
            meshes.push(mesh);
        }

        search_pos = marker_pos + ENTRY_MARKER.len();
    }

    log::info!("HKE parsed: {} collision meshes", meshes.len());
    HkeWorld { meshes }
}

fn parse_entry(data: &[u8], marker_pos: usize) -> Option<HkeCollisionMesh> {
    let mut pos = marker_pos + ENTRY_MARKER.len();

    // Skip type (2 bytes)
    if pos + 2 > data.len() { return None; }
    pos += 2;

    // Read null-terminated name
    let name_end = data[pos..].iter().position(|&b| b == 0)?;
    if name_end == 0 || name_end > 100 { return None; }
    let name = std::str::from_utf8(&data[pos..pos + name_end]).ok()?;
    if !name.bytes().all(|b| b >= 0x20 && b < 0x7F) { return None; }
    pos += name_end + 1; // skip null terminator

    // Read vertex count (u32 LE)
    if pos + 4 > data.len() { return None; }
    let vert_count = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
    pos += 4;
    if vert_count > 100_000 { return None; }

    // Read vertices
    let vert_bytes = vert_count * 12;
    if pos + vert_bytes > data.len() { return None; }
    let mut vertices = Vec::with_capacity(vert_count);
    for i in 0..vert_count {
        let off = pos + i * 12;
        let x = f32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        let y = f32::from_le_bytes(data[off + 4..off + 8].try_into().ok()?);
        let z = f32::from_le_bytes(data[off + 8..off + 12].try_into().ok()?);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() { return None; }
        vertices.push([x, y, z]);
    }
    pos += vert_bytes;

    // Read triangle count (u32 LE)
    if pos + 4 > data.len() { return None; }
    let tri_count = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
    pos += 4;
    if tri_count > 500_000 { return None; }

    // Read triangle indices
    let idx_bytes = tri_count * 12;
    if pos + idx_bytes > data.len() { return None; }
    let mut triangles = Vec::with_capacity(tri_count);
    for i in 0..tri_count {
        let off = pos + i * 12;
        let a = u32::from_le_bytes(data[off..off + 4].try_into().ok()?);
        let b = u32::from_le_bytes(data[off + 4..off + 8].try_into().ok()?);
        let c = u32::from_le_bytes(data[off + 8..off + 12].try_into().ok()?);
        if a as usize >= vert_count || b as usize >= vert_count || c as usize >= vert_count {
            return None; // invalid index
        }
        triangles.push([a, b, c]);
    }

    Some(HkeCollisionMesh {
        name: name.to_string(),
        vertices,
        triangles,
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() { return None; }
    for i in start..=haystack.len() - needle.len() {
        if haystack[i..i + needle.len()] == *needle {
            return Some(i);
        }
    }
    None
}
