use binary_reader::{BinaryReader, Endian};

use wasm_bindgen::JsValue;
use web_sys::console;

use js_sys::Float32Array;

use crate::director::chunks::MediaChunk;

#[derive(Clone)]
pub struct SoundChunk {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    sample_count: u32,
    codec: String,
    data: Vec<u8>,
}

impl SoundChunk {
    pub fn new(data: Vec<u8>) -> SoundChunk {
        SoundChunk {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_count: 0,
            codec: "raw_pcm".into(),
            data,
        }
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn codec(&self) -> String {
        self.codec.clone()
    }

    pub fn data(&self) -> Vec<u8> {
        self.data.clone()
    }

    pub fn set_metadata(&mut self, sample_rate: u32, channels: u16, bits_per_sample: u16) {
        self.sample_rate = sample_rate;
        self.channels = channels;
        self.bits_per_sample = bits_per_sample;
        self.sample_count =
            (self.data.len() / (channels as usize * (bits_per_sample / 8) as usize)) as u32;
        console::log_1(
            &format!(
                "Updated metadata: channels={}, sample_rate={}, bits={}",
                self.channels, self.sample_rate, self.bits_per_sample
            )
            .into(),
        );
    }

    pub fn debug_get_samples(&self) -> Result<Float32Array, JsValue> {
        let max_samples = 100;
        let num_samples_to_process = std::cmp::min(self.sample_count as usize, max_samples);
        let num_output_floats = num_samples_to_process * self.channels as usize;

        // 1. Create the Float32Array to return to JS
        let output_array = Float32Array::new_with_length(num_output_floats as u32);

        // The data is Vec<u8> which we need to read as 16-bit integers
        let mut byte_reader = BinaryReader::from_vec(&self.data);
        // Director audio is Big Endian
        byte_reader.endian = Endian::Big;

        // 16-bit signed max value for normalization
        const MAX_I16_F: f32 = 32768.0;

        for i in 0..num_output_floats {
            // Read one 16-bit sample (u16) from the Big-Endian data.
            // BinaryReader handles the Big Endian interpretation for us.
            // Note: read_i16() would be better, but we need the normalization step.

            let signed_sample_i16 = match byte_reader.read_i16() {
                Ok(val) => val,
                Err(_) => break, // Stop if we run out of data
            };

            // 2. Normalization: Convert signed 16-bit integer to a float between -1.0 and 1.0
            let normalized_sample = signed_sample_i16 as f32 / MAX_I16_F;

            // 3. Write to the output array
            output_array.set_index(i as u32, normalized_sample);
        }

        console::log_1(&format!("Debug Sample Array size: {}", output_array.length()).into());
        Ok(output_array)
    }
}

impl Default for SoundChunk {
    fn default() -> Self {
        Self {
            channels: 1,
            sample_rate: 22050,
            bits_per_sample: 16,
            sample_count: 0,
            codec: "raw_pcm".to_string(),
            data: Vec::new(),
        }
    }
}

impl SoundChunk {
    pub fn from_snd_chunk(reader: &mut BinaryReader) -> Result<SoundChunk, String> {
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

        console::log_1(&"Parsing Director MX 2004 snd chunk (Finalized Multi-offset check)".into());

        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let read_start = reader.pos;

        // --- Header Reading (for logging) ---
        // Note: The BinaryReader position must be reset after reading the header for logging.
        let mut header_bytes = Vec::new();
        for _ in 0..64 {
            match reader.read_u8() {
                Ok(byte) => header_bytes.push(byte),
                Err(_) => break,
            }
        }

        if !header_bytes.is_empty() {
            // ... (Header logging code remains here) ...
        }

        reader.pos = read_start; // Reset position to start reading fields

        // --- 1. Read All Candidate Sample Rates ---

        // Offsets 0x00 - 0x03 (4 bytes)
        let _ = reader
            .read_u32()
            .map_err(|e| format!("Failed to read 0x00: {}", e))?; // Skip 0x00-0x03

        // Offset 0x04 (4 bytes): Sample Rate ENCODED (Rate A - 16000 Hz target)
        let sample_rate_encoded = reader
            .read_u32()
            .map_err(|e| format!("Failed to read 0x04: {}", e))?;

        // Calculate 16000 Hz target rate and snap to 16000
        let mut rate_a = (sample_rate_encoded as f64 / 6.144).round() as u32;
        if rate_a > 15990 && rate_a < 16020 {
            rate_a = 16000;
        }

        // Skip to 0x16 (We are at 0x08, need to skip 14 bytes)
        for _ in 0..14 {
            let _ = reader
                .read_u8()
                .map_err(|e| format!("Failed to skip to 0x16: {}", e))?;
        }

        // Offset 0x16 (2 bytes): Sample Rate u16 (Rate B - 22050 Hz target)
        let rate_b_u16 = reader
            .read_u16()
            .map_err(|e| format!("Failed to read 0x16: {}", e))?;
        let rate_b = rate_b_u16 as u32;

        // Skip to 0x2A (We are at 0x18, need to skip 18 bytes)
        for _ in 0..(0x2A - 0x18) {
            let _ = reader
                .read_u8()
                .map_err(|e| format!("Failed to skip to 0x2A: {}", e))?;
        }

        // Offset 0x2A (2 bytes): Sample Rate u16 (Rate C - 44100 Hz target)
        let rate_c_u16 = reader
            .read_u16()
            .map_err(|e| format!("Failed to read 0x2A: {}", e))?;
        let rate_c = rate_c_u16 as u32;

        // --- 2. Determine Final Sample Rate, Channels & Bits Per Sample ---

        let mut sample_rate: u32;
        let mut bits_per_sample: u16;
        let mut channels: u16;

        // Priority 1: Check for 22050 Hz (Rate B)
        if rate_b == 22050 {
            // These sounds must be 16-bit mono to yield the correct short durations (0.5s, 1s, 2s, 4s)
            sample_rate = 22050;
            bits_per_sample = 16;
            channels = 1;
        }
        // Priority 2: Check for 44100 Hz (Rate C)
        else if rate_c == 44100 {
            // These sounds are 16-bit mono to yield the correct durations (1.95s, 1.56s, etc.)
            sample_rate = 44100;
            bits_per_sample = 16;
            channels = 1;
        }
        // Priority 3: Fallback (Rate A / 16000 Hz)
        else {
            // This handles the first chunk
            sample_rate = rate_a;
            bits_per_sample = 16;
            channels = 1;
        }

        // Log the selection
        console::log_1(
            &format!(
                "Selected Format: {} Hz, {}-bit, {} channels (RateA={}, RateB={}, RateC={})",
                sample_rate, bits_per_sample, channels, rate_a, rate_b, rate_c
            )
            .into(),
        );

        // --- 3. Skip Remaining Header and Read Data ---

        // We are at offset 0x2C (byte 44). Skip the remaining 64 - 44 = 20 bytes of header.
        for _ in 0..(0x40 - 0x2C) {
            let _ = reader
                .read_u8()
                .map_err(|e| format!("Failed to skip remaining header: {}", e))?;
        }

        // Read audio data (starts after the 64-byte header)
        let mut data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            data.push(byte);
        }

        reader.endian = original_endian;

        if data.is_empty() {
            return Err("snd chunk contains no audio data after header".to_string());
        }

        // --- 4. Final Calculation ---

        let bytes_per_sample = (bits_per_sample / 8) as u32;
        let bytes_per_frame = channels as u32 * bytes_per_sample;
        let sample_count = data.len() as u32 / bytes_per_frame;

        // Calculate duration and round to 3 decimal places for comparison
        let duration = (sample_count as f64 / sample_rate as f64 * 1000.0).round() / 1000.0;

        console::log_1(
            &format!(
                "🎵 Final snd: {} Hz, {}-bit, {} bytes → {} samples, {:.3}s",
                sample_rate,
                bits_per_sample,
                data.len(),
                sample_count,
                duration
            )
            .into(),
        );

        Ok(SoundChunk {
            channels,
            sample_rate,
            bits_per_sample,
            sample_count,
            codec: String::from("raw_pcm"),
            data: data_test,
        })
    }

    /// Convert to WAV bytes
    pub fn to_wav(&self) -> Vec<u8> {
        let mut wav = Vec::new();

        let byte_rate = self.sample_rate * self.channels as u32 * self.bits_per_sample as u32 / 8;
        let block_align = self.channels * self.bits_per_sample / 8;

        // RIFF header
        wav.extend_from_slice(b"RIFF");
        let chunk_size = 36 + self.data.len() as u32;
        wav.extend_from_slice(&chunk_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt subchunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&self.channels.to_le_bytes());
        wav.extend_from_slice(&self.sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&self.bits_per_sample.to_le_bytes());

        // data subchunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(self.data.len() as u32).to_le_bytes());

        // Audio data - Director stores 16-bit in big-endian, WAV needs little-endian
        if self.bits_per_sample == 16 {
            for chunk in self.data.chunks_exact(2) {
                wav.push(chunk[1]);
                wav.push(chunk[0]);
            }
            if self.data.len() % 2 == 1 {
                wav.push(*self.data.last().unwrap());
            }
        } else {
            wav.extend_from_slice(&self.data);
        }

        wav
    }

    pub fn from_media(media: &MediaChunk) -> SoundChunk {
        let codec = media.get_codec_name();

        // For IMA ADPCM, the data_size_field contains the uncompressed size
        // Calculate sample_count from uncompressed size, not compressed data
        let (sample_count, bits_per_sample) = if codec == "ima_adpcm" {
            // Director stores data_size_field as the number of SAMPLES, not bytes!
            // This is why we were getting half duration - we were dividing by 2
            let uncompressed_samples = media.data_size_field;
            (uncompressed_samples, 16)
        } else if codec == "mp3" {
            // For MP3, we can't easily calculate sample count without decoding
            // Use compressed size as estimate
            (0, 0)
        } else {
            // Raw PCM - data is in bytes, 16-bit = 2 bytes per sample
            ((media.audio_data.len() / 2) as u32, 16)
        };

        console::log_1(
            &format!(
                "from_media: codec={}, data_size_field={}, audio_data.len()={}, sample_count={}",
                codec,
                media.data_size_field,
                media.audio_data.len(),
                sample_count
            )
            .into(),
        );

        SoundChunk {
            channels: 1,
            sample_rate: media.sample_rate,
            bits_per_sample,
            sample_count,
            codec: codec.to_string(),
            data: media.audio_data.clone(),
        }
    }
}
