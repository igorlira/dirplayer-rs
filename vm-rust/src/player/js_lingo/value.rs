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
}

pub type JsObjectRef = Rc<RefCell<JsObject>>;
pub type JsArrayRef = Rc<RefCell<JsArray>>;
pub type JsFunctionRef = Rc<JsFunction>;

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
pub struct JsFunction {
    pub atom: Rc<JsFunctionAtom>,
}

pub struct NativeFn {
    pub name: &'static str,
    pub call: Box<dyn Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static>,
}

#[derive(Debug)]
pub struct JsError {
    pub message: String,
}

impl JsError {
    pub fn new(s: impl Into<String>) -> Self {
        JsError { message: s.into() }
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
        }
    }
}
