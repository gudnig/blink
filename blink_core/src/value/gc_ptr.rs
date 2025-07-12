use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use mmtk::util::ObjectReference;
use parking_lot::RwLock;
use crate::error::BlinkError;
use crate::module::{Module, SerializedModuleSource};
use crate::runtime::BlinkObjectModel;
use crate::value::Callable;
use crate::env::Env;
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
        let type_tag = BlinkObjectModel::get_type_tag(self.0);
        type_tag
    }

    pub fn to_heap_value(&self) -> HeapValue {
        let (header, type_tag) = BlinkObjectModel::get_header(self.0);
        let data_size = header.total_size as usize - ObjectHeader::SIZE;
        
        match type_tag {
            TypeTag::Str => {
                                    HeapValue::Str(self.read_string(data_size))
                                }
            TypeTag::List => {
                                    HeapValue::List(self.read_vec(data_size))
                                }
            TypeTag::Vector => {
                                    HeapValue::Vector(self.read_vec(data_size))
                                }
            TypeTag::Map => {
                                    HeapValue::Map(self.read_blink_hash_map())
                                }
            TypeTag::Set => HeapValue::Set(self.read_blink_hash_set()),
            TypeTag::Error => HeapValue::Error(self.read_error()),
            TypeTag::UserDefinedFunction => HeapValue::Function(self.read_callable()),
            TypeTag::Macro => todo!(),
            TypeTag::Future => todo!(),
            TypeTag::Env => HeapValue::Env(self.read_env()),
            TypeTag::Module => HeapValue::Module(self.read_module()),
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


pub fn read_env(&self) -> Env {

        //TODO this is not correct, need to match how env is allocated
        unsafe {
            let data_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Read vars count
            let vars_count = *(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Variables are stored sorted by symbol ID
            let vars_ptr = data_ptr.add(offset) as *const (u32, ValueRef);
            let vars_slice = std::slice::from_raw_parts(vars_ptr, vars_count);
            
            // Convert to HashMap for current interface
            let vars = vars_slice.to_vec();
            offset += vars_count * (std::mem::size_of::<u32>() + std::mem::size_of::<ValueRef>());
            
            // Read modules count
            let modules_count = *(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Read module aliases (also stored as pairs)
            let modules_ptr = data_ptr.add(offset) as *const (u32, u32);
            let modules_slice = std::slice::from_raw_parts(modules_ptr, modules_count);
            
            let available_modules = modules_slice.to_vec();
            offset += modules_count * (std::mem::size_of::<u32>() + std::mem::size_of::<u32>());
            
            // Read parent reference
            let parent_ref_ptr = data_ptr.add(offset) as *const Option<ObjectReference>;
            let parent = std::ptr::read(parent_ref_ptr);
            
            
            
            Env { vars, parent, available_modules }
        }
    }



    // Fast lookup without reconstructing HashMap
    pub fn lookup_var(&self, symbol_id: u32) -> Option<ValueRef> {
        unsafe {
            let data_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let vars_count = *(data_ptr as *const u32) as usize;
            let vars_ptr = data_ptr.add(std::mem::size_of::<u32>()) as *const (u32, ValueRef);
            let vars_slice = std::slice::from_raw_parts(vars_ptr, vars_count);
            
            // Binary search on sorted array - O(log n)
            match vars_slice.binary_search_by_key(&symbol_id, |(k, _)| *k) {
                Ok(index) => Some(vars_slice[index].1),
                Err(_) => None,
            }
        }
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


    pub fn read_string(&self, data_size: usize) -> String {
        unsafe {
            let data_start = self.0.to_raw_address().as_usize() as *const u8;
            

            let str_slice = std::slice::from_raw_parts(data_start, data_size);
            
            String::from_utf8_unchecked(str_slice.to_vec())
            
        }
    }

    pub fn read_vec(&self, data_size: usize) -> Vec<ValueRef> {
        unsafe {
            let vec_data_ptr = self.0.to_raw_address().as_usize() as *const ValueRef;
            let item_count = data_size / std::mem::size_of::<ValueRef>();
            let items = std::slice::from_raw_parts(vec_data_ptr, item_count);
            items.to_vec()
        }
    }

    pub fn read_module(&self) -> Module {
        unsafe {
            let mut ptr = self.0.to_raw_address().as_usize() as *const u8;
            
            // Read module name
            let name = *(ptr as *const u32);
            ptr = ptr.add(std::mem::size_of::<u32>());
            
            // Read environment reference
            let env = *(ptr as *const ObjectReference);
            ptr = ptr.add(std::mem::size_of::<ObjectReference>());
            
            // Read exports count
            let exports_count = *(ptr as *const u32) as usize;
            ptr = ptr.add(std::mem::size_of::<u32>());
            
            // Read exports
            let mut exports = Vec::with_capacity(exports_count);
            for _ in 0..exports_count {
                let export_id = *(ptr as *const u32);
                exports.push(export_id);
                ptr = ptr.add(std::mem::size_of::<u32>());
            }
            
            // Read module source
            let source_variant = *(ptr as *const u8);
            ptr = ptr.add(std::mem::size_of::<u8>());
            
            let source = match source_variant {
                0 => SerializedModuleSource::Repl,
                1 => {
                    let symbol_id = *(ptr as *const u32);
                    ptr = ptr.add(std::mem::size_of::<u32>());
                    SerializedModuleSource::BlinkFile(symbol_id)
                }
                // ... other variants
                _ => SerializedModuleSource::Repl,
            };
            
            // Read ready flag
            let ready = *(ptr as *const bool);
            
            Module { name, env, exports, source, ready }
        }
    }

    pub fn read_callable(&self) -> Callable {
        unsafe {
            let mut ptr = self.0.to_raw_address().as_usize() as *const u8;
            
            // Read params count
            let params_count = *(ptr as *const u32) as usize;
            ptr = ptr.add(std::mem::size_of::<u32>());
            
            // Read parameter IDs
            let mut params = Vec::with_capacity(params_count);
            for _ in 0..params_count {
                let param_id = *(ptr as *const u32);
                params.push(param_id);
                ptr = ptr.add(std::mem::size_of::<u32>());
            }
            
            // Read body count
            let body_count = *(ptr as *const u32) as usize;
            ptr = ptr.add(std::mem::size_of::<u32>());
            
            // Read body expressions
            let mut body = Vec::with_capacity(body_count);
            for _ in 0..body_count {
                let expr = *(ptr as *const ValueRef);
                body.push(expr);
                ptr = ptr.add(std::mem::size_of::<ValueRef>());
            }
            
            // Read environment reference
            let env = *(ptr as *const ObjectReference);
            ptr = ptr.add(std::mem::size_of::<ObjectReference>());
            
            // Read variadic flag
            let is_variadic = *(ptr as *const bool);
            
            Callable {
                params,
                body,
                env,
                is_variadic,
            }
        }
    }
}