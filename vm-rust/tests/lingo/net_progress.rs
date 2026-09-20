use vm_rust::player::net_progress::{
    in_progress_bytes_total, is_true_mid_file_progress, should_hold_nested_dcr,
    synthetic_mid_file_progress,
};

#[test]
fn mid_file_progress_is_strictly_between_zero_and_total() {
    assert!(!is_true_mid_file_progress(0, 100));
    assert!(is_true_mid_file_progress(50, 200));
    assert!(!is_true_mid_file_progress(100, 100));
    // Missing Content-Length: Lingo still reports InProgress with total=loaded,
    // which is 100% — DGS showGameLoadStats no-ops that, so it is not "mid-file".
    assert!(!is_true_mid_file_progress(50, 0));
}

#[test]
fn in_progress_total_uses_loaded_when_server_omitted_content_length() {
    assert_eq!(in_progress_bytes_total(50, 0), 50);
    assert_eq!(in_progress_bytes_total(50, 200), 200);
    assert_eq!(in_progress_bytes_total(0, 0), 0);
}

#[test]
fn nested_dcr_hold_requires_playing_dcr_with_flash() {
    let url = "https://swf.neopets.com/games/g349.dcr";
    assert!(should_hold_nested_dcr(url, false, true, true));
    assert!(should_hold_nested_dcr(
        "https://swf.neopets.com/games/g349.DCR?cache=1",
        false,
        true,
        true,
    ));

    assert!(!should_hold_nested_dcr(url, true, true, true)); // go() wait
    assert!(!should_hold_nested_dcr(url, false, false, true)); // initial movie
    assert!(!should_hold_nested_dcr(url, false, true, false)); // no Flash
    assert!(!should_hold_nested_dcr(
        "https://swf.neopets.com/games/g349.dir",
        false,
        true,
        true,
    ));
}

#[test]
fn synthetic_mid_progress_opens_a_window_when_download_was_one_chunk() {
    assert_eq!(synthetic_mid_file_progress(0, 0, 100), Some((50, 100)));
    assert_eq!(synthetic_mid_file_progress(100, 100, 100), Some((50, 100)));
    assert_eq!(synthetic_mid_file_progress(0, 0, 0), Some((1, 2)));
    // Already a real mid-file window — do not overwrite.
    assert_eq!(synthetic_mid_file_progress(50, 200, 200), None);
}
