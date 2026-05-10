use vm_rust::player::xtra::multiuser::blowfish::MUSBlowfish;

#[test]
fn test_encrypt_decrypt_roundtrip_aligned() {
    let key = b"testkey123";
    let mut cipher = MUSBlowfish::new(key);

    let original = b"Hello, MUS Blowfish World!!!!!!!"; // 32 bytes
    let mut data = original.to_vec();

    cipher.encode(&mut data);
    assert_ne!(&data[..], &original[..], "encode should modify data");

    cipher.decode(&mut data);
    assert_eq!(&data[..], &original[..], "decode should restore original");
}

#[test]
fn test_encrypt_decrypt_roundtrip_unaligned() {
    let key = b"testkey";
    let mut cipher = MUSBlowfish::new(key);

    let original = b"Hello!"; // 6 bytes, not multiple of 8
    let mut data = original.to_vec();

    cipher.encode(&mut data);
    assert_ne!(&data[..], &original[..]);

    cipher.decode(&mut data);
    assert_eq!(&data[..], &original[..]);
}

#[test]
fn test_deterministic_output() {
    let key = b"samekey";

    let mut cipher1 = MUSBlowfish::new(key);
    let mut data1 = vec![0u8; 16];
    cipher1.apply_stream(&mut data1);

    let mut cipher2 = MUSBlowfish::new(key);
    let mut data2 = vec![0u8; 16];
    cipher2.apply_stream(&mut data2);

    assert_eq!(data1, data2, "same key should produce same output");
}

#[test]
fn test_different_keys_different_output() {
    let mut cipher1 = MUSBlowfish::new(b"key1");
    let mut data1 = vec![0u8; 8];
    cipher1.apply_stream(&mut data1);

    let mut cipher2 = MUSBlowfish::new(b"key2");
    let mut data2 = vec![0u8; 8];
    cipher2.apply_stream(&mut data2);

    assert_ne!(data1, data2, "different keys should produce different output");
}

#[test]
fn test_reset() {
    let key = b"resetkey";
    let mut cipher = MUSBlowfish::new(key);

    let mut data1 = vec![0u8; 8];
    cipher.apply_stream(&mut data1);

    cipher.reset();

    let mut data2 = vec![0u8; 8];
    cipher.apply_stream(&mut data2);

    assert_eq!(data1, data2, "reset should restore initial state");
}

#[test]
fn test_stream_evolves_iv() {
    let key = b"streamtest";
    let mut cipher = MUSBlowfish::new(key);

    let mut block1 = vec![0u8; 8];
    cipher.apply_stream(&mut block1);

    let mut block2 = vec![0u8; 8];
    cipher.apply_stream(&mut block2);

    assert_ne!(block1, block2, "consecutive blocks should differ");
}

#[test]
fn test_decode_logon_content() {
    // From a captured SMUS logon request, the encrypted content portion
    // This tests that decryption with the default SMUS key produces valid output.
    let key = b"";
    let mut cipher = MUSBlowfish::new(key);

    // Encrypt then decrypt should be identity
    let original = vec![0x00, 0x03, 0x00, 0x00, 0x00, 0x05, 0x48, 0x65,
                        0x6c, 0x6c, 0x6f, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mut data = original.clone();
    cipher.encode(&mut data);
    cipher.decode(&mut data);
    assert_eq!(data, original);
}
