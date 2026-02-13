/// Physical Font Section Parser + Delta-encoded Character Records

use super::bit_reader::PfrBitReader;
use super::types::{PhysicalFontRecord, CharacterRecord};

/// Parse the physical font section
pub fn parse_physical_font(
    data: &[u8],
    phys_offset: usize,
    phys_end: usize,
    max_chars: u16,
) -> Result<PhysicalFontRecord, String> {
    if phys_offset >= data.len() {
        return Err("Physical font offset out of range".to_string());
    }
    let phys_end = phys_end.min(data.len());
    if phys_end <= phys_offset {
        return Err("Physical font end is not after offset".to_string());
    }

    let mut reader = PfrBitReader::from_offset(data, phys_offset);
    let mut record = PhysicalFontRecord::new();

    // FontRefNumber (u16, 2 bytes)
    let _font_ref_number = reader.read_u16();

    // Outline resolution (u16)
    record.outline_resolution = reader.read_u16();
    if record.outline_resolution == 0 {
        record.outline_resolution = 2048;
    }

    // Metrics resolution (u16)
    record.metrics_resolution = reader.read_u16();
    if record.metrics_resolution == 0 {
        record.metrics_resolution = record.outline_resolution;
    }

    // Bounding box (4 x i16)
    record.x_min = reader.read_i16();
    record.y_min = reader.read_i16();
    record.x_max = reader.read_i16();
    record.y_max = reader.read_i16();

    // 8 flag bits (1 byte)
    // Read as individual bits from MSB to LSB
    let extra_items_present = reader.read_bit();
    let _zero_bit = reader.read_bit();
    let _three_byte_gps_offset = reader.read_bit();
    let _two_byte_gps_size = reader.read_bit();
    let _ascii_code_specified = reader.read_bit();
    let proportional_escapement = reader.read_bit();
    let _two_byte_char_code = reader.read_bit();
    let _vertical_escapement = reader.read_bit();

    record.flags = 0;
    if proportional_escapement { record.flags |= 0x04; }

    // Standard set width when proportionalEscapement == false
    if !proportional_escapement {
        record.standard_set_width = reader.read_i16();
    }

    // Extra items
    if extra_items_present {
        let n_extra_items = reader.read_u8();
        for _ in 0..n_extra_items {
            if reader.remaining() < 2 {
                break;
            }

            let item_size = reader.read_u8() as usize;
            let item_type = reader.read_u8();

            let item_start = reader.position();

            match item_type {
                1 => {
                    // Bitmap section specification
                    let _font_bct_size = reader.read_i24();
                    let _zeros = reader.read_bits(3);
                    let two_byte_n_bmap_chars = reader.read_bit();
                    let three_byte_bct_offset = reader.read_bit();
                    let three_byte_bct_size = reader.read_bit();
                    let two_byte_yppm = reader.read_bit();
                    let two_byte_xppm = reader.read_bit();
                    let n_bitmap_sizes = reader.read_bit() as usize;

                    for _ in 0..n_bitmap_sizes {
                        let _xppm = if two_byte_xppm {
                            reader.read_u16() as u32
                        } else {
                            reader.read_u8() as u32
                        };

                        let _yppm = if two_byte_yppm {
                            reader.read_u16() as u32
                        } else {
                            reader.read_u8() as u32
                        };

                        let _zeros2 = reader.read_bits(5);
                        let _three_byte_gps_offset = reader.read_bit();
                        let _two_byte_gps_size = reader.read_bit();
                        let _two_byte_char_code = reader.read_bit();

                        let _bct_size = if three_byte_bct_size {
                            reader.read_u24()
                        } else {
                            reader.read_u16() as u32
                        };

                        let bct_offset = if three_byte_bct_offset {
                            reader.read_u24()
                        } else {
                            reader.read_u16() as u32
                        };

                        let _n_bmap_chars = if two_byte_n_bmap_chars {
                            reader.read_u16() as u32
                        } else {
                            reader.read_u8() as u32
                        };

                        record.bitmap_size_table_offset = bct_offset;
                    }

                    record.has_bitmap_section = true;
                }
                2 => {
                    // FontID (null-terminated string)
                    if item_size > 0 && item_start + item_size <= data.len() {
                        let mut font_id_bytes = Vec::new();
                        for _ in 0..item_size {
                            let ch = reader.read_u8();
                            if ch == 0 { break; }
                            font_id_bytes.push(ch);
                        }
                        record.font_id = String::from_utf8_lossy(&font_id_bytes).to_string();
                    }
                }
                3 => {
                    // Stem snap tables
                    // Read as nibbles: sshSize (4 bits) + ssvSize (4 bits)
                    // Then ssvSize i16 values + sshSize i16 values
                    let ssh_size = reader.read_bits(4) as usize;
                    let ssv_size = reader.read_bits(4) as usize;
                    for _ in 0..ssv_size {
                        reader.read_i16();
                    }
                    for _ in 0..ssh_size {
                        reader.read_i16();
                    }
                }
                _ => {
                    // Unknown type, skip
                    reader.skip(item_size);
                }
            }

        }
    }

    // nAuxBytes (24-bit)
    let n_aux_bytes = reader.read_u24() as usize;
    if n_aux_bytes > 0 && n_aux_bytes < 10000 {
        // Normal case: consume aux data payload.
        reader.skip(n_aux_bytes);
    } else if n_aux_bytes >= 10000 {
        let _start_pos = reader.position();

        while reader.position() != phys_end {
            let probe_pos = reader.position();

            let n_blue_values = reader.read_u8() as usize;
            let byte_counter = (n_blue_values * 2) + 6;

            // Need room to skip and read 16 bits
            let n_chars_pos = reader.position() + byte_counter;
            if n_chars_pos + 2 > phys_end {
                // not enough room, slide window
                reader.set_position(probe_pos + 1);
                continue;
            }

            reader.set_position(n_chars_pos);
            let n_characters = reader.read_u16();

            if n_characters == max_chars {
                reader.set_position(probe_pos);

                // Found the "final" marker
                break;
            }

            // No match -> slide forward by 1 byte
            reader.set_position(probe_pos + 1);
        }
    }

    // Blue values
    let n_blue_values = reader.read_u8() as usize;
    let mut blue_values = Vec::with_capacity(n_blue_values);
    for _ in 0..n_blue_values {
        blue_values.push(reader.read_i16());
    }
    let blue_fuzz = reader.read_u8();
    let blue_scale = reader.read_u8();
    record.blue_values = blue_values;
    record.blue_fuzz = blue_fuzz;
    record.blue_scale = blue_scale;

    // StdVW and StdHW
    record.metrics.std_vw = reader.read_u16() as i16;
    record.metrics.std_hw = reader.read_u16() as i16;

    // Initialize metrics from bounding box
    record.metrics.x_min = record.x_min;
    record.metrics.y_min = record.y_min;
    record.metrics.x_max = record.x_max;
    record.metrics.y_max = record.y_max;
    record.metrics.units_per_em = record.outline_resolution;
    record.metrics.ascender = record.y_max;
    record.metrics.descender = record.y_min;

    // Number of characters (u16)
    let n_characters = reader.read_u16() as usize;

    // PFR1 delta-encoded character records
    parse_character_records_pfr1(&mut reader, &mut record, n_characters, proportional_escapement);

    Ok(record)
}

/// Parse PFR1 delta-encoded character records
fn parse_character_records_pfr1(
    reader: &mut PfrBitReader,
    record: &mut PhysicalFontRecord,
    n_characters: usize,
    _proportional: bool,
) {
    if n_characters == 0 {
        return;
    }

    // Delta state, charCode starts at -1
    let mut char_code: i32 = -1;
    let mut set_width: i32 = record.standard_set_width as i32;
    let mut gps_size: i32 = 0;
    let mut gps_offset: i32 = 0;

    for _i in 0..n_characters {
        if reader.remaining() < 1 {
            break;
        }

        // Read flag byte
        let flags = reader.read_u8();

        // Calculate next gps_offset BEFORE reading deltas (previous offset + previous size)
        let next_gps_offset = gps_offset + gps_size;

        // bits 0-1: char code delta
        // charCode always incremented by 1 first
        let char_code_mode = flags & 0x03;
        char_code += 1; // unconditional +1
        match char_code_mode {
            0 => {} // no further change
            1 => {
                char_code += reader.read_u8() as i32;
            }
            2 => {
                char_code += reader.read_u16() as i32;
            }
            _ => {} // mode 3 treated same as 0 for charCode
        }

        // bits 2-3: set width
        let set_width_mode = (flags >> 2) & 0x03;
        match set_width_mode {
            0 => {} // unchanged
            1 => {
                set_width += reader.read_u8() as i32;
            }
            2 => {
                set_width -= reader.read_u8() as i32;
            }
            3 => {
                set_width = reader.read_i16() as i32;
            }
            _ => {}
        }

        // bits 4-5: gps size
        let gps_size_mode = (flags >> 4) & 0x03;
        match gps_size_mode {
            0 => {
                gps_size = reader.read_u8() as i32;
            }
            1 => {
                gps_size = reader.read_u8() as i32 + 256;
            }
            2 => {
                gps_size = reader.read_u8() as i32 + 512;
            }
            3 => {
                gps_size = reader.read_u16() as i32;
            }
            _ => {}
        }

        // bits 6-7: gps offset
        let gps_offset_mode = (flags >> 6) & 0x03;
        match gps_offset_mode {
            0 => {
                // Sequential: use calculated next offset
                gps_offset = next_gps_offset;
            }
            1 => {
                // Delta from calculated next offset
                gps_offset = next_gps_offset + reader.read_u8() as i32;
            }
            2 => {
                // Absolute 16-bit offset
                gps_offset = reader.read_u16() as i32;
            }
            3 => {
                // Absolute 24-bit offset
                gps_offset = reader.read_u24() as i32;
            }
            _ => {}
        }

        record.char_records.push(CharacterRecord {
            char_code: char_code as u32,
            set_width: set_width as u16,
            gps_size: gps_size as u32,
            gps_offset: gps_offset as u32,
        });
    }
}

/// Initialize font-level stroke tables from physical font bounding box
pub fn initialize_stroke_tables_fallback(record: &mut PhysicalFontRecord) {
    let x_min = record.x_min as i32;
    let x_max = record.x_max as i32;
    let y_min = record.y_min as i32;
    let y_max = record.y_max as i32;

    record.stroke_x_count = 8;
    record.stroke_y_count = 8;

    record.stroke_x_keys = vec![
        -1,
        0,
        (x_max / 6) as i16,
        (x_max / 3) as i16,
        (x_max / 2) as i16,
        (2 * x_max / 3) as i16,
        (5 * x_max / 6) as i16,
        x_max as i16,
    ];

    record.stroke_y_keys = vec![
        -1,
        y_min as i16,
        (y_min + (y_max - y_min) / 6) as i16,
        (y_min + (y_max - y_min) / 3) as i16,
        (y_min + (y_max - y_min) / 2) as i16,
        (y_min + 2 * (y_max - y_min) / 3) as i16,
        (y_min + 5 * (y_max - y_min) / 6) as i16,
        y_max as i16,
    ];

    record.stroke_x_scales = vec![256; record.stroke_x_count as usize];
    record.stroke_y_scales = vec![256; record.stroke_y_count as usize];

    let shift = 12;
    record.stroke_x_values = vec![
        0,
        0 << shift,
        (x_max / 6) << shift,
        (x_max / 3) << shift,
        (x_max / 2) << shift,
        (2 * x_max / 3) << shift,
        (5 * x_max / 6) << shift,
        x_max << shift,
    ];

    record.stroke_y_values = vec![
        0,
        y_min << shift,
        (y_min + (y_max - y_min) / 6) << shift,
        (y_min + (y_max - y_min) / 3) << shift,
        (y_min + (y_max - y_min) / 2) << shift,
        (y_min + 2 * (y_max - y_min) / 3) << shift,
        (y_min + 5 * (y_max - y_min) / 6) << shift,
        y_max << shift,
    ];

    record.stroke_tables_initialized = true;
}
