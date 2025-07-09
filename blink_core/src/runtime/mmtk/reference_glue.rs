use mmtk::{util::ObjectReference, vm::ReferenceGlue};

use crate::runtime::BlinkVM;


// Minimal ReferenceGlue implementation
pub struct BlinkReferenceGlue;

impl ReferenceGlue<BlinkVM> for BlinkReferenceGlue {
    type FinalizableType = ObjectReference;
    
    fn set_referent(_reff: ObjectReference, _referent: ObjectReference) {
        // Weak references not implemented yet
    }
    
    fn get_referent(_object: ObjectReference) -> Option<ObjectReference> {
        None // No weak references yet
    }
    
    fn clear_referent(_object: ObjectReference) {
        // No weak references yet
    }
    
    fn enqueue_references(_references: &[ObjectReference], _tls: mmtk::util::VMWorkerThread) {
        // No reference processing yet
    }
}