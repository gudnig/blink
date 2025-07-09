// blink_core/src/runtime/mmtk/object_model.rs

use mmtk::{
    util::{
        metadata::{side_metadata::SideMetadataSpec, MetadataSpec},
        Address, ObjectReference
    },
    vm::{ObjectModel, VMGlobalLogBitSpec, VMLocalForwardingBitsSpec, VMLocalForwardingPointerSpec, VMLocalLOSMarkNurserySpec, VMLocalMarkBitSpec}
};
use crate::{runtime::BlinkVM, value::ValueRef};

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
    Future = 8,
    Env = 9,
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
            TypeTag::Future => "future",
            TypeTag::Env => "env",
        }
    }
}

#[repr(C)]
pub struct ObjectHeader {
    pub type_tag: i8,
    pub total_size: u32,
    pub _padding: [u8; 3],
}

impl ObjectHeader {
    pub fn new(type_tag: TypeTag, data_size: usize) -> Self {
        Self {
            type_tag: type_tag as i8,
            total_size: (std::mem::size_of::<Self>() + data_size) as u32,
            _padding: [0; 3],
        }
    }
    
    pub fn get_type(&self) -> TypeTag {
        unsafe { std::mem::transmute(self.type_tag) }
    }
}

pub struct BlinkObjectModel;

impl ObjectModel<BlinkVM> for BlinkObjectModel {
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = 0;
    
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::side_first();
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec = VMLocalForwardingPointerSpec::side_first();
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec = VMLocalForwardingBitsSpec::side_after(&Self::LOCAL_FORWARDING_POINTER_SPEC.as_spec());
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::side_after(&Self::LOCAL_FORWARDING_BITS_SPEC.as_spec());
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec = VMLocalLOSMarkNurserySpec::side_after(&Self::LOCAL_MARK_BIT_SPEC.as_spec());
    
    fn copy(
        from: ObjectReference,
        _semantics: mmtk::util::copy::CopySemantics,
        _copy_context: &mut mmtk::util::copy::GCWorkerCopyContext<BlinkVM>,
    ) -> ObjectReference {
        // For NoGC, we don't actually copy - just return the original
        from
    }
    
    fn copy_to(
        _from: ObjectReference,
        _to: ObjectReference,
        _region: Address,
    ) -> Address {
        // For NoGC, not needed
        panic!("copy_to called on NoGC plan")
    }
    
    fn get_reference_when_copied_to(
        _from: ObjectReference,
        to: Address,
    ) -> ObjectReference {
        ObjectReference::from_raw_address(to).unwrap()
    }
    
    fn get_current_size(object: ObjectReference) -> usize {
        unsafe {
            let header_ptr = object.to_raw_address().as_usize() as *const ObjectHeader;
            let header = std::ptr::read(header_ptr);
            header.total_size as usize
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
    
    fn get_type_descriptor(_reference: ObjectReference) -> &'static [i8] {
        let header_ptr = _reference.to_raw_address().as_usize() as *const ObjectHeader;
        let header = unsafe { std::ptr::read(header_ptr) };
        let type_tag = header.type_tag as usize;
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
        object.to_raw_address()
    }
    
    fn ref_to_header(object: ObjectReference) -> Address {
        object.to_raw_address()
    }
    
    fn dump_object(object: ObjectReference) {
        println!("Blink object at {:?}", object.to_raw_address());
    }
}