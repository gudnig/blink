use std::hash::{Hash, Hasher};

use mmtk::util::ObjectReference;
use crate::error::BlinkError;
use crate::value::Callable;
use crate::{collections::{BlinkHashMap, BlinkHashSet}, value::ValueRef};
use crate::{runtime::{ObjectHeader, TypeTag}, value::HeapValue};



#[derive(Debug, Copy, Clone)]
pub struct GcPtr(pub ObjectReference);

impl Hash for GcPtr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let heap_val = self.to_heap_value();
        heap_val.hash(state);
    }
}

impl PartialEq for GcPtr {
    fn eq(&self, other: &Self) -> bool {
        let type_tag: TypeTag = self.type_tag();
        match type_tag {
            // ref equality for error, user defined function, macro, future, env
            TypeTag::Error => self.0 == other.0,
            TypeTag::UserDefinedFunction => self.0 == other.0,
            TypeTag::Macro => self.0 == other.0,
            TypeTag::Future => self.0 == other.0,
            TypeTag::Env => self.0 == other.0,
            _ => {
                let heap_val = self.to_heap_value();
                let other_heap_val = other.to_heap_value();
                heap_val == other_heap_val
            }
        }
    }
}
impl Eq for GcPtr {}

impl GcPtr {
    pub fn new(ptr: ObjectReference) -> Self {
        Self(ptr)
    }

    pub fn type_tag(&self) -> TypeTag {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let header_ptr = base_ptr as *const ObjectHeader;
            let header = std::ptr::read(header_ptr);
            header.get_type()
        }
    }

    pub fn to_heap_value(&self) -> HeapValue {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let header_ptr = base_ptr as *const ObjectHeader;
            let header = std::ptr::read(header_ptr);
            
            match header.get_type() {
                TypeTag::Str => {
                                HeapValue::Str(self.read_string())
                            }
                TypeTag::List => {
                                HeapValue::List(self.read_vec())
                            }
                TypeTag::Vector => {
                                HeapValue::Vector(self.read_vec())
                            }
                TypeTag::Map => {
                                HeapValue::Map(self.read_blink_hash_map())
                            }
                TypeTag::Set => HeapValue::Set(self.read_blink_hash_set()),
                TypeTag::Error => HeapValue::Error(self.read_error()),
                
                TypeTag::UserDefinedFunction => HeapValue::Function(self.read_callable()),
                TypeTag::Macro => todo!(),
                TypeTag::Future => todo!(),
                TypeTag::Env => todo!(),
            }
        }
    }

    pub fn read_map(&self) -> Vec<(ValueRef, ValueRef)> {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *mut u8;
            let header_ptr = base_ptr as *mut ObjectHeader;
            let header = std::ptr::read(header_ptr);
            
            // Verify this is actually a map
            debug_assert_eq!(header.type_tag, TypeTag::Map as i8);
            
            let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
            
            // Read metadata
            let bucket_count_ptr = data_start as *const usize;
            let bucket_count = std::ptr::read(bucket_count_ptr);
            
            let item_count_ptr = data_start.add(std::mem::size_of::<usize>()) as *const usize;
            let item_count = std::ptr::read(item_count_ptr);
            
            // Calculate layout
            let header_size = std::mem::size_of::<usize>() * 2;
            let buckets_size = bucket_count * std::mem::size_of::<u32>();
            
            // Read all pairs
            let pairs_ptr = data_start.add(header_size + buckets_size) as *const ValueRef;
            let mut pairs = Vec::with_capacity(item_count);
            
            for i in 0..item_count {
                let key = std::ptr::read(pairs_ptr.add(i * 2));
                let val = std::ptr::read(pairs_ptr.add(i * 2 + 1));
                pairs.push((key, val));
            }
            
            pairs
        }
    }

    pub fn read_callable(&self) -> Callable {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let header_ptr = base_ptr as *const ObjectHeader;
            let header = std::ptr::read(header_ptr);
        }
        todo!()
    }

    pub fn read_error(&self) -> BlinkError {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let header_ptr = base_ptr as *const ObjectHeader;
            let header = std::ptr::read(header_ptr);
        }
        todo!()
    }

    pub fn read_blink_hash_map(&self) -> BlinkHashMap {
        let pairs = self.read_map();
        BlinkHashMap::from_pairs(pairs)
    }



    pub fn read_set(&self) -> Vec<ValueRef> {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *mut u8;
            let header_ptr = base_ptr as *mut ObjectHeader;
            let header = std::ptr::read(header_ptr);

            // Verify this is actually a set
            debug_assert_eq!(header.type_tag, TypeTag::Set as i8);
            
            let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
            
            // Read metadata
            let bucket_count_ptr = data_start as *const usize;
            let bucket_count = std::ptr::read(bucket_count_ptr);
            
            let item_count_ptr = data_start.add(std::mem::size_of::<usize>()) as *const usize;
            let item_count = std::ptr::read(item_count_ptr);
            
            // Calculate layout
            let header_size = std::mem::size_of::<usize>() * 2;
            let buckets_size = bucket_count * std::mem::size_of::<u32>();
            
            // Read all items
            let items_ptr = data_start.add(header_size + buckets_size) as *const ValueRef;
            let mut items = Vec::with_capacity(item_count);
            
            for i in 0..item_count {
                let item = std::ptr::read(items_ptr.add(i));
                items.push(item);
            }
            
            items
        }
    }

    pub fn read_blink_hash_set(&self) -> BlinkHashSet {
        let items = self.read_set();
        BlinkHashSet::from_values(&items)
    }


    pub fn read_string(&self) -> String {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
            
            // This gives you the STRING length, not the total data size
            let len_ptr = data_start as *const u32;
            let string_len = std::ptr::read(len_ptr) as usize;
            
            // Now read exactly string_len bytes
            let str_data_ptr = data_start.add(std::mem::size_of::<u32>());
            let str_slice = std::slice::from_raw_parts(str_data_ptr, string_len);
            
            String::from_utf8_unchecked(str_slice.to_vec())
            
        }
    }

    pub fn read_vec(&self) -> Vec<ValueRef> {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
            
            // Read length
            let len_ptr = data_start as *const usize;
            let len = std::ptr::read(len_ptr);
            
            // Skip capacity (we stored len + capacity)
            let vec_data_ptr = data_start
                .add(std::mem::size_of::<usize>() * 2) as *const ValueRef;
            
            // Read the ValueRef array
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                let item = std::ptr::read(vec_data_ptr.add(i));
                items.push(item);
            }
            items
        }
    }
}