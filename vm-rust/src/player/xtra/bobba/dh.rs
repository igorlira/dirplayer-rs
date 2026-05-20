use num::{BigUint, Zero};

/// Finite-field Diffie-Hellman parameters used by BobbaXtra.
/// Hard-coded prime and generator extracted from BobbaXtra.x32
/// (sub_10006370 and sub_10006440).
const PRIME_DEC: &str = "6321588818011308852490424172322127705247412954225642330613911900319542284212329136481845922188834873975036249041\
                        02572293826728806813079";
const GENERATOR_DEC: &str = "23786635532332886537261431906453031264918297";

/// Serialized public/shared-secret byte width: 56 bytes = 448 bits, which
/// matches sub_10005260's 0x38u output size and is large enough for the
/// ~438-bit prime.
pub const PUBLIC_KEY_BYTES: usize = 56;

pub fn prime() -> BigUint {
    BigUint::parse_bytes(PRIME_DEC.as_bytes(), 10).expect("BobbaXtra prime parses")
}

pub fn generator() -> BigUint {
    BigUint::parse_bytes(GENERATOR_DEC.as_bytes(), 10).expect("BobbaXtra generator parses")
}

/// Reduce 56 random bytes into a private exponent x in [2, p-1], matching
/// sub_10006510: x = (raw mod (p - 2)) + 2.
pub fn private_key_from_random(raw: &[u8; PUBLIC_KEY_BYTES], p: &BigUint) -> BigUint {
    let raw_bn = BigUint::from_bytes_be(raw);
    let p_minus_2: BigUint = p - BigUint::from(2u32);
    (raw_bn % p_minus_2) + BigUint::from(2u32)
}

/// Validate a peer public key Y: must be in [1, p-1].
pub fn is_valid_public_key(y: &BigUint, p: &BigUint) -> bool {
    !y.is_zero() && y < p
}

/// Serialize a bignum as big-endian, left-padded to PUBLIC_KEY_BYTES.
pub fn to_fixed_be(value: &BigUint) -> [u8; PUBLIC_KEY_BYTES] {
    let raw = value.to_bytes_be();
    let mut out = [0u8; PUBLIC_KEY_BYTES];
    debug_assert!(raw.len() <= PUBLIC_KEY_BYTES);
    out[PUBLIC_KEY_BYTES - raw.len()..].copy_from_slice(&raw);
    out
}

/// Decimal-ASCII encoding matches BobbaXtra's sub_10002A60.
pub fn to_decimal(value: &BigUint) -> String {
    value.to_str_radix(10)
}

pub fn parse_decimal(s: &str) -> Option<BigUint> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    BigUint::parse_bytes(trimmed.as_bytes(), 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use num::traits::Pow;

    #[test]
    fn prime_and_generator_parse() {
        let p = prime();
        let g = generator();
        assert!(g < p);
        // p has 132 or 133 decimal digits ≈ 438 bits, so it fits in 56 bytes.
        assert!(p.bits() <= (PUBLIC_KEY_BYTES * 8) as u64);
        assert!(p.bits() > 400);
    }

    #[test]
    fn public_key_round_trip() {
        let p = prime();
        let g = generator();
        // Tiny private key for a quick sanity check; modpow itself is exercised
        // at runtime when GeneratePublicKey is called.
        let x = BigUint::from(3u32);
        let y = g.modpow(&x, &p);
        let bytes = to_fixed_be(&y);
        let decoded = BigUint::from_bytes_be(&bytes);
        assert_eq!(decoded, y);
        assert_eq!(y, g.clone().pow(3u32));
    }

    #[test]
    fn private_key_in_range() {
        let p = prime();
        let raw = [0xFFu8; PUBLIC_KEY_BYTES];
        let x = private_key_from_random(&raw, &p);
        assert!(x >= BigUint::from(2u32));
        assert!(x < p);
    }

    #[test]
    fn validate_public_key() {
        let p = prime();
        assert!(!is_valid_public_key(&BigUint::zero(), &p));
        assert!(is_valid_public_key(&BigUint::from(1u32), &p));
        assert!(!is_valid_public_key(&p, &p));
        assert!(is_valid_public_key(&(&p - BigUint::from(1u32)), &p));
    }
}
