// blink_core/src/runtime/mmtk/collection.rs
// PROPER MMTk Collection implementation - let MMTk handle coordination

use std::sync::atomic::{AtomicBool, Ordering};
use mmtk::{
    util::{VMThread, VMWorkerThread, VMMutatorThread}, 
    vm::Collection, 
    Mutator
};

use crate::runtime::BlinkVM;

pub struct BlinkCollection;

// Simple global flag - MMTk will coordinate the timing
static GC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

impl Collection<BlinkVM> for BlinkCollection {
    fn stop_all_mutators<F>(_tls: VMWorkerThread, mut mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        println!("🔒 MMTk: Stopping all mutators");
        let vm = BlinkVM::get_instance();
        vm.clear_reachable_handles();
        // Set the flag - MMTk controls when this happens
        
        
        // Visit all mutators for root scanning
        // MMTk will call this method when it's time to stop mutators
        crate::runtime::BlinkActivePlan::visit_all_mutators(mutator_visitor);
        
        println!("🔒 MMTk: All mutators stopped");
    }

    fn resume_mutators(_tls: VMWorkerThread) {
        println!("🔓 MMTk: Resuming all mutators");

        let vm = BlinkVM::get_instance();
        let reachable_handles = vm.get_reachable_handles();
        vm.handle_registry.write().gc_sweep_unreachable_futures(&reachable_handles);
        // Clear the flag - MMTk controls when this happens
        GC_IN_PROGRESS.store(false, Ordering::SeqCst);
        
        println!("🔓 MMTk: All mutators resumed");
    }

    fn block_for_gc(_tls: VMMutatorThread) {
        // THIS is where you implement VM-specific blocking
        // MMTk calls this automatically when allocation needs to block
        
        println!("🚦 MMTk: Mutator {:?} blocking for GC", std::thread::current().id());
        
        // Set flag when GC blocking starts
        GC_IN_PROGRESS.store(true, Ordering::SeqCst);
        
        // Block until MMTk says we can continue
        while GC_IN_PROGRESS.load(Ordering::SeqCst) {
            std::thread::park_timeout(std::time::Duration::from_millis(1));
        }
        
        println!("🚦 MMTk: Mutator {:?} resumed after GC", std::thread::current().id());
    }

    fn spawn_gc_thread(_tls: VMThread, ctx: mmtk::vm::GCThreadContext<BlinkVM>) {
        let mmtk = crate::runtime::GLOBAL_MMTK.get()
            .expect("MMTK not initialized");
            
        match ctx {
            mmtk::vm::GCThreadContext::Worker(worker) => {
                println!("🔧 MMTk: Spawning GC worker thread");
                
                std::thread::spawn(move || {
                    // Create TLS for this GC worker thread
                    let tls = VMWorkerThread(VMThread(mmtk::util::OpaquePointer::from_address(
                        unsafe { mmtk::util::Address::from_usize(thread_id_as_usize()) },
                    )));
                    
                    // Run the GC worker - MMTk handles all coordination
                    worker.run(tls, mmtk);
                });
            }
        }
    }
}

// Helper function
fn thread_id_as_usize() -> usize {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let thread_id = std::thread::current().id();
    let mut hasher = DefaultHasher::new();
    thread_id.hash(&mut hasher);
    hasher.finish() as usize
}
