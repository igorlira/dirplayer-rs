use fxhash::FxHashMap;
use lasso::{Rodeo, Spur};

use crate::player::symbols::builtin::BuiltInSymbol;

pub struct SymbolTable {
    interner: Rodeo,
    original_strings: FxHashMap<Spur, String>,
    /// Spurs whose display spelling was claimed by a MOVIE's own name table.
    /// A builtin spelling may be overridden by the first movie name that
    /// collides with it; a movie's claim is then final, so a movie that spells
    /// the same name two ways keeps the FIRST spelling — which is Director's
    /// first-writer-wins rule. Without this, the last entry in the name table
    /// would win and an internally inconsistent movie would get whichever
    /// casing happened to come later.
    movie_claimed_display: fxhash::FxHashSet<Spur>,
    /// Display spellings as they stood after `init_builtin_symbols()`, before
    /// any movie claimed one. `reset_movie_display_claims` restores this so each
    /// movie starts from the builtin baseline instead of inheriting whatever the
    /// previously-loaded movie claimed — the interner itself is monotonic (spurs
    /// must stay valid forever), so only the DISPLAY layer is reset.
    builtin_display_baseline: FxHashMap<Spur, String>,
    pub spur_to_builtin: FxHashMap<Spur, BuiltInSymbol>,
    pub builtin_to_spur: FxHashMap<BuiltInSymbol, Spur>,
}

pub static mut SYMBOL_TABLE: Option<SymbolTable> = None;

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            interner: Rodeo::default(),
            original_strings: FxHashMap::default(),
            movie_claimed_display: fxhash::FxHashSet::default(),
            builtin_display_baseline: FxHashMap::default(),
            spur_to_builtin: FxHashMap::default(),
            builtin_to_spur: FxHashMap::default(),
        }
    }

    pub fn intern(&mut self, string: &str) -> Spur {
        // Fast path: no allocation when the text is already lowercase, which
        // every `Symbol::from_str("someliteral")` in the codebase is — and there
        // are hundreds, many on hot paths (per-hit prop-list keys in
        // modelsUnderRay, parent-chain walks, per-frame property names).
        // `to_lowercase()` allocates a String unconditionally, so those all paid
        // for a heap allocation to produce an identical string.
        //
        // Restricted to pure ASCII: a non-ASCII byte can still be uppercase in
        // Unicode (`É`), and `to_lowercase()` is Unicode-aware, so only an
        // all-ASCII string with no ASCII uppercase is guaranteed to lowercase to
        // itself.
        if string.is_ascii() && !string.bytes().any(|b| b.is_ascii_uppercase()) {
            if let Some(spur) = self.interner.get(string) {
                return spur;
            }
            let spur = self.interner.get_or_intern(string);
            if !self.original_strings.contains_key(&spur) {
                self.original_strings.insert(spur, string.to_owned());
            }
            return spur;
        }
        let lower_string = string.to_lowercase();
        let spur = self.interner.get_or_intern(&lower_string);
        if !self.original_strings.contains_key(&spur) {
            self.original_strings.insert(spur, string.to_owned());
        }
        spur
    }

    /// Intern and CLAIM the display spelling, overwriting any already recorded.
    ///
    /// Only for names read from a cast's own name table (LNAM). Director keeps
    /// one global display spelling per symbol, claimed by the FIRST writer, and
    /// a string probe matches a symbol key by comparing against that spelling —
    /// measured in the Message window:
    ///
    ///   [#nodeName:"test"]                       -- claims "nodeName"
    ///   [#nodename:"test"].getaProp("nodeName")  -- "test"   (matches display)
    ///   [#nodeName:"test"].getaProp("nodename")  -- <Void>   (misses)
    ///
    /// Note the key's own spelling is irrelevant; the global entry governs.
    ///
    /// In Shockwave the first writer is the movie, because there is no builtin
    /// spelling table seeded ahead of it. We intern ~900 builtins in
    /// `init_builtin_symbols()` at startup, so a movie symbol colliding with one
    /// inherits the BUILTIN's casing: Habbo v31's `#nodename` reported as
    /// "nodeName" (the XML DOM property), and the catalogue's
    /// `tdata.getaProp("nodename")` missed — "Malformed node data nodeName".
    ///
    /// Interning the cast's names authoritatively restores Shockwave's ordering:
    /// the movie's own spelling wins over a builtin it never referenced.
    pub fn intern_authoritative(&mut self, string: &str) -> Spur {
        let lower_string = string.to_lowercase();
        let spur = self.interner.get_or_intern(&lower_string);
        // Override a BUILTIN's spelling, but only once: the first movie name to
        // claim a spur keeps it. A movie that spells the same name two ways
        // (`#nodeName` in one script, `#nodename` in another) therefore keeps the
        // first, matching Director; overwriting unconditionally would hand it to
        // whichever entry sat later in the name table.
        if !self.movie_claimed_display.contains(&spur) {
            self.original_strings.insert(spur, string.to_owned());
            self.movie_claimed_display.insert(spur);
        }
        spur
    }

    /// Record the current display spellings as the builtin baseline. Called
    /// once, right after `init_builtin_symbols()`.
    pub fn snapshot_builtin_display(&mut self) {
        self.builtin_display_baseline = self.original_strings.clone();
    }

    /// Drop every movie display claim and restore the builtin baseline.
    ///
    /// Director resets its symbol table per movie; ours is a process-global,
    /// monotonic interner, so without this the FIRST movie loaded would claim
    /// spellings for every movie after it. That makes behaviour depend on load
    /// order — a real hazard for the e2e suite, which runs ~48 movies in one
    /// process, and for any session that navigates between movies.
    ///
    /// Only display spellings are reset. Spurs stay valid, so `Symbol` values
    /// held across the movie change keep their identity.
    pub fn reset_movie_display_claims(&mut self) {
        self.movie_claimed_display.clear();
        for (spur, spelling) in &self.builtin_display_baseline {
            self.original_strings.insert(*spur, spelling.clone());
        }
    }

    /// The INTERNED (lowercased) spelling. `intern` lowercases before
    /// interning, so this is the case-normalised form and the only safe thing
    /// to `match` string literals against — `get_original_string` returns
    /// whichever casing was seen FIRST, which depends on load order.
    pub fn get_lower_string(&self, spur: &Spur) -> &str {
        self.interner.resolve(spur)
    }

    pub fn get_original_string(&self, spur: &Spur) -> &str {
        self.original_strings.get(spur).expect("Original string not found").as_str()
    }
}

static SYMBOL_TABLE_INIT: std::sync::Once = std::sync::Once::new();

pub fn init_symbol_table() {
    // Idempotent: the interner is global and monotonic — it never needs
    // resetting between players/movies, so guard the one-time setup with a
    // `Once`. This makes the function safe to call repeatedly (e.g. on every
    // `init_player`) and from unit tests that exercise symbol-interning code
    // paths (the Lingo parser interns chunk-type symbols) without standing up
    // a full player. The `Once` also makes init safe under the parallel test
    // runner, which shares the `static mut SYMBOL_TABLE` across threads.
    SYMBOL_TABLE_INIT.call_once(|| {
        unsafe {
            SYMBOL_TABLE = Some(SymbolTable::new());
        }
        crate::player::symbols::builtin::init_builtin_symbols();
        unsafe {
            if let Some(t) = SYMBOL_TABLE.as_mut() {
                t.snapshot_builtin_display();
            }
        }
    });
}

/// Clear per-movie display-spelling claims. Call when a movie loads, so casts
/// start from the builtin baseline rather than the previous movie's claims.
pub fn reset_movie_symbol_display() {
    init_symbol_table();
    unsafe {
        if let Some(t) = SYMBOL_TABLE.as_mut() {
            t.reset_movie_display_claims();
        }
    }
}

pub fn get_symbol_spur(string: &str) -> Spur {
    init_symbol_table();
    unsafe {
        SYMBOL_TABLE
            .as_mut()
            .expect("Symbol table not initialized")
            .intern(string)
    }
}

/// `get_symbol_spur`, but the caller CLAIMS the display spelling. See
/// `SymbolTable::intern_authoritative` — use only for a cast's own name table.
pub fn get_symbol_spur_authoritative(string: &str) -> Spur {
    init_symbol_table();
    unsafe {
        SYMBOL_TABLE
            .as_mut()
            .expect("Symbol table not initialized")
            .intern_authoritative(string)
    }
}

pub fn get_spur_string_owned(spur: Spur) -> String {
    unsafe {
        SYMBOL_TABLE
            .as_ref()
            .expect("Symbol table not initialized")
            .get_original_string(&spur)
            .to_owned()
    }
}

pub fn get_spur_string(spur: Spur) -> &'static str {
    unsafe {
        SYMBOL_TABLE
            .as_ref()
            .expect("Symbol table not initialized")
            .get_original_string(&spur)
    }
}

pub fn spur(string: &str) -> Spur {
    get_symbol_spur(string)
}

pub struct BuiltinKeywords {
    
}
