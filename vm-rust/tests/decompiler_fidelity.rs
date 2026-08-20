//! Decompiler fidelity checks against the live fuse_client cast.
//!
//! These pin down places where our Lingo emission diverged from what Director
//! actually compiled, found by diffing an exported castpack against
//! ProjectorRays' decompilation of the same cast. Each test names the defect it
//! guards so a regression points straight at the emitter.
//!
//! `projectorrays_reference_matches` additionally diffs every script against a
//! ProjectorRays output directory when `PROJECTORRAYS_REFERENCE_DIR` points at
//! one; without that env var it is skipped, so the suite stays self-contained.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fxhash::FxHashMap;
use vm_rust::{
    director::{
        enums::MemberType, file::read_director_file_bytes, lingo::constants::opcode_names,
    },
    player::{
        bitmap::manager::BitmapManager,
        cast_lib::{CastLib, CastLibState},
        cast_member::{CastMember, CastMemberType},
        export::decompile_script,
    },
};

fn load_fuse_client_cast() -> CastLib {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");
    load_cast(&workspace_root.join("public/dcr_woodpecker/fuse_client.cct"))
}

/// Load a cast file, taking its first cast.
fn load_cast(cast_path: &Path) -> CastLib {
    load_named_cast(cast_path, None)
}

/// Load one cast out of a movie or cast file. `wanted` names the cast as
/// ProjectorRays does — "Internal" for the file's own cast, otherwise the cast
/// library's name. `None` takes the first.
fn load_named_cast(cast_path: &Path, wanted: Option<&str>) -> CastLib {
    vm_rust::player::symbols::symbol_table::init_symbol_table();

    let file_name = cast_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("cast.cct")
        .to_string();
    let bytes = std::fs::read(&cast_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {}", cast_path.display(), error));
    let base_path = format!(
        "file://{}",
        cast_path.parent().expect("cast parent").to_string_lossy()
    );
    let dir = read_director_file_bytes(&bytes, &file_name, &base_path)
        .unwrap_or_else(|error| panic!("failed to parse {file_name}: {error}"));
    let cast_def = match wanted {
        Some(want) => dir
            .casts
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(want))
            // A movie's own cast is often unnamed; ProjectorRays calls it "Internal".
            .or_else(|| {
                if want.eq_ignore_ascii_case("Internal") {
                    dir.casts.iter().find(|c| c.name.is_empty())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("no cast named {want} in {}", cast_path.display())),
        None => dir.casts.first().expect("cast definition"),
    };

    let mut cast = CastLib {
        name: file_name.trim_end_matches(".cct").to_string(),
        file_name: cast_path.to_string_lossy().to_string(),
        number: 1,
        is_external: false,
        state: CastLibState::Loaded,
        lctx: cast_def.lctx.clone(),
        members: FxHashMap::default(),
        scripts: FxHashMap::default(),
        preload_mode: 0,
        capital_x: cast_def.capital_x,
        dir_version: cast_def.dir_version,
        palette_id_offset: cast_def.palette_id_offset,
        name_symbols: Vec::new(),
        name_index: std::cell::RefCell::new(None),
        font_table: HashMap::new(),
    };

    let mut bitmap_manager = BitmapManager::new();
    let mut member_numbers = cast_def.members.keys().copied().collect::<Vec<_>>();
    member_numbers.sort_unstable();
    for member_number in member_numbers {
        let member_def = cast_def.members.get(&member_number).unwrap();
        // Only script members are built by default: constructing other kinds
        // reaches for browser APIs that abort under a native test. Set
        // LOAD_ALL_MEMBERS to include them (needed to see attached scripts).
        if std::env::var_os("LOAD_ALL_MEMBERS").is_none()
            && !matches!(member_def.chunk.member_type, MemberType::Script)
        {
            continue;
        }
        let member = CastMember::from(
            cast.number,
            member_number,
            member_def,
            &cast.lctx,
            &mut bitmap_manager,
            cast.dir_version,
            cast.palette_id_offset,
            &dir.font_table,
        );
        if std::env::var_os("LOAD_ALL_MEMBERS").is_some()
            || matches!(member.member_type, CastMemberType::Script(_))
        {
            cast.insert_member(member_number, member);
        }
    }
    cast
}

/// Decompiled source for every script member, keyed by member name.
///
/// Serialized: the symbol interner behind `Symbol::from_str` is a `static mut`
/// shared across the parallel test runner, and interning from several threads
/// at once corrupts it. The VM itself is single-threaded, so this is a
/// constraint of the harness rather than of the code under test.
fn decompiled_scripts() -> Vec<(String, String)> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let cast = load_fuse_client_cast();
    let mut numbers = cast.scripts.keys().copied().collect::<Vec<_>>();
    numbers.sort_unstable();
    numbers
        .iter()
        .filter_map(|number| {
            let script = cast.scripts.get(number)?;
            Some((script.name.clone(), decompile_script(script, &cast)))
        })
        .collect()
}

fn source_for(scripts: &[(String, String)], name: &str) -> String {
    scripts
        .iter()
        .find(|(script_name, _)| script_name == name)
        .unwrap_or_else(|| panic!("script {name} not found in fuse_client.cct"))
        .1
        .clone()
}

/// A `case` label's body must survive decompilation. RC4 Class lost all three
/// branches of `case tMode of`, leaving an empty `#old, VOID:` label.
#[test]
fn case_labels_keep_their_bodies() {
    let scripts = decompiled_scripts();
    let rc4 = source_for(&scripts, "RC4 Class");

    assert!(
        rc4.contains("#artificialKey:") && rc4.contains("#new:"),
        "RC4 Class lost case labels; got:\n{rc4}"
    );
    assert!(
        rc4.contains("pKey[i + 1] = charToNum("),
        "RC4 Class lost the body of its first case label; got:\n{rc4}"
    );

    // A case label must never be immediately followed by `end case`.
    for (name, source) in &scripts {
        let lines: Vec<&str> = source.lines().map(|line| line.trim()).collect();
        for pair in lines.windows(2) {
            let is_label = pair[0].ends_with(':') && !pair[0].starts_with("--");
            assert!(
                !(is_label && pair[1] == "end case"),
                "{name}: empty case label {:?} directly before `end case`",
                pair[0]
            );
        }
    }
}

/// Properties have no compiled default values — Director initializes them all
/// to VOID. Pairing them positionally with the literal pool invented
/// initializers like `property pItemList = "Object already exists:"`.
#[test]
fn properties_have_no_invented_initializers() {
    for (name, source) in decompiled_scripts() {
        for line in source.lines() {
            let line = line.trim();
            if line.starts_with("property ") {
                assert!(
                    !line.contains('='),
                    "{name}: property declaration gained an initializer: {line}"
                );
            }
        }
    }
}

/// Lingo string literals have no backslash escapes: special characters are
/// their own constants. `"\r"` in source is a literal backslash followed by
/// `r`, not a carriage return.
/// Note: a literal `"\r"` in the output can be genuine — `Text Manager Class`
/// really does key a proplist on the two characters `\` and `r`. What must
/// never happen is a control character being *rendered* as an escape.
#[test]
fn special_characters_use_lingo_constants() {
    let scripts = decompiled_scripts();

    for (name, source) in &scripts {
        // No raw control characters may survive into a string literal either.
        assert!(
            !source.contains('\r') && !source.chars().any(|c| c == '\t'),
            "{name}: raw control character left inside the emitted source"
        );
    }

    let object_base = source_for(&scripts, "Object Base Class");
    assert!(
        object_base.contains("& RETURN &"),
        "Object Base Class should concatenate RETURN; got:\n{object_base}"
    );

    let broker = source_for(&scripts, "Broker Manager Class");
    assert!(
        broker.contains("TAB &"),
        "Broker Manager Class should concatenate TAB; got:\n{}",
        broker
            .lines()
            .filter(|line| line.contains("put "))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// A chunk expression whose `last` index is the literal 0 is a single-chunk
/// reference: `char e of tdata`, not `char e to 0 of tdata`.
#[test]
fn single_chunk_expressions_omit_the_range() {
    for (name, source) in decompiled_scripts() {
        assert!(
            !source.contains(" to 0 of "),
            "{name}: single chunk reference rendered as a range:\n{}",
            source
                .lines()
                .filter(|line| line.contains(" to 0 of "))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// The movie property table is 0-based; an off-by-one turned `the long time`
/// into `the abbr time`.
#[test]
fn movie_property_names_are_not_shifted() {
    let scripts = decompiled_scripts();
    let connection = source_for(&scripts, "Connection Instance Class");
    assert!(
        connection.contains("the long time"),
        "Connection Instance Class should use `the long time`; got:\n{}",
        connection
            .lines()
            .filter(|line| line.contains("time"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// VOID is a constant, not a call.
#[test]
fn void_is_a_constant() {
    for (name, source) in decompiled_scripts() {
        assert!(!source.contains("void()"), "{name}: emitted void() for VOID");
    }
}

fn reference_dir() -> Option<PathBuf> {
    std::env::var_os("PROJECTORRAYS_REFERENCE_DIR").map(PathBuf::from)
}

/// Full diff against a ProjectorRays export, when one is available locally.
/// Set PROJECTORRAYS_REFERENCE_DIR to the directory holding its `.ls` files.
#[test]
fn projectorrays_reference_matches() {
    let Some(dir) = reference_dir() else {
        eprintln!("PROJECTORRAYS_REFERENCE_DIR not set; skipping reference diff");
        return;
    };

    let normalize = |text: &str| {
        text.replace("\r\n", "\n")
            .lines()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string()
    };

    let mut references: HashMap<String, PathBuf> = HashMap::new();
    for entry in std::fs::read_dir(&dir).expect("reference dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ls") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        // "MovieScript 10 - CastLoad  API" -> "CastLoad  API"
        let name = stem.split_once(" - ").map(|(_, n)| n).unwrap_or(&stem);
        references.insert(name.to_string(), path.clone());
    }

    let mut mismatches = Vec::new();
    for (name, source) in decompiled_scripts() {
        let Some(path) = references.get(&name) else {
            continue;
        };
        // ProjectorRays writes the cast's original 8-bit text, not UTF-8.
        let raw = std::fs::read(path).expect("reference file");
        let expected = normalize(&String::from_utf8_lossy(&raw));
        let actual = normalize(&source);
        if expected != actual {
            let first_diff = expected
                .lines()
                .zip(actual.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| format!("  line {}:\n    want: {a}\n    got:  {b}", i + 1))
                .unwrap_or_else(|| "  (length differs)".to_string());
            mismatches.push(format!("{name}:\n{first_diff}"));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} script(s) differ from ProjectorRays:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

/// Debug aid: `cargo test --test decompiler_fidelity dump_script -- --ignored --nocapture`
/// with DUMP_SCRIPT set to a script name.
#[test]
#[ignore]
fn dump_script() {
    let name = std::env::var("DUMP_SCRIPT").unwrap_or_else(|_| "RC4 Class".to_string());
    let scripts = decompiled_scripts();
    println!("{}", source_for(&scripts, &name));
}

/// Debug aid: writes every decompiled script to DUMP_DIR for external diffing.
/// `cargo test --test decompiler_fidelity dump_all -- --ignored`
#[test]
#[ignore]
fn dump_all() {
    let dir = PathBuf::from(std::env::var("DUMP_DIR").expect("DUMP_DIR"));
    std::fs::create_dir_all(&dir).expect("create dump dir");
    for (name, source) in decompiled_scripts() {
        std::fs::write(dir.join(format!("{name}.ls")), source).expect("write script");
    }
}

/// Director pushes a magnitude and negates it rather than pushing a negative
/// constant, so `-20` must compile to `pushint8 20; inv`. The parser folds the
/// sign into the literal, which makes this the compiler's job.
#[test]
fn negative_literals_compile_to_push_and_inv() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let result = vm_rust::player::lingo_compiler::compile_lingo("on t\n  x = -20\nend\n", 1)
        .expect("compile");
    let handler = result.chunk.handlers.first().expect("one handler");
    let ops: Vec<String> = handler
        .bytecode_array
        .iter()
        .map(|bc| format!("{:?}", bc.opcode))
        .collect();
    assert!(
        ops.iter().any(|op| op == "Inv"),
        "expected an Inv opcode for the negative literal, got {ops:?}"
    );
}

#[test]
fn the_number_of_castlibs_compiles_to_legacy_property() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let result =
        vm_rust::player::lingo_compiler::compile_lingo("on t\n  x = the number of castLibs\nend\n", 1)
            .expect("compile");
    let handler = result.chunk.handlers.first().expect("handler");
    let ops: Vec<String> = handler
        .bytecode_array
        .iter()
        .map(|bc| format!("{:?}:{}", bc.opcode, bc.obj))
        .collect();
    assert!(
        ops.iter().any(|op| op.starts_with("Get:")),
        "expected legacy Get opcode, got {ops:?}"
    );
}

#[test]
fn count_of_castlib_compiles_to_legacy_property() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for source in [
        "on t\n  x = the number of castMembers of castLib tC\nend\n",
        "on t\n  x = the number of castMembers of castLib(tC)\nend\n",
    ] {
        let result = vm_rust::player::lingo_compiler::compile_lingo(source, 1)
            .unwrap_or_else(|e| panic!("compile failed for {source:?}: {e}"));
        let handler = result.chunk.handlers.first().expect("handler");
        let ops: Vec<String> = handler
            .bytecode_array
            .iter()
            .map(|bc| format!("{:?}:{}", bc.opcode, bc.obj))
            .collect();
        assert!(
            ops.iter().any(|op| op.starts_with("Get:")),
            "expected legacy Get for {source:?}, got {ops:?}"
        );
    }
}

#[test]
#[ignore]
fn probe_castlib_count() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for src in [
        "on t me, tCastNum
  tMemberCount = the number of castMembers of castLib(tCastNum)
end
",
        "on t me
  tMemberAmount = the number of castMembers of castLib(pBin)
end
",
    ] {
        let r = vm_rust::player::lingo_compiler::compile_lingo(src, 1).expect("compile");
        let h = r.chunk.handlers.first().unwrap();
        let ops: Vec<String> = h.bytecode_array.iter().map(|b| format!("{:?}", b.opcode)).collect();
        println!("{src:?}
  -> {ops:?}");
    }
}

fn describe(d: &vm_rust::director::lingo::datum::Datum) -> String {
    use vm_rust::director::lingo::datum::Datum;
    match d {
        Datum::String(s) => format!("str {s:?}"),
        Datum::Int(i) => format!("int {i}"),
        Datum::Float(f) => format!("float {f}"),
        _ => "other".to_string(),
    }
}

#[test]
#[ignore]
fn probe_literal_pool() {
    let name = std::env::var("PROBE_SCRIPT").unwrap_or_else(|_| "Multiuser Instance Class".to_string());
    let scripts = decompiled_scripts();
    let src = source_for(&scripts, &name);
    let cast = load_fuse_client_cast();
    let original = cast
        .scripts
        .values()
        .find(|s| s.name == name)
        .expect("script");
    let recompiled =
        vm_rust::player::lingo_compiler::compile_lingo(&src, 1).expect("compile");
    println!("--- original literals ({}) ---", original.chunk.literals.len());
    for (i, l) in original.chunk.literals.iter().enumerate().take(30) {
        println!("{i:3}: {}", describe(l));
    }
    println!("--- recompiled literals ({}) ---", recompiled.chunk.literals.len());
    for (i, l) in recompiled.chunk.literals.iter().enumerate().take(30) {
        println!("{i:3}: {}", describe(l));
    }
}

/// `rgb()` with computed components. The literal-digit form was the only
/// three-argument rule, so a call like `rgb(tColor[1], tColor[2], tColor[3])`
/// failed to parse at the first comma.
#[test]
fn rgb_accepts_computed_components() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for source in [
        "on t me, tColor
  x = rgb(tColor[1], tColor[2], tColor[3])
end
",
        "on t me, a, b, c
  x = rgb(a + 1, b * 2, c)
end
",
        "on t
  x = rgb(255, 255, 255)
end
",
    ] {
        vm_rust::player::lingo_compiler::compile_lingo(source, 1)
            .unwrap_or_else(|e| panic!("failed to compile {source:?}: {e}"));
    }
}

#[test]
#[ignore]
fn probe_last_chunk() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for src in [
        "on t me, tHex
  tLc = the last char of tHex
end
",
        "on t me, tHex
  tLc = the last char in tHex
end
",
        "on t me, tHex
  tLc = last char of tHex
end
",
    ] {
        match vm_rust::player::lingo_compiler::compile_lingo(src, 1) {
            Ok(_) => println!("OK   {src:?}"),
            Err(e) => println!("FAIL {src:?}
     {}", e.lines().next().unwrap_or("")),
        }
    }
}

/// A `global` declared at script level applies to every handler. Previously the
/// top-level parser skipped the line, so such names compiled as locals — a
/// silent behaviour change for any script written the way Director and
/// ProjectorRays write them.
#[test]
fn script_level_globals_apply_to_handlers() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let hoisted = "global gCore\n\non t\n  gCore = 1\nend\n";
    let per_handler = "on t\n  global gCore\n  gCore = 1\nend\n";

    let ops = |source: &str| {
        let r = vm_rust::player::lingo_compiler::compile_lingo(source, 1).expect("compile");
        r.chunk.handlers[0]
            .bytecode_array
            .iter()
            .map(|b| format!("{:?}", b.opcode))
            .collect::<Vec<_>>()
    };

    let hoisted_ops = ops(hoisted);
    assert_eq!(
        hoisted_ops,
        ops(per_handler),
        "hoisted and per-handler globals must compile identically"
    );
    assert!(
        hoisted_ops.iter().any(|op| op.contains("SetGlobal")),
        "expected a global write, got {hoisted_ops:?}"
    );
}

/// Dump any cast's decompiled scripts: CAST_PATH + DUMP_DIR.
#[test]
#[ignore]
fn dump_cast() {
    let cast_path = PathBuf::from(std::env::var("CAST_PATH").expect("CAST_PATH"));
    let dir = PathBuf::from(std::env::var("DUMP_DIR").expect("DUMP_DIR"));
    std::fs::create_dir_all(&dir).expect("create dump dir");

    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let cast = load_cast(&cast_path);
    let mut numbers = cast.scripts.keys().copied().collect::<Vec<_>>();
    numbers.sort_unstable();
    for number in numbers {
        if let Some(script) = cast.scripts.get(&number) {
            let source = decompile_script(script, &cast);
            let safe = vm_rust::player::export::sanitize_filename(&script.name);
            std::fs::write(dir.join(format!("{safe}.ls")), source).expect("write");
        }
    }
    println!("wrote {} scripts", cast.scripts.len());
}

/// Batch-decompile a corpus of casts for comparison against ProjectorRays.
/// CORPUS_ROOT is walked for `<name>/casts/External` directories; the matching
/// `<name>.cct`/`.cst` beside them is decompiled into DUMP_DIR/<name-hash>/.
/// A `_index.txt` maps each output directory to its source cast.

/// Normalised disassembly of every handler in a script, for comparison against
/// ProjectorRays' `.lasm` dumps.
///
/// This checks a different layer than the `.ls` comparison: `.ls` only tells us
/// whether two decompilers *render* alike, and two readers can agree on source
/// while disagreeing on what the bytecode says. Instruction offsets, opcodes and
/// operands are what we actually read out of the chunk, so matching them is
/// evidence about the reader rather than the writer.
///
/// Only `offset / mnemonic / operand` is emitted. The text ProjectorRays prints
/// after the dot leader is its own decompiler's commentary, so it would just
/// re-test rendering under another name.
fn disassemble_script(script: &vm_rust::player::script::Script, cast: &CastLib) -> String {
    let names = opcode_names();
    let lctx = match cast.lctx.as_ref() {
        Some(l) => l,
        None => return String::new(),
    };
    let mut out = String::new();
    for name in script.handler_names.iter() {
        let Some(handler) = script.get_own_handler(*name) else { continue };
        let handler_name = lctx
            .names
            .get(handler.name_id as usize)
            .cloned()
            .unwrap_or_else(|| format!("(name {})", handler.name_id));
        out.push_str(&format!("handler {handler_name}
"));
        for bc in handler.bytecode_array.iter() {
            let mnemonic = names
                .get(&bc.opcode)
                .map(|m| m.to_string())
                .unwrap_or_else(|| format!("unk{:02X}", bc.opcode as u16));
            // `obj` carries raw f32 bits for pushfloat32; print the value so it
            // lines up with what ProjectorRays writes.
            let operand = if bc.opcode == vm_rust::director::lingo::opcode::OpCode::PushFloat32 {
                let f = f32::from_bits(bc.obj as u32);
                let mut t = format!("{}", f);
                if !t.contains('.') && !t.contains('e') && !t.contains("inf") && !t.contains("NaN") {
                    t.push_str(".0");
                }
                t
            } else {
                bc.obj.to_string()
            };
            out.push_str(&format!("{}	{}	{}
", bc.pos, mnemonic, operand));
        }
    }
    out
}

#[test]
#[ignore]
fn dump_corpus() {
    let root = PathBuf::from(std::env::var("CORPUS_ROOT").expect("CORPUS_ROOT"));
    let out = PathBuf::from(std::env::var("DUMP_DIR").expect("DUMP_DIR"));
    std::fs::create_dir_all(&out).expect("create dump dir");

    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut targets: Vec<(PathBuf, PathBuf)> = Vec::new();
    collect_targets(&root, &mut targets);
    targets.sort();
    eprintln!("found {} cast/reference pairs", targets.len());

    // Written incrementally: a malformed movie can take the process down with a
    // hard fault that `catch_unwind` cannot see, so progress must survive it.
    // `_progress.txt` names the cast currently being read, and CORPUS_START
    // resumes past it.
    let start: usize = std::env::var("CORPUS_START")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let progress_path = out.join("_progress.txt");
    let mut index = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("_index.txt"))
        .expect("open index");
    let mut ok = 0usize;
    let mut failed: Vec<String> = Vec::new();

    // Files known to take the process down are skipped wholesale: one bad movie
    // usually kills every cast inside it.
    let skip: Vec<String> = std::env::var("CORPUS_SKIP_FILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| {
            t.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();

    for (i, (cast_path, reference_dir)) in targets.iter().enumerate().skip(start) {
        if skip.iter().any(|s| *s == cast_path.display().to_string()) {
            continue;
        }
        let _ = std::fs::write(&progress_path, format!("{i}\t{}", cast_path.display()));
        let slot = out.join(format!("c{i:04}"));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let wanted = reference_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Internal");
            let cast = load_named_cast(cast_path, Some(wanted));
            std::fs::create_dir_all(&slot).expect("slot dir");
            let mut numbers = cast.scripts.keys().copied().collect::<Vec<_>>();
            numbers.sort_unstable();
            for number in numbers {
                if let Some(script) = cast.scripts.get(&number) {
                    let source = decompile_script(script, &cast);
                    // Keyed by member number: ProjectorRays names unnamed scripts
                    // after their type and slot, so only the slot matches reliably.
                    let _ = std::fs::write(slot.join(format!("{number}.ls")), source);
                    let _ = std::fs::write(
                        slot.join(format!("{number}.lasm")),
                        disassemble_script(script, &cast),
                    );
                }
            }
        }));
        match result {
            Ok(()) => {
                ok += 1;
                use std::io::Write;
                let _ = writeln!(
                    index,
                    "c{i:04}\t{}\t{}",
                    cast_path.display(),
                    reference_dir.display()
                );
                let _ = index.flush();
            }
            Err(_) => failed.push(cast_path.display().to_string()),
        }
    }

    let _ = std::fs::write(&progress_path, format!("{}\tDONE", targets.len()));
    {
        use std::io::Write;
        let mut failures = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out.join("_failed.txt"))
            .expect("open failures");
        for f in &failed {
            let _ = writeln!(failures, "{f}");
        }
    }
    eprintln!("decompiled {ok} casts, {} panicked", failed.len());
}

fn collect_targets(dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // `<movie>/casts/<CastName>/` sits beside `<movie>.<ext>`; each subdirectory
        // is one cast inside that file, "Internal" being the file's own.
        if path.file_name().and_then(|n| n.to_str()) == Some("casts") {
            if let (Some(parent), Some(name)) = (
                path.parent().and_then(|p| p.parent()),
                path.parent().and_then(|p| p.file_name()),
            ) {
                let mut source: Option<PathBuf> = None;
                for ext in ["dcr", "cct", "dir", "cst", "dxr", "cxt"] {
                    let candidate = parent.join(format!("{}.{}", name.to_string_lossy(), ext));
                    if candidate.is_file() {
                        source = Some(candidate);
                        break;
                    }
                }
                if let Some(source) = source {
                    if let Ok(subs) = std::fs::read_dir(&path) {
                        for sub in subs.flatten() {
                            let sub_path = sub.path();
                            if sub_path.is_dir() {
                                out.push((source.clone(), sub_path));
                            }
                        }
                    }
                }
            }
            continue;
        }
        collect_targets(&path, out);
    }
}

/// `contains0` is the `starts` operator. Rendering it as `contains` changes the
/// meaning of the test — `"abc" starts "b"` is false, `contains` is true.
#[test]
fn starts_operator_round_trips() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let result =
        vm_rust::player::lingo_compiler::compile_lingo("on t me, s\n  x = s starts \"http\"\nend\n", 1)
            .expect("compile `starts`");
    let ops: Vec<String> = result.chunk.handlers[0]
        .bytecode_array
        .iter()
        .map(|b| format!("{:?}", b.opcode))
        .collect();
    assert!(
        ops.iter().any(|op| op == "Contains0Str"),
        "expected Contains0Str for `starts`, got {ops:?}"
    );
}

/// Reproduce a single cast load: CAST_PATH (+ optional CAST_NAME).
#[test]
#[ignore]
fn probe_one_cast() {
    let p = PathBuf::from(std::env::var("CAST_PATH").expect("CAST_PATH"));
    let want = std::env::var("CAST_NAME").ok();
    eprintln!("loading {} (cast {:?})", p.display(), want);
    let cast = load_named_cast(&p, want.as_deref());
    eprintln!("loaded: {} script members", cast.scripts.len());
}

/// Narrow where a crashing movie dies: parse only, no member construction.
#[test]
#[ignore]
fn probe_parse_only() {
    let p = PathBuf::from(std::env::var("CAST_PATH").expect("CAST_PATH"));
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let bytes = std::fs::read(&p).expect("read");
    eprintln!("read {} bytes", bytes.len());
    let name = p.file_name().unwrap().to_string_lossy().to_string();
    let base = format!("file://{}", p.parent().unwrap().to_string_lossy());
    match read_director_file_bytes(&bytes, &name, &base) {
        Ok(dir) => {
            eprintln!("parsed OK: {} casts", dir.casts.len());
            for c in &dir.casts {
                eprintln!("  cast {:?} id={} members={}", c.name, c.id, c.members.len());
            }
        }
        Err(e) => eprintln!("parse error: {e}"),
    }
}

#[test]
#[ignore]
fn probe_new_syntax() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for src in [
        "on t
  x = new xtra(\"fileio\")
end
",
        "on t
  x = new(xtra, \"fileio\")
end
",
        "on t
  x = new script(\"foo\")
end
",
    ] {
        match vm_rust::player::lingo_compiler::compile_lingo(src, 1) {
            Ok(_) => println!("OK   {src:?}"),
            Err(e) => println!("FAIL {src:?} :: {}", e.lines().next().unwrap_or("")),
        }
    }
}

/// `new xtra("fileio")` is Director's constructor syntax and compiles to the
/// dedicated `newobj` opcode. Writing it as `new(xtra, "fileio")` reads as a
/// call to `new` with the type as an argument, and compiled to the wrong thing.
#[test]
fn new_object_uses_newobj_opcode() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let result =
        vm_rust::player::lingo_compiler::compile_lingo("on t\n  x = new xtra(\"fileio\")\nend\n", 1)
            .expect("compile `new xtra(...)`");
    let ops: Vec<String> = result.chunk.handlers[0]
        .bytecode_array
        .iter()
        .map(|b| format!("{:?}", b.opcode))
        .collect();
    assert!(
        ops.iter().any(|op| op == "NewObj"),
        "expected a NewObj opcode, got {ops:?}"
    );
}

#[test]
#[ignore]
fn probe_verbose_syntax() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for src in [
        "on t
  set verspr to 46
end
",
        "on t me
  set the loc of sprite 1 to point(0, 0)
end
",
        "on t me
  x = the bDebug of me
end
",
        "on t me
  set the bDebug of me to 1
end
",
        "on t
  put EMPTY into field \"x\"
end
",
        "on t me
  x = the number of castMembers of castLib 2
end
",
        "on t me
  set the itemDelimiter to \",\"
end
",
    ] {
        match vm_rust::player::lingo_compiler::compile_lingo(src, 1) {
            Ok(_) => println!("OK   {}", src.lines().nth(1).unwrap_or("").trim()),
            Err(e) => println!("FAIL {} :: {}", src.lines().nth(1).unwrap_or("").trim(), e.lines().next().unwrap_or("")),
        }
    }
}

/// A float literal must survive recompilation exactly. `pushfloat32` carries
/// the value inline as a 32-bit float, so it is only usable when that is
/// lossless; everything else belongs in the literal pool as a double.
#[test]
fn float_literals_keep_their_precision() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for (source, value) in [
        ("on t\n  x = 0.8\nend\n", 0.8f64),
        ("on t\n  x = 0.05\nend\n", 0.05f64),
        ("on t\n  x = 0.98999999999999999\nend\n", 0.98999999999999999f64),
        ("on t\n  x = 8.5\nend\n", 8.5f64),
        ("on t\n  x = 1.0\nend\n", 1.0f64),
    ] {
        let result = vm_rust::player::lingo_compiler::compile_lingo(source, 1).expect("compile");
        let handler = &result.chunk.handlers[0];
        let mut seen: Option<f64> = None;
        for bc in &handler.bytecode_array {
            match bc.opcode {
                vm_rust::director::lingo::opcode::OpCode::PushFloat32 => {
                    seen = Some(f32::from_bits(bc.obj as u32) as f64);
                }
                vm_rust::director::lingo::opcode::OpCode::PushCons => {
                    if let Some(vm_rust::director::lingo::datum::Datum::Float(f)) =
                        result.chunk.literals.get(bc.obj as usize)
                    {
                        seen = Some(*f);
                    }
                }
                _ => {}
            }
        }
        let seen = seen.unwrap_or_else(|| panic!("no float pushed for {source:?}"));
        assert_eq!(
            seen.to_bits(),
            value.to_bits(),
            "{source:?} recompiled to {seen} instead of {value}"
        );
    }
}

#[test]
#[ignore]
fn probe_verbose_batch() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for src in [
        "on t
  sound stop 1
end
",
        "on t
  sound fadeIn 4, 60
end
",
        "on t
  play frame \"win\"
end
",
        "on t
  play done
end
",
        "on t me, n
  set the locV of sprite (n + 1) to 0
end
",
        "on t
  set the member of sprite 7 to member \"playGame\"
end
",
        "on t
  cursor [cast \"hand\", cast \"handMatte\"]
end
",
        "on t me, a, b
  set oCar to new(script \"sp: car\", a, b)
end
",
    ] {
        match vm_rust::player::lingo_compiler::compile_lingo(src, 1) {
            Ok(_) => println!("OK   {}", src.lines().nth(1).unwrap_or("").trim()),
            Err(e) => println!("FAIL {} :: {}", src.lines().nth(1).unwrap_or("").trim(), e.lines().next().unwrap_or("")),
        }
    }
}

#[test]
#[ignore]
fn probe_chunk_verbose() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    for src in [
        "on t
  nl = the number of lines in the text of member \"help text\"
end
",
        "on t me
  myname = word 1 of the name of the member of sprite (the spriteNum of me)
end
",
        "on t me, s
  x = char 1 of the text of member \"a\"
end
",
        "on t me, s
  x = item 1 of line 2 of s
end
",
    ] {
        match vm_rust::player::lingo_compiler::compile_lingo(src, 1) {
            Ok(_) => println!("OK   {}", src.lines().nth(1).unwrap_or("").trim()),
            Err(e) => println!("FAIL {} :: {}", src.lines().nth(1).unwrap_or("").trim(), e.lines().next().unwrap_or("")),
        }
    }
}

/// Which member types actually carry an attached script? Reads the raw chunks,
/// so it does not need to construct members.
#[test]
#[ignore]
fn probe_attached_script_types() {
    let root = PathBuf::from(std::env::var("CORPUS_ROOT").expect("CORPUS_ROOT"));
    let mut targets: Vec<(PathBuf, PathBuf)> = Vec::new();
    collect_targets(&root, &mut targets);
    targets.sort();
    targets.dedup_by(|a, b| a.0 == b.0);

    let skip: Vec<String> = std::env::var("CORPUS_SKIP_FILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default();

    let mut tally: HashMap<String, usize> = HashMap::new();
    for (cast_path, _) in targets.iter().take(1500) {
        if skip.iter().any(|s| *s == cast_path.display().to_string()) {
            continue;
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bytes = std::fs::read(cast_path).ok()?;
            let name = cast_path.file_name()?.to_str()?.to_string();
            let base = format!("file://{}", cast_path.parent()?.to_string_lossy());
            let dir = read_director_file_bytes(&bytes, &name, &base).ok()?;
            let mut local: Vec<String> = Vec::new();
            for cast in &dir.casts {
                for member in cast.members.values() {
                    let sid = member.chunk.member_info.as_ref().map(|i| i.header.script_id).unwrap_or(0);
                    if sid > 0 {
                        local.push(format!("{:?}", member.chunk.member_type));
                    }
                }
            }
            Some(local)
        }));
        if let Ok(Some(list)) = r {
            for t in list {
                *tally.entry(t).or_insert(0) += 1;
            }
        }
    }
    let mut rows: Vec<_> = tally.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("--- members carrying an attached script ---");
    for (t, n) in rows {
        println!("{n:7}  {t}");
    }
}

/// A member script attached to a non-script member must survive export: the
/// member's yml gains a `member_script:` section and the script is written
/// beside it. Previously the behaviour was dropped silently.
#[test]
fn attached_member_scripts_are_exported() {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let Some(path) = std::env::var_os("ATTACHED_SCRIPT_CAST") else {
        eprintln!("ATTACHED_SCRIPT_CAST not set; skipping");
        return;
    };
    let cast = load_named_cast(&PathBuf::from(path), std::env::var("CAST_NAME").ok().as_deref());

    // Every registered script whose member is not itself a script member is an
    // attached one, and must be reachable for export.
    let mut attached = 0;
    for (number, script) in cast.scripts.iter() {
        let Some(member) = cast.members.get(number) else { continue };
        if !matches!(member.member_type, CastMemberType::Script(_)) {
            attached += 1;
            assert!(
                !decompile_script(script, &cast).trim().is_empty(),
                "attached script on member {number} decompiled to nothing"
            );
        }
    }
    eprintln!("attached member scripts found: {attached}");
}

#[test]
#[ignore]
fn probe_find_attached_cast() {
    let root = PathBuf::from(std::env::var("CORPUS_ROOT").expect("CORPUS_ROOT"));
    let mut targets: Vec<(PathBuf, PathBuf)> = Vec::new();
    collect_targets(&root, &mut targets);
    targets.sort();
    targets.dedup_by(|a, b| a.0 == b.0);
    let skip: Vec<String> = std::env::var("CORPUS_SKIP_FILE")
        .ok().and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().map(|l| l.trim().to_string()).collect()).unwrap_or_default();
    for (cast_path, ref_dir) in targets.iter() {
        if skip.iter().any(|s| *s == cast_path.display().to_string()) { continue; }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bytes = std::fs::read(cast_path).ok()?;
            let name = cast_path.file_name()?.to_str()?.to_string();
            let base = format!("file://{}", cast_path.parent()?.to_string_lossy());
            let dir = read_director_file_bytes(&bytes, &name, &base).ok()?;
            for cast in &dir.casts {
                let n = cast.members.values().filter(|m| {
                    let sid = m.chunk.member_info.as_ref().map(|i| i.header.script_id).unwrap_or(0);
                    sid > 0 && !matches!(m.chunk.member_type, MemberType::Script)
                }).count();
                if n > 0 { return Some((cast.name.clone(), n)); }
            }
            None
        }));
        if let Ok(Some((cname, n))) = r {
            println!("FOUND {} | cast={:?} attached={}", cast_path.display(), cname, n);
            println!("REF {}", ref_dir.display());
            return;
        }
    }
}

/// End-to-end: the exporter writes a `.ls` for an attached member script and
/// records it in the member's yml.
#[test]
#[ignore]
fn probe_export_attached() {
    let path = PathBuf::from(std::env::var("ATTACHED_SCRIPT_CAST").expect("ATTACHED_SCRIPT_CAST"));
    let cast = load_named_cast(&path, std::env::var("CAST_NAME").ok().as_deref());
    for (number, script) in cast.scripts.iter() {
        let Some(member) = cast.members.get(number) else { continue };
        if matches!(member.member_type, CastMemberType::Script(_)) { continue; }
        println!("member {} ({}) has attached script, type {:?}",
            number, member.name, script.script_type);
        let src = decompile_script(script, &cast);
        println!("--- first lines ---");
        for l in src.lines().take(4) { println!("  {l}"); }
    }
}

/// Handler argument names are Lingo identifiers. They were being passed through
/// `yaml_string`, which quotes anything YAML reads as a boolean — so a handler
/// taking an argument called `yes` was exported as `on vote player, "yes"`,
/// which does not compile.
#[test]
fn handler_arguments_are_not_quoted() {
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    // Round-trip through the compiler: the name must survive as an identifier.
    let source = "on vote player, yes\n  return yes\nend\n";
    let result = vm_rust::player::lingo_compiler::compile_lingo(source, 1).expect("compile");
    let handler = &result.chunk.handlers[0];
    let names: Vec<String> = handler
        .argument_name_ids
        .iter()
        .filter_map(|id| result.names.get(*id as usize).cloned())
        .collect();
    assert!(
        names.iter().any(|n| n == "yes"),
        "expected an argument named `yes`, got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains('"')),
        "argument names must not be quoted: {names:?}"
    );
}

/// Do JavaScript-syntax scripts carry property/global declarations in the chunk?
#[test]
#[ignore]
fn probe_js_script_decls() {
    let root = PathBuf::from(std::env::var("CORPUS_ROOT").expect("CORPUS_ROOT"));
    let mut targets: Vec<(PathBuf, PathBuf)> = Vec::new();
    collect_targets(&root, &mut targets);
    targets.sort();
    targets.dedup_by(|a, b| a.0 == b.0);
    let mut found = 0;
    for (cast_path, _) in targets.iter() {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bytes = std::fs::read(cast_path).ok()?;
            let name = cast_path.file_name()?.to_str()?.to_string();
            let base = format!("file://{}", cast_path.parent()?.to_string_lossy());
            let dir = read_director_file_bytes(&bytes, &name, &base).ok()?;
            let mut hits = Vec::new();
            for cast in &dir.casts {
                let Some(lctx) = &cast.lctx else { continue };
                for (id, chunk) in lctx.scripts.iter() {
                    let is_js = chunk.literals.iter().any(|l| {
                        matches!(l, vm_rust::director::lingo::datum::Datum::JavaScript(_))
                    });
                    if is_js {
                        hits.push((
                            *id,
                            chunk.property_name_ids.len(),
                            chunk.global_name_ids.len(),
                            chunk.handlers.len(),
                        ));
                    }
                }
            }
            Some(hits)
        }));
        if let Ok(Some(hits)) = r {
            for (id, props, globals, handlers) in hits {
                println!("{} script {id}: properties={props} globals={globals} lingo_handlers={handlers}",
                    cast_path.file_name().unwrap().to_string_lossy());
                found += 1;
                if found >= 12 { return; }
            }
        }
    }
    println!("total JS scripts seen: {found}");
}

/// Inspect one cast's JS scripts and what declarations they carry.
#[test]
#[ignore]
fn probe_js_in_cast() {
    let p = PathBuf::from(std::env::var("CAST_PATH").expect("CAST_PATH"));
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let bytes = std::fs::read(&p).expect("read");
    let name = p.file_name().unwrap().to_str().unwrap().to_string();
    let base = format!("file://{}", p.parent().unwrap().to_string_lossy());
    let dir = read_director_file_bytes(&bytes, &name, &base).expect("parse");
    let mut js = 0;
    for cast in &dir.casts {
        let Some(lctx) = &cast.lctx else { continue };
        for (id, chunk) in lctx.scripts.iter() {
            let is_js = chunk.literals.iter().any(|l| {
                matches!(l, vm_rust::director::lingo::datum::Datum::JavaScript(_))
            });
            if !is_js { continue; }
            js += 1;
            let props: Vec<String> = chunk.property_name_ids.iter()
                .filter_map(|i| lctx.names.get(*i as usize).cloned()).collect();
            let globals: Vec<String> = chunk.global_name_ids.iter()
                .filter_map(|i| lctx.names.get(*i as usize).cloned()).collect();
            println!("script {id}: properties={props:?} globals={globals:?} lingo_handlers={}",
                chunk.handlers.len());
        }
    }
    println!("JS scripts in this cast: {js}");
}

/// Print a cast's name table, to check whether a reference `.lasm`'s operand
/// numbering belongs to the same build of the movie we are reading.
#[test]
#[ignore]
fn probe_name_table() {
    let p = PathBuf::from(std::env::var("CAST_PATH").expect("CAST_PATH"));
    let wanted = std::env::var("CAST_NAME").ok();
    vm_rust::player::symbols::symbol_table::init_symbol_table();
    let cast = load_named_cast(&p, wanted.as_deref());
    let lctx = cast.lctx.as_ref().expect("lctx");
    println!("names: {}", lctx.names.len());
    for i in 0..lctx.names.len().min(60) {
        println!("  [{i}] {}", lctx.names[i]);
    }
}
