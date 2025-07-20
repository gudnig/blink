use crate::module::ModuleRegistry;
use crate::runtime::SymbolTable;
use crate::value::{GcPtr, ValueRef};
use mmtk::util::ObjectReference;

#[derive(Clone, Debug)]
pub struct Env {
    pub vars: Vec<(u32, ValueRef)>,
    pub parent: Option<ObjectReference>,
    pub symbol_aliases: Vec<(u32, (u32, u32))>,
    pub module_aliases: Vec<(u32, u32)>,           // alias -> module_id
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: Vec::new(),
            parent: None,
            symbol_aliases: Vec::new(),
            module_aliases: Vec::new(),
        }
    }

    pub fn with_parent(parent: ObjectReference) -> Self {
        Env {
            vars: Vec::new(),
            parent: Some(parent),
            symbol_aliases: Vec::new(),
            module_aliases: Vec::new(),
        }
    }

    // FIXED: Maintain sorted order for binary search
    pub fn set(&mut self, key: u32, val: ValueRef) {
        match self.vars.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(idx) => self.vars[idx].1 = val,  // Update existing
            Err(idx) => self.vars.insert(idx, (key, val)), // Insert at correct position
        }
    }

    pub fn resolve_symbol(&self, symbol_id: u32, symbol_table: &SymbolTable, module_registry: &ModuleRegistry) -> Option<ValueRef> {
        if symbol_table.is_qualified(symbol_id) {
            let (module_id, symbol_id) = symbol_table.get_qualified(symbol_id)?;
            self.resolve_qualified_symbol(module_id, symbol_id, module_registry) 
        } else {
            self.resolve_simple_symbol(symbol_id, module_registry)
        }
    }

    pub fn resolve_simple_symbol(&self, symbol_id: u32, module_registry: &ModuleRegistry) -> Option<ValueRef> {
        // 1. Local vars
        if let Some(val) = self.get_var(symbol_id) {
            return Some(val);
        }
        
        // 2. Imported symbol aliases (two-stage)
        if let Some((mod_id, sym)) = self.resolve_symbol_alias(symbol_id) {
            return self.resolve_qualified_symbol(mod_id, sym, module_registry);
        }
        
        // 3. Move to parent and try again
        if let Some(parent_ref) = self.parent {
            let parent_env = GcPtr::new(parent_ref).read_env();
            return parent_env.resolve_simple_symbol(symbol_id, module_registry);
        }
        
        None
    }

    pub fn resolve_qualified_symbol(&self, module_part: u32, symbol: u32, module_registry: &ModuleRegistry) -> Option<ValueRef> {
        
        // Try alias first
        if let Some(actual_module_id) = self.resolve_module_alias(module_part) {
            return self.resolve_module_symbol(actual_module_id, symbol, module_registry);
        }
        
        // Fall back to direct module name lookup
        if module_registry.get_module(module_part).is_some() {
            return self.resolve_module_symbol(module_part, symbol, module_registry);
        }
        
        None
    }

    pub fn resolve_symbol_alias(&self, symbol_id: u32) -> Option<(u32, u32)> {
        self.symbol_aliases.binary_search_by_key(&symbol_id, |(k, _)| *k)
            .map(|idx| self.symbol_aliases[idx].1)
            .ok()
    }

    pub fn resolve_module_alias(&self, alias: u32) -> Option<u32> {
        self.module_aliases.binary_search_by_key(&alias, |(k, _)| *k)
            .map(|idx| self.module_aliases[idx].1)
            .ok()
    }

    pub fn resolve_module_symbol(&self, module_id: u32, symbol_id: u32, module_registry: &ModuleRegistry) -> Option<ValueRef> {
        let module_ref = module_registry.get_module(module_id)?;
        
        let module = GcPtr::new(module_ref).read_module();
        let module_env = GcPtr::new(module.env).read_env();
        module_env.get_var(symbol_id)
    }

    // Helper methods for binary search access
    
    pub fn get_var(&self, symbol: u32) -> Option<ValueRef> {
        
        for (i, (key, _)) in self.vars.iter().enumerate() {
            
        }
        
        let result = self.vars.binary_search_by_key(&symbol, |(k, _)| *k)
            .map(|idx| self.vars[idx].1)
            .ok();
        
        
        result
    }

    pub fn add_module_import(&mut self, alias: u32, module_id: u32, symbol_id: u32) {
        let target = (module_id, symbol_id);
        match self.symbol_aliases.binary_search_by_key(&alias, |(k, _)| *k) {
            Ok(idx) => self.symbol_aliases[idx].1 = target,
            Err(idx) => self.symbol_aliases.insert(idx, (alias, target)),
        }
    }
    
    // Get the full target: (module, symbol)
    pub fn get_module_import(&self, alias: u32) -> Option<(u32, u32)> {
        self.symbol_aliases.binary_search_by_key(&alias, |(k, _)| *k)
            .map(|idx| self.symbol_aliases[idx].1)
            .ok()
    }

    pub fn add_module_alias(&mut self, alias: u32, module_id: u32) {
        match self.module_aliases.binary_search_by_key(&alias, |(k, _)| *k) {
            Ok(idx) => self.module_aliases[idx].1 = module_id,
            Err(idx) => self.module_aliases.insert(idx, (alias, module_id)),
        }
    }

    pub fn get_module_alias(&self, alias: u32) -> Option<u32> {
        self.module_aliases.binary_search_by_key(&alias, |(k, _)| *k)
            .map(|idx| self.module_aliases[idx].1)
            .ok()
    }
}