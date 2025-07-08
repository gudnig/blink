use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, atomic::{AtomicU64, AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};
use tokio::sync::oneshot;
use parking_lot::RwLock;

use crate::future::BlinkFuture;
use crate::{
    eval::{EvalContext, EvalResult},
    value::ValueRef,
    runtime::BlinkVM,
    env::Env,
};

pub type GoroutineId = u64;

// Box the context to avoid lifetime issues
pub struct GoroutineTask {
    pub id: GoroutineId,
    pub context: EvalContext,
    pub state: GoroutineState,
}

pub enum GoroutineState {
    Ready {
        task: Box<dyn FnOnce(&mut EvalContext) -> EvalResult + Send>,
    },
    Suspended {
        future: BlinkFuture,
        resume: Box<dyn FnOnce(ValueRef, &mut EvalContext) -> EvalResult + Send>,
    },
    WaitingForTokio {
        receiver: oneshot::Receiver<ValueRef>,
        resume: Box<dyn FnOnce(ValueRef, &mut EvalContext) -> EvalResult + Send>,
    },
    Completed,
}


pub trait GoroutineScheduler {
    // Creation and lifecycle
    fn start(&mut self);  
    fn shutdown(&mut self);  

    // Goroutine management  
    fn spawn<F>(&self, ctx: EvalContext, task: F) -> GoroutineId
    where 
        F: FnOnce(&mut EvalContext) -> EvalResult + Send + 'static;
    

    // GC coordination
    fn stop_for_gc(&self);     // Pause for GC
    fn resume_after_gc(&self); // Resume after GC
}

// Shared scheduler state
pub struct SchedulerState {
    pub ready_queue: VecDeque<GoroutineTask>,
    pub suspended_tasks: Vec<GoroutineTask>,
    pub next_id: AtomicU64,
    pub running: AtomicBool,
    pub stopped_for_gc: AtomicBool,
}

