use std::{collections::HashMap, ffi::c_void, path::PathBuf};
use mmtk::{
    util::{address, options::PlanSelector, Address, ObjectReference}, MMTKBuilder, MMTK
    
};
use parking_lot::RwLock;

use crate::{
    env::Env, module::{Module, ModuleRegistry, SerializedModuleSource}, parser::ReaderContext, runtime::{
        HandleRegistry, SymbolTable, ValueMetadataStore
    }, telemetry::TelemetryEvent, value::{Callable, GcPtr, ValueRef}
};

extern "C" {
    // Apple-specific JIT protection functions
    fn pthread_jit_write_protect_np(enabled: i32);
    fn pthread_jit_write_protect_supported_np() -> i32;
    fn sys_icache_invalidate(start: *const c_void, size: usize);
}

// Constants for Apple Silicon mmap
const MAP_JIT: i32 = 0x800;
const MAP_PRIVATE: i32 = 0x0002;
const MAP_ANON: i32 = 0x1000;

pub struct BlinkVM {
    pub mmtk: Box<MMTK<BlinkVM>>,
    pub symbol_table: RwLock<SymbolTable>,
    pub global_env: Option<ObjectReference>,
    pub global_module: Option<ObjectReference>,
    pub telemetry_sink: Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>,
    pub module_registry: RwLock<ModuleRegistry>,
    pub file_to_modules: RwLock<HashMap<PathBuf, Vec<String>>>,
    pub reader_macros: RwLock<ReaderContext>,
    pub value_metadata: RwLock<ValueMetadataStore>,

    pub gc_roots: RwLock<Vec<ObjectReference>>,  // Track all roots
    pub handle_registry: RwLock<HandleRegistry>,
}

impl Default for BlinkVM {
    fn default() -> Self {
        Self::new()
    }
}

impl BlinkVM {

    
    pub fn global_env(&self) -> ObjectReference {
        self.global_env.unwrap()
    }
    
    fn construct_vm(mmtk: Box<mmtk::MMTK<BlinkVM>>) -> Self {
        
        Self {
            mmtk,
            symbol_table: RwLock::new(SymbolTable::new()),
            global_env: None,
            global_module: None,
            telemetry_sink: None,
            module_registry: RwLock::new(ModuleRegistry::new()),
            file_to_modules: RwLock::new(HashMap::new()),
            reader_macros: RwLock::new(ReaderContext::new()),
            value_metadata: RwLock::new(ValueMetadataStore::new()),
            handle_registry: RwLock::new(HandleRegistry::new()),
            gc_roots: RwLock::new(Vec::new()),
        }
    }

    fn init_global_env(&mut self) -> ObjectReference {
        let global_env = self.alloc_env(Env::new());
        self.global_env = Some(global_env);
        

        let global_module_name = self.symbol_table.write().intern("global");
        let global_module = Module {
            name: global_module_name,
            env: global_env,
            exports: vec![],
            source: SerializedModuleSource::Global,
            ready: true,
        };

        // Register as GC root
        self.add_gc_root(global_env);
        
        global_env
    }

    pub fn new() -> Self {
        // Standard MMTK initialization for non-Apple Silicon
        let mut builder = MMTKBuilder::new();
        builder.options.plan.set(PlanSelector::NoGC);
        
        let mmtk = mmtk::memory_manager::mmtk_init(&builder);
        
        let mut vm = Self::construct_vm(mmtk);
        vm.init_global_env();
        vm.preload_builtin_reader_macros();
        vm.register_builtins();
        
        vm
        
    }

    pub fn add_gc_root(&self, obj_ref: ObjectReference) {
        self.gc_roots.write().push(obj_ref);
    }


    fn build_simple_macro(&mut self,name: &str) -> u32 {
        let symbol_id = self.symbol_table.write().intern(name);
        let global_module_name = self.symbol_table.write().intern("global");
        let list = self.alloc_vec_or_list(vec![ValueRef::symbol(symbol_id), ValueRef::symbol(symbol_id)], true);
        let body = vec![ValueRef::Heap(GcPtr::new(list))];
        let call = Callable {
            
            is_variadic: false,
            body: body,
            env: self.global_env.unwrap(),
            params: vec![symbol_id],

        };

        let mut global_env = GcPtr::new(self.global_env.unwrap()).read_env();
        let global_module = GcPtr::new(self.global_module.unwrap()).read_module();
        let macro_ref = self.alloc_macro(call);

        global_env.set(symbol_id, ValueRef::Heap(GcPtr::new(macro_ref)));

        // realloc global env TODO optimize
        let global_env_ref = self.alloc_env(global_env);
        
        let global_module = Module {
            name: global_module_name,
            env: global_env_ref,
            exports: vec![],
            source: SerializedModuleSource::Global,
            ready: true,
        };

        let global_module_ref = self.alloc_module(&global_module);
        
        self.module_registry.write().modules.insert(global_module_name, global_module_ref);
        self.global_module = Some(global_module_ref);
        self.global_env = Some(global_env_ref);
        
        symbol_id

    }

    pub fn preload_builtin_reader_macros(&mut self) {
        
        
        let quote = self.build_simple_macro("quo");
        let quasiquote = self.build_simple_macro("quasiquote");
        let unquote = self.build_simple_macro("unquote");
        let unquote_splicing = self.build_simple_macro("unquote-splicing");
        let deref = self.build_simple_macro("deref");

        let mut rm = self.reader_macros.write();

        // Single character reader macros
        rm.reader_macros
            .insert("\'".into(), quote);
        rm.reader_macros
            .insert("`".into(), quasiquote);
        rm.reader_macros
            .insert("~".into(), unquote);

        rm.reader_macros
        .insert("~@".into(), unquote_splicing);

        rm.reader_macros
        .insert("@".into(), deref);

        
    }
}