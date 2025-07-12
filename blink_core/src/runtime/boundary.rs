use std::collections::{HashMap, HashSet};

use crate::{
    error::BlinkError,
    eval::EvalContext,
    value::{
        pack_bool, pack_nil, pack_number, unpack_immediate, ImmediateValue, IsolatedValue, ValueRef,
    },
    value::HeapValue,
};

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
                        let vm = self.context.vm.clone();
                        let symbol_name = vm
                            .symbol_table
                            .read()
                            .get_symbol(s)
                            .map(|s| s.to_string())
                            .ok_or_else(|| format!("Symbol not found: {}", s))?;
                        Ok(IsolatedValue::Symbol(symbol_name))
                    }
                    ImmediateValue::Keyword(k) => {
                        let keyword_name = self
                            .context
                            .get_keyword_name(ValueRef::Immediate(k as u64))
                            .ok_or_else(|| format!("Keyword not found: {}", k))?;
                        Ok(IsolatedValue::Keyword(keyword_name))
                    }
                    _ => Err(format!(
                        "Unsupported immediate value: {}",
                        unpacked.type_tag()
                    )),
                }
            }

            ValueRef::Heap(gc_ptr) => {
                let heap_val = value.read_heap_value();
                if let Some(heap_val) = heap_val {
                    match heap_val {
                        HeapValue::List(value_refs) => {
                                                let isolated_values: Vec<IsolatedValue> = value_refs
                                                    .into_iter()
                                                    .map(|item| self.extract_isolated(item))
                                                    .collect::<Result<Vec<IsolatedValue>, String>>()?;
                                                Ok(IsolatedValue::List(isolated_values))
                                            }
                        HeapValue::Vector(value_refs) => {
                                                let isolated_values: Vec<IsolatedValue> = value_refs
                                                    .into_iter()
                                                    .map(|item| self.extract_isolated(item))
                                                    .collect::<Result<Vec<IsolatedValue>, String>>()?;
                                                Ok(IsolatedValue::Vector(isolated_values))
                                            }
                        HeapValue::Map(blink_hash_map) => {
                                                let isolated_values = blink_hash_map
                                                    .into_iter()
                                                    .map(|(k, v)| {
                                                        let k = self.extract_isolated(k)?;
                                                        let v = self.extract_isolated(v)?;
                                                        Ok((k, v))
                                                    })
                                                    .collect::<Result<Vec<(IsolatedValue, IsolatedValue)>, String>>()?;

                                                let map = HashMap::from_iter(isolated_values);
                                                Ok(IsolatedValue::Map(map))
                                            }
                        HeapValue::Str(s) => Ok(IsolatedValue::String(s)),
                        HeapValue::Set(blink_hash_set) => {
                                                let isolated_values: Vec<IsolatedValue> = blink_hash_set
                                                    .into_iter()
                                                    .map(|item| self.extract_isolated(item))
                                                    .collect::<Result<Vec<IsolatedValue>, String>>()?;
                                                Ok(IsolatedValue::Set(HashSet::from_iter(isolated_values)))
                                            }
                        HeapValue::Error(blink_error) => {
                                                let error = BlinkError::eval(blink_error.to_string());
                                                Ok(IsolatedValue::Error(error.to_string()))
                                            }
                        HeapValue::Function(callable) => {
                                                let handle = self.context.register_function(value);
                                                Ok(IsolatedValue::Function(handle))
                                            }
                        HeapValue::Macro(callable) => {
                                                let handle = self.context.register_function(value);
                                                Ok(IsolatedValue::Macro(handle))
                                            }
                        HeapValue::Future(blink_future) => {
                                                let handle = self.context.register_future(value);
                                                Ok(IsolatedValue::Future(handle))
                                            }
                        HeapValue::Env(env) => {
                                                Err(format!("Env is not supported for boundary crossing"))
                                            }
                        HeapValue::Module(module) => {
                            Err(format!("Module is not supported for boundary crossing"))
                        }
                    }
                } else {
                    Err(format!("Unsupported value type for boundary crossing"))
                }
            }
            _ => Err(format!("Unsupported value type for boundary crossing")),
        }
    }

    fn alloc_from_isolated(&mut self, value: IsolatedValue) -> ValueRef {
        match value {
            IsolatedValue::Number(n) => ValueRef::Immediate(pack_number(n)),
            IsolatedValue::Bool(b) => ValueRef::Immediate(pack_bool(b)),
            IsolatedValue::Symbol(s) => self.context.intern_symbol(&s),
            IsolatedValue::Keyword(k) => self.context.intern_keyword(&k),
            IsolatedValue::String(s) => self.context.string_value(&s),
            IsolatedValue::List(items) => {
                let value_refs: Vec<ValueRef> = items
                    .into_iter()
                    .map(|item| self.alloc_from_isolated(item))
                    .collect();
                self.context.list_value(value_refs)
            }
            IsolatedValue::Vector(items) => {
                let value_refs: Vec<ValueRef> = items
                    .into_iter()
                    .map(|item| self.alloc_from_isolated(item))
                    .collect();
                self.context.vector_value(value_refs)
            }
            IsolatedValue::Map(map) => {
                let pairs: Vec<(ValueRef, ValueRef)> = map
                    .into_iter()
                    .map(|(k, v)| (self.alloc_from_isolated(k), self.alloc_from_isolated(v)))
                    .collect();

                self.context.map_value(pairs)
            }
            IsolatedValue::Set(set) => {
                let value_refs: Vec<ValueRef> = set
                    .into_iter()
                    .map(|item| self.alloc_from_isolated(item))
                    .collect();
                self.context.set_value(value_refs)
            }
            IsolatedValue::Function(handle) => self
                .context
                .resolve_function(handle)
                .unwrap_or(ValueRef::Immediate(pack_nil())),
            IsolatedValue::Macro(handle) => self
                .context
                .resolve_function(handle)
                .unwrap_or(ValueRef::Immediate(pack_nil())),
            IsolatedValue::Future(handle) => self
                .context
                .resolve_future(handle)
                .unwrap_or(ValueRef::Immediate(pack_nil())),
            IsolatedValue::Error(msg) => {
                let error = BlinkError::eval(msg);
                self.context.error_value(error)
            }
            IsolatedValue::Nil => ValueRef::Immediate(pack_nil()),
        }
    }
}
