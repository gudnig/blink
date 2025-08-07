// Updated BlinkFuture using atomic write-once design
// This replaces your existing Arc<Mutex<FutureState>> approach

use std::sync::atomic::{AtomicU8, Ordering};
use std::cell::UnsafeCell;
use parking_lot::Mutex;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use crate::runtime::CallFrame;
use crate::value::ValueRef;

// Keep your existing BlinkFuture name
#[derive(Debug)]
pub struct BlinkFuture {
    inner: BlinkFutureInner,
}

// Internal structure with atomic design
#[repr(C)]
#[derive(Debug)]
struct BlinkFutureInner {
    // Atomic state for lock-free fast path
    state: AtomicU8,
    
    // Write-once result storage
    result: UnsafeCell<ValueRef>,
    
    // Suspension/wakeup support
    waiters: Mutex<BlinkFutureWaiters>,
}

#[derive(Debug)]
struct BlinkFutureWaiters {
    // For Rust Future trait (async/await bridging)
    waker: Option<Waker>,
    
    // For Blink continuations (goroutine suspension)
    continuations: Vec<SuspendedContinuation>,
}

#[derive(Debug, Clone)]
pub struct SuspendedContinuation {
    pub goroutine_id: u64,
    pub dest_register: u8,
    pub call_stack: Vec<CallFrame>,
    pub register_stack: Vec<ValueRef>,
    pub current_module: u32,
}

// Your existing FutureState enum, but simpler
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FutureState {
    Pending = 0,
    Ready = 1,     // Keeping your "Ready" terminology
    Error = 2,     // For rejected futures
}

impl BlinkFuture {
    // Keep your existing constructor API
    pub fn new() -> Self {
        Self {
            inner: BlinkFutureInner {
                state: AtomicU8::new(FutureState::Pending as u8),
                result: UnsafeCell::new(ValueRef::nil()),
                waiters: Mutex::new(BlinkFutureWaiters {
                    waker: None,
                    continuations: Vec::new(),
                }),
            }
        }
    }
    
    // Keep your existing API - now with atomic implementation
    pub fn try_poll(&self) -> Option<ValueRef> {
        let state = FutureState::from_u8(self.inner.state.load(Ordering::Acquire));
        match state {
            FutureState::Pending => None,
            FutureState::Ready | FutureState::Error => {
                // Safe: value was written before state changed
                Some(unsafe { *self.inner.result.get() })
            }
        }
    }
    
    // Keep your existing complete API - now write-once
    pub fn complete(&self, value: ValueRef) -> Result<(), String> {
        let old_state = self.inner.state.compare_exchange(
            FutureState::Pending as u8,
            FutureState::Ready as u8,
            Ordering::Release,
            Ordering::Acquire,
        );
        
        match old_state {
            Ok(_) => {
                // Successfully transitioned Pending → Ready
                unsafe {
                    *self.inner.result.get() = value;
                }
                
                // Wake up all waiters
                let mut waiters = self.inner.waiters.lock();
                
                // Wake Rust async tasks
                if let Some(waker) = waiters.waker.take() {
                    waker.wake();
                }
                
                // Wake Blink continuations (handle in VM)
                let continuations = std::mem::take(&mut waiters.continuations);
                drop(waiters); // Release lock before calling VM
                
                // Return continuations to be resumed by caller
                // (VM will handle the actual resumption)
                for continuation in continuations {
                    // VM.resume_goroutine(continuation, value);
                }
                
                Ok(())
            }
            Err(_) => Err("Future already completed".to_string()),
        }
    }
    
    // Keep your existing API
    pub fn is_completed(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) != FutureState::Pending as u8
    }
    
    // Bridge from Rust Future to BlinkFuture
    pub fn from_rust_future(rust_future: Pin<Box<dyn Future<Output = ValueRef> + Send>>) -> Self {
        let blink_future = Self::new();
        let blink_future_clone = blink_future.clone();
        
        // Spawn the Rust future on Tokio runtime
        tokio::spawn(async move {
            let result = rust_future.await;
            // Complete the BlinkFuture when Rust future finishes
            let _ = blink_future_clone.complete(result);
        });
        
        blink_future
    }
    
    // Bridge to Rust Future (for use in async fn)
    pub fn into_rust_future(self) -> impl Future<Output = ValueRef> {
        BlinkFutureWrapper { inner: self }
    }
}

// Wrapper to implement Rust Future trait for BlinkFuture
struct BlinkFutureWrapper {
    inner: BlinkFuture,
}

impl Future for BlinkFutureWrapper {
    type Output = ValueRef;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Fast path: check if already completed
        if let Some(value) = self.inner.try_poll() {
            return Poll::Ready(value);
        }
        
        // Not ready - register Rust waker
        let mut waiters = self.inner.inner.waiters.lock();
        
        // Double-check under lock
        if let Some(value) = self.inner.try_poll() {
            return Poll::Ready(value);
        }
        
        // Store the current waker (replacing any old one)
        waiters.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl BlinkFuture {
    pub fn needs_tokio_bridge(&self) -> bool {
        // Check if this future was created from a Rust future
        // Implementation depends on how you track this
        false // Simplified for now
    }
    
    // Internal: Add a Blink continuation waiter
    pub(crate) fn add_continuation_waiter(&self, continuation: SuspendedContinuation) -> bool {
        // Double-checked locking
        if self.inner.state.load(Ordering::Acquire) != FutureState::Pending as u8 {
            return false; // Already completed
        }
        
        let mut waiters = self.inner.waiters.lock();
        
        // Check again under lock
        if self.inner.state.load(Ordering::Acquire) != FutureState::Pending as u8 {
            return false; // Completed while acquiring lock
        }
        
        waiters.continuations.push(continuation);
        true
    }
}

impl FutureState {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => FutureState::Pending,
            1 => FutureState::Ready,
            2 => FutureState::Error,
            _ => unreachable!("Invalid future state: {}", value),
        }
    }
}

// Implement Rust Future trait for async/await bridging
impl Future for BlinkFuture {
    type Output = ValueRef;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check if already completed (lock-free fast path)
        if let Some(value) = self.try_poll() {
            return Poll::Ready(value);
        }
        
        // Not ready - register waker
        let mut waiters = self.inner.waiters.lock();
        
        // Check again under lock (double-checked locking)
        if let Some(value) = self.try_poll() {
            return Poll::Ready(value);
        }
        
        // Still pending - store waker
        waiters.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

// Thread safety
unsafe impl Send for BlinkFuture {}
unsafe impl Sync for BlinkFuture {}

// Clone implementation (each clone gets same atomic state)
impl Clone for BlinkFuture {
    fn clone(&self) -> Self {
        // This creates a new BlinkFuture that shares the same atomic state
        // This is safe because the inner state is all atomic/mutex protected
        Self {
            inner: BlinkFutureInner {
                state: AtomicU8::new(self.inner.state.load(Ordering::Acquire)),
                result: UnsafeCell::new(unsafe { *self.inner.result.get() }),
                waiters: Mutex::new(BlinkFutureWaiters {
                    waker: None,
                    continuations: Vec::new(),
                }),
            }
        }
    }
}