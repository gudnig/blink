use std::collections::{HashMap};




pub type GoroutineId = u64;
use tokio::task::JoinHandle;

use std::sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}};

use crate::{runtime::AsyncContext, eval::{EvalContext, EvalResult}};

pub struct TokioGoroutineScheduler {
    // Track running goroutines
    goroutines: Arc<Mutex<HashMap<GoroutineId, JoinHandle<()>>>>,
    
    // ID generation
    next_id: AtomicU64,
    
    // Tokio runtime handle
    runtime: tokio::runtime::Handle,
}

impl TokioGoroutineScheduler {
    pub fn new() -> Self {
        Self {
            goroutines: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
            runtime: tokio::runtime::Handle::current(),
        }
    }
    
    pub fn spawn_with_context<F>(&self, mut ctx: EvalContext, task: F) -> GoroutineId 
    where 
        F: FnOnce(&mut EvalContext) -> EvalResult + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
    
        let handle = tokio::spawn(async move {
            ctx.async_ctx = AsyncContext::Goroutine(id);
    
            let mut result = task(&mut ctx);
    
            loop {
                match result {
                    EvalResult::Value(val) => {
                        // Task errored
                        if ctx.is_err(&val) {
                            let err = ctx.get_err(&val);
                            eprintln!("Goroutine {} failed: {}", id, err);
                            break;
                        }
                        // Task completed successfully
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
        id
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