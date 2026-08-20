use std::path::Path;

use fxhash::FxHashMap;
use vm_rust::{
    director::{
        chunks::handler::{Bytecode, HandlerDef},
        file::{get_variable_multiplier, read_director_file_bytes},
        lingo::{opcode::OpCode, script::ScriptContext},
    },
    player::{
        bitmap::manager::BitmapManager,
        cast_member::{CastMember, CastMemberType},
        cast_lib::{CastLib, CastLibState},
        export::decompile_script,
        lingo_compiler::{compile_lingo, inject_into_lctx},
    },
};

#[derive(Clone)]
struct RoundtripCase {
    cast_number: u32,
    cast_name: String,
    member_number: u32,
    script_name: String,
    script_number: u16,
    lctx: ScriptContext,
    source: String,
    original_handlers: Vec<OriginalHandler>,
}

#[derive(Clone)]
struct OriginalHandler {
    name: String,
    bytecodes: Vec<(String, i64)>,
    rendered: Vec<String>,
}

fn normalize_rendered_text(text: &str) -> String {
    let without_pos = if text.starts_with('[') {
        text.split_once(']')
            .map(|(_, rest)| rest.trim_start())
            .unwrap_or(text)
    } else {
        text
    };
    without_pos.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_bytecodes(bytecodes: &[Bytecode]) -> Vec<(String, i64)> {
    // Remove proven dead code: no-op Jmp instructions whose target is the very next instruction.
    // These are semantically equivalent to no instruction at all and arise from certain
    // Director compiler patterns (e.g., if-without-else ending with a void method call).
    // After removing them, re-compute all affected jump byte offsets.
    let noop_pos: std::collections::HashSet<usize> = bytecodes
        .windows(2)
        .filter(|w| {
            w[0].opcode == OpCode::Jmp
                && (w[0].pos as i64 + w[0].obj) as usize == w[1].pos
        })
        .map(|w| w[0].pos)
        .collect();

    // Compute (removed_instr_start_byte, removed_instr_size) for each no-op jmp.
    let removed: Vec<(usize, usize)> = bytecodes
        .windows(2)
        .filter(|w| noop_pos.contains(&w[0].pos))
        .map(|w| (w[0].pos, w[1].pos - w[0].pos))
        .collect();

    let bytes_before = |pos: usize| -> usize {
        removed.iter().filter(|(s, _)| *s < pos).map(|(_, sz)| sz).sum()
    };

    bytecodes
        .iter()
        .filter(|bc| !noop_pos.contains(&bc.pos))
        .map(|bc| {
            let adj_pos = bc.pos - bytes_before(bc.pos);
            let adj_obj = match bc.opcode {
                OpCode::Jmp | OpCode::JmpIfZ => {
                    let abs = (bc.pos as i64 + bc.obj) as usize;
                    let adj = abs - bytes_before(abs);
                    adj as i64 - adj_pos as i64
                }
                OpCode::EndRepeat => {
                    let abs = bc.pos.saturating_sub(bc.obj as usize);
                    let adj = abs - bytes_before(abs);
                    (adj_pos - adj) as i64
                }
                _ => bc.obj,
            };
            (format!("{:?}", bc.opcode), adj_obj)
        })
        .collect()
}

fn render_bytecodes(
    bytecodes: &[Bytecode],
    lctx: &ScriptContext,
    handler: &HandlerDef,
    multiplier: u32,
) -> Vec<String> {
    bytecodes
        .iter()
        .enumerate()
        .map(|(index, bytecode)| {
            format!(
                "{index:03}: {}",
                normalize_rendered_text(&bytecode.to_bytecode_text(lctx, handler, multiplier))
            )
        })
        .collect()
}

fn compiler_ready_source(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let normalized = if trimmed.starts_with("property ") {
                match trimmed.split_once('=') {
                    Some((before, _)) => before.trim_end().to_string(),
                    None => line.to_string(),
                }
            } else {
                line.to_string()
            };

            normalized
                .replace("the the number of ", "the number of ")
                .replace("the last char of ", "last char of ")
                .replace("the last word of ", "last word of ")
                .replace("the last item of ", "last item of ")
                .replace("the last line of ", "last line of ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_fuse_client_cast() -> CastLib {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");
    let cast_path = workspace_root.join("public/dcr_woodpecker/fuse_client.cct");
    let bytes = std::fs::read(&cast_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {}", cast_path.display(), error));
    let base_path = format!(
        "file://{}",
        cast_path
            .parent()
            .expect("cast parent directory")
            .to_string_lossy()
    );
    let dir = read_director_file_bytes(&bytes, "fuse_client.cct", &base_path)
        .unwrap_or_else(|error| panic!("failed to parse fuse_client.cct: {error}"));
    let cast_def = dir
        .casts
        .first()
        .unwrap_or_else(|| panic!("fuse_client.cct did not contain a cast definition"));

    let mut cast = CastLib {
        name: "fuse_client".to_string(),
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
    };

    let mut bitmap_manager = BitmapManager::new();
    let mut member_numbers = cast_def.members.keys().copied().collect::<Vec<_>>();
    member_numbers.sort_unstable();

    for member_number in member_numbers {
        let member_def = cast_def.members.get(&member_number).unwrap();
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
        if matches!(member.member_type, CastMemberType::Script(_)) {
            cast.insert_member(member_number, member);
        }
    }

    cast
}

fn load_roundtrip_cases() -> Vec<RoundtripCase> {
    let cast = load_fuse_client_cast();
    let lctx = cast.lctx.as_ref().expect("fuse_client cast lctx");
    let multiplier = get_variable_multiplier(cast.capital_x, cast.dir_version);
    let mut script_numbers = cast.scripts.keys().copied().collect::<Vec<_>>();
    script_numbers.sort_unstable();

    let mut cases = Vec::new();
    for member_number in script_numbers {
        let script = cast.scripts.get(&member_number).unwrap();
        let source = compiler_ready_source(&decompile_script(script, &cast));
        let original_handlers = script
            .handler_names
            .iter()
            .filter_map(|name| script.get_own_handler(name).map(|handler| (name, handler)))
            .map(|(name, handler)| OriginalHandler {
                name: name.clone(),
                bytecodes: normalize_bytecodes(&handler.bytecode_array),
                rendered: render_bytecodes(&handler.bytecode_array, lctx, handler, multiplier),
            })
            .collect::<Vec<_>>();

        if !original_handlers.is_empty() {
            cases.push(RoundtripCase {
                cast_number: cast.number,
                cast_name: cast.name.clone(),
                member_number,
                script_name: script.name.clone(),
                script_number: script.chunk.script_number,
                lctx: lctx.clone(),
                source,
                original_handlers,
            });
        }
    }

    cases.sort_by(|left, right| {
        (left.cast_number, left.member_number, left.script_name.as_str())
            .cmp(&(right.cast_number, right.member_number, right.script_name.as_str()))
    });
    cases
}

fn roundtrip_script(case: &RoundtripCase) -> Result<Vec<String>, String> {
    roundtrip_script_handlers(case, None)
}

fn roundtrip_script_handlers(
    case: &RoundtripCase,
    selected_handlers: Option<&[&str]>,
) -> Result<Vec<String>, String> {
    let compiled = compile_lingo(&case.source, case.script_number)?;
    let mut lctx = case.lctx.clone();
    inject_into_lctx(&mut lctx, compiled, case.script_number as u32);
    let compiled_chunk = lctx
        .scripts
        .get(&(case.script_number as u32))
        .ok_or_else(|| format!("compiled script {} missing from lctx", case.script_name))?;

    let mut mismatches = Vec::new();
    let multiplier = 1;

    for original in &case.original_handlers {
        if let Some(selected_handlers) = selected_handlers {
            if !selected_handlers
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&original.name))
            {
                continue;
            }
        }

        let handler_name_id = lctx
            .names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(&original.name))
            .ok_or_else(|| format!("compiled handler '{}' missing from name table", original.name))?
            as u16;

        let compiled_handler = compiled_chunk
            .handlers
            .iter()
            .find(|handler| handler.name_id == handler_name_id)
            .ok_or_else(|| format!("compiled handler '{}' missing from script chunk", original.name))?;

        let compiled_bytecodes = normalize_bytecodes(&compiled_handler.bytecode_array);
        if compiled_bytecodes != original.bytecodes {
            let compiled_rendered = render_bytecodes(
                &compiled_handler.bytecode_array,
                &lctx,
                compiled_handler,
                multiplier,
            )
            .join("\n");
            mismatches.push(format!(
                "{}::{}\nexpected:\n{}\nactual:\n{}",
                case.script_name,
                original.name,
                original.rendered.join("\n"),
                compiled_rendered,
            ));
        }
    }

    Ok(mismatches)
}

#[test]
#[ignore]
fn dump_failing_sources() {
    let cases = load_roundtrip_cases();
    for name in &[] as &[&str] {
        if let Some(case) = cases.iter().find(|c| c.script_name == *name) {
            println!("=== {} ===\n{}\n", name, case.source);
        }
    }
}

#[test]
fn fuse_client_roundtrip_cases_load_from_live_cast() {
    let cases = load_roundtrip_cases();
    assert!(!cases.is_empty(), "expected fuse_client.cct to expose script roundtrip cases");

    let handler_count = cases.iter().map(|case| case.original_handlers.len()).sum::<usize>();
    assert!(handler_count > 0, "expected fuse_client.cct to expose handlers");
}

#[test]
fn compiler_matches_live_fuse_client_for_selected_scripts() {
    let cases = load_roundtrip_cases();
    let selected = [
        ("Broker Manager API", None),
        ("Object API", Some(&["constructObjectManager"][..])),
    ];

    for (script_name, selected_handlers) in selected {
        let case = cases
            .iter()
            .find(|case| case.script_name == script_name)
            .unwrap_or_else(|| panic!("missing live script {script_name}"));

        let mismatches = roundtrip_script_handlers(case, selected_handlers)
            .unwrap_or_else(|error| panic!("{} compile failed: {error}\n\n{}", case.script_name, case.source));

        assert!(
            mismatches.is_empty(),
            "{} roundtrip mismatches in cast {} ({})\n\nsource:\n{}\n\n{}",
            mismatches.len(),
            case.cast_number,
            case.cast_name,
            case.source,
            mismatches.join("\n\n")
        );
    }
}

#[test]
#[ignore = "broad live compiler roundtrip sweep for fuse_client.cct"]
fn compiler_matches_live_fuse_client_for_all_scripts() {
    let cases = load_roundtrip_cases();
    let mut mismatches = Vec::new();
    let mut compile_errors = Vec::new();

    for case in &cases {
        match roundtrip_script(case) {
            Ok(script_mismatches) if script_mismatches.is_empty() => {}
            Ok(script_mismatches) => mismatches.extend(script_mismatches),
            Err(error) => compile_errors.push(format!(
                "{} (cast {} member {}) compile error: {}",
                case.script_name,
                case.cast_number,
                case.member_number,
                error
            )),
        }
    }

    let mut failures = Vec::new();
    failures.extend(compile_errors);
    failures.extend(mismatches);

    assert!(
        failures.is_empty(),
        "{} live roundtrip failures\n\n{}",
        failures.len(),
        failures.iter().take(200).cloned().collect::<Vec<_>>().join("\n\n")
    );
}