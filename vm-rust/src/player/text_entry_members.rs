//! Members the host page declares as text entries.
//!
//! Some movies draw their own text input instead of using an editable field:
//! a plain Text member whose content a movie-level `on keyDown` rewrites. On
//! a desktop that works as is, since every key reaches the movie, but a touch
//! frontend has no way to tell that a tap on such a member should raise the
//! on-screen keyboard. The page that knows the movie names those members
//! here (see `set_text_entry_members`); the VM keeps a general mechanism.

use std::cell::RefCell;

thread_local! {
    static TEXT_ENTRY: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

pub fn set_text_entry(names: Vec<String>) {
    TEXT_ENTRY.with(|b| *b.borrow_mut() = names);
}

/// True when this member was declared a text entry. Name match is
/// case-insensitive, matching Director's own name lookups.
pub fn is_text_entry(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let lower = name.to_lowercase();
    TEXT_ENTRY.with(|b| b.borrow().iter().any(|n| n == &lower))
}
