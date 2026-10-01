use binary_reader::{BinaryReader, Endian};
use std::convert::TryInto;

use log::{debug};

use super::audio_format;

#[derive(Debug, Clone)]
pub struct MediaChunk {
    pub sample_rate: u32,
    pub data_size_field: u32,
    pub guid: Option<[u8; 16]>,
    pub audio_data: Vec<u8>,
    pub is_compressed: bool,
}

impl MediaChunk {
    /// Read one media chunk.
    ///
    /// `may_consume` says the caller will not need the reader's buffer again,
    /// so this can TAKE it instead of copying. An ediM chunk is a whole sound
    /// file, and the measured movie ships 29 MB of speech across 214 of them, so the
    /// difference is a copy of the entire soundtrack on every movie load.
    pub fn from_reader(reader: &mut BinaryReader, may_consume: bool) -> Result<Self, String> {
        let start = reader.pos;
        let end = reader.length.min(reader.data.len());

        // Work out WHAT this chunk is before touching the bytes. All three fast
        // paths keep the tail from some offset and differ only in that offset
        // and the flags, so decide on a borrow and materialise once.
        //   (offset, is_compressed)
        let plan: Option<(usize, bool)> = {
            let all = &reader.data[start..end];
            if audio_format::is_ogg(all) {
                // Ogg Opus, from the re-encoded movies. It has no Director
                // sound header and needs no resync: the stream starts at byte
                // zero, so the whole plan is "keep it all, it is compressed".
                debug!("MediaChunk: detected Ogg stream ({} bytes)", all.len());
                Some((0, true))
            } else if all.len() >= 3 && all[0] == 0xFF && all[1] == 0xD8 && all[2] == 0xFF {
                // JPEG bitmap data, not sound. MediaChunk carries both, and
                // parsing a sound header here would eat the JPEG magic.
                debug!("MediaChunk: detected JPEG data ({} bytes), skipping sound header parse", all.len());
                Some((0, false))
            } else if all.len() >= 10 && &all[0..3] == b"ID3" {
                // An ID3v2-tagged MP3 has no Director sound header either, and
                // parsing one reads the tag's own text as numeric fields
                // (AreaZero: headerSize 0x49443303 = "ID3", sampleRate
                // 0x07765443, dataSizeField 0x00426C75 - the TCON genre frame).
                // ID3v2 sizes are synchsafe: 7 bits per byte, high bit clear.
                let flags = all[5];
                let tag_size = ((all[6] as usize & 0x7F) << 21)
                    | ((all[7] as usize & 0x7F) << 14)
                    | ((all[8] as usize & 0x7F) << 7)
                    | (all[9] as usize & 0x7F);
                // 10-byte header, plus a 10-byte footer when the footer flag is set.
                let tag_len = 10 + tag_size + if flags & 0x10 != 0 { 10 } else { 0 };
                if tag_len < all.len() {
                    let after = &all[tag_len..];
                    // Some encoders (WMP-tagged MP3 speech) leave junk
                    // between the tag end and the first MPEG frame; resync to a
                    // verified frame chain so codec sniffing sees a real sync.
                    let at_sync = audio_format::has_mpeg_sync(after);
                    let extra = if at_sync {
                        0
                    } else {
                        Self::find_mp3_frame_chain(after).unwrap_or(0)
                    };
                    debug!(
                        "MediaChunk: ID3v2 tag of {} bytes stripped (+{} junk), {} bytes of MP3 remain",
                        tag_len, extra, after.len() - extra
                    );
                    Some((tag_len + extra, true))
                } else {
                    debug!(
                        "MediaChunk: ID3v2 tag length {} >= data length {}; parsing as-is",
                        tag_len, all.len()
                    );
                    None
                }
            } else if let Some(off) =
                Self::find_mp3_frame_chain_windowed(all, all.len().min(2048))
            {
                // A bare MP3 stream: the measured movie stores its speech that way,
                // usually starting directly on a frame sync (whose first u32,
                // e.g. 0xFFFA90C0, used to be read as headerSize, consuming the
                // sync and yielding "raw PCM" at a garbage sample rate),
                // occasionally after a few hundred junk bytes. A verified frame
                // CHAIN within the first 2 KB is required, so a genuine ediM
                // sound header - small big-endian headerSize, first byte 0x00 -
                // is never misread as MPEG audio.
                debug!(
                    "MediaChunk: detected bare MP3 stream at offset {} ({} bytes), skipping sound header parse",
                    off,
                    all.len() - off
                );
                Some((off, true))
            } else {
                None
            }
        };

        if let Some((off, is_compressed)) = plan {
            // Take the buffer when the caller has said it is ours; otherwise
            // copy the tail. `drain` shifts in place, so `off > 0` costs no
            // allocation either.
            let mut audio_data = if may_consume && start == 0 && end == reader.data.len() {
                reader.pos = end;
                std::mem::take(&mut reader.data)
            } else {
                reader.data[start..end].to_vec()
            };
            if off > 0 {
                audio_data.drain(..off);
            }
            return Ok(MediaChunk {
                // Both compressed forms carry their own rate in the stream.
                sample_rate: 0,
                data_size_field: audio_data.len() as u32,
                guid: None,
                audio_data,
                is_compressed,
            });
        }

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

        // Read all remaining data as audio data, in one go rather than per byte.
        let remaining = reader.length.saturating_sub(reader.pos);
        let audio_data: Vec<u8> = reader
            .read_bytes(remaining)
            .map(|b| b.to_vec())
            .unwrap_or_default();

        // Detect compression type
        // MP3: starts with 0xFF 0xFx
        let is_mp3 = audio_format::has_mpeg_sync(&audio_data);

        // IMA ADPCM: data is significantly smaller than data_size_field
        // data_size_field represents uncompressed PCM size
        let compression_ratio = if audio_data.len() > 0 {
            data_size_field as f32 / audio_data.len() as f32
        } else {
            1.0
        };

        let is_ima_adpcm = compression_ratio > 2.0 && !is_mp3;
        let is_compressed = is_mp3 || is_ima_adpcm;

        debug!(
            "MediaChunk: {} bytes (expected {}), ratio={:.2}, mp3={}, ima_adpcm={}, rate={}",
            audio_data.len(),
            data_size_field,
            compression_ratio,
            is_mp3,
            is_ima_adpcm,
            sample_rate
        );

        reader.endian = original_endian;

        Ok(MediaChunk {
            sample_rate,
            data_size_field,
            guid,
            audio_data,
            is_compressed,
        })
    }


    /// Offset of the first MPEG frame whose declared length lands on another
    /// valid frame header (or exactly at end of data). Scans the whole buffer.
    fn find_mp3_frame_chain(data: &[u8]) -> Option<usize> {
        Self::find_mp3_frame_chain_windowed(data, data.len())
    }

    /// Like `find_mp3_frame_chain`, but only considers candidate frame STARTS
    /// within the first `window` bytes; chains are still verified against the
    /// full buffer.
    fn find_mp3_frame_chain_windowed(data: &[u8], window: usize) -> Option<usize> {
        for off in 0..window.min(data.len().saturating_sub(4)) {
            if let Some(len) = audio_format::mpeg_frame_len(&data[off..]) {
                match data.get(off + len..) {
                    Some(next) if audio_format::mpeg_frame_len(next).is_some() => return Some(off),
                    Some(rest) if rest.is_empty() => return Some(off),
                    None => return Some(off),
                    _ => {}
                }
            }
        }
        None
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

        if audio_format::is_ogg(&self.audio_data) {
            return "ogg";
        }

        // Check for MP3
        if audio_format::has_mpeg_sync(&self.audio_data) {
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
