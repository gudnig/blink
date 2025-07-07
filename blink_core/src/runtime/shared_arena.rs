use std::sync::Arc;

use generational_arena::{Arena, Index};
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};

use crate::value::{SharedValue, ValueRef};

#[derive(Debug)]
pub struct SharedArena {
    arena: Arena<SharedValue>,       // ← No Arc needed!
    collector: epoch::Collector,
}

impl SharedArena {
    pub fn new() -> Self {
        SharedArena {
            arena: Arena::new(),
            collector: epoch::Collector::new(),
        }
    }
    
    pub fn alloc(&mut self, value: SharedValue) -> ValueRef {
        let index = self.arena.insert(value);
        ValueRef::Shared(index)
    }
    
    pub fn get(&self, index: Index) -> Option<&SharedValue> {
        self.arena.get(index)
    }
    
    pub fn remove(&mut self, index: Index) -> Option<SharedValue> {
        self.arena.remove(index)
    }
}
