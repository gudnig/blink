use std::hash::{Hash, Hasher};
use crate::{SuspendedContinuation, ValueRef, GLOBAL_VM};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FutureHandle {
    pub(crate) id: u64,
    pub(crate) generation: u32, //  generation counter for stale detection
}

impl Hash for FutureHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.generation.hash(state); // Include generation in hash
    }
}

impl FutureHandle {
    pub fn new(id: u64, generation: u32) -> Self {
        FutureHandle { id, generation }
    }

    pub fn complete(&self, result: ValueRef) -> Result<(), String> {
        let future_val = ValueRef::future_handle(self.id, self.generation);
        let vm = GLOBAL_VM.get().expect("BlinkVM not initialized");
        vm.complete_future_value(future_val, result)?;
        Ok(())
    }

    pub fn try_poll(&self) -> Option<ValueRef> {
        let vm = GLOBAL_VM.get().expect("BlinkVM not initialized");
        let registry = vm.handle_registry.read();
        let entry = registry.resolve_future(self);
        if let Some(entry) = entry {
            entry.try_poll()
        } else {
            None
        }
    }

    pub fn register_continuation(&self, continuation: SuspendedContinuation) -> Result<Option<ValueRef>, String> {
        let vm = GLOBAL_VM.get().expect("BlinkVM not initialized");
        let registry = vm.handle_registry.write();
        let entry = registry.resolve_future(self);
        if let Some(entry) = entry {
            Ok(entry.register_continuation(continuation))
        } else {
            Err("Future not found".to_string())
        }
    }
}