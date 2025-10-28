use binary_reader::{BinaryReader, Endian};

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

    return reader.read_pascal_string().unwrap();
}

pub fn read_string(item_bufs: &Vec<Vec<u8>>, index: usize) -> String {
    if index >= item_bufs.len() {
        return "".to_owned();
    }

    let buf = &item_bufs[index];
    return String::from_utf8(buf.to_vec()).unwrap();
}

pub fn read_u16(item_bufs: &Vec<Vec<u8>>, index: usize, item_endian: Endian) -> u16 {
    if index >= item_bufs.len() {
        return 0;
    }

    let mut reader = BinaryReader::from_vec(&item_bufs[index]);
    reader.set_endian(item_endian);
    return reader.read_u16().unwrap();
}
