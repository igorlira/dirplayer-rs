/// Pure helpers for `getStreamStatus` mid-file progress and the nested-`.dcr`
/// preloader hold. Kept free of `web_sys` so native unit tests can cover them.

/// True when Lingo `getStreamStatus` is a real mid-file `InProgress`
/// (`0 < bytesSoFar < bytesTotal`). DGS `showGameLoadStats` no-ops at 0% and 100%.
pub fn is_true_mid_file_progress(bytes_loaded: u64, bytes_total: u64) -> bool {
    bytes_loaded > 0 && bytes_total > bytes_loaded
}

/// `#bytesTotal` while still in flight. If the server omitted Content-Length,
/// report loaded-so-far so percentloaded is not divide-by-zero.
pub fn in_progress_bytes_total(bytes_loaded: u64, bytes_total: u64) -> u64 {
    bytes_total.max(bytes_loaded)
}

/// Whether `maybe_hold_dcr_for_preloader` should run for this fetch.
pub fn should_hold_nested_dcr(
    url: &str,
    goto_wait_active: bool,
    is_playing: bool,
    has_flash: bool,
) -> bool {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    if !path.ends_with(".dcr") {
        return false;
    }
    if goto_wait_active || !is_playing {
        return false;
    }
    has_flash
}

/// If the download never opened a mid-file window (one-chunk / cache), the
/// (bytes_loaded, bytes_total) to report so the next `getStreamStatus` is
/// InProgress. `None` if progress is already mid-file.
pub fn synthetic_mid_file_progress(
    loaded: u64,
    reported_total: u64,
    bytes_len: u64,
) -> Option<(u64, u64)> {
    let total = reported_total.max(bytes_len).max(2);
    if loaded == 0 || loaded >= total {
        let mid = (total / 2).max(1).min(total - 1);
        Some((mid, total))
    } else {
        None
    }
}
