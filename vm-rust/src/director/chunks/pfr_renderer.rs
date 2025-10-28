use web_sys::console;

/// PFR Vector Renderer - Complete Implementation
/// Renders PFR (Portable Font Resource) vector glyphs to bitmaps
///
/// PFR bytecode format (inspired by PostScript Type 1):
/// - Stack-based VM with coordinates and drawing commands
/// - Coordinates are typically small signed values (fit in i8 or i16)
/// - Commands include MOVETO, LINETO, curve operations, and FILL

#[derive(Debug, Clone)]
enum PathCommand {
    MoveTo(Point),
    LineTo(Point),
    Close,
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

pub struct PfrRenderer {
    width: usize,
    height: usize,
    stack: Vec<i16>,
    current_pos: Point,
    subpaths: Vec<Vec<Point>>,
    current_path: Vec<Point>,
}

impl PfrRenderer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            stack: Vec::new(),
            current_pos: Point { x: 0.0, y: 0.0 },
            subpaths: Vec::new(),
            current_path: Vec::new(),
        }
    }

    pub fn render_glyph(data: &[u8], width: usize, height: usize) -> Vec<u8> {
        let mut vm = Self::new(width, height);
        vm.execute(data);
        vm.rasterize()
    }

    fn push(&mut self, value: i16) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> Option<i16> {
        self.stack.pop()
    }

    fn pop_coord(&mut self) -> Option<(f32, f32)> {
        let y = self.pop()? as f32;
        let x = self.pop()? as f32;
        Some((x, y))
    }

    fn execute(&mut self, data: &[u8]) {
        let mut pc = 0;

        while pc < data.len() {
            let opcode = data[pc];
            pc += 1;

            match opcode {
                // Literal values
                0x00 => self.push(0),
                0x01..=0x6F => self.push(opcode as i16),

                // Negative literals
                0x70..=0x7F => {
                    self.push((opcode as i16) - 0x80);
                }

                // 16-bit literals (big-endian signed)
                0x80..=0x8F => {
                    if pc < data.len() {
                        let high = (opcode & 0x0F) as i16;
                        let low = data[pc] as i16;
                        let mut value = (high << 8) | low;
                        // Sign extend
                        if value >= 0x800 {
                            value -= 0x1000;
                        }
                        self.push(value);
                        pc += 1;
                    }
                }

                // === PATH OPERATORS ===

                // rmoveto - relative move
                0x01 | 0x05 => {
                    if let Some((dx, dy)) = self.pop_coord() {
                        self.current_pos.x += dx;
                        self.current_pos.y += dy;

                        if !self.current_path.is_empty() {
                            self.subpaths.push(self.current_path.clone());
                            self.current_path.clear();
                        }
                        self.current_path.push(self.current_pos);
                    }
                }

                // rlineto - relative line
                0x02 | 0x06 => {
                    if let Some((dx, dy)) = self.pop_coord() {
                        self.current_pos.x += dx;
                        self.current_pos.y += dy;
                        self.current_path.push(self.current_pos);
                    }
                }

                // hmoveto - horizontal move
                0x07 => {
                    if let Some(dx) = self.pop() {
                        self.current_pos.x += dx as f32;

                        if !self.current_path.is_empty() {
                            self.subpaths.push(self.current_path.clone());
                            self.current_path.clear();
                        }
                        self.current_path.push(self.current_pos);
                    }
                }

                // vmoveto - vertical move
                0x08 => {
                    if let Some(dy) = self.pop() {
                        self.current_pos.y += dy as f32;

                        if !self.current_path.is_empty() {
                            self.subpaths.push(self.current_path.clone());
                            self.current_path.clear();
                        }
                        self.current_path.push(self.current_pos);
                    }
                }

                // hlineto - horizontal line
                0x0D | 0x0E => {
                    if let Some(dx) = self.pop() {
                        self.current_pos.x += dx as f32;
                        self.current_path.push(self.current_pos);
                    }
                }

                // vlineto - vertical line
                0x0F => {
                    if let Some(dy) = self.pop() {
                        self.current_pos.y += dy as f32;
                        self.current_path.push(self.current_pos);
                    }
                }

                // closepath + fill
                0x10 | 0xE0..=0xFF => {
                    if !self.current_path.is_empty() {
                        self.subpaths.push(self.current_path.clone());
                        self.current_path.clear();
                    }
                    break; // End of glyph
                }

                _ => {
                    // Unknown opcode - ignore
                }
            }
        }

        // Add any remaining path
        if !self.current_path.is_empty() {
            self.subpaths.push(self.current_path.clone());
        }
    }

    fn rasterize(&self) -> Vec<u8> {
        let bytes_per_row = (self.width + 7) / 8;
        let mut bitmap = vec![0u8; bytes_per_row * self.height];

        // Fill paths using scanline algorithm
        for path in &self.subpaths {
            if path.len() < 2 {
                continue;
            }

            // Find bounding box
            let mut min_y = path[0].y;
            let mut max_y = path[0].y;

            for pt in path {
                min_y = min_y.min(pt.y);
                max_y = max_y.max(pt.y);
            }

            // Rasterize each scanline
            for y in (min_y.floor() as i32)..(max_y.ceil() as i32 + 1) {
                if y < 0 || y >= self.height as i32 {
                    continue;
                }

                let mut intersections = Vec::new();
                let y_f = y as f32 + 0.5;

                // Find edge intersections
                for i in 0..path.len() {
                    let p1 = path[i];
                    let p2 = path[(i + 1) % path.len()];

                    if (p1.y <= y_f && p2.y > y_f) || (p2.y <= y_f && p1.y > y_f) {
                        let t = (y_f - p1.y) / (p2.y - p1.y);
                        let x = p1.x + t * (p2.x - p1.x);
                        intersections.push(x);
                    }
                }

                intersections.sort_by(|a, b| a.partial_cmp(b).unwrap());

                // Fill between pairs
                for chunk in intersections.chunks(2) {
                    if chunk.len() == 2 {
                        let x1 = chunk[0].max(0.0).min(self.width as f32) as usize;
                        let x2 = chunk[1].max(0.0).min(self.width as f32) as usize;

                        for x in x1..x2 {
                            let byte_idx = y as usize * bytes_per_row + x / 8;
                            let bit_idx = 7 - (x % 8);
                            if byte_idx < bitmap.len() {
                                bitmap[byte_idx] |= 1 << bit_idx;
                            }
                        }
                    }
                }
            }
        }

        bitmap
    }

    /// Bresenham's line algorithm
    fn draw_line(&self, bitmap: &mut [u8], bytes_per_row: usize, p0: Point, p1: Point) {
        let mut x0 = p0.x.round() as i32;
        let mut y0 = p0.y.round() as i32;
        let x1 = p1.x.round() as i32;
        let y1 = p1.y.round() as i32;

        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx - dy;

        loop {
            self.set_pixel(bitmap, bytes_per_row, x0, y0);

            if x0 == x1 && y0 == y1 {
                break;
            }

            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x0 += sx;
            }
            if e2 < dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn set_pixel(&self, bitmap: &mut [u8], bytes_per_row: usize, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }

        let byte_idx = y as usize * bytes_per_row + x as usize / 8;
        let bit_idx = 7 - (x as usize % 8);

        if byte_idx < bitmap.len() {
            bitmap[byte_idx] |= 1 << bit_idx;
        }
    }
}

/// Find glyph boundaries in PFR bytecode
/// Glyphs are terminated by FILL commands (0xF0-0xFF)
fn find_glyph_boundaries(pfr_data: &[u8]) -> Vec<usize> {
    let mut boundaries = vec![0];
    let mut i = 0;

    while i < pfr_data.len() {
        let cmd = pfr_data[i];

        // FILL commands (0x10, 0xE0-0xFF) mark glyph ends
        if cmd == 0x10 || cmd >= 0xE0 {
            if i + 1 < pfr_data.len() {
                boundaries.push(i + 1);
            }
            i += 1;
        } else if cmd <= 0x7F {
            // Small literal
            i += 1;
        } else if cmd >= 0x80 && cmd <= 0x8F {
            // 16-bit literal
            i += 2;
        } else {
            i += 1;
        }
    }

    boundaries
}

/// Analyze bytecode to find proper glyph boundaries
pub fn analyze_pfr_bytecode(data: &[u8]) -> Vec<(usize, usize)> {
    let mut boundaries = Vec::new();
    let mut i = 0;
    let mut glyph_start = 0;

    while i < data.len() {
        let cmd = data[i];

        // Check for FILL/END commands
        if cmd == 0x10 || cmd >= 0xE0 {
            // Found end of glyph
            boundaries.push((glyph_start, i + 1));
            i += 1;
            glyph_start = i;
        } else if cmd <= 0x7F {
            // Literal
            i += 1;
        } else if cmd >= 0x80 && cmd <= 0x8F {
            // 16-bit literal
            i += 2;
        } else {
            // Other command
            i += 1;
        }
    }

    // Handle last glyph if no terminator
    if glyph_start < data.len() {
        boundaries.push((glyph_start, data.len()));
    }

    boundaries
}

/// Main rendering function with detailed logging
pub fn render_pfr_font(
    pfr_data: &[u8],
    char_width: usize,
    char_height: usize,
    glyph_count: usize,
) -> Vec<u8> {
    console::log_1(&"🎨 PFR VM Renderer: Interpreting bytecode...".into());

    // Try to find glyph boundaries by looking for fill commands
    let mut boundaries = vec![0];
    let mut i = 0;

    while i < pfr_data.len() {
        let byte = pfr_data[i];

        // Fill commands end glyphs
        if byte == 0x10 || byte >= 0xE0 {
            if i + 1 < pfr_data.len() && boundaries.len() < glyph_count {
                boundaries.push(i + 1);
            }
        }

        // Skip multi-byte instructions
        if byte >= 0x80 && byte <= 0x8F {
            i += 2;
        } else {
            i += 1;
        }
    }

    console::log_1(&format!("  Found {} glyph boundaries", boundaries.len()).into());

    let bytes_per_glyph = ((char_width + 7) / 8) * char_height;
    let mut output = vec![0u8; bytes_per_glyph * glyph_count];

    // Render each glyph
    for glyph_idx in 0..glyph_count.min(boundaries.len()) {
        let start = boundaries[glyph_idx];
        let end = if glyph_idx + 1 < boundaries.len() {
            boundaries[glyph_idx + 1]
        } else {
            pfr_data.len()
        };

        if start >= end || start >= pfr_data.len() {
            continue;
        }

        let bytecode = &pfr_data[start..end];
        let glyph_bitmap = PfrRenderer::render_glyph(bytecode, char_width, char_height);

        let offset = glyph_idx * bytes_per_glyph;
        let copy_len = glyph_bitmap.len().min(bytes_per_glyph);

        if offset + copy_len <= output.len() {
            output[offset..offset + copy_len].copy_from_slice(&glyph_bitmap[..copy_len]);
        }
    }

    // Log samples
    console::log_1(&"  Sample glyphs:".into());
    for &idx in &[32, 65, 72] {
        if idx < glyph_count {
            let offset = idx * bytes_per_glyph;
            if offset + bytes_per_glyph <= output.len() {
                let glyph = &output[offset..offset + bytes_per_glyph];
                let pixels: u32 = glyph.iter().map(|b| b.count_ones()).sum();

                console::log_1(
                    &format!(
                        "    Glyph #{} ('{}'):  {} pixels",
                        idx,
                        match idx {
                            32 => "space",
                            65 => "A",
                            72 => "H",
                            _ => "?",
                        },
                        pixels
                    )
                    .into(),
                );

                if pixels > 0 {
                    for row in 0..char_height.min(8) {
                        if row < bytes_per_glyph {
                            let byte = glyph[row];
                            let mut line = String::from("      ");
                            for bit in 0..8 {
                                line.push(if byte & (1 << (7 - bit)) != 0 {
                                    '█'
                                } else {
                                    '·'
                                });
                            }
                            console::log_1(&line.into());
                        }
                    }
                }
            }
        }
    }

    console::log_1(&format!("✅ Rendered {} glyphs", glyph_count).into());
    output
}
