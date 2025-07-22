// blink_core/src/runtime/mmtk/collection.rs
// Collection implementation that properly supports mutator_visitor

use std::{hash::{DefaultHasher, Hash, Hasher}, sync::{atomic::{AtomicBool, Ordering}, Arc}};
use mmtk::{
    util::{Address, OpaquePointer, VMThread, VMWorkerThread}, 
    vm::{Collection, GCThreadContext}, 
    Mutator
};
use parking_lot::{Condvar, Mutex};



use crate::runtime::{BlinkActivePlan, BlinkVM, GC_COORDINATOR};

pub struct BlinkCollection;

impl Collection<BlinkVM> for BlinkCollection {
    fn stop_all_mutators<F>(_tls: VMWorkerThread, mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        // This is called by MMTk GC worker thread
        // Pass the mutator_visitor to visit each mutator for stack scanning
        
        
        println!("Stopping all mutators GC requested");
        BlinkActivePlan::stop_all_mutators_impl(mutator_visitor);

    }

    fn resume_mutators(_tls: VMWorkerThread) {
        // This is called by MMTk GC worker thread  
        
        BlinkActivePlan::resume_all_mutators_impl();
        
    }

    fn block_for_gc(_tls: mmtk::util::VMMutatorThread) {
        // This is called on the specific mutator thread that triggered GC
        // The thread should block itself and wait for GC to complete
        
        println!("Mutator {:?} blocking for GC", std::thread::current().id());

        
        // Block until GC is complete
        BlinkActivePlan::gc_poll();
        
        println!("Mutator {:?} unblocked after GC", std::thread::current().id());
    }

    fn spawn_gc_thread(_tls: VMThread, ctx: GCThreadContext<BlinkVM>) {
        let mmtk = crate::runtime::GLOBAL_MMTK.get()
            .expect("MMTK not initialized");
            
        match ctx {
            GCThreadContext::Worker(worker) => {
                println!("Spawning GC worker thread");
                
                std::thread::spawn(move || {
                    println!("GC worker thread started");
                    
                    // Create TLS for this GC worker thread
                    let tls = VMWorkerThread(VMThread(OpaquePointer::from_address(
                        unsafe { Address::from_usize(thread_id_as_usize()) },
                    )));
                    
                    // Run the GC worker
                    worker.run(tls, mmtk);
                    
                    println!("GC worker thread finished");
                });
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