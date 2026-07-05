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
// SWA (Shockwave Audio / MP3) compression, aka the "SWA Decompression Xtra"
// codec. This is the SAME class as SND_COMPRESSION_GUID above, but with the
// trailing 8 GUID bytes read in the other order (`from_reader` treats them as
// endian-sensitive u32s, so a big-endian RIFX movie yields these values while a
// little-endian XFIR movie yields the swapped SND_COMPRESSION_GUID form — the
// same GUID/GUID2 split ZLIB needs). Verified from nomiss.dcr's Fcdr table:
// `7204a889-afd0-11cf-a22200a0-2453444c` next to "requires the SWA Decompression
// Xtra". SWA members store their MP3 bytes raw for the sound decoder, so this is
// recognised only to store-raw-without-warning — NOT routed through the SND
// (`from_snd_chunk`) path, which would corrupt the audio.
pub const SWA_COMPRESSION_GUID: MoaID =
    MoaID::new(0x7204A889, 0xAFD0, 0x11CF, 0xA22200A0, 0x2453444C);
// Little-endian (XFIR) read of the same on-disk SWA GUID bytes — the trailing 8
// bytes `A2 22 00 A0 24 53 44 4C` read as u32s give A00022A2 / 4C445324 under a
// little-endian reader (XFIR) vs A22200A0 / 2453444C under big-endian (RIFX),
// exactly like ZLIB_COMPRESSION_GUID vs …GUID2. Verified from 15love_hs.dcr
// (XFIR/MDGF) whose Fcdr strings say "SWA Decompressor Xtra from Macromedia".
pub const SWA_COMPRESSION_GUID2: MoaID =
    MoaID::new(0x7204A889, 0xAFD0, 0x11CF, 0xA00022A2, 0x4C445324);
pub const ZLIB_COMPRESSION_GUID: MoaID =
    MoaID::new(0xAC99E904, 0x0070, 0x0B36, 0x00080000, 0x347A3707);
pub const ZLIB_COMPRESSION_GUID2: MoaID =
    MoaID::new(0xAC99E904, 0x0070, 0x0B36, 0x00000800, 0x07377A34);
