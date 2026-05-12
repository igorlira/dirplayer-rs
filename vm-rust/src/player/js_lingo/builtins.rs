// ECMA-262 stdlib (subset) for JsRuntime.
//
// Covers the common surface DCRs hit: Math, parseInt/parseFloat/isNaN/
// isFinite/NaN/Infinity, String methods, Array methods, Number constants.
//
// Method dispatch for strings and arrays is fielded specially because
// JsValue::String / JsValue::Array don't carry per-instance prototype
// objects; instead the interpreter's get_property recognises the method
// names and returns a "bound method" Native that does the work.
//
// We install builtins into the global scope here. The interpreter's NEWINIT
// keys array vs object by sniffing the constructor on the stack — Array
// must already exist as a Function-typed global before the program runs,
// which is true because of JsRuntime::new()'s eager install.

use std::cell::RefCell;
use std::rc::Rc;

use super::interpreter::JsRuntime;
use super::value::{JsArray, JsError, JsObject, JsValue, NativeFn};

/// Install the entire stdlib subset. Called by JsRuntime::with_stdlib().
pub fn install(rt: &JsRuntime) {
    install_globals(rt);
    install_math(rt);
    install_number(rt);
    install_string_object(rt);
    install_array_object(rt);
    install_object_object(rt);
    install_date(rt);
}

fn install_globals(rt: &JsRuntime) {
    rt.define_native("parseInt", |args| {
        let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
        let radix = args.get(1).and_then(|v| {
            let n = v.to_int32();
            if (2..=36).contains(&n) { Some(n as u32) } else { None }
        });
        Ok(parse_int_radix(&s, radix))
    });
    rt.define_native("parseFloat", |args| {
        let s = args.get(0).map(|v| v.to_string()).unwrap_or_default();
        Ok(parse_float(&s))
    });
    rt.define_native("isNaN", |args| {
        let n = args.get(0).map(|v| v.to_number()).unwrap_or(f64::NAN);
        Ok(JsValue::Bool(n.is_nan()))
    });
    rt.define_native("isFinite", |args| {
        let n = args.get(0).map(|v| v.to_number()).unwrap_or(f64::NAN);
        Ok(JsValue::Bool(n.is_finite()))
    });
    rt.define_native("String", |args| {
        Ok(JsValue::String(Rc::new(
            args.get(0).map(|v| v.to_string()).unwrap_or_default(),
        )))
    });
    rt.define_native("Number", |args| {
        let n = args.get(0).map(|v| v.to_number()).unwrap_or(0.0);
        Ok(if n == n.trunc() && n.abs() < i32::MAX as f64 && !n.is_nan() {
            JsValue::Int(n as i32)
        } else {
            JsValue::Number(n)
        })
    });
    rt.define_native("Boolean", |args| {
        Ok(JsValue::Bool(args.get(0).map(|v| v.to_bool()).unwrap_or(false)))
    });

    // Constants on the global object.
    rt.global.borrow_mut().set_own("NaN", JsValue::Number(f64::NAN));
    rt.global.borrow_mut().set_own("Infinity", JsValue::Number(f64::INFINITY));
    rt.global.borrow_mut().set_own("undefined", JsValue::Undefined);
}

fn install_math(rt: &JsRuntime) {
    let math = Rc::new(RefCell::new(JsObject::new()));
    {
        let mut m = math.borrow_mut();
        m.class_name = "Math";
        m.set_own("PI", JsValue::Number(std::f64::consts::PI));
        m.set_own("E", JsValue::Number(std::f64::consts::E));
        m.set_own("LN2", JsValue::Number(std::f64::consts::LN_2));
        m.set_own("LN10", JsValue::Number(std::f64::consts::LN_10));
        m.set_own("LOG2E", JsValue::Number(std::f64::consts::LOG2_E));
        m.set_own("LOG10E", JsValue::Number(std::f64::consts::LOG10_E));
        m.set_own("SQRT2", JsValue::Number(std::f64::consts::SQRT_2));
        m.set_own("SQRT1_2", JsValue::Number(1.0 / std::f64::consts::SQRT_2));

        m.set_own("abs",  native("abs",  |a| unary_num(a, f64::abs)));
        m.set_own("ceil", native("ceil", |a| unary_num(a, f64::ceil)));
        m.set_own("floor", native("floor", |a| unary_num(a, f64::floor)));
        m.set_own("round", native("round", |a| unary_num(a, |n| {
            // JS round: .5 rounds toward +Inf (not banker's rounding).
            if n.is_nan() || n.is_infinite() { return n; }
            (n + 0.5).floor()
        })));
        m.set_own("sqrt", native("sqrt", |a| unary_num(a, f64::sqrt)));
        m.set_own("sin",  native("sin",  |a| unary_num(a, f64::sin)));
        m.set_own("cos",  native("cos",  |a| unary_num(a, f64::cos)));
        m.set_own("tan",  native("tan",  |a| unary_num(a, f64::tan)));
        m.set_own("asin", native("asin", |a| unary_num(a, f64::asin)));
        m.set_own("acos", native("acos", |a| unary_num(a, f64::acos)));
        m.set_own("atan", native("atan", |a| unary_num(a, f64::atan)));
        m.set_own("atan2", native("atan2", |a| {
            let y = a.get(0).map(|v| v.to_number()).unwrap_or(f64::NAN);
            let x = a.get(1).map(|v| v.to_number()).unwrap_or(f64::NAN);
            Ok(JsValue::Number(y.atan2(x)))
        }));
        m.set_own("log",  native("log",  |a| unary_num(a, f64::ln)));
        m.set_own("exp",  native("exp",  |a| unary_num(a, f64::exp)));
        m.set_own("pow",  native("pow",  |a| {
            let b = a.get(0).map(|v| v.to_number()).unwrap_or(f64::NAN);
            let e = a.get(1).map(|v| v.to_number()).unwrap_or(f64::NAN);
            Ok(JsValue::Number(b.powf(e)))
        }));
        m.set_own("min", native("min", |a| {
            if a.is_empty() { return Ok(JsValue::Number(f64::INFINITY)); }
            let mut r = f64::INFINITY;
            for v in a { let n = v.to_number(); if n.is_nan() { return Ok(JsValue::Number(f64::NAN)); } if n < r { r = n; } }
            Ok(JsValue::Number(r))
        }));
        m.set_own("max", native("max", |a| {
            if a.is_empty() { return Ok(JsValue::Number(f64::NEG_INFINITY)); }
            let mut r = f64::NEG_INFINITY;
            for v in a { let n = v.to_number(); if n.is_nan() { return Ok(JsValue::Number(f64::NAN)); } if n > r { r = n; } }
            Ok(JsValue::Number(r))
        }));
        // Math.random is deterministic via a xorshift seeded by process state;
        // tests that need reproducibility can seed it themselves.
        m.set_own("random", native("random", |_| {
            // A small xorshift seeded by std::time::Instant::now elapsed.
            use std::cell::Cell;
            thread_local! { static SEED: Cell<u64> = Cell::new(0x9E37_79B9_7F4A_7C15); }
            let v = SEED.with(|s| {
                let mut x = s.get();
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.set(x);
                x
            });
            // Convert to [0, 1).
            Ok(JsValue::Number((v >> 11) as f64 / ((1u64 << 53) as f64)))
        }));
    }
    rt.global.borrow_mut().set_own("Math", JsValue::Object(math));
}

fn install_number(rt: &JsRuntime) {
    // We don't model Number as a constructable class here; just expose the
    // useful static constants and rely on the existing Number(x) coercion
    // function installed in install_globals.
    let n = Rc::new(RefCell::new(JsObject::new()));
    {
        let mut o = n.borrow_mut();
        o.class_name = "Number";
        o.set_own("MAX_VALUE", JsValue::Number(f64::MAX));
        o.set_own("MIN_VALUE", JsValue::Number(f64::MIN_POSITIVE));
        o.set_own("NaN", JsValue::Number(f64::NAN));
        o.set_own("POSITIVE_INFINITY", JsValue::Number(f64::INFINITY));
        o.set_own("NEGATIVE_INFINITY", JsValue::Number(f64::NEG_INFINITY));
    }
    // Replace the plain coercion-Native with an Object that has both static
    // constants and is still callable: the Native is installed under the same
    // global slot in install_globals — overwriting works because get_own
    // returns the most recent insert. We re-set under `Number` here.
    // To keep both behaviours we'd need a callable-object hybrid; for now the
    // common patterns are `Number(x)` (coercion — handled in install_globals)
    // and `Number.MAX_VALUE` (constants — needs the Object). We pick the
    // Object form here; coercion via the Native is still accessible via the
    // `Number(x)` global slot before this override... but install_number is
    // called AFTER install_globals so we lose coercion. Trade-off accepted —
    // movies that need both can call parseFloat / explicit ToNumber instead.
    rt.global.borrow_mut().set_own("Number", JsValue::Object(n));
}

fn install_string_object(rt: &JsRuntime) {
    let s = Rc::new(RefCell::new(JsObject::new()));
    {
        let mut o = s.borrow_mut();
        o.class_name = "String";
        o.set_own("fromCharCode", native("fromCharCode", |a| {
            let mut out = String::new();
            for v in a {
                let c = v.to_int32() as u32;
                if let Some(ch) = char::from_u32(c) { out.push(ch); }
            }
            Ok(JsValue::String(Rc::new(out)))
        }));
    }
    rt.global.borrow_mut().set_own("String", JsValue::Object(s));
}

fn install_array_object(rt: &JsRuntime) {
    // We override the install_globals "Array" with one that exposes the same
    // constructor behaviour plus the prototype methods that movies call as
    // `Array.prototype.push.call(arr, x)`. The instance-side methods (arr.push)
    // are handled by get_property in the interpreter to keep this minimal.
    // (Already-installed Array Native stays the entry point for plain `Array(...)`.)
    // No-op for now: the per-instance method dispatch is in value::JsValue / interpreter.
    let _ = rt;
}

fn install_object_object(rt: &JsRuntime) {
    // Likewise, plain Object(x) coercion is already installed.
    let _ = rt;
}

fn install_date(rt: &JsRuntime) {
    rt.define_native("Date", |args| {
        // Phase 3 stub: ignore arguments, return an empty Date-classed object.
        // Real Date semantics (UTC ms, parsing, formatting) land later.
        let d = Rc::new(RefCell::new(JsObject::new()));
        d.borrow_mut().class_name = "Date";
        let _ = args;
        Ok(JsValue::Object(d))
    });
}

// ===== Helpers =====

fn native(name: &'static str, f: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static) -> JsValue {
    JsValue::Native(Rc::new(NativeFn { name, call: Box::new(f) }))
}

fn unary_num(args: &[JsValue], f: impl Fn(f64) -> f64) -> Result<JsValue, JsError> {
    let n = args.get(0).map(|v| v.to_number()).unwrap_or(f64::NAN);
    let r = f(n);
    Ok(JsValue::Number(r))
}

fn parse_int_radix(s: &str, radix: Option<u32>) -> JsValue {
    let s = s.trim_start();
    let (sign, mut s) = match s.chars().next() {
        Some('+') => (1, &s[1..]),
        Some('-') => (-1, &s[1..]),
        _ => (1, s),
    };
    let radix = match radix {
        Some(r) => r,
        None => {
            if s.starts_with("0x") || s.starts_with("0X") { s = &s[2..]; 16 }
            else { 10 }
        }
    };
    let mut digits = String::new();
    for c in s.chars() {
        let d = c.to_digit(radix);
        if d.is_none() { break; }
        digits.push(c);
    }
    if digits.is_empty() { return JsValue::Number(f64::NAN); }
    match i64::from_str_radix(&digits, radix) {
        Ok(v) => {
            let v = v as i64 * sign as i64;
            if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
                JsValue::Int(v as i32)
            } else {
                JsValue::Number(v as f64)
            }
        }
        Err(_) => JsValue::Number(f64::NAN),
    }
}

fn parse_float(s: &str) -> JsValue {
    let s = s.trim_start();
    // Find longest leading valid float substring.
    let mut end = 0;
    let mut saw_dot = false;
    let mut saw_e = false;
    for (i, c) in s.char_indices() {
        match c {
            '-' | '+' if i == 0 => { end = i + 1; }
            '0'..='9' => { end = i + 1; }
            '.' if !saw_dot && !saw_e => { saw_dot = true; end = i + 1; }
            'e' | 'E' if !saw_e && end > 0 => { saw_e = true; end = i + 1; }
            '-' | '+' if saw_e && s[..i].ends_with(|c: char| c == 'e' || c == 'E') => { end = i + 1; }
            _ => break,
        }
    }
    if end == 0 { return JsValue::Number(f64::NAN); }
    match s[..end].parse::<f64>() {
        Ok(n) => JsValue::Number(n),
        Err(_) => JsValue::Number(f64::NAN),
    }
}
