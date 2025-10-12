use crate::{value::{ChannelHandle, FutureHandle}, Goroutine, GoroutineId, SuspendedContinuation, ValueRef};

#[derive(Debug)]
pub enum SchedulerAction {
    Continue,           // Keep running current goroutine
    Yield,              // Yield to other goroutines
    Block,              // Block on future/channel
    Complete(ValueRef), // Goroutine completed with result
}


pub trait GoroutineScheduler : Send + Sync + 'static {
    fn new() -> Self; 
    fn has_ready_goroutines(&self) -> bool;
    fn spawn(&mut self, function: ValueRef) -> Result<GoroutineId, String>;
    fn run_single_iteration<F>(&mut self, execute_step: F) -> bool
        where F: FnMut(&mut Goroutine) -> SchedulerAction;
    fn run_to_completion<F>(&mut self, execute_step: F)
        where F: FnMut(&mut Goroutine) -> SchedulerAction;

    fn channel_send(&mut self, handle: ChannelHandle, value: ValueRef) -> SchedulerAction;
    fn channel_receive(&mut self, handle: ChannelHandle) -> (SchedulerAction, Option<ValueRef>);
    fn future_add_waiter(&mut self, handle: FutureHandle, continuation: SuspendedContinuation) -> Option<ValueRef>;
    fn complete_future(&mut self, handle: FutureHandle, value: ValueRef) -> Vec<GoroutineId>;
    fn unblock(&mut self, goroutine_id: GoroutineId);
}   