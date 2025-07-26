use core::fmt;
use std::{
    fmt::Display,
    hash::{Hash, Hasher},
};

use mmtk::util::ObjectReference;

use crate::{
    error::BlinkError,
    future::BlinkFuture,
    runtime::{TypeTag},
    value::{
        is_bool, is_number, is_symbol, pack_bool, pack_keyword, pack_nil, pack_number,
        pack_symbol, unpack_immediate, ContextualNativeFn, GcPtr, HeapValue, ImmediateValue,
        IsolatedNativeFn, NativeFn,
    },
    collections::{BlinkHashMap, BlinkHashSet},
};

#[derive(Debug, Copy, Clone)]
pub enum ValueRef {
    Immediate(u64),
    Heap(GcPtr),
    Native(usize),
}

#[derive(Debug)]
pub struct ModuleRef {
    pub module: u32,
    pub symbol: u32,
}

#[derive(Debug)]
pub struct Callable {
    pub params: Vec<u32>,
    pub body: Vec<ValueRef>,
    pub module: u32,
    pub env: ObjectReference,
    pub is_variadic: bool,
}

impl Display for ValueRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueRef::Immediate(packed) => write!(f, "{}", unpack_immediate(*packed)),
            ValueRef::Heap(gc_ptr) => write!(f, "{}", gc_ptr.to_heap_value()),
            ValueRef::Native(n) => write!(f, "native-function-{}", n),
        }
    }
}

impl Hash for ValueRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            ValueRef::Immediate(packed) => packed.hash(state),
            ValueRef::Heap(gc_ptr) => gc_ptr.hash(state),
            ValueRef::Native(n) => {
                "native".hash(state);
                n.hash(state);
            }
        }
    }
}

impl PartialEq for ValueRef {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ValueRef::Immediate(packed), ValueRef::Immediate(other_packed)) => {
                packed == other_packed
            }
            (ValueRef::Heap(gc_ptr), ValueRef::Heap(other_gc_ptr)) => gc_ptr == other_gc_ptr,
            (ValueRef::Native(n), ValueRef::Native(other_n)) => n == other_n,
            _ => false,
        }
    }
}

impl Eq for ValueRef {}

impl ValueRef {
    // ------------------------------------------------------------
    // Constructors
    // ------------------------------------------------------------

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
        ValueRef::Immediate(pack_nil())
    }

    pub fn isolated_native_fn(boxed_fn: IsolatedNativeFn) -> Self {
        let ptr = Box::into_raw(Box::new(boxed_fn)) as *mut IsolatedNativeFn as usize;
        debug_assert!(ptr & 1 == 0, "Pointer must be aligned");
        ValueRef::Native(ptr | 0) // Tag 0 for isolated
    }

    pub fn contextual_native_fn(boxed_fn: ContextualNativeFn) -> Self {
        let ptr = Box::into_raw(Box::new(boxed_fn)) as *mut ContextualNativeFn as usize;
        debug_assert!(ptr & 1 == 0, "Pointer must be aligned");
        ValueRef::Native(ptr | 1) // Tag 1 for contextual
    }

    pub fn type_tag(&self) -> &'static str {
        match self {
            ValueRef::Immediate(packed) => unpack_immediate(*packed).type_tag(),
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag().to_str(),
            ValueRef::Native(_) => "native-function",
        }
    }

    // Type checking
    pub fn is_number(&self) -> bool {
        match self {
            ValueRef::Immediate(packed) => is_number(*packed),
            _ => false,
        }
    }

    pub fn is_string(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::Str,
            _ => false,
        }
    }
    pub fn is_error(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::Error,
            _ => false,
        }
    }


    pub fn is_symbol(&self) -> bool {
        match self {
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(*packed);
                if let ImmediateValue::Symbol(_) = unpacked {
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn is_keyword(&self) -> bool {
        match self {
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(*packed);
                if let ImmediateValue::Keyword(_) = unpacked {
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn is_list(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::List,
            _ => false,
        }
    }

    pub fn is_vec(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::Vector,
            _ => false,
        }
    }

    pub fn is_map(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::Map,
            _ => false,
        }
    }

    pub fn is_set(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::Set,
            _ => false,
        }
    }

    pub fn is_future(&self) -> bool {
        match self {
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag() == TypeTag::Future,
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
            }
            _ => true,
        }
    }

    // ------------------------------------------------------------
    // Value extraction
    // ------------------------------------------------------------

    pub fn get_future(&self) -> Option<BlinkFuture> {
        if self.is_future() {
            match self {
                ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                    HeapValue::Future(future) => Some(future.clone()),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }
    pub fn get_string(&self) -> Option<String> {
        if self.is_string() {
            match self {
                ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                    HeapValue::Str(str) => Some(str.clone()),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn get_list(&self) -> Option<Vec<ValueRef>> {
        if self.is_list() {
            match self {
                ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                    HeapValue::List(list) => Some(list.clone()),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn get_vec(&self) -> Option<Vec<ValueRef>> {
        if self.is_vec() {
            match self {
                ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                    HeapValue::Vector(vec) => Some(vec.clone()),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn get_map(&self) -> Option<BlinkHashMap> {
        if self.is_map() {
            match self {
                ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                    HeapValue::Map(map) => Some(map.clone()),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn get_set(&self) -> Option<BlinkHashSet> {
        if self.is_set() {
            match self {
                ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                    HeapValue::Set(set) => Some(set.clone()),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }
    pub fn get_error(&self) -> Option<BlinkError> {
        if !self.is_error() {
            return None;
        }

        if let ValueRef::Heap(gc_ptr) = self {
            let heap_value = gc_ptr.to_heap_value();
            match heap_value {
                HeapValue::Error(error) => Some(error),
                _ => None,
            }
        } else {
            None
        }
    }

    // Value extraction
    pub fn get_number(&self) -> Option<f64> {
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

    pub fn get_bool(&self) -> Option<bool> {
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

    pub fn type_name(&self) -> &'static str {
        match self {
            ValueRef::Immediate(packed) => match unpack_immediate(*packed) {
                ImmediateValue::Number(_) => "number",
                ImmediateValue::Bool(_) => "boolean",
                ImmediateValue::Symbol(_) => "symbol",
                ImmediateValue::Nil => "nil",
                ImmediateValue::Keyword(_) => "keyword",
            },
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag().to_str(),
            ValueRef::Native(_) => "native-function",
        }
    }

    pub fn get_symbol(&self) -> Option<u32> {
        match self {
            ValueRef::Immediate(packed) => {
                if is_symbol(*packed) {
                    Some((packed >> 3) as u32)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_keyword(&self) -> Option<u32> {
        match self {
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(*packed);
                match unpacked {
                    ImmediateValue::Keyword(k) => Some(k),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn get_native_fn(&self) -> Option<&NativeFn> {
        match self {
            ValueRef::Native(ptr) => {
                let raw_ptr = ptr & !1; // Clear tag bit
                if ptr & 1 == 0 {
                    // Isolated function
                    let fn_ptr = raw_ptr as *const NativeFn;
                    Some(unsafe { &*fn_ptr })
                } else {
                    // Contextual function
                    let fn_ptr = raw_ptr as *const NativeFn;
                    Some(unsafe { &*fn_ptr })
                }
            }
            _ => None,
        }
    }


}
