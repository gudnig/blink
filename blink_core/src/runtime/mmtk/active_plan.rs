// blink_core/src/runtime/mmtk/active_plan.rs
// Complete thread-safe mutator implementation that avoids deadlock

use mmtk::util::alloc::{AllocationOptions, OnAllocationFail};
use mmtk::util::Address;
use mmtk::{util::ObjectReference, vm::ActivePlan, Mutator, ObjectQueue};
use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use parking_lot::{Mutex, Condvar};
use std::sync::{OnceLock, Arc};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::ThreadId;

use crate::runtime::{BlinkVM, ObjectHeader, TypeTag};

// Thread-safe wrapper for UnsafeCell<Box<Mutator>>
struct ThreadSafeMutator {
    inner: UnsafeCell<Box<Mutator<BlinkVM>>>,
}

impl ThreadSafeMutator {
    fn new(mutator: Box<Mutator<BlinkVM>>) -> Self {
        Self {
            inner: UnsafeCell::new(mutator),
        }
    }

    fn get(&self) -> *mut Mutator<BlinkVM> {
        unsafe { (*self.inner.get()).as_mut() }
    }
}

// SAFETY: We manually ensure thread safety through GC coordination
unsafe impl Send for ThreadSafeMutator {}
unsafe impl Sync for ThreadSafeMutator {}

// Thread-local mutator storage - each thread gets its own mutator
thread_local! {
    static MUTATOR: RefCell<Option<Arc<ThreadSafeMutator>>> = RefCell::new(None);
    static THREAD_TLS: RefCell<Option<mmtk::util::VMMutatorThread>> = RefCell::new(None);
}

// Global mutator tracking - store Arc<ThreadSafeMutator> for safe sharing
static GLOBAL_MUTATORS: OnceLock<Mutex<HashMap<ThreadId, Arc<ThreadSafeMutator>>>> = OnceLock::new();
static ACTIVE_MUTATOR_THREADS: OnceLock<Mutex<Vec<ThreadId>>> = OnceLock::new();
static THREAD_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
static GC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

// GC coordination - used to park/unpark mutator threads
pub static GC_COORDINATOR: OnceLock<Arc<(Mutex<bool>, Condvar)>> = OnceLock::new();

static GC_REQUEST: OnceLock<Mutex<bool>> = OnceLock::new();

pub struct BlinkActivePlan;

impl BlinkActivePlan {
    /// Create VMMutatorThread for current thread
    fn create_vm_mutator_thread() -> mmtk::util::VMMutatorThread {
        let _thread_id = std::thread::current().id();
        let unique_id = THREAD_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        
        let opaque = mmtk::util::OpaquePointer::from_address(
            unsafe { mmtk::util::Address::from_usize(unique_id) }
        );
        let vm_thread = mmtk::util::VMThread(opaque);
        let tls = mmtk::util::VMMutatorThread(vm_thread);
        
        // Store in thread-local for is_mutator() checks
        THREAD_TLS.with(|cell| {
            *cell.borrow_mut() = Some(tls);
        });
        
        tls
    }

    pub fn alloc_with_gc_request(mutator: &mut Mutator<BlinkVM>, type_tag: &TypeTag, data_size: &usize) -> ObjectReference {
        let total_size = (ObjectHeader::SIZE + data_size + 7) & !7;
        let options = AllocationOptions {
            on_fail: OnAllocationFail::RequestGC
        };


        let start = mmtk::memory_manager::alloc_with_options(
            mutator,
            total_size,
            8,
            0,
            mmtk::AllocationSemantics::Default,
            options
        );

        let header_ptr = start.to_mut_ptr::<ObjectHeader>();
        unsafe { std::ptr::write(header_ptr, ObjectHeader::new(*type_tag, *data_size)) };
    
        // Return reference pointing after header
        ObjectReference::from_raw_address(start + ObjectHeader::SIZE).unwrap()
    }

    pub fn alloc_with_oom_failure(mutator: &mut Mutator<BlinkVM>, type_tag: &TypeTag, data_size: &usize) -> ObjectReference {
        let total_size = (ObjectHeader::SIZE + data_size + 7) & !7;

        let options = AllocationOptions {
            on_fail: OnAllocationFail::ReturnFailure
        };

        let mut start= unsafe { Address::zero() };
        while start.is_zero() {
            start = mmtk::memory_manager::alloc_with_options(
                mutator,
                total_size,
                8,
                0,
                mmtk::AllocationSemantics::Default,
                options
            );
            if start.is_zero() {
                // allocation failed yield so gc workers can schedule/finish gc
                std::thread::yield_now();
                continue;
            }
            
            let header_ptr = start.to_mut_ptr::<ObjectHeader>();
            unsafe { std::ptr::write(header_ptr, ObjectHeader::new(*type_tag, *data_size)) };
        }
        unsafe {
            ObjectReference::from_raw_address_unchecked(start + ObjectHeader::SIZE )   
        }
    }

    /// Main allocation function - creates mutator on first use
    pub fn with_mutator<T>(f: impl FnOnce(&mut Mutator<BlinkVM>, Box<dyn Fn( &mut Mutator<BlinkVM>, &TypeTag, &usize) -> ObjectReference>) -> T) -> T {
        // First check if GC is in progress and block if needed
        Self::gc_poll();
        
        MUTATOR.with(|mutator_cell| {
            let request_gc = {
                // Lock to figure out if this thread should request GC
                let mut lock = GC_REQUEST.get_or_init(|| Mutex::new(true));
                let mut request_gc = lock.lock();
                if *request_gc {
                    // If true the thread should request GC so we set it to false so other threads don't request GC
                    // This thread is now responsible for resetting the request_gc flag
                    *request_gc = false;
                    true
                } else {
                    false
                }
                // Then drop the lock so other threads can check if they should request GC
            };

            let alloc_options: AllocationOptions = if request_gc {
                AllocationOptions {
                    on_fail: OnAllocationFail::RequestGC
                }
            } else {
                AllocationOptions {
                    on_fail: OnAllocationFail::ReturnFailure
                }
            };

            let mut mutator_ref = mutator_cell.borrow_mut();
            
            if mutator_ref.is_none() {
                // First time - create mutator for this thread
                let tls = Self::create_vm_mutator_thread();
                let mmtk = crate::runtime::GLOBAL_MMTK.get()
                    .expect("MMTK not initialized");
                
                let boxed_mutator = mmtk::memory_manager::bind_mutator(mmtk, tls);
                let thread_safe_mutator = Arc::new(ThreadSafeMutator::new(boxed_mutator));
                
                // Store locally for fast access
                *mutator_ref = Some(thread_safe_mutator.clone());
                
                // Register globally for GC coordination
                Self::register_mutator_thread(std::thread::current().id(), thread_safe_mutator);
                
                println!("Created mutator for thread {:?}", std::thread::current().id());
            }
            
            // Access the mutator through ThreadSafeMutator
            let arc_mutator = mutator_ref.as_ref().unwrap();
            
            // SAFETY: This is safe because:
            // 1. We're in the owning thread of this mutator
            // 2. GC coordination ensures no concurrent access during GC
            // 3. Only this thread accesses this mutator during normal operation
            let mutator = unsafe { &mut *arc_mutator.get() };
            if request_gc {
                f(mutator, Box::new(Self::alloc_with_gc_request))
            } else {
                f(mutator, Box::new(Self::alloc_with_oom_failure))
            }
            
        })
    }

    /// Register a mutator thread for GC coordination
    fn register_mutator_thread(thread_id: ThreadId, mutator: Arc<ThreadSafeMutator>) {
        // Add to active threads list
        let active_threads = ACTIVE_MUTATOR_THREADS.get_or_init(|| Mutex::new(Vec::new()));
        let mut threads = active_threads.lock();
        if !threads.contains(&thread_id) {
            threads.push(thread_id);
        }
        
        // Store globally for GC access
        let global_mutators = GLOBAL_MUTATORS.get_or_init(|| Mutex::new(HashMap::new()));
        global_mutators.lock().insert(thread_id, mutator);
    }

    /// Block current thread if GC is in progress (for stop-the-world coordination)
    pub fn gc_poll() {
        let coordinator = GC_COORDINATOR.get_or_init(|| {
            Arc::new((Mutex::new(false), Condvar::new()))
        });
        
        let (lock, cvar) = &**coordinator;
        let mut gc_in_progress = lock.lock();
        
        while *gc_in_progress {
            println!("Mutator {:?} parking for GC...", std::thread::current().id());
            cvar.wait(&mut gc_in_progress);
            println!("Mutator {:?} resumed after GC", std::thread::current().id());
        }
    }

    /// Signal all mutators to stop and visit them (called by GC worker)
    pub fn stop_all_mutators_impl<F>(mut mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        println!("GC Worker: Stopping all mutators");
        
        // Acquire GC lock to ensure exclusive access
        {
            let lock_arc = GC_COORDINATOR.get_or_init(|| {
                Arc::new((Mutex::new(false), Condvar::new()))
            });
            let (lock, cvar) = &**lock_arc;
            let mut lock = lock.lock();
            *lock = true;
            GC_IN_PROGRESS.store(true, Ordering::SeqCst);
            
        }
        // Now visit all mutators for stack scanning
        if let Some(global_mutators) = GLOBAL_MUTATORS.get() {
            let mutator_map = global_mutators.lock();
            
            for (_thread_id, arc_mutator) in mutator_map.iter() {
                // SAFETY: This is safe because:
                // 1. We hold the GC lock ensuring exclusive access
                // 2. All mutators are stopped via the GC coordinator
                // 3. This is during stop-the-world GC phase
                let mutator = unsafe { &mut *arc_mutator.get() };
                
                // Extend lifetime for the visitor
                let static_mutator: &'static mut Mutator<BlinkVM> = unsafe {
                    std::mem::transmute::<&mut Mutator<BlinkVM>, &'static mut Mutator<BlinkVM>>(mutator)
                };
                
                mutator_visitor(static_mutator);
            }
        }
        
        println!("GC Worker: All mutators stopped and visited");
    }

    /// Resume all mutators (called by GC worker)
    pub fn resume_all_mutators_impl() {
        println!("GC Worker: Resuming all mutators");
        
        let coordinator = GC_COORDINATOR.get_or_init(|| {
            Arc::new((Mutex::new(false), Condvar::new()))
        });
        let (lock, cvar) = &**coordinator;
        
        {
            let mut gc_in_progress = lock.lock();
            *gc_in_progress = false;
            GC_IN_PROGRESS.store(false, Ordering::SeqCst);
        }
        cvar.notify_all();
        
        println!("GC Worker: All mutators resumed");
        // GC lock is released when _gc_guard goes out of scope
    }

    /// Get all active mutator threads (for debugging)
    pub fn get_active_threads() -> Vec<ThreadId> {
        ACTIVE_MUTATOR_THREADS
            .get()
            .map(|threads| threads.lock().clone())
            .unwrap_or_default()
    }
}

impl ActivePlan<BlinkVM> for BlinkActivePlan {
    fn is_mutator(tls: mmtk::util::VMThread) -> bool {
        THREAD_TLS.with(|stored_tls| {
            stored_tls.borrow()
                .map_or(false, |stored| stored.0 == tls)
        })
    }

    fn mutator(_tls: mmtk::util::VMMutatorThread) -> &'static mut Mutator<BlinkVM> {
        // This should only be called from MMTk during GC when we know the mutator exists
        MUTATOR.with(|mutator_cell| {
            let mutator_ref = mutator_cell.borrow();
            let arc_mutator = mutator_ref.as_ref().expect("No mutator for current thread");
            
            // SAFETY: This is safe because:
            // 1. This is only called during GC when all mutators are stopped
            // 2. We have exclusive access through GC coordination
            // 3. The mutator lives as long as the thread
            let mutator = unsafe { &mut *arc_mutator.get() };
            
            // Extend lifetime for MMTk
            unsafe { 
                std::mem::transmute::<&mut Mutator<BlinkVM>, &'static mut Mutator<BlinkVM>>(mutator)
            }
        })
    }

    fn number_of_mutators() -> usize {
        ACTIVE_MUTATOR_THREADS
            .get()
            .map(|threads| threads.lock().len())
            .unwrap_or(0)
    }

    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<BlinkVM>> + 'a> {
        // This is called by MMTk during GC to iterate over all mutators
        // We'll collect all mutators and return them as an iterator
        
        let mut mutators = Vec::new();
        
        if let Some(global_mutators) = GLOBAL_MUTATORS.get() {
            let mutator_map = global_mutators.lock();
            
            for (_thread_id, arc_mutator) in mutator_map.iter() {
                // SAFETY: This is safe during GC when all threads are stopped
                // and we have exclusive access through GC coordination
                let mutator = unsafe { &mut *arc_mutator.get() };
                let mutator_ref = unsafe { 
                    std::mem::transmute::<&mut Mutator<BlinkVM>, &'a mut Mutator<BlinkVM>>(mutator)
                };
                mutators.push(mutator_ref);
            }
        }
        
        Box::new(mutators.into_iter())
    }

    fn vm_trace_object<Q: ObjectQueue>(
        queue: &mut Q,
        object: ObjectReference,
        _worker: &mut mmtk::scheduler::GCWorker<BlinkVM>,
    ) -> ObjectReference {
        // Simple implementation - just enqueue for tracing
        queue.enqueue(object);
        object
    }
}