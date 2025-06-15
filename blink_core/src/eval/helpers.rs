use std::collections::HashMap;

use crate::{error::BlinkError, eval::context::EvalContext, value_ref::{pack_bool, unpack_immediate, ImmediateValue, ModuleRef, SharedValue, ValueRef}};

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

    pub fn get_bool(&self, val: ValueRef) -> Option<bool> {
        if let ValueRef::Shared(idx) = val {
            if let Some(shared) = self.shared_arena.read().get(idx) {
                if let SharedValue::Bool(b) = shared.as_ref() {
                    return Some(*b);
                }
            }
        }
        None
    }

    pub fn with_map<T>(&self, val: ValueRef, f: impl FnOnce(&HashMap<ValueRef, ValueRef>) -> T) -> Option<T> {
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

    pub fn module_value(&mut self, module_name: &str, symbol_name: &str) -> ValueRef {
        let val = SharedValue::Module(ModuleRef {
            module: module_name.to_string(),
            symbol: symbol_name.to_string(),
        });
        self.shared_arena.write().alloc(val)
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

    // Formatting ----------------------------
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
            ValueRef::Nil => "nil".to_string(),
            ValueRef::Gc(_) => "<gc-object>".to_string(),
        }
    }
    
    fn format_shared_value(&self, val: &SharedValue) -> String {
        match val {
            SharedValue::Str(s) => format!("\"{}\"", s),
            SharedValue::Symbol(s) => s.clone(),
            SharedValue::Keyword(k) => format!(":{}", k),
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
            SharedValue::Number(_) => "#<number>".to_string(),
            SharedValue::Bool(_) => "#<bool>".to_string(),
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
