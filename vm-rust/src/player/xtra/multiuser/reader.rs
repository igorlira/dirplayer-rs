use binary_reader::BinaryReader;
use num::FromPrimitive;

use crate::{director::static_datum::StaticDatum, player::xtra::multiuser::{MultiuserMessage, blowfish::MUSBlowfish, types::MusLingoValueTag}};


pub const MUS_HEADER: u16 = 0x7200;
pub const MUS_FRAME_HEADER_SIZE: usize = 6; // 2 (header) + 4 (payload size)

pub trait MusReader {
    fn read_mus_string(&mut self) -> Result<String, std::io::Error>;
    fn read_mus_message(&mut self, cipher: Option<&mut MUSBlowfish>) -> Result<MultiuserMessage, std::io::Error>;
    fn read_mus_message_payload(payload: &[u8], cipher: Option<&mut MUSBlowfish>) -> Result<MultiuserMessage, std::io::Error>;
    fn read_mus_lingo_value(&mut self) -> Result<StaticDatum, std::io::Error>;
}

impl MusReader for BinaryReader {
    fn read_mus_string(&mut self) -> Result<String, std::io::Error> {
        let length = self.read_u32()? as usize;
        let bytes = self.read_bytes(length)?;
        let result_string = bytes.iter().map(|&b| b as char).collect::<String>();
        if length % 2 != 0 {
            self.read_bytes(1)?;
        }
        Ok(result_string)
    }

    fn read_mus_lingo_value(&mut self) -> Result<StaticDatum, std::io::Error> {
        let tag = self.read_u16()?;
        match MusLingoValueTag::from_u16(tag) {
            Some(MusLingoValueTag::Void) => Ok(StaticDatum::Void),
            Some(MusLingoValueTag::Int) => {
                let value = self.read_i32()?;
                Ok(StaticDatum::Int(value))
            }
            Some(MusLingoValueTag::String) => {
                let value = self.read_mus_string()?;
                Ok(StaticDatum::String(value))
            }
            Some(MusLingoValueTag::Symbol) => {
                let name = self.read_mus_string()?;
                Ok(StaticDatum::Symbol(name))
            }
            Some(MusLingoValueTag::List) => {
                let count = self.read_u32()?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push(self.read_mus_lingo_value()?);
                }
                Ok(StaticDatum::List(items))
            }
            Some(MusLingoValueTag::PropList) => {
                let count = self.read_u32()?;
                let mut pairs = Vec::new();
                for _ in 0..count {
                    let key = self.read_mus_lingo_value()?;
                    let value = self.read_mus_lingo_value()?;
                    pairs.push((key, value));
                }
                Ok(StaticDatum::PropList(pairs))
            }
            Some(MusLingoValueTag::Media) => {
                let len = self.read_u32()? as usize;
                let data = self.read_bytes(len)?.to_vec();
                if len % 2 != 0 {
                    self.read_u8()?;
                }
                Ok(StaticDatum::Media(data.to_vec())) 
            }
            _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Unknown Lingo value tag: {}", tag))),
        }
    }

    fn read_mus_message(&mut self, cipher: Option<&mut MUSBlowfish>) -> Result<MultiuserMessage, std::io::Error> {
        let header = self.read_u16()?;
        if header != MUS_HEADER {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid message header"));
        }
        let message_size = self.read_u32()?;
        let payload = self.read_bytes(message_size as usize)?;
        Self::read_mus_message_payload(&payload, cipher)
    }

    fn read_mus_message_payload(payload: &[u8], cipher: Option<&mut MUSBlowfish>) -> Result<MultiuserMessage, std::io::Error> {
        let mut payload_reader = BinaryReader::from_u8(payload);

        let error_code = payload_reader.read_i32()?;
        let timestamp = payload_reader.read_u32()?;
        let subject = payload_reader.read_mus_string()?;
        let sender_id = payload_reader.read_mus_string()?;
        let recipient_count = payload_reader.read_u32()?;
        let recipients = (0..recipient_count).map(|_| {
            let recipient_id = payload_reader.read_mus_string()?;
            Ok(recipient_id)
        }).collect::<Result<Vec<String>, std::io::Error>>()?;

        let is_encrypted = subject == "Logon" && recipients.len() == 1 && recipients[0] == "System";

        // Calculate content byte offset by summing header field sizes
        let header_string_size = |s: &str| -> usize {
            4 + s.len() + (if s.len() % 2 != 0 { 1 } else { 0 })
        };
        let mut content_offset: usize = 4 + 4; // error_code + timestamp
        content_offset += header_string_size(&subject);
        content_offset += header_string_size(&sender_id);
        content_offset += 4; // recipient_count
        for r in &recipients {
            content_offset += header_string_size(r);
        }
        let content_bytes = &payload[content_offset..];

        let content = if is_encrypted {
            if let Some(cipher) = cipher {
                let mut decrypted = content_bytes.to_vec();
                cipher.apply_stream(&mut decrypted);
                let mut content_reader = BinaryReader::from_u8(&decrypted);
                content_reader.read_mus_lingo_value()?
            } else {
                StaticDatum::Void
            }
        } else {
            let mut content_reader = BinaryReader::from_u8(content_bytes);
            content_reader.read_mus_lingo_value()?
        };

        Ok(MultiuserMessage {
            recipients,
            subject,
            content,
            error_code,
            sender_id,
            time_stamp: timestamp,
        })
    }
}