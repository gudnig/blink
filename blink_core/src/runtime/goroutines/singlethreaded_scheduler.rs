// Single-threaded cooperative goroutine scheduler
// This gives you the API structure for later multi-threading

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use crate::value::ValueRef;
use crate::runtime::{set_current_goroutine_id, BlinkVM, CallFrame};

// === Goroutine State ===

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoroutineState {
    Ready,      // Can be scheduled
    Running,    // Currently executing
    Blocked,    // Waiting on future/channel
    Completed,  // Finished execution
}

#[derive(Debug)]
pub struct Goroutine {
    pub id: u32,
    pub state: GoroutineState,
    pub call_stack: Vec<CallFrame>,
    pub register_stack: Vec<ValueRef>,
    pub current_module: u32,
    pub instruction_pointer: usize,
    pub result: Option<ValueRef>,
}

impl Goroutine {
    pub fn new(id: u32, initial_function: ValueRef) -> Self {
        Self {
            id,
            state: GoroutineState::Ready,
            call_stack: todo!(),//vec![CallFrame::new(initial_function, 0)],
            register_stack: Vec::new(),
            current_module: 0,
            instruction_pointer: 0,
            result: None,
        }
    }
}

// === Scheduler ===

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
    pub fn spawn(&mut self, function: ValueRef) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let goroutine = Goroutine::new(id, function);
        
        // Set the goroutine ID for locking purposes
        set_current_goroutine_id(id);
        
        // Add to ready queue
        self.ready_queue.push_back(id);
        
        // Store in goroutines vector
        //make sure the goroutines vector is long enough
        if id as usize >= self.goroutines.len() {
            let additional = (id as usize) - self.goroutines.len() + 1;
            self.goroutines.reserve(additional);

        }
        self.goroutines[id as usize] = Some(goroutine);
        
        id
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
    
    /// Complete current goroutine
    pub fn complete_current(&mut self, result: ValueRef) {
        if let Some(current_id) = self.current_goroutine {
            if let Some(goroutine) = &mut self.goroutines[current_id as usize] {
                goroutine.state = GoroutineState::Completed;
                goroutine.result = Some(result);
            }
            self.current_goroutine = None;
        }
    }
    
    /// Get current running goroutine
    pub fn current(&self) -> Option<&Goroutine> {
        self.current_goroutine.and_then(|id| {
            self.goroutines[id as usize].as_ref()
        })
    }
    
    /// Get mutable reference to current goroutine
    pub fn current_mut(&mut self) -> Option<&mut Goroutine> {
        self.current_goroutine.and_then(|id| {
            self.goroutines[id as usize].as_mut()
        })
    }
    
    /// Check if scheduler has work to do
    pub fn has_work(&self) -> bool {
        !self.ready_queue.is_empty() || self.current_goroutine.is_some()
    }
    
    /// Run scheduler until all goroutines complete
    pub fn run_to_completion<F>(&mut self, mut execute_step: F) 
    where 
        F: FnMut(&mut Goroutine) -> SchedulerAction
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
                        SchedulerAction::Complete(result) => {
                            self.complete_current(result);
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
