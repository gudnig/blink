use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::Arc};
use libloading::Library;
use mmtk::util::ObjectReference;
use parking_lot::RwLock;
use crate::{env::Env, runtime::BlinkVM, ValueRef};

/// Module source specification
/// // For heap storage, serialize to a simpler enum
#[derive(Copy, Clone, Debug)]
pub enum SerializedModuleSource {
    Repl,
    BlinkFile(u32),
    NativeDylib(u32),       
    BlinkPackage(u32),   
    Cargo(u32),
    //Git { repo: u32, reference: Option<u32> }, Leave this for now 
    Url(u32),
    BlinkDll(u32),
    Wasm(u32),
}


#[derive(Clone, Debug)]
pub struct Module {
    pub name: u32,
    pub imports: HashMap<u32, (u32, u32)>, // alias -> (module_id, symbol_id)
    pub exports: HashMap<u32, ValueRef>,
    pub source: SerializedModuleSource, // Simplified for heap storage
    pub ready: bool,
}

/// Registry supporting all module types
#[derive(Debug)]
pub struct ModuleRegistry {
    /// All modules by ID
    pub modules: HashMap<u32, Module>,
    
    /// Files that have been evaluated (for Blink modules)
    evaluated_files: HashSet<u32>,
    
    /// File -> module IDs mapping (for multi-module .blink files)
    file_modules: HashMap<u32, Vec<u32>>,
    module_files: HashMap<u32, u32>,
    
    /// Native libraries that have been loaded
    loaded_libraries: HashMap<u32, libloading::Library>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        ModuleRegistry {
            modules: HashMap::new(),
            evaluated_files: HashSet::new(),
            file_modules: HashMap::new(),
            module_files: HashMap::new(),
            loaded_libraries: HashMap::new(),
        }
    }

    pub fn remove_module(&mut self, name: u32) -> bool {
        self.modules.remove(&name).is_some()
    }
    
    /// Remove a native library from storage
    pub fn remove_native_library(&mut self, path: u32) -> bool {
        self.loaded_libraries.remove(&path).is_some()
    }

    pub fn store_native_library(&mut self, path: u32, lib: Library) {
        self.loaded_libraries.insert(path.clone(), lib);
    }
    
    pub fn register_module(&mut self, module: Module) {
        let name = module.name; // Get name before moving module
        
        
        self.modules.insert(name, module);
        
    }

    pub fn get_module(&self, module_id: u32) -> Option<&Module> {
        self.modules.get(&module_id)
    }
 
    pub fn find_module_file(&self, module_name: u32) -> Option<u32> {
        self.module_files.get(&module_name).copied()
    }
    
    
    pub fn mark_file_evaluated(&mut self, path: u32) {
        
        self.evaluated_files.insert(path);
    }
    
    pub fn is_file_evaluated(&self, path: u32) -> bool {
        self.evaluated_files.contains(&path)
    }
    
    /// Get all modules defined in a file
    pub fn modules_in_file(&self, file: u32) -> Vec<u32> {
        self.file_modules.get(&file).cloned().unwrap_or_default()
    }

    pub fn update_module(&mut self, module_id: u32, symbol_id: u32, value: ValueRef) {
        let module = self.modules.get_mut(&module_id).unwrap();
        module.exports.insert(symbol_id, value);
    }
    

    pub fn resolve_symbol(&self, module_id: u32, symbol_id: u32) -> Option<ValueRef> {
        let module = self.modules.get(&module_id)?;
        match module.exports.get(&symbol_id) {
            Some(val) => Some(*val),
            None => {
                let module = self.modules.get(&module_id)?;
                module.imports.get(&symbol_id).and_then(|(module_id, symbol_id)| self.resolve_symbol(*module_id, *symbol_id))
            }
        }
    }
}


