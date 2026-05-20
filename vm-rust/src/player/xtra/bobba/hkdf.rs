use super::sha256::{Sha256, BLOCK_SIZE, OUTPUT_SIZE};

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; OUTPUT_SIZE] {
    let mut k = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = {
            let mut h = Sha256::new();
            h.update(key);
            h.finalize()
        };
        k[..OUTPUT_SIZE].copy_from_slice(&digest);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0u8; BLOCK_SIZE];
    let mut opad = [0u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }

    let inner = {
        let mut h = Sha256::new();
        h.update(&ipad);
        h.update(data);
        h.finalize()
    };

    let mut h = Sha256::new();
    h.update(&opad);
    h.update(&inner);
    h.finalize()
}

pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; OUTPUT_SIZE] {
    if salt.is_empty() {
        hmac_sha256(&[0u8; OUTPUT_SIZE], ikm)
    } else {
        hmac_sha256(salt, ikm)
    }
}

pub fn hkdf_expand(prk: &[u8], info: &[u8], out: &mut [u8]) {
    debug_assert!(out.len() <= 255 * OUTPUT_SIZE);
    let mut t: Vec<u8> = Vec::with_capacity(OUTPUT_SIZE + info.len() + 1);
    let mut written = 0;
    let mut counter: u8 = 1;
    while written < out.len() {
        let mut data: Vec<u8> = Vec::with_capacity(t.len() + info.len() + 1);
        data.extend_from_slice(&t);
        data.extend_from_slice(info);
        data.push(counter);
        let block = hmac_sha256(prk, &data);
        let take = (out.len() - written).min(OUTPUT_SIZE);
        out[written..written + take].copy_from_slice(&block[..take]);
        written += take;
        counter = counter.wrapping_add(1);
        t.clear();
        t.extend_from_slice(&block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    // RFC 4231 Test Case 1
    #[test]
    fn hmac_rfc4231_case1() {
        let key = [0x0b; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    // RFC 4231 Test Case 2
    #[test]
    fn hmac_rfc4231_case2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // RFC 5869 Test Case 1
    #[test]
    fn hkdf_rfc5869_case1() {
        let ikm = [0x0b; 22];
        let salt = (0x00u8..=0x0c).collect::<Vec<u8>>();
        let info = (0xf0u8..=0xf9).collect::<Vec<u8>>();
        let prk = hkdf_extract(&salt, &ikm);
        assert_eq!(
            hex(&prk),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        let mut okm = [0u8; 42];
        hkdf_expand(&prk, &info, &mut okm);
        assert_eq!(
            hex(&okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
    }
}
