// Single-threaded cooperative goroutine scheduler
// This gives you the API structure for later multi-threading

use crate::runtime::execution_context::FunctionRef;
use crate::runtime::{set_current_goroutine_id, BlinkVM, CallFrame, TypeTag};
use crate::value::{FutureHandle, GcPtr, ValueRef};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use crate::{FutureEntry, SuspendedContinuation};

pub type GoroutineId = u32;

// === Goroutine State ===

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoroutineState {
    Ready,     // Can be scheduled
    Running,   // Currently executing
    Blocked,   // Waiting on future/channel
    Completed, // Finished execution
}

#[derive(Debug)]
pub struct Goroutine {
    pub id: u32,
    pub state: GoroutineState,
    pub call_stack: Vec<crate::runtime::execution_context::CallFrame>,
    pub register_stack: Vec<ValueRef>,
    pub current_module: u32,
    pub instruction_pointer: usize,
}

impl Goroutine {
    pub fn new(id: u32, initial_function: ValueRef) -> Result<Self, String> {
        // Create initial call frame from the function
        let call_frame = Self::create_initial_frame(initial_function)?;
        let current_module = call_frame.current_module;

        // Allocate registers for the initial function
        let mut register_stack = Vec::new();
        let reg_count = match &call_frame.func {
            crate::runtime::execution_context::FunctionRef::CompiledFunction(compiled_fn, _) => {
                compiled_fn.register_count as usize
            }
            crate::runtime::execution_context::FunctionRef::Closure(closure_obj, _) => {
                let template_fn = GcPtr::new(closure_obj.template).read_callable();
                template_fn.register_count as usize
            }
            crate::runtime::execution_context::FunctionRef::Native(_) => {
                1 // Native functions need at least 1 register for return value
            }
        };

        // Pre-allocate registers with nil values (same as normal execution)
        for _ in 0..reg_count {
            register_stack.push(ValueRef::nil());
        }

        Ok(Self {
            id,
            state: GoroutineState::Ready,
            call_stack: vec![call_frame],
            register_stack,
            current_module,
            instruction_pointer: 0,
        })
    }


    fn create_initial_frame(func_value: ValueRef) -> Result<CallFrame, String> {
        match func_value {
            ValueRef::Heap(heap) => {
                let type_tag = heap.type_tag();
                let obj_ref = heap.0;
                match type_tag {
                    TypeTag::UserDefinedFunction | TypeTag::Macro => {
                        let compiled_func = heap.read_callable();
                        let module = compiled_func.module;
                        Ok(CallFrame {
                            func: FunctionRef::CompiledFunction(compiled_func, Some(obj_ref)),
                            pc: 0,
                            reg_start: 0,
                            reg_count: 0, // Will be set when registers are allocated
                            current_module: module,
                        })
                    }
                    TypeTag::Closure => {
                        let closure_obj = heap.read_closure();
                        let template_fn = GcPtr::new(closure_obj.template).read_callable();
                        let module = template_fn.module;
                        Ok(CallFrame {
                            func: FunctionRef::Closure(closure_obj, Some(obj_ref)),
                            pc: 0,
                            reg_start: 0,
                            reg_count: 0, // Will be set when registers are allocated
                            current_module: module,
                        })
                    }
                    _ => Err(format!(
                        "Invalid function value for goroutine: {:?}",
                        func_value
                    )),
                }
            }
            ValueRef::Handle(native) => {
                Ok(CallFrame {
                    func: FunctionRef::Native(native),
                    pc: 0,
                    reg_start: 0,
                    reg_count: 0,      // Will be set when registers are allocated
                    current_module: 0, // Native functions don't have modules
                })
            }
            _ => Err(format!(
                "Invalid function value for goroutine: {:?}",
                func_value
            )),
        }
    }
}

// === Scheduler ===

#[derive(Debug)]
pub struct SingleThreadedScheduler {
    ready_queue: VecDeque<u32>,
    goroutines: Vec<Option<Goroutine>>,
    current_goroutine: Option<u32>,
    next_id: AtomicU32,
}

impl SingleThreadedScheduler {
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
            goroutines: Vec::new(),
            current_goroutine: None,
            next_id: AtomicU32::new(1),
        }
    }

    /// Spawn a new goroutine
    pub fn spawn(&mut self, function: ValueRef) -> Result<u32, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let goroutine = Goroutine::new(id, function)?;

        // Set the goroutine ID for locking purposes
        set_current_goroutine_id(id);

        // Add to ready queue
        self.ready_queue.push_back(id);

        // Store in goroutines vector
        // Make sure the goroutines vector is long enough
        while id as usize >= self.goroutines.len() {
            self.goroutines.push(None);
        }
        self.goroutines[id as usize] = Some(goroutine);

        Ok(id)
    }




    // Better version that accepts the result value
    pub fn resume_goroutine_with_result(&mut self, continuation: SuspendedContinuation, result: ValueRef) {
        let goroutine_id = continuation.goroutine_id;

        let mut goroutine = Goroutine {
            id: goroutine_id,
            state: GoroutineState::Ready,
            call_stack: continuation.call_stack,
            register_stack: continuation.register_stack,
            current_module: continuation.current_module,
            instruction_pointer: continuation.resume_pc,
        };

        // Store the result in the destination register
        let dest_reg = continuation.dest_register as usize;
        if let Some(frame) = goroutine.call_stack.last() {
            let reg_index = frame.reg_start + dest_reg;
            if reg_index < goroutine.register_stack.len() {
                goroutine.register_stack[reg_index] = result;
            }
        }

        // MISSING: Store the goroutine back in the scheduler's storage
        while goroutine_id as usize >= self.goroutines.len() {
            self.goroutines.push(None);
        }
        self.goroutines[goroutine_id as usize] = Some(goroutine);

        self.ready_queue.push_back(goroutine_id);
    }

    /// Check if there are any ready goroutines
    pub fn has_ready_goroutines(&self) -> bool {
        !self.ready_queue.is_empty()
    }

    /// Get the next goroutine to run
    pub fn schedule(&mut self) -> Option<u32> {
        if let Some(next_id) = self.ready_queue.pop_front() {
            if let Some(goroutine) = &mut self.goroutines[next_id as usize] {
                goroutine.state = GoroutineState::Running;
                self.current_goroutine = Some(next_id);

                // Update thread-local goroutine ID
                set_current_goroutine_id(next_id);

                return Some(next_id);
            }
        }
        None
    }

    /// Yield current goroutine back to ready queue
    pub fn yield_current(&mut self) {
        if let Some(current_id) = self.current_goroutine {
            if let Some(goroutine) = &mut self.goroutines[current_id as usize] {
                goroutine.state = GoroutineState::Ready;
                self.ready_queue.push_back(current_id);
            }
            self.current_goroutine = None;
        }
    }

    /// Block current goroutine (waiting on future/channel)
    pub fn block_current(&mut self) {
        if let Some(current_id) = self.current_goroutine {
            if let Some(goroutine) = &mut self.goroutines[current_id as usize] {
                goroutine.state = GoroutineState::Blocked;
            }
            self.current_goroutine = None;
        }
    }

    /// Unblock a goroutine (future completed/channel ready)
    pub fn unblock(&mut self, goroutine_id: u32) {
        if let Some(goroutine) = &mut self.goroutines[goroutine_id as usize] {
            if goroutine.state == GoroutineState::Blocked {
                goroutine.state = GoroutineState::Ready;
                self.ready_queue.push_back(goroutine_id);
            }
        }
    }

    /// Complete current goroutine (fire-and-forget - result is discarded)
    pub fn complete_current(&mut self) {
        if let Some(current_id) = self.current_goroutine {
            if let Some(goroutine) = &mut self.goroutines[current_id as usize] {
                goroutine.state = GoroutineState::Completed;
                // Fire-and-forget: don't store result, just mark as completed
            }
            self.current_goroutine = None;
        }
    }

    /// Get current running goroutine
    pub fn current(&self) -> Option<&Goroutine> {
        self.current_goroutine
            .and_then(|id| self.goroutines[id as usize].as_ref())
    }

    /// Get mutable reference to current goroutine
    pub fn current_mut(&mut self) -> Option<&mut Goroutine> {
        self.current_goroutine
            .and_then(|id| self.goroutines[id as usize].as_mut())
    }

    /// Check if scheduler has work to do
    pub fn has_work(&self) -> bool {
        !self.ready_queue.is_empty() || self.current_goroutine.is_some()
    }

    /// Run scheduler until all goroutines complete
    /// Run a single iteration (execute one step of one goroutine) for background scheduling
    pub fn run_single_iteration<F>(&mut self, mut execute_step: F) -> bool
    where
        F: FnMut(&mut Goroutine) -> SchedulerAction,
    {
        if let Some(goroutine_id) = self.schedule() {
            let action = {
                let goroutine = self.current_mut().unwrap();
                execute_step(goroutine)
            };

            match action {
                SchedulerAction::Continue => {
                    // Keep this goroutine scheduled for next iteration
                    self.yield_current();
                }
                SchedulerAction::Yield => {
                    self.yield_current();
                }
                SchedulerAction::Block => {
                    self.block_current();
                }
                SchedulerAction::Complete(_result) => {
                    // Fire-and-forget: discard result
                    self.complete_current();
                }
            }
            true // Work was done
        } else {
            false // No work to do
        }
    }

    pub fn run_to_completion<F>(&mut self, mut execute_step: F)
    where
        F: FnMut(&mut Goroutine) -> SchedulerAction,
    {
        while self.has_work() {
            if let Some(goroutine_id) = self.schedule() {
                loop {
                    let action = {
                        let goroutine = self.current_mut().unwrap();
                        execute_step(goroutine)
                    };

                    match action {
                        SchedulerAction::Continue => {
                            // Keep running this goroutine
                        }
                        SchedulerAction::Yield => {
                            self.yield_current();
                            break;
                        }
                        SchedulerAction::Block => {
                            self.block_current();
                            break;
                        }
                        SchedulerAction::Complete(_result) => {
                            // Fire-and-forget: discard result
                            self.complete_current();
                            break;
                        }
                    }
                }
            }
        }
    }
}

// === Scheduler Action ===

#[derive(Debug)]
pub enum SchedulerAction {
    Continue,           // Keep running current goroutine
    Yield,              // Yield to other goroutines
    Block,              // Block on future/channel
    Complete(ValueRef), // Goroutine completed with result
}
