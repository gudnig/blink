use std::sync::Arc;

use parking_lot::RwLock;

use crate::{collections::{BlinkHashMap, BlinkHashSet}, error::BlinkError, future::BlinkFuture, runtime::{EvalContext, TypeTag}, value::{unpack_immediate, Callable, GcPtr, HeapValue, ImmediateValue, NativeFn, ValueRef}};
use crate::env::Env;

impl EvalContext {


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
    pub fn bool_value(&mut self, value: bool) -> ValueRef {
        ValueRef::boolean(value)
    }

    pub fn number_value(&mut self, value: f64) -> ValueRef {
        ValueRef::number(value)
    }
    pub fn symbol_value(&mut self, value: &str) -> ValueRef {
        let mut symbol_table =  self.vm.symbol_table.write();
        let symbol = symbol_table.intern(value);
        ValueRef::symbol(symbol)
    }

    pub fn nil_value(&mut self) -> ValueRef {
        ValueRef::nil()
    }

    pub fn string_value(&mut self, value: &str) -> ValueRef {
        let r = self.vm.alloc_str(value);
        ValueRef::Heap(GcPtr::new(r))
    }

    pub fn list_value(&mut self, values: Vec<ValueRef>) -> ValueRef {
        
        let object_ref = self.vm.alloc_vec_or_list(values, true);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn vector_value(&mut self, values: Vec<ValueRef>) -> ValueRef {
        let object_ref = self.vm.alloc_vec_or_list(values, false);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn map_value(&mut self, pairs: Vec<(ValueRef, ValueRef)>) -> ValueRef {
        let map = BlinkHashMap::from_pairs(pairs);
        let object_ref = self.vm.alloc_blink_hash_map(map);
        ValueRef::Heap(GcPtr::new(object_ref))
    }   

    pub fn set_value(&mut self, set: Vec<ValueRef>) -> ValueRef {
        let set = BlinkHashSet::from_iter(set);
        let object_ref = self.vm.alloc_blink_hash_set(set);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn future_value(&mut self, future: BlinkFuture) -> ValueRef {
        let object_ref = self.vm.alloc_future(future);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn native_function_value(&mut self, func: NativeFn) -> ValueRef {
        match func {
            NativeFn::Isolated(f) => {
                ValueRef::isolated_native_fn(f)
            }
            NativeFn::Contextual(f) => {
                ValueRef::contextual_native_fn(f)
            }
        }
    }

    pub fn user_defined_function_value(&mut self, function: Callable) -> ValueRef {
        let object_ref = self.vm.alloc_user_defined_fn(function);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn macro_value(&mut self, mac: Callable) -> ValueRef {
        let object_ref = self.vm.alloc_macro(mac);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn module_value(&mut self, module_name: u32, symbol_name: u32) -> ValueRef {
        ValueRef::module(module_name, symbol_name)
    }

    pub fn empty_map_value(&mut self) -> ValueRef {
        let map = BlinkHashMap::new();
        let object_ref = self.vm.alloc_blink_hash_map(map);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_set_value(&mut self) -> ValueRef {
        let set = BlinkHashSet::new();
        let object_ref = self.vm.alloc_blink_hash_set(set);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_list_value(&mut self) -> ValueRef {
        let list = Vec::new();
        let object_ref = self.vm.alloc_vec_or_list(list, true);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn empty_vector_value(&mut self) -> ValueRef {
        let vector = Vec::new();
        let object_ref = self.vm.alloc_vec_or_list(vector, false);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    pub fn error_value(&mut self, error: BlinkError) -> ValueRef {
        let object_ref = self.vm.alloc_error(error);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    // Error creation ----------------------------
    pub fn eval_error(&mut self, message: &str) -> ValueRef {
        self.error_value(BlinkError::eval(message))
    }
    
    pub fn arity_error(&mut self, expected: usize, got: usize, form: &str) -> ValueRef {
        self.error_value(BlinkError::arity(expected, got, form))
    }
    
    pub fn undefined_symbol_error(&mut self, name: &str) -> ValueRef {
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
    
    /// Extract symbol name from specific argument position
    pub fn require_symbol(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<String, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        self.get_symbol_name(args[index])
            .ok_or_else(|| BlinkError::eval(format!("{} expects symbol at position {}", fn_name, index)))
    }
    
    /// Extract keyword name from specific argument position
    pub fn require_keyword(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<String, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        self.get_keyword_name(args[index])
            .ok_or_else(|| BlinkError::eval(format!("{} expects keyword at position {}", fn_name, index)))
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

    // ============================================================================
    // FORMATTING
    // ============================================================================
    
    pub fn format_value(&self, val: ValueRef) -> String {
        match val {
            ValueRef::Immediate(packed) => {
                match unpack_immediate(packed) {
                    ImmediateValue::Number(n) => n.to_string(),
                    ImmediateValue::Bool(b) => b.to_string(),
                    ImmediateValue::Symbol(id) => {
                                                        self.vm.symbol_table.read().get_symbol(id).unwrap_or("<unknown>").to_string()
                                                    }
                    ImmediateValue::Nil => "nil".to_string(),
                    ImmediateValue::Keyword(id) => {
                                        self.vm.symbol_table.read().get_symbol(id).unwrap_or("<unknown>").to_string()
                                    }
                    ImmediateValue::ModuleRef(module, symbol) => {
                        let symbol_table = self.vm.symbol_table.read();
                        let module_name = symbol_table.get_symbol(module).unwrap_or("<unknown>");
                        let symbol_name = symbol_table.get_symbol(symbol).unwrap_or("<unknown>");
                        format!("#<module {}/{}>", module_name, symbol_name)
                    },
                }
            }
            ValueRef::Heap(gc_ptr) => {
                self.format_heap_value(&gc_ptr.to_heap_value())
            }
            ValueRef::Native(native) => {
                format!("#<native fn {:?}>", native)
            }
        }
    }
    
    fn format_heap_value(&self, val: &HeapValue) -> String {
        match val {
            HeapValue::Str(s) => format!("\"{}\"", s),
            HeapValue::List(items) => {
                                        let formatted: Vec<String> = items.iter()
                                            .map(|item| self.format_value(*item))
                                            .collect();
                                        format!("({})", formatted.join(" "))
                                    }
            HeapValue::Vector(items) => {
                                        let formatted: Vec<String> = items.iter()
                                            .map(|item| self.format_value(*item))
                                            .collect();
                                        format!("[{}]", formatted.join(" "))
                                    }
            HeapValue::Map(map) => {
                                        let formatted: Vec<String> = map.iter()
                                            .map(|(k, v)| format!("{} {}", self.format_value(*k), self.format_value(*v)))
                                            .collect();
                                        format!("{{{}}}", formatted.join(", "))
                                    }
            HeapValue::Error(e) => format!("#<error: {}>", e),
            HeapValue::Function(f) => format!("#<fn {:?}>", f.params),
            HeapValue::Set(hash_set) => format!("#<set {:?}>", hash_set),
            HeapValue::Future(blink_future) => format!("#<future {:?}>", blink_future),
            HeapValue::Macro(mac) => format!("#<macro {:?}>", mac),
            HeapValue::Env(env) => format!("#<env {:?}>", env),
            HeapValue::Module(module) => {
                let symbol_table = self.vm.symbol_table.read();
                let module_name = symbol_table.get_symbol(module.name).unwrap_or("<unknown>");
                format!("#<module {}>", module_name)
            }
        }
    }


    pub fn get_vector_elements(&self, val: ValueRef) -> Result<Vec<ValueRef>, String> {
        match val {
            ValueRef::Heap(gc_ptr) => {
                match gc_ptr.to_heap_value() {
                        HeapValue::Vector(items) => Ok(items.clone()),
                        HeapValue::List(items) if !items.is_empty() => {
                            // Handle (vector elem1 elem2 ...) form
                            if let Some(head_name) = self.get_symbol_name(items[0]) {
                                if head_name == "vector" { // TODO this could be int comparison
                                    Ok(items[1..].to_vec())
                                } else {
                                    Err("let expects a vector of bindings".to_string())
                                }
                            } else {
                                Err("let expects a vector of bindings".to_string())
                            }
                        }
                        _ => Err("let expects a vector of bindings".to_string()),
                    }
                
            }
            _ => Err("let expects a vector of bindings".to_string()),
        }
    }

}
