/// Little-endian binary reader for W3D block data.
/// Block data in the IFX format uses little-endian byte order.
pub struct W3dBlockReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> W3dBlockReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        if self.pos >= self.data.len() { 0 } else { self.data.len() - self.pos }
    }

    pub fn read_u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() {
            return Err(format!("W3dBlockReader: read_u8 at {}, len={}", self.pos, self.data.len()));
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn read_u16(&mut self) -> Result<u16, String> {
        let lo = self.read_u8()? as u16;
        let hi = self.read_u8()? as u16;
        Ok((hi << 8) | lo)
    }

    pub fn read_u32(&mut self) -> Result<u32, String> {
        let lo = self.read_u16()? as u32;
        let hi = self.read_u16()? as u32;
        Ok((hi << 16) | lo)
    }

    pub fn read_i32(&mut self) -> Result<i32, String> {
        Ok(self.read_u32()? as i32)
    }

    pub fn read_f32(&mut self) -> Result<f32, String> {
        let bits = self.read_u32()?;
        Ok(f32::from_bits(bits))
    }

    /// Read an IFX string and lowercase it (Director normalizes W3D names to lowercase)
    pub fn read_ifx_name(&mut self) -> Result<String, String> {
        self.read_ifx_string().map(|s| s.to_lowercase())
    }

    pub fn read_ifx_string(&mut self) -> Result<String, String> {
        let len = self.read_u16()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        if self.pos + len > self.data.len() {
            return Err(format!("W3dBlockReader: read_ifx_string len={} at {}, data_len={}", len, self.pos, self.data.len()));
        }
        let s: String = self.data[self.pos..self.pos + len].iter().map(|&b| b as char).collect();
        self.pos += len;
        Ok(s)
    }

    pub fn read_matrix4x4(&mut self) -> Result<[f32; 16], String> {
        let mut m = [0.0f32; 16];
        for i in 0..16 {
            m[i] = self.read_f32()?;
        }
        Ok(m)
    }

    pub fn read_vec4(&mut self) -> Result<[f32; 4], String> {
        Ok([self.read_f32()?, self.read_f32()?, self.read_f32()?, self.read_f32()?])
    }

    pub fn read_color_rgba(&mut self) -> Result<[f32; 4], String> {
        self.read_vec4()
    }

    pub fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, String> {
        let actual = count.min(self.data.len().saturating_sub(self.pos));
        let result = self.data[self.pos..self.pos + actual].to_vec();
        self.pos += actual;
        Ok(result)
    }

    pub fn skip(&mut self, count: usize) {
        self.pos += count;
    }
}
