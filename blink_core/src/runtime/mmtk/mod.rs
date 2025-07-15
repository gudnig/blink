mod object_model;
mod scanning;
mod collection;
mod active_plan;
mod reference_glue;
mod slot;
mod gc_work;

use mmtk::vm::VMBinding;
pub use object_model::*;
pub use scanning::*;
pub use collection::*;
pub use active_plan::*;
pub use reference_glue::*;
pub use slot::*;
pub use gc_work::*;

use crate::runtime::BlinkVM;


impl VMBinding for BlinkVM {
    type VMObjectModel = BlinkObjectModel;
    type VMScanning = BlinkScanning;
    type VMCollection = BlinkCollection;
    type VMActivePlan = BlinkActivePlan;
    type VMReferenceGlue = BlinkReferenceGlue;
    type VMSlot = BlinkSlot;
    type VMMemorySlice = BlinkMemorySlice;
    
    const ALIGNMENT_VALUE: usize = 0xdead_beef;
    const MIN_ALIGNMENT: usize = 8; // 8-byte alignment, typical for 64-bit systems
    const MAX_ALIGNMENT: usize = 64; // Maximum alignment, adjust as needed
    const USE_ALLOCATION_OFFSET: bool = true;
    const ALLOC_END_ALIGNMENT: usize = 1;
}