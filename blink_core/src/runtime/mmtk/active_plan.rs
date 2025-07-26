// blink_core/src/runtime/mmtk/active_plan.rs
// SIMPLIFIED: Remove all custom coordination, let MMTk handle it

use mmtk::util::Address;
use mmtk::{util::ObjectReference, vm::ActivePlan, Mutator, ObjectQueue};
use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use parking_lot::Mutex;
use std::sync::{OnceLock, Arc};
use std::sync::atomic::{AtomicUsize, Ordering};
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

unsafe impl Send for ThreadSafeMutator {}
unsafe impl Sync for ThreadSafeMutator {}

thread_local! {
    static MUTATOR: RefCell<Option<Arc<ThreadSafeMutator>>> = RefCell::new(None);
    static THREAD_TLS: RefCell<Option<mmtk::util::VMMutatorThread>> = RefCell::new(None);
}

// Global mutator tracking - only for MMTk iteration, not coordination
pub static GLOBAL_MUTATORS: OnceLock<Mutex<HashMap<ThreadId, Arc<ThreadSafeMutator>>>> = OnceLock::new();
static ACTIVE_MUTATOR_THREADS: OnceLock<Mutex<Vec<ThreadId>>> = OnceLock::new();
static THREAD_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub struct BlinkActivePlan;

impl BlinkActivePlan {
    fn create_vm_mutator_thread() -> mmtk::util::VMMutatorThread {
        let unique_id = THREAD_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        
        let opaque = mmtk::util::OpaquePointer::from_address(
            unsafe { mmtk::util::Address::from_usize(unique_id) }
        );
        let vm_thread = mmtk::util::VMThread(opaque);
        let tls = mmtk::util::VMMutatorThread(vm_thread);
        
        THREAD_TLS.with(|cell| {
            *cell.borrow_mut() = Some(tls);
        });
        
        tls
    }

    pub fn with_mutator<T>(f: impl FnOnce(&mut Mutator<BlinkVM>) -> T) -> T {
        MUTATOR.with(|mutator_cell| {
            let mut mutator_ref = mutator_cell.borrow_mut();
            
            if mutator_ref.is_none() {
                let tls = Self::create_vm_mutator_thread();
                let mmtk = crate::runtime::GLOBAL_MMTK.get()
                    .expect("MMTK not initialized");
                
                let boxed_mutator = mmtk::memory_manager::bind_mutator(mmtk, tls);
                let thread_safe_mutator = Arc::new(ThreadSafeMutator::new(boxed_mutator));
                
                *mutator_ref = Some(thread_safe_mutator.clone());
                Self::register_mutator_thread(std::thread::current().id(), thread_safe_mutator);
            }
            
            let arc_mutator = mutator_ref.as_ref().unwrap();
            let mutator = unsafe { &mut *arc_mutator.get() };
            
            // SIMPLIFIED: Just call the function, let MMTk handle coordination
            f(mutator)
        })
    }

    pub fn alloc(mutator: &mut Mutator<BlinkVM>, type_tag: &TypeTag, data_size: &usize) -> ObjectReference {
        let total_size = (ObjectHeader::SIZE + data_size + 7) & !7;
        
        // Try fast path
        let mut start = mmtk::memory_manager::alloc(
            mutator,
            total_size,
            8,
            0,
            mmtk::AllocationSemantics::Default,
        );

        // If it fails, fall back to slow path
        if start.is_zero() {
            start = mmtk::memory_manager::alloc_slow(
                mutator,
                total_size,
                8,
                0,
                mmtk::AllocationSemantics::Default,
            );
        }

        // Now continue as before
        if start.is_zero() {
            panic!("Failed to allocate object of size {}", total_size);
        }

        if total_size <= 0 {
            println!("🔧 ALLOC: Failed to allocate object");
            println!("🔧 ALLOC: total_size: {}", total_size);
            println!("🔧 ALLOC: data_size: {}", data_size);
            println!("🔧 ALLOC: type_tag: {:?}", type_tag);
            println!("🔧 ALLOC: start: {:?}", start);
            panic!("Failed to allocate object");
        }
    
        let header = ObjectHeader::new(*type_tag, *data_size);
        
        
        let header_ptr = start.to_mut_ptr::<ObjectHeader>();
        unsafe { std::ptr::write(header_ptr, header) };

        
        // VERIFY: Read back the header to make sure it was written correctly
        let verified_header = unsafe { std::ptr::read(header_ptr) };
        
        
        let obj_ref = ObjectReference::from_raw_address(start + ObjectHeader::SIZE).unwrap();
        
        mmtk::memory_manager::post_alloc(mutator, obj_ref, total_size, mmtk::AllocationSemantics::Default);
        
        
        obj_ref
    }

    fn register_mutator_thread(thread_id: ThreadId, mutator: Arc<ThreadSafeMutator>) {
        let active_threads = ACTIVE_MUTATOR_THREADS.get_or_init(|| Mutex::new(Vec::new()));
        let mut threads = active_threads.lock();
        if !threads.contains(&thread_id) {
            threads.push(thread_id);
        }
        
        let global_mutators = GLOBAL_MUTATORS.get_or_init(|| Mutex::new(HashMap::new()));
        global_mutators.lock().insert(thread_id, mutator);
    }

    /// For Collection trait - visit all mutators
    pub fn visit_all_mutators<F>(mut mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        if let Some(global_mutators) = GLOBAL_MUTATORS.get() {
            let mutator_map = global_mutators.lock();
            
            for (_thread_id, arc_mutator) in mutator_map.iter() {
                let mutator = unsafe { &mut *arc_mutator.get() };
                let static_mutator: &'static mut Mutator<BlinkVM> = unsafe {
                    std::mem::transmute::<&mut Mutator<BlinkVM>, &'static mut Mutator<BlinkVM>>(mutator)
                };
                mutator_visitor(static_mutator);
            }
        }
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
        MUTATOR.with(|mutator_cell| {
            let mutator_ref = mutator_cell.borrow();
            let arc_mutator = mutator_ref.as_ref().expect("No mutator for current thread");
            let mutator = unsafe { &mut *arc_mutator.get() };
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
        let mut mutators = Vec::new();
        
        if let Some(global_mutators) = GLOBAL_MUTATORS.get() {
            let mutator_map = global_mutators.lock();
            
            for (_thread_id, arc_mutator) in mutator_map.iter() {
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
        queue.enqueue(object);
        object
    }
}