use binary_rw::{BinaryError, BinaryWriter, Endian, MemoryStream};

use crate::{
    director::static_datum::StaticDatum,
    player::xtra::multiuser::{MultiuserMessage, blowfish::MUSBlowfish, types::MusLingoValueTag},
};

pub trait MusWriter {
    fn write_mus_string(&mut self, value: &str) -> Result<usize, BinaryError>;
    fn write_mus_lingo_value(&mut self, value: &StaticDatum) -> Result<usize, BinaryError>;
}

impl MusWriter for BinaryWriter<'_> {
    fn write_mus_string(&mut self, value: &str) -> Result<usize, BinaryError> {
        let bytes: Vec<u8> = value.chars().map(|c| c as u8).collect();
        let len = bytes.len();
        let mut written = self.write_u32(len as u32)?;
        written += self.write_bytes(&bytes)?;
        if len % 2 != 0 {
            written += self.write_u8(0)?;
        }
        Ok(written)
    }

    fn write_mus_lingo_value(&mut self, value: &StaticDatum) -> Result<usize, BinaryError> {
        match value {
            StaticDatum::Void => self.write_u16(MusLingoValueTag::Void as u16),
            StaticDatum::Int(i) => {
                let mut written = self.write_u16(MusLingoValueTag::Int as u16)?;
                written += self.write_i32(*i)?;
                Ok(written)
            }
            StaticDatum::String(s) => {
                let mut written = self.write_u16(MusLingoValueTag::String as u16)?;
                written += self.write_mus_string(s)?;
                Ok(written)
            }
            StaticDatum::Symbol(name) => {
                let mut written = self.write_u16(MusLingoValueTag::Symbol as u16)?;
                written += self.write_mus_string(name)?;
                Ok(written)
            }
            StaticDatum::List(items) => {
                let mut written = self.write_u16(MusLingoValueTag::List as u16)?;
                written += self.write_u32(items.len() as u32)?;
                for item in items {
                    written += self.write_mus_lingo_value(item)?;
                }
                Ok(written)
            }
            StaticDatum::PropList(pairs) => {
                let mut written = self.write_u16(MusLingoValueTag::PropList as u16)?;
                written += self.write_u32(pairs.len() as u32)?;
                for (key, value) in pairs {
                    written += self.write_mus_lingo_value(key)?;
                    written += self.write_mus_lingo_value(value)?;
                }
                Ok(written)
            }
            StaticDatum::IntPoint(x, y) => {
                let mut written = self.write_u16(MusLingoValueTag::Point as u16)?;
                written += self.write_i32(*x)?;
                written += self.write_i32(*y)?;
                Ok(written)
            }
            StaticDatum::IntRect(l, t, r, b) => {
                let mut written = self.write_u16(MusLingoValueTag::Rect as u16)?;
                written += self.write_i32(*t)?;
                written += self.write_i32(*l)?;
                written += self.write_i32(*b)?;
                written += self.write_i32(*r)?;
                Ok(written)
            }
            StaticDatum::Media(media_data) => {
                let mut written = self.write_u16(MusLingoValueTag::Media as u16)?;
                written += self.write_u32(media_data.len() as u32)?;
                written += self.write_bytes(&media_data)?;
                if media_data.len() % 2 != 0 {
                    written += self.write_u8(0)?;
                }
                Ok(written)
            }
            _ => self.write_u16(MusLingoValueTag::Void as u16),
        }
    }
}

impl MultiuserMessage {
    pub fn to_bytes(&self, cipher: Option<&mut MUSBlowfish>) -> Vec<u8> {
        let mut payload_stream = MemoryStream::new();
        {
            let mut w = BinaryWriter::new(&mut payload_stream, Endian::Big);
            w.write_i32(self.error_code).unwrap();
            w.write_u32(self.time_stamp).unwrap();
            w.write_mus_string(&self.subject).unwrap();
            w.write_mus_string(&self.sender_id).unwrap();
            w.write_u32(self.recipients.len() as u32).unwrap();
            for r in &self.recipients {
                w.write_mus_string(r).unwrap();
            }

            let is_encrypted = self.subject == "Logon"
                && self.recipients.len() == 1
                && self.recipients[0] == "System";

            if is_encrypted {
                if let Some(cipher) = cipher {
                    let mut content_stream = MemoryStream::new();
                    {
                        let mut cw = BinaryWriter::new(&mut content_stream, Endian::Big);
                        cw.write_mus_lingo_value(&self.content).unwrap();
                    }
                    let mut content_bytes: Vec<u8> = content_stream.into();
                    cipher.apply_stream(&mut content_bytes);
                    w.write_bytes(&content_bytes).unwrap();
                } else {
                    w.write_mus_lingo_value(&self.content).unwrap();
                }
            } else {
                w.write_mus_lingo_value(&self.content).unwrap();
            }
        }
        let payload_bytes: Vec<u8> = payload_stream.into();

        let mut msg_stream = MemoryStream::new();
        {
            let mut w = BinaryWriter::new(&mut msg_stream, Endian::Big);
            w.write_u16(0x7200).unwrap();
            w.write_u32(payload_bytes.len() as u32).unwrap();
            w.write_bytes(&payload_bytes).unwrap();
        }
        msg_stream.into()
    }
}
