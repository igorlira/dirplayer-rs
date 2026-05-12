use binary_reader::{BinaryReader, Endian};

use super::encoding::decode_win1252;
use super::reader::DirectorExt;

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
