use mmtk::{scheduler::{GCWork, ProcessEdgesWork, WorkBucketStage}, util::{Address, ObjectReference}, vm::{slot::{self, MemorySlice}, Scanning, VMBinding}, Mutator};

use crate::{runtime::{BlinkMemorySlice, BlinkObjectModel, BlinkSlot, BlinkVM, ObjectHeader, ScanBlinkVMRoots, TypeTag, GLOBAL_VM}, value::{SourceRange, ValueRef}};

use mmtk::vm::slot::Slot;

// Minimal Scanning implementation for NoGC
pub struct BlinkScanning;

impl Scanning<BlinkVM> for BlinkScanning {

    
    fn scan_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        _tls: mmtk::util::VMWorkerThread,
        object: ObjectReference,
        slot_visitor: &mut SV,
    ) {
        println!("Scanning object: {:?}", object);
        let type_tag = BlinkObjectModel::get_type_tag(object);
        println!("Type tag: {:?}", type_tag);
        
        
        match type_tag {
            TypeTag::Module => Self::scan_module_object(slot_visitor, object),
            TypeTag::Macro | TypeTag::UserDefinedFunction => Self::scan_callable_object(slot_visitor, object),
            TypeTag::Env => Self::scan_env_object(slot_visitor, object),
            TypeTag::List => Self::scan_vec_or_list_object(slot_visitor, object),
            TypeTag::Vector => Self::scan_vec_or_list_object(slot_visitor, object),
            TypeTag::Map => Self::scan_map_object(slot_visitor, object),
            TypeTag::Str => {
                // No object references to scan - just raw string data
            },
            TypeTag::Set => Self::scan_set_object(slot_visitor, object),
            TypeTag::Error => Self::scan_error_object(slot_visitor, object),
            TypeTag::Future => todo!(),
        }
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
        factory: impl mmtk::vm::RootsWorkFactory<<BlinkVM as VMBinding>::VMSlot>
    ) {
        println!("Scanning VM specific roots");

        
        
        let static_mmtk = crate::runtime::GLOBAL_MMTK.get().expect("MMTK not initialized");
        let free_bytes = mmtk::memory_manager::free_bytes(static_mmtk);
        println!("Free bytes: {:?}", free_bytes);
        println!("Total bytes: {:?}", mmtk::memory_manager::total_bytes(static_mmtk));
        let scan_work = ScanBlinkVMRoots::new(factory);
        let packets: Vec<Box<dyn GCWork<BlinkVM>>> = vec![Box::new(scan_work)];
        
        mmtk::memory_manager::add_work_packets(static_mmtk, WorkBucketStage::Prepare, packets);
        
    }
    fn supports_return_barrier() -> bool {
        false // No return barriers for NoGC
    }

    fn prepare_for_roots_re_scanning() {
        // No-op for NoGC
    }


    
}

impl BlinkScanning {

    fn scan_module_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference) {
        // Only need to scan the first ObjectReference!
        let env_slot = BlinkSlot::ObjectRef(object.to_raw_address()); // Points to env_ref
        slot_visitor.visit_slot(env_slot);
    }

    fn scan_callable_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Scan env reference (raw ObjectReference)
            let env_addr = Address::from_usize(data_ptr.add(offset) as usize);
            let env_slot = BlinkSlot::ObjectRef(env_addr);
            slot_visitor.visit_slot(env_slot);
            offset += std::mem::size_of::<ObjectReference>();
            
            // Read body count
            let body_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Scan body expressions using helper
            Self::scan_value_ref_seq(slot_visitor, data_ptr, body_count, offset);
        }
    }

    fn scan_vec_or_list_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        let data_ptr = object.to_raw_address().as_usize() as *const u8;
            
        // Get the data size from the object header
        let header = BlinkObjectModel::get_header(object).0;
        let data_size = header.total_size as usize - ObjectHeader::SIZE;
        let item_count = data_size / std::mem::size_of::<ValueRef>();
        
        // Scan all ValueRef items starting from offset 0
        Self::scan_value_ref_seq(slot_visitor, data_ptr, item_count, 0);
    }

    fn scan_map_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Read bucket_count
            let bucket_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const usize);
            offset += std::mem::size_of::<usize>();
            
            // Read item_count
            let item_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const usize);
            offset += std::mem::size_of::<usize>();
            
            // Skip bucket offsets (non-reference data)
            offset += bucket_count * std::mem::size_of::<u32>();
            
            // Scan key-value pairs (each pair is 2 ValueRefs)
            Self::scan_value_ref_seq(slot_visitor, data_ptr, item_count * 2, offset);
        }
    }


    // Helper function to scan a sequence of ValueRefs
    fn scan_value_ref_seq<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        start_ptr: *const u8,
        size: usize,
        mut offset: usize) {
        unsafe {
            for i in 0..size {
                let value_ref_ptr = start_ptr.add(offset) as *const ValueRef;
                let value_ref = std::ptr::read_unaligned(value_ref_ptr);
                
                match value_ref {
                    ValueRef::Heap(_) => {
                        let slot = BlinkSlot::ValueRef(Address::from_ptr(value_ref_ptr));
                        slot_visitor.visit_slot(slot);
                    }
                    _ => {} // Skip immediate/native values
                }
                
                offset  += std::mem::size_of::<ValueRef>();
            }
        }
    }

    fn scan_set_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Read bucket_count
            let bucket_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const usize);
            offset += std::mem::size_of::<usize>();
            
            // Read item_count
            let item_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const usize);
            offset += std::mem::size_of::<usize>();
            
            // Skip bucket offsets (non-reference data)
            offset += bucket_count * std::mem::size_of::<u32>();
            
            // Scan items (each item is 1 ValueRef, unlike map's 2 per pair)
            Self::scan_value_ref_seq(slot_visitor, data_ptr, item_count, offset);
        }
    }

    fn scan_env_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Scan parent reference (Option<ObjectReference>)
            let parent_addr = Address::from_usize(data_ptr.add(offset) as usize);
            let parent_slot = BlinkSlot::OptionObjectRef(parent_addr);  // ← Fixed!
            slot_visitor.visit_slot(parent_slot);
            offset += std::mem::size_of::<Option<ObjectReference>>();
            
            // Read vars count
            let vars_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Skip other counts
            offset += std::mem::size_of::<u32>() * 2; // symbol_aliases_count + module_aliases_count
            
            // Scan ValueRef array (the actual variable values)
            Self::scan_value_ref_seq(slot_visitor, data_ptr, vars_count, offset);
        }
    }

    fn scan_error_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Skip message (non-reference data)
            let message_len = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>() + message_len;
            
            // Skip position (non-reference data)
            offset += std::mem::size_of::<Option<SourceRange>>();
            
            // Read error type discriminant
            let discriminant = std::ptr::read_unaligned(data_ptr.add(offset) as *const u8);
            offset += std::mem::size_of::<u8>();
            
            // Only UserDefined errors (discriminant 6) can contain references
            if discriminant == 6 { // UserDefined variant
                // Read the Option<ObjectReference> discriminant
                let has_data = std::ptr::read_unaligned(data_ptr.add(offset) as *const u8);
                offset += std::mem::size_of::<u8>();
                
                if has_data == 1 { // Some(ValueRef)
                    let value_ref_addr = Address::from_usize(data_ptr.add(offset) as usize);
                    let value_ref_slot = BlinkSlot::ValueRef(value_ref_addr);
                    slot_visitor.visit_slot(value_ref_slot);
                }
                // If has_data == 0 (None), no reference to scan
            }
            // All other error types contain only non-reference data
        }
    }
}

