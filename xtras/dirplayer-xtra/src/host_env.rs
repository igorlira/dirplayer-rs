/// Bindings to the host-env imports provided by dirplayer.
///
/// These are declared as `extern "C"` functions that the host wires up during
/// WASM instantiation.  Plugin code calls these directly; the `#[xtra_plugin]`
/// macro re-exports them under the `dirplayer_xtra::host_env` path.

extern "C" {
    /// Write a debug-level log message.
    fn dirplayer_host_log(msg_ptr: *const u8, msg_len: i32);

    /// Fill a buffer in the plugin's linear memory with cryptographically
    /// random bytes.  Returns 0 on success, -1 on failure.
    fn dirplayer_host_random_fill(buf_ptr: *mut u8, len: i32) -> i32;

    /// Read a value from persistent storage (localStorage).
    /// Writes the UTF-8 value into `result_ptr` (up to `result_max_len` bytes).
    /// Returns the actual number of bytes written, or -1 if the key is absent.
    fn dirplayer_host_storage_get(
        key_ptr: *const u8,
        key_len: i32,
        result_ptr: *mut u8,
        result_max_len: i32,
    ) -> i32;

    /// Write a key/value pair to persistent storage (localStorage).
    /// Returns 0 on success, -1 on error.
    fn dirplayer_host_storage_set(
        key_ptr: *const u8,
        key_len: i32,
        val_ptr: *const u8,
        val_len: i32,
    ) -> i32;
}

/// Write a message to the host's debug log.
pub fn log(msg: &str) {
    unsafe {
        dirplayer_host_log(msg.as_ptr(), msg.len() as i32);
    }
}

/// Return `len` cryptographically random bytes.
pub fn random_fill(len: usize) -> Result<Vec<u8>, &'static str> {
    let mut buf = vec![0u8; len];
    let rc = unsafe { dirplayer_host_random_fill(buf.as_mut_ptr(), len as i32) };
    if rc == 0 {
        Ok(buf)
    } else {
        Err("random_fill failed")
    }
}

/// Read a value from localStorage.  Returns None if the key is absent.
pub fn storage_get(key: &str) -> Option<String> {
    // Pre-allocate a generous buffer; grow and retry if the value is larger.
    let mut buf = vec![0u8; 4096];
    let actual = unsafe {
        dirplayer_host_storage_get(
            key.as_ptr(),
            key.len() as i32,
            buf.as_mut_ptr(),
            buf.len() as i32,
        )
    };
    if actual < 0 {
        return None;
    }
    buf.truncate(actual as usize);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Write a key/value pair to localStorage.
pub fn storage_set(key: &str, val: &str) -> Result<(), &'static str> {
    let rc = unsafe {
        dirplayer_host_storage_set(
            key.as_ptr(),
            key.len() as i32,
            val.as_ptr(),
            val.len() as i32,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err("storage_set failed")
    }
}
