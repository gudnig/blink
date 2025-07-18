// blink_core/src/runtime/mmtk/collection.rs
// Collection implementation that works with thread-local mutators

use mmtk::{vm::{ActivePlan, Collection}, Mutator};
use crate::runtime::{BlinkVM, BlinkActivePlan};

pub struct BlinkCollection;

impl Collection<BlinkVM> for BlinkCollection {
    fn stop_all_mutators<F>(_tls: mmtk::util::VMWorkerThread, mut mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        println!("Stopping all mutators for GC");
        
        // With pure thread-local storage, we can only access the current thread's mutator
        // In a real multi-threaded implementation, you'd need a different approach
        // For single-threaded or simple cases, this should work
        
        // Access the current thread's mutator if it exists
        let has_mutator = BlinkActivePlan::number_of_mutators() > 0;
        
        if has_mutator {
            // Get the current thread's mutator and visit it
            let tls = BlinkActivePlan::get_current_tls();
            let mutator = BlinkActivePlan::mutator(tls);
            mutator_visitor(mutator);
            println!("Visited 1 mutator");
        } else {
            println!("No mutators to stop");
        }
    }
    
    fn resume_mutators(_tls: mmtk::util::VMWorkerThread) {
        println!("Resuming mutators after GC");
        // In a thread-local model, mutators resume automatically when threads continue
    }
    
    fn block_for_gc(_tls: mmtk::util::VMMutatorThread) {
        println!("Blocking mutator for GC");
        // Block the current thread until GC completes
        // In a simple single-threaded implementation, this might be a no-op
    }
    
    fn spawn_gc_thread(tls: mmtk::util::VMThread, ctx: mmtk::vm::GCThreadContext<BlinkVM>) {
        println!("Spawning GC thread");

        let tls = mmtk::util::VMWorkerThread(tls);
        let mmtk = crate::runtime::GLOBAL_MMTK.get().unwrap();
        match ctx {
            mmtk::vm::GCThreadContext::Worker(worker) => {
                std::thread::spawn(move || {
                    println!("GC worker thread started");
                    worker.run(tls, mmtk);
                    println!("GC worker thread finished");
                });
            }
        }
    }
}