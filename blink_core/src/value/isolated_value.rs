use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};


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

#[derive(Clone, Debug, PartialEq)]
pub struct FutureHandle {
    pub(crate) id: u64,
}

impl Hash for FutureHandle {
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