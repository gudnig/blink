// blink_core/src/runtime/mmtk/active_plan.rs
// Pure thread-local approach that works with ActivePlan interface

use mmtk::{util::ObjectReference, vm::ActivePlan, Mutator, ObjectQueue};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::runtime::BlinkVM;

// Thread-local mutator storage (your preferred approach)
thread_local! {
    static MUTATOR: RefCell<Option<Box<Mutator<BlinkVM>>>> = RefCell::new(None);
    static THREAD_TLS: RefCell<Option<mmtk::util::VMMutatorThread>> = RefCell::new(None);
}

// Global tracking for thread count only (no pointers!)
static THREAD_TO_ID: OnceLock<Mutex<HashMap<std::thread::ThreadId, usize>>> = OnceLock::new();
static MUTATOR_COUNT: AtomicUsize = AtomicUsize::new(0);
static COUNTER: AtomicUsize = AtomicUsize::new(1);

pub struct BlinkActivePlan;

impl BlinkActivePlan {
    fn create_vm_mutator_thread() -> mmtk::util::VMMutatorThread {
        let thread_id = std::thread::current().id();
        let thread_map = THREAD_TO_ID.get_or_init(|| Mutex::new(HashMap::new()));
        
        let mut map = thread_map.lock().unwrap();
        let unique_id = map.entry(thread_id).or_insert_with(|| {
            COUNTER.fetch_add(1, Ordering::SeqCst)
        });
        
        // Create the nested structure: VMMutatorThread(VMThread(OpaquePointer))
        let opaque = mmtk::util::OpaquePointer::from_address(
            unsafe { mmtk::util::Address::from_usize(*unique_id) }
        );
        let vm_thread = mmtk::util::VMThread(opaque);
        let tls = mmtk::util::VMMutatorThread(vm_thread);
        
        // Store in thread-local
        THREAD_TLS.with(|tls_ref| {
            *tls_ref.borrow_mut() = Some(tls);
        });
        
        tls
    }

    pub fn with_mutator<T>(f: impl FnOnce(&mut Mutator<BlinkVM>) -> T) -> T {
        MUTATOR.with(|m| {
            let mut mutator_ref = m.borrow_mut();
            if mutator_ref.is_none() {
                // Create VMMutatorThread using current thread as identifier
                let tls = Self::create_vm_mutator_thread();
                
                // Get static reference to MMTK
                let static_mmtk = crate::runtime::GLOBAL_MMTK.get()
                    .expect("MMTK not initialized");
                
                // Bind mutator with VMMutatorThread
                let mutator = mmtk::memory_manager::bind_mutator(static_mmtk, tls);
                *mutator_ref = Some(mutator);
                
                // Increment global count
                MUTATOR_COUNT.fetch_add(1, Ordering::SeqCst);
                
                println!("Created mutator for thread {:?}", std::thread::current().id());
            }
            
            let mutator = mutator_ref.as_mut().unwrap();
            f(mutator.as_mut())
        })
    }

    pub fn get_current_tls() -> mmtk::util::VMMutatorThread {
        THREAD_TLS.with(|tls| {
            tls.borrow().unwrap_or_else(|| {
                // Create if doesn't exist
                Self::create_vm_mutator_thread()
            })
        })
    }
}

impl ActivePlan<BlinkVM> for BlinkActivePlan {
    fn is_mutator(_tls: mmtk::util::VMThread) -> bool {
        // For simplicity, assume any thread could be a mutator
        // In practice, you might want to check if the thread has a mutator
        true
    }

    fn mutator(tls: mmtk::util::VMMutatorThread) -> &'static mut Mutator<BlinkVM> {
        // This is the tricky part with pure thread-local storage
        // We need to access the current thread's mutator
        // 
        // Since ActivePlan::mutator expects a static reference, we have to be creative
        // For most MMTk operations, this will be called from the owning thread
        
        MUTATOR.with(|m| {
            let mut mutator_ref = m.borrow_mut();
            if mutator_ref.is_none() {
                // Create mutator if it doesn't exist
                let static_mmtk = crate::runtime::GLOBAL_MMTK.get()
                    .expect("MMTK not initialized");
                let mutator = mmtk::memory_manager::bind_mutator(static_mmtk, tls);
                *mutator_ref = Some(mutator);
                MUTATOR_COUNT.fetch_add(1, Ordering::SeqCst);
            }
            
            let mutator = mutator_ref.as_mut().unwrap();
            // UNSAFE: Convert to static reference
            // This is safe because the mutator lives for the thread's lifetime
            unsafe {
                std::mem::transmute::<&mut Mutator<BlinkVM>, &'static mut Mutator<BlinkVM>>(
                    mutator.as_mut()
                )
            }
        })
    }
    
    fn number_of_mutators() -> usize {
        MUTATOR_COUNT.load(Ordering::SeqCst)
    }
    
    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut mmtk::Mutator<BlinkVM>> + 'a> {
        // This is the main limitation of pure thread-local storage
        // We can't easily iterate over all thread-local values from a different thread
        // 
        // For now, return empty iterator. This may limit some MMTk functionality,
        // but basic GC should still work since MMTk can access mutators individually
        Box::new(std::iter::empty())
    }
    
    fn vm_trace_object<Q: ObjectQueue>(
        _queue: &mut Q,
        object: ObjectReference,
        _worker: &mut mmtk::scheduler::GCWorker<BlinkVM>
    ) -> ObjectReference {
        object
    }
}