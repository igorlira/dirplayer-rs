// MCP (Model Context Protocol) query functions for VM debugging
// These functions return JSON strings for use with the MCP server

use std::rc::Rc;

use fxhash::FxHashMap;
use serde::Serialize;

use crate::director::{
    enums::ScriptType,
    file::get_variable_multiplier,
    lingo::{decompiler::handler::decompile_handler, script::ScriptContext as LingoScriptContext},
};

use super::{
    allocator::{DatumAllocatorTrait, ScriptInstanceAllocatorTrait},
    cast_lib::{CastLib, CastMemberRef},
    datum_formatting::format_concrete_datum,
    datum_ref::DatumId,
    script::Script,
    DirPlayer,
};

// ============================================================================
// Response types for MCP tools
// ============================================================================

#[derive(Serialize)]
pub struct McpScriptInfo {
    pub cast_lib: i32,
    pub cast_member: i32,
    pub name: String,
    pub script_type: String,
    pub handlers: Vec<String>,
}

#[derive(Serialize)]
pub struct McpScriptDetails {
    pub cast_lib: i32,
    pub cast_member: i32,
    pub name: String,
    pub script_type: String,
    pub handlers: Vec<McpHandlerInfo>,
    pub properties: Vec<String>,
}

#[derive(Serialize)]
pub struct McpHandlerInfo {
    pub name: String,
    pub arguments: Vec<String>,
    pub locals: Vec<String>,
    pub bytecode_count: usize,
}

#[derive(Serialize)]
pub struct McpBytecodeInstruction {
    pub pos: usize,
    pub opcode: String,
    pub operand: i64,
    pub text: String,
}

#[derive(Serialize)]
pub struct McpDisassemblyResult {
    pub handler_name: String,
    pub arguments: Vec<String>,
    pub bytecode: Vec<McpBytecodeInstruction>,
}

#[derive(Serialize)]
pub struct McpDecompiledLine {
    pub text: String,
    pub indent: u32,
    pub bytecode_indices: Vec<usize>,
}

#[derive(Serialize)]
pub struct McpDecompileResult {
    pub handler_name: String,
    pub arguments: Vec<String>,
    pub lines: Vec<McpDecompiledLine>,
    pub source: String,
}

#[derive(Serialize)]
pub struct McpScopeInfo {
    pub index: usize,
    pub script_name: String,
    pub cast_lib: i32,
    pub cast_member: i32,
    pub handler_name: String,
    pub bytecode_index: usize,
    pub locals: FxHashMap<String, McpDatumValue>,
    pub args: Vec<McpDatumValue>,
    pub stack_depth: usize,
}

#[derive(Serialize)]
pub struct McpCallStack {
    pub scopes: Vec<McpScopeInfo>,
    pub current_scope_index: Option<usize>,
}

#[derive(Serialize)]
pub struct McpExecutionState {
    pub is_playing: bool,
    pub is_paused: bool,
    pub current_frame: u32,
    pub total_frames: usize,
    pub at_breakpoint: bool,
    pub movie_loaded: bool,
    pub movie_title: String,
    pub stage_width: u32,
    pub stage_height: u32,
}

#[derive(Serialize)]
pub struct McpDatumValue {
    pub datum_id: Option<usize>,
    pub type_name: String,
    pub value: String,
}

#[derive(Serialize)]
pub struct McpGlobalsResult {
    pub globals: FxHashMap<String, McpDatumValue>,
}

#[derive(Serialize)]
pub struct McpLocalsResult {
    pub scope_index: usize,
    pub handler_name: String,
    pub locals: FxHashMap<String, McpDatumValue>,
    pub args: Vec<McpArgInfo>,
}

#[derive(Serialize)]
pub struct McpArgInfo {
    pub name: String,
    pub value: McpDatumValue,
}

#[derive(Serialize)]
pub struct McpDatumInspection {
    pub datum_id: usize,
    pub type_name: String,
    pub value: String,
    pub properties: Option<FxHashMap<String, McpDatumValue>>,
}

#[derive(Serialize)]
pub struct McpCastMemberInfo {
    pub cast_lib: i32,
    pub cast_member: i32,
    pub name: String,
    pub member_type: String,
}

#[derive(Serialize)]
pub struct McpCastMemberDetails {
    pub cast_lib: i32,
    pub cast_member: i32,
    pub name: String,
    pub member_type: String,
    pub script_type: Option<String>,
    pub handlers: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct McpBreakpointInfo {
    pub script_name: String,
    pub handler_name: String,
    pub bytecode_index: usize,
}

#[derive(Serialize)]
pub struct McpBreakpointList {
    pub breakpoints: Vec<McpBreakpointInfo>,
}

#[derive(Serialize)]
pub struct McpError {
    pub error: String,
}

// ============================================================================
// Helper functions
// ============================================================================

/// Serialize result to JSON, with error fallback
fn to_json<T: Serialize>(result: &T) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|e| {
        serde_json::to_string(&McpError { error: e.to_string() }).unwrap()
    })
}

/// Create an error JSON response
fn mcp_error(msg: impl Into<String>) -> String {
    serde_json::to_string(&McpError { error: msg.into() }).unwrap()
}

fn script_type_str(script_type: &ScriptType) -> &'static str {
    match script_type {
        ScriptType::Movie => "movie",
        ScriptType::Parent => "parent",
        ScriptType::Score => "score",
        ScriptType::Member => "member",
        ScriptType::Invalid => "invalid",
        ScriptType::Unknown => "unknown",
    }
}

fn datum_to_mcp_value(player: &DirPlayer, datum_ref: &super::DatumRef) -> McpDatumValue {
    let datum = player.get_datum(datum_ref);
    McpDatumValue {
        datum_id: match datum_ref {
            super::DatumRef::Void => None,
            super::DatumRef::Ref(id, _) => Some(*id),
        },
        type_name: datum.type_str().to_string(),
        value: format_concrete_datum(datum, player),
    }
}

fn get_script_info(script: &Script) -> McpScriptInfo {
    McpScriptInfo {
        cast_lib: script.member_ref.cast_lib,
        cast_member: script.member_ref.cast_member,
        name: script.name.clone(),
        script_type: script_type_str(&script.script_type).to_string(),
        handlers: script.handler_names.clone(),
    }
}

/// Helper struct for script context lookups
struct ScriptLookup<'a> {
    script: &'a Rc<Script>,
    cast: &'a CastLib,
    lctx: &'a LingoScriptContext,
    multiplier: u32,
}

/// Look up script, cast, and lctx for a member reference
fn get_script_context<'a>(
    player: &'a DirPlayer,
    member_ref: &CastMemberRef,
) -> Result<ScriptLookup<'a>, String> {
    let script = player
        .movie
        .cast_manager
        .get_script_by_ref(member_ref)
        .ok_or_else(|| {
            format!(
                "Script not found at cast_lib={}, cast_member={}",
                member_ref.cast_lib, member_ref.cast_member
            )
        })?;

    let cast = player
        .movie
        .cast_manager
        .get_cast(member_ref.cast_lib as u32)
        .map_err(|_| format!("Cast library {} not found", member_ref.cast_lib))?;

    let lctx = cast
        .lctx
        .as_ref()
        .ok_or_else(|| "Script context not available".to_string())?;

    Ok(ScriptLookup {
        script,
        cast,
        lctx,
        multiplier: get_variable_multiplier(cast.capital_x, cast.dir_version),
    })
}

/// Get handler name from name ID using lctx
fn get_handler_name(lctx: Option<&LingoScriptContext>, name_id: u16) -> String {
    lctx.and_then(|l| l.names.get(name_id as usize))
        .cloned()
        .unwrap_or_else(|| format!("handler_{}", name_id))
}

/// Get argument names from handler using lctx
fn get_argument_names(lctx: Option<&LingoScriptContext>, arg_ids: &[u16]) -> Vec<String> {
    arg_ids
        .iter()
        .map(|&id| {
            lctx.and_then(|l| l.names.get(id as usize))
                .cloned()
                .unwrap_or_else(|| format!("arg_{}", id))
        })
        .collect()
}

// ============================================================================
// MCP Query Functions
// ============================================================================

/// List all scripts in the movie
pub fn mcp_list_scripts(player: &DirPlayer) -> String {
    let scripts: Vec<McpScriptInfo> = player
        .movie
        .cast_manager
        .casts
        .iter()
        .flat_map(|cast| cast.scripts.values().map(|s| get_script_info(s)))
        .collect();

    to_json(&scripts)
}

/// Get detailed information about a specific script
pub fn mcp_get_script(player: &DirPlayer, cast_lib: i32, cast_member: i32) -> String {
    let member_ref = CastMemberRef { cast_lib, cast_member };

    let ctx = match get_script_context(player, &member_ref) {
        Ok(c) => c,
        Err(e) => return mcp_error(e),
    };

    let handlers: Vec<McpHandlerInfo> = ctx
        .script
        .handlers
        .iter()
        .map(|(name, handler)| McpHandlerInfo {
            name: name.clone(),
            arguments: get_argument_names(Some(ctx.lctx), &handler.argument_name_ids),
            locals: handler
                .local_name_ids
                .iter()
                .map(|&id| {
                    ctx.lctx
                        .names
                        .get(id as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("local_{}", id))
                })
                .collect(),
            bytecode_count: handler.bytecode_array.len(),
        })
        .collect();

    to_json(&McpScriptDetails {
        cast_lib: ctx.script.member_ref.cast_lib,
        cast_member: ctx.script.member_ref.cast_member,
        name: ctx.script.name.clone(),
        script_type: script_type_str(&ctx.script.script_type).to_string(),
        handlers,
        properties: ctx.script.properties.borrow().keys().cloned().collect(),
    })
}

/// Disassemble a handler (show bytecode)
pub fn mcp_disassemble_handler(
    player: &DirPlayer,
    cast_lib: i32,
    cast_member: i32,
    handler_name: &str,
) -> String {
    let member_ref = CastMemberRef { cast_lib, cast_member };

    let ctx = match get_script_context(player, &member_ref) {
        Ok(c) => c,
        Err(e) => return mcp_error(e),
    };

    let handler = match ctx.script.get_own_handler(&handler_name.to_lowercase()) {
        Some(h) => h,
        None => return mcp_error(format!("Handler '{}' not found in script", handler_name)),
    };

    to_json(&McpDisassemblyResult {
        handler_name: handler_name.to_string(),
        arguments: get_argument_names(Some(ctx.lctx), &handler.argument_name_ids),
        bytecode: handler
            .bytecode_array
            .iter()
            .map(|bc| McpBytecodeInstruction {
                pos: bc.pos,
                opcode: format!("{:?}", bc.opcode),
                operand: bc.obj,
                text: bc.to_bytecode_text(ctx.lctx, &handler, ctx.multiplier),
            })
            .collect(),
    })
}

/// Decompile a handler (show Lingo source)
pub fn mcp_decompile_handler(
    player: &DirPlayer,
    cast_lib: i32,
    cast_member: i32,
    handler_name: &str,
) -> String {
    let member_ref = CastMemberRef { cast_lib, cast_member };

    let ctx = match get_script_context(player, &member_ref) {
        Ok(c) => c,
        Err(e) => return mcp_error(e),
    };

    let handler = match ctx.script.get_own_handler(&handler_name.to_lowercase()) {
        Some(h) => h,
        None => return mcp_error(format!("Handler '{}' not found in script", handler_name)),
    };

    let decompiled = decompile_handler(
        &handler,
        &ctx.script.chunk,
        ctx.lctx,
        ctx.cast.dir_version,
        ctx.multiplier,
    );

    // Build full source with indentation
    let source = decompiled
        .lines
        .iter()
        .map(|line| format!("{}{}", "  ".repeat(line.indent as usize), line.text))
        .collect::<Vec<_>>()
        .join("\n");

    to_json(&McpDecompileResult {
        handler_name: decompiled.name,
        arguments: decompiled.arguments,
        lines: decompiled
            .lines
            .iter()
            .map(|line| McpDecompiledLine {
                text: line.text.clone(),
                indent: line.indent,
                bytecode_indices: line.bytecode_indices.clone(),
            })
            .collect(),
        source,
    })
}

/// Get the current call stack
pub fn mcp_get_call_stack(player: &DirPlayer) -> String {
    let scopes: Vec<McpScopeInfo> = player
        .scopes
        .iter()
        .enumerate()
        .filter(|(_, scope)| scope.script_ref.cast_lib != 0 || scope.script_ref.cast_member != 0)
        .map(|(index, scope)| {
            let script_name = player
                .movie
                .cast_manager
                .get_script_by_ref(&scope.script_ref)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let lctx = player
                .movie
                .cast_manager
                .get_cast(scope.script_ref.cast_lib as u32)
                .ok()
                .and_then(|c| c.lctx.as_ref());

            McpScopeInfo {
                index,
                script_name,
                cast_lib: scope.script_ref.cast_lib,
                cast_member: scope.script_ref.cast_member,
                handler_name: get_handler_name(lctx, scope.handler_name_id),
                bytecode_index: scope.bytecode_index,
                locals: scope
                    .locals
                    .iter()
                    .map(|(name, datum_ref)| (name.clone(), datum_to_mcp_value(player, datum_ref)))
                    .collect(),
                args: scope
                    .args
                    .iter()
                    .map(|datum_ref| datum_to_mcp_value(player, datum_ref))
                    .collect(),
                stack_depth: scope.stack.len(),
            }
        })
        .collect();

    to_json(&McpCallStack {
        current_scope_index: if scopes.is_empty() { None } else { Some(scopes.len() - 1) },
        scopes,
    })
}

/// Get execution state
pub fn mcp_get_execution_state(player: &DirPlayer) -> String {
    to_json(&McpExecutionState {
        is_playing: player.is_playing,
        is_paused: player.is_script_paused,
        current_frame: player.movie.current_frame,
        total_frames: player
            .movie
            .score
            .sprite_spans
            .iter()
            .map(|span| span.end_frame as usize)
            .max()
            .unwrap_or(0),
        at_breakpoint: player.current_breakpoint.is_some(),
        movie_loaded: !player.movie.score.sprite_spans.is_empty(),
        movie_title: player.title.clone(),
        stage_width: player.stage_size.0,
        stage_height: player.stage_size.1,
    })
}

/// Get all global variables
pub fn mcp_get_globals(player: &DirPlayer) -> String {
    to_json(&McpGlobalsResult {
        globals: player
            .globals
            .iter()
            .map(|(name, datum_ref)| (name.clone(), datum_to_mcp_value(player, datum_ref)))
            .collect(),
    })
}

/// Get locals for a specific scope
pub fn mcp_get_locals(player: &DirPlayer, scope_index: Option<usize>) -> String {
    let index = scope_index.unwrap_or_else(|| player.scopes.len().saturating_sub(1));

    let scope = match player.scopes.get(index) {
        Some(s) => s,
        None => return mcp_error(format!("Scope index {} not found", index)),
    };

    let lctx = player
        .movie
        .cast_manager
        .get_cast(scope.script_ref.cast_lib as u32)
        .ok()
        .and_then(|c| c.lctx.as_ref());

    // Get argument names from handler definition
    let arg_names: Vec<String> = player
        .movie
        .cast_manager
        .get_script_by_ref(&scope.script_ref)
        .and_then(|script| script.get_own_handler_by_name_id(scope.handler_name_id))
        .map(|handler| get_argument_names(lctx, &handler.argument_name_ids))
        .unwrap_or_default();

    to_json(&McpLocalsResult {
        scope_index: index,
        handler_name: get_handler_name(lctx, scope.handler_name_id),
        locals: scope
            .locals
            .iter()
            .map(|(name, datum_ref)| (name.clone(), datum_to_mcp_value(player, datum_ref)))
            .collect(),
        args: scope
            .args
            .iter()
            .enumerate()
            .map(|(i, datum_ref)| McpArgInfo {
                name: arg_names.get(i).cloned().unwrap_or_else(|| format!("arg{}", i)),
                value: datum_to_mcp_value(player, datum_ref),
            })
            .collect(),
    })
}

/// Inspect a datum by ID
pub fn mcp_inspect_datum(player: &DirPlayer, datum_id: DatumId) -> String {
    let datum_ref = match player.allocator.get_datum_ref(datum_id) {
        Some(r) => r,
        None => return mcp_error(format!("Datum with ID {} not found", datum_id)),
    };

    let datum = player.get_datum(&datum_ref);

    // For script instances and prop lists, include properties
    let properties = match datum {
        crate::director::lingo::datum::Datum::ScriptInstanceRef(instance_ref) => Some(
            player
                .allocator
                .get_script_instance(instance_ref)
                .properties
                .iter()
                .map(|(name, datum_ref)| (name.clone(), datum_to_mcp_value(player, datum_ref)))
                .collect(),
        ),
        crate::director::lingo::datum::Datum::PropList(entries, _) => Some(
            entries
                .iter()
                .map(|(k, v)| {
                    (format_concrete_datum(player.get_datum(k), player), datum_to_mcp_value(player, v))
                })
                .collect(),
        ),
        _ => None,
    };

    to_json(&McpDatumInspection {
        datum_id,
        type_name: datum.type_str().to_string(),
        value: format_concrete_datum(datum, player),
        properties,
    })
}

/// List cast members
pub fn mcp_list_cast_members(player: &DirPlayer, cast_lib: Option<i32>) -> String {
    let members: Vec<McpCastMemberInfo> = player
        .movie
        .cast_manager
        .casts
        .iter()
        .filter(|cast| cast_lib.map_or(true, |lib| cast.number as i32 == lib))
        .flat_map(|cast| {
            cast.members.iter().map(move |(&member_num, member)| McpCastMemberInfo {
                cast_lib: cast.number as i32,
                cast_member: member_num as i32,
                name: member.name.clone(),
                member_type: member.member_type.type_string().to_string(),
            })
        })
        .collect();

    to_json(&members)
}

/// Inspect a cast member
pub fn mcp_inspect_cast_member(player: &DirPlayer, cast_lib: i32, cast_member: i32) -> String {
    let member_ref = CastMemberRef { cast_lib, cast_member };

    let cast = match player.movie.cast_manager.get_cast(cast_lib as u32) {
        Ok(c) => c,
        Err(_) => return mcp_error(format!("Cast library {} not found", cast_lib)),
    };

    let member = match cast.members.get(&(cast_member as u32)) {
        Some(m) => m,
        None => return mcp_error(format!("Cast member {} not found in cast library {}", cast_member, cast_lib)),
    };

    // For script members, include script type and handlers
    let (script_type, handlers) = player
        .movie
        .cast_manager
        .get_script_by_ref(&member_ref)
        .map(|script| {
            (
                Some(script_type_str(&script.script_type).to_string()),
                Some(script.handler_names.clone()),
            )
        })
        .unwrap_or((None, None));

    to_json(&McpCastMemberDetails {
        cast_lib,
        cast_member,
        name: member.name.clone(),
        member_type: member.member_type.type_string().to_string(),
        script_type,
        handlers,
    })
}

/// List all breakpoints
pub fn mcp_list_breakpoints(player: &DirPlayer) -> String {
    to_json(&McpBreakpointList {
        breakpoints: player
            .breakpoint_manager
            .breakpoints
            .iter()
            .map(|bp| McpBreakpointInfo {
                script_name: bp.script_name.clone(),
                handler_name: bp.handler_name.clone(),
                bytecode_index: bp.bytecode_index,
            })
            .collect(),
    })
}
