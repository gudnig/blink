use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use generational_arena::Index;
use parking_lot::RwLock;

use crate::{
    collections::ValueContext, env::Env, error::BlinkError, eval::{EvalContext, EvalResult}, runtime::{ContextualBoundary, SharedArena, ValueBoundary}, value::{is_bool, is_number, is_symbol, pack_bool, pack_keyword, pack_nil, pack_number, pack_symbol, unpack_immediate, ImmediateValue, IsolatedValue, SharedValue}
};

#[derive(Debug, Copy, Clone)]
pub enum GcPtr {
    NothingTodo,
}

#[derive(Debug, Copy, Clone)]
pub enum ValueRef {
    Immediate(u64),
    Gc(GcPtr),
    Shared(Index),
}

pub type IsolatedNativeFn = Box<dyn Fn(Vec<IsolatedValue>) -> Result<IsolatedValue, String> + Send + Sync>;
pub type ContextualNativeFn = Box<dyn Fn(Vec<ValueRef>, &mut EvalContext) -> EvalResult + Send + Sync>;


pub enum NativeFn {
    Isolated(IsolatedNativeFn),
    Contextual(ContextualNativeFn),
}

impl std::fmt::Debug for NativeFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeFn::Isolated(_) => write!(f, "NativeFn::Isolated(<function>)"),
            NativeFn::Contextual(_) => write!(f, "NativeFn::Contextual(<function>)"),
        }
    }
}

impl NativeFn {
    pub fn call(&self, args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
        match self {
            NativeFn::Isolated(f) => {
                let mut boundary = ContextualBoundary::new(ctx);
                
                // Extract to isolated values
                let isolated_args: Result<Vec<_>, _> = args.iter()
                    .map(|arg| boundary.extract_isolated(*arg))
                    .collect();
                let isolated_args = isolated_args.map_err(|e| BlinkError::eval(e.to_string()));

                if let Err(e) = isolated_args {
                    return EvalResult::Value(ctx.eval_error(&e.to_string()));
                }
                let isolated_args = isolated_args.unwrap();
                // Call function
                let result = f(isolated_args);

                match result {
                    Ok(result) => {
                        // Convert back
                        EvalResult::Value(boundary.alloc_from_isolated(result))
                    }
                    Err(e) => {
                        EvalResult::Value(ctx.eval_error(&e.to_string()))
                    }
                }
            }
            
            NativeFn::Contextual(f) => {
                f(args, ctx)
            }
        }
    }
}

#[derive(Debug)]
pub struct ModuleRef {
    pub module: u32,
    pub symbol: u32,
}

#[derive(Debug)]
pub struct UserDefinedFn {
    pub params: Vec<u32>,
    pub body: Vec<ValueRef>,
    pub env: Arc<RwLock<Env>>, // closure capture
}

#[derive(Debug)]
pub struct Macro {
    pub params: Vec<u32>,
    pub body: Vec<ValueRef>,
    pub env: Arc<RwLock<Env>>,
    pub is_variadic: bool,
}

impl GcPtr {
    pub fn is_error(&self) -> bool {
        false //TODO: implement this
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
        ValueRef::Immediate(pack_nil())
    }

    pub fn type_tag(&self, arena: &SharedArena) -> &'static str {
        match self {
            ValueRef::Immediate(packed) => unpack_immediate(*packed).type_tag(),
            ValueRef::Shared(idx) => arena
                .get(*idx)
                .map(|v| v.type_tag())
                .unwrap_or("invalid-reference"),
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
            ValueRef::Shared(idx) => arena
                .get(*idx)
                .map(|v| matches!(v.as_ref(), SharedValue::Str(_)))
                .unwrap_or(false),
            _ => false,
        }
    }
    pub fn is_error(&self, arena: &SharedArena) -> bool {
        match self {
            ValueRef::Shared(idx) => {
                arena
                    .get(*idx)
                    .map(|v| v.is_error()) // Much cleaner!
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

    pub fn type_name(&self, arena: &SharedArena) -> &'static str {
        match self {
            ValueRef::Immediate(packed) => match unpack_immediate(*packed) {
                ImmediateValue::Number(_) => "number",
                ImmediateValue::Bool(_) => "boolean",
                ImmediateValue::Symbol(_) => "symbol",
                ImmediateValue::Nil => "nil",
                ImmediateValue::Keyword(_) => "keyword",
            },
            ValueRef::Shared(idx) => arena
                .get(*idx)
                .map(|v| v.type_tag())
                .unwrap_or("invalid-reference"),
            ValueRef::Gc(_) => "gc-object",
        }
    }
    

    pub fn hash_with_context<H: Hasher>(&self, state: &mut H, context: &ValueContext) {
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
                if let Some(shared_val) = context.arena().read().get(*idx) {
                    shared_val.hash_with_context(state, context);
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
        }
    }

    /// Check equality with another ValueRef, with access to the shared arena
    pub fn eq_with_context(&self, other: &Self, context: &ValueContext) -> bool {
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

                match (context.arena().read().get(*a_idx), context.arena().read().get(*b_idx)) {
                    (Some(a_val), Some(b_val)) => a_val.eq_with_context(b_val, context),
                    _ => false, // At least one invalid reference
                }
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
}