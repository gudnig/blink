// blink_core/src/runtime/mmtk/collection.rs
// Collection implementation that works with thread-local mutators

use std::hash::{DefaultHasher, Hash, Hasher};

use mmtk::{memory_manager, util::{Address, OpaquePointer, VMThread, VMWorkerThread}, vm::{ActivePlan, Collection, GCThreadContext}, Mutator};
use crate::runtime::{gc_poll, init_gc_park, BlinkActivePlan, BlinkVM, MUTATORS};

pub struct BlinkCollection;

impl Collection<BlinkVM> for BlinkCollection {
    fn stop_all_mutators<F>(_tls: VMWorkerThread, mut mutator_visitor: F)
where
    F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        println!("Stopping all mutators for GC");

        // First, signal all mutators to park
        let (lock, _) = &*init_gc_park();
        {
            let mut is_gc = lock.lock();
            *is_gc = true;
        }

        // Give mutators a chance to see the signal and park themselves
        std::thread::yield_now();
        
        if let Some(mutators) = MUTATORS.get() {
            let map = mutators.lock().unwrap();
            
            // Process each mutator
            for (thread_id, arc_mutex_mutator) in map.iter() {
                // Try to lock with a timeout to detect deadlocks
                match arc_mutex_mutator.try_lock() {
                    Ok(mut guard) => {
                        let static_mutator: &'static mut Mutator<BlinkVM> = unsafe {
                            &mut *(guard.as_mut() as *mut _)
                        };
                        mutator_visitor(static_mutator);
                    }
                    Err(_) => {
                        println!("Warning: Could not lock mutator for thread {:?} - it may be the requesting thread", thread_id);
                        // Skip this mutator - it's likely the one that requested GC
                    }
                }
            }
        }
    }

    fn resume_mutators(_tls: VMWorkerThread) {
        println!("Resuming mutators after GC");

        let (lock, cvar) = &*init_gc_park();
        {
            let mut is_gc = lock.lock();
            *is_gc = false;
        }
        cvar.notify_all();
    }

    fn block_for_gc(_tls: mmtk::util::VMMutatorThread) {
        println!("Blocking mutator for GC");
        gc_poll();
    }

    fn spawn_gc_thread(_tls: VMThread, ctx: GCThreadContext<BlinkVM>) {
        println!("Spawning GC thread with context: {:?}", std::any::type_name::<GCThreadContext<BlinkVM>>());
    
        let mmtk = crate::runtime::GLOBAL_MMTK.get().unwrap();
        match ctx {
            mmtk::vm::GCThreadContext::Worker(worker) => {
                println!("GC worker context received: spawning...");
                std::thread::spawn(move || {
                    println!("GC worker thread started");
    
                    let tls = VMWorkerThread(VMThread(OpaquePointer::from_address(
                        unsafe { Address::from_usize(thread_id_as_usize()) },
                    )));
    
                    worker.run(tls, mmtk);
    
                    println!("GC worker thread finished");
                });
            }
            other => {
                println!("Received unhandled GCThreadContext variant: {:?}", std::any::type_name::<GCThreadContext<BlinkVM>>());
            }
        }
    }
    
}

fn thread_id_as_usize() -> usize {
    let thread_id = std::thread::current().id();
    let mut hasher = DefaultHasher::new();
    thread_id.hash(&mut hasher);
    hasher.finish() as usize
}

