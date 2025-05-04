use crate::env::Env;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LispNode {
    pub value: Value,
    pub pos: Option<SourcePos>,
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
    Map(HashMap<String, BlinkValue>),
    NativeFunc(fn(Vec<BlinkValue>) -> Result<BlinkValue, String>), // Rust-native functions
    FuncUserDefined {
        params: Vec<String>,
        body: Vec<BlinkValue>,
        env: Arc<RwLock<Env>>, // closure capture
    },

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
            Value::Nil => "nil",
        }
    }
}

use crate::error::SourcePos;
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
pub fn num_at(n: f64, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Number(n),
        pos,
    })))
}

pub fn bool_val_at(b: bool, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Bool(b),
        pos,
    })))
}

pub fn str_val_at(s: &str, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Str(s.to_string()),
        pos,
    })))
}

pub fn sym_at(s: &str, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Symbol(s.to_string()),
        pos,
    })))
}

pub fn keyword_at(k: &str, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Keyword(k.to_string()),
        pos,
    })))
}

pub fn list_val_at(xs: Vec<BlinkValue>, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::List(xs),
        pos,
    })))
}

pub fn vector_val_at(xs: Vec<BlinkValue>, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Vector(xs),
        pos,
    })))
}

pub fn map_val_at(m: HashMap<String, BlinkValue>, pos: Option<SourcePos>) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Map(m),
        pos,
    })))
}

pub fn nil_at(pos: Option<SourcePos>) -> BlinkValue {
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
