use crate::env::Env;
use crate::error::LispError;
use crate::future::BlinkFuture;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::hash::{Hash, Hasher};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct SourcePos {
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for SourcePos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.col)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct SourceRange {
    pub start: SourcePos,
    pub end: SourcePos,
}

impl std::fmt::Display for SourceRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[derive(Clone, Debug)]
pub struct LispNode {
    pub value: Value,
    pub pos: Option<SourceRange>,
}

impl fmt::Display for LispNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[derive(Clone, Debug)]
pub struct BlinkValue(pub Arc<RwLock<LispNode>>);

impl Deref for BlinkValue {
    type Target = Arc<RwLock<LispNode>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Hash for BlinkValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.read().value {
            Value::Number(n) => {
                        // Handle float/int equality: 1.0 == 1
                        if n.fract() == 0.0 && n.is_finite() {
                            // Hash as integer if it's a whole number
                            (*n as i64).hash(state);
                        } else {
                            n.to_bits().hash(state);
                        }
                    }
            Value::Str(s) => s.hash(state),
            Value::Symbol(s) => s.hash(state),
            Value::Keyword(k) => {
                        ":".hash(state);
                        k.hash(state);
                    }
            Value::List(items) => {
                        "list".hash(state);
                        for item in items {
                            item.hash(state);
                        }
                    }
            Value::Bool(b) => b.hash(state),
            Value::Vector(blink_values) => {
                "vector".hash(state); // Type discriminator
                blink_values.len().hash(state); // Length for efficiency
                for item in blink_values {
                    item.hash(state);
                }
            }
            
            Value::Map(hash_map) => {
                "map".hash(state);
                hash_map.len().hash(state);
                // Maps are unordered, so we need order-independent hashing
                let mut pairs: Vec<_> = hash_map.iter().collect();
                pairs.sort_by(|a, b| {
                    // Sort by hash of key for deterministic ordering
                    let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
                    let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
                    a.0.hash(&mut hasher_a);
                    b.0.hash(&mut hasher_b);
                    hasher_a.finish().cmp(&hasher_b.finish())
                });
                for (k, v) in pairs {
                    k.hash(state);
                    v.hash(state);
                }
            }
            
            Value::NativeFunc(f) => {
                "native-func".hash(state);
                // Function pointers can be hashed by address
                (*f as *const fn(Vec<BlinkValue>) -> Result<BlinkValue, String>).hash(state);
            }
            
            Value::FuncUserDefined { params, body, env } => {
                "user-func".hash(state);
                params.hash(state);
                body.hash(state);
                // For env, we could hash the env's ID or skip it
                // Skipping env means functions with same params/body are equal
                // even if captured in different environments
            }
            
            Value::ModuleReference { module, symbol } => {
                "module-ref".hash(state);
                module.hash(state);
                symbol.hash(state);
            }
            
            Value::Macro { params, body, env, is_variadic } => {
                "macro".hash(state);
                params.hash(state);
                body.hash(state);
                is_variadic.hash(state);
                // Same env consideration as functions
            }
            
            Value::Future(blink_future) => {
                "future".hash(state);
                // figure out how to hash futures maybe an id?
                todo!();
            }
            
            Value::Error(lisp_error) => {
                "error".hash(state);
                // Hash the error message/type
                format!("{:?}", lisp_error).hash(state);
            }
            
            Value::Nil => {
                "nil".hash(state);
            }
        }
    }
}

impl PartialEq for BlinkValue {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (&self.read().value, &other.read().value) {
            // Same types
            (Number(a), Number(b)) => {
                // Handle float/int equality: 1.0 == 1
                if a.fract() == 0.0 && b.fract() == 0.0 {
                    *a as i64 == *b as i64
                } else {
                    a == b
                }
            }
            (Bool(a), Bool(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Symbol(a), Symbol(b)) => a == b,
            (Keyword(a), Keyword(b)) => a == b,
            (Nil, Nil) => true,
            
            (List(a), List(b)) => a == b,  // Recursive
            (Vector(a), Vector(b)) => a == b,
            (Map(a), Map(b)) => a == b,
            
            (NativeFunc(a), NativeFunc(b)) => {
                // Function pointer equality
                std::ptr::eq(a as *const _, b as *const _)
            }
            
            (FuncUserDefined { params: p1, body: b1, .. }, 
             FuncUserDefined { params: p2, body: b2, .. }) => {
                // Functions equal if same params and body
                // (ignoring environment for now)
                p1 == p2 && b1 == b2
            }
            
            (ModuleReference { module: m1, symbol: s1 }, 
             ModuleReference { module: m2, symbol: s2 }) => {
                m1 == m2 && s1 == s2
            }
            
            (Macro { params: p1, body: b1, is_variadic: v1, .. },
             Macro { params: p2, body: b2, is_variadic: v2, .. }) => {
                p1 == p2 && b1 == b2 && v1 == v2
            }
            
            // Different types are never equal
            _ => false
        }
    }
}

impl Eq for BlinkValue {}

#[allow(dead_code)]
impl BlinkValue {
    pub fn is_nil(&self) -> bool {
        matches!(self.read().value, Value::Nil)
    }

    pub fn type_tag(&self) -> &'static str {
        self.read().value.type_tag()
    }

    pub fn as_str(&self) -> Option<String> {
        if let Value::Str(s) = &self.read().value {
            Some(s.clone())
        } else {
            None
        }
    }

    pub fn error(err: LispError) -> Self {
        BlinkValue::from(Value::Error(err))
    }

    pub fn is_error(&self) -> bool {
        matches!(&self.read().value, Value::Error(_))
    }

    pub fn to_string_repr(&self) -> String {
        format!("{:?}", self.read().value)
    }
}

#[derive(Clone)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Str(String),
    Symbol(String),
    Keyword(String),    
    List(Vec<BlinkValue>),
    Vector(Vec<BlinkValue>),
    Map(HashMap<BlinkValue, BlinkValue>),
    NativeFunc(fn(Vec<BlinkValue>) -> Result<BlinkValue, String>), // Rust-native functions
    FuncUserDefined {
        params: Vec<String>,
        body: Vec<BlinkValue>,
        env: Arc<RwLock<Env>>, // closure capture
    },
    // New type for imported symbols
    ModuleReference {
        module: String,
        symbol: String,
    },
    Macro {
        params: Vec<String>,
        body: Vec<BlinkValue>,
        env: Arc<RwLock<Env>>,
        is_variadic: bool,
    },
    Future(BlinkFuture),
    Error(LispError),
    Nil,
}
impl Value {
    pub fn type_tag(&self) -> &'static str {
        match self {
            Value::Number(_) => "number",
            Value::Bool(_) => "bool",
            Value::Str(_) => "string",
            Value::Symbol(_) => "symbol",
            Value::Keyword(_) => "keyword",
            Value::List(_) => "list",
            Value::Vector(_) => "vector",
            Value::Map(_) => "map",
            Value::FuncUserDefined { .. } => "fn",
            Value::NativeFunc(_) => "native-fn",
            Value::ModuleReference { .. } => "imported-symbol",
            Value::Nil => "nil",
            Value::Macro { .. } => "macro",
            Value::Future(_) => "future",
            Value::Error(_) => "error",
        }
    }
    
}

use std::fmt;

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "\"{}\"", s),
            Value::Symbol(s) => write!(f, "{}", s),
            Value::Keyword(s) => write!(f, ":{}", s),
            Value::List(xs) => write!(f, "({} items)", xs.len()),
            Value::Vector(xs) => write!(f, "[{} items]", xs.len()),
            Value::Map(m) => write!(f, "{{map with {} keys}}", m.len()),
            Value::FuncUserDefined { params, .. } => write!(f, "#<fn {:?}>", params),
            Value::NativeFunc(_) => write!(f, "#<native-fn>"),
            Value::ModuleReference { .. } => write!(f, "#<imported-symbol>"),
            Value::Macro { .. } => write!(f, "#<macro>"),
            Value::Future(_) => write!(f, "#<future>"),
            Value::Error(_) => write!(f, "#<error>"),
            Value::Nil => write!(f, "nil"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "\"{}\"", s),
            Value::Symbol(s) => write!(f, "{}", s),
            Value::Keyword(s) => write!(f, ":{}", s),
            Value::List(xs) => {
                write!(f, "(")?;
                for (i, val) in xs.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", val.read())?;
                }
                write!(f, ")")
            }
            Value::Vector(xs) => {
                write!(f, "[")?;
                for (i, val) in xs.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", val.read())?;
                }
                write!(f, "]")
            }
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{} {}", k, v.read())?;
                }
                write!(f, "}}")
            }
            Value::FuncUserDefined { .. } => write!(f, "#<fn>"),
            Value::NativeFunc(_) => write!(f, "#<native-fn>"),
            Value::ModuleReference { .. } => write!(f, "#<imported-symbol>"),
            Value::Macro { .. } => write!(f, "#<macro>"),
            Value::Future(_) => write!(f, "#<future>"),
            Value::Error(_) => write!(f, "#<error>"),
            Value::Nil => write!(f, "nil"),
        }
    }
}

impl fmt::Display for BlinkValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.read().value)
    }
}

// --- Value with position ---
pub fn num_from_token(token: &str, start: SourcePos) -> BlinkValue {
    let end = SourcePos {
        line: start.line,
        col: start.col + token.len(),
    };
    let value = token.parse::<f64>().unwrap(); // you should have validated before

    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Number(value),
        pos: Some(SourceRange { start, end }),
    })))
}


pub fn bool_val_at(b: bool, start: Option<SourcePos>) -> BlinkValue {
    let pos = start.map(|start| {
        let len = if b { 4 } else { 5 }; // "true" or "false"
        let end = SourcePos {
            line: start.line,
            col: start.col + len,
        };
        SourceRange { start, end }
    });

    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Bool(b),
        pos,
    })))
}

pub fn str_val_at(s: &str, start: Option<SourcePos>) -> BlinkValue {
    let pos = start.map(|start| {
        let end = SourcePos {
            line: start.line,
            col: start.col + s.len(),
        };
        SourceRange { start, end }
    });
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Str(s.to_string()),
        pos,
    })))
}

pub fn sym_at(name: &str, start: Option<SourcePos>) -> BlinkValue {
    let pos = start.map(|start| {
        let end = SourcePos {
            line: start.line,
            col: start.col + name.len(), // or whatever length you need
        };
        SourceRange {
            start,
            end,
        }
    });

    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Symbol(name.to_string()),
        pos,
    })))
}


pub fn keyword_at(k: &str, start: Option<SourcePos>) -> BlinkValue {
    let pos = start.map(|start| {
        let end = SourcePos {
            line: start.line,
            col: start.col + k.len() + 1,
        };
        SourceRange { start, end }
    });
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Keyword(k.to_string()),
        pos,
    })))
}

pub fn list_val_at(xs: Vec<BlinkValue>, start: Option<SourcePos>) -> BlinkValue {
    let end = xs
        .last()
        .and_then(|v| v.read().pos.as_ref().map(|r| r.end.clone()))
        .map(|mut pos| {
            pos.col += 1; // include closing paren
            pos
        });

    let range = match (start, end) {
        (Some(start), Some(end)) => Some(SourceRange { start, end }),
        _ => None,
    };

    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::List(xs),
        pos: range,
    })))
}




pub fn vector_val_at(xs: Vec<BlinkValue>, pos: Option<SourcePos>) -> BlinkValue {
    let pos = pos.map(|pos| {
        let end = SourcePos {
            line: pos.line,
            col: pos.col + xs.len(),
        };
        SourceRange { start: pos, end }
    });
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Vector(xs),
        pos,
    })))
}
pub fn num_at(n: f64, start: Option<SourcePos>) -> BlinkValue {
    let end = start.as_ref().map(|s| SourcePos {
        line: s.line,
        col: s.col + n.to_string().len(), // crude width estimation
    });

    let range = match (start, end) {
        (Some(start), Some(end)) => Some(SourceRange { start, end }),
        _ => None,
    };

    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Number(n),
        pos: range,
    })))
}



pub fn map_val_at(m: HashMap<BlinkValue, BlinkValue>, pos: Option<SourcePos>) -> BlinkValue {
    let pos = pos.map(|pos| {
        let end = SourcePos {
            line: pos.line,
            col: pos.col + m.len(),
        };
        SourceRange { start: pos, end }
    });
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Map(m),
        pos,
    })))
}

pub fn nil_at(start: Option<SourcePos>) -> BlinkValue {
    let pos = start.map(|start| {
        let end = SourcePos {
            line: start.line,
            col: start.col + 3,
        };
        SourceRange { start, end }
    });
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Nil,
        pos,
    })))
}

pub fn num(n: f64) -> BlinkValue {
    num_at(n, None)
}

pub fn bool_val(b: bool) -> BlinkValue {
    bool_val_at(b, None)
}

pub fn str_val(s: &str) -> BlinkValue {
    str_val_at(s, None)
}

pub fn sym(s: &str) -> BlinkValue {
    sym_at(s, None)
}

#[allow(dead_code)]
pub fn keyword(k: &str) -> BlinkValue {
    keyword_at(k, None)
}

pub fn nil() -> BlinkValue {
    nil_at(None)
}

pub fn list_val(xs: Vec<BlinkValue>) -> BlinkValue {
    list_val_at(xs, None)
}

use std::convert::From;

impl From<&str> for BlinkValue {
    fn from(s: &str) -> Self {
        str_val(s)
    }
}

impl From<String> for BlinkValue {
    fn from(s: String) -> Self {
        str_val(&s)
    }
}

impl From<f64> for BlinkValue {
    fn from(n: f64) -> Self {
        num(n)
    }
}

impl From<bool> for BlinkValue {
    fn from(b: bool) -> Self {
        bool_val(b)
    }
}

impl From<Value> for BlinkValue {
    fn from(val: Value) -> Self {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: val,
            pos: None,
        })))
    }
}
