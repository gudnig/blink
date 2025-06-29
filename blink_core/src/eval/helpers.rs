use std::sync::Arc;

use parking_lot::RwLock;

use crate::{collections::{BlinkHashMap, BlinkHashSet, ValueContext}, error::BlinkError, future::BlinkFuture, runtime::EvalContext, value::{pack_bool, unpack_immediate, ImmediateValue, Macro, ModuleRef, NativeFn, SharedValue, UserDefinedFn, ValueRef}};
use crate::env::Env;

impl EvalContext {

    // Getters
    pub fn get_err(&self, value: &ValueRef) -> BlinkError {
        let arena = self.shared_arena.read();
        match value {
            ValueRef::Shared(idx) => {
                if let Some(shared) = arena.get(*idx) {
                    if let SharedValue::Error(e) = shared.as_ref() {
                        return e.clone();
                    }
                    else {
                        BlinkError::eval("Expected error")
                    }
                } else {
                    BlinkError::eval("Expected error")
                }
            },
            _ => {
                BlinkError::eval("Expected error")
            }
        }
    }
    
    pub fn get_string(&self, val: ValueRef) -> Option<String> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::Str(s) = shared.as_ref() {
                    return Some(s.clone());
                }
            }
        }
        None
    }

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

    pub fn get_future(&self, val: ValueRef) -> Option<BlinkFuture> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::Future(future) = shared.as_ref() {
                    return Some(future.clone());
                }
            }
        }
        None
    }



    
    pub fn get_vec(&self, val: ValueRef) -> Option<Vec<ValueRef>> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::Vector(items) = shared.as_ref() {
                    return Some(items.clone());
                }
            }
        }
        None
    }

    pub fn get_map(&self, val: ValueRef) -> Option<BlinkHashMap> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::Map(map) = shared.as_ref() {
                    return Some(map.clone());
                }
            }
        }
        None
    }

    pub fn get_set(&self, val: ValueRef) -> Option<BlinkHashSet> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::Set(set) = shared.as_ref() {
                    return Some(set.clone());
                }
            }
        }
        None
    }

    pub fn get_list(&self, val: ValueRef) -> Option<Vec<ValueRef>> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::List(items) = shared.as_ref() {
                    return Some(items.clone());
                }
            }
        }
        None
    }

    pub fn get_vec_or_list_items(&self, val: ValueRef) -> Option<Vec<ValueRef>> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                match shared.as_ref() {
                    SharedValue::List(items) => Some(items.clone()),
                    SharedValue::Vector(items) => Some(items.clone()), // Accept vectors too
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    }
    

    pub fn with_map<T>(&self, val: ValueRef, f: impl FnOnce(&BlinkHashMap) -> T) -> Option<T> {
        if let ValueRef::Shared(idx) = val {
            let arena = self.shared_arena.read();
            if let Some(shared) = arena.get(idx) {
                if let SharedValue::Map(map) = shared.as_ref() {
                    return Some(f(map));
                }
            }
        }
        None
    }
    
    

    // Checking
    pub fn is_err(&self, value: &ValueRef) -> bool {
        let arena = self.shared_arena.read();
        value.is_error(&arena)
    }

    pub fn is_nil(&self, value: &ValueRef) -> bool {
        match value {
            ValueRef::Immediate(val) => {
                let unpacked = unpack_immediate(*val);
                if let ImmediateValue::Nil = unpacked {
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }


    // Value creation
    pub fn bool_value(&mut self, value: bool) -> ValueRef {
        ValueRef::boolean(value)
    }

    pub fn number_value(&mut self, value: f64) -> ValueRef {
        ValueRef::number(value)
    }
    pub fn symbol_value(&mut self, value: &str) -> ValueRef {
        let symbol = self.symbol_table.write().intern(value);
        ValueRef::symbol(symbol)
    }

    pub fn nil_value(&mut self) -> ValueRef {
        ValueRef::nil()
    }

    pub fn string_value(&mut self, value: &str) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Str(value.to_string()))
    }

    pub fn list_value(&mut self, values: Vec<ValueRef>) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::List(values))
    }

    pub fn vector_value(&mut self, values: Vec<ValueRef>) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Vector(values))
    }

    pub fn map_value(&mut self, pairs: Vec<(ValueRef, ValueRef)>) -> ValueRef {
        let map = BlinkHashMap::from_pairs(pairs, ValueContext::new(self.shared_arena.clone()));
        self.shared_arena.write().alloc(SharedValue::Map(map))
    }   

    pub fn set_value(&mut self, set: Vec<ValueRef>) -> ValueRef {
        let set = BlinkHashSet::from_iter(set, ValueContext::new(self.shared_arena.clone()));
        self.shared_arena.write().alloc(SharedValue::Set(set))
    }

    pub fn future_value(&mut self, future: BlinkFuture) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Future(future))
    }

    pub fn native_function_value(&mut self, func: NativeFn) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::NativeFunction(func))
    }

    pub fn user_defined_function_value(&mut self, params: Vec<u32>, body: Vec<ValueRef>, env: Arc<RwLock<Env>>) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::UserDefinedFunction(UserDefinedFn { params, body, env }))
    }

    pub fn macro_value(&mut self, mac: Macro) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Macro(mac))
    }

    pub fn module_value(&mut self, module_name: u32, symbol_name: u32) -> ValueRef {
        let val = SharedValue::Module(ModuleRef {
            module: module_name,
            symbol: symbol_name,
        });
        self.shared_arena.write().alloc(val)
    }

    pub fn empty_map_value(&mut self) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Map(BlinkHashMap::new(ValueContext::new(self.shared_arena.clone()))))
    }

    pub fn empty_set_value(&mut self) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Set(BlinkHashSet::new(ValueContext::new(self.shared_arena.clone()))))
    }

    pub fn empty_list_value(&mut self) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::List(Vec::new()))
    }

    pub fn empty_vector_value(&mut self) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Vector(Vec::new()))
    }
    pub fn error_value(&mut self, error: BlinkError) -> ValueRef {
        self.shared_arena.write().alloc(SharedValue::Error(error))
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
        
        args[index].try_get_number()
            .ok_or_else(|| BlinkError::eval(format!("{} expects number at position {}", fn_name, index)))
    }
    
    /// Extract string from specific argument position
    pub fn require_string(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<String, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        self.get_string(args[index])    
            .ok_or_else(|| BlinkError::eval(format!("{} expects string at position {}", fn_name, index)))
    }
    
    /// Extract boolean from specific argument position
    pub fn require_bool(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<bool, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        args[index].try_get_bool()
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
        
        self.get_vec_or_list_items(args[index])
            .ok_or_else(|| BlinkError::eval(format!("{} expects list/vector at position {}", fn_name, index)))
    }

    pub fn require_list(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<Vec<ValueRef>, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        self.get_list(args[index])
            .ok_or_else(|| BlinkError::eval(format!("{} expects list at position {}", fn_name, index)))
    }



    pub fn require_vec(&self, args: &[ValueRef], index: usize, fn_name: &str) -> Result<Vec<ValueRef>, BlinkError> {
        if index >= args.len() {
            return Err(BlinkError::arity(index + 1, args.len(), fn_name));
        }
        
        self.get_vec(args[index])
            .ok_or_else(|| BlinkError::eval(format!("{} expects vector at position {}", fn_name, index)))
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
                                        self.symbol_table.read().get_symbol(id).unwrap_or("<unknown>").to_string()
                                    }
                    ImmediateValue::Nil => "nil".to_string(),
                    ImmediateValue::Keyword(id) => {
                        self.symbol_table.read().get_symbol(id).unwrap_or("<unknown>").to_string()
                    }
                }
            }
            ValueRef::Shared(idx) => {
                if let Some(shared) = self.shared_arena.read().get(idx) {
                    self.format_shared_value(shared)
                } else {
                    "<invalid-ref>".to_string()
                }
            }            
            ValueRef::Gc(_) => "<gc-object>".to_string(),
        }
    }
    
    fn format_shared_value(&self, val: &SharedValue) -> String {
        match val {
            SharedValue::Str(s) => format!("\"{}\"", s),
            SharedValue::List(items) => {
                        let formatted: Vec<String> = items.iter()
                            .map(|item| self.format_value(*item))
                            .collect();
                        format!("({})", formatted.join(" "))
                    }
            SharedValue::Vector(items) => {
                        let formatted: Vec<String> = items.iter()
                            .map(|item| self.format_value(*item))
                            .collect();
                        format!("[{}]", formatted.join(" "))
                    }
            SharedValue::Map(map) => {
                        let formatted: Vec<String> = map.iter()
                            .map(|(k, v)| format!("{} {}", self.format_value(*k), self.format_value(*v)))
                            .collect();
                        format!("{{{}}}", formatted.join(", "))
                    }
            SharedValue::Error(e) => format!("#<error: {}>", e),
            SharedValue::NativeFunction(_) => "#<native-fn>".to_string(),
            SharedValue::UserDefinedFunction(f) => format!("#<fn {:?}>", f.params),
            SharedValue::Set(hash_set) => format!("#<set {:?}>", hash_set),
            SharedValue::Future(blink_future) => format!("#<future {:?}>", blink_future),
            SharedValue::Module(module_ref) => format!("#<module {:?}>", module_ref),
            SharedValue::Macro(_) => "#<macro>".to_string(),
        }
    }


    pub fn get_vector_elements(&self, val: ValueRef) -> Result<Vec<ValueRef>, String> {
        match val {
            ValueRef::Shared(idx) => {
                if let Some(shared) = self.shared_arena.read().get(idx) {
                    match shared.as_ref() {
                        SharedValue::Vector(items) => Ok(items.clone()),
                        SharedValue::List(items) if !items.is_empty() => {
                            // Handle (vector elem1 elem2 ...) form
                            if let Some(head_name) = self.get_symbol_name(items[0]) {
                                if head_name == "vector" {
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
                } else {
                    Err("Invalid reference".to_string())
                }
            }
            _ => Err("let expects a vector of bindings".to_string()),
        }
    }

}
