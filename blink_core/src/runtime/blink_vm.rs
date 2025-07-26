use std::{collections::HashMap, ffi::c_void, path::PathBuf, sync::{Arc, OnceLock}};
use mmtk::{
    util::{address, options::PlanSelector, Address, ObjectReference}, MMTKBuilder, Mutator, MMTK
    
};
use parking_lot::RwLock;

use crate::{
    env::Env, module::{Module, ModuleRegistry, SerializedModuleSource}, parser::ReaderContext, runtime::{
        BlinkActivePlan, CompiledFunction, ExecutionContext, HandleRegistry, SymbolTable, ValueMetadataStore
    }, telemetry::TelemetryEvent, value::{Callable, FunctionHandle, FutureHandle, GcPtr, SourceRange, ValueRef}
};

pub static GLOBAL_VM: OnceLock<Arc<BlinkVM>> = OnceLock::new();
pub static GLOBAL_MMTK: OnceLock<Box<MMTK<BlinkVM>>> = OnceLock::new(); 

#[derive(Clone, Copy, Debug)]
pub enum SpecialFormId {
    Apply = 0,
    If = 1,
    Def = 2,
    Fn = 3,
    Do = 4,
    Let = 5,
    And = 6,
    Or = 7,
    Try = 8,
    Imp = 9,
    Mod = 10,
    Load = 11,
    Macro = 12,
    Loop = 13,
    Recur = 14,
    Eval = 15,
    Rmac = 16,
    Quasiquote = 17,
    Unquote = 18,
    UnquoteSplicing = 19,
    Go = 20,
    Deref = 21,
    Quote = 22,
}

impl SpecialFormId {
    pub fn from_u32(id: u32) -> Self {
        match id {
            0 => SpecialFormId::Apply,
            1 => SpecialFormId::If,
            2 => SpecialFormId::Def,
            3 => SpecialFormId::Fn,
            4 => SpecialFormId::Do,
            5 => SpecialFormId::Let,
            6 => SpecialFormId::And,    
            7 => SpecialFormId::Or,
            8 => SpecialFormId::Try,
            9 => SpecialFormId::Imp,
            10 => SpecialFormId::Mod,
            11 => SpecialFormId::Load,
            12 => SpecialFormId::Macro,
            13 => SpecialFormId::Loop,
            14 => SpecialFormId::Recur,
            15 => SpecialFormId::Eval,
            16 => SpecialFormId::Rmac,
            17 => SpecialFormId::Quasiquote,
            18 => SpecialFormId::Unquote,
            19 => SpecialFormId::UnquoteSplicing,
            20 => SpecialFormId::Go,
            21 => SpecialFormId::Deref,
            _ => panic!("Invalid special form id: {}", id),
        }
    }

    pub fn to_string(&self) -> &str {
        match self {
            SpecialFormId::If => "if",
            SpecialFormId::Def => "def",
            SpecialFormId::Macro => "macro",
            SpecialFormId::Rmac => "rmac",
            SpecialFormId::Quasiquote => "quasiquote",
            SpecialFormId::Unquote => "unquote",
            SpecialFormId::UnquoteSplicing => "unquote-splicing",
            SpecialFormId::Deref => "deref",
            SpecialFormId::Go => "go",
            SpecialFormId::Apply => "apply",
            SpecialFormId::Fn => "fn",
            SpecialFormId::Do => "do",
            SpecialFormId::Let => "let",
            SpecialFormId::And => "and",
            SpecialFormId::Or => "or",
            SpecialFormId::Try => "try",
            SpecialFormId::Imp => "imp",
            SpecialFormId::Mod => "mod",
            SpecialFormId::Load => "load",
            SpecialFormId::Loop => "loop",
            SpecialFormId::Recur => "recur",
            SpecialFormId::Eval => "eval",
            SpecialFormId::Quote => "quote",
        }
    }
}


pub struct BlinkVM {
    // pub mmtk: Box<MMTK<BlinkVM>>,
    
    pub symbol_table: RwLock<SymbolTable>,
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
            builder.options.plan.set(PlanSelector::MarkSweep);
            let threads = *builder.options.threads;
            println!("Threads: {:?}", threads);
            mmtk::memory_manager::mmtk_init(&builder)
        })
    }

    fn construct_vm() -> Self {
        Self {
            // Remove mmtk field
            symbol_table: RwLock::new(SymbolTable::new()),
            
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

        let core_module_id = vm.symbol_table.write().intern("core");

        let core_module = Module {
            name: core_module_id,
            imports: HashMap::new(),
            exports: HashMap::new(),
            source: SerializedModuleSource::Repl,
            ready: true,
        };

        vm.module_registry.write().register_module(core_module);


        vm.register_special_forms();
        vm.init_global_env();
        vm.preload_builtin_reader_macros(core_module_id);
        vm.register_builtins(core_module_id);
        vm.register_builtin_macros(core_module_id);
        vm.register_complex_macros(core_module_id);

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
        

        
        global_env
    }

    pub fn alloc_compiled_function(&self, bytecode: CompiledFunction) -> ObjectReference {
        // Store the compiled bytecode in a heap object
        // This would be similar to your existing alloc_user_defined_fn
        // but stores CompiledBytecode instead of raw expressions
        todo!("Allocate compiled function object")
    }

    pub fn register_function(&self, handle: ValueRef) -> FunctionHandle {
        self.handle_registry.write().register_function(handle)
    }

    pub fn resolve_function(&self, handle: FunctionHandle) -> Option<ValueRef> {
        self.handle_registry.read().resolve_function(&handle)
    }

    pub fn register_future(&self, handle: ValueRef) -> FutureHandle {
        self.handle_registry.write().register_future(handle)
    }

    pub fn resolve_future(&self, handle: FutureHandle) -> Option<ValueRef> {
        self.handle_registry.read().resolve_future(&handle)
    }

    pub fn intern_keyword(&self, name: &str) -> ValueRef {
        let keyword_id = self.symbol_table.write().intern(name);
        ValueRef::keyword(keyword_id)
    }

    pub fn intern_symbol(&self, name: &str) -> ValueRef {
        let symbol_id = self.symbol_table.write().intern(name);
        ValueRef::symbol(symbol_id)
    }

    pub fn intern_symbol_id(&self, name: &str) -> u32 {
        self.symbol_table.write().intern(name)
    }

    pub fn get_pos(&self, value: ValueRef) -> Option<SourceRange> {
        value.get_or_create_id().and_then(|id| self.value_metadata.read().get_position(id))
    }

    pub fn get_roots(&self) -> Vec<ObjectReference> {
        let mut roots = vec![];
        // TODO: Possible optimzation we can maintain roots in gc_roots and not have to scan the module registry
        for module in self.module_registry.read().modules.values() {
            for (_, value) in module.exports.iter() {
                if let ValueRef::Heap(gc_ptr) = value {
                    roots.push(gc_ptr.0);
                }
            }
        }
        roots.append(&mut self.gc_roots.read().clone());
        roots
    }

    pub fn update_module(&self, module_id: u32, symbol_id: u32, value: ValueRef) {
        let mut binding = self.module_registry.write();
        binding.update_module(module_id, symbol_id, value);
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
        st.intern_special_form(SpecialFormId::If as u32,"if");
        st.intern_special_form(SpecialFormId::Def as u32,"def");
        st.intern_special_form(SpecialFormId::Macro as u32,"mac");
        st.intern_special_form(SpecialFormId::Rmac as u32,"rmac");
        st.intern_special_form(SpecialFormId::Quasiquote as u32,"quasiquote");
        st.intern_special_form(SpecialFormId::Unquote as u32,"unquote");
        st.intern_special_form(SpecialFormId::UnquoteSplicing as u32,"unquote-splicing");
        st.intern_special_form(SpecialFormId::Deref as u32,"deref");
        st.intern_special_form(SpecialFormId::Go as u32,"go");
        st.intern_special_form(SpecialFormId::Imp as u32,"imp"); 
        st.intern_special_form(SpecialFormId::Mod as u32,"mod");
        st.intern_special_form(SpecialFormId::Load as u32,"load");
        st.intern_special_form(SpecialFormId::Try as u32,"try");
        st.intern_special_form(SpecialFormId::Imp as u32,"imp");
        st.intern_special_form(SpecialFormId::Mod as u32,"mod");
        st.intern_special_form(SpecialFormId::Load as u32,"load");
        st.intern_special_form(SpecialFormId::Macro as u32,"macro");
    }

    pub fn resolve_global_symbol(&self, module_id: u32, symbol_id: u32) -> Option<ValueRef> {
        
        let module_registry = self.module_registry.read();
        module_registry.resolve_symbol(module_id, symbol_id)
    }


    fn build_simple_macro(&mut self,name: &str, module: u32) -> u32 {
        
        let symbol_id = self.symbol_table.write().intern(name);
        let list = self.alloc_vec_or_list(vec![ValueRef::symbol(symbol_id), ValueRef::symbol(symbol_id)], true);
        let body = vec![ValueRef::Heap(GcPtr::new(list))];
        let empty_env = self.alloc_env(Env::new());
        let call = Callable {
            module: module,
            is_variadic: false,
            body: body,
            env: empty_env,
            params: vec![symbol_id],

        };

        
        
        let macro_ref = self.alloc_macro(call);
        let value = ValueRef::Heap(GcPtr::new(macro_ref));
        self.update_module(module, symbol_id, value);
        symbol_id

    }

    pub fn preload_builtin_reader_macros(&mut self, module: u32) {
        
        
        let quote = self.build_simple_macro("quo", module);
    
        let quasiquote = self.build_simple_macro("quasiquote", module);
        let unquote = self.build_simple_macro("unquote", module);
        let unquote_splicing = self.build_simple_macro("unquote-splicing", module);
        let deref = self.build_simple_macro("deref", module);

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