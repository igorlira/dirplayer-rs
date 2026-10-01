use binary_reader::{BinaryReader, Endian};

use super::encoding::decode_win1252;
use super::reader::DirectorExt;

/// A cast member's name is stored the way script literals are: Mac Roman on
/// every platform. A Windows-packaged movie stored a name with an ø as byte
/// 0xBF, which Win-1252 reads as "¿", so `sprite.member = "<that name>"`
/// from a script never found the member and the sprite stayed empty.
pub fn read_pascal_string_macroman(item_bufs: &Vec<Vec<u8>>, index: usize, item_endian: Endian) -> String {
    if index >= item_bufs.len() {
        return "".to_owned();
    }
    let mut reader = BinaryReader::from_vec(&item_bufs[index]);
    reader.set_endian(item_endian);
    if reader.length == 0 {
        return "".to_owned();
    }
    let len = reader.read_u8().unwrap_or(0) as usize;
    reader.read_string_macroman(len).unwrap_or_default()
}

pub fn read_pascal_string(item_bufs: &Vec<Vec<u8>>, index: usize, item_endian: Endian) -> String {
    if index >= item_bufs.len() {
        return "".to_owned();
    }

    let mut reader = BinaryReader::from_vec(&item_bufs[index]);
    reader.set_endian(item_endian);

    if reader.length == 0 {
        return "".to_owned();
    }

    return reader.read_pascal_string().unwrap_or_else(|e| panic!("Failed to read pascal string: {e}"));
}

pub fn read_string(item_bufs: &Vec<Vec<u8>>, index: usize) -> String {
    if index >= item_bufs.len() {
        return "".to_owned();
    }

    // Win-1252, not UTF-8. See io::encoding for context.
    let buf = &item_bufs[index];
    return decode_win1252(buf);
}

pub fn read_u16(item_bufs: &Vec<Vec<u8>>, index: usize, item_endian: Endian) -> u16 {
    if index >= item_bufs.len() {
        return 0;
    }

    let mut reader = BinaryReader::from_vec(&item_bufs[index]);
    reader.set_endian(item_endian);
    return reader.read_u16().unwrap();
}
