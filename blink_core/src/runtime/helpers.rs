use std::sync::Arc;

use parking_lot::RwLock;

use crate::{collections::{BlinkHashMap, BlinkHashSet}, error::BlinkError, future::BlinkFuture, runtime::{BlinkVM, CompiledFunction, ExecutionContext, TypeTag}, value::{unpack_immediate, Callable, GcPtr, HeapValue, ImmediateValue, NativeFn, ValueRef}};
use crate::env::Env;

impl BlinkVM {


    pub fn get_number(&self, val: ValueRef) -> Option<f64> {
        if let ValueRef::Immediate(packed) = val {
            let unpacked = unpack_immediate(packed);
            if let ImmediateValue::Number(n) = unpacked {
                return Some(n);
            }
        } 
        None
    }

    pub fn get_symbol_id(&self, val: ValueRef) -> Option<u32> {
        if let ValueRef::Immediate(packed) = val {
            let unpacked = unpack_immediate(packed);
            if let ImmediateValue::Symbol(id) = unpacked {
                return Some(id);
            }
        }
        None
    }

    pub fn get_symbol_name(&self, id: u32) -> Option<String> {
        self.symbol_table.read().get_symbol(id)
    }

    pub fn get_bool(&self, val: ValueRef) -> Option<bool> {
        if let ValueRef::Immediate(packed) = val {
            let unpacked = unpack_immediate(packed);
            if let ImmediateValue::Bool(b) = unpacked {
                return Some(b);
            }
        }
        None

    }

    // ------------------------------------------------------------
    // Value creation
    // ------------------------------------------------------------
    pub fn bool_value(self, value: bool) -> ValueRef {
        ValueRef::boolean(value)
    }

    pub fn number_value(self, value: f64) -> ValueRef {
        ValueRef::number(value)
    }
    pub fn symbol_value(self, value: &str) -> ValueRef {
        let mut symbol_table =  self.symbol_table.write();
        let symbol = symbol_table.intern(value);
        ValueRef::symbol(symbol)
    }

    pub fn nil_value( &self) -> ValueRef {
        ValueRef::nil()
    }

    pub fn string_value(&self, value: &str) -> ValueRef {
        let r = self.alloc_str(value);
        ValueRef::Heap(GcPtr::new(r))
    }

    pub fn list_value( &self, values: Vec<ValueRef>) -> ValueRef {
        
        let object_ref = self.alloc_vec_or_list(values, true, None);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn vector_value( &self, values: Vec<ValueRef>) -> ValueRef {
        let object_ref = self.alloc_vec_or_list(values, false, None);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn map_value(&self, pairs: Vec<(ValueRef, ValueRef)>) -> ValueRef {
        let map = BlinkHashMap::from_pairs(pairs);
        let object_ref = self.alloc_blink_hash_map(map);
        ValueRef::Heap(GcPtr::new(object_ref))
    }   

    pub fn set_value( &self, set: Vec<ValueRef>) -> ValueRef {
        let set = BlinkHashSet::from_iter(set);
        let object_ref = self.alloc_blink_hash_set(set);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn future_value( &self, future: BlinkFuture) -> ValueRef {
        let object_ref = self.alloc_future(future);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn native_function_value( &self, func: NativeFn) -> ValueRef {
        match func {
            NativeFn::Isolated(f) => {
                ValueRef::isolated_native_fn(f)
            }
            NativeFn::Contextual(f) => {
                ValueRef::contextual_native_fn(f)
            }
        }
    }

    pub fn user_defined_function_value( &self, function: CompiledFunction) -> ValueRef {
        let object_ref = self.alloc_user_defined_fn(function);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_map_value( &self) -> ValueRef {
        let map = BlinkHashMap::new();
        let object_ref = self.alloc_blink_hash_map(map);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_set_value( &self) -> ValueRef {
        let set = BlinkHashSet::new();
        let object_ref = self.alloc_blink_hash_set(set);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_list_value( &self) -> ValueRef {
        let list = Vec::new();
        let object_ref = self.alloc_vec_or_list(list, true, None);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_vector_value( &self) -> ValueRef {
        let vector = Vec::new();
        let object_ref = self.alloc_vec_or_list(vector, false, None);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    pub fn error_value( &self, error: BlinkError) -> ValueRef {
        let object_ref = self.alloc_error(error);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    // Error creation ----------------------------
    pub fn eval_error( &self, message: &str) -> ValueRef {
        self.error_value(BlinkError::eval(message))
    }
    
    pub fn arity_error( &self, expected: usize, got: usize, form: &str) -> ValueRef {
        self.error_value(BlinkError::arity(expected, got, form))
    }
    
    pub fn undefined_symbol_error( &self, name: &str) -> ValueRef {
        self.error_value(BlinkError::undefined_symbol(name))
    }

    
    // ============================================================================
    // SAFE VALUE EXTRACTION (WITH GOOD ERROR MESSAGES)
    // ============================================================================
    
    /// Extract number from specific argument position
    pub fn require_number(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<f64, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        args[index].get_number()
            .ok_or_else(|| BlinkError::eval(format!("{} expects number at position {}", fn_name, index)))
    }
    
    /// Extract string from specific argument position
    pub fn require_string(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<String, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        args[index].get_string()    
            .ok_or_else(|| BlinkError::eval(format!("{} expects string at position {}", fn_name, index)))
    }
    
    /// Extract boolean from specific argument position
    pub fn require_bool(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<bool, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        args[index].get_bool()
            .ok_or_else(|| BlinkError::eval(format!("{} expects boolean at position {}", fn_name, index)))
    }
    

    
    /// Extract keyword name from specific argument position
    pub fn require_keyword(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<String, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        self.get_keyword_name(args[index])
            .ok_or_else(|| BlinkError::eval(format!("{} expects keyword at position {}", fn_name, index)))
    }

    pub fn get_keyword_name(&self, val: ValueRef) -> Option<String> {
        if let ValueRef::Immediate(packed) = val {
            if let ImmediateValue::Keyword(id) = unpack_immediate(packed) {
                let symbol_table = self.symbol_table.read();
                let full_name = symbol_table.get_symbol(id);
                println!("Full name: {:?}", full_name);
                // Strip the ":" prefix and convert to owned String
                let name = full_name.map(|s| s.to_string());
                println!("Name: {:?}", name);
                name
            } else {
                None
            }
        } else {
            None
        }
    }
    
    /// Extract list/vector items from specific argument position
    pub fn require_vec_or_list(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<Vec<ValueRef>, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        if let Some(vec) = args[index].get_vec() {
            return Ok(vec);
        } else if let Some(list) = args[index].get_list() {
            return Ok(list);
        }
        Err(BlinkError::eval(format!("{} expects list/vector at position {}", fn_name, index)))
    }

    pub fn require_list(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<Vec<ValueRef>, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        if let Some(list) = args[index].get_list() {
            return Ok(list);
        }
        Err(BlinkError::eval(format!("{} expects list at position {}", fn_name, index)))
    }



    pub fn require_vec(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<Vec<ValueRef>, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        if let Some(vec) = args[index].get_vec() {
            return Ok(vec);
        }
        Err(BlinkError::eval(format!("{} expects vector at position {}", fn_name, index)))
    }

}
