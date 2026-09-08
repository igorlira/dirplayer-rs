//! What a buffer of audio bytes IS, asked in one place.
//!
//! These questions used to be answered wherever they were needed: two
//! byte-identical copies of the MPEG frame-length table (MediaChunk and
//! SoundChunk), two of the frame-start scan, and the raw sync-word test
//! `data[0] == 0xFF && (data[1] & 0xE0) == 0xE0` written out six times across
//! four files. Nothing had drifted, but the cost was real and was paid: adding
//! Ogg support meant finding four of those sites by hand and hoping that was
//! all of them.
//!
//! Only what is provably the same lives here. The two frame SCANS are not:
//! MediaChunk accepts a lone frame that ends exactly at the buffer end and
//! skips the last four bytes as a candidate start, SoundChunk rejects the
//! first and allows the second, and it hardcodes a 2048-byte window where
//! MediaChunk takes one. Merging them would quietly change how `snd ` chunks
//! are read, and there is no evidence saying which difference was meant. So
//! each scan stays where it is, calling these primitives and documenting that
//! it differs. Extracting what is identical and NAMING what is not is the
//! whole of the tidy-up; pretending the rest is identical would be a bug.

/// Every Ogg page starts with this capture pattern, the first one included,
/// so for an Ogg stream the test IS the whole test. That is the difference
/// from MPEG audio in one line: an MPEG stream has to be FOUND inside a
/// buffer, which is what everything below exists for, while an Ogg stream
/// simply starts.
pub fn is_ogg(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"OggS"
}

/// An MPEG audio frame header begins with eleven set bits.
///
/// This is a sync word, not a guarantee: eleven set bits occur in ordinary
/// PCM too. Callers that need certainty chain it with `mpeg_frame_len` and
/// require the next header to land where the length says it will.
pub fn has_mpeg_sync(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0
}

/// Length in bytes of the MPEG audio frame starting at `h`, or None if `h`
/// is not a frame header.
///
/// Covers MPEG 1, 2 and 2.5, layers I to III, which is more than Director
/// itself ever wrote but exactly what re-encoded speech clips and the
/// odd MPEG-2 sound effect need.
pub fn mpeg_frame_len(h: &[u8]) -> Option<usize> {
    if !has_mpeg_sync(h) || h.len() < 4 {
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
        3 => base_rate,     // MPEG 1
        2 => base_rate / 2, // MPEG 2
        _ => base_rate / 4, // MPEG 2.5
    };
    let padding = ((h[2] >> 1) & 0x01) as u32;
    let len = if layer == 3 {
        (12 * bitrate / sample_rate + padding) * 4
    } else {
        let coeff = if mpeg1 { 144 } else { 72 };
        coeff * bitrate / sample_rate + padding
    };
    if len < 4 {
        None
    } else {
        Some(len as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128 kbit/s 44100 Hz mono MPEG-1 Layer III header, which is what every
    /// speech clip in the measured movie was: FF FB 90 40 -> 417 bytes + padding bit.
    const MP3_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x40];

    #[test]
    fn reads_a_real_frame_header() {
        assert_eq!(mpeg_frame_len(&MP3_HEADER), Some(417));
    }

    #[test]
    fn rejects_the_reserved_encodings() {
        // bitrate index 15 is "bad", index 0 is "free", sample rate 3 reserved.
        for bad in [[0xFF, 0xFB, 0xF0, 0x40], [0xFF, 0xFB, 0x00, 0x40],
                    [0xFF, 0xFB, 0x9C, 0x40]] {
            assert_eq!(mpeg_frame_len(&bad), None, "{:02X?} should not parse", bad);
        }
    }

    #[test]
    fn sync_alone_is_not_a_frame() {
        // Eleven set bits, then a reserved layer: the sync test says maybe,
        // the length parse says no. That split is the point of having both.
        let h = [0xFF, 0xF9, 0x90, 0x40];
        assert!(has_mpeg_sync(&h));
        assert_eq!(mpeg_frame_len(&h), None);
    }

    #[test]
    fn ogg_is_not_mistaken_for_mpeg() {
        let page = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00";
        assert!(is_ogg(page));
        assert!(!has_mpeg_sync(page));
        assert_eq!(mpeg_frame_len(page), None);
    }


}
