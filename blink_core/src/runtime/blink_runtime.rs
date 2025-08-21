use std::sync::Arc;

use mmtk::util::ObjectReference;
use parking_lot::RwLock;

use crate::{
    runtime::{
        BlinkVM, EvalResult, ExecutionContext, GoroutineId, GoroutineScheduler,
        SingleThreadedScheduler,
    },
    value::ValueRef,
};

pub static TOKIO_HANDLE: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();
pub static GLOBAL_RUNTIME: std::sync::OnceLock<Arc<BlinkRuntime>> = std::sync::OnceLock::new();

pub fn get_tokio_handle() -> &'static tokio::runtime::Handle {
    TOKIO_HANDLE
        .get()
        .expect("Tokio runtime not initialized - call BlinkRuntime::new() first")
}

// Runtime owns both VM and concrete scheduler
pub struct BlinkRuntime {
    pub vm: Arc<BlinkVM>,
    pub scheduler: SingleThreadedScheduler,
    pub execution_context: ExecutionContext,
}

impl BlinkRuntime {
    pub fn init(&self) {
        let handle = tokio::runtime::Handle::current();
        TOKIO_HANDLE.set(handle.clone()).ok();
    }

    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        TOKIO_HANDLE
            .get()
            .expect("Runtime not initialized - call init() first")
    }

    pub fn spawn_goroutine(&self, task: ValueRef) -> GoroutineId {
        todo!()
    }
}
