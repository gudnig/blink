use crate::module::ModuleRegistry;
use crate::value_ref::{SharedValue, ValueRef};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Env {
    pub vars: HashMap<String, ValueRef>,
    pub parent: Option<Arc<RwLock<Env>>>,
    pub available_modules: HashMap<String, String>, // alias -> full_module_name
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: HashMap::new(),
            parent: None,
            available_modules: HashMap::new(),
        }
    }

    pub fn with_parent(parent: Arc<RwLock<Env>>) -> Self {
        Env {
            vars: HashMap::new(),
            parent: Some(parent),
            available_modules: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, val: ValueRef) {
        self.vars.insert(key.to_string(), val);
    }

    pub fn get_with_registry(&self, key: &str, registry: &ModuleRegistry) -> Option<ValueRef> {
        // Check local variables FIRST
        if let Some(val) = self.vars.get(key) {
            match val {
                // Handle module references - resolve them
                ValueRef::Shared(idx) => {
                    // Need to check if this is a module reference in the shared arena
                    // For now, just return the value - module reference resolution
                    // would happen at a higher level
                    return Some(*val);
                }
                _ => return Some(*val),
            }
        }
        
        // Check for qualified name (module/symbol)
        if let Some((module_alias, symbol)) = key.split_once('/') {
            if let Some(module_name) = self.available_modules.get(module_alias) {
                if let Some(module) = registry.get_module(module_name) {
                    return module.read().env.read().get_local(symbol);
                }
            }
        }
        
        // Check parent
        if let Some(parent) = &self.parent {
            return parent.read().get_with_registry(key, registry);
        }
        
        None
    }
    pub fn get_local(&self, key: &str) -> Option<ValueRef> {
        // Check local variables
        if let Some(val) = self.vars.get(key) {
            return Some(val.clone());
        }
        
        // Check parent environment
        if let Some(parent) = &self.parent {
            return parent.read().get_local(key);
        }
        
        None
    }

}
