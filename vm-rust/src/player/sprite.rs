use super::{cast_lib::CastMemberRef, script_ref::ScriptInstanceRef};

#[allow(dead_code)]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ColorRef {
    Rgb(u8, u8, u8),
    PaletteIndex(u8),
}

impl ColorRef {
    pub fn from_hex(hex: &str) -> ColorRef {
        // Tolerate stray whitespace around the value and the '#'. Habbo data
        // contains e.g. `rgb(" #000000")` (leading space); Director parses such
        // strings leniently to the intended color. Without trimming, the '#'
        // isn't stripped, the slice `" #"` fails to parse, and the old
        // `unwrap_or(255)` defaulted to bright red (255,0,0) — so every
        // `struct.font.bold`-colored header (Navigator nav_roominfo / roomlist)
        // rendered red instead of black. Also guards the slicing against
        // short/odd strings (no panic, no red fallback).
        let hex = hex.trim().trim_start_matches('#').trim();
        let byte = |start: usize| -> u8 {
            hex.get(start..start + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .unwrap_or(0)
        };
        ColorRef::Rgb(byte(0), byte(2), byte(4))
    }
    // Convert a ColorRef to a palette index using a palette slice.
    pub fn to_index(&self, palette: &[(u8, u8, u8)]) -> u8 {
        match self {
            ColorRef::PaletteIndex(i) => *i,
            ColorRef::Rgb(r, g, b) => {
                let mut best_index = 0;
                let mut best_distance = u32::MAX;
                for (i, &(pr, pg, pb)) in palette.iter().enumerate() {
                    let dr = *r as i32 - pr as i32;
                    let dg = *g as i32 - pg as i32;
                    let db = *b as i32 - pb as i32;
                    let distance = (dr*dr + dg*dg + db*db) as u32;
                    if distance < best_distance {
                        best_distance = distance;
                        best_index = i;
                    }
                }
                best_index as u8
            }
        }
    }
}

impl ToString for ColorRef {
    fn to_string(&self) -> String {
        match self {
            ColorRef::Rgb(r, g, b) => format!("rgb({}, {}, {})", r, g, b),
            ColorRef::PaletteIndex(i) => format!("color({})", i),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum CursorRef {
    System(i32),
    Member(Vec<i32>),
}

#[derive(Clone)]
pub struct Sprite {
    pub number: usize,
    pub name: String,
    pub puppet: bool,
    pub visible: bool,
    pub stretch: i32,
    pub loc_h: i32,
    pub loc_v: i32,
    pub loc_z: i32,
    pub width: i32,
    pub height: i32,
    pub ink: i32,
    pub blend: i32,
    pub rotation: f64,
    pub skew: f64,
    pub flip_h: bool,
    pub flip_v: bool,
    pub back_color: i32,
    pub color: ColorRef,
    pub bg_color: ColorRef,
    pub member: Option<CastMemberRef>,
    pub script_instance_list: Vec<ScriptInstanceRef>,
    pub cursor_ref: Option<CursorRef>,
    pub editable: bool,
    pub moveable: bool,
    pub constraint: i32, // 0 = stage, >0 = sprite number that constrains movement
    pub trails: bool,
    pub entered: bool,
    pub exited: bool,
    /// Set by `puppetSprite(N, FALSE)`: the sprite keeps its member/visual state
    /// for the rest of the CURRENT handler (Director defers the revert), but if
    /// it is still unpuppeted at the next frame tick it reverts to the Score.
    /// For a pure-puppet channel (no Score span — Coke Studios' furniture pool)
    /// that revert is a reset to empty. Cleared the moment the sprite is
    /// re-puppeted or given a new member (BrickOut re-puppets in the same
    /// handler, so it never reverts and keeps its makeStage member).
    pub pending_unpuppet_revert: bool,
    /// Set when the initial-load `begin_all_sprites` pass has already applied
    /// this channel's Score properties and then cleared `entered` so that
    /// `beginSprite` re-fires once the movie actually plays. The second pass
    /// must re-enter the span and rebuild behaviors, but must NOT re-apply the
    /// Score properties: `prepareMovie` runs between the two passes, and
    /// Director's rule is that script changes to a non-puppet sprite "last for
    /// the life of the current sprite" (11.5 Scripting Dictionary,
    /// puppetSprite()) — the span has not ended, so nothing should revert.
    ///
    /// dkbarrel's `prepareMovie` calls `codeInit` → `ShowText(1)`, which points
    /// the placard channels 961/962 at member "pressplay" and moves them to
    /// point(48,166). The second pass put member "blanktext" and point(200,184)
    /// straight back and the title screen lost its "Press Play to Begin!" text.
    /// `locZ`, written by a later loop in the same handler and absent from the
    /// Score channel data, survived — which is what identified the culprit.
    ///
    /// The existing `!sprite.puppet` guard doesn't cover this: dkbarrel's
    /// `puppetAll()` only puppets channels 1-150.
    ///
    /// On its own this is NOT enough to skip the second pass — see
    /// `script_wrote_since_span_init`.
    pub score_props_already_applied: bool,
    /// Set by `sprite_set_prop` (every Lingo write path) and cleared by
    /// `begin_sprites` right after it applies the Score properties, so it means
    /// "script has written to this channel since the Score last initialised
    /// it".
    ///
    /// The second `begin_all_sprites` pass skips re-applying Score properties
    /// only when this is set as well as `score_props_already_applied`. Skipping
    /// on `score_props_already_applied` alone regressed Habbo v1, whose sprite 2
    /// shape stopped drawing: the first pass runs while the movie is still
    /// loading, so a member whose cast isn't resolvable yet takes the non-shape
    /// ink/blend path, and the second pass is what fixed it up once the cast was
    /// there. Channels that no script touched must keep getting that fix-up.
    pub script_wrote_since_span_init: bool,
    pub quad: Option<[(i32, i32); 4]>, // [topLeft, topRight, bottomRight, bottomLeft] -- TODO: Tie this to position and size
    pub fore_color: i32,
    pub has_fore_color: bool,
    pub has_back_color: bool,
    pub has_visible_mod: bool,
    pub has_blend_mod: bool,
    pub has_size_tweened: bool,
    pub has_size_changed: bool,
    pub bitmap_size_owned_by_sprite: bool,
    /// Set when a Lingo script explicitly assigns `the width/height of sprite`.
    /// Such a size is authoritative — it must be used verbatim and bypass the
    /// score-data bbox heuristic in `get_concrete_sprite_rect` (which can
    /// misread an intentionally-scaled, rounding-skewed proportional size as a
    /// bounding-box approximation and snap it back to the bitmap's native
    /// size). Used by hackey's pseudo-3D `Translate` to shrink distant sprites.
    pub explicit_lingo_size: bool,
    // Base (score-defined) values
    pub base_loc_h: i32,
    pub base_loc_v: i32,
    pub base_width: i32,
    pub base_height: i32,
    pub base_rotation: f64,
    pub base_blend: i32,
    pub base_skew: f64,
    pub base_color: ColorRef,
    pub base_bg_color: ColorRef,
    /// Active camera name(s) for Shockwave3D sprites (set via sprite.camera(1) = ...)
    pub w3d_camera: Option<String>,
    /// Additional cameras for multi-camera rendering (index 2+)
    pub w3d_cameras: Vec<String>,
    /// Last on-screen rect captured when this sprite left its span. Director
    /// keeps a score channel's `the rect of sprite` at its last value even
    /// after the member clears to 0 (empty channel between two spans), so init
    /// scripts that read `sprite(1).rect` on a transition frame still see the
    /// real viewport size. Set only on span exit (begin_sprites); reset() clears
    /// it, so a genuinely cleared channel still reports a zero rect.
    pub retained_rect: Option<(i32, i32, i32, i32)>,
    /// Last frame Lingo asserted on this Flash sprite via `sprite.frame = N`
    /// (numeric). Sprite-owned so it SURVIVES a member swap (Director contract:
    /// the sprite `frame` property persists across `sprite.member =`) and is
    /// re-projected onto a freshly (re)created Ruffle instance at load time —
    /// which is how shared-member sprites show DIFFERENT frames (StoryScramble's
    /// 3 story tiles show unique posters even though they share cast 2:1) and how
    /// the bogeyman's `frame = 1` before a straw/longarm swap carries over.
    /// `None` = Lingo never set a numeric frame (free-run / pausedAtStart).
    /// Cleared by `reset()` (channel clear / endSprite / member=0).
    pub flash_asserted_frame: Option<i32>,
    /// Last sampled Flash root frame, used to detect a wrap (end-of-timeline)
    /// so a `loop = false` Flash member is halted at its final frame instead of
    /// looping forever (Director plays it once; `the playing of sprite`
    /// becomes FALSE). 0 = not yet sampled.
    pub flash_prev_frame: i32,
}

/// Threshold for detecting skew flip (in degrees)
const SKEW_FLIP_EPSILON: f64 = 0.1;

/// Check if a skew value represents a flip transform (±180°)
///
/// In Director, skew=180 (or -180) combined with rotation=180 produces
/// a vertical flip (left-right mirror) instead of an upside-down rotation.
///
/// Mathematically, this checks if |skew| ≈ 180°
#[inline]
pub fn is_skew_flip(skew: f64) -> bool {
    (skew.abs() - 180.0).abs() < SKEW_FLIP_EPSILON
}

impl Sprite {
    /// Blend value for rendering. When visible was explicitly set via Lingo
    /// and the sprite is visible, blend=0 is treated as 100 (fully opaque).
    /// The stored blend value stays unchanged for Lingo property reads.
    #[inline]
    /// The blend percentage to render with.
    ///
    /// This used to rewrite a score blend of 0 to 100 whenever a script had
    /// touched `.visible` and hadn't set `.blend` — a guess dating from when
    /// the score blend byte was read raw, where a 0 was usually a parse
    /// artefact rather than an authored value. `convert_raw_blend` later
    /// gained the authoritative gate (score byte 22 bit 0x10 — blend is only
    /// meaningful when that flag is set, else the byte is a junk default and
    /// the sprite is opaque), so a 0 arriving here is now genuinely authored
    /// and must be honoured.
    ///
    /// The guess also over-fired: `sprite(N).visible = 1` sets
    /// `has_visible_mod`, so any blanket visibility loop opted a sprite in.
    /// monsterattack's Streaming behavior does exactly that
    /// (`repeat with i = 1 to 40: sprite(i).visible = 1`), which resurrected
    /// its ButtonMask — an invisible click target authored at blend 0 — into
    /// a solid black box over the game card.
    pub fn effective_blend(&self) -> i32 {
        self.blend
    }

    /// Check if this sprite has a skew flip transform
    #[inline]
    pub fn has_skew_flip(&self) -> bool {
        is_skew_flip(self.skew)
    }

    pub fn new(number: usize) -> Sprite {
        Sprite {
            number,
            name: "".to_owned(),
            puppet: false,
            visible: true,
            stretch: 0,
            loc_h: 0,
            loc_v: 0,
            loc_z: number as i32,
            width: 0,
            height: 0,
            ink: 0,
            blend: 100,
            rotation: 0.0,
            skew: 0.0,
            flip_h: false,
            flip_v: false,
            back_color: 0,
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
            member: None,
            script_instance_list: vec![],
            cursor_ref: None,
            editable: false,
            moveable: false,
            constraint: 0,
            trails: false,
            entered: false,
            exited: false,
            pending_unpuppet_revert: false,
            score_props_already_applied: false,
            script_wrote_since_span_init: false,
            quad: None,
            fore_color: 255,
            has_fore_color: false,
            has_back_color: false,
            has_visible_mod: false,
            has_blend_mod: false,
            has_size_tweened: false,
            has_size_changed: false,
            bitmap_size_owned_by_sprite: false,
            explicit_lingo_size: false,
            base_loc_h: 0,
            base_loc_v: 0,
            base_width: 0,
            base_height: 0,
            base_rotation: 0.0,
            base_blend: 100,
            base_skew: 0.0,
            base_color: ColorRef::PaletteIndex(255),
            base_bg_color: ColorRef::PaletteIndex(0),
            w3d_camera: None,
            w3d_cameras: Vec::new(),
            retained_rect: None,
            flash_asserted_frame: None,
            flash_prev_frame: 0,
        }
    }

    pub fn reset_for_member_change(&mut self) {
        self.skew = 0.0;
        self.flip_h = false;
        self.flip_v = false;
        self.rotation = 0.0;
        // Colours are NOT reset here. Director keeps a sprite's foreColor and
        // backColor across a member swap — "if a sprite channel is not a
        // puppet, any changes that script makes to a sprite last for the life
        // of the current sprite only" (11.5 Scripting Dictionary,
        // puppetSprite()); a member assignment is not the end of that life,
        // and for a puppet the properties persist until script changes them.
        //
        // This used to fall back to the placeholder colours whenever
        // `has_back_color` / `has_fore_color` was clear. That flag only tracks
        // colours set from LINGO, so a colour the SCORE authored looked
        // identical to one never set at all, and every member swap wiped it:
        //
        //   Candystand Miniature Golf's hole-18 billboard is an ink-36 sprite
        //   whose score record carries backColor 2 — magenta in the bitmap's
        //   web216 palette, the chroma key for its three animation frames. It
        //   drew correctly on the frame it entered, then the hole code stamped
        //   the next frame's member in, `bg_color` fell back to palette index 0
        //   (white), the colour key stopped matching anything, and the magenta
        //   surround rendered opaque for the rest of the hole.
        //
        // It also left the sprite internally inconsistent: `bg_color` (the
        // ColorRef the renderer keys on) was reset while `back_color` (the
        // integer `the backColor` returns) was not, so Lingo reported 2 while
        // the renderer used 0. Anything that resets one must reset both.
    }

    pub fn reset(&mut self) {
        self.name = "".to_owned();
        self.puppet = false;
        self.visible = true;
        self.stretch = 0;
        self.loc_h = 0;
        self.loc_v = 0;
        self.loc_z = self.number as i32;
        self.width = 0;
        self.height = 0;
        self.ink = 0;
        self.blend = 100;
        self.rotation = 0.0;
        self.skew = 0.0;
        self.flip_h = false;
        self.flip_v = false;
        self.back_color = 0;
        self.color = ColorRef::PaletteIndex(255);
        self.bg_color = ColorRef::PaletteIndex(0);
        self.member = None;
        self.script_instance_list.clear();
        self.cursor_ref = None;
        self.editable = false;
        self.constraint = 0;
        self.entered = false;
        self.exited = false;
        self.pending_unpuppet_revert = false;
        self.score_props_already_applied = false;
        self.script_wrote_since_span_init = false;
        self.quad = None;
        self.fore_color = 255;
        self.has_fore_color = false;
        self.has_back_color = false;
        self.has_visible_mod = false;
        self.has_size_tweened = false;
        self.has_size_changed = false;
        self.bitmap_size_owned_by_sprite = false;
        self.explicit_lingo_size = false;
        self.retained_rect = None;
        self.flash_asserted_frame = None;
        self.flash_prev_frame = 0;
    }
}
