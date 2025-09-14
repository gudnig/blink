// blink_core/src/runtime/mmtk/object_model.rs
use mmtk::{
    util::{
        copy::{CopySemantics, GCWorkerCopyContext},
        Address, ObjectReference
    },
    vm::{ObjectModel, VMGlobalLogBitSpec, VMLocalForwardingBitsSpec, 
        VMLocalForwardingPointerSpec, VMLocalLOSMarkNurserySpec, VMLocalMarkBitSpec}, 
};
use crate::runtime::{BlinkVM, mmtk::object_header::{ObjectHeader}};

#[repr(i8)]
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum TypeTag {
    List = 0,
    Vector = 1,
    Map = 2,
    Str = 3,
    Set = 4,
    Error = 5,
    UserDefinedFunction = 6,
    Macro = 7,
    Closure = 8,
    Env = 9,
    ListNode = 10,
}

impl TypeTag {
    pub fn to_str(&self) -> &'static str {
        match self {
            TypeTag::List => "list",
            TypeTag::Vector => "vector",
            TypeTag::Map => "map",
            TypeTag::Str => "str",
            TypeTag::Set => "set",
            TypeTag::Error => "error",
            TypeTag::UserDefinedFunction => "user-function",
            TypeTag::Macro => "macro",
            TypeTag::Env => "env",
            TypeTag::Closure => "closure",
            TypeTag::ListNode => "list-node",
        }
    }
}

pub struct BlinkObjectModel;

impl ObjectModel<BlinkVM> for BlinkObjectModel {
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = ObjectHeader::SIZE as isize;
    
    // Keep your existing side-based specs for GC metadata
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::side_first();
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::side_first();
    
    // Header-based metadata specs
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec = VMLocalForwardingBitsSpec::in_header(2);
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec = VMLocalForwardingPointerSpec::in_header(8);
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec = VMLocalLOSMarkNurserySpec::in_header(3);
    
    // ... rest of your implementation stays the same
    
    fn copy(
        from: ObjectReference,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<BlinkVM>,
    ) -> ObjectReference {
        let total_size = Self::get_current_size(from);
        
        // Allocate space for the copy
        let to_obj = copy_context.alloc_copy(from, total_size, 8, 0, semantics);
        let obj_ref = ObjectReference::from_raw_address(to_obj).unwrap();
        
        // Copy the entire object including header
        let from_start = Self::ref_to_header(from);
        let to_start = Self::ref_to_header(obj_ref);
        
        unsafe {
            std::ptr::copy_nonoverlapping(
                from_start.to_ptr::<u8>(),
                to_start.to_mut_ptr::<u8>(),
                total_size
            );
            
            // Reset lockword in the copy (objects start unlocked)
            let to_header = to_start.to_mut_ptr::<ObjectHeader>();
            (*to_header).lockword.store(0, std::sync::atomic::Ordering::Relaxed);
            
            // Object ID is copied automatically with the header - no extra work needed!
        }
        
        obj_ref
    }
    
    fn copy_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address {
        let total_size = Self::get_current_size(from);
        
        // Copy the entire object including header
        let from_start = Self::ref_to_header(from);
        let to_start = Self::ref_to_header(to);
        
        unsafe {
            std::ptr::copy_nonoverlapping(
                from_start.to_ptr::<u8>(),
                to_start.to_mut_ptr::<u8>(),
                total_size
            );
            
            // Reset lockword in the copy (objects start unlocked)
            let to_header = to_start.to_mut_ptr::<ObjectHeader>();
            (*to_header).lockword.store(0, std::sync::atomic::Ordering::Relaxed);
            
            // Object ID is automatically copied with the header - no extra work needed!
        }
        
        // Return the address after the copied object
        to.to_raw_address() + total_size
    }

    
    fn get_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference {
        // The object reference should point to the same offset within the object
        // as it did in the original object
        let from_start = Self::ref_to_object_start(from);
        let offset = from.to_raw_address() - from_start;
        
        ObjectReference::from_raw_address(to + offset).unwrap()
    }
    
    fn get_current_size(object: ObjectReference) -> usize {
        unsafe {
            // Go back to header start
            let header_ptr = Self::ref_to_header(object).to_ptr::<ObjectHeader>();
            (*header_ptr).total_size as usize
        }
    }
    
    fn get_size_when_copied(object: ObjectReference) -> usize {
        Self::get_current_size(object)
    }
    
    fn get_align_when_copied(_object: ObjectReference) -> usize {
        8 // 8-byte alignment
    }
    
    fn get_align_offset_when_copied(_object: ObjectReference) -> usize {
        0
    }
    
    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8] {
        let header_ptr = Self::ref_to_header(reference).to_ptr::<ObjectHeader>();
        let type_tag = unsafe { (*header_ptr).type_tag };
        
        // Return the appropriate type descriptor based on the object's type
        match type_tag {
            0 => &[0], // List
            1 => &[1], // Vector
            2 => &[2], // Map
            3 => &[3], // Str
            4 => &[4], // Set
            5 => &[5], // Error
            6 => &[6], // UserDefinedFunction
            7 => &[7], // Macro
            8 => &[8], // Future
            9 => &[9], // Env
            _ => &[127], // Unknown/invalid type
        }
    }
    
    fn ref_to_object_start(object: ObjectReference) -> Address {
        // Object data starts after header
        object.to_raw_address()
    }
    
    fn ref_to_header(object: ObjectReference) -> Address {
        // Go back from object data to header
        let object_addr = object.to_raw_address();
        let header_size = ObjectHeader::SIZE;
        
        // Validate that the address is valid and won't cause underflow
        if object_addr.as_usize() < header_size {
            panic!("Invalid object address: {:?} is too small to contain header of size {}", object_addr, header_size);
        }
        
        object_addr - header_size
    }
    
    fn dump_object(object: ObjectReference) {
        unsafe {
            let header = Self::ref_to_header(object).to_ptr::<ObjectHeader>();
            let type_tag = (*header).get_type();
            let size = (*header).total_size;
            println!(
                "Blink {} object at {:?}, total size: {} bytes", 
                type_tag.to_str(),
                object.to_raw_address(),
                size
            );
        }
    }
}

// Helper functions for allocation
impl BlinkObjectModel {
    /// Get the type tag of an object
    pub fn get_type_tag(object: ObjectReference) -> TypeTag {
        unsafe {
            let header_ptr = Self::ref_to_header(object).to_ptr::<ObjectHeader>();
            (*header_ptr).get_type()
        }
    }

    pub fn get_header(object: ObjectReference) -> (ObjectHeader, TypeTag) {
        unsafe {
            let header_ptr = Self::ref_to_header(object).to_ptr::<ObjectHeader>();
            let header = std::ptr::read(header_ptr);
            let type_tag = std::mem::transmute::<i8, TypeTag>(header.type_tag);
            (header, type_tag)
        }
    }
    
    /// Get just the data size (excluding header)
    pub fn get_data_size(object: ObjectReference) -> usize {
        Self::get_current_size(object) - ObjectHeader::SIZE
    }
}

