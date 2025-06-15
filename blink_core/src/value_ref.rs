use std::{collections::{HashMap, HashSet}, hash::{Hash, Hasher}, sync::Arc };


use generational_arena::Index;
use parking_lot::RwLock;

use crate::{env::Env, error::BlinkError, future::BlinkFuture, shared_arena::SharedArena};

#[derive(Debug, Copy, Clone)]
pub enum GcPtr {
    NothingTodo,
}

#[derive(Debug, Copy, Clone)]
pub enum ValueRef {
    Immediate(u64),           
    Gc(GcPtr),               
    Shared(Index), 
    Nil,
}


pub type NativeFn = fn(Vec<ValueRef>) -> Result<ValueRef, BlinkError>;

#[derive(Debug)]
pub struct ModuleRef {
    pub module: String,
    pub symbol: String,
}

#[derive(Debug)]
pub struct UserDefinedFn {
    pub params: Vec<String>,
    pub body: Vec<ValueRef>,
    pub env: Arc<RwLock<Env>>, // closure capture
}

#[derive(Debug)]
pub struct Macro {
    pub params: Vec<String>,
    pub body: Vec<ValueRef>,
    pub env: Arc<RwLock<Env>>,
    pub is_variadic: bool,
}


#[derive(Debug)]
pub enum SharedValue {

    // Lisp data that needs to be shared but eventaully will be moved to the GC heap
    List(Vec<ValueRef>),
    Vector(Vec<ValueRef>),
    Map(HashMap<ValueRef, ValueRef>),
    Str(String),
    Symbol(String),
    Keyword(String),
    Set(HashSet<ValueRef>),
    Number(f64),
    Bool(bool),
    Error(BlinkError),
    //TODO env could be a reference to a shared value
    
    // Runtime objects
    Future(BlinkFuture),
    NativeFunction(NativeFn),
    Module(ModuleRef),
    UserDefinedFunction(UserDefinedFn),
    Macro(Macro),

}


impl SharedValue {
    pub fn type_tag(&self) -> &'static str  {
        match self {
            SharedValue::Number(_) => "number",
            SharedValue::Bool(_) => "bool",
            SharedValue::Str(_) => "string",
            SharedValue::Keyword(_) => "keyword",
            SharedValue::Error(_) => "error",
            _ => todo!(),
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            SharedValue::Error(_) => true,
            _ => false,
        }
    }


    pub fn hash_with_arena<H: Hasher>(&self, state: &mut H, arena: &SharedArena) {
        match self {
            SharedValue::Str(s) => {
                "string".hash(state);
                s.hash(state);
            }
            SharedValue::Symbol(s) => {
                "symbol".hash(state);
                s.hash(state);
            }
            SharedValue::Keyword(k) => {
                "keyword".hash(state);
                k.hash(state);
            }
            SharedValue::List(items) => {
                "list".hash(state);
                items.len().hash(state);
                for item in items {
                    item.hash_with_arena(state, arena);
                }
            }
            SharedValue::Vector(items) => {
                "vector".hash(state);
                items.len().hash(state);
                for item in items {
                    item.hash_with_arena(state, arena);
                }
            }
            SharedValue::Map(map) => {
                "map".hash(state);
                map.len().hash(state);
                // Maps are unordered, so we need order-independent hashing
                let mut pairs: Vec<_> = map.iter().collect();
                pairs.sort_by(|a, b| {
                    // Sort by hash of key for deterministic ordering
                    let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
                    let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
                    a.0.hash_with_arena(&mut hasher_a, arena);
                    b.0.hash_with_arena(&mut hasher_b, arena);
                    hasher_a.finish().cmp(&hasher_b.finish())
                });
                for (k, v) in pairs {
                    k.hash_with_arena(state, arena);
                    v.hash_with_arena(state, arena);
                }
            }
            SharedValue::Set(set) => {
                "set".hash(state);
                set.len().hash(state);
                // Sets are unordered, collect and sort for deterministic hashing
                let mut items: Vec<_> = set.iter().collect();
                items.sort_by(|a, b| {
                    let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
                    let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
                    a.hash_with_arena(&mut hasher_a, arena);
                    b.hash_with_arena(&mut hasher_b, arena);
                    hasher_a.finish().cmp(&hasher_b.finish())
                });
                for item in items {
                    item.hash_with_arena(state, arena);
                }
            }
            SharedValue::Number(n) => {
                "number".hash(state);
                if n.fract() == 0.0 && n.is_finite() {
                    (*n as i64).hash(state);
                } else {
                    n.to_bits().hash(state);
                }
            }
            SharedValue::Bool(b) => {
                "bool".hash(state);
                b.hash(state);
            }
            SharedValue::Error(e) => {
                "error".hash(state);
                // Hash the error message/type
                format!("{:?}", e).hash(state);
            }
            SharedValue::Future(_) => {
                "future".hash(state);
                // TODO: implement future hashing (maybe use an ID?)
                "future-placeholder".hash(state);
            }
            SharedValue::NativeFunction(f) => {
                "native-func".hash(state);
                // Function pointers can be hashed by address
                (*f as *const NativeFn).hash(state);
            }
            SharedValue::Module(module_ref) => {
                "module-ref".hash(state);
                module_ref.module.hash(state);
                module_ref.symbol.hash(state);
            }
            SharedValue::UserDefinedFunction(func) => {
                "user-func".hash(state);
                func.params.hash(state);
                func.body.len().hash(state);
                for expr in &func.body {
                    expr.hash_with_arena(state, arena);
                }
                // Note: we're not hashing the environment for now
            }
            SharedValue::Macro(mac) => {
                "macro".hash(state);
                mac.params.hash(state);
                mac.is_variadic.hash(state);
                mac.body.len().hash(state);
                for expr in &mac.body {
                    expr.hash_with_arena(state, arena);
                }
                // Note: we're not hashing the environment for now
            }
        }
    }

    /// Check equality with another SharedValue
    pub fn eq_with_arena(&self, other: &Self, arena: &SharedArena) -> bool {
        match (self, other) {
            (SharedValue::Number(a), SharedValue::Number(b)) => {
                // Handle float/int equality: 1.0 == 1
                if a.fract() == 0.0 && b.fract() == 0.0 {
                    *a as i64 == *b as i64
                } else {
                    a == b
                }
            }
            (SharedValue::Bool(a), SharedValue::Bool(b)) => a == b,
            (SharedValue::Str(a), SharedValue::Str(b)) => a == b,
            (SharedValue::Symbol(a), SharedValue::Symbol(b)) => a == b,
            (SharedValue::Keyword(a), SharedValue::Keyword(b)) => a == b,
            
            (SharedValue::List(a), SharedValue::List(b)) => {
                a.len() == b.len() && 
                a.iter().zip(b.iter()).all(|(x, y)| x.eq_with_arena(y, arena))
            }
            (SharedValue::Vector(a), SharedValue::Vector(b)) => {
                a.len() == b.len() && 
                a.iter().zip(b.iter()).all(|(x, y)| x.eq_with_arena(y, arena))
            }
            (SharedValue::Map(a), SharedValue::Map(b)) => {
                a.len() == b.len() && 
                a.iter().all(|(k, v)| {
                    b.iter().any(|(k2, v2)| {
                        k.eq_with_arena(k2, arena) && v.eq_with_arena(v2, arena)
                    })
                })
            }
            (SharedValue::Set(a), SharedValue::Set(b)) => {
                a.len() == b.len() && 
                a.iter().all(|item| {
                    b.iter().any(|item2| item.eq_with_arena(item2, arena))
                })
            }
            
            (SharedValue::NativeFunction(a), SharedValue::NativeFunction(b)) => {
                // Function pointer equality
                std::ptr::eq(a as *const _, b as *const _)
            }
            
            (SharedValue::UserDefinedFunction(a), SharedValue::UserDefinedFunction(b)) => {
                // Functions equal if same params and body
                // (ignoring environment for now)
                a.params == b.params && 
                a.body.len() == b.body.len() &&
                a.body.iter().zip(b.body.iter()).all(|(x, y)| x.eq_with_arena(y, arena))
            }
            
            (SharedValue::Module(a), SharedValue::Module(b)) => {
                a.module == b.module && a.symbol == b.symbol
            }
            
            (SharedValue::Macro(a), SharedValue::Macro(b)) => {
                a.params == b.params && 
                a.is_variadic == b.is_variadic &&
                a.body.len() == b.body.len() &&
                a.body.iter().zip(b.body.iter()).all(|(x, y)| x.eq_with_arena(y, arena))
            }
            
            // Different types are never equal
            _ => false
        }
    }
}

impl GcPtr {
    pub fn is_error(&self) -> bool {
        false   //TODO: implement this
    }
}

impl ValueRef {
    // Constructors
    pub fn number(n: f64) -> Self {
        ValueRef::Immediate(pack_number(n))
    }
    
    pub fn boolean(b: bool) -> Self {
        ValueRef::Immediate(pack_bool(b))
    }
    
    pub fn symbol(id: u32) -> Self {
        ValueRef::Immediate(pack_symbol(id))
    }

    pub fn keyword(id: u32) -> Self {
        ValueRef::Immediate(pack_keyword(id))
    }
    
    pub fn nil() -> Self {
        ValueRef::Nil  // Or ValueRef::Immediate(pack_nil())
    }

    pub fn type_tag(&self, arena: &SharedArena) -> &'static str  {
        match self {
            ValueRef::Immediate(packed) => unpack_immediate(*packed).type_tag(),
            ValueRef::Shared(idx) => {
                arena.get(*idx)
                    .map(|v| v.type_tag())
                    .unwrap_or("invalid-reference")
            }
            _ => todo!(),
        }
    }
    
    // Type checking
    pub fn is_number(&self) -> bool {
        match self {
            ValueRef::Immediate(packed) => is_number(*packed),
            _ => false,
        }
    }
    
    pub fn is_string(&self, arena: &SharedArena) -> bool {
        match self {
            ValueRef::Shared(idx) => {
                arena.get(*idx)
                    .map(|v| matches!(v.as_ref(), SharedValue::Str(_)))
                    .unwrap_or(false)
            }
            _ => false,
        }
    }
    pub fn is_error(&self, arena: &SharedArena) -> bool {
        match self {
            ValueRef::Shared(idx) => {
                arena.get(*idx)
                    .map(|v| v.is_error())  // Much cleaner!
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            ValueRef::Immediate(packed) => {
                let val = unpack_immediate(*packed);
                match val {
                    ImmediateValue::Bool(b) => b,
                    ImmediateValue::Nil => false,
                    _ => true,
                }
            },
            ValueRef::Nil => false,
            _ => true,
        }
    }
    
    // Value extraction
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ValueRef::Immediate(packed) => {
                if is_number(*packed) {
                    Some(f64::from_bits(*packed))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ValueRef::Immediate(packed) => {
                if is_bool(*packed) {
                    Some(((packed >> 3) & 1) != 0)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    
    pub fn type_name(&self, arena: &SharedArena) -> &'static str  {
        match self {
            ValueRef::Immediate(packed) => {
                match unpack_immediate(*packed) {
                    ImmediateValue::Number(_) => "number",
                    ImmediateValue::Bool(_) => "boolean", 
                    ImmediateValue::Symbol(_) => "symbol",
                    ImmediateValue::Nil => "nil",
                    ImmediateValue::Keyword(_) => "keyword",
                }
            }
            ValueRef::Shared(idx) => {
                arena.get(*idx)
                    .map(|v| v.type_tag())
                    .unwrap_or("invalid-reference")
            }
            ValueRef::Nil => "nil",
            ValueRef::Gc(_) => "gc-object",
        }
    }
    
    pub fn hash_with_arena<H: Hasher>(&self, state: &mut H, arena: &SharedArena) {
        match self {
            ValueRef::Immediate(packed) => {
                // Tag to distinguish from shared values
                0u8.hash(state);
                match unpack_immediate(*packed) {
                    ImmediateValue::Number(n) => {
                                        // Handle float/int equality: 1.0 == 1
                                        if n.fract() == 0.0 && n.is_finite() {
                                            // Hash as integer if it's a whole number
                                            (n as i64).hash(state);
                                        } else {
                                            n.to_bits().hash(state);
                                        }
                                    }
                    ImmediateValue::Bool(b) => b.hash(state),
                    ImmediateValue::Symbol(s) => s.hash(state),
                    ImmediateValue::Nil => "nil".hash(state),
                    ImmediateValue::Keyword(_) => todo!(),
                }
            }
            ValueRef::Shared(idx) => {
                // Tag to distinguish from immediate values
                1u8.hash(state);
                if let Some(shared_val) = arena.get(*idx) {
                    shared_val.hash_with_arena(state, arena);
                } else {
                    // Invalid reference - hash the index itself
                    "invalid-reference".hash(state);
                    idx.hash(state);
                }
            }
            ValueRef::Gc(_gc_ptr) => {
                2u8.hash(state);
                // TODO: implement GC pointer hashing
                "gc-object".hash(state);
            }
            ValueRef::Nil => {
                3u8.hash(state);
                "nil".hash(state);
            }
        }
    }

    /// Check equality with another ValueRef, with access to the shared arena
    pub fn eq_with_arena(&self, other: &Self, arena: &SharedArena) -> bool {
        match (self, other) {
            // Immediate values can be compared directly
            (ValueRef::Immediate(a), ValueRef::Immediate(b)) => {
                let val_a = unpack_immediate(*a);
                let val_b = unpack_immediate(*b);
                match (val_a, val_b) {
                    (ImmediateValue::Number(a), ImmediateValue::Number(b)) => {
                        // Handle float/int equality: 1.0 == 1
                        if a.fract() == 0.0 && b.fract() == 0.0 {
                            a as i64 == b as i64
                        } else {
                            a == b
                        }
                    }
                    (ImmediateValue::Bool(a), ImmediateValue::Bool(b)) => a == b,
                    (ImmediateValue::Symbol(a), ImmediateValue::Symbol(b)) => a == b,
                    (ImmediateValue::Nil, ImmediateValue::Nil) => true,
                    _ => false, // Different immediate types
                }
            }

            // Shared values need arena access
            (ValueRef::Shared(a_idx), ValueRef::Shared(b_idx)) => {
                if a_idx == b_idx {
                    return true; // Same reference
                }
                
                match (arena.get(*a_idx), arena.get(*b_idx)) {
                    (Some(a_val), Some(b_val)) => a_val.eq_with_arena(b_val, arena),
                    _ => false, // At least one invalid reference
                }
            }

            // Nil variants
            (ValueRef::Nil, ValueRef::Nil) => true,
            (ValueRef::Nil, ValueRef::Immediate(packed)) | 
            (ValueRef::Immediate(packed), ValueRef::Nil) => {
                matches!(unpack_immediate(*packed), ImmediateValue::Nil)
            }

            // GC objects - TODO: implement proper comparison
            (ValueRef::Gc(_), ValueRef::Gc(_)) => {
                // For now, only equal if same pointer
                std::ptr::eq(self, other)
            }

            // Different variants are never equal
            _ => false,
        }
    }

}

// NaN-tagging constants
const NAN_MASK: u64 = 0x7FF0_0000_0000_0000;
const TAG_MASK: u64 = 0x7;

const BOOL_TAG: u64 = 1;
const SYMBOL_TAG: u64 = 2;
const NIL_TAG: u64 = 3;
const KEYWORD_TAG: u64 = 4;

// Packing functions
pub fn pack_number(n: f64) -> u64 {
    let bits = n.to_bits();
    if (bits & NAN_MASK) == NAN_MASK {
        panic!("Cannot pack NaN");
    }
    bits
}

pub fn pack_bool(b: bool) -> u64 {
    NAN_MASK | ((b as u64) << 3) | BOOL_TAG
}

pub fn pack_symbol(symbol_id: u32) -> u64 {
    NAN_MASK | ((symbol_id as u64) << 3) | SYMBOL_TAG
}

pub fn pack_nil() -> u64 {
    NAN_MASK | NIL_TAG
}

pub fn pack_keyword(keyword_id: u32) -> u64 {
    NAN_MASK | ((keyword_id as u64) << 3) | KEYWORD_TAG
}

// Unpacking
pub enum ImmediateValue {
    Number(f64),
    Bool(bool),
    Symbol(u32),
    Keyword(u32),
    Nil,
}

impl ImmediateValue {
    pub fn type_tag(&self) -> &'static str  {
        match self {
            ImmediateValue::Number(_) => "number",
            ImmediateValue::Bool(_) => "bool",
            ImmediateValue::Symbol(_) => "symbol",
            ImmediateValue::Keyword(_) => "keyword",
            ImmediateValue::Nil => "nil",
        }
    }
}

pub fn unpack_immediate(packed: u64) -> ImmediateValue {
    if (packed & NAN_MASK) != NAN_MASK {
        // Regular number
        ImmediateValue::Number(f64::from_bits(packed))
    } else {
        // Tagged value
        match packed & TAG_MASK {
            BOOL_TAG => ImmediateValue::Bool(((packed >> 3) & 1) != 0),
            SYMBOL_TAG => ImmediateValue::Symbol((packed >> 3) as u32),
            NIL_TAG => ImmediateValue::Nil,
            _ => panic!("Invalid immediate tag: {}", packed & TAG_MASK),
        }
    }
}

// Convenient type checking
pub fn is_number(packed: u64) -> bool {
    (packed & NAN_MASK) != NAN_MASK
}

pub fn is_bool(packed: u64) -> bool {
    (packed & NAN_MASK) == NAN_MASK && (packed & TAG_MASK) == BOOL_TAG
}

pub fn is_nil(packed: u64) -> bool {
    packed == (NAN_MASK | NIL_TAG)
}

// Helper trait for collections that need ValueRef as keys
pub trait ValueRefKey {
    fn hash_key<H: Hasher>(&self, state: &mut H, arena: &SharedArena);
    fn eq_key(&self, other: &Self, arena: &SharedArena) -> bool;
}

impl ValueRefKey for ValueRef {
    fn hash_key<H: Hasher>(&self, state: &mut H, arena: &SharedArena) {
        self.hash_with_arena(state, arena);
    }

    fn eq_key(&self, other: &Self, arena: &SharedArena) -> bool {
        self.eq_with_arena(other, arena)
    }
}
