use std::{
    hash::{Hash, Hasher},
    sync::Arc,
    };

    
use parking_lot::RwLock;



use crate::{
    collections::ValueContext, env::Env, error::BlinkError, eval::{EvalContext, EvalResult}, runtime::{ContextualBoundary, TypeTag, ValueBoundary}, value::{is_bool, is_number, is_symbol, pack_bool, pack_keyword, pack_module, pack_nil, pack_number, pack_symbol, unpack_immediate, ContextualNativeFn, GcPtr, HeapValue, ImmediateValue, IsolatedNativeFn, IsolatedValue, NativeFn}
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
    pub env: Arc<RwLock<Env>>,
    pub is_variadic: bool,
}


impl Hash for ValueRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            ValueRef::Immediate(packed) => packed.hash(state),
            ValueRef::Heap(gc_ptr) => gc_ptr.hash(state),
            ValueRef::Native(n) => {
                "native".hash(state);
                n.hash(state);
            },
        }
    }
}

impl PartialEq for ValueRef {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ValueRef::Immediate(packed), ValueRef::Immediate(other_packed)) => packed == other_packed,
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

    pub fn module(module: u32, symbol: u32) -> Self {
        ValueRef::Immediate(pack_module(module, symbol))
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

    pub fn type_name(&self) -> &'static str {
        match self {
            ValueRef::Immediate(packed) => match unpack_immediate(*packed) {
                ImmediateValue::Number(_) => "number",
                ImmediateValue::Bool(_) => "boolean",
                ImmediateValue::Symbol(_) => "symbol",
                ImmediateValue::Nil => "nil",
                ImmediateValue::Keyword(_) => "keyword",
                ImmediateValue::Module(_, _) => "module",
            },
            ValueRef::Heap(gc_ptr) => gc_ptr.type_tag().to_str(),
            ValueRef::Native(_) => "native-function",
        }
    }


    pub fn try_get_number(&self) -> Option<f64> {
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

    pub fn try_get_bool(&self) -> Option<bool> {
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

    pub fn try_get_symbol(&self) -> Option<u32> {
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

    pub fn try_get_keyword(&self) -> Option<u32> {
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

    pub fn call_native(&self, args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
        match self {
            ValueRef::Native(tagged_ptr) => {
                let ptr = tagged_ptr & !1; // Clear the tag bit
                
                if tagged_ptr & 1 == 0 {
                    // Tag 0 = Isolated function
                    let boxed_fn_ptr = ptr as *const IsolatedNativeFn;
                    let boxed_fn = unsafe { &*boxed_fn_ptr };
                    
                    // Convert args and call
                    let mut boundary = ContextualBoundary::new(ctx);
                    let isolated_args: Result<Vec<_>, _> = args.iter()
                        .map(|arg| boundary.extract_isolated(*arg))
                        .collect();
                    
                    match isolated_args {
                        Ok(isolated_args) => {
                            match boxed_fn(isolated_args) {
                                Ok(result) => EvalResult::Value(boundary.alloc_from_isolated(result)),
                                Err(e) => EvalResult::Value(ctx.error_value(BlinkError::eval(e))),
                            }
                        }
                        Err(e) => EvalResult::Value(ctx.error_value(BlinkError::eval(e))),
                    }
                } else {
                    // Tag 1 = Contextual function
                    let boxed_fn_ptr = ptr as *const ContextualNativeFn;
                    let boxed_fn = unsafe { &*boxed_fn_ptr };
                    
                    // Call directly
                    boxed_fn(args, ctx)
                }
            }
            _ => EvalResult::Value(ctx.error_value(BlinkError::eval("Not a native function"))),
        }
    }
}