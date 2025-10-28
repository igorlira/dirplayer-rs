use binary_reader::{BinaryReader, Endian};
use std::convert::TryInto;
use web_sys::console;

#[derive(Debug, Clone)]
pub struct MediaChunk {
    pub sample_rate: u32,
    pub data_size_field: u32,
    pub guid: Option<[u8; 16]>,
    pub audio_data: Vec<u8>,
    pub is_compressed: bool,
}

impl MediaChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<Self, String> {
        let mut data_test = Vec::new();

        let r_begin = reader.pos;
        while let Ok(byte) = reader.read_u8() {
            data_test.push(byte);
        }

        let hex_dump = data_test
            .iter()
            .map(|b| format!("{:02X} ", b))
            .collect::<Vec<String>>()
            .join(" ");
        console::log_1(
            &format!(
                "WAV Hex Dump (Full File, {} bytes):\n{}",
                data_test.len(),
                hex_dump
            )
            .into(),
        );

        reader.pos = r_begin;

        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let header_size = reader.read_u32().map_err(|e| e.to_string())?;
        let _unknown1 = reader.read_u32().map_err(|e| e.to_string())?;
        let sample_rate = reader.read_u32().map_err(|e| e.to_string())?;
        let _sample_rate2 = reader.read_u32().map_err(|e| e.to_string())?;
        let _unknown2 = reader.read_u32().map_err(|e| e.to_string())?;
        let data_size_field = reader.read_u32().map_err(|e| e.to_string())?;

        let bytes_read = 24;
        let skip_bytes = (header_size as usize).saturating_sub(bytes_read);

        // Read GUID if present
        let guid = if skip_bytes >= 16 {
            let b = reader.read_bytes(16).map_err(|e| e.to_string())?;
            Some(b.try_into().unwrap())
        } else {
            None
        };

        // Skip remaining header padding
        if skip_bytes > 16 {
            let _ = reader.read_bytes(skip_bytes - 16);
        } else if skip_bytes > 0 && skip_bytes < 16 {
            let _ = reader.read_bytes(skip_bytes);
        }

        // Read all remaining data as audio data
        let mut audio_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            audio_data.push(byte);
        }

        // Detect compression type
        // MP3: starts with 0xFF 0xFx
        let is_mp3 =
            audio_data.len() >= 2 && audio_data[0] == 0xFF && (audio_data[1] & 0xE0) == 0xE0;

        // IMA ADPCM: data is significantly smaller than data_size_field
        // data_size_field represents uncompressed PCM size
        let compression_ratio = if audio_data.len() > 0 {
            data_size_field as f32 / audio_data.len() as f32
        } else {
            1.0
        };

        let is_ima_adpcm = compression_ratio > 2.0 && !is_mp3;
        let is_compressed = is_mp3 || is_ima_adpcm;

        console::log_1(
            &format!(
                "MediaChunk: {} bytes (expected {}), ratio={:.2}, mp3={}, ima_adpcm={}, rate={}",
                audio_data.len(),
                data_size_field,
                compression_ratio,
                is_mp3,
                is_ima_adpcm,
                sample_rate
            )
            .into(),
        );

        reader.endian = original_endian;

        Ok(MediaChunk {
            sample_rate,
            data_size_field,
            guid,
            audio_data: data_test,
            is_compressed,
        })
    }

    // Helper to extract sample rate from MP3 frame header
    fn get_mp3_sample_rate(frame_header: &[u8]) -> Option<u32> {
        if frame_header.len() < 4 {
            return None;
        }

        // MP3 frame: FF Fx xx xx
        // Byte 2, bits 2-3 contain sample rate index
        let sample_rate_bits = (frame_header[2] >> 2) & 0x03;

        // MPEG version from byte 1, bits 3-4
        let mpeg_version = (frame_header[1] >> 3) & 0x03;

        match (mpeg_version, sample_rate_bits) {
            (3, 0) => Some(44100), // MPEG-1
            (3, 1) => Some(48000),
            (3, 2) => Some(32000),
            (2, 0) => Some(22050), // MPEG-2
            (2, 1) => Some(24000),
            (2, 2) => Some(16000),
            (0, 0) => Some(11025), // MPEG-2.5
            (0, 1) => Some(12000),
            (0, 2) => Some(8000),
            _ => None,
        }
    }

    pub fn get_codec_name(&self) -> &str {
        if let Some(guid) = self.guid {
            // Check against known DirectSound/Windows Media GUIDs
            // 5A08CD40-535B-11D0-A8BB-00A0C9008A48 is IMA ADPCM
            if &guid[0..8] == &[0x5A, 0x08, 0xCD, 0x40, 0x53, 0x5B, 0x11, 0xD0] {
                return "ima_adpcm";
            }
        }

        // Check for MP3
        if self.audio_data.len() >= 2
            && self.audio_data[0] == 0xFF
            && (self.audio_data[1] & 0xE0) == 0xE0
        {
            return "mp3";
        }

        // Check for IMA ADPCM by compression ratio
        let compression_ratio = if self.audio_data.len() > 0 {
            self.data_size_field as f32 / self.audio_data.len() as f32
        } else {
            1.0
        };

        if compression_ratio > 2.0 {
            "ima_adpcm"
        } else {
            "raw_pcm"
        }
    }

    pub fn is_sound(&self) -> bool {
        // Consider both compressed (MP3) and raw PCM as valid sound
        self.is_compressed || !self.audio_data.is_empty()
    }
}
