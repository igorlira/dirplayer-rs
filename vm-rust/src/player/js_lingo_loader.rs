// Glue between the JS-Lingo interpreter and the rest of dirplayer.
//
// Loaded scripts: `register_js_script` is called from cast_lib::insert_member
// for any script whose Lscr literal-data area starts with the SpiderMonkey
// XDR magic. It decodes the script, instantiates a JsRuntime with the
// stdlib + a player-bound bridge, runs the program once to hoist globals,
// and stores the runtime in a thread-local registry keyed by member ref.
//
// Handler dispatch: `try_invoke_js_handler` is the hook the existing
// `player_call_script_handler` consults before walking Lingo bytecode.
// If the named handler exists as a JS function value on the script's
// runtime global, we invoke it and convert the return value back to a
// DatumRef.
//
// Why thread_local? The interpreter holds Rc-based state that isn't Send,
// so a global is the right shape; dirplayer is single-threaded under wasm
// in practice.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::director::lingo::datum::Datum;
use crate::player::cast_lib::CastMemberRef;
use crate::player::allocator::DatumAllocatorTrait;
use crate::player::datum_ref::DatumRef;
use crate::player::js_lingo::host_bridge::{JsHostBridge, StubBridge};
use crate::player::js_lingo::value::{JsError, JsObjectRef, JsValue};
use crate::player::js_lingo::{decode_script, disasm::disassemble, JsScriptIR};
use crate::player::js_lingo::interpreter::JsRuntime;
use crate::player::reserve_player_mut;
use crate::player::script::Script;
use crate::player::symbols::symbol::Symbol;

thread_local! {
    /// JS runtime per script (one per JS-Lingo cast member).
    static JS_RUNTIMES: RefCell<HashMap<CastMemberRef, Rc<RefCell<JsRuntime>>>> = RefCell::new(HashMap::new());

    /// Live JS objects handed to the Lingo side as `Datum::JsObjectRef(id)`.
    /// Each entry keeps the object alive together with the runtime that owns
    /// it
    static JS_OBJECTS: RefCell<HashMap<u32, (Rc<RefCell<JsRuntime>>, JsObjectRef)>> = RefCell::new(HashMap::new());
    static NEXT_JS_OBJECT_ID: RefCell<u32> = RefCell::new(1);

    /// Runtime whose code is currently executing. `js_value_to_datum_ref`
    /// needs to know which interpreter a returned object belongs to, and the
    /// conversion happens deep inside the bridge where the script ref isn't
    /// threaded through. Pushed/popped around every `invoke`, so it behaves
    /// correctly when one JS script calls into another.
    static CURRENT_RUNTIME: RefCell<Vec<Rc<RefCell<JsRuntime>>>> = RefCell::new(Vec::new());
}

/// Run `f` with `runtime` marked as the executing runtime.
fn with_current_runtime<T>(runtime: &Rc<RefCell<JsRuntime>>, f: impl FnOnce() -> T) -> T {
    CURRENT_RUNTIME.with(|s| s.borrow_mut().push(runtime.clone()));
    let result = f();
    CURRENT_RUNTIME.with(|s| { s.borrow_mut().pop(); });
    result
}

/// Register a JS object for Lingo-side use and return its handle. Objects
/// are de-duplicated by pointer identity
fn register_js_object(runtime: &Rc<RefCell<JsRuntime>>, obj: &JsObjectRef) -> u32 {
    JS_OBJECTS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((id, _)) = m.iter().find(|(_, (_, o))| Rc::ptr_eq(o, obj)) {
            return *id;
        }
        let id = NEXT_JS_OBJECT_ID.with(|n| {
            let mut n = n.borrow_mut();
            let id = *n;
            *n += 1;
            id
        });
        m.insert(id, (runtime.clone(), obj.clone()));
        id
    })
}

/// Resolve a `Datum::JsObjectRef` handle back to its runtime and object.
pub fn get_js_object(id: u32) -> Option<(Rc<RefCell<JsRuntime>>, JsObjectRef)> {
    JS_OBJECTS.with(|m| m.borrow().get(&id).cloned())
}

/// Whether an object must cross the bridge as a live handle rather than a
/// `PropList` copy: it does as soon as anything on it is callable, since a
/// copy would flatten those methods into strings. Plain data objects keep
/// the historical PropList conversion so existing JS→Lingo records (and the
/// Lingo code that walks them with `getAt`/`getPropAt`) are unaffected.
fn object_has_callable(obj: &JsObjectRef) -> bool {
    let mut current = Some(obj.clone());
    // Bounded walk: a malformed proto cycle must not hang the player.
    for _ in 0..32 {
        let Some(o) = current else { return false };
        let b = o.borrow();
        if b.props.iter().any(|(_, v)| matches!(v, JsValue::Function(_) | JsValue::Native(_))) {
            return true;
        }
        current = b.proto.clone();
    }
    false
}

/// Invoke a method on a live JS object on behalf of Lingo. `name` is matched
/// case-sensitively first, then case-insensitively — Lingo symbols don't
/// preserve the case JS declared the property with.
///
/// Returns `None` when the object has no callable property of that name, so
/// the caller can raise Lingo's own "no handler" error.
pub fn invoke_js_object_method(
    id: u32,
    name: &str,
    args: &[DatumRef],
) -> Option<Result<DatumRef, String>> {
    let (runtime, obj) = get_js_object(id)?;
    let callee = resolve_js_object_prop(&obj, name)?;
    if !matches!(callee, JsValue::Function(_) | JsValue::Native(_)) {
        return None;
    }
    let js_args: Vec<JsValue> = reserve_player_mut(|player| {
        args.iter().map(|d| datum_ref_to_js_value(player, d)).collect()
    });
    // `this` is the object itself: a JS-Lingo method reached through the
    // object (rather than through the script's global scope) is a genuine
    // method call, so `this.foo` must see the instance's own properties.
    let this_value = JsValue::Object(obj.clone());
    // Convert inside the scope — see `try_invoke_js_handler`.
    Some(with_current_runtime(&runtime, || {
        let result = runtime.borrow_mut().invoke(&callee, js_args, this_value);
        match result {
            Ok(v) => Ok(reserve_player_mut(|player| js_value_to_datum_ref(player, &v))),
            Err(e) => Err(e.message),
        }
    }))
}

/// Read a property off a live JS object for Lingo. `None` = absent, so the
/// caller can decide what a missing property means.
pub fn get_js_object_prop(id: u32, name: &str) -> Option<DatumRef> {
    let (runtime, obj) = get_js_object(id)?;
    let value = resolve_js_object_prop(&obj, name)?;
    // Scoped, so a nested object with methods of its own also crosses as a
    // live handle rather than being flattened.
    Some(with_current_runtime(&runtime, || {
        reserve_player_mut(|player| js_value_to_datum_ref(player, &value))
    }))
}

/// Look a property up on a live JS object, walking the proto chain, with the
/// case-insensitive fallback Lingo callers need. `None` = absent.
fn resolve_js_object_prop(obj: &JsObjectRef, name: &str) -> Option<JsValue> {
    let mut current = Some(obj.clone());
    for _ in 0..32 {
        let o = current?;
        let b = o.borrow();
        if let Some(v) = b.get_own(name) {
            return Some(v.clone());
        }
        if let Some((_, v)) = b.props.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            return Some(v.clone());
        }
        current = b.proto.clone();
    }
    None
}

/// Write a property on a live JS object, reusing the existing key's casing
/// when one matches case-insensitively so Lingo writes don't shadow the
/// property JS declared.
pub fn set_js_object_prop(obj: &JsObjectRef, name: &str, value: JsValue) {
    let mut b = obj.borrow_mut();
    let key = b
        .props
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| name.to_string());
    b.set_own(&key, value);
}

/// Public entry called from cast_lib::insert_member at script-load time.
/// Detects whether a Script is JS-Lingo, sets up the runtime, and emits
/// a disassembly to the log for diagnostics.
pub fn diagnose_js_script(script: &Script) {
    let Some(payload) = extract_js_payload(script) else { return; };
    log::info!(
        "[js-lingo] {}:{} loading JSScript ({} bytes)",
        script.member_ref.cast_lib, script.member_ref.cast_member, payload.len()
    );
    match decode_script(payload) {
        Ok(ir) => {
            for line in disassemble(&ir).lines() {
                log::info!("[js-lingo]   {}", line);
            }
            register_runtime(&script.member_ref, ir);
        }
        Err(e) => {
            log::warn!("[js-lingo]   decode failed: {}", e);
        }
    }
}

/// Look up the JSScript payload from the literals area, if present.
fn extract_js_payload(script: &Script) -> Option<&[u8]> {
    for lit in &script.chunk.literals {
        if let Datum::JavaScript(b) = lit {
            return Some(b);
        }
    }
    None
}

fn register_runtime(member_ref: &CastMemberRef, ir: JsScriptIR) {
    let runtime = Rc::new(RefCell::new(JsRuntime::with_stdlib()));
    // Wire a player-bound bridge so trace/sprite/member route to existing
    // Director machinery instead of the StubBridge default.
    let bridge: Rc<RefCell<dyn JsHostBridge>> = Rc::new(RefCell::new(PlayerBridge));
    runtime.borrow_mut().set_bridge(bridge);
    runtime.borrow().install_director_globals();

    // Run the program once to hoist globals and execute initialisers.
    let ir = Rc::new(ir);
    // The temporary `runtime.borrow_mut()` for `.run_program(...)` outlives
    // the match's RHS in Rust 2021; binding the result first ends the
    // mutable borrow before we re-borrow as immutable below.
    let init_result = runtime.borrow_mut().run_program(&ir);
    match init_result {
        Ok(_) => {
            let rt = runtime.borrow();
            let g = rt.global.borrow();
            let mut fns: Vec<&str> = Vec::new();
            let mut vars: Vec<(String, String)> = Vec::new();
            for (k, v) in &g.props {
                match v {
                    JsValue::Function(_) | JsValue::Native(_) => fns.push(k.as_str()),
                    JsValue::Undefined => vars.push((k.clone(), "undefined".into())),
                    JsValue::String(s) => {
                        let preview = if s.len() > 60 { format!("{}...", &s[..60]) } else { (**s).clone() };
                        vars.push((k.clone(), format!("{:?}", preview)));
                    }
                    JsValue::Int(i) => vars.push((k.clone(), i.to_string())),
                    JsValue::Number(n) => vars.push((k.clone(), n.to_string())),
                    JsValue::Bool(b) => vars.push((k.clone(), b.to_string())),
                    JsValue::Null => vars.push((k.clone(), "null".into())),
                    JsValue::Array(a) => vars.push((k.clone(), format!("[len {}]", a.borrow().items.len()))),
                    JsValue::Object(_) => vars.push((k.clone(), "[object]".into())),
                    JsValue::Iterator(_) => vars.push((k.clone(), "[for-in iter]".into())),
                    JsValue::DirectorRef(r) => vars.push((k.clone(), format!("{:?}", r))),
                }
            }
            log::debug!(
                "[js-lingo] {}:{} program init OK — {} functions, {} non-function globals",
                member_ref.cast_lib, member_ref.cast_member, fns.len(), vars.len()
            );
            log::debug!("[js-lingo]   functions: {}", fns.join(", "));
            for (k, v) in &vars {
                log::debug!("[js-lingo]   var {} = {}", k, v);
            }
        }
        Err(e) => {
            log::warn!(
                "[js-lingo] {}:{} program init FAILED: {}",
                member_ref.cast_lib, member_ref.cast_member, e
            );
        }
    }
    JS_RUNTIMES.with(|m| {
        m.borrow_mut().insert(member_ref.clone(), runtime);
    });
}

/// Drop every registered JS-Lingo runtime. Called on movie reset so the
/// previous movie's runtimes (and everything their global scope holds) don't
/// leak across a movie switch — this map is a `thread_local`, so unlike the
/// player's own caches it is NOT freed when the `DirPlayer` is dropped.
/// Runtimes are re-registered lazily by `diagnose_js_script` as the new
/// movie's scripts load.
pub fn clear_all_runtimes() {
    JS_RUNTIMES.with(|m| m.borrow_mut().clear());
    // Object handles keep their runtime alive, so they must go too — otherwise
    // clearing JS_RUNTIMES wouldn't actually free the previous movie's
    // interpreters.
    JS_OBJECTS.with(|m| m.borrow_mut().clear());
}

/// Hook called from `player_call_script_handler_raw_args` before the
/// existing Lingo handler lookup. `receiver` is the Script instance that
/// owns the handler (Director's `me`): when set, it's prepended to the
/// arg list as JS arg0, matching the calling convention SpiderMonkey
/// emits for `script(X).handler(a, b)` -> `handler(me, a, b)`. Returns:
/// - `Some(Ok(datum))` — JS handler ran successfully, here's the return value.
/// - `Some(Err(msg))` — JS handler exists but threw.
/// - `None` — no JS handler for this script/name; fall back to Lingo.
pub fn try_invoke_js_handler(
    script_member_ref: &CastMemberRef,
    handler_name: &str,
    args: &[DatumRef],
    has_receiver: bool,
) -> Option<Result<DatumRef, String>> {
    let runtime_opt = JS_RUNTIMES.with(|m| m.borrow().get(script_member_ref).cloned());
    let runtime = runtime_opt?;

    let callee = {
        let rt = runtime.borrow();
        let g = rt.global.borrow();
        g.get_own(handler_name).cloned()?
    };
    if !matches!(callee, JsValue::Function(_) | JsValue::Native(_)) {
        return None;
    }

    // Convert args. When the call is a method on a Script object (Director's
    // `script(X).handler(...)` syntax), Lingo bytecode reserves slot 0 for
    // the script instance (`me`) and starts user args at slot 1. The JS
    // SpiderMonkey emitter reads `getarg 1` / `getarg 2` for the first/second
    // user arg, so we must prepend a `me`-equivalent value here.
    //
    // The upstream `receiver` Option isn't reliable for all dispatch paths:
    // event scripts have receiver=None and JS function declares 0 args, and
    // `script(X).method()` paths sometimes also arrive with receiver=None
    // because of how the Lingo VM constructs the dispatch. As a precise
    // fallback we look at the JS function's declared `nargs`: if it's
    // exactly one more than we supply, the function expected a `me` slot
    // and we synthesise one. Strictly-matching arity (the common case) is
    // left alone.
    let mut js_args: Vec<JsValue> = reserve_player_mut(|player| {
        args.iter().map(|d| datum_ref_to_js_value(player, d)).collect()
    });
    // Prepend the `me` slot IFF the JS function's first parameter is named
    // like a receiver (`me`, `mee`, `self`, `_me`, `this_`, `_self`). This
    // is regardless of how Lingo dispatched -- the function declaration is
    // the ground truth. Director's JS-Lingo convention is `me` as the
    // first param when the function is intended as a script method; Habbo
    // movies use `mee` variant.
    let mut me_prepended = false;
    if let JsValue::Function(f) = &callee {
        let first_arg_is_me = f
            .atom
            .bindings
            .iter()
            .find(|b| b.kind == super::js_lingo::xdr::JsBindingKind::Argument)
            .map(|b| {
                let n = b.name.to_lowercase();
                n == "me" || n == "mee" || n == "self" || n == "_me" || n == "this_" || n == "_self"
            })
            .unwrap_or(false);
        if first_arg_is_me {
            js_args.insert(0, JsValue::Undefined);
            me_prepended = true;
        }
    }
    let _ = has_receiver;

    let arg_summary = js_args
        .iter()
        .map(|v| format!("{:?}", v))
        .collect::<Vec<_>>()
        .join(", ");
    // log::debug!(
    //     "[js-call v3 me={}] {}:{}::{}({})",
    //     me_prepended, script_member_ref.cast_lib, script_member_ref.cast_member, handler_name, arg_summary
    // );

    // Director's JS-Lingo treats the calling script as `this` for method
    // calls. Pass the runtime's global object so `this.helper(...)` inside
    // a handler resolves to sibling top-level functions in the same script
    // (Habbo's BigInt port relies on this pattern heavily).
    let this_value = JsValue::Object(runtime.borrow().global.clone());

    // The return-value conversion must run INSIDE the current-runtime scope:
    // that's where a returned object gets attributed to the interpreter that
    // owns it. Converting after the scope pops would leave the registry with
    // no runtime to attach and silently fall back to a PropList copy.
    Some(with_current_runtime(&runtime, || {
        let result = runtime.borrow_mut().invoke(&callee, js_args, this_value);
        match result {
            Ok(v) => Ok(reserve_player_mut(|player| js_value_to_datum_ref(player, &v))),
            Err(e) => Err(e.message),
        }
    }))
}

// ===== Bridge implementation that routes JS calls through Director runtime =====

struct PlayerBridge;

impl JsHostBridge for PlayerBridge {
    fn trace(&mut self, args: &[JsValue]) {
        let line = args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
        log::info!("[js-trace] {}", line);
    }
    fn sprite(&mut self, channel: i32) -> JsValue {
        // Phase 6.4: live sprite proxy. Property reads / writes routed via
        // get_property / set_property in the interpreter round-trip through
        // sprite_get_prop / sprite_set_prop, so `sprite(3).locH = 100`
        // actually moves the sprite on stage.
        JsValue::DirectorRef(crate::player::js_lingo::value::DirectorRefKind::Sprite(channel as i16))
    }
    fn member(&mut self, args: &[JsValue]) -> JsValue {
        // member(memberNameOrNum {, castNameOrNum}). The Lingo VM's
        // existing "member" handler accepts the same shape and resolves
        // the name to a CastMemberRef -- route through it so we get
        // consistent name/number/cast-lib resolution, then wrap the
        // resulting Datum back as a DirectorRef.
        let datum_args: Vec<DatumRef> = reserve_player_mut(|player| {
            args.iter().map(|v| js_value_to_datum_ref(player, v)).collect()
        });
        match crate::player::handlers::manager::BuiltInHandlerManager::call_handler(Symbol::from_str("member"), &datum_args) {
            Ok(dref) => reserve_player_mut(|player| datum_ref_to_js_value(player, &dref)),
            Err(_) => JsValue::Undefined,
        }
    }

    /// Fall-through for anything else the JS script tries to invoke as a
    /// global. We route through `BuiltInHandlerManager::call_handler`, which
    /// is the same registry Lingo uses for `gotoNetPage`, `getNetText`,
    /// `puppetTempo`, `count`, `script`, etc.
    fn call_global(&mut self, name: &str, args: &[JsValue]) -> Result<JsValue, JsError> {
        let result = reserve_player_mut(|player| {
            // Convert JsValue args into DatumRefs Lingo can consume.
            let datum_args: Vec<DatumRef> = args.iter()
                .map(|v| js_value_to_datum_ref(player, v))
                .collect();
            let r = crate::player::handlers::manager::BuiltInHandlerManager::call_handler(Symbol::from_str(name), &datum_args);
            // Convert return value back.
            match r {
                Ok(dref) => Ok(datum_ref_to_js_value(player, &dref)),
                Err(e) => Err(e),
            }
        });
        result.map_err(|e| JsError::new(format!("{}: {}", name, e.message)))
    }
}

// ===== Datum <-> JsValue =====

/// Convert a DatumRef (Lingo value) into a JsValue for use in JS code.
///
/// Coverage: primitives convert directly. Lists become Arrays, prop-lists
/// become Objects (lossy because Lingo prop-lists are case-insensitive).
/// References (Sprite, Member, etc.) wrap into JsObjects tagged with
/// the original Datum string form so reads still see something useful.
pub fn datum_ref_to_js_value(player: &mut crate::player::DirPlayer, dref: &DatumRef) -> JsValue {
    let d = player.allocator.get_datum(dref).clone();
    match d {
        Datum::Int(i) => JsValue::Int(i),
        Datum::Float(f) => JsValue::Number(f),
        Datum::String(s) => JsValue::String(Rc::new(s)),
        Datum::Symbol(s) => JsValue::String(Rc::new(s.to_string())),
        Datum::Void | Datum::Null => JsValue::Undefined,
        Datum::List(_, items, _) => {
            let arr: Vec<JsValue> = items.iter().map(|r| datum_ref_to_js_value(player, r)).collect();
            JsValue::Array(Rc::new(RefCell::new(crate::player::js_lingo::value::JsArray { items: arr })))
        }
        Datum::PropList(pairs, _) => {
            let mut obj = crate::player::js_lingo::value::JsObject::new();
            for (k_ref, v_ref) in pairs {
                let key = datum_ref_to_string(player, &k_ref);
                let v = datum_ref_to_js_value(player, &v_ref);
                obj.set_own(&key, v);
            }
            JsValue::Object(Rc::new(RefCell::new(obj)))
        }
        Datum::ScriptRef(member_ref) => script_ref_to_js_proxy(member_ref),
        // Round-trip a live handle back to the very same JS object, so
        // identity survives a Lingo→JS→Lingo hop
        Datum::JsObjectRef(id) => match get_js_object(id) {
            Some((_, obj)) => JsValue::Object(obj),
            None => JsValue::Undefined,
        },
        // Live Director-owned references. Property reads/writes round-trip
        // through datum_handlers via get_property / set_property in the
        // interpreter, so `sprite(3).locH = 100` actually moves the sprite
        // and reads back the new value.
        Datum::SpriteRef(n) => JsValue::DirectorRef(
            crate::player::js_lingo::value::DirectorRefKind::Sprite(n),
        ),
        Datum::CastMember(mref) => JsValue::DirectorRef(
            crate::player::js_lingo::value::DirectorRefKind::Member {
                cast_lib: mref.cast_lib,
                cast_member: mref.cast_member,
            },
        ),
        _ => {
            // Coarse fallback: stringify via the existing formatter so refs
            // like `(member 3 of castLib 1)` keep their readable form.
            let s = crate::player::datum_formatting::format_datum(dref, player);
            JsValue::String(Rc::new(s))
        }
    }
}

/// Wrap a `Datum::ScriptRef` as a JsObject that re-exposes the target
/// script's JS-Lingo handlers as callable Natives. Lets JS code do
/// `script("X").method(args)` and have it dispatch back into the right
/// runtime via try_invoke_js_handler.
fn script_ref_to_js_proxy(member_ref: CastMemberRef) -> JsValue {
    use crate::player::js_lingo::value::{JsObject, NativeFn};
    let mut obj = JsObject::new();
    obj.class_name = "ScriptRef";
    // Mirror the script-ref coordinates so the proxy round-trips back to a
    // DatumRef when JS hands it to another Lingo builtin.
    obj.set_own("__script_lib__", JsValue::Int(member_ref.cast_lib));
    obj.set_own("__script_member__", JsValue::Int(member_ref.cast_member));

    // If the target script has a JS-Lingo runtime registered, enumerate its
    // top-level handlers and bind each as a Native that dispatches back
    // through try_invoke_js_handler. This is what enables
    // `script("X").method(...)` from JS code.
    let runtime_opt = JS_RUNTIMES.with(|m| m.borrow().get(&member_ref).cloned());
    if let Some(runtime) = runtime_opt {
        let handler_names: Vec<String> = {
            let rt = runtime.borrow();
            let g = rt.global.borrow();
            g.props
                .iter()
                .filter_map(|(k, v)| match v {
                    JsValue::Function(_) | JsValue::Native(_) => Some(k.clone()),
                    _ => None,
                })
                .collect()
        };
        for name in handler_names {
            let ref_clone = member_ref.clone();
            let name_clone = name.clone();
            let native = NativeFn {
                name: "<script_method>",
                call: Box::new(move |args| invoke_script_method(&ref_clone, &name_clone, args)),
            };
            obj.set_own(&name, JsValue::Native(Rc::new(native)));
        }
    }
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Synchronous dispatch of a JS handler from within JS code. Bridges the
/// args back to DatumRefs, routes through try_invoke_js_handler, converts
/// the return value.
fn invoke_script_method(
    member_ref: &CastMemberRef,
    handler_name: &str,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let datum_args: Vec<DatumRef> = reserve_player_mut(|player| {
        args.iter().map(|v| js_value_to_datum_ref_from_jsvalue(player, v)).collect()
    });
    match try_invoke_js_handler(member_ref, handler_name, &datum_args, true) {
        Some(Ok(dref)) => {
            let v = reserve_player_mut(|player| datum_ref_to_js_value(player, &dref));
            Ok(v)
        }
        Some(Err(msg)) => Err(JsError::new(format!("{}: {}", handler_name, msg))),
        None => Err(JsError::new(format!(
            "script handler {} not found on {}:{}",
            handler_name, member_ref.cast_lib, member_ref.cast_member
        ))),
    }
}

/// Wrapper around `js_value_to_datum_ref` to break the existing function's
/// recursion (it's already defined further down).
fn js_value_to_datum_ref_from_jsvalue(player: &mut crate::player::DirPlayer, v: &JsValue) -> DatumRef {
    js_value_to_datum_ref(player, v)
}

fn datum_ref_to_string(player: &mut crate::player::DirPlayer, dref: &DatumRef) -> String {
    let d = player.allocator.get_datum(dref).clone();
    match d {
        Datum::String(s) => s,
        Datum::Symbol(s) => s.to_string(),
        Datum::Int(i) => i.to_string(),
        Datum::Float(f) => f.to_string(),
        _ => crate::player::datum_formatting::format_datum(dref, player),
    }
}

/// Convert a JsValue back into a DatumRef so the Lingo VM can consume it.
pub fn js_value_to_datum_ref(player: &mut crate::player::DirPlayer, v: &JsValue) -> DatumRef {
    let datum = match v {
        JsValue::Undefined => Datum::Void,
        JsValue::Null => Datum::Null,
        JsValue::Bool(b) => Datum::Int(if *b { 1 } else { 0 }),
        JsValue::Int(i) => Datum::Int(*i),
        JsValue::Number(n) => Datum::Float(*n),
        JsValue::String(s) => Datum::String((**s).clone()),
        JsValue::Array(a) => {
            // Convert each element to a DatumRef and wrap in a Lingo list.
            let items: std::collections::VecDeque<DatumRef> = a
                .borrow()
                .items
                .iter()
                .map(|x| js_value_to_datum_ref(player, x))
                .collect();
            Datum::List(crate::director::lingo::datum::DatumType::List, items, false)
        }
        JsValue::Object(o) => {
            // Objects carrying methods stay live (see `object_has_callable`).
            // Without a runtime in scope there's nothing to invoke against
            // later, so those fall through to the copy below.
            if object_has_callable(o) {
                if let Some(runtime) = CURRENT_RUNTIME.with(|s| s.borrow().last().cloned()) {
                    let id = register_js_object(&runtime, o);
                    return player.allocator.alloc_datum(Datum::JsObjectRef(id)).unwrap();
                }
            }
            let pairs: std::collections::VecDeque<crate::director::lingo::datum::PropListPair> = o
                .borrow()
                .props
                .iter()
                .map(|(k, val)| {
                    let key_dr = player.allocator.alloc_datum(Datum::Symbol(Symbol::from_str(k))).unwrap();
                    let val_dr = js_value_to_datum_ref(player, val);
                    (key_dr, val_dr)
                })
                .collect();
            Datum::PropList(pairs, false)
        }
        JsValue::Function(_) | JsValue::Native(_) => Datum::String("[Function]".to_string()),
        JsValue::Iterator(_) => Datum::String("[for-in iter]".to_string()),
        JsValue::DirectorRef(k) => {
            use crate::player::js_lingo::value::DirectorRefKind;
            match k {
                DirectorRefKind::Sprite(n) => Datum::SpriteRef(*n),
                DirectorRefKind::Member { cast_lib, cast_member } => {
                    Datum::CastMember(crate::player::cast_lib::CastMemberRef {
                        cast_lib: *cast_lib,
                        cast_member: *cast_member,
                    })
                }
            }
        }
    };
    player.allocator.alloc_datum(datum).unwrap()
}
