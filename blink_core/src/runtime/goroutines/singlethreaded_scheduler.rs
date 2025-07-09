use std::{
    sync::{atomic::Ordering, Arc},
    thread::{self, JoinHandle},
};

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::{
    eval::{EvalContext, EvalResult},
    runtime::{
        BlinkVM, GoroutineId, GoroutineScheduler, GoroutineState, GoroutineTask, SchedulerState,
    },
    value::ValueRef,
};

pub struct SingleThreadScheduler {
    state: Arc<Mutex<SchedulerState>>,
    tokio_runtime: tokio::runtime::Handle,
    scheduler_thread: Option<JoinHandle<()>>,
}

impl GoroutineScheduler for SingleThreadScheduler {
    fn start(&mut self) {
        let state = self.state.clone();
        let tokio_runtime = self.tokio_runtime.clone();

        let handle = thread::spawn(move || {
            Self::run_scheduler_loop(state, tokio_runtime);
        });

        self.scheduler_thread = Some(handle);
    }

    fn spawn<F>(&self, ctx: EvalContext, task: F) -> GoroutineId
    where
        F: FnOnce(&mut EvalContext) -> EvalResult + Send + 'static,
    {
        let mut state = self.state.lock();
        let id = state.next_id.fetch_add(1, Ordering::SeqCst);

        let goroutine = GoroutineTask {
            id,
            context: ctx,
            state: GoroutineState::Ready {
                task: Box::new(task),
            },
        };

        state.ready_queue.push_back(goroutine);
        id
    }

    // GC coordination methods
    fn stop_for_gc(&self) {
        let state = self.state.lock();
        state.stopped_for_gc.store(true, Ordering::SeqCst);
    }

    fn resume_after_gc(&self) {
        let state = self.state.lock();
        state.stopped_for_gc.store(false, Ordering::SeqCst);
    }

    fn shutdown(&mut self) {
        {
            let state = self.state.lock();
            state.running.store(false, Ordering::SeqCst);
        }

        if let Some(handle) = self.scheduler_thread.take() {
            let _ = handle.join();
        }
    }
}

// Integration with BlinkVM
impl BlinkVM {
    pub fn spawn_goroutine<F>(&self, task: F) -> GoroutineId
    where
        F: FnOnce(&mut EvalContext) -> EvalResult + Send + 'static,
    {
        // Access scheduler from VM - you'll need to add this field
        // self.goroutine_scheduler.spawn(self, task)
        todo!("Add scheduler field to BlinkVM")
    }
}

impl SingleThreadScheduler {
    fn gc_pause(state: &Arc<Mutex<SchedulerState>>) {
        // Pause execution until GC is complete
        loop {
            let state = state.lock();
            if !state.stopped_for_gc.load(Ordering::SeqCst) {
                break;
            }
            drop(state);
            thread::yield_now();
        }
    }
    // Main scheduler loop - runs in dedicated thread
    fn run_scheduler_loop(
        state: Arc<Mutex<SchedulerState>>,
        tokio_runtime: tokio::runtime::Handle,
    ) {
        {
            let state = state.lock();
            state.running.store(true, Ordering::SeqCst);
        }

        loop {
            let should_continue = {
                let state = state.lock();
                state.running.load(Ordering::SeqCst)
            };

            if !should_continue {
                break;
            }

            // Check if GC requested a stop
            {
                let state_guard = state.lock();
                if state_guard.stopped_for_gc.load(Ordering::SeqCst) {
                    drop(state_guard);
                    Self::gc_pause(&state);
                    continue;
                }
            }

            // Process ready tasks
            let goroutine_opt = {
                let mut state = state.lock();
                state.ready_queue.pop_front()
            };

            if let Some(goroutine) = goroutine_opt {
                Self::execute_goroutine(goroutine, &state, &tokio_runtime);
            }

            // Check suspended tasks for completion
            Self::poll_suspended_tasks(&state);

            // If no work, yield briefly
            let has_work = {
                let state = state.lock();
                !state.ready_queue.is_empty() || !state.suspended_tasks.is_empty()
            };

            if !has_work {
                thread::yield_now();
            }
        }
    }

    fn execute_goroutine(
        goroutine: GoroutineTask, // Take ownership instead of &mut
        state: &Arc<Mutex<SchedulerState>>,
        _tokio_runtime: &tokio::runtime::Handle,
    ) -> Option<GoroutineTask> {
        // Return the goroutine if it needs to be suspended
        let mut goroutine = goroutine;

        let result = match std::mem::replace(&mut goroutine.state, GoroutineState::Completed) {
            GoroutineState::Ready { task } => task(&mut goroutine.context),
            GoroutineState::WaitingForTokio {
                mut receiver,
                resume,
            } => {
                match receiver.try_recv() {
                    Ok(value) => {
                        // Future completed, resume execution
                        resume(value, &mut goroutine.context)
                    }
                    Err(oneshot::error::TryRecvError::Empty) => {
                        // Still waiting, return to suspended queue
                        goroutine.state = GoroutineState::WaitingForTokio { receiver, resume };
                        return Some(goroutine); // Return for re-queuing
                    }
                    Err(oneshot::error::TryRecvError::Closed) => {
                        EvalResult::Value(ValueRef::Immediate(crate::value::pack_nil()))
                    }
                }
            }
            GoroutineState::Suspended { future, resume } => {
                if future.needs_tokio_bridge() {
                    let (tx, rx) = oneshot::channel();

                    tokio::spawn(async move {
                        let result = future.await;
                        let _ = tx.send(result);
                    });

                    goroutine.state = GoroutineState::WaitingForTokio {
                        receiver: rx,
                        resume,
                    };
                    return Some(goroutine); // Return for re-queuing
                } else {
                    match future.try_poll() {
                        Some(value) => resume(value, &mut goroutine.context),
                        None => {
                            goroutine.state = GoroutineState::Suspended { future, resume };
                            return Some(goroutine); // Return for re-queuing
                        }
                    }
                }
            }
            GoroutineState::Completed => return None, // Task done
        };

        // Handle the result from execution
        match result {
            EvalResult::Value(_val) => {
                // Task completed
                None // Don't re-queue
            }
            EvalResult::Suspended { future, resume } => {
                if future.needs_tokio_bridge() {
                    let (tx, rx) = oneshot::channel();

                    tokio::spawn(async move {
                        let result = future.await;
                        let _ = tx.send(result);
                    });

                    goroutine.state = GoroutineState::WaitingForTokio {
                        receiver: rx,
                        resume,
                    };
                } else {
                    goroutine.state = GoroutineState::Suspended { future, resume };
                }

                Some(goroutine) // Return for re-queuing
            }
        }
    }

    fn poll_suspended_tasks(state: &Arc<Mutex<SchedulerState>>) {
        let mut state = state.lock();
        let mut i = 0;

        while i < state.suspended_tasks.len() {
            let should_move_to_ready = {
                if let GoroutineState::Suspended { future, .. } = &state.suspended_tasks[i].state {
                    future.try_poll().is_some()
                } else {
                    false
                }
            };

            if should_move_to_ready {
                // Remove and move to ready queue
                let goroutine = state.suspended_tasks.swap_remove(i);
                state.ready_queue.push_back(goroutine);
                // Don't increment i since we removed an element
            } else {
                i += 1;
            }
        }
    }
}
