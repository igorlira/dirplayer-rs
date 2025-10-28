use std::io::Read;

use binary_reader::BinaryReader;

pub trait DirectorExt {
    fn read_var_int(&mut self) -> Result<i32, std::io::Error>;
    fn read_zlib_bytes(&mut self, length: usize) -> Result<Vec<u8>, std::io::Error>;
    fn read_pascal_string(&mut self) -> Result<String, std::io::Error>;
    fn read_string(&mut self, len: usize) -> Result<String, std::io::Error>;
    fn read_apple_float_80(&mut self) -> Result<f64, String>;
    fn eof(&self) -> bool;
    fn bytes_left(&self) -> usize;
}

impl DirectorExt for BinaryReader {
    // TODO: u32?
    fn read_var_int(&mut self) -> Result<i32, std::io::Error> {
        let mut val: i32 = 0;
        let mut b: u8;
        loop {
            b = self.read_u8().unwrap();
            val = (val << 7) | ((b & 0x7f) as i32); // The 7 least significant bits are appended to the result
            if b >> 7 == 0 {
                // If the most significant bit is 1, there's another byte after
                break;
            }
        }
        return Ok(val);
    }

    fn bytes_left(&self) -> usize {
        self.length.saturating_sub(self.pos)
    }

    fn read_zlib_bytes(&mut self, length: usize) -> Result<Vec<u8>, std::io::Error> {
        let compressed_bytes = self.read_bytes(length)?;
        let mut decompressed = Vec::new();
        let mut decoder = flate2::read::ZlibDecoder::new(&compressed_bytes[..]);

        decoder.read_to_end(&mut decompressed)?;

        return Ok(decompressed);
    }

    fn read_pascal_string(&mut self) -> Result<String, std::io::Error> {
        let len = self.read_u8().unwrap() as usize;
        return self.read_string(len);
    }

    fn read_string(&mut self, len: usize) -> Result<String, std::io::Error> {
        let bytes = self.read_bytes(len).unwrap();
        return Ok(unsafe { String::from_utf8_unchecked(bytes.to_vec()).to_string() });
    }

    fn read_apple_float_80(&mut self) -> Result<f64, String> {
        // Floats are stored as an "80 bit IEEE Standard 754 floating
        // point number (Standard Apple Numeric Environment [SANE] data type
        // Extended).

        let data = self.read_bytes(10).unwrap();
        let exponent = u16::from_be_bytes([data[0], data[1]]);
        let f64sign: u64 = ((exponent & 0x8000) as u64) << 48;
        let exponent = exponent & 0x7fff;

        let fraction_bytes = [
            data[2], data[3], data[4], data[5], data[6], data[7], data[8], data[9],
        ];
        let mut fraction: u64 = u64::from_be_bytes(fraction_bytes);
        fraction &= 0x7fffffffffffffff;

        let f64exp: u64;
        if exponent == 0 {
            f64exp = 0;
        } else if exponent == 0x7fff {
            f64exp = 0x7ff;
        } else {
            let normexp = exponent as i64 - 0x3fff;
            if normexp < -0x3fe || normexp >= 0x3ff {
                panic!("Constant float exponent too big for a double");
            }
            f64exp = (normexp + 0x3ff) as u64;
        }
        let f64exp = f64exp << 52;
        let f64fract = fraction >> 11;
        let f64bin = f64sign | f64exp | f64fract;

        let bytes = f64bin.to_be_bytes();
        return Ok(f64::from_be_bytes(bytes));
    }

    fn eof(&self) -> bool {
        return self.pos >= self.length;
    }
}
