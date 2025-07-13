use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::Arc};
use libloading::Library;
use mmtk::util::ObjectReference;
use parking_lot::RwLock;
use crate::{env::Env, runtime::BlinkVM};

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
    pub env: ObjectReference,
    pub exports: Vec<u32>,              // Sorted
    pub source: SerializedModuleSource, // Simplified for heap storage
    pub ready: bool,
}

/// Registry supporting all module types
#[derive(Debug)]
pub struct ModuleRegistry {
    /// All modules by ID
    pub modules: HashMap<u32, ObjectReference>,
    
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
    
    pub fn register_module(&mut self, module: &Module, vm: Arc<BlinkVM>) -> ObjectReference {
        let name = module.name; // Get name before moving module
        
        let module_ref = vm.alloc_module(module);
        self.modules.insert(name, module_ref);
        module_ref
    }
 
    pub fn find_module_file(&self, module_name: u32) -> Option<u32> {
        self.module_files.get(&module_name).copied()
    }
    
    /// Get module by ID
    pub fn get_module(&self, name: u32) -> Option<ObjectReference> {
        self.modules.get(&name).copied()
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
}

#[derive(Debug, Clone)]
pub enum ImportType {
    File(String),                       // (imp "module-name") -> module name as symbol ID
    Symbols { 
        symbols: Vec<u32>,           // Symbol IDs to import
        module: u32,                 // Module name as symbol ID
        aliases: HashMap<u32, u32>,  // original symbol ID -> alias symbol ID
    },                               // (imp [sym1 sym2] :from module)
}