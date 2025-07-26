use std::sync::Arc;

use mmtk::util::ObjectReference;
use parking_lot::RwLock;

use crate::runtime::{BlinkVM, EvalResult, ExecutionContext, GoroutineId, GoroutineScheduler, SingleThreadScheduler};

pub static TOKIO_HANDLE: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();
pub static GLOBAL_RUNTIME: std::sync::OnceLock<Arc<BlinkRuntime<SingleThreadScheduler>>> = std::sync::OnceLock::new();


pub fn get_tokio_handle() -> &'static tokio::runtime::Handle {
    TOKIO_HANDLE
        .get()
        .expect("Tokio runtime not initialized - call BlinkRuntime::new() first")
}

// Runtime owns both VM and concrete scheduler
pub struct BlinkRuntime<S: GoroutineScheduler> {
    pub vm: Arc<BlinkVM>,
    pub scheduler: S, // Concrete type, not trait object
    pub execution_context: ExecutionContext,
}

impl<S: GoroutineScheduler> BlinkRuntime<S> {

    pub fn init(&self) {
        let handle = tokio::runtime::Handle::current();
        TOKIO_HANDLE.set(handle.clone()).ok();
    }

    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        TOKIO_HANDLE
            .get()
            .expect("Runtime not initialized - call init() first")
    }

    pub fn spawn_goroutine<F>(&self, task: F) -> GoroutineId
    where
        F: FnOnce(Arc<BlinkVM>) -> EvalResult + Send + 'static,
    {
        let vm = self.vm.clone();
        //let ctx = EvalContext::new(self.vm.global_env.unwrap(), vm);

        // Pass the task directly - the scheduler will call it with &mut EvalContext
        self.scheduler.spawn(vm, task)
    }
}
