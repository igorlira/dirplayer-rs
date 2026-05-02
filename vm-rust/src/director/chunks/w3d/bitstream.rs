/// ECMA-363 arithmetic decoder for IFX/U3D compressed bitstream data.
/// Ported from IFXBitStreamCompressed.cs.
///
/// Key design:
/// - Context 0 (Context8): static context for uncompressed bytes, range 256, SwapBits8 applied
/// - Contexts 1..STATIC_FULL (0x400): dynamic contexts with adaptive histograms
/// - Contexts > STATIC_FULL: static contexts with uniform distribution
/// - Contexts >= MAX_RANGE: written uncompressed

const CONTEXT8: u32 = 0;
const STATIC_FULL: u32 = 0x00000400;
const MAX_RANGE: u32 = STATIC_FULL + 0x00003FFF;
const HALF_MASK: u32 = 0x00008000;
const NOT_HALF_MASK: u32 = 0x00007FFF;
const QUARTER_MASK: u32 = 0x00004000;
const NOT_THREE_QUARTER_MASK: u32 = 0x00003FFF;
const ELEPHANT: u16 = 0x1FFF;
const MAXIMUM_SYMBOL_IN_HISTOGRAM: u32 = 0x0000FFFF;
const ARRAY_SIZE_INCR: usize = 32;

const SWAP8: [u32; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
const FAST_NOT_MASK: [u32; 5] = [0x0000FFFF, 0x00007FFF, 0x00003FFF, 0x00001FFF, 0x00000FFF];
const READ_COUNT: [u32; 16] = [4, 3, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0];

pub struct IFXBitStreamCompressed {
    // Arithmetic decoder state
    high: u32,
    low: u32,
    underflow: u32,
    code: u32,

    // Data buffer (as u32 array, LE)
    data: Vec<u32>,
    data_position: u32,
    data_local: u32,
    data_local_next: u32,
    data_bit_offset: i32,

    // Context manager (histogram arrays)
    symbol_count: Vec<Option<Vec<u16>>>,
    cumulative_count: Vec<Option<Vec<u16>>>,
}

impl IFXBitStreamCompressed {
    pub fn new(block_data: &[u8]) -> Self {
        // Pack bytes into u32 LE dwords
        let num_dwords = (block_data.len() + 3) / 4;
        let mut data = vec![0u32; num_dwords + 16384]; // generous padding for arithmetic decoder overread
        for (i, &b) in block_data.iter().enumerate() {
            data[i / 4] |= (b as u32) << ((i % 4) * 8);
        }

        let data_local = data[0];
        let data_local_next = if data.len() > 1 { data[1] } else { 0 };

        let symbol_count = vec![None; (STATIC_FULL + 1) as usize];
        let cumulative_count = vec![None; (STATIC_FULL + 1) as usize];

        Self {
            high: 0x0000FFFF,
            low: 0,
            underflow: 0,
            code: 0,
            data,
            data_position: 0,
            data_local,
            data_local_next,
            data_bit_offset: 0,
            symbol_count,
            cumulative_count,
        }
    }

    // ─── Public Read API ───

    pub fn read_u8(&mut self) -> u8 {
        // Fast path when decoder is in pristine state
        if self.high == 0xFFFF && self.low == 0 && self.underflow == 0 {
            let mut raw = self.data_local >> self.data_bit_offset;
            if self.data_bit_offset > 24 {
                raw |= self.data_local_next << (32 - self.data_bit_offset);
            }
            self.data_bit_offset += 8;
            if self.data_bit_offset >= 32 {
                self.data_bit_offset -= 32;
                self.increment_position();
            }
            return raw as u8; // NO SwapBits8 on fast path
        }
        let mut u_value = self.read_symbol_dispatch(0x500);
        Self::swap_bits8(&mut u_value);
        u_value as u8
    }

    pub fn read_u16(&mut self) -> u16 {
        let lo = self.read_u8() as u16;
        let hi = self.read_u8() as u16;
        lo | (hi << 8)
    }

    pub fn read_u32(&mut self) -> u32 {
        let lo = self.read_u16() as u32;
        let hi = self.read_u16() as u32;
        lo | (hi << 16)
    }

    pub fn read_i32(&mut self) -> i32 {
        self.read_u32() as i32
    }

    pub fn read_f32(&mut self) -> f32 {
        let bits = self.read_u32();
        f32::from_bits(bits)
    }

    pub fn read_ifx_string(&mut self) -> String {
        let len = self.read_u16() as usize;
        if len == 0 {
            return String::new();
        }
        let chars: String = (0..len).map(|_| self.read_u8() as char).collect();
        chars
    }

    /// Get current byte position in stream (for diagnostics)
    pub fn byte_position(&self) -> usize {
        ((self.data_position << 5) as usize + self.data_bit_offset as usize) / 8
    }

    pub fn read_compressed_u32(&mut self, context: u32) -> u32 {
        if context != CONTEXT8 && context < MAX_RANGE {
            let symbol = self.read_symbol_dispatch(context);
            if symbol != 0 {
                return symbol - 1;
            }
            // Escape: read uncompressed
            let val = self.read_u32();
            if context < STATIC_FULL {
                self.add_symbol(context, val + 1);
            }
            return val;
        }
        self.read_u32()
    }

    pub fn read_compressed_u16(&mut self, context: u32) -> u16 {
        if context != 0 && context < MAX_RANGE {
            let symbol = self.read_symbol_dispatch(context);
            if symbol != 0 {
                return (symbol - 1) as u16;
            }
            let val = self.read_u16();
            if context < STATIC_FULL {
                self.add_symbol(context, val as u32 + 1);
            }
            return val;
        }
        self.read_u16()
    }

    pub fn read_compressed_u8(&mut self, context: u32) -> u8 {
        if context != 0 && context < MAX_RANGE {
            let symbol = self.read_symbol_dispatch(context);
            if symbol != 0 {
                return (symbol - 1) as u8;
            }
            let val = self.read_u8();
            if context < STATIC_FULL {
                self.add_symbol(context, val as u32 + 1);
            }
            return val;
        }
        self.read_u8()
    }

    // ─── Arithmetic Decoder Core ───

    fn read_symbol_dispatch(&mut self, context: u32) -> u32 {
        if context == CONTEXT8 {
            return self.read_symbol_static(0x500);
        }
        if context > STATIC_FULL {
            return self.read_symbol_static(context);
        }
        self.read_symbol_dynamic(context)
    }

    fn read_symbol_dynamic(&mut self, context: u32) -> u32 {
        // Save position
        let position = (self.data_position << 5) + self.data_bit_offset as u32;

        // Read one bit into code
        let code_bit = self.read_bit();
        self.code = code_bit;

        self.data_bit_offset += self.underflow as i32;
        while self.data_bit_offset >= 32 {
            self.data_bit_offset -= 32;
            self.increment_position();
        }

        // Read 15 more bits
        let temp = self.read_15_bits();
        self.code <<= 15;
        self.code |= temp;

        // Seek back
        self.seek_to_bit(position);

        // Get total count
        let total_cum_freq = self.get_total_symbol_frequency(context);
        let range = self.high.wrapping_add(1).wrapping_sub(self.low);

        if total_cum_freq == 0 {
            return 1;
        }

        // Determine symbol from code
        // Use u32 wrapping arithmetic to match C# behavior exactly
        let code_range = 1u32.wrapping_add(self.code).wrapping_sub(self.low);
        let code_cum_freq = (total_cum_freq as u64 * code_range as u64 - 1) / range as u64;
        let u_value = self.get_symbol_from_frequency(context, code_cum_freq as u32);

        // Update state
        let value_cum_freq = self.get_cumulative_symbol_frequency(context, u_value);
        let value_freq = self.get_symbol_frequency(context, u_value);

        let mut low = self.low;
        let mut high;
        // Match C# u32 arithmetic: range * combined / total (all in u32, wrapping)
        high = low.wrapping_sub(1).wrapping_add(
            (range.wrapping_mul(value_cum_freq + value_freq)) / total_cum_freq
        );
        low = low.wrapping_add(
            (range.wrapping_mul(value_cum_freq)) / total_cum_freq
        );

        // Update context
        if context < STATIC_FULL && context != CONTEXT8 {
            self.add_symbol(context, u_value);
        }

        // Count bits to read
        let mut bit_count = READ_COUNT[(((low >> 12) ^ (high >> 12)) & 0x0F) as usize] as i32;
        low &= FAST_NOT_MASK[bit_count as usize];
        high &= FAST_NOT_MASK[bit_count as usize];
        high <<= bit_count;
        low <<= bit_count;
        high |= (1u32 << bit_count) - 1;

        // Regular count
        let mut masked_low = HALF_MASK & low;
        let mut masked_high = HALF_MASK & high;
        while (masked_low | masked_high) == 0
            || (masked_low == HALF_MASK && masked_high == HALF_MASK)
        {
            low = (NOT_HALF_MASK & low) << 1;
            high = ((NOT_HALF_MASK & high) << 1) | 1;
            masked_low = HALF_MASK & low;
            masked_high = HALF_MASK & high;
            bit_count += 1;
        }

        let saved_bits_low = masked_low;
        let saved_bits_high = masked_high;

        if bit_count > 0 {
            bit_count += self.underflow as i32;
            self.underflow = 0;
        }

        // Count underflow bits
        masked_low = QUARTER_MASK & low;
        masked_high = QUARTER_MASK & high;
        let mut underflow_count = 0u32;
        while masked_low == 0x4000 && masked_high == 0 {
            low &= NOT_THREE_QUARTER_MASK;
            high &= NOT_THREE_QUARTER_MASK;
            low += low;
            high += high;
            high |= 1;
            masked_low = QUARTER_MASK & low;
            masked_high = QUARTER_MASK & high;
            underflow_count += 1;
        }

        self.underflow += underflow_count;
        low |= saved_bits_low;
        high |= saved_bits_high;
        self.low = low;
        self.high = high;

        self.data_bit_offset += bit_count;
        while self.data_bit_offset >= 32 {
            self.data_bit_offset -= 32;
            self.increment_position();
        }

        u_value
    }

    fn read_symbol_static(&mut self, context: u32) -> u32 {
        let position = (self.data_position << 5) + self.data_bit_offset as u32;

        let code_bit = self.read_bit();
        self.code = code_bit;

        self.data_bit_offset += self.underflow as i32;
        while self.data_bit_offset >= 32 {
            self.data_bit_offset -= 32;
            self.increment_position();
        }

        let temp = self.read_15_bits();
        self.code <<= 15;
        self.code |= temp;

        self.seek_to_bit(position);

        let total_cum_freq = context - 1024;
        let range = self.high.wrapping_add(1).wrapping_sub(self.low);

        // Use u32 wrapping arithmetic to match C# behavior exactly
        let code_range = 1u32.wrapping_add(self.code).wrapping_sub(self.low);
        let u_value = (code_range as u64 * total_cum_freq as u64 - 1) / range as u64;
        let u_value = u_value as u32;

        // Match C# u32 arithmetic
        let mut low = self.low.wrapping_add(range.wrapping_mul(u_value) / total_cum_freq);
        let mut high = self.low.wrapping_sub(1).wrapping_add(range.wrapping_mul(u_value + 1) / total_cum_freq);

        let mut bit_count = READ_COUNT[(((low >> 12) ^ (high >> 12)) & 0x0F) as usize] as i32;
        low &= FAST_NOT_MASK[bit_count as usize];
        high &= FAST_NOT_MASK[bit_count as usize];
        high <<= bit_count;
        low <<= bit_count;
        high |= (1u32 << bit_count) - 1;

        let mut masked_low = HALF_MASK & low;
        let mut masked_high = HALF_MASK & high;
        while (masked_low | masked_high) == 0
            || (masked_low == HALF_MASK && masked_high == HALF_MASK)
        {
            low = (NOT_HALF_MASK & low) << 1;
            high = ((NOT_HALF_MASK & high) << 1) | 1;
            masked_low = HALF_MASK & low;
            masked_high = HALF_MASK & high;
            bit_count += 1;
        }

        let saved_bits_low = masked_low;
        let saved_bits_high = masked_high;

        if bit_count > 0 {
            bit_count += self.underflow as i32;
            self.underflow = 0;
        }

        masked_low = QUARTER_MASK & low;
        masked_high = QUARTER_MASK & high;
        let mut underflow_count = 0u32;
        while masked_low == 0x4000 && masked_high == 0 {
            low &= NOT_THREE_QUARTER_MASK;
            high &= NOT_THREE_QUARTER_MASK;
            low += low;
            high += high;
            high |= 1;
            masked_low = QUARTER_MASK & low;
            masked_high = QUARTER_MASK & high;
            underflow_count += 1;
        }

        self.underflow += underflow_count;
        low |= saved_bits_low;
        high |= saved_bits_high;
        self.low = low;
        self.high = high;

        self.data_bit_offset += bit_count;
        while self.data_bit_offset >= 32 {
            self.data_bit_offset -= 32;
            self.increment_position();
        }

        u_value
    }

    // ─── Bit I/O ───

    fn read_bit(&mut self) -> u32 {
        let r_value = (self.data_local >> self.data_bit_offset) & 1;
        self.data_bit_offset += 1;
        if self.data_bit_offset >= 32 {
            self.data_bit_offset -= 32;
            self.increment_position();
        }
        r_value
    }

    fn read_15_bits(&mut self) -> u32 {
        let mut u_value = self.data_local >> self.data_bit_offset;
        if self.data_bit_offset > 17 {
            u_value |= self.data_local_next << (32 - self.data_bit_offset);
        }

        u_value <<= 1; // left shift by 1
        u_value = (SWAP8[((u_value >> 12) & 0xf) as usize])
            | ((SWAP8[((u_value >> 8) & 0xf) as usize]) << 4)
            | ((SWAP8[((u_value >> 4) & 0xf) as usize]) << 8)
            | ((SWAP8[(u_value & 0xf) as usize]) << 12);

        self.data_bit_offset += 15;
        if self.data_bit_offset >= 32 {
            self.data_bit_offset -= 32;
            self.increment_position();
        }

        u_value
    }

    fn increment_position(&mut self) {
        self.data_position += 1;
        let pos = self.data_position as usize;
        self.data_local = if pos < self.data.len() { self.data[pos] } else { 0 };
        self.data_local_next = if pos + 1 < self.data.len() { self.data[pos + 1] } else { 0 };
    }

    fn seek_to_bit(&mut self, position: u32) {
        self.data_position = position >> 5;
        self.data_bit_offset = (position & 0x1F) as i32;
        let pos = self.data_position as usize;
        self.data_local = if pos < self.data.len() { self.data[pos] } else { 0 };
        self.data_local_next = if pos + 1 < self.data.len() { self.data[pos + 1] } else { 0 };
    }

    fn swap_bits8(r_value: &mut u32) {
        *r_value = (SWAP8[(*r_value & 0xf) as usize] << 4)
            | SWAP8[((*r_value >> 4) & 0xf) as usize];
    }

    // ─── Context Manager ───

    fn add_symbol(&mut self, context: u32, symbol: u32) {
        if !Self::is_dynamic_context(context) || symbol >= MAXIMUM_SYMBOL_IN_HISTOGRAM {
            return;
        }

        let ctx = context as usize;

        // Ensure arrays are large enough
        let needs_resize = match &self.cumulative_count[ctx] {
            None => true,
            Some(v) => v.len() <= symbol as usize,
        };

        if needs_resize {
            let new_len = symbol as usize + ARRAY_SIZE_INCR;
            let is_new = self.cumulative_count[ctx].is_none();

            let mut new_cum = vec![0u16; new_len];
            let mut new_sym = vec![0u16; new_len];

            if is_new {
                // New context: initialize escape symbol with freq 1
                new_cum[0] = 1;
                new_sym[0] = 1;
            } else {
                let old_cum = self.cumulative_count[ctx].as_ref().unwrap();
                let old_sym = self.symbol_count[ctx].as_ref().unwrap();
                new_cum[..old_cum.len()].copy_from_slice(old_cum);
                new_sym[..old_sym.len()].copy_from_slice(old_sym);
            }

            self.cumulative_count[ctx] = Some(new_cum);
            self.symbol_count[ctx] = Some(new_sym);
        }

        let cum_count = self.cumulative_count[ctx].as_mut().unwrap();
        let sym_count = self.symbol_count[ctx].as_mut().unwrap();

        if cum_count[0] >= ELEPHANT {
            // Rescale to prevent overflow
            let len = cum_count.len();
            let mut temp_accum: u16 = 0;
            for i in (0..len).rev() {
                sym_count[i] >>= 1;
                temp_accum += sym_count[i];
                cum_count[i] = temp_accum;
            }
            sym_count[0] += 1;
            cum_count[0] += 1;
        }

        sym_count[symbol as usize] += 1;
        for i in 0..=symbol as usize {
            cum_count[i] += 1;
        }
    }

    fn get_symbol_frequency(&self, context: u32, symbol: u32) -> u32 {
        if Self::is_dynamic_context(context) {
            if let Some(sym_count) = &self.symbol_count[context as usize] {
                if (symbol as usize) < sym_count.len() {
                    return sym_count[symbol as usize] as u32;
                }
            }
            if symbol == 0 { return 1; } // escape default
            return 0;
        }
        // Static context: uniform distribution, frequency = 1
        1
    }

    fn get_cumulative_symbol_frequency(&self, context: u32, symbol: u32) -> u32 {
        if Self::is_dynamic_context(context) {
            if let Some(cum_count) = &self.cumulative_count[context as usize] {
                if (symbol as usize) < cum_count.len() {
                    return (cum_count[0] - cum_count[symbol as usize]) as u32;
                }
                return cum_count[0] as u32;
            }
            return 0;
        }
        // Static contexts: direct mapping
        symbol
    }

    fn get_total_symbol_frequency(&self, context: u32) -> u32 {
        if Self::is_dynamic_context(context) {
            if let Some(cum_count) = &self.cumulative_count[context as usize] {
                return cum_count[0] as u32;
            }
            return 1; // just escape symbol
        }
        if context == CONTEXT8 { return 256; }
        // Static context: range = context - 1024
        context - 1024
    }

    fn get_symbol_from_frequency(&self, context: u32, symbol_frequency: u32) -> u32 {
        if Self::is_dynamic_context(context) {
            if let Some(cum_count) = &self.cumulative_count[context as usize] {
                if symbol_frequency == 0 { return 0; }
                if (cum_count[0] as u32) < symbol_frequency { return 0; }

                let mut r_value = 0u32;
                for i in 0..cum_count.len() as u32 {
                    if self.get_cumulative_symbol_frequency(context, i) <= symbol_frequency {
                        r_value = i;
                    } else {
                        break;
                    }
                }
                return r_value;
            }
            return 0;
        }
        // Static contexts: direct mapping
        symbol_frequency
    }

    fn is_dynamic_context(context: u32) -> bool {
        context != CONTEXT8 && context <= STATIC_FULL
    }
}
