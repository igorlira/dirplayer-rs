#[cfg(not(target_arch = "wasm32"))]
mod lingo;
#[cfg(not(target_arch = "wasm32"))]
mod lingo_compiler_shared_cast_surface;
mod e2e;
mod multiuser;

#[test]
#[ignore]
fn dump_fuse_client_names() {
    use std::path::Path;
    use vm_rust::director::file::{get_variable_multiplier, read_director_file_bytes};
    
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("public/dcr_woodpecker/fuse_client.cct");
    let bytes = std::fs::read(&path).unwrap();
    let dir = read_director_file_bytes(&bytes, "fuse_client.cct", "file://").unwrap();
    let cast = dir.casts.first().unwrap();
    let lctx = cast.lctx.as_ref().unwrap();
    for id in [79usize, 165, 201, 376, 749, 1073, 1074, 1109, 1110, 1257, 1465, 1480] {
        let name = lctx.names.get(id).map(|s| s.as_str()).unwrap_or("UNKNOWN");
        println!("{id}: {name}");
    }
}

#[test]
#[ignore]
fn dump_fuse_client_info() {
    use std::path::Path;
    use std::collections::BTreeSet;
    use vm_rust::director::file::read_director_file_bytes;
    use vm_rust::player::{bitmap::manager::BitmapManager, cast_member::{CastMember, CastMemberType}, cast_lib::{CastLib, CastLibState}};
    use fxhash::FxHashMap;
    use vm_rust::director::file::get_variable_multiplier;
    use vm_rust::director::lingo::opcode::OpCode;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("public/dcr_woodpecker/fuse_client.cct");
    let bytes = std::fs::read(&path).unwrap();
    let dir = read_director_file_bytes(&bytes, "fuse_client.cct", "file://").unwrap();
    let cast_def = dir.casts.first().unwrap();
    println!("dir_version: {}", cast_def.dir_version);
    println!("capital_x: {}", cast_def.capital_x);
    let lctx = cast_def.lctx.as_ref().unwrap();

    let mut thebuiltin_names: BTreeSet<String> = BTreeSet::new();
    let mut movieprop_names: BTreeSet<String> = BTreeSet::new();

    let mut cast = CastLib {
        name: "fuse_client".to_string(),
        file_name: path.to_string_lossy().to_string(),
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
    for (member_number, member_def) in &cast_def.members {
        let member = CastMember::from(cast.number, *member_number, member_def, &cast_def.lctx, &mut bitmap_manager, cast.dir_version, cast.palette_id_offset, &dir.font_table);
        if matches!(member.member_type, CastMemberType::Script(_)) {
            cast.insert_member(*member_number, member);
        }
    }

    for (_, script) in &cast.scripts {
        for handler_name in &script.handler_names {
            if let Some(handler) = script.get_own_handler(handler_name) {
                for bc in &handler.bytecode_array {
                    let name = lctx.names.get(bc.obj as usize).map(|s| s.as_str()).unwrap_or("?");
                    match bc.opcode {
                        OpCode::TheBuiltin => { thebuiltin_names.insert(name.to_string()); }
                        OpCode::GetMovieProp => { movieprop_names.insert(name.to_string()); }
                        _ => {}
                    }
                }
            }
        }
    }

    println!("TheBuiltin names:");
    for name in &thebuiltin_names { println!("  {name}"); }
    println!("GetMovieProp names:");
    for name in &movieprop_names { println!("  {name}"); }
}
