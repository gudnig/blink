use core::fmt;
use std::{
    fmt::Display,
    hash::{Hash, Hasher},
};

use mmtk::util::ObjectReference;

use crate::{
    collections::{BlinkHashMap, BlinkHashSet}, error::BlinkError, runtime::{CompiledFunction, TypeTag}, value::{
        is_bool, is_number, is_symbol, pack_bool, pack_keyword, pack_nil, pack_number, pack_symbol, unpack_immediate, ContextualNativeFn, FutureHandle, GcPtr, HeapValue, ImmediateValue, IsolatedNativeFn, NativeFn
    }
};

#[derive(Debug, Copy, Clone)]
pub enum ValueRef {
    Immediate(u64),
    Heap(GcPtr),
    Handle(usize),
}

// Handle type tags
const FN_TAG: usize = 0;
const FUTURE_HANDLE_TAG: usize = 1;
const CHANNEL_HANDLE_TAG: usize = 2;
const _RESERVED_TAG: usize = 3; // Reserved for future use

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
            ValueRef::Handle(tagged_ptr) => {
                match tagged_ptr & 3 {
                    ISOLATED_FN_TAG => write!(f, "isolated-function"),
                    CONTEXTUAL_FN_TAG => write!(f, "contextual-function"),
                    FUTURE_HANDLE_TAG => {
                        let packed = *tagged_ptr as u64;
                        let id = packed >> 32;
                        let generation = ((packed >> 2) & 0x3FFFFFFF) as u32;
                        write!(f, "future(id:{}, gen:{})", id, generation)
                    }
                    _ => write!(f, "unknown-handle"),
                }
            }
        }
    }
}

impl Hash for ValueRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            ValueRef::Immediate(packed) => packed.hash(state),
            ValueRef::Heap(gc_ptr) => gc_ptr.hash(state),
            ValueRef::Handle(n) => {
                "handle".hash(state);
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
            (ValueRef::Handle(n), ValueRef::Handle(other_n)) => n == other_n,
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
        let native_fn = NativeFn::Isolated(boxed_fn);
        Self::native_function(native_fn)
    }



    pub fn contextual_native_fn(boxed_fn: ContextualNativeFn) -> Self {
        let native_fn = NativeFn::Contextual(boxed_fn);
        Self::native_function(native_fn)
    }

    pub fn native_function(func: NativeFn) -> Self {
        let ptr = Box::into_raw(Box::new(func)) as usize;
        debug_assert!(ptr & 3 == 0, "Pointer must be 4-byte aligned");
        ValueRef::Handle(ptr | FN_TAG)
    }

    pub fn future_handle(id: u64, generation: u32) -> Self {
        // Pack: 32 bits ID + 30 bits generation + 2 bits tag
        debug_assert!(generation < (1 << 30), "Generation too large for 30 bits");
        let packed = (id << 32) | ((generation as u64) << 2) | FUTURE_HANDLE_TAG as u64;
        ValueRef::Handle(packed as usize)
    }

    pub fn channel_handle(id: u64, generation: u32) -> Self {
        // Pack: 32 bits ID + 30 bits generation + 2 bits tag
        debug_assert!(generation < (1 << 30), "Generation too large for 30 bits");
        let packed = (id << 32) | ((generation as u64) << 2) | CHANNEL_HANDLE_TAG as u64;
        ValueRef::Handle(packed as usize)
    }

    pub fn type_tag(&self) -> &'static str {
        match self {
            ValueRef::Immediate(packed) => unpack_immediate(*packed).type_tag(),
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag().to_str(),
            ValueRef::Handle(handle) => match handle & 3 {
                FN_TAG => "native-function",
                FUTURE_HANDLE_TAG => "future",
                CHANNEL_HANDLE_TAG => "channel",
                _ => "unknown",
            }
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

    pub fn get_native_fn(&self) -> Option<&NativeFn> {
        match self {
            ValueRef::Handle(tagged_ptr) => {
                let tag = tagged_ptr & 3;
                let ptr = tagged_ptr & !3; // Clear tag bits
                
                match tag {
                    ISOLATED_FN_TAG => {
                        let fn_ptr = ptr as *const NativeFn;
                        Some(unsafe { &*fn_ptr })
                    }
                    CONTEXTUAL_FN_TAG => {
                        let fn_ptr = ptr as *const NativeFn;
                        Some(unsafe { &*fn_ptr })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn get_future_handle(&self) -> Option<FutureHandle> {
        match self {
            ValueRef::Handle(tagged_ptr) => {
                if tagged_ptr & 3 == FUTURE_HANDLE_TAG {
                    let packed = *tagged_ptr as u64;
                    let id = packed >> 32;
                    let generation = ((packed >> 2) & 0x3FFFFFFF) as u32; // 30 bits
                    Some(FutureHandle { id, generation })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    

    pub fn get_compiled_function(&self) -> Option<CompiledFunction> {
        match self {
            ValueRef::Heap(gc_ptr) => match gc_ptr.to_heap_value() {
                HeapValue::Function(function) => Some(function),
                _ => None,
            },
            _ => None,
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
            ValueRef::Handle(tagged_ptr) => {
                let tag = tagged_ptr & 3;
                match tag {
                    ISOLATED_FN_TAG => "native-function",
                    CONTEXTUAL_FN_TAG => "native-function",
                    FUTURE_HANDLE_TAG => "future",
                    _ => "unknown",
                }
            }
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

    // ------------------------------------------------------------
    // Type checking
    // ------------------------------------------------------------
    pub fn is_future(&self) -> bool {
        matches!(self, ValueRef::Handle(tagged_ptr) if tagged_ptr & 3 == FUTURE_HANDLE_TAG)
    }

    pub fn is_channel(&self) -> bool {
        matches!(self, ValueRef::Handle(tagged_ptr) if tagged_ptr & 3 == CHANNEL_HANDLE_TAG)
    }

    pub fn is_native_fn(&self) -> bool {
        matches!(self, ValueRef::Handle(tagged_ptr) if (tagged_ptr & 3) == FN_TAG)
    }
    


}
