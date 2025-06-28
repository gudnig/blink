use crate::value::unpack_immediate;
use crate::{runtime::SharedArena, value::ValueRef};
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::sync::Arc;


mod hash_map;
mod hash_set;
pub use hash_map::*;
pub use hash_set::*;
use parking_lot::RwLock;

/// Unified context providing access to all memory management systems
#[derive(Clone, Debug)]
pub struct ValueContext {
    arena: Arc<RwLock<SharedArena>>,
    // gc_context: Arc<GcContext>,  // Future
}


impl ValueContext {
    pub fn new(arena: Arc<RwLock<SharedArena>>) -> Self {
        Self { arena }
    }

    pub fn arena(&self) -> &RwLock<SharedArena> {
        &self.arena
    }

    // Future methods:
    // pub fn gc_context(&self) -> &GcContext { &self.gc_context }
    // pub fn with_gc(arena: Arc<SharedArena>, gc: Arc<GcContext>) -> Self { ... }
}

/// Wrapper that makes ValueRef hashable and comparable with full context
#[derive(Clone, Debug)]
pub struct ContextualValueRef {
    value: ValueRef,
    context: &ValueContext,
}

impl ContextualValueRef {
    pub fn new(value: ValueRef, context: &ValueContext) -> Self {
        Self { value, context }
    }

    pub fn value(&self) -> &ValueRef { &self.value }
    pub fn context(&self) -> &ValueContext { &self.context }
    pub fn into_value(self) -> ValueRef { self.value }
}

impl Hash for ContextualValueRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash_with_context(state, &self.context);
    }
}

impl PartialEq for ContextualValueRef {
    fn eq(&self, other: &Self) -> bool {
        // Ensure contexts are compatible (same arena, same GC context)
        std::sync::Arc::ptr_eq(&self.context.arena, &other.context.arena) &&
        // Future: self.context.gc_context == other.context.gc_context &&
        self.value.eq_with_context(&other.value, &self.context)
    }
}

impl Eq for ContextualValueRef {}

impl Display for ContextualValueRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.value {
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(packed);
                write!(f, "{}", unpacked)
            }
            ValueRef::Gc(gc_ptr) => todo!(),
            ValueRef::Shared(index) => {
                let shared_val = self.context.arena().read().get(index);
                write!(f, "{}", shared_val)
            }
        }
        
    }
}