use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::task::Waker;
use dashmap::DashSet;
use parking_lot::Mutex;

use crate::value::{FunctionHandle, FutureHandle};
use crate::ValueRef;

#[derive(Debug)]
pub struct FutureEntry {
    pub generation: u32, // Immutable once created
    pub state: AtomicU8, // 0=pending, 1=fulfilled, 2=rejected
    pub result: Mutex<Option<ValueRef>>, // Protected result storage
    pub waiters: Mutex<FutureWaiters>, // Multi-awaiter support
}

#[derive(Debug)]
pub struct FutureWaiters {
    // For Rust async/await integration
    pub async_wakers: Vec<Waker>,
    // For Blink goroutine continuations
    pub continuations: Vec<SuspendedContinuation>,
}

#[derive(Debug, Clone)]
pub struct SuspendedContinuation {
    pub goroutine_id: u32,
    pub dest_register: u8,
    pub call_stack: Vec<crate::runtime::CallFrame>,
    pub register_stack: Vec<ValueRef>,
    pub current_module: u32,
    pub resume_pc: usize,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FutureState {
    Pending = 0,
    Ready = 1,
    Error = 2,
}

impl From<u8> for FutureState {
    fn from(value: u8) -> Self {
        match value {
            0 => FutureState::Pending,
            1 => FutureState::Ready,
            2 => FutureState::Error,
            _ => panic!("Invalid future state: {}", value),
        }
    }
}

impl FutureEntry {
    fn new(generation: u32) -> Self {
        Self {
            generation,
            state: AtomicU8::new(FutureState::Pending as u8),
            result: Mutex::new(None),
            waiters: Mutex::new(FutureWaiters {
                async_wakers: Vec::new(),
                continuations: Vec::new(),
            }),
        }
    }

    // Atomic completion with exactly-once semantics
    pub fn complete(&self, value: ValueRef) -> Result<Vec<SuspendedContinuation>, String> {
        // Use compare-and-swap for exactly-once completion
        match self.state.compare_exchange(
            FutureState::Pending as u8,
            FutureState::Ready as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // Successfully transitioned to Ready state
                *self.result.lock() = Some(value);
                
                // Wake all waiters
                let mut waiters = self.waiters.lock();
                
                // Wake async tasks
                for waker in waiters.async_wakers.drain(..) {
                    waker.wake();
                }
                
                // Return continuations for goroutine scheduler
                let continuations = waiters.continuations.drain(..).collect();
                
                Ok(continuations)
            }
            Err(_) => Err("Future already completed".to_string()),
        }
    }

    pub fn fail(&self, error: ValueRef) -> Result<Vec<SuspendedContinuation>, String> {
        match self.state.compare_exchange(
            FutureState::Pending as u8,
            FutureState::Error as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                *self.result.lock() = Some(error);
                
                let mut waiters = self.waiters.lock();
                for waker in waiters.async_wakers.drain(..) {
                    waker.wake();
                }
                
                let continuations = waiters.continuations.drain(..).collect();
                Ok(continuations)
            }
            Err(_) => Err("Future already completed".to_string()),
        }
    }

    pub fn try_poll(&self) -> Option<ValueRef> {
        if self.state.load(Ordering::Acquire) != FutureState::Pending as u8 {
            self.result.lock().clone()
        } else {
            None
        }
    }

    pub fn register_async_waker(&self, waker: Waker) -> Option<ValueRef> {
        // Check if already completed (double-checked locking pattern)
        if let Some(result) = self.try_poll() {
            return Some(result);
        }

        let mut waiters = self.waiters.lock();
        
        // Check again under lock
        if let Some(result) = self.try_poll() {
            return Some(result);
        }

        // Still pending - register waker
        waiters.async_wakers.push(waker);
        None
    }

    pub fn register_continuation(&self, continuation: SuspendedContinuation) -> Option<ValueRef> {
        if let Some(result) = self.try_poll() {
            return Some(result);
        }

        let mut waiters = self.waiters.lock();
        
        if let Some(result) = self.try_poll() {
            return Some(result);
        }

        waiters.continuations.push(continuation);
        None
    }
}


pub struct HandleRegistry {
    pub functions: HashMap<u64, ValueRef>, // For native functions (unchanged)
    pub futures: HashMap<u64, FutureEntry>, // For futures
    next_id: AtomicU64,
    next_generation: AtomicU32,
}

impl HandleRegistry {
    pub fn new() -> Self {
        HandleRegistry {
            functions: HashMap::new(),
            futures: HashMap::new(),
            next_id: AtomicU64::new(0),
            next_generation: AtomicU32::new(0),
        }
    }

    // Function registration (unchanged)
    pub fn register_function(&mut self, func: ValueRef) -> FunctionHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.functions.insert(id, func);
        FunctionHandle {
            id: id,
            name: None,
        }
    }

    pub fn resolve_function(&self, handle: &FunctionHandle) -> Option<ValueRef> {
        self.functions.get(&handle.id).cloned()
    }

    pub fn create_future(&mut self) -> FutureHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        
        // Ensure generation fits in 30 bits
        let generation = generation & 0x3FFFFFFF;
        
        let entry = FutureEntry::new(generation);
        self.futures.insert(id, entry);
        
        let handle = FutureHandle { id, generation };
        handle
    }

    pub fn resolve_future(&self, handle: &FutureHandle) -> Option<&FutureEntry> {
        self.futures.get(&handle.id).and_then(|entry| {
            // Validate generation to detect stale handles
            if entry.generation == handle.generation {
                Some(entry.clone())
            } else {
                None // Stale handle
            }
        })
    }

    // Cleanup for GC
    #[inline]
    pub fn gc_sweep_unreachable_futures(&mut self, reachable_handles: &DashSet<FutureHandle>) {
        self.futures.retain(|id, entry| {
            let handle = FutureHandle { id: *id, generation: entry.generation };
            let state = FutureState::from(entry.state.load(Ordering::Acquire));

            // 1. CHEAPEST: Keep all pending futures (single atomic read)
            if state == FutureState::Pending {
                return true;
            }

            // 2. MEDIUM: Keep completed futures with waiters (lock + vec len check)
            {
                let waiters = entry.waiters.lock();
                if !waiters.continuations.is_empty() || !waiters.async_wakers.is_empty() {
                    return true;
                }
            }

            // 3. MOST EXPENSIVE: Check GC reachability (hash set lookup)
            let handle = FutureHandle { id: *id, generation: entry.generation };
            reachable_handles.contains(&handle)
        });
    }
}
