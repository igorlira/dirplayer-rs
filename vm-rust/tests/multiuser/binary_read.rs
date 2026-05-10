use binary_reader::BinaryReader;
use vm_rust::player::xtra::multiuser::blowfish::DEFAULT_CIPHER_KEY;
use vm_rust::player::xtra::multiuser::{blowfish::MUSBlowfish, reader::MusReader};
use vm_rust::director::static_datum::StaticDatum;


#[test]
fn test_mus_read_logon_request() {
    let data = include_bytes!("messages/mus_logon_request.bin");
    let mut reader = BinaryReader::from_u8(data);
    let mut cipher = MUSBlowfish::new(DEFAULT_CIPHER_KEY);
    let msg = reader.read_mus_message(Some(&mut cipher)).expect("Failed to read logon request");
    let content = msg.content;

    assert_eq!(msg.subject, "Logon");
    assert_eq!(msg.recipients.len(), 1);
    assert_eq!(msg.recipients[0], "System");
    assert_eq!(msg.sender_id, "user");
    if let StaticDatum::List(items) = content {
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], StaticDatum::String("movieId".to_string()));
        assert_eq!(items[1], StaticDatum::String("user".to_string()));
        assert_eq!(items[2], StaticDatum::String("pass".to_string()));
    } else {
        panic!("Expected content to be a list");
    }
}

#[test]
fn test_mus_read_logon_response() {
    let data = include_bytes!("messages/mus_logon_response.bin");
    let mut reader = BinaryReader::from_u8(data);
    let msg = reader.read_mus_message(None).expect("Failed to read logon response");
    assert_eq!(msg.subject, "Logon");
    assert_eq!(msg.recipients.len(), 1);
    assert_eq!(msg.recipients[0], "*");
    assert_eq!(msg.sender_id, "System");
    assert_eq!(msg.error_code, 0);
    assert_eq!(msg.content, StaticDatum::String("Kepler: Habbo Hotel shockwave emulator".to_string()));
}
