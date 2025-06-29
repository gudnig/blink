use std::collections::{HashMap, HashSet};

use crate::{collections::ValueContext, error::BlinkError, eval::EvalContext, value::{  pack_bool, pack_nil, pack_number, unpack_immediate, ImmediateValue, IsolatedValue}, collections::{BlinkHashMap, BlinkHashSet}, value::{SharedValue, ValueRef}};





pub trait ValueBoundary {
    fn extract_isolated(&self, value: ValueRef) -> Result<IsolatedValue, String>;
    fn alloc_from_isolated(&mut self, value: IsolatedValue) -> ValueRef;
    
    // Convenience methods for primitives
    fn extract_string(&self, value: ValueRef) -> Result<String, String> {
        match self.extract_isolated(value)? {
            IsolatedValue::String(s) => Ok(s),
            other => Err(format!("Expected string, got {}", other.type_name())),
        }
    }
    
    fn extract_number(&self, value: ValueRef) -> Result<f64, String> {
        match self.extract_isolated(value)? {
            IsolatedValue::Number(n) => Ok(n),
            other => Err(format!("Expected number, got {}", other.type_name())),
        }
    }
    
    fn extract_bool(&self, value: ValueRef) -> Result<bool, String> {
        match self.extract_isolated(value)? {
            IsolatedValue::Bool(b) => Ok(b),
            other => Err(format!("Expected bool, got {}", other.type_name())),
        }
    }
    fn extract_nil(&self, value: ValueRef) -> Result<(), String> {
        match self.extract_isolated(value)? {
            IsolatedValue::Nil => Ok(()),
            other => Err(format!("Expected nil, got {}", other.type_name())),
        }
    }
    
}



// Current implementation
pub struct ContextualBoundary<'a> {
    pub context: &'a mut EvalContext,
}

impl<'a> ContextualBoundary<'a> {
    pub fn new(context: &'a mut EvalContext) -> Self {
        Self { context }
    }
}

impl<'a> ValueBoundary for ContextualBoundary<'a> {
    fn extract_isolated(&self, value: ValueRef) -> Result<IsolatedValue, String> {
        match value {
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(packed);
                match unpacked {
                    ImmediateValue::Number(n) => Ok(IsolatedValue::Number(n)),
                    ImmediateValue::Bool(b) => Ok(IsolatedValue::Bool(b)),
                    ImmediateValue::Nil => Ok(IsolatedValue::Nil),
                    ImmediateValue::Symbol(s) => {
                        let symbol_name = self.context.symbol_table.read().get_symbol(s)
                            .map(|s| s.to_string())
                            .ok_or_else(|| format!("Symbol not found: {}", s))?;
                        Ok(IsolatedValue::Symbol(symbol_name))
                    },
                    ImmediateValue::Keyword(k) => {
                        let keyword_name = self.context.get_keyword_name(ValueRef::Immediate(k as u64))
                            .ok_or_else(|| format!("Keyword not found: {}", k))?;
                        Ok(IsolatedValue::Keyword(keyword_name))
                    },
                    _ => Err(format!("Unsupported immediate value: {}", unpacked.type_tag())),
                }
            },
            
            ValueRef::Shared(shared_ref) => {
                let shared_value = self.context.shared_arena.read().get(shared_ref)
                    .ok_or("Shared value not found")?
                    .clone();
                
                match shared_value.as_ref() {
                    SharedValue::Str(s) => Ok(IsolatedValue::String(s.clone())),
                    
                    SharedValue::List(items) => {
                        let isolated_items: Result<Vec<_>, _> = items.iter()
                            .map(|item| self.extract_isolated(*item))
                            .collect();
                        Ok(IsolatedValue::List(isolated_items?))
                    },
                    
                    SharedValue::Vector(items) => {
                        let isolated_items: Result<Vec<_>, _> = items.iter()
                            .map(|item| self.extract_isolated(*item))
                            .collect();
                        Ok(IsolatedValue::Vector(isolated_items?))
                    },
                    
                    SharedValue::Map(map) => {
                        let mut isolated_map = HashMap::new();
                        for (k, v) in map.iter() {
                            let isolated_key = self.extract_isolated(*k)?;
                            let isolated_value = self.extract_isolated(*v)?;
                            isolated_map.insert(isolated_key, isolated_value);
                        }
                        Ok(IsolatedValue::Map(isolated_map))
                    },
                    
                    SharedValue::Set(set) => {
                        let mut isolated_set = HashSet::new();
                        for item in set.iter() {
                            isolated_set.insert(self.extract_isolated(*item)?);
                        }
                        Ok(IsolatedValue::Set(isolated_set))
                    },
                    
                    SharedValue::Error(error) => {
                        Ok(IsolatedValue::Error(error.message.clone()))
                    },
                    
                    // These become handles:
                    SharedValue::NativeFunction(_) => {
                        let handle = self.context.handle_registry.write().register_function(value);
                        Ok(IsolatedValue::Function(handle))
                    },
                    
                    SharedValue::UserDefinedFunction(_) => {
                        let handle = self.context.handle_registry.write().register_function(value);
                        Ok(IsolatedValue::Function(handle))
                    },
                    
                    SharedValue::Macro(_) => {
                        let handle = self.context.handle_registry.write().register_function(value);
                        Ok(IsolatedValue::Macro(handle))
                    },
                    
                    SharedValue::Future(_) => {
                        let handle = self.context.handle_registry.write().register_future(value);
                        Ok(IsolatedValue::Future(handle))
                    },
                    
                    SharedValue::Module(_) => {
                        // Modules don't cross boundary
                        Ok(IsolatedValue::Nil)
                    },
                }
            },
            _ => Err(format!("Unsupported value type for boundary crossing")),
        }
    }
    
    fn alloc_from_isolated(&mut self, value: IsolatedValue) -> ValueRef {
        match value {
            IsolatedValue::Number(n) => {
                        ValueRef::Immediate(pack_number(n))
                    },
            IsolatedValue::Bool(b) => {
                        ValueRef::Immediate(pack_bool(b))
                    },
            IsolatedValue::Symbol(s) => {
                        self.context.intern_symbol(&s)
                    },
            IsolatedValue::Keyword(k) => {
                        self.context.intern_keyword(&k)
                    },
            IsolatedValue::String(s) => {
                        let shared_value =  SharedValue::Str(s);
                        let shared_ref = self.context.shared_arena.write().alloc(shared_value);
                        shared_ref
                    },
            IsolatedValue::List(items) => {
                        let value_refs: Vec<ValueRef> = items.into_iter()
                            .map(|item| self.alloc_from_isolated(item))
                            .collect();
                        let shared_value = SharedValue::List(value_refs);
                        let shared_ref = self.context.shared_arena.write().alloc(shared_value);
                        shared_ref
                    },
            IsolatedValue::Vector(items) => {
                        let value_refs: Vec<ValueRef> = items.into_iter()
                            .map(|item| self.alloc_from_isolated(item))
                            .collect();
                        let shared_value = SharedValue::Vector(value_refs);
                        let shared_ref = self.context.shared_arena.write().alloc(shared_value);
                        shared_ref
                    },
            IsolatedValue::Map(map) => {

                
                        let mut value_map = BlinkHashMap::new(ValueContext::new(self.context.shared_arena.clone()));
                
                        for (k, v) in map {
                            let key_ref = self.alloc_from_isolated(k);
                            let val_ref = self.alloc_from_isolated(v);
                            value_map.insert(key_ref, val_ref);
                        }
                        let shared_value = SharedValue::Map(value_map);
                        let shared_ref = self.context.shared_arena.write().alloc(shared_value);
                        shared_ref
                    },
            IsolatedValue::Set(set) => {
                        let mut value_set = BlinkHashSet::new(ValueContext::new(self.context.shared_arena.clone()));
                        for item in set {
                            value_set.insert(self.alloc_from_isolated(item));
                        }
                        let shared_value = SharedValue::Set(value_set);
                        let shared_ref = self.context.shared_arena.write().alloc(shared_value);
                        shared_ref
                    },
            IsolatedValue::Function(handle) => {
                        self.context.handle_registry.read().resolve_function(&handle)
                            .unwrap_or(ValueRef::Immediate(pack_nil()))
                    },
            IsolatedValue::Macro(handle) => {
                        self.context.handle_registry.read().resolve_function(&handle)
                            .unwrap_or(ValueRef::Immediate(pack_nil()))
                    },
            IsolatedValue::Future(handle) => {
                        self.context.handle_registry.read().resolve_future(&handle)
                            .unwrap_or(ValueRef::Immediate(pack_nil()))
                    },
            IsolatedValue::Error(msg) => {
                        let error = BlinkError::eval(msg);
                        let shared_value = SharedValue::Error(error);
                        let shared_ref = self.context.shared_arena.write().alloc(shared_value);
                        shared_ref
                    },
            IsolatedValue::Nil => {
                ValueRef::Immediate(pack_nil())
            }
        }
    }
}