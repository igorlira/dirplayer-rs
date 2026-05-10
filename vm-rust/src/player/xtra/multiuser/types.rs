use num_derive::{FromPrimitive, ToPrimitive};

#[derive(Debug, FromPrimitive, ToPrimitive)]
#[repr(u16)]
pub enum MusLingoValueTag {
    Void = 0,
    Int = 1,
    Symbol = 2,
    String = 3,
    Picture = 5,
    Float = 6,
    List = 7,
    Point = 8,
    Rect = 9,
    PropList = 10,
    Color = 18,
    Date = 19,
    Media = 20,
    _3DVector = 22,
    _3DTransform = 23,
}
