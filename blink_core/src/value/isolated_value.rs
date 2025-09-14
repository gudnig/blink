use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use crate::value::FutureHandle;

#[derive(Clone, Debug)]
pub struct FunctionHandle {
    pub(crate) id: u64,
    pub(crate) name: Option<String>, // For debugging
}

impl PartialEq for FunctionHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Hash for FunctionHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}






#[derive(Clone, Debug)]
pub enum IsolatedValue {
    Number(f64),
    String(String),
    Symbol(String),
    Keyword(String),
    Bool(bool),
    List(Vec<IsolatedValue>),
    Vector(Vec<IsolatedValue>),
    Set(HashSet<IsolatedValue>),
    Map(HashMap<IsolatedValue, IsolatedValue>),
    Function(FunctionHandle),
    Macro(FunctionHandle),
    Future(FutureHandle),
    Error(String),
    Nil,
}

impl PartialEq for IsolatedValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (IsolatedValue::Number(a), IsolatedValue::Number(b)) => a == b,
            (IsolatedValue::String(a), IsolatedValue::String(b)) => a == b,
            (IsolatedValue::Bool(a), IsolatedValue::Bool(b)) => a == b,
            (IsolatedValue::List(a), IsolatedValue::List(b)) => a == b,
            (IsolatedValue::Map(a), IsolatedValue::Map(b)) => a == b,
            (IsolatedValue::Symbol(a), IsolatedValue::Symbol(b)) => a == b,
            (IsolatedValue::Keyword(a), IsolatedValue::Keyword(b)) => a == b,
            (IsolatedValue::Vector(a), IsolatedValue::Vector(b)) => a == b,
            (IsolatedValue::Set(a), IsolatedValue::Set(b)) => {
                a.iter().all(|v| b.contains(v)) && b.iter().all(|v| a.contains(v)) && a.len() == b.len()
            
            },
            (IsolatedValue::Function(a), IsolatedValue::Function(b)) => a == b,
            (IsolatedValue::Macro(a), IsolatedValue::Macro(b)) => a == b,
            (IsolatedValue::Future(a), IsolatedValue::Future(b)) => a == b,
            (IsolatedValue::Error(a), IsolatedValue::Error(b)) => a == b,
            (IsolatedValue::Nil, IsolatedValue::Nil) => true,
            _ => false,
        }
    }
}


impl Hash for IsolatedValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            IsolatedValue::Number(n) => (*n as u64).hash(state),
            IsolatedValue::String(s) => s.hash(state),
            IsolatedValue::Bool(b) => (*b as u64).hash(state),
            IsolatedValue::List(l) => l.hash(state),
            IsolatedValue::Map(m) => {
                        let mut pairs = m.iter().collect::<Vec<_>>();
                        pairs.sort_by(|a, b| {
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
                    },
            IsolatedValue::Symbol(s) => s.hash(state),
            IsolatedValue::Keyword(k) => k.hash(state),
            IsolatedValue::Vector(v) => v.hash(state),
            IsolatedValue::Set(s) => {
                        let mut items = s.iter().collect::<Vec<_>>();
                        items.sort_by(|a, b| {
                            let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
                            let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
                            a.hash(&mut hasher_a);
                            b.hash(&mut hasher_b);
                            hasher_a.finish().cmp(&hasher_b.finish())
                        });
                        for item in items {
                            item.hash(state);
                        }
                    },
            IsolatedValue::Function(f) => f.hash(state),
            IsolatedValue::Macro(m) => m.hash(state),
            IsolatedValue::Future(f) => f.hash(state),
            IsolatedValue::Error(_) => todo!(),
            IsolatedValue::Nil => 0u64.hash(state),
        }
    }
}

impl Eq for IsolatedValue {}

impl IsolatedValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            IsolatedValue::Number(_) => "number",
            IsolatedValue::String(_) => "string",
            IsolatedValue::Bool(_) => "bool",
            IsolatedValue::List(_) => "list",
            IsolatedValue::Map(_) => "map",
            IsolatedValue::Nil => "nil",
            IsolatedValue::Symbol(_) => "symbol",
            IsolatedValue::Keyword(_) => "keyword",
            IsolatedValue::Vector(_) => "vector",
            IsolatedValue::Set(_) => "set",
            IsolatedValue::Function(_) => "function",
            IsolatedValue::Macro(_) => "macro",
            IsolatedValue::Future(_) => "future",
            IsolatedValue::Error(_) => "error",
        }
    }
    
    pub fn is_truthy(&self) -> bool {
        !matches!(self, IsolatedValue::Bool(false) | IsolatedValue::Nil)
    }
}

impl IsolatedValue {
    pub fn as_number(&self) -> Result<f64, String> {
        match self {
            IsolatedValue::Number(n) => Ok(*n),
            _ => Err(format!("Expected number, got {}", self.type_name())),
        }
    }
    
    pub fn as_string(&self) -> Result<&str, String> {
        match self {
            IsolatedValue::String(s) => Ok(s),
            _ => Err(format!("Expected string, got {}", self.type_name())),
        }
    }
    
    pub fn as_bool(&self) -> Result<bool, String> {
        match self {
            IsolatedValue::Bool(b) => Ok(*b),
            _ => Err(format!("Expected boolean, got {}", self.type_name())),
        }
    }
    
    pub fn as_list(&self) -> Result<&Vec<IsolatedValue>, String> {
        match self {
            IsolatedValue::List(list) => Ok(list),
            _ => Err(format!("Expected list, got {}", self.type_name())),
        }
    }

    pub fn as_map(&self) -> Result<&HashMap<IsolatedValue, IsolatedValue>, String> {
        match self {
            IsolatedValue::Map(map) => Ok(map),
            _ => Err(format!("Expected map, got {}", self.type_name())),
        }
    }

    pub fn as_vector(&self) -> Result<&Vec<IsolatedValue>, String> {
        match self {
            IsolatedValue::Vector(vector) => Ok(vector),
            _ => Err(format!("Expected vector, got {}", self.type_name())),
        }
    }

    pub fn as_set(&self) -> Result<&HashSet<IsolatedValue>, String> {
        match self {
            IsolatedValue::Set(set) => Ok(set),
            _ => Err(format!("Expected set, got {}", self.type_name())),
        }
    }
    
    pub fn as_symbol(&self) -> Result<&str, String> {
        match self {
            IsolatedValue::Symbol(s) => Ok(s),
            _ => Err(format!("Expected symbol, got {}", self.type_name())),
        }
    }
    
    pub fn as_keyword(&self) -> Result<&str, String> {
        match self {
            IsolatedValue::Keyword(k) => Ok(k),
            _ => Err(format!("Expected keyword, got {}", self.type_name())),
        }
    }
    
    pub fn as_function(&self) -> Result<&FunctionHandle, String> {
        match self {
            IsolatedValue::Function(f) => Ok(f),
            _ => Err(format!("Expected function, got {}", self.type_name())),
        }
    }
    
    pub fn as_future(&self) -> Result<&FutureHandle, String> {
        match self {
            IsolatedValue::Future(f) => Ok(f),
            _ => Err(format!("Expected future, got {}", self.type_name())),
        }
    }
    
    // Convenience for common patterns
    pub fn expect_arity(&self, args: &[IsolatedValue], expected: usize) -> Result<(), String> {
        if args.len() != expected {
            Err(format!("Expected {} arguments, got {}", expected, args.len()))
        } else {
            Ok(())
        }
    }

}

impl IsolatedValue {
    // For working with sequences (lists and vectors)
    pub fn as_sequence(&self) -> Result<&[IsolatedValue], String> {
        match self {
            IsolatedValue::List(list) => Ok(list),
            IsolatedValue::Vector(vec) => Ok(vec),
            _ => Err(format!("Expected list or vector, got {}", self.type_name())),
        }
    }
    
    // Collection length
    pub fn len(&self) -> Result<usize, String> {
        match self {
            IsolatedValue::List(list) => Ok(list.len()),
            IsolatedValue::Vector(vec) => Ok(vec.len()),
            IsolatedValue::Map(map) => Ok(map.len()),
            IsolatedValue::Set(set) => Ok(set.len()),
            IsolatedValue::String(s) => Ok(s.len()),
            _ => Err(format!("Type {} has no length", self.type_name())),
        }
    }
    
    // Check if collection is empty
    pub fn is_empty(&self) -> Result<bool, String> {
        Ok(self.len()? == 0)
    }
}

impl IsolatedValue {
    // Convenience constructors
    pub fn number(n: f64) -> Self {
        IsolatedValue::Number(n)
    }
    
    pub fn string(s: impl Into<String>) -> Self {
        IsolatedValue::String(s.into())
    }
    
    pub fn symbol(s: impl Into<String>) -> Self {
        IsolatedValue::Symbol(s.into())
    }
    
    pub fn keyword(k: impl Into<String>) -> Self {
        IsolatedValue::Keyword(k.into())
    }
    
    pub fn list(items: Vec<IsolatedValue>) -> Self {
        IsolatedValue::List(items)
    }
    
    pub fn vector(items: Vec<IsolatedValue>) -> Self {
        IsolatedValue::Vector(items)
    }
    
    pub fn nil() -> Self {
        IsolatedValue::Nil
    }
    
    pub fn error(msg: impl Into<String>) -> Self {
        IsolatedValue::Error(msg.into())
    }
}

impl IsolatedValue {
    // Try to coerce to number (useful for math operations)
    pub fn try_as_number(&self) -> Result<f64, String> {
        match self {
            IsolatedValue::Number(n) => Ok(*n),
            IsolatedValue::String(s) => s.parse().map_err(|_| format!("Cannot parse '{}' as number", s)),
            _ => Err(format!("Cannot convert {} to number", self.type_name())),
        }
    }
    
    // Try to coerce to string
    pub fn try_as_string(&self) -> String {
        match self {
            IsolatedValue::String(s) => s.clone(),
            IsolatedValue::Symbol(s) => s.clone(),
            IsolatedValue::Keyword(k) => format!(":{}", k),
            IsolatedValue::Number(n) => n.to_string(),
            IsolatedValue::Bool(b) => b.to_string(),
            IsolatedValue::Nil => "nil".to_string(),
            _ => format!("#<{}>", self.type_name()),
        }
    }
}