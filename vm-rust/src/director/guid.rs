use std::fmt::Display;

use binary_reader::BinaryReader;

#[derive(Clone, Copy)]
pub struct MoaID {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: u32,
    data5: u32,
}

impl MoaID {
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: u32, data5: u32) -> MoaID {
        return MoaID {
            data1: data1,
            data2: data2,
            data3: data3,
            data4: data4,
            data5: data5,
        };
    }

    pub fn from_reader(reader: &mut BinaryReader) -> MoaID {
        return MoaID {
            data1: reader.read_u32().unwrap(),
            data2: reader.read_u16().unwrap(),
            data3: reader.read_u16().unwrap(),
            data4: reader.read_u32().unwrap(),
            data5: reader.read_u32().unwrap(),
        };
    }
}

impl PartialEq for MoaID {
    fn eq(&self, other: &Self) -> bool {
        self.data1 == other.data1
            && self.data2 == other.data2
            && self.data3 == other.data3
            && self.data4 == other.data4
            && self.data5 == other.data5
    }
}

impl Display for MoaID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        return f.write_fmt(format_args!(
            "{:#010x}-{:#06x}-{:#06x}-{:#010x}-{:#010x}",
            self.data1, self.data2, self.data3, self.data4, self.data5
        ));
    }
}
//                                                           0xac99e904-        0x0070-0x0b36-0x00080000-0x347a3707
pub const FONTMAP_COMPRESSION_GUID: MoaID =
    MoaID::new(0x8A4679A1, 0x3720, 0x11D0, 0xA0002392, 0xB16808C9);
pub const NULL_COMPRESSION_GUID: MoaID =
    MoaID::new(0xAC99982E, 0x005D, 0x0D50, 0x00080000, 0x347A3707);
pub const SND_COMPRESSION_GUID: MoaID =
    MoaID::new(0x7204A889, 0xAFD0, 0x11CF, 0xA00022A2, 0x4C445323);
pub const ZLIB_COMPRESSION_GUID: MoaID =
    MoaID::new(0xAC99E904, 0x0070, 0x0B36, 0x00080000, 0x347A3707);
pub const ZLIB_COMPRESSION_GUID2: MoaID =
    MoaID::new(0xAC99E904, 0x0070, 0x0B36, 0x00000800, 0x07377A34);
