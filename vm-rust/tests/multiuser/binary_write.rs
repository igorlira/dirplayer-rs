use binary_reader::BinaryReader;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::xtra::multiuser::blowfish::DEFAULT_CIPHER_KEY;
use vm_rust::player::xtra::multiuser::{
    MultiuserMessage,
    blowfish::MUSBlowfish,
    reader::MusReader,
};

#[test]
fn test_write_read_roundtrip_plain() {
    let msg = MultiuserMessage {
        error_code: 0,
        time_stamp: 12345,
        subject: "Hello".to_string(),
        sender_id: "user1".to_string(),
        recipients: vec!["user2".to_string(), "user3".to_string()],
        content: StaticDatum::String("test content".to_string()),
    };

    let bytes = msg.to_bytes(None);
    let mut reader = BinaryReader::from_u8(&bytes);
    let read_msg = reader.read_mus_message(None).expect("Failed to read message");

    assert_eq!(read_msg.error_code, 0);
    assert_eq!(read_msg.time_stamp, 12345);
    assert_eq!(read_msg.subject, "Hello");
    assert_eq!(read_msg.sender_id, "user1");
    assert_eq!(read_msg.recipients, vec!["user2", "user3"]);
    assert_eq!(read_msg.content, StaticDatum::String("test content".to_string()));
}

#[test]
fn test_write_read_roundtrip_list_content() {
    let msg = MultiuserMessage {
        error_code: 0,
        time_stamp: 0,
        subject: "Data".to_string(),
        sender_id: "server".to_string(),
        recipients: vec!["*".to_string()],
        content: StaticDatum::List(vec![
            StaticDatum::String("abc".to_string()),
            StaticDatum::String("def".to_string()),
        ]),
    };

    let bytes = msg.to_bytes(None);
    let mut reader = BinaryReader::from_u8(&bytes);
    let read_msg = reader.read_mus_message(None).expect("Failed to read message");

    assert_eq!(read_msg.subject, "Data");
    if let StaticDatum::List(items) = read_msg.content {
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], StaticDatum::String("abc".to_string()));
        assert_eq!(items[1], StaticDatum::String("def".to_string()));
    } else {
        panic!("Expected list content");
    }
}

#[test]
fn test_write_read_roundtrip_encrypted_logon() {
    let msg = MultiuserMessage {
        error_code: 0,
        time_stamp: 0,
        subject: "Logon".to_string(),
        sender_id: "user".to_string(),
        recipients: vec!["System".to_string()],
        content: StaticDatum::List(vec![
            StaticDatum::String("movieId".to_string()),
            StaticDatum::String("user".to_string()),
            StaticDatum::String("pass".to_string()),
        ]),
    };

    let mut write_cipher = MUSBlowfish::new(DEFAULT_CIPHER_KEY);
    let bytes = msg.to_bytes(Some(&mut write_cipher));

    let mut read_cipher = MUSBlowfish::new(DEFAULT_CIPHER_KEY);
    let mut reader = BinaryReader::from_u8(&bytes);
    let read_msg = reader.read_mus_message(Some(&mut read_cipher)).expect("Failed to read message");

    assert_eq!(read_msg.subject, "Logon");
    assert_eq!(read_msg.sender_id, "user");
    assert_eq!(read_msg.recipients, vec!["System"]);
    if let StaticDatum::List(items) = read_msg.content {
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], StaticDatum::String("movieId".to_string()));
        assert_eq!(items[1], StaticDatum::String("user".to_string()));
        assert_eq!(items[2], StaticDatum::String("pass".to_string()));
    } else {
        panic!("Expected list content");
    }
}

#[test]
fn test_write_read_roundtrip_void_content() {
    let msg = MultiuserMessage {
        error_code: -1,
        time_stamp: 999,
        subject: "Error".to_string(),
        sender_id: "System".to_string(),
        recipients: vec![],
        content: StaticDatum::Void,
    };

    let bytes = msg.to_bytes(None);
    let mut reader = BinaryReader::from_u8(&bytes);
    let read_msg = reader.read_mus_message(None).expect("Failed to read message");

    assert_eq!(read_msg.error_code, -1);
    assert_eq!(read_msg.time_stamp, 999);
    assert_eq!(read_msg.subject, "Error");
    assert_eq!(read_msg.sender_id, "System");
    assert!(read_msg.recipients.is_empty());
    assert_eq!(read_msg.content, StaticDatum::Void);
}

#[test]
fn test_write_matches_known_logon_request() {
    // Write the same logon request as in the binary test file and verify it reads back correctly
    let expected_data = include_bytes!("messages/mus_logon_request.bin");

    let msg = MultiuserMessage {
        error_code: 0,
        time_stamp: 0,
        subject: "Logon".to_string(),
        sender_id: "user".to_string(),
        recipients: vec!["System".to_string()],
        content: StaticDatum::List(vec![
            StaticDatum::String("movieId".to_string()),
            StaticDatum::String("user".to_string()),
            StaticDatum::String("pass".to_string()),
        ]),
    };

    let mut cipher = MUSBlowfish::new(DEFAULT_CIPHER_KEY);
    let written_bytes = msg.to_bytes(Some(&mut cipher));

    // Verify the written bytes match the expected binary exactly
    assert_eq!(written_bytes.len(), expected_data.len(), "Message length mismatch");
    assert_eq!(written_bytes, expected_data.to_vec(), "Written bytes do not match expected binary");
}
