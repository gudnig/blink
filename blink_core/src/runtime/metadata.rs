use std::{collections::HashMap, sync::Arc};

use crate::{eval::EvalContext, value::SourceRange, value::{GcPtr, ValueRef}};

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
    pub fn object_id(&self) -> ValueId {
        todo!()
    }

    
}

impl ValueRef {
    // Generate unique ID for trackable values
    pub fn get_or_create_id(&self) -> Option<ValueId> {
        match self {
            ValueRef::Immediate(_) => None,
            ValueRef::Shared(idx) => Some(idx.into_raw_parts().0 as u64),
            ValueRef::Gc(ptr) => {
                // Future: use GC object ID
                Some(ptr.object_id())
            }
        }
    }
    
    pub fn with_position(self, pos: SourceRange, ctx: &mut EvalContext) -> Self {
        if let Some(id) = self.get_or_create_id() {
            ctx.value_metadata.write().set_position(id, pos);
        }
        self
    }
}