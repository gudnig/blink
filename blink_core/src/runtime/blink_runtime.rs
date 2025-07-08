use std::sync::Arc;

use parking_lot::RwLock;

use crate::{eval::{EvalContext, EvalResult}, runtime::{BlinkVM, GoroutineId, GoroutineScheduler, SingleThreadScheduler}, Env};

pub static TOKIO_HANDLE: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

pub fn get_tokio_handle() -> &'static tokio::runtime::Handle {
    TOKIO_HANDLE.get().expect("Tokio runtime not initialized - call BlinkRuntime::new() first")
}

// Runtime owns both VM and concrete scheduler
pub struct BlinkRuntime<S: GoroutineScheduler> {
    pub vm: Arc<BlinkVM>,
    pub scheduler: S,  // Concrete type, not trait object
}

impl<S: GoroutineScheduler> BlinkRuntime<S> {
    pub fn create_context(&self) -> EvalContext {
        EvalContext::new(self.vm.global_env.clone(), self.vm.clone())
    }
    pub fn create_context_with_env(&self, env: Arc<RwLock<Env>>) -> EvalContext {
        EvalContext::new(env, self.vm.clone())
    }

    pub fn init(&self) {
        let handle = tokio::runtime::Handle::current();
        TOKIO_HANDLE.set(handle.clone()).ok(); 
    }

    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        TOKIO_HANDLE.get().expect("Runtime not initialized - call init() first")
    }

    pub fn spawn_goroutine<F>(&self, task: F) -> GoroutineId
    where
        F: FnOnce(&mut EvalContext) -> EvalResult + Send + 'static 
    {
        let vm = self.vm.clone();
        let ctx = EvalContext::new(self.vm.global_env.clone(), vm);
        
        // Pass the task directly - the scheduler will call it with &mut EvalContext
        self.scheduler.spawn(ctx, task)
    }
}