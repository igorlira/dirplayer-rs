use std::{sync::Arc, vec};

use binary_reader::BinaryReader;
use log::warn;
use num::ToPrimitive;
use num_derive::{FromPrimitive, ToPrimitive};
use std::convert::TryInto;

use crate::{
    director::enums::BitmapInfo,
    player::{
        cast_lib::CastMemberRef, handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
        sprite::ColorRef, symbols::{builtin::BuiltInSymbol, symbol::Symbol},
    },
};
use num::FromPrimitive;

use super::{
    mask::BitmapMask,
    palette::{
        GRAYSCALE_16_PALETTE, GRAYSCALE_4_PALETTE, GRAYSCALE_PALETTE, MAC_16_PALETTE,
        METALLIC16_PALETTE, METALLIC_PALETTE, NTSC16_PALETTE, NTSC_PALETTE, PASTELS16_PALETTE,
        PASTELS_PALETTE, RAINBOW16_PALETTE, RAINBOW_PALETTE, SYSTEM_MAC_PALETTE,
        SYSTEM_WIN_PALETTE, VIVID16_PALETTE, VIVID_PALETTE, WEB_216_PALETTE, WIN_16_PALETTE,
    },
    palette_map::PaletteMap,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteRef {
    BuiltIn(BuiltInPalette),
    Member(CastMemberRef),
    /// Use the movie's default palette (first available custom palette, or system palette if none)
    /// This is used when palette_id=0 (meaning "use default" rather than a specific member)
    Default,
}

impl PaletteRef {
    /// Create a PaletteRef from parsed palette_id and clut_cast_lib values.
    ///
    /// - i < 0: builtin palette enum value (e.g., -1=SystemMac, -3=GrayScale)
    /// - i > 0: custom palette member number
    /// - clut_cast_lib: the cast lib containing the palette (0 = search all cast libs)
    pub fn from(i: i16, clut_cast_lib: i16, bitmap_cast_lib: u32) -> Self {
        if i < 0 {
            match BuiltInPalette::from_i16(i) {
                Some(palette) => PaletteRef::BuiltIn(palette),
                None => {
                    web_sys::console::warn_1(
                        &format!("Unknown built-in palette ID: {}, defaulting to SystemWin", i).into()
                    );
                    PaletteRef::BuiltIn(BuiltInPalette::SystemWin)
                }
            }
        } else if i == 0 {
            PaletteRef::BuiltIn(get_system_default_palette())
        } else {
            // clut_cast_lib >= 0: use as-is (0 = search all, >0 = explicit cast lib)
            // clut_cast_lib < 0: not set, use bitmap's own cast lib (ScummVM: _cast->_castLibID)
            let cast_lib = if clut_cast_lib >= 0 {
                clut_cast_lib as i32
            } else {
                bitmap_cast_lib as i32
            };
            PaletteRef::Member(CastMemberRef {
                cast_lib,
                cast_member: i as i32,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, ToPrimitive, FromPrimitive, PartialEq, Eq)]
pub enum BuiltInPalette {
    GrayScale = -3,
    Pastels = -4,
    Vivid = -5,
    Ntsc = -6,
    Metallic = -7,
    Web216 = -8,
    Vga = -9,
    SystemWinDir4 = -101,
    SystemWin = -102,
    SystemMac = -1,
    Rainbow = -2,
}

impl BuiltInPalette {
    pub fn from_symbol(symbol: Symbol) -> Option<Self> {
        match symbol.into_builtin_or_error().ok()? {
            BuiltInSymbol::Grayscale => Some(BuiltInPalette::GrayScale),
            BuiltInSymbol::Pastels => Some(BuiltInPalette::Pastels),
            BuiltInSymbol::Vivid => Some(BuiltInPalette::Vivid),
            BuiltInSymbol::Ntsc => Some(BuiltInPalette::Ntsc),
            BuiltInSymbol::Metallic => Some(BuiltInPalette::Metallic),
            BuiltInSymbol::Web216 => Some(BuiltInPalette::Web216),
            BuiltInSymbol::Vga => Some(BuiltInPalette::Vga),
            BuiltInSymbol::SystemWinDir4 => Some(BuiltInPalette::SystemWinDir4),
            BuiltInSymbol::SystemWin => Some(BuiltInPalette::SystemWin),
            BuiltInSymbol::SystemMac => Some(BuiltInPalette::SystemMac),
            BuiltInSymbol::Rainbow => Some(BuiltInPalette::Rainbow),
            _ => None,
        }
    }

    pub fn symbol(&self) -> BuiltInSymbol {
        match self {
            BuiltInPalette::GrayScale => BuiltInSymbol::Grayscale,
            BuiltInPalette::Pastels => BuiltInSymbol::Pastels,
            BuiltInPalette::Vivid => BuiltInSymbol::Vivid,
            BuiltInPalette::Ntsc => BuiltInSymbol::Ntsc,
            BuiltInPalette::Metallic => BuiltInSymbol::Metallic,
            BuiltInPalette::Web216 => BuiltInSymbol::Web216,
            BuiltInPalette::Vga => BuiltInSymbol::Vga,
            BuiltInPalette::SystemWinDir4 => BuiltInSymbol::SystemWinDir4,
            BuiltInPalette::SystemWin => BuiltInSymbol::SystemWin,
            BuiltInPalette::SystemMac => BuiltInSymbol::SystemMac,
            BuiltInPalette::Rainbow => BuiltInSymbol::Rainbow,
        }
    }
}

thread_local! {
    // The movie's default system palette, set at load time from the config's
    // platform byte. Mac-authored movies (e.g. Director-4 titles like thead)
    // default to System-Mac; Windows movies to System-Win. These differ at the
    // high palette indices, which matters for indexed bitmaps and shape pattern
    // fills (thead's pattern background reads gray under System-Mac but wrong
    // colours under System-Win). Defaults to System-Win until a movie loads.
    static DEFAULT_SYSTEM_PALETTE: std::cell::Cell<BuiltInPalette> =
        const { std::cell::Cell::new(BuiltInPalette::SystemWin) };
}

/// Set the default system palette from the movie config's platform byte.
/// Director platform IDs: 1 = Mac, larger values (256+) = Windows.
pub fn set_default_system_palette_from_platform(platform: u16) {
    let pal = if platform == 1 {
        BuiltInPalette::SystemMac
    } else {
        BuiltInPalette::SystemWin
    };
    DEFAULT_SYSTEM_PALETTE.with(|p| p.set(pal));
}

pub fn get_system_default_palette() -> BuiltInPalette {
    DEFAULT_SYSTEM_PALETTE.with(|p| p.get())
}

/// Returns the palette index in `palette` whose RGB is closest (smallest
/// squared Euclidean distance) to `(r,g,b)`. Used by Director's
/// `color.paletteIndex` getter: per the 11.5 Scripting Dictionary p.832,
/// reading `.paletteIndex` on an RGB color returns the nearest match in the
/// current palette regardless of color type — e.g. `rgb(155,0,75).paletteIndex`
/// resolves to 106 on the default palette.
pub fn nearest_palette_index(r: u8, g: u8, b: u8, palette: &BuiltInPalette) -> u8 {
    let mut best_index: u8 = 0;
    let mut best_distance: i32 = i32::MAX;
    for i in 0u16..256u16 {
        if let Some((pr, pg, pb)) = lookup_builtin_palette(palette, i as u8, 8) {
            let dr = pr as i32 - r as i32;
            let dg = pg as i32 - g as i32;
            let db = pb as i32 - b as i32;
            let d = dr * dr + dg * dg + db * db;
            if d < best_distance {
                best_distance = d;
                best_index = i as u8;
                if d == 0 { break; }
            }
        }
    }
    best_index
}

#[derive(Clone)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    pub bit_depth: u8,          // Current storage format
    pub original_bit_depth: u8, // Original format (for palette selection)
    pub data: Vec<u8>,          // RGBA
    pub palette_ref: PaletteRef,
    pub matte: Option<Arc<BitmapMask>>,
    pub use_alpha: bool,
    pub trim_white_space: bool,
    pub was_trimmed: bool,
    /// Version counter for cache invalidation (incremented when bitmap data changes)
    pub version: u32,
    /// Higher-resolution twin of this bitmap, used only to keep
    /// Lingo-COMPOSED artwork sharp on a scaled stage.
    ///
    /// `width`/`height`/`data` above stay exactly what Director sees, in movie
    /// units — every script-visible number and every existing reader is
    /// unaffected. See `HiResTwin`.
    pub hi_res: HiResTwin,
}

/// The hi-res twin of a `Bitmap`, and the accounting that decides when keeping
/// one stops being worth it.
///
/// A twin is a parallel buffer at `scale` times the bitmap's movie-unit size,
/// carrying the same picture rendered (not magnified) at the stage scale. It is
/// seeded by a text/field member's `.image` — which re-rasterises the text with
/// `fontSize`, the box rect, `fixedLineSpace`, spacing and tab stops all
/// multiplied by the stage scale — and carried forward by `copyPixels`, so a
/// bitmap COMPOSED out of `.image` snapshots keeps it. The renderer uploads it
/// in place of `data` when the sprite is magnified.
///
/// Purely an enhancement: any op that cannot maintain it drops it and the
/// bitmap renders exactly as it did before twins existed.
#[derive(Clone, Default)]
pub struct HiResTwin {
    /// The twin itself, about `width*scale` by `height*scale`. Its own
    /// `hi_res` is always empty — twins do not nest.
    pub image: Option<Box<Bitmap>>,
    /// Scale of `image` relative to `width`/`height`. 1.0 when there is none.
    pub scale: f64,
    /// Destination pixels mirrored into the twin since it was created, used by
    /// the budget in `banned` below.
    pub written: u64,
    /// Blits mirrored into the twin since it was created — the other half of
    /// that budget, and the one that actually catches a framebuffer. Spectral
    /// Wizard composes into a 640x480 offscreen with many SMALL blits, so a
    /// pixel budget generous enough for one composed panel never fires there,
    /// while a blit count separates the two cleanly: a composed panel takes a
    /// few dozen blits in total, a framebuffer takes that many every second.
    pub blits: u32,
    /// A twin that has been PROMISED but not yet rendered: the text/field
    /// member to re-rasterise from, and the scale to do it at.
    ///
    /// `.image` records this instead of rasterising a second time on the spot,
    /// because most `.image` calls never compose their result into anything
    /// displayed — Spectral Wizard bakes 250+ of them straight into the stage
    /// framebuffer, which is banned from twins, so every one of those second
    /// rasterisations was pure waste (measured: 23% of the movie's total run
    /// time). It is materialised on demand by the paths that actually lead to
    /// the screen: a `copyPixels` that reads this bitmap as its SOURCE, and
    /// `member.image = ` assignment.
    pub pending: Option<(CastMemberRef, f64)>,
    /// Set once this bitmap has proved to be a FRAMEBUFFER rather than a
    /// composed picture, after which it is never twinned again.
    ///
    /// The twin pays for itself on a picture composed once and then displayed
    /// for many frames (a navigator's room list). It is a disaster on a buffer
    /// recomposed every frame: an "imaging Lingo" title like Spectral Wizard
    /// bakes its speech bubbles into `(the stage).image`, so one text `.image`
    /// landing there would make every subsequent per-frame blit run twice —
    /// once at 1:1 and once at scale² the pixels. That measured 4x slower over
    /// the whole movie.
    ///
    /// The discriminator is FREQUENCY, not size or content: see
    /// `HiResTwin::WRITE_BUDGET`.
    pub banned: bool,
}

impl HiResTwin {
    /// How many times over the twin may be rewritten before this bitmap is
    /// treated as a framebuffer and banned. A composed panel rewrites its twin
    /// a couple of times in total; a per-frame framebuffer passes this within
    /// a second or two and then costs exactly what it did before twins.
    const WRITE_BUDGET: u64 = 8;
    /// Mirrored blits allowed before this bitmap is treated as a framebuffer.
    /// The Coke Studios room list needs about ten (the text, seven rows of
    /// dotted leaders, then one per refresh).
    const BLIT_BUDGET: u32 = 250;

    #[inline]
    pub fn is_some(&self) -> bool {
        self.image.is_some()
    }

    /// The scale this bitmap's twin has, or would have once materialised.
    /// 1.0 when there is neither.
    #[inline]
    pub fn effective_scale(&self) -> f64 {
        if self.image.is_some() {
            self.scale
        } else {
            self.pending.map(|(_, s)| s).unwrap_or(1.0)
        }
    }
}

impl Bitmap {
    /// Movie-unit size scaled into this bitmap's hi-res space.
    #[inline]
    pub fn hi_res_dims(width: u16, height: u16, scale: f64) -> (u16, u16) {
        (
            ((width as f64 * scale).round() as i64).clamp(1, u16::MAX as i64) as u16,
            ((height as f64 * scale).round() as i64).clamp(1, u16::MAX as i64) as u16,
        )
    }

    /// Promise a hi-res twin without rendering it yet — see `HiResTwin::pending`.
    #[inline]
    pub fn promise_hi_res(&mut self, member: CastMemberRef, scale: f64) {
        if self.hi_res.banned || scale <= 1.0 || !scale.is_finite() {
            return;
        }
        self.hi_res.pending = Some((member, scale));
    }

    /// Attach a hi-res twin at `scale`.
    ///
    /// The twin MUST be exactly `width*scale` by `height*scale`, because every
    /// rect that later reaches it is the movie-unit rect multiplied by that one
    /// number. A re-run text layout does not land there on its own — rounding
    /// each line's height independently accumulates, so a 616px box at 2.077
    /// comes back 1276 tall rather than 1279 — so the twin is FITTED into a
    /// buffer of the exact size, top-left aligned: a couple of rows of padding
    /// (or of crop) at the far edge, where a text box has its slack anyway.
    ///
    /// A twin that is nowhere near the right size is refused outright rather
    /// than stretched — that would mean the two runs laid the text out
    /// differently, and a plausible-looking wrong picture is worse than the
    /// magnified one.
    pub fn set_hi_res(&mut self, hi: Bitmap, scale: f64) {
        if self.hi_res.banned || scale <= 1.0 || !scale.is_finite() || self.width == 0 || self.height == 0 {
            return;
        }
        let (want_w, want_h) = Self::hi_res_dims(self.width, self.height, scale);
        let off_w = hi.width.abs_diff(want_w) as f64 / want_w as f64;
        let off_h = hi.height.abs_diff(want_h) as f64 / want_h as f64;
        if off_w > 0.05 || off_h > 0.05 {
            return;
        }
        let bpp = hi.bit_depth as usize / 8;
        if bpp == 0 {
            return;
        }
        let fitted = if hi.width == want_w && hi.height == want_h {
            hi
        } else {
            let mut fitted = Bitmap::new(
                want_w,
                want_h,
                hi.bit_depth,
                hi.original_bit_depth,
                if hi.matte.is_some() { 8 } else { 0 },
                hi.palette_ref.clone(),
            );
            fitted.use_alpha = hi.use_alpha;
            // Text renders on a transparent ground, so the padding rows must be
            // transparent too; `Bitmap::new` fills 32-bit buffers with 0xFF
            // (opaque white), which would paint a bar across the bottom of the
            // twin. Only the copied region below is meaningful.
            if hi.bit_depth == 32 {
                fitted.data.fill(0);
            }
            let copy_w = hi.width.min(want_w) as usize;
            let copy_h = hi.height.min(want_h) as usize;
            for y in 0..copy_h {
                let src = y * hi.width as usize * bpp;
                let dst = y * want_w as usize * bpp;
                let n = copy_w * bpp;
                if src + n <= hi.data.len() && dst + n <= fitted.data.len() {
                    fitted.data[dst..dst + n].copy_from_slice(&hi.data[src..src + n]);
                }
            }
            fitted
        };
        self.hi_res.image = Some(Box::new(fitted));
        self.hi_res.scale = scale;
        self.hi_res.written = 0;
        self.hi_res.blits = 0;
        self.hi_res.pending = None;
    }

    /// Forget the hi-res twin. Called by every mutation that cannot keep it in
    /// step with `data` — the bitmap then simply renders magnified, as before.
    #[inline]
    pub fn invalidate_hi_res(&mut self) {
        self.hi_res.image = None;
        self.hi_res.scale = 1.0;
        self.hi_res.written = 0;
        self.hi_res.blits = 0;
        self.hi_res.blits = 0;
        self.hi_res.pending = None;
    }

    /// Drop the twin AND refuse to ever build another for this bitmap.
    ///
    /// For bitmaps known up front to be framebuffers — `(the stage).image`
    /// above all — rather than waiting for `note_hi_res_written`'s budget to
    /// discover it frame by frame.
    #[inline]
    pub fn ban_hi_res(&mut self) {
        self.invalidate_hi_res();
        self.hi_res.banned = true;
    }

    /// Charge `pixels` against the twin's write budget, and drop (and ban) the
    /// twin once this bitmap has behaved like a framebuffer for long enough.
    /// Returns true while the twin is still worth maintaining.
    pub fn note_hi_res_written(&mut self, pixels: u64) -> bool {
        let Some(hi) = self.hi_res.image.as_deref() else { return false };
        let budget = (hi.width as u64 * hi.height as u64).max(1) * HiResTwin::WRITE_BUDGET;
        self.hi_res.written = self.hi_res.written.saturating_add(pixels);
        self.hi_res.blits = self.hi_res.blits.saturating_add(1);
        if self.hi_res.written > budget || self.hi_res.blits > HiResTwin::BLIT_BUDGET {
            self.ban_hi_res();
            return false;
        }
        true
    }

    /// Give this bitmap a hi-res twin at `scale` if it has none, by magnifying
    /// its current contents. Used when a hi-res source is copied into a plain
    /// destination: the parts already there stay as sharp as they were (i.e.
    /// not at all), and the incoming text lands sharp.
    pub fn ensure_hi_res(&mut self, scale: f64) {
        if self.hi_res.is_some() || self.hi_res.banned || scale <= 1.0 || !scale.is_finite() {
            return;
        }
        // Sub-byte depths (1/2/4) are packed; magnifying them here would need
        // the bit arithmetic every other path already has, for members that in
        // practice never carry composed text. Leave them on the magnify path.
        let bpp = self.bit_depth as usize / 8;
        if bpp == 0 || self.width == 0 || self.height == 0 {
            return;
        }
        let (w, h) = Self::hi_res_dims(self.width, self.height, scale);
        let mut hi = Bitmap::new(
            w,
            h,
            self.bit_depth,
            self.original_bit_depth,
            if self.matte.is_some() { 8 } else { 0 },
            self.palette_ref.clone(),
        );
        hi.use_alpha = self.use_alpha;
        // Nearest-neighbour magnify: this is the fallback content, and the
        // whole point of the hi-res path is that it is never resampled twice.
        if hi.data.len() < w as usize * h as usize * bpp
            || self.data.len() < self.width as usize * self.height as usize * bpp
        {
            return;
        }
        for y in 0..h as usize {
            let sy = (((y as f64 + 0.5) / scale).floor() as usize).min(self.height as usize - 1);
            for x in 0..w as usize {
                let sx = (((x as f64 + 0.5) / scale).floor() as usize).min(self.width as usize - 1);
                let src = (sy * self.width as usize + sx) * bpp;
                let dst = (y * w as usize + x) * bpp;
                hi.data[dst..dst + bpp].copy_from_slice(&self.data[src..src + bpp]);
            }
        }
        self.hi_res.image = Some(Box::new(hi));
        self.hi_res.scale = scale;
        self.hi_res.written = 0;
        self.hi_res.blits = 0;
    }
}

impl Bitmap {
    /// The `rect` of this 32-bit bitmap as a new 32-bit bitmap, RGBA copied
    /// byte for byte. A crop keeps the alpha channel; going through an ink
    /// path does not, since ink Copy skips fully transparent source pixels
    /// and leaves the destination's initial fill behind them. A rect that
    /// runs past the source is padded with transparent pixels when the image
    /// has an alpha channel; Bitmap::new's white showed as a bar under a claw
    /// a movie cropped taller than its image.
    pub fn crop_rgba(&self, left: i32, top: i32, width: u16, height: u16) -> Bitmap {
        let mut out = Bitmap::new(width, height, 32, 32, if self.use_alpha { 8 } else { 0 }, self.palette_ref.clone());
        if self.use_alpha {
            out.data.fill(0);
        }
        out.use_alpha = self.use_alpha;
        out.trim_white_space = self.trim_white_space;
        let sw = self.width as i32;
        let sh = self.height as i32;
        for dy in 0..height as i32 {
            let sy = top + dy;
            if sy < 0 || sy >= sh { continue; }
            for dx in 0..width as i32 {
                let sx = left + dx;
                if sx < 0 || sx >= sw { continue; }
                let si = ((sy * sw + sx) * 4) as usize;
                let di = ((dy * width as i32 + dx) * 4) as usize;
                out.data[di..di + 4].copy_from_slice(&self.data[si..si + 4]);
            }
        }
        out
    }

    pub fn new(
        width: u16,
        height: u16,
        bit_depth: u8,
        original_bit_depth: u8,
        alpha_depth: u8,
        palette_ref: PaletteRef,
    ) -> Self {
        let initial_color = match bit_depth {
            16 | 32 => 255,
            _ => 0,
        };

        // Backstop against absurd allocations: a bad/oversized width×height (e.g.
        // a nested sub-movie's mis-measured field height of ~15000 px, or a
        // transient huge stage rect) makes the `vec![…]` below abort the whole
        // WASM module with `handle_alloc_error` — an unrecoverable crash. Movie
        // bitmaps are never larger than a few thousand px per side; clamp to 1×1
        // (and log) rather than take down the player.
        if (width as usize).saturating_mul(height as usize) > 8192 * 8192 {
            warn!(
                "[bitmap] refusing absurd Bitmap::new({}x{} depth={}) — clamped to 1x1",
                width, height, bit_depth
            );
            return Self::new(1, 1, bit_depth, original_bit_depth, alpha_depth, palette_ref);
        }

        // `bit_depth as usize / 8` truncates to 0 for sub-byte depths
        // (1, 2, 4), leaving `data` empty even though set_pixel does packed
        // bit/nibble arithmetic on it. Compute bytes from total bits so
        // sub-byte bitmaps get a properly sized buffer.
        let total_bits = width as usize * height as usize * bit_depth as usize;
        let total_bytes = (total_bits + 7) / 8;
        let data = vec![initial_color; total_bytes];

        // For 32-bit images, always create a matte OR handle alpha in the data
        let matte = if alpha_depth > 0 || bit_depth == 32 {
            Some(Arc::new(BitmapMask::new(
                width.try_into().unwrap(),
                height.try_into().unwrap(),
                true, // fill mask if alpha exists
            )))
        } else {
            None
        };

        Self {
            width,
            height,
            bit_depth,
            original_bit_depth,
            data,
            palette_ref,
            matte,
            use_alpha: false,
            trim_white_space: false,
            was_trimmed: false,
            version: 0,
            hi_res: HiResTwin::default(),
        }
    }

    /// Increment the version counter to indicate the bitmap data has changed.
    /// This is used by the WebGL2 texture cache to know when to re-upload textures.
    pub fn mark_dirty(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
}

fn get_num_channels(bit_depth: u8) -> Result<u8, String> {
    match bit_depth {
        1 | 2 | 4 | 8 => Ok(1),  // 8-bit and below: 1 byte per pixel
        16 => Ok(2),              // 16-bit: 2 bytes per pixel
        32 => Ok(4),              // 32-bit: 4 bytes per pixel
        _ => Err("Invalid bit depth".to_string()),
    }
}

fn get_alignment_width(bit_depth: u8) -> Result<u16, String> {
    match bit_depth {
        // 1-bit: rows aligned to 16-bit (word) boundaries = 16 pixels per row minimum
        1 => Ok(16),
        4 | 32 => Ok(4),
        2 | 8 => Ok(2),
        16 => Ok(1),  // 16-bit aligns like 8-bit (1 byte per pixel)
        _ => Err("Invalid bit depth".to_string()),
    }
}

fn decode_bitmap_1bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    // Decodes 1-bit to 8-bit indexed
    let mut scan_data = vec![0; data.len() * 8];
    let mut p = 0;
    for i in 0..data.len() {
        let byte = data[i];
        for j in 1..=8 {
            let bit = (byte & (0x1 << (8 - j))) >> (8 - j);
            scan_data[p] = if bit == 1 { 0xFF } else { 0x00 };
            p += 1;
        }
    }

    let mut result = vec![0; width as usize * height as usize];
    for y in 0..scan_height {
        for x in 0..scan_width {
            // Use usize arithmetic to avoid u16 overflow for large images
            // e.g., y=152, scan_width=432 -> 152*432=65664 which overflows u16 (max 65535)
            let scan_index = y as usize * scan_width as usize + x as usize;
            if x < width {
                let pixel_index = y as usize * width as usize + x as usize;
                if scan_index >= scan_data.len() {
                    return Err(format!(
                        "decode_bitmap_1bit: scan_index {} >= scan_data.len() {}",
                        scan_index,
                        scan_data.len()
                    ));
                }
                let pixel = scan_data[scan_index];
                result[pixel_index] = pixel;
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 8,
        original_bit_depth: 1, // Keep original 1-bit depth for proper Director rendering semantics
        width,
        height,
        data: result,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
        hi_res: HiResTwin::default(),
    })
}

fn decode_bitmap_2bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    let mut decoded_data = Vec::new();

    for i in 0..data.len() {
        let original_value = data[i];
        let left_value = (original_value & 0xC0) >> 6;
        let middle_left_value = (original_value & 0x30) >> 4;
        let middle_right_value = (original_value & 0x0C) >> 2;
        let right_value = original_value & 0x03;

        // Keep raw 2-bit palette indices (0-3), like 4-bit decode keeps 0-15.
        // Color resolution happens later via palette lookup.
        decoded_data.push(left_value);
        decoded_data.push(middle_left_value);
        decoded_data.push(middle_right_value);
        decoded_data.push(right_value);
    }

    let mut result_bmp = vec![0; width as usize * height as usize];
    for y in 0..scan_height {
        for x in 0..scan_width {
            let compressed_index = y as usize * scan_width as usize + x as usize;
            if compressed_index >= decoded_data.len() {
                return Err(format!(
                    "decode_bitmap_2bit: compressed_index {} >= decoded_data.len() {}",
                    compressed_index,
                    decoded_data.len()
                ));
            }
            if x < width {
                let pixel_index = y as usize * width as usize + x as usize;
                let pixel = decoded_data[compressed_index];
                result_bmp[pixel_index] = pixel;
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 8,
        original_bit_depth: 2,
        width,
        height,
        data: result_bmp,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
        hi_res: HiResTwin::default(),
    })
}

fn decode_bitmap_4bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    // Decode 4-bit data to 8-bit indexed (each nibble becomes a byte with value 0-15)
    let mut decoded_data = Vec::new();

    for i in 0..data.len() {
        let original_value = data[i];
        let left_value = (original_value & 0xF0) >> 4;
        let right_value = original_value & 0x0F;

        decoded_data.push(left_value);
        decoded_data.push(right_value);
    }

    // Create result as 8-bit indexed (one byte per pixel)
    let mut result_bmp = vec![0; width as usize * height as usize];

    for y in 0..height {
        for x in 0..width {
            let scan_index = y as usize * scan_width as usize + x as usize;

            if scan_index >= decoded_data.len() {
                return Err(format!(
                    "decode_bitmap_4bit: scan_index {} >= decoded_data.len() {}",
                    scan_index,
                    decoded_data.len()
                ));
            }

            let pixel = decoded_data[scan_index];
            let pixel_index = y as usize * width as usize + x as usize;

            // Store as 8-bit indexed (values 0-15)
            result_bmp[pixel_index] = pixel;
        }
    }

    Ok(Bitmap {
        bit_depth: 8,          // Stored as 8-bit
        original_bit_depth: 4, // But was originally 4-bit
        width,
        height,
        data: result_bmp,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
        hi_res: HiResTwin::default(),
    })
}

fn decode_bitmap_16bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
    skip_compression: bool,
) -> Result<Bitmap, String> {
    let expected_size = scan_width as usize * scan_height as usize * 2;

    if data.len() < expected_size {
        return Err(format!(
            "16-bit bitmap: insufficient data (got {}, expected {})",
            data.len(), expected_size
        ));
    }

    let mut result = vec![0u8; width as usize * height as usize * 4];

    for y in 0..height as usize {
        for x in 0..width as usize {
            let pixel16: u16 = if skip_compression {
                // Uncompressed: sequential bytes, 2 per pixel
                // High byte followed by low byte
                let offset = (y * scan_width as usize + x) * 2;
                let high = data[offset];
                let low = data[offset + 1];
                u16::from_be_bytes([high, low])
            } else {
                // Compressed (RLE-decoded): planar per scanline
                // For each row: all high bytes, then all low bytes
                let row_offset = y * scan_width as usize * 2;
                let high = data[row_offset + x];
                let low = data[row_offset + scan_width as usize + x];
                u16::from_be_bytes([high, low])
            };

            // RGB555 - extract 5-bit components
            let r5 = ((pixel16 >> 10) & 0x1F) as u8;
            let g5 = ((pixel16 >> 5) & 0x1F) as u8;
            let b5 = (pixel16 & 0x1F) as u8;

            // Convert 5-bit to 8-bit by shifting left and filling lower bits
            let dst = (y * width as usize + x) * 4;
            result[dst]     = (r5 << 3) | (r5 >> 2);
            result[dst + 1] = (g5 << 3) | (g5 >> 2);
            result[dst + 2] = (b5 << 3) | (b5 >> 2);
            result[dst + 3] = 255;
        }
    }

    Ok(Bitmap {
        width,
        height,
        bit_depth: 32,
        original_bit_depth: 16,
        data: result,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
        hi_res: HiResTwin::default(),
    })
}

fn decode_generic_bitmap(
    width: u16,
    height: u16,
    bit_depth: u8,
    num_channels: u8,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    // Sanity check: prevent capacity overflow from garbage BitmapInfo values
    const MAX_BITMAP_PIXELS: usize = 8192 * 8192; // 64 megapixels
    let total_pixels = width as usize * height as usize;
    if total_pixels > MAX_BITMAP_PIXELS {
        return Err(format!(
            "decode_generic_bitmap: bitmap {}x{} exceeds maximum size",
            width, height
        ));
    }

    let bytes_per_pixel = bit_depth / 8;
    let expected_size = scan_width as usize * scan_height as usize * num_channels as usize * bytes_per_pixel as usize;

    if expected_size != data.len() {
        warn!(
            "decode_generic_bitmap: Expected {} bytes, got {}",
            expected_size,
            data.len()
        );
        let actual_bit_depth = bit_depth * num_channels;
        return Ok(Bitmap::new(
            width,
            height,
            actual_bit_depth,
            bit_depth,
            0,
            palette_ref,
        ));
    } else {
        let mut result =
            vec![
                0;
                width as usize * height as usize * num_channels as usize * bytes_per_pixel as usize
            ];

        // FIX: The indexing was wrong - channels and bytes should be multiplied, not added
        for y in 0..scan_height {
            for x in 0..scan_width {
                if x >= width {
                    continue;
                }
                for c in 0..num_channels {
                    for b in 0..bytes_per_pixel {
                        let scan_index = (y as usize
                            * scan_width as usize
                            * num_channels as usize
                            * bytes_per_pixel as usize)
                            + (x as usize * num_channels as usize * bytes_per_pixel as usize)
                            + (c as usize * bytes_per_pixel as usize)
                            + b as usize;

                        let result_index = (y as usize
                            * width as usize
                            * num_channels as usize
                            * bytes_per_pixel as usize)
                            + (x as usize * num_channels as usize * bytes_per_pixel as usize)
                            + (c as usize * bytes_per_pixel as usize)
                            + b as usize;

                        if scan_index >= data.len() || result_index >= result.len() {
                            warn!(
                                "decode_generic_bitmap: scan_index {} >= data.len() {} or result_index {} >= result.len() {}",
                                scan_index,
                                data.len(),
                                result_index,
                                result.len()
                            );
                            continue;
                        }
                        result[result_index] = data[scan_index];
                    }
                }
            }
        }

        let actual_bit_depth = bit_depth * num_channels;
        return Ok(Bitmap {
            width,
            height,
            bit_depth: actual_bit_depth,
            original_bit_depth: bit_depth,
            data: result,
            palette_ref,
            matte: None,
            use_alpha: false,
            trim_white_space: false,
            was_trimmed: false,
            version: 0,
            hi_res: HiResTwin::default(),
        });
    }
}

pub fn bitmap_to_hex_string(bitmap: &Bitmap) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# width={} height={} bit_depth={} data_len={}\n",
        bitmap.width,
        bitmap.height,
        bitmap.bit_depth,
        bitmap.data.len()
    ));

    let bytes_per_row = (bitmap.data.len() as f64 / bitmap.height as f64).ceil() as usize;
    for (i, b) in bitmap.data.iter().enumerate() {
        s.push_str(&format!("{:02X}", b));
        if (i + 1) % bytes_per_row == 0 {
            s.push('\n');
        } else {
            s.push(' ');
        }
    }

    s
}

// Converts a NUU-encoded bitmap to a raw bitmap
pub fn decompress_bitmap(
    data: &[u8],
    info: &BitmapInfo,
    cast_lib: u32,
    version: u16,
) -> Result<Bitmap, String> {
    // Check if the BITD data is actually JPEG-compressed
    if data.len() >= 4 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return decode_jpeg_bitd(data, info, cast_lib);
    }

    // Use clutCastLib from bitmap data if explicitly specified (> 0).
    // clut_cast_lib == 0 means "search all castLibs" — use 0 as sentinel so
    // resolve_unresolved_palette_refs can find the correct palette.
    // clut_cast_lib == -1 means "not set" — use the bitmap's own castLib.
    let palette_cast_lib = if info.clut_cast_lib > 0 {
        info.clut_cast_lib as u32
    } else if info.clut_cast_lib == 0 && info.palette_id > 0 {
        0 // sentinel: search all castLibs during resolution
    } else {
        cast_lib
    };

    let mut result = Vec::new();
    let mut _current_index = 0;
    let num_channels = get_num_channels(info.bit_depth)?;
    let alignment_width = get_alignment_width(info.bit_depth)?;

    let mut reader = BinaryReader::from_u8(data);

    let scan_height = info.height;
    let mut scan_width = if info.pitch > 0 && info.bit_depth > 0 {
        // Pitch is the row byte stride. Convert to pixel width:
        // scan_width = pitch * 8 / bit_depth
        (info.pitch as u32 * 8 / info.bit_depth as u32) as u16
    } else if info.width % alignment_width == 0 {
        info.width
    } else {
        alignment_width * info.width.div_ceil(alignment_width)
    };

    let expected_len = if info.bit_depth == 32 && version >= 400 {
        scan_width as usize * scan_height as usize * num_channels as usize
    } else if info.bit_depth == 1 {
        // For 1-bit: scan_width is in pixels, each row is scan_width/8 bytes
        (scan_width as usize / 8) * scan_height as usize
    } else if info.bit_depth == 2 {
        // For 2-bit: scan_width is in pixels, each row is scan_width/4 bytes
        (scan_width as usize / 4) * scan_height as usize
    } else if info.bit_depth == 4 {
        // For 4-bit: scan_width is in pixels, each row is scan_width/2 bytes
        (scan_width as usize / 2) * scan_height as usize
    } else {
        scan_width as usize * scan_height as usize * num_channels as usize
    };

    let data_was_uncompressed = reader.length >= expected_len;

    if data_was_uncompressed {
        result.extend_from_slice(&reader.data[..expected_len]);
    } else {
        while result.len() < expected_len {
            let control = match reader.read_u8() {
                Ok(v) => v as u16,
                Err(_) => break, // truncated stream is OK in Director
            };

            if control < 0x80 {
                // Literal run: copy next (control + 1) bytes
                let count = control + 1;
                for _ in 0..count {
                    if result.len() >= expected_len {
                        break;
                    }
                    match reader.read_u8() {
                        Ok(v) => result.push(v),
                        Err(_) => break,
                    }
                }
            } else if control == 0x80 {
                // No-op: skip this byte (PackBits standard)
                continue;
            } else {
                // Repeat run: repeat next byte (257 - control) times
                let count = 257 - control;
                let val = match reader.read_u8() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                for _ in 0..count {
                    if result.len() >= expected_len {
                        break;
                    }
                    result.push(val);
                }
            }
        }
    }

    if info.pitch > 0 {
        // Pitch was provided — keep the pitch-based scan_width
    } else if result.len() == info.width as usize * info.height as usize * num_channels as usize {
        scan_width = info.width;
    } else if info.bit_depth == 32 && version >= 400 {
        // For 32-bit D4+ format without pitch info, use actual width (no padding)
        scan_width = info.width;
    } else if info.width % alignment_width == 0 {
        scan_width = info.width;
    } else {
        scan_width = alignment_width * info.width.div_ceil(alignment_width);
    }

    let mut bitmap = match info.bit_depth {
        1 => decode_bitmap_1bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        2 => decode_bitmap_2bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        4 => decode_bitmap_4bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        8 => decode_generic_bitmap(
            info.width,
            info.height,
            8,
            1,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        16 => decode_bitmap_16bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
            data_was_uncompressed,
        ),
        32 => {
            // For 32-bit bitmaps in Director, the encoding is special:
            // - Uncompressed data: direct interleaved ARGB (any version)
            // - D3 and below: always direct ARGB
            // - D4+: RLE compressed, with each scanline containing A R G B channels separately (planar)
            //
            // When `data_was_uncompressed` is true, the raw data was already the expected size
            // so no RLE decompression was applied — the data is in direct ARGB format.

            let is_direct_format = if data_was_uncompressed {
                true // Uncompressed data is always interleaved ARGB
            } else if version < 300 {
                result.len() >= (info.width as usize * info.height as usize * 4)
            } else if version < 400 {
                result.len() == (info.width as usize * info.height as usize * 4)
            } else {
                false
            };

            if is_direct_format {
                // Direct ARGB format (uncompressed data, or D3)
                let mut result_bitmap = decode_generic_bitmap(
                    info.width,
                    info.height,
                    8,
                    4,
                    scan_width,
                    scan_height,
                    PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
                    &result,
                )?;

                // Convert from ARGB (file format) to RGBA (internal format).
                // The rest of the code reads 32-bit pixels as [R, G, B, A] at each
                // 4-byte offset, but direct format stores [A, R, G, B].
                let data = &mut result_bitmap.data;
                for i in (0..data.len()).step_by(4) {
                    let a = data[i];
                    let r = data[i + 1];
                    let g = data[i + 2];
                    let b = data[i + 3];
                    data[i] = r;
                    data[i + 1] = g;
                    data[i + 2] = b;
                    data[i + 3] = a;
                }

                Ok(Bitmap {
                    width: result_bitmap.width,
                    height: result_bitmap.height,
                    bit_depth: 32,
                    original_bit_depth: 32,
                    data: result_bitmap.data,
                    palette_ref: PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
                    matte: None,
                    use_alpha: info.use_alpha,
                    trim_white_space: info.trim_white_space,
                    was_trimmed: false,
                    version: 0,
                    hi_res: HiResTwin::default(),
                })
            } else {
                // D4+ format: each scanline has channels laid out as A R G B sequentially
                // We need to reorder from [A...A][R...R][G...G][B...B] per line to ARGB per pixel
                let mut final_data = vec![0u8; info.width as usize * info.height as usize * 4];

                let mut oob_warned = false;
                for y in 0..info.height as usize {
                    for x in 0..info.width as usize {
                        let line_offset = y * scan_width as usize * 4;
                        let pixel_idx = (y * info.width as usize + x) * 4;

                        // Check bounds
                        if line_offset + x + 3 * scan_width as usize >= result.len() {
                            if !oob_warned {
                                web_sys::console::warn_1(&format!(
                                    "32-bit decode: Out of bounds at y={}, x={}. line_offset={}, result.len()={} (further warnings suppressed)",
                                    y, x, line_offset, result.len()
                                ).into());
                                oob_warned = true;
                            }
                            continue;
                        }

                        // Read from separate channels
                        let a = result[line_offset + x]; // Alpha
                        let r = result[line_offset + x + scan_width as usize]; // Red
                        let g = result[line_offset + x + 2 * scan_width as usize]; // Green
                        let b = result[line_offset + x + 3 * scan_width as usize]; // Blue

                        // Write as ARGB (or RGBA depending on your rendering system)
                        final_data[pixel_idx] = r;
                        final_data[pixel_idx + 1] = g;
                        final_data[pixel_idx + 2] = b;
                        final_data[pixel_idx + 3] = a;
                    }
                }

                Ok(Bitmap {
                    width: info.width,
                    height: info.height,
                    bit_depth: 32,
                    original_bit_depth: 32,
                    data: final_data,
                    palette_ref: PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
                    matte: None,
                    use_alpha: info.use_alpha,
                    trim_white_space: info.trim_white_space,
                    was_trimmed: false,
                    version: 0,
                    hi_res: HiResTwin::default(),
                })
            }
        }
        _ => Err(format!(
            "Decompression not implemented for bitmap width {}, height {}, bit depth {}",
            info.width, info.height, info.bit_depth
        )),
    }?;

    bitmap.use_alpha = info.use_alpha;
    bitmap.trim_white_space = info.trim_white_space;
    
    Ok(bitmap)
}

/// Encode a Bitmap back to raw byte format (pre-RLE).
/// This reverses the decoding done by decompress_bitmap.
pub fn encode_bitmap_data(bitmap: &Bitmap, target_bit_depth: u8, pitch: u16) -> Result<Vec<u8>, String> {
    let scan_width = if pitch > 0 && target_bit_depth > 0 {
        (pitch as u32 * 8 / target_bit_depth as u32) as u16
    } else {
        bitmap.width
    };

    match target_bit_depth {
        1 => encode_bitmap_1bit(bitmap, scan_width),
        2 => encode_bitmap_2bit(bitmap, scan_width),
        4 => encode_bitmap_4bit(bitmap, scan_width),
        8 => encode_bitmap_8bit(bitmap, scan_width),
        16 => encode_bitmap_16bit(bitmap, scan_width),
        32 => encode_bitmap_32bit(bitmap, scan_width),
        _ => Err(format!("Unsupported target bit depth for encoding: {}", target_bit_depth)),
    }
}

fn encode_bitmap_1bit(bitmap: &Bitmap, scan_width: u16) -> Result<Vec<u8>, String> {
    // Inverse of decode_bitmap_1bit: pack 8-bit indexed (0x00/0xFF) back to 1-bit packed
    let bytes_per_row = scan_width as usize / 8;
    let mut result = Vec::with_capacity(bytes_per_row * bitmap.height as usize);

    for y in 0..bitmap.height as usize {
        for byte_idx in 0..bytes_per_row {
            let mut byte = 0u8;
            for bit_idx in 0..8u8 {
                let x = byte_idx * 8 + bit_idx as usize;
                if x < bitmap.width as usize {
                    let pixel_index = y * bitmap.width as usize + x;
                    if bitmap.data[pixel_index] >= 0x80 {
                        byte |= 0x80 >> bit_idx;
                    }
                }
            }
            result.push(byte);
        }
    }
    Ok(result)
}

fn encode_bitmap_2bit(bitmap: &Bitmap, scan_width: u16) -> Result<Vec<u8>, String> {
    // Inverse of decode_bitmap_2bit: reverse float scaling and pack 4 pixels per byte
    let bytes_per_row = scan_width as usize / 4;
    let mut result = Vec::with_capacity(bytes_per_row * bitmap.height as usize);

    for y in 0..bitmap.height as usize {
        for byte_idx in 0..bytes_per_row {
            let mut byte = 0u8;
            for i in 0..4u8 {
                let x = byte_idx * 4 + i as usize;
                let val = if x < bitmap.width as usize {
                    let pixel = bitmap.data[y * bitmap.width as usize + x];
                    // Reverse: ((val as f32) / 3.0 * 255.0) → round(pixel * 3.0 / 255.0)
                    ((pixel as f32 * 3.0 / 255.0).round() as u8).min(3)
                } else {
                    0
                };
                byte |= val << (6 - i * 2);
            }
            result.push(byte);
        }
    }
    Ok(result)
}

fn encode_bitmap_4bit(bitmap: &Bitmap, scan_width: u16) -> Result<Vec<u8>, String> {
    // Inverse of decode_bitmap_4bit: pack pairs of 8-bit indexed (0-15) into nibbles
    let bytes_per_row = scan_width as usize / 2;
    let mut result = Vec::with_capacity(bytes_per_row * bitmap.height as usize);

    for y in 0..bitmap.height as usize {
        for byte_idx in 0..bytes_per_row {
            let x0 = byte_idx * 2;
            let x1 = byte_idx * 2 + 1;
            let v0 = if x0 < bitmap.width as usize {
                bitmap.data[y * bitmap.width as usize + x0] & 0x0F
            } else {
                0
            };
            let v1 = if x1 < bitmap.width as usize {
                bitmap.data[y * bitmap.width as usize + x1] & 0x0F
            } else {
                0
            };
            result.push((v0 << 4) | v1);
        }
    }
    Ok(result)
}

fn encode_bitmap_8bit(bitmap: &Bitmap, scan_width: u16) -> Result<Vec<u8>, String> {
    // Inverse of decode_generic_bitmap for 8-bit: copy pixel values with row padding
    let mut result = Vec::with_capacity(scan_width as usize * bitmap.height as usize);

    for y in 0..bitmap.height as usize {
        for x in 0..scan_width as usize {
            if x < bitmap.width as usize {
                let pixel_index = y * bitmap.width as usize + x;
                result.push(bitmap.data[pixel_index]);
            } else {
                result.push(0);
            }
        }
    }
    Ok(result)
}

fn encode_bitmap_16bit(bitmap: &Bitmap, scan_width: u16) -> Result<Vec<u8>, String> {
    // Inverse of decode_bitmap_16bit (RLE/planar format):
    // For each row: all high bytes of RGB555, then all low bytes
    let mut result = vec![0u8; scan_width as usize * bitmap.height as usize * 2];

    for y in 0..bitmap.height as usize {
        let row_offset = y * scan_width as usize * 2;
        for x in 0..scan_width as usize {
            let (high, low) = if x < bitmap.width as usize {
                let pixel_idx = (y * bitmap.width as usize + x) * 4;
                let r = bitmap.data[pixel_idx];
                let g = bitmap.data[pixel_idx + 1];
                let b = bitmap.data[pixel_idx + 2];
                // Convert to RGB555
                let r5 = (r >> 3) as u16;
                let g5 = (g >> 3) as u16;
                let b5 = (b >> 3) as u16;
                let pixel16 = (r5 << 10) | (g5 << 5) | b5;
                let bytes = pixel16.to_be_bytes();
                (bytes[0], bytes[1])
            } else {
                (0, 0)
            };
            result[row_offset + x] = high;
            result[row_offset + scan_width as usize + x] = low;
        }
    }
    Ok(result)
}

fn encode_bitmap_32bit(bitmap: &Bitmap, scan_width: u16) -> Result<Vec<u8>, String> {
    // Inverse of D4+ 32-bit decode: planar per scanline [A][R][G][B]
    let mut result = vec![0u8; scan_width as usize * bitmap.height as usize * 4];

    for y in 0..bitmap.height as usize {
        let line_offset = y * scan_width as usize * 4;
        for x in 0..scan_width as usize {
            if x < bitmap.width as usize {
                let pixel_idx = (y * bitmap.width as usize + x) * 4;
                let r = bitmap.data[pixel_idx];
                let g = bitmap.data[pixel_idx + 1];
                let b = bitmap.data[pixel_idx + 2];
                let a = bitmap.data[pixel_idx + 3];
                result[line_offset + x] = a;
                result[line_offset + scan_width as usize + x] = r;
                result[line_offset + 2 * scan_width as usize + x] = g;
                result[line_offset + 3 * scan_width as usize + x] = b;
            }
        }
    }
    Ok(result)
}

/// Compress data using PackBits RLE algorithm (inverse of the decompression in decompress_bitmap).
/// Control byte semantics:
///   0x00..0x7F: literal run of (control + 1) bytes follows
///   0x80:       no-op (not emitted by compressor)
///   0x81..0xFF: repeat run, next byte repeated (257 - control) times
///
/// Runs and literals are capped at 127 (not the PackBits theoretical max of
/// 128). Real Director / Shockwave never emits a 128-length op — its longest
/// repeat control is 0x82 (127×) and longest literal is 0x7E (127 bytes). The
/// old Director Multiuser xtra's RLE decoder sizes its run buffer for 127, so
/// a 128-length run (control 0x81) overruns it and crashes the connection when
/// the receiving client deserializes the media (observed: real Shockwave
/// disconnects while *viewing* a dirplayer photo). Matching the 127 cap keeps
/// dirplayer's media byte-compatible with what Director itself produces.
pub fn compress_bitmap(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let len = data.len();
    let mut i = 0;

    while i < len {
        // Look ahead for a run of identical bytes (capped at 127)
        let mut run_len = 1;
        while i + run_len < len && data[i + run_len] == data[i] && run_len < 127 {
            run_len += 1;
        }

        if run_len >= 3 {
            // Emit repeat run: control = (257 - count) as u8
            result.push((257 - run_len) as u8);
            result.push(data[i]);
            i += run_len;
        } else {
            // Collect literal bytes until we hit a run of 3+ (capped at 127)
            let start = i;
            let mut literal_len = 0;

            while i < len && literal_len < 127 {
                // Check if we're about to start a run of 3+ identical bytes
                if i + 2 < len && data[i] == data[i + 1] && data[i] == data[i + 2] {
                    break;
                }
                literal_len += 1;
                i += 1;
            }

            if literal_len > 0 {
                result.push((literal_len - 1) as u8);
                result.extend_from_slice(&data[start..start + literal_len]);
            }
        }
    }

    result
}

#[inline]
fn lookup_builtin_palette(palette: &BuiltInPalette, color_index: u8, original_bit_depth: u8) -> Option<(u8, u8, u8)> {
    match palette {
        BuiltInPalette::GrayScale => {
            // Uses 4-color palette for 2-bit images and 16-color palette for 4-bit images
            if original_bit_depth == 2 {
                GRAYSCALE_4_PALETTE.get(color_index as usize).copied()
            } else if original_bit_depth == 4 {
                GRAYSCALE_16_PALETTE.get(color_index as usize).copied()
            } else {
                GRAYSCALE_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::SystemMac => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                MAC_16_PALETTE.get(color_index as usize).copied()
            } else {
                SYSTEM_MAC_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::SystemWin => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                WIN_16_PALETTE.get(color_index as usize).copied()
            } else {
                SYSTEM_WIN_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Rainbow => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                RAINBOW16_PALETTE.get(color_index as usize).copied()
            } else {
                RAINBOW_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Pastels => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                PASTELS16_PALETTE.get(color_index as usize).copied()
            } else {
                PASTELS_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Vivid => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                VIVID16_PALETTE.get(color_index as usize).copied()
            } else {
                VIVID_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Ntsc => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                NTSC16_PALETTE.get(color_index as usize).copied()
            } else {
                NTSC_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Metallic => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                METALLIC16_PALETTE.get(color_index as usize).copied()
            } else {
                METALLIC_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Web216 => WEB_216_PALETTE.get(color_index as usize).copied(),
        // Vga and SystemWinDir4 fall back to SystemWin palette
        BuiltInPalette::Vga | BuiltInPalette::SystemWinDir4 => {
            if original_bit_depth == 4 {
                WIN_16_PALETTE.get(color_index as usize).copied()
            } else {
                SYSTEM_WIN_PALETTE.get(color_index as usize).copied()
            }
        }
    }
}

#[inline]
fn color_fallback(color_index: u8) -> (u8, u8, u8) {
    if color_index == 0 {
        (255, 255, 255)
    } else if color_index == 255 {
        (0, 0, 0)
    } else {
        (255, 0, 255) // magenta for missing colors
    }
}

#[inline]
pub fn resolve_color_ref(
    palettes: &PaletteMap,
    color_ref: &ColorRef,
    palette_ref: &PaletteRef,
    original_bit_depth: u8,
) -> (u8, u8, u8) {
    match color_ref {
        ColorRef::Rgb(r, g, b) => (*r, *g, *b),
        ColorRef::PaletteIndex(color_index) => {
            let idx = *color_index;
            match palette_ref {
                PaletteRef::BuiltIn(palette) => {
                    lookup_builtin_palette(palette, idx, original_bit_depth)
                        .unwrap_or_else(|| color_fallback(idx))
                }
                PaletteRef::Member(member_ref) => {
                    // cast_lib 0 = search all cast libs by member number
                    let palette_member = if member_ref.cast_lib == 0 {
                        palettes.find_by_member(member_ref.cast_member as u32)
                    } else {
                        let slot_number = CastMemberRefHandlers::get_cast_slot_number(
                            member_ref.cast_lib as u32,
                            member_ref.cast_member as u32,
                        );
                        palettes.get(slot_number as usize)
                            .or_else(|| palettes.find_by_member(member_ref.cast_member as u32))
                    };
                    if let Some(member) = palette_member {
                        member.colors.get(idx as usize).copied()
                            .unwrap_or_else(|| color_fallback(idx))
                    } else if let Some(member) = palettes.find_by_cast_lib(member_ref.cast_lib as u32) {
                        // Fallback: exact palette member not found (stale clutId from old numbering),
                        // use any palette in the same cast library
                        member.colors.get(idx as usize).copied()
                            .unwrap_or_else(|| color_fallback(idx))
                    } else {
                        lookup_builtin_palette(&get_system_default_palette(), idx, original_bit_depth)
                            .unwrap_or_else(|| color_fallback(idx))
                    }
                }
                PaletteRef::Default => {
                    // palette_id=0 means "no specific palette set" - use system default palette
                    lookup_builtin_palette(&get_system_default_palette(), idx, original_bit_depth)
                        .unwrap_or_else(|| color_fallback(idx))
                }
            }
        }
    }
}

/// Pre-resolve a full palette into an RGB lookup table.
/// For 4-bit bitmaps returns 16 entries, for 8-bit returns 256 entries.
/// This avoids calling `resolve_color_ref` per-pixel in hot loops.
#[inline]
pub fn resolve_palette_table(
    palettes: &PaletteMap,
    palette_ref: &PaletteRef,
    original_bit_depth: u8,
) -> Vec<(u8, u8, u8)> {
    // Always resolve all 256 entries. The internal bit_depth may be wider
    // than original_bit_depth (e.g. 1-bit stored as 8-bit), so
    // get_pixel_color_ref can return any PaletteIndex in 0..255.
    let mut table = Vec::with_capacity(256);
    for i in 0..256u16 {
        table.push(resolve_color_ref(
            palettes,
            &ColorRef::PaletteIndex(i as u8),
            palette_ref,
            original_bit_depth,
        ));
    }
    table
}

/// A resolved palette plus a memo of RGB -> nearest-palette-index answers.
///
/// Writing one pixel into an indexed bitmap means finding the closest palette
/// entry to an arbitrary RGB triple, which was an O(256) linear scan **per
/// destination pixel** inside `Bitmap::set_pixel_fast` — the dominant cost of
/// every software blit that can't take one of the raw index-copy fast paths
/// (anything scaled, rotated, flipped, blended, masked or cross-palette).
///
/// The memo is keyed on the EXACT 24-bit colour, so a hit returns precisely
/// what the linear scan would have returned — this is a pure speedup, not an
/// approximation. Blits have very high colour locality (a tile blit sees at
/// most the source palette's 256 colours), so after the first few rows almost
/// every lookup is a hit.
pub struct PaletteQuantizer {
    table: Vec<(u8, u8, u8)>,
    memo: std::cell::RefCell<fxhash::FxHashMap<u32, u8>>,
}

impl PaletteQuantizer {
    pub fn new(table: Vec<(u8, u8, u8)>) -> Self {
        Self { table, memo: std::cell::RefCell::new(fxhash::FxHashMap::default()) }
    }

    /// The quantizer used for a non-indexed (16/32-bit) destination, which
    /// never needs a nearest-colour search.
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    #[inline]
    pub fn table(&self) -> &[(u8, u8, u8)] {
        &self.table
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Nearest entry to `(r, g, b)` by the L1 channel distance, over the first
    /// `limit` entries. Ties go to the lowest index, matching the `<` in the
    /// scan this replaces.
    #[inline]
    pub fn nearest(&self, r: u8, g: u8, b: u8, limit: usize) -> u8 {
        // Only the full-palette (8-bit) case is memoised. The 4-bit case scans
        // 16 entries, which is cheaper than a hash lookup.
        if limit < self.table.len() {
            return Self::scan(&self.table, r, g, b, limit);
        }
        let key = (r as u32) << 16 | (g as u32) << 8 | b as u32;
        if let Some(&idx) = self.memo.borrow().get(&key) {
            return idx;
        }
        let idx = Self::scan(&self.table, r, g, b, limit);
        let mut memo = self.memo.borrow_mut();
        // A 32-bit photographic source quantized down to 8-bit can present
        // millions of distinct colours, none of them repeating. Cap the memo so
        // it stays a cache and not a leak; palette-indexed sources (the case
        // this exists for) never come close to the limit.
        if memo.len() >= 1 << 16 {
            memo.clear();
        }
        memo.insert(key, idx);
        idx
    }

    #[inline]
    fn scan(table: &[(u8, u8, u8)], r: u8, g: u8, b: u8, limit: usize) -> u8 {
        let mut result_index: u8 = 0;
        let mut result_distance = i32::MAX;
        for (idx, &(pr, pg, pb)) in table.iter().enumerate().take(limit) {
            let distance = (r as i32 - pr as i32).abs()
                + (g as i32 - pg as i32).abs()
                + (b as i32 - pb as i32).abs();
            if distance < result_distance {
                result_index = idx as u8;
                result_distance = distance;
            }
        }
        result_index
    }
}

/// How many distinct destination palettes keep a warm memo.
///
/// This must be more than one. A single slot looked sufficient but is not: a
/// frame typically blits into several offscreen bitmaps with different
/// palettes, and one slot thrashes between them, so every lookup misses and
/// pays the linear scan PLUS a hash insert — strictly worse than no memo at
/// all. A handful of slots covers the working set; the lookup is a linear walk
/// comparing resolved tables, which is trivial next to a per-pixel scan.
const QUANTIZER_CACHE_SLOTS: usize = 4;

thread_local! {
    /// Recently used destination quantizers, most-recent first, reused across
    /// `copyPixels` calls so the memos stay warm frame to frame. Validated by
    /// comparing the resolved palette table itself, which is self-invalidating
    /// — no palette-version plumbing to get wrong.
    static DST_QUANTIZERS: std::cell::RefCell<Vec<std::rc::Rc<PaletteQuantizer>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// `resolve_palette_table`, but returning a memoising quantizer, reusing a
/// cached one whenever the palette resolves to the same table.
pub fn resolve_palette_quantizer(
    palettes: &PaletteMap,
    palette_ref: &PaletteRef,
    original_bit_depth: u8,
) -> std::rc::Rc<PaletteQuantizer> {
    let table = resolve_palette_table(palettes, palette_ref, original_bit_depth);
    DST_QUANTIZERS.with(|cell| {
        let mut cache = cell.borrow_mut();
        if let Some(pos) = cache.iter().position(|q| q.table() == table.as_slice()) {
            // Move to front so the eviction below drops the coldest palette.
            let hit = cache.remove(pos);
            cache.insert(0, hit.clone());
            return hit;
        }
        let fresh = std::rc::Rc::new(PaletteQuantizer::new(table));
        cache.insert(0, fresh.clone());
        cache.truncate(QUANTIZER_CACHE_SLOTS);
        fresh
    })
}

/// Decompress PackBits/RLE-compressed alpha data with even-padded row width.
/// Director stores alpha rows padded to 2-byte boundaries; without accounting for
/// this, odd-width bitmaps get a cumulative 1-byte-per-row diagonal shear.
pub fn decompress_alpha_rle(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    let padded_width = (width + 1) & !1;
    let pixel_count = width * height;
    let padded_total = padded_width * height;
    let mut result = Vec::with_capacity(padded_total);
    let mut pos = 0;
    while result.len() < padded_total && pos < data.len() {
        let control = data[pos] as u16;
        pos += 1;

        if control < 0x80 {
            let count = (control + 1) as usize;
            for _ in 0..count {
                if result.len() >= padded_total || pos >= data.len() {
                    break;
                }
                result.push(data[pos]);
                pos += 1;
            }
        } else if control == 0x80 {
            continue;
        } else {
            let count = (257 - control) as usize;
            if pos >= data.len() { break; }
            let val = data[pos];
            pos += 1;
            for _ in 0..count {
                if result.len() >= padded_total {
                    break;
                }
                result.push(val);
            }
        }
    }

    // Strip row padding if needed
    if padded_width > width {
        let mut stripped = Vec::with_capacity(pixel_count);
        for row in 0..height {
            let row_start = row * padded_width;
            let row_end = row_start + width;
            if row_end <= result.len() {
                stripped.extend_from_slice(&result[row_start..row_end]);
            }
        }
        stripped
    } else {
        result
    }
}

/// Decode a JPEG-compressed BITD chunk. The BITD data starts with a JPEG stream (RGB),
/// optionally followed by a separate alpha channel after the JPEG end marker (FFD9).
fn decode_jpeg_bitd(data: &[u8], info: &BitmapInfo, cast_lib: u32) -> Result<Bitmap, String> {
    use image::ImageDecoder;
    use std::io::Cursor;

    // Find the end of the JPEG stream (last FFD9 marker)
    let mut jpeg_end_pos = data.len();
    for i in (0..data.len().saturating_sub(1)).rev() {
        if data[i] == 0xFF && data[i + 1] == 0xD9 {
            jpeg_end_pos = i + 2;
            break;
        }
    }

    let jpeg_data = &data[..jpeg_end_pos];
    let alpha_data = &data[jpeg_end_pos..];

    // Decode the JPEG
    let cursor = Cursor::new(jpeg_data);
    let decoder = image::codecs::jpeg::JpegDecoder::new(cursor)
        .map_err(|e| format!("Failed to create JPEG decoder for BITD: {}", e))?;

    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();

    let mut image_data = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut image_data)
        .map_err(|e| format!("Failed to read JPEG image from BITD: {}", e))?;

    let pixel_count = width as usize * height as usize;

    // Convert to RGBA, incorporating separate alpha channel if available
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);

    // Decompress alpha data if present (may be PackBits/RLE compressed)
    let alpha_bytes = if !alpha_data.is_empty() {
        let mut alpha_result = decompress_alpha_rle(alpha_data, width as usize, height as usize);

        // If RLE didn't expand to expected size, try raw
        if alpha_result.len() < pixel_count && alpha_data.len() >= pixel_count {
            alpha_result = alpha_data[..pixel_count].to_vec();
        }

        Some(alpha_result)
    } else {
        None
    };

    match color_type {
        image::ColorType::Rgb8 => {
            for (i, chunk) in image_data.chunks(3).enumerate() {
                rgba_data.push(chunk[0]);
                rgba_data.push(chunk[1]);
                rgba_data.push(chunk[2]);
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba_data.push(alpha);
            }
        }
        image::ColorType::L8 => {
            for (i, &gray) in image_data.iter().enumerate() {
                rgba_data.push(gray);
                rgba_data.push(gray);
                rgba_data.push(gray);
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba_data.push(alpha);
            }
        }
        _ => {
            return Err(format!("Unsupported JPEG color type in BITD: {:?}", color_type));
        }
    }

    Ok(Bitmap {
        width: width as u16,
        height: height as u16,
        bit_depth: 32,
        original_bit_depth: 32,
        data: rgba_data,
        palette_ref: PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
        matte: None,
        use_alpha: info.use_alpha,
        trim_white_space: info.trim_white_space,
        was_trimmed: false,
        version: 0,
        hi_res: HiResTwin::default(),
    })
}

pub fn decode_jpeg_bitmap(data: &[u8], info: &BitmapInfo, alfa_data: Option<&Vec<u8>>) -> Result<Bitmap, String> {
    use image::ImageDecoder;
    use std::io::Cursor;

    // Use the `image` crate to decode JPEG
    let cursor = Cursor::new(data);
    let decoder = image::codecs::jpeg::JpegDecoder::new(cursor)
        .map_err(|e| format!("Failed to create JPEG decoder: {}", e))?;

    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();
    let pixel_count = (width * height) as usize;

    let mut image_data = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut image_data)
        .map_err(|e| format!("Failed to read JPEG image: {}", e))?;

    // Decompress ALFA chunk data if present (PackBits/RLE compressed)
    let alpha_bytes = if let Some(raw_alfa) = alfa_data {
        let mut alpha_result = decompress_alpha_rle(raw_alfa, width as usize, height as usize);

        // If RLE didn't produce enough bytes, try treating as raw uncompressed
        if alpha_result.len() < pixel_count && raw_alfa.len() >= pixel_count {
            alpha_result = raw_alfa[..pixel_count].to_vec();
        }

        Some(alpha_result)
    } else {
        None
    };

    let has_alfa = alpha_bytes.is_some();

    // Convert to RGBA, incorporating ALFA channel if available
    let rgba_data = match color_type {
        image::ColorType::Rgb8 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for (i, chunk) in image_data.chunks(3).enumerate() {
                rgba.push(chunk[0]); // R
                rgba.push(chunk[1]); // G
                rgba.push(chunk[2]); // B
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba.push(alpha);
            }
            rgba
        }
        image::ColorType::L8 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for (i, &gray) in image_data.iter().enumerate() {
                rgba.push(gray);
                rgba.push(gray);
                rgba.push(gray);
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba.push(alpha);
            }
            rgba
        }
        _ => {
            return Err(format!("Unsupported JPEG color type: {:?}", color_type));
        }
    };

    Ok(Bitmap {
        width: width as u16,
        height: height as u16,
        bit_depth: 32,
        original_bit_depth: 32,
        data: rgba_data,
        palette_ref: PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
        matte: None,
        use_alpha: if has_alfa { info.use_alpha } else { false },
        trim_white_space: info.trim_white_space,
        was_trimmed: false,
        version: 0,
        hi_res: HiResTwin::default(),
    })
}

#[cfg(test)]
mod crop_tests {
    use super::*;

    #[test]
    fn crop_keeps_the_alpha_channel() {
        // 3x3, opaque red centre, everything else fully transparent.
        let mut src = Bitmap::new(3, 3, 32, 32, 8, PaletteRef::BuiltIn(BuiltInPalette::SystemWin));
        src.use_alpha = true;
        for i in 0..9 { src.data[i * 4..i * 4 + 4].copy_from_slice(&[0, 0, 0, 0]); }
        src.data[4 * 4..4 * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        let out = src.crop_rgba(1, 0, 2, 2); // columns 1..2, rows 0..1
        assert!(out.use_alpha);
        assert_eq!(&out.data[0..4], &[0, 0, 0, 0], "top-left of the crop is transparent");
        assert_eq!(&out.data[(1 * 2 + 0) * 4..(1 * 2 + 0) * 4 + 4], &[255, 0, 0, 255], "the red pixel lands at (0,1)");
        assert_eq!(&out.data[(1 * 2 + 1) * 4..(1 * 2 + 1) * 4 + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn crop_outside_the_source_is_transparent() {
        let mut src = Bitmap::new(2, 2, 32, 32, 8, PaletteRef::BuiltIn(BuiltInPalette::SystemWin));
        src.use_alpha = true;
        for i in 0..4 { src.data[i * 4..i * 4 + 4].copy_from_slice(&[9, 9, 9, 255]); }
        let out = src.crop_rgba(1, 1, 3, 3); // runs past the right and bottom edge
        assert_eq!(&out.data[0..4], &[9, 9, 9, 255]);
        assert_eq!(&out.data[(2 * 3 + 2) * 4..(2 * 3 + 2) * 4 + 4], &[0, 0, 0, 0], "padding past the source is transparent");
    }
}
