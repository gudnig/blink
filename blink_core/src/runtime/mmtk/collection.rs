use mmtk::{vm::Collection, Mutator};

use crate::runtime::BlinkVM;


// Minimal Collection implementation for NoGC
pub struct BlinkCollection;

impl Collection<BlinkVM> for BlinkCollection {
    fn stop_all_mutators<F>(_tls: mmtk::util::VMWorkerThread, _mutator_visitor: F)
    where
        F: FnMut(&'static mut Mutator<BlinkVM>),
    {
        // For NoGC, we don't stop mutators
    }
    
    fn resume_mutators(_tls: mmtk::util::VMWorkerThread) {
        // For NoGC, we don't resume mutators
    }
    
    fn block_for_gc(_tls: mmtk::util::VMMutatorThread) {
        // For NoGC, we don't block for GC
    }
    
    fn spawn_gc_thread(_tls: mmtk::util::VMThread, _ctx: mmtk::vm::GCThreadContext<BlinkVM>) {
        // For NoGC, we don't spawn GC threads
    }
}