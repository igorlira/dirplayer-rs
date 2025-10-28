use super::{cast_lib::CastMemberRef, script_ref::ScriptInstanceRef};

#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub enum ColorRef {
    Rgb(u8, u8, u8),
    PaletteIndex(u8),
}

impl ColorRef {
    pub fn from_hex(hex: &str) -> ColorRef {
        let hex = hex.trim_start_matches('#');
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap();
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap();
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap();
        ColorRef::Rgb(r, g, b)
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
    pub rotation: f32,
    pub skew: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub back_color: i32,
    pub color: ColorRef,
    pub bg_color: ColorRef,
    pub member: Option<CastMemberRef>,
    pub script_instance_list: Vec<ScriptInstanceRef>,
    pub cursor_ref: Option<CursorRef>,
    pub editable: bool,
    pub entered: bool,
    pub exited: bool,
    pub quad: Option<[(i32, i32); 4]>, // [topLeft, topRight, bottomRight, bottomLeft] -- TODO: Tie this to position and size
}

impl Sprite {
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
            entered: false,
            exited: false,
            quad: None,
        }
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
        self.entered = false;
        self.exited = false;
        self.quad = None;
    }
}
