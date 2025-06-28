use std::sync::Arc;

use generational_arena::{Arena, Index};

use crate::value::{SharedValue, ValueRef};

#[derive(Debug)]
pub struct SharedArena {
    arena: Arena<Arc<SharedValue>>,
}

impl SharedArena {
    pub fn new() -> Self {
        SharedArena {
            arena: Arena::new(),
        }
    }
    
    pub fn alloc(&mut self, value: SharedValue) -> ValueRef {
        let index = self.arena.insert(Arc::new(value));
        ValueRef::Shared(index)
    }
    
    pub fn get(&self, index: Index) -> Option<&Arc<SharedValue>> {
        self.arena.get(index)
    }
    
    pub fn remove(&mut self, index: Index) -> Option<Arc<SharedValue>> {
        self.arena.remove(index)
    }
}
