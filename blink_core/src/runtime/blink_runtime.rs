use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

use mmtk::util::ObjectReference;
use parking_lot::{Mutex, RwLock};

use crate::{
    runtime::{
        BlinkVM, EvalResult, ExecutionContext, Goroutine, GoroutineId, SchedulerAction, SingleThreadedScheduler
    },
    value::{GcPtr, ValueRef}, GoroutineScheduler,
};

pub static TOKIO_HANDLE: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();
pub static GLOBAL_RUNTIME: std::sync::OnceLock<Arc<BlinkRuntime<SingleThreadedScheduler>>> = std::sync::OnceLock::new();

pub fn get_tokio_handle() -> &'static tokio::runtime::Handle {
    TOKIO_HANDLE
        .get()
        .expect("Tokio runtime not initialized - call BlinkRuntime::new() first")
}

// Runtime owns both VM and concrete scheduler
#[derive(Debug)]
pub struct BlinkRuntime<S: GoroutineScheduler> {
    pub vm: Arc<BlinkVM>,
    pub scheduler: Mutex<S>,
    pub execution_context: ExecutionContext,
    scheduler_running: AtomicBool,
}

unsafe impl<S: GoroutineScheduler> Send for BlinkRuntime<S> {}
unsafe impl<S: GoroutineScheduler> Sync for BlinkRuntime<S> {}

impl BlinkRuntime<SingleThreadedScheduler> {
    /// Initialize the global runtime singleton (single-threaded only)
    pub fn init_global(vm: Arc<BlinkVM>, current_module: u32) -> Result<Arc<Self>, String> {
        let runtime = Arc::new(Self::new(vm, current_module));
        runtime.init();
        
        // Start the background scheduler
        runtime.start_background_scheduler();
        
        match GLOBAL_RUNTIME.set(runtime.clone()) {
            Ok(()) => Ok(runtime),
            Err(_) => Err("Global runtime already initialized".to_string()),
        }
    }
}


impl<S: GoroutineScheduler> BlinkRuntime<S> {
    pub fn new(vm: Arc<BlinkVM>, current_module: u32) -> Self {
        let execution_context = ExecutionContext::new(vm.clone(), current_module);
        let scheduler = Mutex::new(S::new());
        
        Self {
            vm,
            scheduler,
            execution_context,
            scheduler_running: AtomicBool::new(false),
        }
    }

    pub fn init(&self) {
        let handle = tokio::runtime::Handle::current();
        TOKIO_HANDLE.set(handle.clone()).ok();
    }


    /// Start the background scheduler thread (native threads only)
    pub fn start_background_scheduler(self: &Arc<Self>) {
        if self.scheduler_running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let runtime = Arc::clone(self);
            
            // Spawn native thread for scheduler (no Tokio dependency)
            thread::spawn(move || {
                runtime.background_scheduler_loop();
            });
        }
    }

    /// Stop the background scheduler thread
    pub fn stop_background_scheduler(&self) {
        self.scheduler_running.store(false, Ordering::SeqCst);
    }

    /// Main background scheduler loop (native thread version)
    fn background_scheduler_loop(&self) {
        while self.scheduler_running.load(Ordering::SeqCst) {
            // Check if there are any goroutines to run
            let has_work = {
                let scheduler = self.scheduler.lock();
                scheduler.has_ready_goroutines()
            };

            if has_work {
                // Run one iteration of the scheduler
                let _ = self.run_scheduler_once();
            } else {
                // No work to do, sleep briefly to avoid busy-waiting
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    pub fn tokio_handle(&self) -> &tokio::runtime::Handle {
        TOKIO_HANDLE
            .get()
            .expect("Runtime not initialized - call init() first")
    }

    pub fn spawn_goroutine(&self, task: ValueRef) -> Result<GoroutineId, String> {
        // Lock the scheduler and spawn the goroutine
        let mut scheduler = self.scheduler.lock();
        let goroutine_id = scheduler.spawn(task)?;
        Ok(goroutine_id)
    }

    /// Run the scheduler until all goroutines complete
    pub fn run_scheduler(&self) -> Result<(), String> {
        let mut scheduler = self.scheduler.lock();
        let vm = self.vm.clone();

        scheduler.run_to_completion(move |goroutine| {
            // Execute a single step for this goroutine
            Self::execute_goroutine_step(goroutine, vm.clone())
        });

        Ok(())
    }

    /// Run a single iteration of the scheduler (for background thread)
    fn run_scheduler_once(&self) -> Result<(), String> {
        let mut scheduler = self.scheduler.lock();
        let vm = self.vm.clone();

        scheduler.run_single_iteration(move |goroutine| {
            // Execute a single step for this goroutine
            Self::execute_goroutine_step(goroutine, vm.clone())
        });

        Ok(())
    }

    /// Execute a single step of a goroutine
    fn execute_goroutine_step(
        goroutine: &mut Goroutine,
        vm: Arc<BlinkVM>,
    ) -> SchedulerAction {
        use crate::runtime::execution_context::{ExecutionContext, FunctionRef};
        use crate::runtime::blink_runtime::SchedulerAction;

        // Create a temporary execution context for this goroutine
        let mut temp_context = ExecutionContext::new(vm, goroutine.current_module);
        temp_context.current_goroutine_id = Some(goroutine.id);
        temp_context.call_stack = goroutine.call_stack.clone();
        temp_context.register_stack = goroutine.register_stack.clone();

        // Try to execute one step
        match temp_context.execute_single_step() {
            Ok(should_continue) => {
                // Update goroutine state from context
                goroutine.call_stack = temp_context.call_stack;
                goroutine.register_stack = temp_context.register_stack;
                goroutine.current_module = temp_context.current_module;

                if should_continue && !goroutine.call_stack.is_empty() {
                    SchedulerAction::Continue
                } else {
                    // Goroutine completed
                    let result = if !goroutine.register_stack.is_empty() {
                        goroutine.register_stack[0]
                    } else {
                        crate::value::ValueRef::nil()
                    };
                    SchedulerAction::Complete(result)
                }
            }
            Err(error) => {
                // Check if this is a suspension
                if error == "SUSPENDED" {
                    // Update goroutine state and block it
                    goroutine.call_stack = temp_context.call_stack;
                    goroutine.register_stack = temp_context.register_stack;
                    goroutine.current_module = temp_context.current_module;
                    SchedulerAction::Block
                } else {
                    // Real error - complete goroutine
                    SchedulerAction::Complete(crate::value::ValueRef::nil())
                }
            }
        }
    }
}
