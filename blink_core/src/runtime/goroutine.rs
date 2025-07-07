use std::collections::{HashMap};

pub type GoroutineId = u64;
use tokio::task::JoinHandle;

use std::sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}};

use crate::{error::BlinkError, eval::{EvalContext, EvalResult}, runtime::{AsyncContext, BlinkVM}};

pub struct TokioGoroutineScheduler<'vm> {
    // Track running goroutines
    goroutines: Arc<Mutex<HashMap<GoroutineId, JoinHandle<()>>>>,
    
    // ID generation
    next_id: AtomicU64,
    
    // Tokio runtime handle
    pub runtime: tokio::runtime::Handle,
    vm: &'vm BlinkVM
}

impl<'vm> TokioGoroutineScheduler<'vm> {
    pub fn new(vm: &'vm BlinkVM) -> Self {
        Self {
            goroutines: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            runtime: tokio::runtime::Handle::current(),
            vm,
        }
    }
    
    pub fn spawn_with_context<F>(&self, mut ctx: EvalContext, task: F) -> Result<GoroutineId, BlinkError>
    where 
        F: FnOnce(&mut EvalContext) -> EvalResult + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let ctx = ctx.with_env(ctx.env.clone());
    
        let handle = tokio::spawn(async move {
            ctx.async_ctx = AsyncContext::Goroutine(id);
    
            let mut result = task(&mut ctx);
    
            loop {
                match result {
                    EvalResult::Value(val) => {
                        if let Some(err) = val.get_error() {
                            eprintln!("Goroutine {} failed: {}", id, err);
                        }
                        break;
                    }
                    EvalResult::Suspended { future, resume } => {
                        let val = future.await;
    
                        result = resume(val, &mut ctx);
                    }
                }
            }
        });
    
        self.goroutines.lock().unwrap().insert(id, handle);
        Ok(id)
    }
    
    
    
    pub async fn join(&self, id: GoroutineId) -> Result<(), String> {
        let handle = {
            let mut goroutines = self.goroutines.lock().unwrap();
            goroutines.remove(&id)
        };
        
        if let Some(handle) = handle {
            handle.await.map_err(|e| e.to_string())?;
        }
        
        Ok(())
    }
    
    pub fn abort(&self, id: GoroutineId) -> bool {
        let mut goroutines = self.goroutines.lock().unwrap();
        if let Some(handle) = goroutines.remove(&id) {
            handle.abort();
            true
        } else {
            false
        }
    }
}