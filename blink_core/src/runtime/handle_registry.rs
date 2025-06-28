use std::{collections::HashMap, sync::atomic::{AtomicU64, Ordering}};

use crate::{value::{FunctionHandle, FutureHandle}, ValueRef};

pub struct HandleRegistry {
    functions: HashMap<u64, ValueRef>,
    futures: HashMap<u64, ValueRef>,
    next_id: AtomicU64,
}

impl HandleRegistry {
    pub fn new() -> Self {
        HandleRegistry {
            functions: HashMap::new(),
            futures: HashMap::new(),
            next_id: AtomicU64::new(0),
        }
    }
    pub fn register_function(&mut self, func: ValueRef) -> FunctionHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.functions.insert(id, func);
        FunctionHandle {
            id,
            name: None,
        }

    }
    pub fn register_future(&mut self, future: ValueRef) -> FutureHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.futures.insert(id, future);
        FutureHandle {
            id,
        }

    }
    pub fn resolve_function(&self, handle: &FunctionHandle) -> Option<ValueRef> {
        self.functions.get(&handle.id).cloned()

    }
    pub fn resolve_future(&self, handle: &FutureHandle) -> Option<ValueRef> {
        self.futures.get(&handle.id).cloned()
    }
}