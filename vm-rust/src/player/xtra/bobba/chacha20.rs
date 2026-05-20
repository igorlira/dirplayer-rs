/// ChaCha20 cipher used by BobbaXtra.
///
/// Per-direction context derived via HKDF: 32-byte key || 12-byte HKDF nonce.
/// The 12-byte nonce is split into a fixed 4-byte prefix (state[13]) and an
/// 8-byte little-endian counter base which is added to the per-direction
/// message counter to form (state[14], state[15]) for each message.
///
/// Block counter (state[12]) starts at 0 and increments per 64-byte chunk.

const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

#[inline]
fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(7);
}

fn block(key: &[u32; 8], block_counter: u32, nonce: &[u32; 3], out: &mut [u8; 64]) {
    let mut state = [0u32; 16];
    state[0..4].copy_from_slice(&CONSTANTS);
    state[4..12].copy_from_slice(key);
    state[12] = block_counter;
    state[13..16].copy_from_slice(nonce);
    let initial = state;

    for _ in 0..10 {
        // Column rounds
        quarter_round(&mut state, 0, 4, 8, 12);
        quarter_round(&mut state, 1, 5, 9, 13);
        quarter_round(&mut state, 2, 6, 10, 14);
        quarter_round(&mut state, 3, 7, 11, 15);
        // Diagonal rounds
        quarter_round(&mut state, 0, 5, 10, 15);
        quarter_round(&mut state, 1, 6, 11, 12);
        quarter_round(&mut state, 2, 7, 8, 13);
        quarter_round(&mut state, 3, 4, 9, 14);
    }

    for i in 0..16 {
        let word = state[i].wrapping_add(initial[i]);
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
}

/// Per-direction stream state derived from the 44-byte HKDF output for one
/// of the four (c2s|s2c)(data|header) contexts.
#[derive(Clone)]
pub struct ChaCha20Direction {
    key: [u32; 8],
    nonce_prefix: u32,
    nonce_base: u64,
}

impl ChaCha20Direction {
    /// Inspect the raw key bytes (used by diagnostic logging only).
    pub fn key_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, w) in self.key.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn nonce_prefix(&self) -> u32 {
        self.nonce_prefix
    }

    pub fn nonce_base(&self) -> u64 {
        self.nonce_base
    }

    /// `okm` is the 44-byte HKDF-Expand output for this direction: key[0..32] || nonce[32..44].
    pub fn from_okm(okm: &[u8; 44]) -> Self {
        let mut key = [0u32; 8];
        for i in 0..8 {
            key[i] = u32::from_le_bytes(okm[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let nonce_prefix = u32::from_le_bytes(okm[32..36].try_into().unwrap());
        let nonce_base = u64::from_le_bytes(okm[36..44].try_into().unwrap());
        ChaCha20Direction { key, nonce_prefix, nonce_base }
    }

    /// XOR `data` in place using the keystream for `msg_counter`.
    /// Block counter resets to 0 at the start of every message.
    pub fn xor(&self, msg_counter: u64, data: &mut [u8]) {
        let msg_nonce = self.nonce_base.wrapping_add(msg_counter);
        let nonce = [
            self.nonce_prefix,
            msg_nonce as u32,
            (msg_nonce >> 32) as u32,
        ];
        let mut block_counter: u32 = 0;
        let mut keystream = [0u8; 64];
        let mut offset = 0;
        while offset < data.len() {
            block(&self.key, block_counter, &nonce, &mut keystream);
            let take = (data.len() - offset).min(64);
            for i in 0..take {
                data[offset + i] ^= keystream[i];
            }
            offset += take;
            block_counter = block_counter.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    // RFC 7539 §2.3.2 test vector: key = 00..1f, nonce = 09000000 4a000000 00000000, counter = 1
    #[test]
    fn block_rfc7539_2_3_2() {
        let mut key = [0u32; 8];
        for i in 0..8 {
            let bytes: [u8; 4] = [
                (i * 4) as u8,
                (i * 4 + 1) as u8,
                (i * 4 + 2) as u8,
                (i * 4 + 3) as u8,
            ];
            key[i] = u32::from_le_bytes(bytes);
        }
        // Nonce bytes: 00 00 00 09 00 00 00 4a 00 00 00 00 → 3 LE-u32 state words.
        let nonce = [0x0900_0000u32, 0x4a00_0000u32, 0x0000_0000u32];
        let mut out = [0u8; 64];
        block(&key, 1, &nonce, &mut out);
        assert_eq!(
            hex(&out),
            "10f1e7e4d13b5915500fdd1fa32071c4c7d1f4c733c068030422aa9ac3d46c4e\
             d2826446079faa0914c2d705d98b02a2b5129cd1de164eb9cbd083e8a2503c4e"
        );
    }

    // RFC 7539 §2.4.2 test vector: encrypts the "Ladies and Gentlemen..." plaintext
    // key = 00..1f, nonce = 00000000 4a000000 00000000, initial counter = 1
    #[test]
    fn rfc7539_2_4_2_encrypt() {
        let mut okm = [0u8; 44];
        for i in 0..32 {
            okm[i] = i as u8;
        }
        // RFC 7539 §2.4.2 nonce bytes: 00 00 00 00 00 00 00 4a 00 00 00 00.
        // We store the 12 nonce bytes verbatim into okm[32..44].
        okm[32..36].copy_from_slice(&[0, 0, 0, 0]);
        okm[36..44].copy_from_slice(&[0, 0, 0, 0x4a, 0, 0, 0, 0]);
        let dir = ChaCha20Direction::from_okm(&okm);

        // RFC's example uses initial block_counter = 1, whereas our xor() starts
        // at 0. Skip the first 64 bytes of keystream by prepending a 64-byte
        // zero pad which xor() will burn for us.
        let plaintext = b"Ladies and Gentlemen of the class of '99: \
If I could offer you only one tip for the future, sunscreen would be it.";
        let mut buf = vec![0u8; 64];
        buf.extend_from_slice(plaintext);
        dir.xor(0, &mut buf);
        let ct = &buf[64..];
        assert_eq!(
            hex(ct),
            "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0b\
             f91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d8\
             07ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab7793736\
             5af90bbf74a35be6b40b8eedf2785e42874d"
        );
    }
}
