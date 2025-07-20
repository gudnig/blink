// Fixed VMActivePlan implementation - Thread-safe approach
// Instead of storing pointers, we'll use a different strategy

use mmtk::vm::{ObjectModel, Scanning, SlotVisitor};
use mmtk::{util::ObjectReference, vm::ActivePlan, Mutator, ObjectQueue};
use parking_lot::MutexGuard;
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, Arc};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;

use crate::runtime::{init_gc_park, BlinkVM};

// Thread-local mutator storage
thread_local! {
    static MUTATOR: RefCell<Option<Arc<Mutex<Box<Mutator<BlinkVM>>>>>> = RefCell::new(None);
    pub static THREAD_TLS: OnceCell<mmtk::util::VMMutatorThread> = OnceCell::new();
}

// Thread-safe tracking using thread IDs instead of pointers
static ACTIVE_THREADS: OnceLock<Mutex<Vec<std::thread::ThreadId>>> = OnceLock::new();
pub static MUTATORS: OnceLock<Mutex<HashMap<ThreadId, Arc<Mutex<Box<Mutator<BlinkVM>>>>>>> = OnceLock::new();
static THREAD_TO_ID: OnceLock<Mutex<HashMap<std::thread::ThreadId, usize>>> = OnceLock::new();
static MUTATOR_COUNT: AtomicUsize = AtomicUsize::new(0);
static COUNTER: AtomicUsize = AtomicUsize::new(1);

pub struct BlinkMutatorIterator<'a> {
    _guard: MutexGuard<'a, ()>,
    counter: usize,
}

impl<'a> BlinkMutatorIterator<'a> {
    fn new(guard: MutexGuard<'a, ()>) -> Self {
        Self {
            _guard: guard,
            counter: 0,
        }
    }
}

impl<'a> Iterator for BlinkMutatorIterator<'a> {
    type Item = &'a mut Mutator<BlinkVM>;
    
    fn next(&mut self) -> Option<Self::Item> {
        None
    }
}

pub fn gc_poll() {
    let (lock, cvar) = &*init_gc_park();
    let mut is_gc = lock.lock();
    while *is_gc {
        println!("Mutator parking for GC...");
        cvar.wait(&mut is_gc);
        println!("Mutator resumed after GC");
    }
}


pub struct BlinkActivePlan;

impl BlinkActivePlan {
    pub fn create_vm_mutator_thread_pre() -> mmtk::util::VMMutatorThread {
        println!("Creating VM mutator thread (pre)");
    
        let thread_id = std::thread::current().id();
        let thread_map = THREAD_TO_ID.get_or_init(|| Mutex::new(HashMap::new()));
    
        let mut map = thread_map.lock().unwrap();
        let unique_id = map.entry(thread_id).or_insert_with(|| COUNTER.fetch_add(1, Ordering::SeqCst));
    
        let opaque = mmtk::util::OpaquePointer::from_address(
            unsafe { mmtk::util::Address::from_usize(*unique_id) }
        );
        let vm_thread = mmtk::util::VMThread(opaque);
        let tls = mmtk::util::VMMutatorThread(vm_thread);
    
        THREAD_TLS.with(|cell| {
            cell.get_or_init(|| tls);
        });
    
        tls
    }
    
    

    

    pub fn with_mutator<T>(f: impl FnOnce(&mut Mutator<BlinkVM>) -> T) -> T {
        gc_poll();  // ensure cooperation with GC
    
        MUTATOR.with(|mutator_cell| {
            if mutator_cell.borrow().is_none() {
                let tls = Self::create_vm_mutator_thread_pre();
    
                let static_mmtk = crate::runtime::GLOBAL_MMTK.get()
                    .expect("MMTK not initialized");
    
                let boxed_mutator = mmtk::memory_manager::bind_mutator(static_mmtk, tls);
                let arc_mutator = Arc::new(Mutex::new(boxed_mutator));
    
                // Register globally
                let mutators = MUTATORS.get_or_init(|| Mutex::new(HashMap::new()));
                mutators.lock().unwrap().insert(std::thread::current().id(), arc_mutator.clone());
    
                // Register thread id
                let active_threads = ACTIVE_THREADS.get_or_init(|| Mutex::new(Vec::new()));
                active_threads.lock().unwrap().push(std::thread::current().id());
    
                *mutator_cell.borrow_mut() = Some(arc_mutator);
    
                MUTATOR_COUNT.fetch_add(1, Ordering::SeqCst);
                println!("Created mutator for thread {:?}", std::thread::current().id());
            }
    
            let arc_mutator = mutator_cell.borrow().as_ref().unwrap().clone();
            let mut guard = arc_mutator.lock().unwrap();
            f(guard.as_mut())
        })
    }
    

 
}

impl ActivePlan<BlinkVM> for BlinkActivePlan {
    fn is_mutator(tls: mmtk::util::VMThread) -> bool {
        THREAD_TLS.with(|current_tls| {
            current_tls.get().map_or(false, |stored_tls| stored_tls.0 == tls)
        })
    }

    fn mutator(_tls: mmtk::util::VMMutatorThread) -> &'static mut Mutator<BlinkVM> {
        let thread_id = std::thread::current().id();

        let mutators = MUTATORS.get().expect("MUTATORS not initialized");
        let map = mutators.lock().unwrap();

        let arc_mutex = map.get(&thread_id).expect("No mutator for current thread");

        // Lock and get the Box<Mutator>
        let mut guard = arc_mutex.lock().unwrap();
        let raw_mutator: *mut Mutator<BlinkVM> = guard.as_mut();

        // SAFETY: We promote to 'static because Blink runs threads till program end
        unsafe { &mut *raw_mutator }
    }

    fn number_of_mutators() -> usize {
        MUTATOR_COUNT.load(Ordering::SeqCst)
    }

    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut Mutator<BlinkVM>> + 'a> {
        if let Some(mutators) = MUTATORS.get() {
            let map = mutators.lock().unwrap();

            let iter = map.values()
                .filter_map(|arc_mutex| {
                    let mut guard = arc_mutex.lock().ok()?;
                    let raw_mutator: *mut Mutator<BlinkVM> = guard.as_mut();
                    Some(unsafe { &mut *raw_mutator as &'a mut Mutator<BlinkVM> })
                })
                .collect::<Vec<_>>()  // materialize into a vec to satisfy 'static lifetime
                .into_iter();

            Box::new(iter)
        } else {
            Box::new(std::iter::empty())
        }
    }

    fn vm_trace_object<Q: ObjectQueue>(
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut mmtk::scheduler::GCWorker<BlinkVM>,
    ) -> ObjectReference {


        
        println!("vm_trace_object called for {:?}", object);
        queue.enqueue(object);
        object
    }
}
