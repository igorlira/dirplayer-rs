use super::{cast_lib::CastMemberRef, script_ref::ScriptInstanceRef};

#[allow(dead_code)]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ColorRef {
    Rgb(u8, u8, u8),
    PaletteIndex(u8),
}

impl ColorRef {
    pub fn from_hex(hex: &str) -> ColorRef {
        let hex = hex.trim_start_matches('#');
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        ColorRef::Rgb(r, g, b)
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
    pub quad: Option<[(i32, i32); 4]>, // [topLeft, topRight, bottomRight, bottomLeft] -- TODO: Tie this to position and size
    pub fore_color: i32,
    pub has_fore_color: bool,
    pub has_back_color: bool,
    pub has_size_tweened: bool,
    pub has_size_changed: bool,
    pub bitmap_size_owned_by_sprite: bool,
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
            quad: None,
            fore_color: 255,
            has_fore_color: false,
            has_back_color: false,
            has_size_tweened: false,
            has_size_changed: false,
            bitmap_size_owned_by_sprite: false,
            base_loc_h: 0,
            base_loc_v: 0,
            base_width: 0,
            base_height: 0,
            base_rotation: 0.0,
            base_blend: 100,
            base_skew: 0.0,
            base_color: ColorRef::PaletteIndex(255),
            base_bg_color: ColorRef::PaletteIndex(0),
        }
    }

    pub fn reset_for_member_change(&mut self) {
        self.skew = 0.0;
        self.flip_h = false;
        self.flip_v = false;
        self.rotation = 0.0;
        self.bg_color = ColorRef::PaletteIndex(0);
        self.color = ColorRef::PaletteIndex(255);
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
        self.quad = None;
        self.fore_color = 255;
        self.has_fore_color = false;
        self.has_back_color = false;
        self.has_size_tweened = false;
        self.has_size_changed = false;
        self.bitmap_size_owned_by_sprite = false;
    }
}
