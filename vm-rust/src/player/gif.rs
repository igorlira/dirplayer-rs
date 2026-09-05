//! Animated GIF cast members.
//!
//! Director shipped an "Animated GIF Asset" Xtra, and a movie that uses it
//! stores the whole `.gif` file as the XMED payload of a type-15 (Xtra media)
//! cast member. Movies reach for it for things that move but are not film
//! loops: flowing liquid, smoke plumes, a shooting star.
//!
//! Without this the payload fell through to the styled-text parser, which
//! found no text and drew an empty white box where the animation belonged.
//!
//! Every frame is decoded once, up front, into its own `Bitmap`, so inks,
//! stretching and matte apply to a GIF as they do to any other bitmap; the
//! member just points at a different frame as time passes. That costs memory
//! for a large GIF and leaves the drawing path untouched.

use crate::director::enums::BitmapInfo;
use crate::player::bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef};
use crate::player::bitmap::manager::BitmapRef;
use crate::player::bitmap::manager::BitmapManager;

/// One decoded GIF: its frames as bitmaps, and how long each is shown.
#[derive(Clone, Debug)]
pub struct GifAnimation {
    pub frames: Vec<BitmapRef>,
    /// Milliseconds per frame, same length as `frames`.
    pub delays_ms: Vec<u32>,
    pub width: u16,
    pub height: u16,
    /// Index currently shown by the cast member.
    pub current: usize,
    /// `performance.now()` reading when the current frame was put up.
    pub last_switch_ms: f64,
    /// Cleared by `sprite(n).pause()`, set again by `resume()`.
    pub running: bool,
}

impl GifAnimation {
    /// Which frame should be showing at `now_ms`, advancing `current` past as
    /// many frames as the elapsed time covers. Returns true when it changed.
    pub fn advance(&mut self, now_ms: f64) -> bool {
        if !self.running || self.frames.len() < 2 {
            return false;
        }
        if self.last_switch_ms <= 0.0 {
            self.last_switch_ms = now_ms;
            return false;
        }
        let mut changed = false;
        // A delay of 0 means "as fast as possible"; clamped like browsers and
        // Director do, so a broken file cannot burn the frame budget.
        let mut guard = 0;
        loop {
            let delay = (*self.delays_ms.get(self.current).unwrap_or(&100)).max(20) as f64;
            if now_ms - self.last_switch_ms < delay {
                break;
            }
            self.last_switch_ms += delay;
            self.current = (self.current + 1) % self.frames.len();
            changed = true;
            guard += 1;
            if guard > 64 {
                // Long pause (tab hidden): resync instead of catching up frame
                // by frame.
                self.last_switch_ms = now_ms;
                break;
            }
        }
        changed
    }

    pub fn current_ref(&self) -> BitmapRef {
        self.frames[self.current.min(self.frames.len() - 1)]
    }
}

/// The GIF's declared background colour, from the Logical Screen Descriptor's
/// background-colour index into the global colour table. Director's GIF Xtra
/// leaves that colour in the pixels, and a movie hides it by drawing the
/// sprite with ink 36 (Background Transparent), which keys out the MEMBER's
/// background colour. So the member has to carry it, or a GIF whose
/// background is not white draws as a solid box.
///
/// None when the GIF has no global colour table.
pub fn background_color(data: &[u8]) -> Option<(u8, u8, u8)> {
    if data.len() < 13 {
        return None;
    }
    let flags = data[10];
    let has_global_table = flags & 0x80 != 0;
    if !has_global_table {
        return None;
    }
    let table_size = 2usize << (flags & 0x07);
    let bg_index = data[11] as usize;
    let table_start = 13;
    if bg_index >= table_size || table_start + table_size * 3 > data.len() {
        return None;
    }
    let o = table_start + bg_index * 3;
    Some((data[o], data[o + 1], data[o + 2]))
}

/// True when these bytes are a GIF file.
pub fn is_gif(data: &[u8]) -> bool {
    data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a")
}

/// Decode every frame into the bitmap manager. Returns None if the data is not
/// a GIF the decoder accepts.
pub fn decode_gif(data: &[u8], bitmap_manager: &mut BitmapManager) -> Option<GifAnimation> {
    use image::AnimationDecoder;
    use image::codecs::gif::GifDecoder;

    if !is_gif(data) {
        return None;
    }
    let decoder = match GifDecoder::new(std::io::Cursor::new(data)) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("[gif] decoder refused {} bytes: {}", data.len(), e);
            return None;
        }
    };
    let mut frames = Vec::new();
    let mut delays_ms = Vec::new();
    let mut width = 0u16;
    let mut height = 0u16;
    for (i, frame) in decoder.into_frames().enumerate() {
        let frame = match frame {
            Ok(f) => f,
            Err(e) => {
                log::warn!("[gif] frame {} failed: {}", i, e);
                break;
            }
        };
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = if den == 0 { 100 } else { num / den.max(1) };
        let buf = frame.into_buffer();
        let (w, h) = (buf.width() as u16, buf.height() as u16);
        if w == 0 || h == 0 {
            continue;
        }
        width = width.max(w);
        height = height.max(h);
        let mut bitmap = Bitmap::new(w, h, 32, 32, 8, PaletteRef::BuiltIn(BuiltInPalette::SystemWin));
        bitmap.data = buf.into_raw();
        bitmap.use_alpha = true;
        frames.push(bitmap_manager.add_bitmap(bitmap));
        delays_ms.push(delay.max(20));
        // A runaway file must not exhaust memory.
        if frames.len() >= 240 {
            log::warn!("[gif] stopping at 240 frames");
            break;
        }
    }
    if frames.is_empty() {
        return None;
    }
    Some(GifAnimation {
        frames,
        delays_ms,
        width,
        height,
        current: 0,
        last_switch_ms: 0.0,
        running: true,
    })
}

/// The BitmapInfo a GIF frame should report: its own size, no palette of its
/// own (the frames are already RGBA), registration in the centre like
/// Director's own imported bitmaps.
pub fn gif_bitmap_info(width: u16, height: u16) -> BitmapInfo {
    BitmapInfo {
        width,
        height,
        reg_x: (width / 2) as i16,
        reg_y: (height / 2) as i16,
        bit_depth: 32,
        palette_id: 0,
        clut_cast_lib: -1,
        pitch: width.saturating_mul(4),
        use_alpha: true,
        trim_white_space: false,
        center_reg_point: true,
    }
}

/// Decoding happens while the cast is being built, before the player exists,
/// so finished animations wait here until `take_pending` moves them onto the
/// player. Keyed by (cast_lib, member number).
static PENDING: std::sync::Mutex<Option<Vec<((u32, u32), GifAnimation)>>> =
    std::sync::Mutex::new(None);

pub fn register_pending(cast_lib: u32, number: u32, anim: GifAnimation) {
    if let Ok(mut guard) = PENDING.lock() {
        guard.get_or_insert_with(Vec::new).push(((cast_lib, number), anim));
    }
}

pub fn take_pending() -> Vec<((u32, u32), GifAnimation)> {
    match PENDING.lock() {
        Ok(mut guard) => guard.take().unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Advance every animation and point its cast member at the current frame.
/// Called once per frame from the movie loop.
pub fn tick_gif_animations() {
    crate::player::reserve_player_mut(|player| {
        // The same clock the movie's own `the milliSeconds` reads, so a GIF
        // keeps step with the Lingo that drives the rest of the scene.
        let now = (chrono::Local::now().timestamp_millis()
            - player.system_start_time.timestamp_millis()) as f64;
        if player.gif_animations.is_empty() {
            return;
        }
        let mut moved: Vec<((u32, u32), BitmapRef)> = Vec::new();
        for (key, anim) in player.gif_animations.iter_mut() {
            if anim.advance(now) {
                moved.push((*key, anim.current_ref()));
            }
        }
        for ((cast_lib, number), image_ref) in moved {
            let member_ref = crate::CastMemberRef { cast_lib: cast_lib as i32, cast_member: number as i32 };
            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                if let crate::player::cast_member::CastMemberType::Bitmap(b) = &mut member.member_type {
                    b.image_ref = image_ref;
                }
            }
        }
    });
}

/// Move animations decoded during cast loading onto the player.
pub fn install_pending() {
    let pending = take_pending();
    if pending.is_empty() {
        return;
    }
    crate::player::reserve_player_mut(|player| {
        for (key, anim) in pending {
            player.gif_animations.insert(key, anim);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::bitmap::manager::BitmapManager;

    /// A two-frame GIF, 2x2, built by hand: red frame then green frame.
    fn tiny_gif() -> Vec<u8> {
        let mut g = Vec::new();
        g.extend_from_slice(b"GIF89a");
        g.extend_from_slice(&[2, 0, 2, 0]);          // 2x2
        g.extend_from_slice(&[0x80, 0, 0]);           // global table, 2 colours
        g.extend_from_slice(&[255, 0, 0, 0, 255, 0]); // red, green
        for idx in [0u8, 1u8] {
            g.extend_from_slice(&[0x21, 0xF9, 4, 0, 5, 0, 0, 0]); // 50 ms delay
            g.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0]);
            // LZW, 2-bit codes: clear, idx x4, end
            g.push(2);
            let data: Vec<u8> = match idx {
                0 => vec![0x84, 0x8F, 0xA9, 0xCB, 0xED, 0x0F, 0x00],
                _ => vec![0x8C, 0x2D, 0x99, 0x87, 0x2A, 0x1C, 0xDC, 0x33, 0xA0, 0x02, 0x75, 0xEC, 0x95, 0xFA, 0xA8, 0xDE, 0x60, 0x8C, 0x04, 0x91, 0x4C, 0x01, 0x00],
            };
            g.push(data.len() as u8);
            g.extend_from_slice(&data);
            g.push(0);
        }
        g.push(0x3B);
        g
    }

    #[test]
    fn recognises_both_gif_signatures() {
        assert!(is_gif(b"GIF87a....."));
        assert!(is_gif(b"GIF89a....."));
        assert!(!is_gif(b"FFFF000000060004"));   // a styled-text XMED payload
        assert!(!is_gif(b"3DGM"));
        assert!(!is_gif(b"GIF"));
    }

    #[test]
    fn decodes_frames_into_bitmaps() {
        let mut mgr = BitmapManager::new();
        let anim = decode_gif(&tiny_gif(), &mut mgr).expect("tiny gif should decode");
        assert!(anim.frames.len() >= 1, "at least one frame");
        assert_eq!(anim.width, 2);
        assert_eq!(anim.height, 2);
        let bmp = mgr.get_bitmap(anim.frames[0]).expect("frame 0 in manager");
        assert_eq!((bmp.width, bmp.height), (2, 2));
        assert_eq!(bmp.data.len(), 2 * 2 * 4, "RGBA");
    }

    #[test]
    fn advances_on_its_own_delays_and_wraps() {
        let mut anim = GifAnimation {
            frames: vec![1, 2, 3],
            delays_ms: vec![100, 100, 100],
            width: 4, height: 4, current: 0, last_switch_ms: 0.0, running: true,
        };
        // First call only starts the clock.
        assert!(!anim.advance(1000.0));
        assert_eq!(anim.current, 0);
        assert!(!anim.advance(1050.0), "half a delay is not a frame");
        assert!(anim.advance(1100.0));
        assert_eq!(anim.current, 1);
        assert!(anim.advance(1300.0), "two delays at once");
        assert_eq!(anim.current, 0, "wraps back to the start");
    }

    #[test]
    fn single_frame_never_advances() {
        let mut anim = GifAnimation {
            frames: vec![7], delays_ms: vec![100],
            width: 1, height: 1, current: 0, last_switch_ms: 0.0, running: true,
        };
        assert!(!anim.advance(9999.0));
        assert_eq!(anim.current_ref(), 7);
    }

    #[test]
    fn a_long_pause_resyncs_instead_of_catching_up() {
        let mut anim = GifAnimation {
            frames: vec![1, 2], delays_ms: vec![20, 20],
            width: 1, height: 1, current: 0, last_switch_ms: 0.0, running: true,
        };
        anim.advance(0.0);
        // Tab hidden for a minute: 3000 frames' worth of delay.
        anim.advance(60_000.0);
        assert_eq!(anim.last_switch_ms, 60_000.0, "clock resynced");
    }
}

/// True when this member number is a GIF the player has loaded.
pub fn is_gif_member(player: &crate::player::DirPlayer, cast_lib: i32, number: i32) -> bool {
    cast_lib >= 0 && number >= 0
        && player.gif_animations.contains_key(&(cast_lib as u32, number as u32))
}

/// True when this sprite's member is an animated GIF.
pub fn sprite_has_gif(datum: &crate::player::datum_ref::DatumRef) -> bool {
    crate::player::reserve_player_ref(|player| {
        let sprite_num = match player.get_datum(datum) {
            crate::director::lingo::datum::Datum::SpriteRef(n) => *n,
            _ => return false,
        };
        player
            .movie
            .score
            .get_sprite(sprite_num)
            .and_then(|s| s.member.as_ref())
            .is_some_and(|m| is_gif_member(player, m.cast_lib, m.cast_member))
    })
}

/// `pause`, `resume`, `rewind`, `play` and `stop` on a sprite showing a GIF.
/// Director's Animated GIF Xtra treats play and resume alike, and stop like
/// pause; rewind returns to the first frame and leaves the running state as it
/// was, which is why movies pair `rewind()` with `pause()`.
pub fn control_sprite_gif(
    datum: &crate::player::datum_ref::DatumRef,
    handler: &str,
) -> Result<crate::player::datum_ref::DatumRef, crate::player::ScriptError> {
    use crate::player::datum_ref::DatumRef;
    crate::player::reserve_player_mut(|player| {
        let sprite_num = match player.get_datum(datum) {
            crate::director::lingo::datum::Datum::SpriteRef(n) => *n,
            _ => return Ok(DatumRef::Void),
        };
        let key = match player
            .movie
            .score
            .get_sprite(sprite_num)
            .and_then(|s| s.member.as_ref())
        {
            Some(m) if m.cast_lib >= 0 && m.cast_member >= 0 => {
                (m.cast_lib as u32, m.cast_member as u32)
            }
            _ => return Ok(DatumRef::Void),
        };
        let image_ref = {
            let Some(anim) = player.gif_animations.get_mut(&key) else {
                return Ok(DatumRef::Void);
            };
            match handler {
                "pause" | "stop" => anim.running = false,
                "resume" | "play" => {
                    anim.running = true;
                    anim.last_switch_ms = 0.0;
                }
                "rewind" => {
                    anim.current = 0;
                    anim.last_switch_ms = 0.0;
                }
                _ => {}
            }
            anim.current_ref()
        };
        let member_ref = crate::CastMemberRef {
            cast_lib: key.0 as i32,
            cast_member: key.1 as i32,
        };
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
            if let crate::player::cast_member::CastMemberType::Bitmap(b) = &mut member.member_type {
                b.image_ref = image_ref;
            }
        }
        Ok(DatumRef::Void)
    })
}

#[cfg(test)]
mod control_tests {
    use super::*;

    fn anim() -> GifAnimation {
        GifAnimation {
            frames: vec![1, 2, 3],
            delays_ms: vec![100, 100, 100],
            width: 4, height: 4, current: 0, last_switch_ms: 0.0, running: true,
        }
    }

    #[test]
    fn a_paused_animation_holds_its_frame() {
        let mut a = anim();
        a.advance(1000.0);
        assert!(a.advance(1100.0));
        assert_eq!(a.current, 1);
        a.running = false;
        assert!(!a.advance(5000.0), "paused, so no frame change");
        assert_eq!(a.current, 1);
    }

    #[test]
    fn resuming_restarts_the_clock_rather_than_jumping() {
        let mut a = anim();
        a.running = false;
        a.last_switch_ms = 0.0;
        a.running = true;
        // First advance after a resume only seeds the clock.
        assert!(!a.advance(60_000.0));
        assert_eq!(a.current, 0, "no catch-up burst");
        assert!(a.advance(60_100.0));
        assert_eq!(a.current, 1);
    }
}
