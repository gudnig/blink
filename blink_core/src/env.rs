use crate::module::ModuleRegistry;
use crate::runtime::SymbolTable;
use crate::value::{GcPtr, ValueRef};
use mmtk::util::{Address, ObjectReference};

// Stack refactor
// 1 remove env from module (auto import core module)
// 2 add stack frames to context and add locals to stack frames
// 3 env is made flat and only used for captured envs and closures
// 4 capture env creates env from stack frames
// 5 EvalResult::Suspended get's a copy of the stack to use for resuming
#[derive(Clone, Debug)]
pub struct Env {
    pub vars: Vec<(u32, ValueRef)>
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: Vec::new(),
        }
    }

    // FIXED: Maintain sorted order for binary search
    pub fn set(&mut self, key: u32, val: ValueRef) {
        match self.vars.binary_search_by_key(&key, |(k, _)| *k) {
            Ok(idx) => self.vars[idx].1 = val,  // Update existing
            Err(idx) => self.vars.insert(idx, (key, val)), // Insert at correct position
        }
    }
    
    pub fn get_var(&self, symbol: u32) -> Option<ValueRef> {
        
        for (i, (key, _)) in self.vars.iter().enumerate() {
            
        }
        
        let result = self.vars.binary_search_by_key(&symbol, |(k, _)| *k)
            .map(|idx| self.vars[idx].1)
            .ok();
        
        
        result
    }

}