use std::{collections::HashMap, sync::Arc};

use crate::{value::SourceRange, value::{GcPtr, ValueRef}};

pub type ValueId = u64;

pub struct TypeInfo {
    name: String,
    kind: String,
}

pub struct ProfileData {
    hits: u64,
    time: u64,
}


pub struct ValueMetadataStore {
    positions: HashMap<ValueId, SourceRange>,
    type_info: HashMap<ValueId, TypeInfo>,      // For future type inference
    profiling: HashMap<ValueId, ProfileData>,   // For JIT decisions
    debug_names: HashMap<ValueId, String>,      // For better debugging
}

impl ValueMetadataStore {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            type_info: HashMap::new(),
            profiling: HashMap::new(),
            debug_names: HashMap::new(),
        }
    }

    pub fn set_position(&mut self, id: ValueId, pos: SourceRange) {
        self.positions.insert(id, pos);
    }

    pub fn get_position(&self, id: ValueId) -> Option<SourceRange> {
        self.positions.get(&id).cloned()
    }
}

impl GcPtr {
    pub fn object_id(&self) -> Option<ValueId> {
        Some(self.0.to_raw_address().as_usize() as u64)
    }

    
}

impl ValueRef {
    // Generate unique ID for trackable values
    pub fn get_or_create_id(&self) -> Option<ValueId> {
        match self {
            ValueRef::Immediate(_) => None,
            ValueRef::Heap(ptr) => {
                        ptr.object_id()
                    }
            ValueRef::Native(ptr) => Some(*ptr as u64),
        }
    }
}