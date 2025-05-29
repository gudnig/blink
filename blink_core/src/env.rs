use crate::module::ModuleRegistry;
use crate::value::BlinkValue;
use crate::Value;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Env {
    pub vars: HashMap<String, BlinkValue>,
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

    pub fn set(&mut self, key: &str, val: BlinkValue) {
        self.vars.insert(key.to_string(), val);
    }

    pub fn get_with_registry(&self, key: &str, registry: &ModuleRegistry) -> Option<BlinkValue> {

        // Check local variables FIRST
        if let Some(val) = self.vars.get(key) {
            match &val.read().value {
                Value::ModuleReference { module, symbol } => {
                    println!("🔗 Resolving module reference: {}::{}", module, symbol);
                    if let Some(mod_arc) = registry.get_module(module) {
                        return mod_arc.read().env.read().get_local(symbol);
                    }
                }
                _ => return Some(val.clone()),
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

    pub fn get_local(&self, key: &str) -> Option<BlinkValue> {
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
