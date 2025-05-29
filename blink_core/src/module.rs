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
    /// Module name (e.g., "math/utils", "serde-json")
    pub name: String,
    
    /// Module environment containing definitions
    pub env: Arc<RwLock<Env>>,
    
    /// Exported symbol names
    pub exports: HashSet<String>,
    
    /// How this module was loaded
    pub source: ModuleSource,
    
    /// Whether this module has been fully loaded/compiled
    pub ready: bool,
}

/// Registry supporting all module types
#[derive(Debug)]
pub struct ModuleRegistry {
    /// All modules by name
    modules: HashMap<String, Arc<RwLock<Module>>>,
    
    /// Files that have been evaluated (for Blink modules)
    evaluated_files: HashSet<PathBuf>,
    
    /// File -> modules mapping (for multi-module .blink files)
    file_modules: HashMap<PathBuf, Vec<String>>,
    module_files: HashMap<String, PathBuf>,
    
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

    pub fn remove_module(&mut self, name: &str) -> bool {
        self.modules.remove(name).is_some()
    }
    
    /// Remove a native library from storage
    pub fn remove_native_library(&mut self, path: &PathBuf) -> bool {
        self.loaded_libraries.remove(path).is_some()
    }

    pub fn store_native_library(&mut self, path: PathBuf, lib: Library) {
        self.loaded_libraries.insert(path, lib);
    }
    pub fn register_module(&mut self, module: Module) -> Arc<RwLock<Module>> {
        let name = module.name.clone(); // Get name before moving module
        
        // Build file_modules mapping for file-based modules
        match &module.source {
            ModuleSource::BlinkFile(path) => {
                self.file_modules
                    .entry(path.clone())
                    .or_insert_with(Vec::new)
                    .push(module.name.clone());
                
                // Also build reverse mapping for fast lookup
                self.module_files.insert(module.name.clone(), path.clone());
            },
            ModuleSource::BlinkDll(path) | 
            ModuleSource::Wasm(path) | 
            ModuleSource::NativeDylib(path) => {
                // For native modules, still track the reverse mapping
                self.module_files.insert(module.name.clone(), path.clone());
            },
            _ => (),
        }
        
        let module_arc = Arc::new(RwLock::new(module));
        self.modules.insert(name, module_arc.clone());
        module_arc
    }
 
    pub fn find_module_file(&self, module_name: &str) -> Option<PathBuf> {
        self.module_files.get(module_name).cloned()
    }
    
    /// Get module by name
    pub fn get_module(&self, name: &str) -> Option<Arc<RwLock<Module>>> {
        self.modules.get(name).cloned()
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
    pub fn modules_in_file(&self, file: &PathBuf) -> Vec<String> {
        self.file_modules.get(file).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub enum ImportType {
    File(String),                    // (imp "module-name")
    Symbols { 
        symbols: Vec<String>, 
        module: String,
        aliases: HashMap<String, String>, // original -> alias
    },                               // (imp [sym1 sym2] :from module)
}