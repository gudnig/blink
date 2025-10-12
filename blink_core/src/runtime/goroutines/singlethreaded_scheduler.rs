// Single-threaded cooperative goroutine scheduler
// This gives you the API structure for later multi-threading

use crate::runtime::execution_context::FunctionRef;
use crate::runtime::{set_current_goroutine_id, BlinkVM, CallFrame, TypeTag};
use crate::value::{ChannelHandle, FutureHandle, GcPtr, ValueRef};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use super::SchedulerAction;
use crate::{FutureEntry, Goroutine, GoroutineId, GoroutineScheduler, GoroutineState, SuspendedContinuation};



// === Scheduler ===

#[derive(Debug)]
pub struct SingleThreadedScheduler {
    ready_queue: VecDeque<u32>,
    goroutines: Vec<Option<Goroutine>>,
    current_goroutine: Option<u32>,
    next_id: AtomicU32,
}


impl SingleThreadedScheduler {

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

    

    
}

impl GoroutineScheduler for SingleThreadedScheduler {
    fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
            goroutines: Vec::new(),
            current_goroutine: None,
            next_id: AtomicU32::new(1),
        }
    }

    fn has_ready_goroutines(&self) -> bool {
        !self.ready_queue.is_empty()
    }

    fn spawn(&mut self, function: ValueRef) -> Result<GoroutineId, String> {
        // Your existing implementation
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let goroutine = Goroutine::new(id, function)?;
        set_current_goroutine_id(id);
        self.ready_queue.push_back(id);
        
        while id as usize >= self.goroutines.len() {
            self.goroutines.push(None);
        }
        self.goroutines[id as usize] = Some(goroutine);
        
        Ok(id)
    }

    /// Run scheduler until all goroutines complete
    /// Run a single iteration (execute one step of one goroutine) for background scheduling
    fn run_single_iteration<F>(&mut self, mut execute_step: F) -> bool
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

    fn run_to_completion<F>(&mut self, mut execute_step: F)
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

    fn unblock(&mut self, goroutine_id: GoroutineId) {
        // Your existing implementation (already there)
        if let Some(goroutine) = &mut self.goroutines[goroutine_id as usize] {
            if goroutine.state == GoroutineState::Blocked {
                goroutine.state = GoroutineState::Ready;
                self.ready_queue.push_back(goroutine_id);
            }
        }
    }

    // NEW: Channel operations (to be implemented)
    fn channel_send(&mut self, handle: ChannelHandle, value: ValueRef) -> SchedulerAction {
        // TODO: Implement channel send
        todo!("Channel send not yet implemented")
    }

    fn channel_receive(&mut self, handle: ChannelHandle) -> (SchedulerAction, Option<ValueRef>) {
        // TODO: Implement channel receive
        todo!("Channel receive not yet implemented")
    }

    // NEW: Future operations (to be implemented)
    fn future_add_waiter(&mut self, handle: FutureHandle, continuation: SuspendedContinuation) -> Option<ValueRef> {
        // TODO: Move future logic from VM to here
        todo!("Future add waiter not yet implemented")
    }

    fn complete_future(&mut self, handle: FutureHandle, value: ValueRef) -> Vec<GoroutineId> {
        // TODO: Move future completion from VM to here
        todo!("Future complete not yet implemented")
    }
}