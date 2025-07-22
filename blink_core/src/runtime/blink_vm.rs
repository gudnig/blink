use std::{collections::HashMap, ffi::c_void, path::PathBuf, sync::{Arc, OnceLock}};
use mmtk::{
    util::{address, options::PlanSelector, Address, ObjectReference}, MMTKBuilder, Mutator, MMTK
    
};
use parking_lot::RwLock;

use crate::{
    env::Env, module::{Module, ModuleRegistry, SerializedModuleSource}, parser::ReaderContext, runtime::{
        BlinkActivePlan, HandleRegistry, SymbolTable, ValueMetadataStore
    }, telemetry::TelemetryEvent, value::{Callable, GcPtr, ValueRef}
};

pub static GLOBAL_VM: OnceLock<Arc<BlinkVM>> = OnceLock::new();
pub static GLOBAL_MMTK: OnceLock<Box<MMTK<BlinkVM>>> = OnceLock::new(); 

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
    // pub mmtk: Box<MMTK<BlinkVM>>,
    pub symbol_table: RwLock<SymbolTable>,
    pub global_env: Option<ObjectReference>,
    pub telemetry_sink: Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>,
    pub module_registry: RwLock<ModuleRegistry>,
    pub file_to_modules: RwLock<HashMap<PathBuf, Vec<String>>>,
    pub reader_macros: RwLock<ReaderContext>,
    pub value_metadata: RwLock<ValueMetadataStore>,

    pub gc_roots: RwLock<Vec<ObjectReference>>,  // Track all roots
    pub handle_registry: RwLock<HandleRegistry>,
}

impl std::fmt::Debug for BlinkVM {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlinkVM")
            .field("global_env", &self.global_env)
            .field("gc_roots_count", &self.gc_roots.read().len())
            .finish()
    }
}

impl Default for BlinkVM {
    fn default() -> Self {
        Self::new()
    }
}

impl BlinkVM {

    
    pub fn get_or_init_mmtk() -> &'static MMTK<BlinkVM> {
        GLOBAL_MMTK.get_or_init(|| {
            let mut builder = MMTKBuilder::new();
            builder.options.plan.set(PlanSelector::SemiSpace);
            let threads = *builder.options.threads;
            println!("Threads: {:?}", threads);
            mmtk::memory_manager::mmtk_init(&builder)
        })
    }

    fn construct_vm() -> Self {
        Self {
            // Remove mmtk field
            symbol_table: RwLock::new(SymbolTable::new()),
            global_env: None,
            telemetry_sink: None,
            module_registry: RwLock::new(ModuleRegistry::new()),
            file_to_modules: RwLock::new(HashMap::new()),
            reader_macros: RwLock::new(ReaderContext::new()),
            value_metadata: RwLock::new(ValueMetadataStore::new()),
            handle_registry: RwLock::new(HandleRegistry::new()),
            gc_roots: RwLock::new(Vec::new()),
        }
    }

    pub fn new() -> Self {
        // Initialize MMTK globally (only happens once)
        let mmtk = Self::get_or_init_mmtk();

        let current_thread = mmtk::util::VMThread(
            mmtk::util::OpaquePointer::from_address(mmtk::util::Address::ZERO)
        );

        mmtk::memory_manager::initialize_collection(mmtk, current_thread);
        
        let mut vm = Self::construct_vm();


        vm.register_special_forms();
        vm.init_global_env();
        vm.preload_builtin_reader_macros();
        vm.register_builtins();
        vm.register_builtin_macros();
        vm.register_complex_macros();

        vm
    }

    pub fn new_arc() -> Arc<BlinkVM> {
        let vm = Self::new();
        let vm_arc = Arc::new(vm);
        GLOBAL_VM.set(vm_arc.clone()).expect("GLOBAL_VM already initialized");
        vm_arc.clone()
    }

    fn init_global_env(&mut self) -> ObjectReference {
        let global_env = self.alloc_env(Env::new());
        self.global_env = Some(global_env);
        

        // Register as GC root
        self.add_gc_root(global_env);
        
        global_env
    }

    pub fn global_env(&self) -> ObjectReference {
        self.global_env.unwrap()
    }

    pub fn modules(&self) -> Vec<ObjectReference> {
        let modules = self.module_registry.read();
        modules.modules.values().map(|m| *m).collect()
    }


    pub fn add_gc_root(&self, obj_ref: ObjectReference) {
        self.gc_roots.write().push(obj_ref);
    }

    pub fn trigger_gc(&self) {
        // println!("Manually triggering GC...");
        // let static_mmtk = GLOBAL_MMTK.get().expect("MMTK not initialized");
    
        // let tls = THREAD_TLS.with(|tls_cell| {
        //     tls_cell.get().cloned().unwrap_or_else(|| {
        //         println!("Initializing TLS for GC trigger thread...");
        //         BlinkActivePlan::create_vm_mutator_thread()
        //     })
        // });
    
        // mmtk::memory_manager::handle_user_collection_request(static_mmtk, tls);
        // println!("GC request completed");
    }
    
    // Add method to get allocation stats
    pub fn print_gc_stats(&self) {
        let static_mmtk = GLOBAL_MMTK.get().expect("MMTK not initialized");
        let used = mmtk::memory_manager::used_bytes(static_mmtk);
        let total = mmtk::memory_manager::total_bytes(static_mmtk);
        let free = mmtk::memory_manager::free_bytes(static_mmtk);
        println!("Memory: {} bytes used, {} bytes free, {} bytes total", used, free, total);
    }


    fn register_special_forms(&mut self) {
        let mut st = self.symbol_table.write();
        st.intern("if");
        st.intern("def");
        st.intern("mac");
        st.intern("rmac");
        st.intern("quasiquote");
        st.intern("unquote");
        st.intern("unquote-splicing");
        st.intern("deref");
        st.intern("go");
        st.intern("imp");
        st.intern("mod");
        st.intern("load");
        st.intern("try");
        st.intern("imp");
        st.intern("mod");
        st.intern("load");
        st.intern("macro");
    }


    fn build_simple_macro(&mut self,name: &str) -> u32 {
        let symbol_id = self.symbol_table.write().intern(name);
        let list = self.alloc_vec_or_list(vec![ValueRef::symbol(symbol_id), ValueRef::symbol(symbol_id)], true);
        let body = vec![ValueRef::Heap(GcPtr::new(list))];
        let call = Callable {
            
            is_variadic: false,
            body: body,
            env: self.global_env.unwrap(),
            params: vec![symbol_id],

        };

        let mut global_env = GcPtr::new(self.global_env.unwrap()).read_env();
        
        let macro_ref = self.alloc_macro(call);

        global_env.set(symbol_id, ValueRef::Heap(GcPtr::new(macro_ref)));

        // realloc global env TODO optimize
        let global_env_ref = self.alloc_env(global_env);

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