use crate::module::ModuleRegistry;
use crate::value::{pack_module, GcPtr, ValueRef};
use mmtk::util::ObjectReference;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Env {
    pub vars: Vec<(u32, ValueRef)>,
    pub parent: Option<ObjectReference>,
    pub available_modules: Vec<(u32, u32)>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: Vec::new(),
            parent: None,
            available_modules: Vec::new(),
        }
    }

    pub fn with_parent(parent: ObjectReference) -> Self {
        Env {
            vars: Vec::new(),
            parent: Some(parent),
            available_modules: Vec::new(),
        }
    }

    // FIXED: Maintain sorted order for binary search
    pub fn set(&mut self, key: u32, val: ValueRef) {
        match self.vars.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(idx) => self.vars[idx].1 = val,  // Update existing
            Err(idx) => self.vars.insert(idx, (key, val)), // Insert at correct position
        }
    }

    // FIXED: Use binary search and proper ObjectReference handling
    pub fn get_with_registry(&self, key: u32, registry: &ModuleRegistry) -> Option<ValueRef> {
        // Check local vars using binary search
        if let Some(val) = self.get_var(key) {
            return Some(val);
        }

        // Check if key is a module alias
        if let Some(module_id) = self.get_module_alias(key) {

            if let Some(module_obj_ref) = registry.get_module(module_id) {
                let module = GcPtr::new(module_obj_ref);
                let env_ref = module.read_module().env;
                let env = GcPtr::new(env_ref).read_env();
                return env.get_with_registry(key, registry);
            }
        }

        // Check parent
        if let Some(parent_ref) = self.parent {
            let parent_env = GcPtr::new(parent_ref).read_env();
            return parent_env.get_with_registry(key, registry);
        }

        None
    }

    // FIXED: Use binary search and proper ObjectReference handling
    pub fn get_local(&self, key: u32) -> Option<ValueRef> {
        // Check local variables using binary search
        if let Some(val) = self.get_var(key) {
            return Some(val);
        }

        // Check parent environment
        if let Some(parent_ref) = self.parent {
            let parent_env = GcPtr::new(parent_ref).read_env();
            return parent_env.get_local(key);
        }

        None
    }

    pub fn get_qualified(&self, module_alias: u32, symbol: u32, registry: &ModuleRegistry) -> Option<ValueRef> {
        // Look up the actual module name from alias using binary search
        if let Some(actual_module) = self.get_module_alias(module_alias) {
            if let Some(module_ref) = registry.get_module(actual_module) {
                // Get the module's environment and look up the symbol
                let module = GcPtr::new(module_ref).read_module();
                let module_env = GcPtr::new(module.env).read_env();
                return module_env.get_local(symbol);
            }
        }

        // Check parent environments
        if let Some(parent_ref) = self.parent {
            let parent_env = GcPtr::new(parent_ref).read_env();
            return parent_env.get_qualified(module_alias, symbol, registry);
        }

        None
    }

    // Helper methods for binary search access
    pub fn get_var(&self, symbol: u32) -> Option<ValueRef> {
        self.vars.binary_search_by_key(&symbol, |(k, _)| *k)
            .map(|idx| self.vars[idx].1)
            .ok()
    }

    pub fn get_module_alias(&self, alias: u32) -> Option<u32> {
        self.available_modules.binary_search_by_key(&alias, |(k, _)| *k)
            .map(|idx| self.available_modules[idx].1)
            .ok()
    }

    // Helper to add module alias while maintaining sorted order
    pub fn add_module_alias(&mut self, alias: u32, module_id: u32) {
        match self.available_modules.binary_search_by_key(&alias, |(k, _)| *k) {
            Ok(idx) => self.available_modules[idx].1 = module_id,  // Update existing
            Err(idx) => self.available_modules.insert(idx, (alias, module_id)), // Insert at correct position
        }
    }
}