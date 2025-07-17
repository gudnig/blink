use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use mmtk::util::ObjectReference;
use parking_lot::RwLock;
use crate::error::{BlinkError, BlinkErrorType, ParseErrorType};
use crate::module::{Module, SerializedModuleSource};
use crate::runtime::BlinkObjectModel;
use crate::value::{Callable, SourceRange};
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
            TypeTag::Macro => HeapValue::Macro(self.read_callable()),
            TypeTag::Future => todo!(),
            TypeTag::Env => HeapValue::Env(self.read_env()),
            TypeTag::Module => HeapValue::Module(self.read_module()),
        }
    }

    pub fn read_map(&self) -> Vec<(ValueRef, ValueRef)> {
        unsafe {
            let base_ptr = self.0.to_raw_address().as_usize() as *mut u8;
            let (_header, type_tag) = BlinkObjectModel::get_header(self.0);
            
            // Verify this is actually a map
            debug_assert_eq!(type_tag, TypeTag::Map);
            
            let data_start = base_ptr;
            
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


    pub fn read_blink_hash_map(&self) -> BlinkHashMap {
        let pairs = self.read_map();
        BlinkHashMap::from_pairs(pairs)
    }


    pub fn read_env(&self) -> Env {
        unsafe {
            let data_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Read parent reference
            let parent = std::ptr::read_unaligned(data_ptr.add(offset) as *const Option<ObjectReference>);
            offset += std::mem::size_of::<Option<ObjectReference>>();
            
            // Read counts
            let vars_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            let symbol_aliases_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            let module_aliases_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Read all ValueRefs
            let mut values = Vec::with_capacity(vars_count);
            for _ in 0..vars_count {
                let value = std::ptr::read_unaligned(data_ptr.add(offset) as *const ValueRef);
                values.push(value);
                offset += std::mem::size_of::<ValueRef>();
            }
            
            // Read all keys (same order as values)
            let mut vars = Vec::with_capacity(vars_count);
            for i in 0..vars_count {
                let key = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
                vars.push((key, values[i]));
                offset += std::mem::size_of::<u32>();
            }
            
            // Read symbol aliases
            let mut symbol_aliases = Vec::new();
            for _ in 0..symbol_aliases_count {
                let alias = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                let module_id = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                let symbol_id = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                symbol_aliases.push((alias, (module_id, symbol_id)));
            }
            
            // Read module aliases
            let mut module_aliases = Vec::new();
            for _ in 0..module_aliases_count {
                let alias = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                let module_id = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                module_aliases.push((alias, module_id));
            }
            
            Env { vars, symbol_aliases, module_aliases, parent }
        }
    }
    

    pub fn read_error(&self) -> BlinkError {
        unsafe {
            let data_ptr = self.0.to_raw_address().as_usize() as *const u8;
            let mut offset = 0;
            
            // Read message length
            let message_len = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Read message bytes
            let message_bytes = std::slice::from_raw_parts(data_ptr.add(offset), message_len);
            let message = String::from_utf8_lossy(message_bytes).into_owned();
            offset += message_len;
            
            // Read position
            let pos = std::ptr::read_unaligned(data_ptr.add(offset) as *const Option<SourceRange>);
            offset += std::mem::size_of::<Option<SourceRange>>();
            
            // Read error type
            let error_type = Self::read_error_type_data(data_ptr.add(offset));
            
            BlinkError {
                message,
                pos,
                error_type,
            }
        }
    }

    unsafe fn read_error_type_data(ptr: *const u8) -> BlinkErrorType {
        let mut offset = 0;
        
        // Read discriminant
        let discriminant = std::ptr::read_unaligned(ptr.add(offset) as *const u8);
        offset += std::mem::size_of::<u8>();
        
        match discriminant {
            0 => BlinkErrorType::Tokenizer,
            1 => {
                let parse_discriminant = std::ptr::read_unaligned(ptr.add(offset) as *const u8);
                let parse_type = match parse_discriminant {
                    0 => ParseErrorType::UnexpectedEof,
                    _ => ParseErrorType::UnexpectedEof, // fallback
                };
                BlinkErrorType::Parse(parse_type)
            },
            2 => {
                let name_len = std::ptr::read_unaligned(ptr.add(offset) as *const u32) as usize;
                offset += std::mem::size_of::<u32>();
                
                let name_bytes = std::slice::from_raw_parts(ptr.add(offset), name_len);
                let name = String::from_utf8_lossy(name_bytes).into_owned();
                
                BlinkErrorType::UndefinedSymbol { name }
            },
            3 => BlinkErrorType::Eval,
            4 => {
                let expected = std::ptr::read_unaligned(ptr.add(offset) as *const usize);
                offset += std::mem::size_of::<usize>();
                
                let got = std::ptr::read_unaligned(ptr.add(offset) as *const usize);
                offset += std::mem::size_of::<usize>();
                
                let form_len = std::ptr::read_unaligned(ptr.add(offset) as *const u32) as usize;
                offset += std::mem::size_of::<u32>();
                
                let form_bytes = std::slice::from_raw_parts(ptr.add(offset), form_len);
                let form = String::from_utf8_lossy(form_bytes).into_owned();
                
                BlinkErrorType::ArityMismatch { expected, got, form }
            },
            5 => {
                let token_len = std::ptr::read_unaligned(ptr.add(offset) as *const u32) as usize;
                offset += std::mem::size_of::<u32>();
                
                let token_bytes = std::slice::from_raw_parts(ptr.add(offset), token_len);
                let token = String::from_utf8_lossy(token_bytes).into_owned();
                
                BlinkErrorType::UnexpectedToken { token }
            },
            6 => {
                let has_data = std::ptr::read_unaligned(ptr.add(offset) as *const u8);
                offset += std::mem::size_of::<u8>();
                
                let data = if has_data == 1 {
                    Some(std::ptr::read_unaligned(ptr.add(offset) as *const ValueRef))
                } else {
                    None
                };
                
                BlinkErrorType::UserDefined { data }
            },
            _ => BlinkErrorType::Eval, // fallback
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
    let data_ptr = self.0.to_raw_address().as_usize() as *const u8;
    
    unsafe {
        let mut offset = 0;
        
        let env = std::ptr::read_unaligned(data_ptr.add(offset) as *const ObjectReference);
        offset += std::mem::size_of::<ObjectReference>();
        
        let name = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
        offset += std::mem::size_of::<u32>();
        
        let exports_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
        offset += std::mem::size_of::<u32>();
        
        let mut exports = Vec::with_capacity(exports_count);
        for i in 0..exports_count {
            let export = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
            exports.push(export);
            offset += std::mem::size_of::<u32>();
        }
        
        let source = std::ptr::read_unaligned(data_ptr.add(offset) as *const SerializedModuleSource);
        offset += std::mem::size_of::<SerializedModuleSource>();
        
        let ready = std::ptr::read_unaligned(data_ptr.add(offset) as *const bool);
        
        Module { name, env, exports, source, ready }
    }
}

    
pub fn read_callable(&self) -> Callable {
    unsafe {
        let data_ptr = self.0.to_raw_address().as_usize() as *const u8;
        let mut offset = 0;
        
        // Read env reference FIRST
        let env = std::ptr::read_unaligned(data_ptr.add(offset) as *const ObjectReference);
        offset += std::mem::size_of::<ObjectReference>();
        
        // Read body count
        let body_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
        offset += std::mem::size_of::<u32>();
        
        // Read all body expressions
        let mut body = Vec::with_capacity(body_count);
        for _ in 0..body_count {
            let expr = std::ptr::read_unaligned(data_ptr.add(offset) as *const ValueRef);
            body.push(expr);
            offset += std::mem::size_of::<ValueRef>();
        }
        
        // Read params count
        let params_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
        offset += std::mem::size_of::<u32>();
        
        // Read parameter IDs
        let mut params = Vec::with_capacity(params_count);
        for _ in 0..params_count {
            let param_id = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32);
            params.push(param_id);
            offset += std::mem::size_of::<u32>();
        }
        
        // Read variadic flag
        let is_variadic = std::ptr::read_unaligned(data_ptr.add(offset) as *const bool);
        
        Callable {
            params,
            body,
            env,
            is_variadic,
        }
    }
}
}