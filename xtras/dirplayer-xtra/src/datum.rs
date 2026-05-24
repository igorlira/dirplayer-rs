use serde::{Deserialize, Serialize};

/// Mirror of the WIT `datum` variant.  Used as the Rust representation for all
/// values crossing the plugin↔host boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Datum {
    Void,
    Int {
        value: i32,
    },
    Float {
        value: f64,
    },
    String {
        value: std::string::String,
    },
    Bool {
        value: bool,
    },
    Symbol {
        value: std::string::String,
    },
    List {
        value: Vec<Datum>,
    },
    #[serde(rename = "prop-list")]
    PropList {
        value: Vec<[Datum; 2]>,
    },
    Point {
        value: [f64; 2],
    },
    Rect {
        value: [f64; 4],
    },
    #[serde(rename = "xtra-ref")]
    XtraRef {
        value: XtraRefValue,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XtraRefValue {
    pub name: std::string::String,
    pub id: u32,
}

impl Datum {
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Datum::String { value } => Some(value),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Datum::Int { value } => Some(*value),
            Datum::Float { value } => Some(*value as i32),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Datum::Float { value } => Some(*value),
            Datum::Int { value } => Some(*value as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Datum::Bool { value } => Some(*value),
            Datum::Int { value } => Some(*value != 0),
            _ => None,
        }
    }

    pub fn as_symbol(&self) -> Option<&str> {
        match self {
            Datum::Symbol { value } => Some(value),
            _ => None,
        }
    }

    pub fn is_void(&self) -> bool {
        matches!(self, Datum::Void)
    }
}

impl From<i32> for Datum {
    fn from(v: i32) -> Self {
        Datum::Int { value: v }
    }
}

impl From<f64> for Datum {
    fn from(v: f64) -> Self {
        Datum::Float { value: v }
    }
}

impl From<std::string::String> for Datum {
    fn from(v: std::string::String) -> Self {
        Datum::String { value: v }
    }
}

impl From<&str> for Datum {
    fn from(v: &str) -> Self {
        Datum::String {
            value: v.to_string(),
        }
    }
}

impl From<bool> for Datum {
    fn from(v: bool) -> Self {
        Datum::Bool { value: v }
    }
}

/// Parse a JSON string produced by the host into a Datum.
pub fn datum_from_json(s: &str) -> Result<Datum, std::string::String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

/// Serialize a Datum to a JSON string for returning to the host.
pub fn datum_to_json(d: &Datum) -> std::string::String {
    serde_json::to_string(d).unwrap_or_else(|_| r#"{"type":"void"}"#.to_string())
}

/// Parse a JSON array of Datums (the args list passed by the host).
pub fn args_from_json(s: &str) -> Result<Vec<Datum>, std::string::String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}
