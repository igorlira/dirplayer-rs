// JavaScript value model.
//
// We don't reuse Director's `Datum` directly: JS has slightly different
// semantics (case-sensitive property names, prototype chain, falsiness,
// strict vs loose equality, NaN, +0/-0) that would force changes across
// hundreds of Lingo-side files. JsValue stays internal to the interpreter
// and converts to/from DatumRef only at the bridge: Lingo→JS calls coerce
// args at entry, JS→Director-builtin calls coerce at the call site.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use super::xdr::JsFunctionAtom;

/// A JavaScript runtime value. Compact and `Clone` — interior mutability
/// lives behind `Rc<RefCell<…>>` for the heap types (Object/Array/Function).
#[derive(Clone)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Number(f64),
    String(Rc<String>),
    Object(JsObjectRef),
    Array(JsArrayRef),
    Function(JsFunctionRef),
    /// Native callable — wraps a Rust closure so Director-runtime builtins
    /// (trace, sprite, member, …) can be exposed as JS functions.
    Native(Rc<NativeFn>),
    /// In-flight `for (k in obj)` enumeration state. Sits on the stack
    /// between FOR* opcode invocations. The first FOR* op observes
    /// `Undefined` in the iter slot and replaces it with an Iterator
    /// holding the snapshotted property keys; subsequent ops advance it.
    /// Matches SpiderMonkey 1.5's `prop_iterator_class` pattern -- see
    /// jsdmx/src/jsinterp.c JSOP_FORNAME / do_forinloop.
    Iterator(JsIteratorRef),
    /// Live handle to a Director-owned object (sprite channel, cast
    /// member). Property reads / writes round-trip through the existing
    /// `sprite_get_prop` / `sprite_set_prop` / cast-member handlers
    /// instead of being stored locally on a JsObject, so
    /// `sprite(3).locH = 100` actually moves the sprite on stage.
    DirectorRef(DirectorRefKind),
}

/// Variant of `JsValue::DirectorRef`. Kept as a plain enum (no Rc) because
/// the underlying handle (channel index / cast-member coordinates) is
/// `Copy`-cheap and Director state lives in the global player, not in this
/// value.
#[derive(Clone, Debug)]
pub enum DirectorRefKind {
    /// `sprite(N)` -- channel number.
    Sprite(i16),
    /// `member M` / `member(M of castLib C)` -- cast-member coordinates.
    Member { cast_lib: i32, cast_member: i32 },
}

pub type JsObjectRef = Rc<RefCell<JsObject>>;
pub type JsArrayRef = Rc<RefCell<JsArray>>;
pub type JsFunctionRef = Rc<JsFunction>;
pub type JsIteratorRef = Rc<RefCell<JsIterator>>;

/// Snapshot-style for-in iterator: captures the target's keys at first
/// iteration and walks the cursor through them. Object iteration yields
/// string keys; array iteration yields numeric indices (as strings, the
/// ECMA-262 way). Mutations to the underlying object during iteration
/// do NOT show up -- the spec actually permits this for objects, and
/// for arrays it matches modern JS engine behaviour for "added keys
/// after iteration started".
#[derive(Default)]
pub struct JsIterator {
    pub keys: Vec<String>,
    pub cursor: usize,
}

impl JsIterator {
    pub fn from_object(obj: &JsObject) -> Self {
        JsIterator {
            keys: obj.props.iter().map(|(k, _)| k.clone()).collect(),
            cursor: 0,
        }
    }

    pub fn from_array(arr: &JsArray) -> Self {
        JsIterator {
            keys: (0..arr.items.len()).map(|i| i.to_string()).collect(),
            cursor: 0,
        }
    }

    /// Pop the next key, or `None` when exhausted.
    pub fn next_key(&mut self) -> Option<String> {
        if self.cursor < self.keys.len() {
            let k = self.keys[self.cursor].clone();
            self.cursor += 1;
            Some(k)
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct JsObject {
    /// Case-sensitive property map. Insertion order is preserved (matches
    /// SpiderMonkey 1.5 Object.keys() observable order).
    pub props: Vec<(String, JsValue)>,
    pub proto: Option<JsObjectRef>,
    pub class_name: &'static str,
}

impl JsObject {
    pub fn new() -> Self {
        JsObject { props: Vec::new(), proto: None, class_name: "Object" }
    }

    pub fn get_own(&self, key: &str) -> Option<&JsValue> {
        self.props.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn set_own(&mut self, key: &str, value: JsValue) {
        for (k, v) in self.props.iter_mut() {
            if k == key {
                *v = value;
                return;
            }
        }
        self.props.push((key.to_string(), value));
    }

    pub fn has_own(&self, key: &str) -> bool {
        self.props.iter().any(|(k, _)| k == key)
    }
}

#[derive(Default)]
pub struct JsArray {
    pub items: Vec<JsValue>,
}

impl JsArray {
    pub fn new() -> Self {
        JsArray { items: Vec::new() }
    }
}

/// A user-defined JavaScript function. `script` holds the XDR-decoded body;
/// `atom` retains the parameter / local name table so the interpreter can
/// resolve variable references back to slots.
///
/// `captured_scope` records the lexical scope object that was in effect at
/// the moment this function value was CREATED -- the scope chain link a
/// closure needs to see outer locals. `None` for top-level functions
/// (those just fall back to the program scope).
pub struct JsFunction {
    pub atom: Rc<JsFunctionAtom>,
    pub captured_scope: Option<JsObjectRef>,
}

pub struct NativeFn {
    pub name: &'static str,
    pub call: Box<dyn Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static>,
}

#[derive(Debug)]
pub struct JsError {
    pub message: String,
    /// The original `throw` operand, if this error came from user JS via
    /// `throw expr`. Native errors (uncallable, undefined arg, opcode
    /// failure) leave this as `None` and the catch handler sees a
    /// freshly-constructed `Error`-style placeholder. Lets `try { ...
    /// throw {code: 42}; } catch (e) { e.code }` round-trip properly.
    pub thrown: Option<JsValue>,
}

impl JsError {
    pub fn new(s: impl Into<String>) -> Self {
        JsError { message: s.into(), thrown: None }
    }

    /// Construct an error wrapping an explicit `throw` value. The message
    /// is rendered from the value so cross-frame propagation (when no
    /// catch handler matches) still surfaces something readable.
    pub fn from_thrown(v: JsValue) -> Self {
        JsError { message: format!("uncaught: {}", v.to_string()), thrown: Some(v) }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JsError: {}", self.message)
    }
}

impl JsValue {
    /// ECMA-262 §9.2 ToBoolean.
    pub fn to_bool(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Int(i) => *i != 0,
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::String(s) => !s.is_empty(),
            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) | JsValue::Native(_) => true,
            JsValue::Iterator(_) => true,
            JsValue::DirectorRef(_) => true,
        }
    }

    /// ECMA-262 §9.3 ToNumber. Loose, matches what SpiderMonkey 1.5 did.
    pub fn to_number(&self) -> f64 {
        match self {
            JsValue::Undefined => f64::NAN,
            JsValue::Null => 0.0,
            JsValue::Bool(b) => if *b { 1.0 } else { 0.0 },
            JsValue::Int(i) => *i as f64,
            JsValue::Number(n) => *n,
            JsValue::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) | JsValue::Native(_) => f64::NAN,
            JsValue::Iterator(_) => f64::NAN,
            // Sprite / member refs as scalar: Director's Lingo converts to
            // the channel number / cast member number. Mirror that so
            // `sprite(3) + 1 == 4` matches Lingo intuition.
            JsValue::DirectorRef(DirectorRefKind::Sprite(n)) => *n as f64,
            JsValue::DirectorRef(DirectorRefKind::Member { cast_member, .. }) => *cast_member as f64,
        }
    }

    /// ECMA-262 §9.5 ToInt32.
    pub fn to_int32(&self) -> i32 {
        let n = self.to_number();
        if n.is_nan() || n.is_infinite() {
            return 0;
        }
        let pos = n.trunc();
        let modulo = pos.rem_euclid(4294967296.0);
        let as_u32 = modulo as u32;
        as_u32 as i32
    }

    /// ECMA-262 §9.8 ToString (light version).
    pub fn to_string(&self) -> String {
        match self {
            JsValue::Undefined => "undefined".into(),
            JsValue::Null => "null".into(),
            JsValue::Bool(b) => b.to_string(),
            JsValue::Int(i) => i.to_string(),
            JsValue::Number(n) => {
                if n.is_nan() { "NaN".into() }
                else if n.is_infinite() { if *n > 0.0 { "Infinity".into() } else { "-Infinity".into() } }
                else if *n == 0.0 { "0".into() }
                else if *n == n.trunc() && n.abs() < 1e21 { format!("{}", *n as i64) }
                else { format!("{}", n) }
            }
            JsValue::String(s) => (**s).clone(),
            JsValue::Object(_) => "[object Object]".into(),
            JsValue::Array(a) => {
                a.borrow().items.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
            }
            JsValue::Function(f) => {
                let name = f.atom.name.as_deref().unwrap_or("anonymous");
                format!("function {}() {{ [native code] }}", name)
            }
            JsValue::Native(f) => format!("function {}() {{ [native code] }}", f.name),
            JsValue::Iterator(_) => "[for-in iterator]".into(),
            JsValue::DirectorRef(DirectorRefKind::Sprite(n)) => format!("(sprite {})", n),
            JsValue::DirectorRef(DirectorRefKind::Member { cast_lib, cast_member }) => {
                format!("(member {} of castLib {})", cast_member, cast_lib)
            }
        }
    }

    pub fn type_of(&self) -> &'static str {
        match self {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object",
            JsValue::Bool(_) => "boolean",
            JsValue::Int(_) | JsValue::Number(_) => "number",
            JsValue::String(_) => "string",
            JsValue::Object(_) | JsValue::Array(_) => "object",
            JsValue::Function(_) | JsValue::Native(_) => "function",
            JsValue::Iterator(_) => "object",
            JsValue::DirectorRef(_) => "object",
        }
    }
}

impl fmt::Debug for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsValue::Undefined => write!(f, "undefined"),
            JsValue::Null => write!(f, "null"),
            JsValue::Bool(b) => write!(f, "{}", b),
            JsValue::Int(i) => write!(f, "{}", i),
            JsValue::Number(n) => write!(f, "{}", n),
            JsValue::String(s) => write!(f, "{:?}", &**s),
            JsValue::Object(_) => write!(f, "[object]"),
            JsValue::Array(a) => write!(f, "[{}]", a.borrow().items.iter().map(|v| format!("{:?}", v)).collect::<Vec<_>>().join(",")),
            JsValue::Function(fun) => write!(f, "[fn {}]", fun.atom.name.as_deref().unwrap_or("?")),
            JsValue::Native(fun) => write!(f, "[native fn {}]", fun.name),
            JsValue::Iterator(it) => {
                let it = it.borrow();
                write!(f, "[for-in iter {}/{}]", it.cursor, it.keys.len())
            }
            JsValue::DirectorRef(k) => write!(f, "{:?}", k),
        }
    }
}
