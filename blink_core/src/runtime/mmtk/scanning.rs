use mmtk::{scheduler::{GCWork, ProcessEdgesWork, WorkBucketStage}, util::{Address, ObjectReference}, vm::{slot::{self, MemorySlice}, RootsWorkFactory, Scanning, VMBinding}, Mutator};

use crate::{runtime::{ BlinkObjectModel, BlinkSlot, BlinkVM, FunctionRef, TypeTag, GLOBAL_RUNTIME}, value::ValueRef};


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
            TypeTag::UserDefinedFunction => Self::scan_callable(slot_visitor, object),
            TypeTag::Env => Self::scan_env_object(slot_visitor, object),
            TypeTag::List => Self::scan_list_header(slot_visitor, object),
            TypeTag::ListNode => Self::scan_list_node(slot_visitor, object),
            TypeTag::Vector => Self::scan_vector_object(slot_visitor, object),
            TypeTag::Map => Self::scan_map_object(slot_visitor, object),
            TypeTag::Str => {
                // No object references to scan - just raw string data
            },
            TypeTag::Set => Self::scan_set_object(slot_visitor, object),
            TypeTag::Error => Self::scan_error_object(slot_visitor, object),
            TypeTag::Future => todo!(),
            TypeTag::Closure => todo!(),
            TypeTag::Macro => Self::scan_callable(slot_visitor, object),
        }
    }

    fn notify_initial_thread_scan_complete(_partial_scan: bool, _tls: mmtk::util::VMWorkerThread) {
        // No-op for NoGC 
        println!("notify_initial_thread_scan_complete called");
        
    }

    

    fn scan_roots_in_mutator_thread(
        _tls: mmtk::util::VMWorkerThread,
        _mutator: &'static mut Mutator<BlinkVM>,
        _factory: impl mmtk::vm::RootsWorkFactory<<BlinkVM as VMBinding>::VMSlot>,
    ) {
        // For NoGC, we don't scan roots
        println!("scan_roots_in_mutator_thread called");
    }
    
    fn scan_vm_specific_roots(
        _tls: mmtk::util::VMWorkerThread,
        mut factory: impl RootsWorkFactory<BlinkSlot>,
    ) {
        const CHUNK: usize = 1024;
        let mut batch: Vec<BlinkSlot> = Vec::with_capacity(CHUNK);
    
        let runtime = GLOBAL_RUNTIME.get().expect("BlinkRuntime not initialized");
        let exec = &runtime.execution_context;
    
        // 1) Call stack: take the *address of the inner ObjectReference* when Some(_)
        for frame in exec.call_stack.iter() {
            match &frame.func {
                FunctionRef::Closure(_, obj_ref_opt)
                | FunctionRef::CompiledFunction(_, obj_ref_opt) => {
                    if let Some(obj_ref) = obj_ref_opt {
                        // Address of the *cell* that stores the ObjectReference inside the Option
                        let cell_addr = Address::from_ptr(obj_ref as *const ObjectReference);
                        batch.push(BlinkSlot::ObjectRef(cell_addr));
                        if batch.len() == CHUNK {
                            factory.create_process_roots_work(std::mem::take(&mut batch));
                        }
                    }
                }
                FunctionRef::Native(_) => {}
            }
        }
    
        // 2) Register stack: ValueRef cells (only push Heap variants)
        for reg_cell in exec.register_stack.iter() {
            if let ValueRef::Heap(_) = reg_cell {
                let cell_addr = Address::from_ptr(reg_cell as *const ValueRef);
                batch.push(BlinkSlot::ValueRef(cell_addr));
                if batch.len() == CHUNK {
                    factory.create_process_roots_work(std::mem::take(&mut batch));
                }
            }
        }
    
        // 3) Any other VM roots stored out-of-heap (example: your module exports)
        {
            let modules = runtime.vm.module_registry.read();
            for module in modules.modules.values() {
                for value in module.exports.values() {
                    if let ValueRef::Heap(_) = value {
                        let cell_addr = Address::from_ptr(value as *const ValueRef);
                        batch.push(BlinkSlot::ValueRef(cell_addr));
                        if batch.len() == CHUNK {
                            factory.create_process_roots_work(std::mem::take(&mut batch));
                        }
                    }
                }
            }
        }
    
        if !batch.is_empty() {
            factory.create_process_roots_work(batch);
        }
    }
    
    
    
    fn supports_return_barrier() -> bool {
        false // No return barriers for NoGC
    }

    fn prepare_for_roots_re_scanning() {
        // No-op for NoGC
    }


    
}

impl BlinkScanning {
    
    /// Scan a list header object: [length: usize][flags: usize][head: ObjectReference][tail: ObjectReference]
    fn scan_list_header<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let header_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Skip length (usize) - no references to scan
            offset += std::mem::size_of::<usize>();
            
            // Check flags (bit 0 = has_head, bit 1 = has_tail)
            let flags = std::ptr::read_unaligned(header_ptr.add(offset) as *const usize);
            let has_head = (flags & 1) != 0;
            let has_tail = (flags & 2) != 0;
            offset += std::mem::size_of::<usize>();
            
            // Scan head pointer only if has_head is true
            let head_ptr = header_ptr.add(offset) as *const ObjectReference;
            if has_head {
                let slot = BlinkSlot::ObjectRef(Address::from_ptr(head_ptr));
                slot_visitor.visit_slot(slot);
            }
            offset += std::mem::size_of::<ObjectReference>();
            
            // Scan tail pointer only if has_tail is true
            let tail_ptr = header_ptr.add(offset) as *const ObjectReference;
            if has_tail {
                let slot = BlinkSlot::ObjectRef(Address::from_ptr(tail_ptr));
                slot_visitor.visit_slot(slot);
            }
        }
    }
    
    /// Scan a list node: [flags: usize][value: ValueRef][next: ObjectReference]
    fn scan_list_node<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let node_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Check flags (bit 0 = has_next)
            let flags = std::ptr::read_unaligned(node_ptr.add(offset) as *const usize);
            let has_next = (flags & 1) != 0;
            offset += std::mem::size_of::<usize>();
            
            // Scan value first
            let value_ptr = node_ptr.add(offset) as *const ValueRef;
            let value = std::ptr::read_unaligned(value_ptr);
            match value {
                ValueRef::Heap(_) => {
                    let slot = BlinkSlot::ValueRef(Address::from_ptr(value_ptr));
                    slot_visitor.visit_slot(slot);
                }
                _ => {} // Skip immediate/native values
            }
            offset += std::mem::size_of::<ValueRef>();
            
            // Scan next pointer only if has_next is true
            let next_ptr = node_ptr.add(offset) as *const ObjectReference;
            if has_next {
                let slot = BlinkSlot::ObjectRef(Address::from_ptr(next_ptr));
                slot_visitor.visit_slot(slot);
            }
        }
    }
    
    /// Scan a vector object: [length: u32][capacity: u32][data_ptr: ObjectReference]
    fn scan_vector_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let header_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Skip length
            offset += std::mem::size_of::<u32>();
            
            // Skip capacity 
            offset += std::mem::size_of::<u32>();
            
            // Scan data pointer
            let data_ptr_ptr = header_ptr.add(offset) as *const ObjectReference;
            let data_ptr = std::ptr::read_unaligned(data_ptr_ptr);
            // Note: data_ptr should never be None/null for vectors, but let's be safe
            let slot = BlinkSlot::ObjectRef(Address::from_ptr(data_ptr_ptr));
            slot_visitor.visit_slot(slot);
            
            // Also scan the data array contents
            Self::scan_vector_data(slot_visitor, data_ptr, object);
        }
    }
    
    /// Scan the data array of a vector
    fn scan_vector_data<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        data_array: ObjectReference,
        vector_header: ObjectReference
    ) {
        unsafe {
            // Get the length from the vector header
            let header_ptr = vector_header.to_raw_address().as_usize() as *const u8;
            let length = std::ptr::read_unaligned(header_ptr as *const u32) as usize;
            
            // Scan each ValueRef in the data array
            let data_ptr = data_array.to_raw_address().as_usize() as *const u8;
            
            for i in 0..length {
                let value_ptr = data_ptr.add(i * std::mem::size_of::<ValueRef>()) as *const ValueRef;
                let value = std::ptr::read_unaligned(value_ptr);
                
                match value {
                    ValueRef::Heap(_) => {
                        let slot = BlinkSlot::ValueRef(Address::from_ptr(value_ptr));
                        slot_visitor.visit_slot(slot);
                    }
                    _ => {} // Skip immediate/native values
                }
            }
        }
    }

    fn scan_callable<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        unsafe {
            let data_ptr = object.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Read constants_count
            let constants_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Skip other metadata (bytecode_len, param_count, reg_count, etc.)
            offset += std::mem::size_of::<u32>(); // bytecode_len
            offset += std::mem::size_of::<u8>();  // parameter_count
            offset += std::mem::size_of::<u8>();  // register_count
            offset += std::mem::size_of::<u32>(); // module
            offset += std::mem::size_of::<u8>();  // register_start
            offset += std::mem::size_of::<u8>();  // has_self_reference
            
            // Scan constants (ValueRef array)
            for i in 0..constants_count {
                let value_ref_ptr = data_ptr.add(offset) as *const ValueRef;
                let value_ref = std::ptr::read_unaligned(value_ref_ptr);
                
                match value_ref {
                    ValueRef::Heap(_) => {
                        let slot = BlinkSlot::ValueRef(Address::from_ptr(value_ref_ptr));
                        slot_visitor.visit_slot(slot);
                    }
                    _ => {} // Skip immediate/native values
                }
                
                offset += std::mem::size_of::<ValueRef>();
            }
            
            // Skip bytecode (Vec<u8>) - no references to scan
        }
    }

    fn scan_env_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        object: ObjectReference
    ) {
        // NOTE: This is currently being used for list nodes
        // If you have actual environment objects, you'll need to distinguish them
        // For now, treat all as list nodes
        Self::scan_list_node(slot_visitor, object);
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
            
            // Scan all values
            Self::scan_value_ref_seq(slot_visitor, data_ptr, item_count, offset);
        }
    }

    fn scan_error_object<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        _object: ObjectReference
    ) {
        // TODO: Implement based on your error object structure
        // For now, assume no references to scan
    }

    // Helper function to scan a sequence of ValueRefs
    fn scan_value_ref_seq<SV: mmtk::vm::SlotVisitor<<BlinkVM as VMBinding>::VMSlot>>(
        slot_visitor: &mut SV,
        start_ptr: *const u8,
        size: usize,
        mut offset: usize
    ) {
        unsafe {
            for _i in 0..size {
                let value_ref_ptr = start_ptr.add(offset) as *const ValueRef;
                let value_ref = std::ptr::read_unaligned(value_ref_ptr);
                
                match value_ref {
                    ValueRef::Heap(_) => {
                        let slot = BlinkSlot::ValueRef(Address::from_ptr(value_ref_ptr));
                        slot_visitor.visit_slot(slot);
                    }
                    _ => {} // Skip immediate/native values
                }
                
                offset += std::mem::size_of::<ValueRef>();
            }
        }
    }
}