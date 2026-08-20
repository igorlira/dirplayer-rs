use binary_reader::{BinaryReader, Endian};

use log::debug;
use wasm_bindgen::JsValue;
use web_sys::console;

use js_sys::Float32Array;

use crate::director::chunks::MediaChunk;

/// Parsed "sndH" chunk - for Director 6+ sounds.
#[derive(Clone, Debug)]
pub struct SndHeaderChunk {
    pub offset: i32,
    pub size: i32,
    pub playback_start: i32,
    pub playback_start_frame: i32,
    pub loop_start: i32,
    pub loop_start_frame: i32,
    pub loop_end: i32,
    pub loop_end_frame: i32,
    pub playback_end: i32,
    pub playback_end_frame: i32,
    pub num_frames: i32,
    pub frame_rate: i32,
    pub byte_rate: i32,
    pub compression_type: [u8; 16],
    pub bits_per_sample: i32,
    pub bytes_per_sample: i32,
    pub num_channels: i32,
    pub bytes_per_frame: i32,
    pub sound_header_type: [u8; 16],
    pub bytes_per_block: i32,
    /// Whether the file that contained this header was big-endian (RIFX/Mac).
    /// sndS audio data follows the file's byte order.
    pub file_endian_is_big: bool,
}

impl SndHeaderChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<SndHeaderChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let offset = reader.read_i32().map_err(|e| format!("sndH offset: {}", e))?;
        let size = reader.read_i32().map_err(|e| format!("sndH size: {}", e))?;
        let playback_start = reader.read_i32().map_err(|e| format!("sndH playbackStart: {}", e))?;
        let playback_start_frame = reader.read_i32().map_err(|e| format!("sndH playbackStartFrame: {}", e))?;
        let loop_start = reader.read_i32().map_err(|e| format!("sndH loopStart: {}", e))?;
        let loop_start_frame = reader.read_i32().map_err(|e| format!("sndH loopStartFrame: {}", e))?;
        let loop_end = reader.read_i32().map_err(|e| format!("sndH loopEnd: {}", e))?;
        let loop_end_frame = reader.read_i32().map_err(|e| format!("sndH loopEndFrame: {}", e))?;
        let playback_end = reader.read_i32().map_err(|e| format!("sndH playbackEnd: {}", e))?;
        let playback_end_frame = reader.read_i32().map_err(|e| format!("sndH playbackEndFrame: {}", e))?;
        let num_frames = reader.read_i32().map_err(|e| format!("sndH numFrames: {}", e))?;
        let frame_rate = reader.read_i32().map_err(|e| format!("sndH frameRate: {}", e))?;
        let byte_rate = reader.read_i32().map_err(|e| format!("sndH byteRate: {}", e))?;

        let mut compression_type = [0u8; 16];
        for i in 0..16 {
            compression_type[i] = reader.read_u8().map_err(|e| format!("sndH compressionType: {}", e))?;
        }

        let bits_per_sample = reader.read_i32().map_err(|e| format!("sndH bitsPerSample: {}", e))?;
        let bytes_per_sample = reader.read_i32().map_err(|e| format!("sndH bytesPerSample: {}", e))?;
        let num_channels = reader.read_i32().map_err(|e| format!("sndH numChannels: {}", e))?;
        let bytes_per_frame = reader.read_i32().map_err(|e| format!("sndH bytesPerFrame: {}", e))?;

        let mut sound_header_type = [0u8; 16];
        for i in 0..16 {
            sound_header_type[i] = reader.read_u8().map_err(|e| format!("sndH soundHeaderType: {}", e))?;
        }

        // Skip platformData (63 × u32 = 252 bytes)
        for _ in 0..63 {
            let _ = reader.read_u32();
        }

        let bytes_per_block = reader.read_i32().unwrap_or(0);
        let file_endian_is_big = original_endian == Endian::Big;

        reader.endian = original_endian;

        debug!(
            "sndH: offset={}, size={}, numFrames={}, frameRate={}, byteRate={}, bitsPerSample={}, bytesPerSample={}, numChannels={}, bytesPerFrame={}, bytesPerBlock={}",
            offset, size, num_frames, frame_rate, byte_rate, bits_per_sample, bytes_per_sample, num_channels, bytes_per_frame, bytes_per_block
        );

        let compression_str = String::from_utf8_lossy(&compression_type);
        let header_type_str = String::from_utf8_lossy(&sound_header_type);
        debug!(
            "sndH: compressionType='{}', soundHeaderType='{}'",
            compression_str.trim_end_matches('\0'),
            header_type_str.trim_end_matches('\0')
        );

        Ok(SndHeaderChunk {
            offset,
            size,
            playback_start,
            playback_start_frame,
            loop_start,
            loop_start_frame,
            loop_end,
            loop_end_frame,
            playback_end,
            playback_end_frame,
            num_frames,
            frame_rate,
            byte_rate,
            compression_type,
            bits_per_sample,
            bytes_per_sample,
            num_channels,
            bytes_per_frame,
            sound_header_type,
            bytes_per_block,
            file_endian_is_big,
        })
    }
}

#[derive(Clone)]
pub struct SoundChunk {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    sample_count: u32,
    codec: String,
    data: Vec<u8>,
    pub version: u16,
    /// Whether the audio data is stored in big-endian byte order.
    /// Mac "snd " resources are always big-endian; sndH/sndS follows the file's byte order.
    big_endian_data: bool,
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
            version: 0,
            big_endian_data: true,
        }
    }

    pub fn from_wav(bytes: &[u8]) -> Result<SoundChunk, String> {
        if bytes.len() < 12 { return Err("WAV too short".into()); }
        if &bytes[0..4] != b"RIFF" { return Err("not a RIFF file".into()); }
        if &bytes[8..12] != b"WAVE" { return Err("not a WAVE file".into()); }

        let mut pos = 12usize;
        let mut channels = 1u16;
        let mut sample_rate = 44100u32;
        let mut bits_per_sample = 16u16;
        let mut pcm_data: &[u8] = &[];
        let mut found_fmt = false;
        let mut found_data = false;

        while pos + 8 <= bytes.len() {
            let chunk_id = &bytes[pos..pos + 4];
            let chunk_size =
                u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            pos += 8;
            let chunk_end = (pos + chunk_size).min(bytes.len());

            if chunk_id == b"fmt " && chunk_size >= 16 && pos + 16 <= bytes.len() {
                channels = u16::from_le_bytes(bytes[pos + 2..pos + 4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
                bits_per_sample =
                    u16::from_le_bytes(bytes[pos + 14..pos + 16].try_into().unwrap());
                found_fmt = true;
            } else if chunk_id == b"data" {
                pcm_data = &bytes[pos..chunk_end];
                found_data = true;
            }

            pos = chunk_end;
            if chunk_size % 2 != 0 { pos += 1; }
        }

        if !found_fmt || !found_data {
            return Err("WAV missing fmt or data chunk".into());
        }

        let bytes_per_sample = (bits_per_sample / 8).max(1) as usize;
        let frame_size = bytes_per_sample * channels as usize;
        let sample_count = if frame_size > 0 { pcm_data.len() / frame_size } else { 0 } as u32;

        Ok(SoundChunk {
            channels,
            sample_rate,
            bits_per_sample,
            sample_count,
            codec: "raw_pcm".into(),
            data: pcm_data.to_vec(),
            version: 0,
            big_endian_data: false,
        })
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

    pub fn big_endian_data(&self) -> bool {
        self.big_endian_data
    }

    pub fn set_metadata(&mut self, sample_rate: u32, channels: u16, bits_per_sample: u16) {
        self.sample_rate = sample_rate;
        self.channels = channels;
        self.bits_per_sample = bits_per_sample;
        self.sample_count =
            (self.data.len() / (channels as usize * (bits_per_sample / 8) as usize)) as u32;
        log::debug!(
            "Updated metadata: channels={}, sample_rate={}, bits={}",
            self.channels, self.sample_rate, self.bits_per_sample
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

        debug!("Debug Sample Array size: {}", output_array.length());
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
            version: 0,
            big_endian_data: true,
        }
    }
}

impl SoundChunk {
    /// Length in bytes of the MPEG audio frame whose 4-byte header starts at `h`,
    /// or None if `h` is not a valid MPEG-1/2/2.5 Layer I-III header.
    fn mp3_frame_len(h: &[u8]) -> Option<usize> {
        if h.len() < 4 || h[0] != 0xFF || (h[1] & 0xE0) != 0xE0 {
            return None;
        }
        let version_id = (h[1] >> 3) & 0x03; // 01 is reserved
        let layer = (h[1] >> 1) & 0x03; // 00 is reserved
        let bitrate_idx = (h[2] >> 4) & 0x0F; // 0 = free, 15 = bad
        let rate_idx = (h[2] >> 2) & 0x03; // 3 is reserved
        if version_id == 1 || layer == 0 || bitrate_idx == 0 || bitrate_idx == 15 || rate_idx == 3 {
            return None;
        }
        let mpeg1 = version_id == 3;
        // kbps tables, indexed by bitrate_idx
        const V1L1: [u32; 16] = [0,32,64,96,128,160,192,224,256,288,320,352,384,416,448,0];
        const V1L2: [u32; 16] = [0,32,48,56,64,80,96,112,128,160,192,224,256,320,384,0];
        const V1L3: [u32; 16] = [0,32,40,48,56,64,80,96,112,128,160,192,224,256,320,0];
        const V2L1: [u32; 16] = [0,32,48,56,64,80,96,112,128,144,160,176,192,224,256,0];
        const V2L23: [u32; 16] = [0,8,16,24,32,40,48,56,64,80,96,112,128,144,160,0];
        let bitrate = match (mpeg1, layer) {
            (true, 3) => V1L1[bitrate_idx as usize],
            (true, 2) => V1L2[bitrate_idx as usize],
            (true, 1) => V1L3[bitrate_idx as usize],
            (false, 3) => V2L1[bitrate_idx as usize],
            (false, _) => V2L23[bitrate_idx as usize],
            _ => 0,
        } * 1000;
        if bitrate == 0 {
            return None;
        }
        let base_rate = [44100u32, 48000, 32000][rate_idx as usize];
        let sample_rate = match version_id {
            3 => base_rate,      // MPEG 1
            2 => base_rate / 2,  // MPEG 2
            _ => base_rate / 4,  // MPEG 2.5
        };
        let padding = ((h[2] >> 1) & 0x01) as u32;
        let len = if layer == 3 {
            // Layer I frames are measured in 4-byte slots
            (12 * bitrate / sample_rate + padding) * 4
        } else {
            let coeff = if mpeg1 { 144 } else { 72 };
            coeff * bitrate / sample_rate + padding
        };
        if len < 4 { None } else { Some(len as usize) }
    }

    /// Offset of the first MP3 frame in `data`, requiring a second valid frame at the
    /// distance the first one declares. Only a short prefix is scanned — a Shockwave
    /// Audio payload begins within the first few bytes, and scanning further would
    /// risk matching noise deep inside genuine PCM.
    fn find_mp3_start(data: &[u8]) -> Option<usize> {
        let limit = data.len().min(2048);
        for off in 0..limit {
            if let Some(len) = Self::mp3_frame_len(&data[off..]) {
                match data.get(off + len..) {
                    // Second frame must follow immediately.
                    Some(next) if Self::mp3_frame_len(next).is_some() => return Some(off),
                    // A single frame at the very start is still an MP3 (short SFX).
                    None if off + len >= data.len() => return Some(off),
                    _ => {}
                }
            }
        }
        None
    }

    pub fn from_snd_chunk(reader: &mut BinaryReader, version: u16) -> Result<SoundChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let start_pos = reader.pos;

        // Read all bytes for reference
        let mut all_bytes = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            all_bytes.push(byte);
        }
        reader.pos = start_pos;

        if all_bytes.len() < 10 {
            reader.endian = original_endian;
            return Err(format!("snd chunk too short: {} bytes", all_bytes.len()));
        }

        debug!("Parsing Mac snd resource ({} bytes)", all_bytes.len());

        // --- Parse Mac snd resource header ---
        // Format: type 1 (0x0001) or type 2 (0x0002)
        let format_type = reader.read_u16().map_err(|e| format!("Failed to read format type: {}", e))?;

        let num_commands: u16;
        match format_type {
            1 => {
                // Type 1: number of data types (modifiers), then modifiers, then commands
                let num_data_types = reader.read_u16().map_err(|e| format!("Type 1: {}", e))?;
                for _ in 0..num_data_types {
                    let _modifier_type = reader.read_u16().map_err(|e| format!("Modifier type: {}", e))?;
                    let _modifier_data = reader.read_u32().map_err(|e| format!("Modifier data: {}", e))?;
                }
                num_commands = reader.read_u16().map_err(|e| format!("Num commands: {}", e))?;
            }
            2 => {
                // Type 2: reference count, then commands
                let _ref_count = reader.read_u16().map_err(|e| format!("Ref count: {}", e))?;
                num_commands = reader.read_u16().map_err(|e| format!("Num commands: {}", e))?;
            }
            _ => {
                // Unknown format type - could be raw audio data or different format
                // Fall back: treat entire data as audio with default settings
                reader.endian = original_endian;
                debug!("Unknown snd format type 0x{:04X}, treating as raw audio", format_type);
                return Ok(SoundChunk {
                    channels: 1,
                    sample_rate: 22050,
                    bits_per_sample: 16,
                    sample_count: (all_bytes.len() / 2) as u32,
                    codec: "raw_pcm".to_string(),
                    data: all_bytes,
                    version,
                    big_endian_data: true,
                });
            }
        }

        // Read sound commands, look for bufferCmd (0x8051 or 0x0051)
        let mut sound_header_offset: Option<usize> = None;
        for _ in 0..num_commands {
            let cmd = reader.read_u16().map_err(|e| format!("Command: {}", e))?;
            let _param1 = reader.read_u16().map_err(|e| format!("Param1: {}", e))?;
            let param2 = reader.read_u32().map_err(|e| format!("Param2: {}", e))?;

            // bufferCmd = 0x0051, with data offset flag = 0x8051
            if (cmd & 0x7FFF) == 0x0051 {
                sound_header_offset = Some(param2 as usize);
            }
        }

        // Sound data header follows commands, or is at the offset specified by bufferCmd
        let header_pos = match sound_header_offset {
            Some(offset) => start_pos + offset,
            None => reader.pos, // Immediately after commands
        };
        reader.pos = header_pos;

        // --- Parse Sound Data Header ---
        let _sample_ptr = reader.read_u32().map_err(|e| format!("samplePtr: {}", e))?;
        let length_or_channels = reader.read_u32().map_err(|e| format!("length/channels: {}", e))?;
        let sample_rate_fixed = reader.read_u32().map_err(|e| format!("sampleRate: {}", e))?;
        let _loop_start = reader.read_u32().map_err(|e| format!("loopStart: {}", e))?;
        let _loop_end = reader.read_u32().map_err(|e| format!("loopEnd: {}", e))?;
        let encode = reader.read_u8().map_err(|e| format!("encode: {}", e))?;
        let _base_frequency = reader.read_u8().map_err(|e| format!("baseFrequency: {}", e))?;

        // The Mac snd resource stores sampleRate as 16.16 Fixed, so 22050 Hz is
        // 0x56220000 and the integer part is the high word. Director 11.5 members
        // instead store a PLAIN integer in the low word — AreaZero and Agent Free Ride
        // both carry 0x00005622 (= 22050), where `>> 16` yields 0. A rate of 0 makes
        // Web Audio's createBuffer throw, so every one of those sounds was dropped.
        //
        // Take the high word when it is a usable rate, else fall back to the low word.
        // Both are checked against Web Audio's accepted range rather than assumed, so a
        // genuinely malformed field still reports 0 instead of a fabricated rate.
        const MIN_RATE: u32 = 3000;
        const MAX_RATE: u32 = 768_000;
        let fixed_point_rate = sample_rate_fixed >> 16;
        let plain_int_rate = sample_rate_fixed & 0xFFFF;
        let sample_rate = if (MIN_RATE..=MAX_RATE).contains(&fixed_point_rate) {
            fixed_point_rate
        } else if (MIN_RATE..=MAX_RATE).contains(&plain_int_rate) {
            plain_int_rate
        } else {
            fixed_point_rate
        };

        let (channels, bits_per_sample, sample_count, audio_data_start);

        match encode {
            0x00 => {
                // Standard Sound Header (stdSH) - 8-bit unsigned mono
                // length_or_channels = numSamples
                channels = 1;
                bits_per_sample = 8;
                sample_count = length_or_channels;
                // Audio data starts immediately after the 22-byte header
                audio_data_start = (header_pos - start_pos) + 22;
                debug!(
                    "stdSH: {} Hz, 8-bit mono, {} samples, audio at offset {}",
                    sample_rate, sample_count, audio_data_start
                );
            }
            0xFF => {
                // Extended Sound Header (extSH) - can be 8 or 16 bit, mono or stereo
                // length_or_channels = numChannels
                channels = length_or_channels as u16;
                let num_frames = reader.read_u32().map_err(|e| format!("numFrames: {}", e))?;
                // Skip: AIFFSampleRate (10) + markerChunk (4) + instrumentChunks (4) + AESRecording (4)
                for _ in 0..22 {
                    let _ = reader.read_u8();
                }
                let sample_size = reader.read_u16().map_err(|e| format!("sampleSize: {}", e))?;
                bits_per_sample = if sample_size == 0 { 16 } else { sample_size };
                sample_count = num_frames;
                // Audio data starts at offset 64 from sound data header
                audio_data_start = (header_pos - start_pos) + 64;
                debug!(
                    "extSH: {} Hz, {}-bit, {} ch, {} frames, audio at offset {}",
                    sample_rate, bits_per_sample, channels, num_frames, audio_data_start
                );
            }
            0xFE => {
                // Compressed Sound Header (cmpSH)
                // Similar to extended header but with compression info
                channels = length_or_channels as u16;
                let num_frames = reader.read_u32().map_err(|e| format!("numFrames: {}", e))?;
                // Skip to get compression format info
                for _ in 0..22 {
                    let _ = reader.read_u8();
                }
                let sample_size = reader.read_u16().map_err(|e| format!("sampleSize: {}", e))?;
                bits_per_sample = if sample_size == 0 { 16 } else { sample_size };
                sample_count = num_frames;
                audio_data_start = (header_pos - start_pos) + 64;
                debug!(
                    "cmpSH: {} Hz, {}-bit, {} ch, {} frames, audio at offset {}",
                    sample_rate, bits_per_sample, channels, num_frames, audio_data_start
                );
            }
            _ => {
                // Non-Mac encode byte (Director writes 0x01). TWO different layouts
                // arrive here and they must be told apart by the length field, not by
                // the encode byte — assuming either one breaks the other movie:
                //
                //  * stdSH-like, 8-bit UNSIGNED mono: `length_or_channels` is the
                //    sample count, which for 8-bit equals the audio byte count.
                //    Agent Free Ride ships these (snd_music: field 1245312, audio
                //    1245312 bytes).
                //  * extSH-like, 16-bit BIG-ENDIAN: `length_or_channels` is the
                //    CHANNEL count (1 or 2), nothing like the byte count. AreaZero
                //    ships these (PlayerFootstep1_SFX: field 1, audio 11064 bytes).
                //
                // Verified by scoring the mean sample-to-sample delta of every
                // candidate reading (real audio is smooth, a wrong one reads as noise):
                //   AFR  snd_music:           8u=0.014  8s=0.21  16BE=0.21  16LE=0.24
                //   AZ   PlayerFootstep1_SFX: 8u=0.164  8s=0.008 16BE=0.008 16LE=0.46
                audio_data_start = (header_pos - start_pos) + 22;
                let audio_bytes = all_bytes.len().saturating_sub(audio_data_start) as u32;
                if length_or_channels == audio_bytes {
                    channels = 1;
                    bits_per_sample = 8;
                    sample_count = length_or_channels;
                } else {
                    channels = if length_or_channels == 2 { 2 } else { 1 };
                    bits_per_sample = 16;
                    // The length field is a channel count here, so Director recorded
                    // no length: derive it from the byte count. AreaZero needs a real
                    // one — its Sound Manager queues playback with
                    // `#endTime: pMember.duration`, which is 0 without it.
                    //
                    // This is only safe because the codec sniff below now finds an MP3
                    // stream that starts a few bytes in. Compressed payloads become
                    // codec=mp3 with sample_count 0, so a byte-derived count is never
                    // applied to them (deriving one for Rasterwerks' SWA members made
                    // the SWA trim clip playback to a fictitious 0.261s).
                    sample_count = audio_bytes / (2 * channels as u32).max(1);
                }
                debug!(
                    "Encode 0x{:02X} (Director variant): {} Hz, {}-bit, {} ch, {} samples, audio at offset {} (lengthField={}, audioBytes={})",
                    encode, sample_rate, bits_per_sample, channels, sample_count,
                    audio_data_start, length_or_channels, audio_bytes
                );
            }
        }

        reader.endian = original_endian;

        // Extract only the audio data bytes (no snd resource header)
        let audio_data = if audio_data_start < all_bytes.len() {
            all_bytes[audio_data_start..].to_vec()
        } else {
            debug!("Warning: audio_data_start {} >= data length {}", audio_data_start, all_bytes.len());
            Vec::new()
        };

        if audio_data.is_empty() {
            return Err("snd chunk contains no audio data".to_string());
        }

        // Detect codec (MP3 vs PCM). Sniffing only offset 0 misses Shockwave Audio
        // members whose MP3 stream starts a few bytes in — Rasterwerks' SFX are like
        // this. They were mislabelled raw_pcm, decoded as PCM, and their `sample_count`
        // placeholder of 1 was the only thing keeping the SWA trim off them.
        //
        // `find_mp3_start` scans a short prefix and requires TWO consecutive valid
        // frame headers, so a chance 0xFF 0xEx byte pair inside real PCM does not
        // trigger it.
        let mp3_offset = Self::find_mp3_start(&audio_data);
        let audio_data = match mp3_offset {
            Some(0) | None => audio_data,
            Some(off) => {
                debug!("snd: MP3 frame sync found at +{}, trimming leading bytes", off);
                audio_data[off..].to_vec()
            }
        };
        let is_mp3 = mp3_offset.is_some();
        let codec = if is_mp3 { "mp3" } else { "raw_pcm" };

        let final_sample_count = if is_mp3 { 0 } else { sample_count };

        debug!(
            "Final snd: {} Hz, {}-bit, {} ch, codec={}, {} audio bytes, {} samples",
            sample_rate, bits_per_sample, channels, codec, audio_data.len(), final_sample_count
        );

        Ok(SoundChunk {
            channels,
            sample_rate,
            bits_per_sample,
            sample_count: final_sample_count,
            codec: codec.to_string(),
            data: audio_data,
            version,
            big_endian_data: true, // Mac snd resources are always big-endian
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

        // Audio data - big-endian 16-bit needs byte-swap to little-endian for WAV
        if self.bits_per_sample == 16 && self.big_endian_data {
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

        debug!(
            "from_media: codec={}, data_size_field={}, audio_data.len()={}, sample_count={}",
            codec,
            media.data_size_field,
            media.audio_data.len(),
            sample_count
        );

        SoundChunk {
            channels: 1,
            sample_rate: media.sample_rate,
            bits_per_sample,
            sample_count,
            codec: codec.to_string(),
            data: media.audio_data.clone(),
            version: 0,
            big_endian_data: true, // Director media chunks are big-endian
        }
    }

    /// Create a SoundChunk from sndH (header) and sndS (samples) chunks.
    /// Uses MoaSoundFormat fields from the sndH header for metadata.
    pub fn from_snd_header_and_samples(header: &SndHeaderChunk, samples: &[u8]) -> SoundChunk {
        let bits_per_sample = if header.bits_per_sample > 0 {
            header.bits_per_sample as u16
        } else {
            16 // default
        };
        let channels = if header.num_channels > 0 {
            header.num_channels as u16
        } else {
            1
        };
        // frameRate is the sample rate, but some sndH headers carry 0 there (seen on
        // AreaZero and Agent Free Ride members). It used to pass through unguarded —
        // unlike bitsPerSample/numChannels above — and a rate of 0 makes Web Audio's
        // createBuffer throw NotSupportedError ("outside the range [3000, 768000]"),
        // so the sound was dropped entirely.
        //
        // Recover it from the header's own byteRate rather than inventing a rate:
        //   byteRate = sampleRate * channels * bytesPerSample
        // and only fall back to a default if that is unusable too.
        let sample_rate = if header.frame_rate > 0 {
            header.frame_rate as u32
        } else {
            let bytes_per_sample = if header.bytes_per_sample > 0 {
                header.bytes_per_sample as u32
            } else {
                (bits_per_sample as u32 + 7) / 8
            };
            let divisor = channels as u32 * bytes_per_sample.max(1);
            let derived = if header.byte_rate > 0 && divisor > 0 {
                header.byte_rate as u32 / divisor
            } else {
                0
            };
            if (3000..=768_000).contains(&derived) {
                debug!(
                    "[SND] sndH frameRate is 0; derived {} Hz from byteRate {} ({} ch x {} bytes)",
                    derived, header.byte_rate, channels, bytes_per_sample
                );
                derived
            } else {
                debug!(
                    "[SND] sndH frameRate is 0 and byteRate {} gives no usable rate ({}); defaulting to 22050 Hz",
                    header.byte_rate, derived
                );
                22050
            }
        };

        // Determine codec from compression_type GUID
        let codec = {
            // Check for null/empty compression type (= raw PCM)
            let is_null = header.compression_type.iter().all(|&b| b == 0);
            if is_null {
                "raw_pcm".to_string()
            } else {
                // Check known GUIDs
                // IMA ADPCM: 5A08CD40-535B-11D0-...
                if header.compression_type[0..4] == [0x5A, 0x08, 0xCD, 0x40] {
                    "ima_adpcm".to_string()
                } else {
                    let type_str = String::from_utf8_lossy(&header.compression_type);
                    debug!("Unknown compression type: {:02X?} ('{}')", header.compression_type, type_str.trim_end_matches('\0'));
                    "raw_pcm".to_string()
                }
            }
        };

        // Calculate sample count
        // num_frames from header is the frame count
        let sample_count = if header.num_frames > 0 {
            header.num_frames as u32
        } else {
            // Fall back to computing from data length
            let bytes_per_sample_val = if bits_per_sample > 0 { (bits_per_sample / 8) as usize } else { 2 };
            let ch = channels as usize;
            if bytes_per_sample_val > 0 && ch > 0 {
                (samples.len() / (bytes_per_sample_val * ch)) as u32
            } else {
                0
            }
        };

        debug!(
            "from_snd_header_and_samples: rate={}, bits={}, ch={}, codec={}, numFrames={}, samples_len={}, sample_count={}",
            sample_rate, bits_per_sample, channels, codec, header.num_frames, samples.len(), sample_count
        );

        SoundChunk {
            channels,
            sample_rate,
            bits_per_sample,
            sample_count,
            codec,
            data: samples.to_vec(),
            version: 0,
            big_endian_data: header.file_endian_is_big, // sndS data follows file byte order
        }
    }
}
