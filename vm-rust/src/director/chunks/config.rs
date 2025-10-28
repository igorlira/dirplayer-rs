use crate::{
    director::utils::{human_version, FOURCC},
    utils::log_i,
};
use binary_reader::{BinaryReader, Endian};

pub struct ConfigChunk {
    /*  0 */ pub len: u16,
    /*  2 */ pub file_version: u16,
    /*  4 */ pub movie_top: u16,
    /*  6 */ pub movie_left: u16,
    /*  8 */ pub movie_bottom: u16,
    /* 10 */ pub movie_right: u16,
    /* 12 */ pub min_member: u16,
    /* 14 */ pub max_member: u16,
    /* 16 */ pub field9: u8,
    /* 17 */ pub field10: u8,

    // Director 6 and below
    /* 18 */ pub pre_d77field11: u16,
    // Director 7 and above
    /* 18 */ pub d7_stage_color_g: u8,
    /* 19 */ pub d7_stage_color_b: u8,

    /* 20 */ pub comment_font: u16,
    /* 22 */ pub comment_size: u16,
    /* 24 */ pub comment_style: u16,

    // Director 6 and below
    /* 26 */ pub pre_d7_stage_color: u16,
    // Director 7 and above
    /* 26 */ pub d7_stage_color_is_rgb: u8,
    /* 27 */ pub d7_stage_color_r: u8,

    /* 28 */ pub bit_depth: u16,
    /* 30 */ pub field17: u8,
    /* 31 */ pub field18: u8,
    /* 32 */ pub field19: u32,
    /* 36 */ pub director_version: u16,
    /* 38 */ pub field21: u16,
    /* 40 */ pub field22: u32,
    /* 44 */ pub field23: u32,
    /* 48 */ pub field24: u32,
    /* 52 */ pub field25: u8,
    /* 53 */ pub field26: u8,
    /* 54 */ pub frame_rate: u16,
    /* 56 */ pub platform: u16,
    /* 58 */ pub protection: u16,
    /* 60 */ pub field29: u32,
    /* 64 */ pub checksum: u32,
    /* 68 */ pub remnants: Vec<u8>,
}

impl ConfigChunk {
    pub fn from_reader(
        reader: &mut BinaryReader,
        _dir_version: u16,
        dir_endian: Endian,
    ) -> Result<ConfigChunk, String> {
        reader.set_endian(binary_reader::Endian::Big);
        reader.jmp(36);

        let raw_version = reader.read_u16().unwrap();
        let dir_version = human_version(raw_version);

        reader.jmp(0);

        let len = reader.read_u16().unwrap();
        let file_version = reader.read_u16().unwrap();
        let movie_top = reader.read_u16().unwrap();
        let movie_left = reader.read_u16().unwrap();
        let movie_bottom = reader.read_u16().unwrap();
        let movie_right = reader.read_u16().unwrap();
        let min_member = reader.read_u16().unwrap();
        let max_member = reader.read_u16().unwrap();
        let field9 = reader.read_u8().unwrap();
        let field10 = reader.read_u8().unwrap();
        let mut pre_d7_field11 = 0;
        let mut d7_stage_color_r: u8 = 0;
        let mut d7_stage_color_g: u8 = 0;
        let mut d7_stage_color_b: u8 = 0;
        if dir_version < 700 {
            pre_d7_field11 = reader.read_u16().unwrap();
        } else {
            d7_stage_color_g = reader.read_u8().unwrap();
            d7_stage_color_b = reader.read_u8().unwrap();
        }
        let comment_font = reader.read_u16().unwrap();
        let comment_size = reader.read_u16().unwrap();
        let comment_style = reader.read_u16().unwrap();
        let mut pre_d7_stage_color: u16 = 0;
        let mut d7_stage_color_is_rgb: u8 = 0;
        if dir_version < 700 {
            pre_d7_stage_color = reader.read_u16().unwrap();
        } else {
            d7_stage_color_is_rgb = reader.read_u8().unwrap();
            d7_stage_color_r = reader.read_u8().unwrap();
        }
        let bit_depth = reader.read_u16().unwrap();
        let field17 = reader.read_u8().unwrap();
        let field18 = reader.read_u8().unwrap();
        let field19 = reader.read_u32().unwrap();
        /* directorVersion = */
        reader.read_u16().unwrap();
        let field21 = reader.read_u16().unwrap();
        let field22 = reader.read_u32().unwrap();
        let field23 = reader.read_u32().unwrap();
        let field24 = reader.read_u32().unwrap();
        let field25 = reader.read_u8().unwrap();
        let field26 = reader.read_u8().unwrap();
        let frame_rate = reader.read_u16().unwrap();
        let platform = reader.read_u16().unwrap();
        let protection = reader.read_u16().unwrap();
        let field29 = reader.read_u32().unwrap();
        let checksum = reader.read_u32().unwrap();
        let remnants = reader.read_bytes(len as usize - reader.pos).unwrap();

        let config = ConfigChunk {
            len: len,
            file_version: file_version,
            movie_top: movie_top,
            movie_left: movie_left,
            movie_bottom: movie_bottom,
            movie_right: movie_right,
            min_member: min_member,
            max_member: max_member,
            field9: field9,
            field10: field10,
            pre_d77field11: pre_d7_field11,
            d7_stage_color_g: d7_stage_color_g,
            d7_stage_color_b: d7_stage_color_b,
            comment_font: comment_font,
            comment_size: comment_size,
            comment_style: comment_style,
            pre_d7_stage_color: pre_d7_stage_color,
            d7_stage_color_is_rgb: d7_stage_color_is_rgb,
            d7_stage_color_r: d7_stage_color_r,
            bit_depth: bit_depth,
            field17: field17,
            field18: field18,
            field19: field19,
            director_version: raw_version,
            field21: field21,
            field22: field22,
            field23: field23,
            field24: field24,
            field25: field25,
            field26: field26,
            frame_rate: frame_rate,
            platform: platform,
            protection: protection,
            field29: field29,
            checksum: checksum,
            remnants: remnants.to_vec(),
        };

        let computed_checksum = config.compute_checksum(dir_endian);
        if checksum != computed_checksum {
            log_i(
                format!("Checksums don't match! Stored: {checksum} Computed: {computed_checksum}")
                    .as_str(),
            );
        }

        return Ok(config);
    }

    pub fn compute_checksum(&self, dir_endian: Endian) -> u32 {
        let ver = human_version(self.director_version);

        let mut check: i64 = self.len as i64 + 1;
        check = check.wrapping_mul(self.file_version as i64 + 2);
        check = check.wrapping_div(self.movie_top as i64 + 3);
        check = check.wrapping_mul(self.movie_left as i64 + 4);
        check = check.wrapping_div(self.movie_bottom as i64 + 5);
        check = check.wrapping_mul(self.movie_right as i64 + 6);
        check = check.wrapping_sub(self.min_member as i64 + 7);
        check = check.wrapping_mul(self.max_member as i64 + 8);
        check = check.wrapping_sub(self.field9 as i64 + 9);
        check = check.wrapping_sub(self.field10 as i64 + 10);

        let operand11 = if ver < 700 {
            self.pre_d77field11 as i64
        } else {
            if let Endian::Little = dir_endian {
                ((self.d7_stage_color_b as i64) << 8 | self.d7_stage_color_g as i64) & 0xFFFF
            } else {
                ((self.d7_stage_color_g as i64) << 8 | self.d7_stage_color_b as i64) & 0xFFFF
            }
        };

        check = check.wrapping_add(operand11 + 11);
        check = check.wrapping_mul(self.comment_font as i64 + 12);
        check = check.wrapping_add(self.comment_size as i64 + 13);

        let operand14 = if ver < 800 {
            (self.comment_size as i64 >> 8) & 0xFF
        } else {
            self.comment_style as i64
        };

        check = check.wrapping_mul(operand14 + 14);

        let operand15 = if ver < 700 {
            self.pre_d7_stage_color as i64
        } else {
            self.d7_stage_color_r as i64
        };

        check = check.wrapping_add(operand15 + 15);
        check = check.wrapping_add(self.bit_depth as i64 + 16);
        check = check.wrapping_add(self.field17 as i64 + 17);
        check = check.wrapping_mul(self.field18 as i64 + 18);
        check = check.wrapping_add(self.field19 as i64 + 19);
        check = check.wrapping_mul(self.director_version as i64 + 20);
        check = check.wrapping_add(self.field21 as i64 + 21);
        check = check.wrapping_add(self.field22 as i64 + 22);
        check = check.wrapping_add(self.field23 as i64 + 23);
        check = check.wrapping_add(self.field24 as i64 + 24);
        check = check.wrapping_mul(self.field25 as i64 + 25);
        check = check.wrapping_add(self.frame_rate as i64 + 26);
        check = check.wrapping_mul(self.platform as i64 + 27);
        check = check.wrapping_mul(self.protection as i64 * 0xE06);
        check = check.wrapping_add(0xFF450000);
        check ^= FOURCC("ralf") as i64;

        (check & 0xFFFFFFFF) as u32
    }
}
