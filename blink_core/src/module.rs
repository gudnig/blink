use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::Arc};
use libloading::Library;
use parking_lot::RwLock;
use crate::env::Env;

/// Module source specification
#[derive(Clone, Debug)]
pub enum ModuleSource {
    Repl,
    // Local sources
    BlinkFile(PathBuf),           // lib/math/utils.blink
    NativeDylib(PathBuf),         // target/release/libmath.so
    
    // External sources (via load)
    BlinkPackage(String),         // From package manager
    Cargo(String),                // Rust crate to compile
    Git { repo: String, reference: Option<String> }, // Git repository
    Url(String),                  // Direct URL
    
    // Future
    BlinkDll(PathBuf),           // Compiled Blink module
    Wasm(PathBuf),               // WebAssembly module
}

/// A module in any supported format
#[derive(Clone, Debug)]
pub struct Module {
    /// Module name ID (e.g., symbol ID for "math/utils", "serde-json")
    pub name: u32,
    
    /// Module environment containing definitions
    pub env: Arc<RwLock<Env>>,
    
    /// Exported symbol IDs
    pub exports: HashSet<u32>,
    
    /// How this module was loaded
    pub source: ModuleSource,
    
    /// Whether this module has been fully loaded/compiled
    pub ready: bool,
}

/// Registry supporting all module types
#[derive(Debug)]
pub struct ModuleRegistry {
    /// All modules by ID
    modules: HashMap<u32, Arc<RwLock<Module>>>,
    
    /// Files that have been evaluated (for Blink modules)
    evaluated_files: HashSet<PathBuf>,
    
    /// File -> module IDs mapping (for multi-module .blink files)
    file_modules: HashMap<PathBuf, Vec<u32>>,
    module_files: HashMap<u32, PathBuf>,
    
    /// Native libraries that have been loaded
    loaded_libraries: HashMap<PathBuf, libloading::Library>,
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
    pub fn remove_native_library(&mut self, path: &PathBuf) -> bool {
        self.loaded_libraries.remove(path).is_some()
    }

    pub fn store_native_library(&mut self, path: &PathBuf, lib: Library) {
        self.loaded_libraries.insert(path.clone(), lib);
    }
    
    pub fn register_module(&mut self, module: Module) -> Arc<RwLock<Module>> {
        let name = module.name; // Get name before moving module
        
        // Build file_modules mapping for file-based modules
        match &module.source {
            ModuleSource::BlinkFile(path) => {
                self.file_modules
                    .entry(path.clone())
                    .or_insert_with(Vec::new)
                    .push(module.name);
                
                // Also build reverse mapping for fast lookup
                self.module_files.insert(module.name, path.clone());
            },
            ModuleSource::BlinkDll(path) | 
            ModuleSource::Wasm(path) | 
            ModuleSource::NativeDylib(path) => {
                // For native modules, still track the reverse mapping
                self.module_files.insert(module.name, path.clone());
            },
            _ => (),
        }
        
        let module_arc = Arc::new(RwLock::new(module));
        self.modules.insert(name, module_arc.clone());
        module_arc
    }
 
    pub fn find_module_file(&self, module_name: u32) -> Option<PathBuf> {
        self.module_files.get(&module_name).cloned()
    }
    
    /// Get module by ID
    pub fn get_module(&self, name: u32) -> Option<Arc<RwLock<Module>>> {
        self.modules.get(&name).cloned()
    }
    
    /// Mark file as evaluated
    pub fn mark_file_evaluated(&mut self, file: PathBuf) {
        self.evaluated_files.insert(file);
    }
    
    /// Check if file has been evaluated
    pub fn is_file_evaluated(&self, file: &PathBuf) -> bool {
        self.evaluated_files.contains(file)
    }
    
    /// Get all modules defined in a file
    pub fn modules_in_file(&self, file: &PathBuf) -> Vec<u32> {
        self.file_modules.get(file).cloned().unwrap_or_default()
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