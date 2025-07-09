use mmtk::{util::ObjectReference, vm::ActivePlan, Mutator, ObjectQueue};

use crate::runtime::BlinkVM;


// Minimal ActivePlan implementation for NoGC
pub struct BlinkActivePlan;

impl ActivePlan<BlinkVM> for BlinkActivePlan {
    fn mutator(_tls: mmtk::util::VMMutatorThread) -> &'static mut Mutator<BlinkVM> {
        // For NoGC, this shouldn't be called
        panic!("ActivePlan::mutator called on NoGC plan")
    }
    
    fn number_of_mutators() -> usize {
        1 // Single-threaded for now
    }
    
    fn is_mutator(_tls: mmtk::util::VMThread) -> bool {
        true // For simplicity, assume all threads are mutators
    }
    
    fn mutators<'a>() -> Box<dyn Iterator<Item = &'a mut mmtk::Mutator<BlinkVM>> + 'a> {
        // For NoGC, return empty iterator
        Box::new(std::iter::empty())
    }
    

    
    fn vm_trace_object<Q: ObjectQueue>(
        _queue: &mut Q,
        _object: ObjectReference,
        _worker: &mut mmtk::scheduler::GCWorker<BlinkVM>
    ) -> ObjectReference {
        // For NoGC, objects don't move
        _object
    }
}