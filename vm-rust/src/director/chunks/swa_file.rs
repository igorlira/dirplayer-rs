use log::debug;

/// Parsed external SWA (.swa) file.
/// SWA files are MP3 audio with a Director-specific header containing
/// metadata and optional cue points.
pub struct SwaFile {
    pub sample_rate: u32,
    pub num_frames: u32,
    pub channels: u16,
    pub copyright_info: String,
    pub cue_point_names: Vec<String>,
    pub cue_point_times: Vec<u32>,
    pub mp3_data: Vec<u8>,
}

impl SwaFile {
    /// Parse an external SWA file from raw bytes.
    ///
    /// Header layout (all big-endian):
    /// 0x00: u32 header_size
    /// 0x04: u32 version
    /// 0x08: u32 sample_rate
    /// 0x0C: u32 sample_rate (dup)
    /// 0x10: u32 num_frames
    /// 0x14: u32 uncompressed_data_size
    /// 0x18: i32 loop_start (-1 = none)
    /// 0x1C: i32 loop_end (-1 = none)
    /// 0x20: u16 channels
    /// 0x22: u16 unknown
    /// 0x24: 4B  "MACR" marker
    /// 0x28: 16B Macromedia GUID
    /// ...   variable metadata + cue points
    /// MP3 data starts at header_size + 4
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        if data.len() < 0x38 {
            return Err(format!("SWA file too small: {} bytes", data.len()));
        }

        let header_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let version = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        if version != 3 {
            return Err(format!(
                "SWA version {} not yet supported",
                version
            ));
        }
        let sample_rate = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let num_frames = u32::from_be_bytes([data[0x10], data[0x11], data[0x12], data[0x13]]);
        let channels = u16::from_be_bytes([data[0x20], data[0x21]]);

        // Parse copyright string at offset 0x40 (starts with © = 0xA9)
        let copyright_info = if data.len() > 0x41 {
            let start = if data[0x40] == 0xA9 { 0x41 } else { 0x40 };
            let end = data[start..header_size].iter()
                .position(|&b| b == 0)
                .map(|p| start + p)
                .unwrap_or(header_size.min(start + 64));
            String::from_utf8_lossy(&data[start..end]).to_string()
        } else {
            String::new()
        };

        debug!(
            "SWA file: header_size={}, sample_rate={}, num_frames={}, channels={}, copyright='{}'",
            header_size, sample_rate, num_frames, channels, copyright_info
        );

        // Parse cue points from the header
        // Cue point block is near the end of the header.
        // We scan for it by looking for the cue count + entries pattern.
        let (cue_point_names, cue_point_times) = Self::parse_cue_points(data, header_size);

        // MP3 data starts at header_size + 4 (4 bytes padding)
        let mp3_offset = header_size + 4;
        if mp3_offset >= data.len() {
            return Err(format!(
                "SWA MP3 offset {} beyond file size {}",
                mp3_offset,
                data.len()
            ));
        }

        // Scan for actual MP3 sync from mp3_offset
        let mp3_start = Self::find_mp3_start(data, mp3_offset)
            .unwrap_or(mp3_offset);

        let mp3_data = data[mp3_start..].to_vec();

        debug!(
            "SWA file: {} cue points, MP3 data {} bytes starting at offset {}",
            cue_point_names.len(),
            mp3_data.len(),
            mp3_start
        );

        Ok(SwaFile {
            sample_rate,
            num_frames,
            channels,
            copyright_info,
            cue_point_names,
            cue_point_times,
            mp3_data,
        })
    }

    /// Find the start of MP3 data by scanning for a sync word.
    fn find_mp3_start(data: &[u8], from: usize) -> Option<usize> {
        for i in from..data.len().saturating_sub(1) {
            if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 && data[i + 1] != 0xFF {
                return Some(i);
            }
        }
        None
    }

    /// Parse cue points from the SWA header.
    /// The cue point block starts with a u32 BE count field somewhere
    /// before header_size, followed by entries of [u32 time_ms + char[32] name].
    fn parse_cue_points(data: &[u8], header_size: usize) -> (Vec<String>, Vec<u32>) {
        let mut names = Vec::new();
        let mut times = Vec::new();

        // The cue point count is typically found by scanning backwards from
        // the header end. Each entry is 36 bytes (4 time + 32 name).
        // We look for a plausible count value where count * 36 + 4 fits
        // within the remaining header space.

        // Try scanning from fixed known offsets first
        // In the real SWA files analyzed, cue points start at a variable offset
        // that depends on the header content. We scan for plausible locations.
        let min_cue_offset = 0x38; // After the fixed header fields
        if header_size <= min_cue_offset + 4 {
            return (names, times);
        }

        // Scan potential count positions
        for offset in (min_cue_offset..header_size.saturating_sub(4)).rev() {
            if offset + 4 > header_size {
                continue;
            }
            let count = u32::from_be_bytes([
                data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
            ]) as usize;

            // Validate: count must be reasonable, and entries must fit exactly
            if count == 0 || count > 100 {
                continue;
            }
            let entries_size = count * 36;
            let block_end = offset + 4 + entries_size;

            // The entries block should end at or very near header_size
            if block_end > header_size || header_size - block_end > 4 {
                continue;
            }

            // Validate that the entries look like cue points
            // (names should contain printable ASCII or nulls)
            let mut valid = true;
            let mut entry_offset = offset + 4;
            for _ in 0..count {
                if entry_offset + 36 > data.len() {
                    valid = false;
                    break;
                }
                // Check name bytes (offset + 4 to offset + 36) are printable or null
                let name_bytes = &data[entry_offset + 4..entry_offset + 36];
                let null_pos = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
                for &b in &name_bytes[..null_pos] {
                    if b < 0x20 || b > 0x7E {
                        valid = false;
                        break;
                    }
                }
                if !valid {
                    break;
                }
                entry_offset += 36;
            }

            if !valid {
                continue;
            }

            // Parse the cue points
            entry_offset = offset + 4;
            for _ in 0..count {
                let time = u32::from_be_bytes([
                    data[entry_offset], data[entry_offset + 1],
                    data[entry_offset + 2], data[entry_offset + 3],
                ]);
                times.push(time);

                let name_bytes = &data[entry_offset + 4..entry_offset + 36];
                let null_pos = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
                let name = String::from_utf8_lossy(&name_bytes[..null_pos]).to_string();
                names.push(name);

                entry_offset += 36;
            }

            debug!("SWA: found {} cue points at header offset 0x{:04X}", count, offset);
            break;
        }

        (names, times)
    }
}
