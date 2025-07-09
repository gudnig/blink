use mmtk::{
    util::{
        metadata::{
            side_metadata::SideMetadataSpec, 
            MetadataSpec,
        }, 
        Address, 
        ObjectReference
    }, 
    vm::{ObjectModel, VMGlobalLogBitSpec, VMLocalForwardingBitsSpec, VMLocalForwardingPointerSpec, VMLocalLOSMarkNurserySpec, VMLocalMarkBitSpec}
};

use crate::{runtime::BlinkVM, value::ValueRef};

#[repr(i8)]
#[derive(PartialEq, Eq, Debug)]
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
            TypeTag::UserDefinedFunction => "function",
            TypeTag::Macro => "macro",
            TypeTag::Future => "future",
            TypeTag::Env => "env",
        }
    }
}

#[repr(C)]
pub struct ObjectHeader {
    pub(crate) // Type information
    type_tag: i8,
    
    // Size of the entire object (header + data)
    total_size: u32,
    
    // Optional: GC metadata can go here instead of MMTk side tables
    // gc_flags: u8,
    // forwarding_ptr: Option<ObjectReference>,
    
    // Padding to ensure proper alignment
    _padding: [u8; 3],
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
        // Safe because we control the values
        unsafe { std::mem::transmute(self.type_tag) }
    }
    
    pub fn data_size(&self) -> usize {
        self.total_size as usize - std::mem::size_of::<Self>()
    }
}

    
    static TYPE_DESCRIPTORS: &[&[i8]] = &[
        &[0], // List
        &[1], // Vector  
        &[2], // Map
        &[3], // Str
        &[4], // Set
        &[5], // Error
        &[6], // Module
        &[7], // UserDefinedFunction
        &[8], // Macro
        &[9], // Future
        &[10], // Env
        &[127], // Unknown
    ];
    


pub struct BlinkObjectModel;

const FORWARDING_POINTER_OFFSET: isize = 0;

impl ObjectModel<BlinkVM> for BlinkObjectModel {
    
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize = 0;
    
    


    
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec = VMGlobalLogBitSpec::side_first();

        
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec = VMLocalForwardingPointerSpec::side_first();
        
    const LOCAL_FORWARDING_BITS_SPEC: mmtk::vm::VMLocalForwardingBitsSpec = VMLocalForwardingBitsSpec::side_after(&Self::LOCAL_FORWARDING_POINTER_SPEC.as_spec());
        
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec = VMLocalMarkBitSpec::side_after(&Self::LOCAL_FORWARDING_POINTER_SPEC.as_spec());
        
        
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec = VMLocalLOSMarkNurserySpec::side_after(&Self::LOCAL_MARK_BIT_SPEC.as_spec());
    
    fn copy(
        _from: ObjectReference,
        _semantics: mmtk::util::copy::CopySemantics,
        _copy_context: &mut mmtk::util::copy::GCWorkerCopyContext<BlinkVM>,
    ) -> ObjectReference {
        // For now, just return the original reference
        // You'll implement actual copying logic later
        todo!("Object copying not yet implemented")
    }
    
    fn copy_to(
        _from: ObjectReference,
        _to: ObjectReference,
        _region: Address,
    ) -> Address {
        todo!("Copy-to not yet implemented")
    }
    
    fn get_reference_when_copied_to(
        _from: ObjectReference,
        _to: Address,
    ) -> ObjectReference {
        todo!("Get reference when copied not yet implemented")
    }
    
    fn get_current_size(_object: ObjectReference) -> usize {
        Self::get_header(_object).total_size as usize
    }
    
    fn get_size_when_copied(_object: ObjectReference) -> usize {
        Self::get_current_size(_object)
    }
    
    fn get_align_when_copied(_object: ObjectReference) -> usize {
        std::mem::align_of::<ValueRef>()
    }
    
    fn get_align_offset_when_copied(_object: ObjectReference) -> usize {
        0
    }
    
    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8] {
        let header = unsafe { reference.to_header::<BlinkVM>().as_ref::<ObjectHeader>() };
        let type_tag = header.type_tag as usize;
        
        // Bounds check to prevent panic
        if type_tag < TYPE_DESCRIPTORS.len() {
            TYPE_DESCRIPTORS[type_tag]
        } else {
            TYPE_DESCRIPTORS[TYPE_DESCRIPTORS.len() - 1]
        }
    }
    
    fn ref_to_object_start(object: ObjectReference) -> Address {
        object.to_raw_address()
    }
    
    fn ref_to_header(object: ObjectReference) -> Address {
        // header is at the beginning
        object.to_raw_address()
    }
    
    fn dump_object(_object: ObjectReference) {
        // Debug printing for objects
        println!("Blink object dump not yet implemented");
    }
}

impl BlinkObjectModel {
    fn get_data_address(object: ObjectReference) -> Address {
        object.to_raw_address() + std::mem::size_of::<ObjectHeader>()
    }

    pub fn get_header(object: ObjectReference) -> &'static ObjectHeader {
        unsafe { object.to_header::<BlinkVM>().as_ref::<ObjectHeader>() }
    }
}