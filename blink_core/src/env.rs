use crate::module::ModuleRegistry;
use crate::{ValueRef};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Env {
    pub vars: HashMap<u32, ValueRef>,
    pub parent: Option<Arc<RwLock<Env>>>,
    pub available_modules: HashMap<u32, u32>, // alias -> full_module_name
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

    pub fn set(&mut self, key: u32, val: ValueRef) {
        self.vars.insert(key, val);
    }

    pub fn get_with_registry(&self, key: u32, registry: &ModuleRegistry) -> Option<ValueRef> {
        //TODO module resolution
        if let Some(val) = self.vars.get(&key) {
            return Some(*val);
        }

        // Note: Qualified name checking removed since we're using u32 IDs now
        // Qualified symbols are handled differently - either:
        // 1. Pre-resolved during parsing via symbol table interning
        // 2. Or handled at eval time with separate qualified lookup

        // Check parent
        if let Some(parent) = &self.parent {
            return parent.read().get_with_registry(key, registry);
        }

        None
    }

    pub fn get_local(&self, key: u32) -> Option<ValueRef> {
        // Check local variables
        if let Some(val) = self.vars.get(&key) {
            return Some(*val);
        }

        // Check parent environment
        if let Some(parent) = &self.parent {
            return parent.read().get_local(key);
        }

        None
    }

    // Helper method for qualified lookups (module/symbol)
    pub fn get_qualified(&self, module_alias: u32, symbol: u32, registry: &ModuleRegistry) -> Option<ValueRef> {
        // Look up the actual module name from alias
        if let Some(&actual_module) = self.available_modules.get(&module_alias) {
            if let Some(module) = registry.get_module(actual_module) {
                return module.read().env.read().get_local(symbol);
            }
        }

        // Check parent environments
        if let Some(parent) = &self.parent {
            return parent.read().get_qualified(module_alias, symbol, registry);
        }

        None
    }
}