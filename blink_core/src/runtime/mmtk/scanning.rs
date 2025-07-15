use mmtk::{scheduler::ProcessEdgesWork, util::{Address, ObjectReference}, vm::{slot::MemorySlice, Scanning, VMBinding}, Mutator};

use crate::{runtime::{BlinkMemorySlice, BlinkSlot, BlinkVM}, value::ValueRef};

use mmtk::vm::slot::Slot;

// Minimal Scanning implementation for NoGC
pub struct BlinkScanning;

impl Scanning<BlinkVM> for BlinkScanning {

    
    fn scan_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        _tls: mmtk::util::VMWorkerThread,
        _object: ObjectReference,
        _slot_visitor: &mut SV,
    ) {
        // For NoGC, we don't need to scan objects
        // This would be implemented for actual GC plans
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: mmtk::util::VMWorkerThread) {
        // No-op for NoGC 
        
    }

    fn scan_roots_in_mutator_thread(
        _tls: mmtk::util::VMWorkerThread,
        _mutator: &'static mut Mutator<BlinkVM>,
        _factory: impl mmtk::vm::RootsWorkFactory<<BlinkVM as VMBinding>::VMSlot>,
    ) {
        // For NoGC, we don't scan roots
    }

    fn scan_vm_specific_roots(
        _tls: mmtk::util::VMWorkerThread,
        _factory: impl mmtk::vm::RootsWorkFactory<<BlinkVM as VMBinding>::VMSlot>
    ) {
        // For NoGC, we don't scan roots
    }

    fn supports_return_barrier() -> bool {
        false // No return barriers for NoGC
    }

    fn prepare_for_roots_re_scanning() {
        // No-op for NoGC
    }


    
}

impl BlinkScanning {

    fn scan_module_objec<T>(trace: &mut T, object: ObjectReference)
    where
        T: ProcessEdgesWork<VM = BlinkVM>, {
        // Only need to scan the first ObjectReference!
        let env_slot = BlinkSlot(object.to_raw_address()); // Points to env_ref
        if let Some(env_ref) = env_slot.load() {
            trace.trace_object(env_ref);
        }
        // Done! Everything else is non-reference data
    }

    fn scan_callable_object<T>(trace: &mut T, object: ObjectReference)
    where
        T: ProcessEdgesWork<VM = BlinkVM>,
    {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Scan env reference (ObjectReference #1)
            let env_addr = Address::from_usize(data_ptr.add(offset) as usize);
            let env_slot = BlinkSlot(env_addr);
            
            // For gencopy, we need to use both approaches:
            // 1. trace_object to mark/copy the object
            // 2. Process the slot so references can be updated
            if let Some(env_ref) = env_slot.load() {
                trace.trace_object(env_ref);
            }
            
            offset += std::mem::size_of::<ObjectReference>();
            
            // Read body count
            let body_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Scan all body expressions
            for _ in 0..body_count {
                let value_addr = Address::from_usize(data_ptr.add(offset) as usize);
                let value_slot = BlinkSlot(value_addr);
                
                if let Some(obj_ref) = value_slot.load() {
                    trace.trace_object(obj_ref);
                    // TODO: Process slot for reference updating
                }
                
                offset += std::mem::size_of::<ValueRef>();
            }
        }
    }

    fn scan_env_object<T>(trace: &mut T, object: ObjectReference)
    where
        T: ProcessEdgesWork<VM = BlinkVM>,
    {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Scan parent reference
            let parent_addr = Address::from_usize(data_ptr.add(offset) as usize);
            let parent_slot = BlinkSlot(parent_addr);
            
            if let Some(parent_ref) = parent_slot.load() {
                trace.trace_object(parent_ref);
            }
            
            offset += std::mem::size_of::<Option<ObjectReference>>();
            
            // Skip to ValueRef array
            offset += std::mem::size_of::<u32>() * 3; // Skip counts
            
            // Get vars count
            let vars_count = std::ptr::read_unaligned(
                (data_ptr.add(std::mem::size_of::<Option<ObjectReference>>()) as *const u8) as *const u32
            ) as usize;
            
            // Scan all ValueRefs
            for _ in 0..vars_count {
                let value_addr = Address::from_usize(data_ptr.add(offset) as usize);
                let value_slot = BlinkSlot(value_addr);
                
                if let Some(obj_ref) = value_slot.load() {
                    trace.trace_object(obj_ref);
                }
                
                offset += std::mem::size_of::<ValueRef>();
            }
        }
    }
}

