use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::{Arc, OnceLock}};
use std::collections::HashSet;
use std::ops::DerefMut;
use std::sync::atomic::Ordering;
use dashmap::DashSet;
use mmtk::{
    util::{options::PlanSelector, ObjectReference}, MMTKBuilder, MMTK
    
};
use parking_lot::RwLock;
use tokio::runtime::Runtime;
use crate::{env::Env, module::{Module, ModuleRegistry, SerializedModuleSource}, parser::ReaderContext, runtime::{
    CompiledFunction, HandleRegistry, SuspendedContinuation, SymbolTable, ValueMetadataStore
}, telemetry::TelemetryEvent, value::{ChannelEntry, ChannelHandle, FunctionHandle, SourceRange, ValueRef}, BlinkRuntime, FutureState, GLOBAL_RUNTIME};
use crate::value::FutureHandle;

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
    pub reachable_futures: DashSet<FutureHandle>,
    pub symbol_table: RwLock<SymbolTable>,
    pub telemetry_sink: Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>,
    pub module_registry: RwLock<ModuleRegistry>,
    pub file_to_modules: RwLock<HashMap<PathBuf, Vec<String>>>,
    pub reader_macros: RwLock<ReaderContext>,
    pub value_metadata: RwLock<ValueMetadataStore>,
    pub gc_roots: RwLock<Vec<ObjectReference>>,  // Track all roots
    pub handle_registry: RwLock<HandleRegistry>,
    pub core_module: Option<u32>,
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
    pub fn get_instance() -> &'static BlinkVM {
        GLOBAL_VM.get().expect("BlinkVM not initialized")
    }
    
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
            reachable_futures: DashSet::new(),
            
            telemetry_sink: None,
            module_registry: RwLock::new(ModuleRegistry::new()),
            file_to_modules: RwLock::new(HashMap::new()),

            reader_macros: RwLock::new(ReaderContext::new()),
            value_metadata: RwLock::new(ValueMetadataStore::new()),
            handle_registry: RwLock::new(HandleRegistry::new()),
            gc_roots: RwLock::new(Vec::new()),
            core_module: None,
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
        vm.core_module = Some(core_module_id);
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

    pub fn create_future(&self) -> ValueRef {
        let mut registry = self.handle_registry.write();
        let handle = registry.create_future();
        ValueRef::future_handle(handle.id, handle.generation)
    }

    pub fn create_channel(&self, capacity: Option<usize>) -> ValueRef {
        let mut registry = self.handle_registry.write();
        let handle = registry.create_channel(capacity);
        ValueRef::channel_handle(handle.id, handle.generation) // You'll need this method
    }

    pub fn resolve_channel(&self, handle: ChannelHandle) -> Option<&mut ChannelEntry> {
        let mut registry = self.handle_registry.write();
        let channel = registry.resolve_channel(&handle);
        channel
    }

    


    pub fn complete_future_value(&self, future_value: ValueRef, result: ValueRef) -> Result<(), String> {
        if let Some(handle) = future_value.get_future_handle() {
            let mut registry = self.handle_registry.write();
            if let Some(entry) = registry.futures.get_mut(&handle.id) {
                if entry.generation != handle.generation {
                    return Err("Stale future handle".to_string());
                }

                match entry.state.compare_exchange(
                    FutureState::Pending as u8,
                    FutureState::Ready as u8,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        // Successfully transitioned - safe to complete
                        *entry.result.lock() = Some(result);

                        let mut waiters = entry.waiters.lock(); // Need mut for drain
                        let runtime = GLOBAL_RUNTIME.get().expect("Runtime not initialized");

                        // Resume all continuations
                        {
                            let mut scheduler = runtime.scheduler.lock(); // Lock acquired once
                            for continuation in waiters.continuations.drain(..) {
                                scheduler.resume_goroutine_with_result(continuation, result);
                            }
                        }

                        // Wake async tasks
                        for waker in waiters.async_wakers.drain(..) {
                            waker.wake();
                        }

                        Ok(())
                    }
                    // Should this be an error?
                    Err(_) => Err("Future already completed".to_string()),
                }
            } else {
                Err("Future not found".to_string())
            }
        } else {
            Err("Not a future".to_string())
        }
    }


    pub fn future_add_waiter(&self, future: FutureHandle, continuation: SuspendedContinuation) -> Result<Option<ValueRef>, String> {
        let mut registry = self.handle_registry.write();
        if let Some(entry) = registry.futures.get_mut(&future.id) {
            if entry.generation != future.generation {
                return Err("Stale future handle".to_string());
            }
            {

                Ok(entry.register_continuation(continuation))

            }
        } else {
            Err("Future not found".to_string())
        }
    }
    // Called during GC

    pub fn mark_future_handle_reachable(&self, value: ValueRef) {
        let handle = value.get_future_handle();
        if let Some(handle) = handle {
            self.reachable_futures.insert(handle);
        }

    }

    pub fn clear_reachable_handles(&self) {
        self.reachable_futures.clear();
    }

    pub fn get_reachable_handles(&self) -> DashSet<FutureHandle> {
        self.reachable_futures.iter().map(|h| *h).collect()
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
        st.intern("load");
        st.intern("macro");
        st.intern("loop");
        st.intern("recur");
        st.intern("quote");
    }

    pub fn resolve_global_symbol(&self, module_id: u32, symbol_id: u32) -> Option<ValueRef> {
        
        let module_registry = self.module_registry.read();
        let core_module = self.core_module.unwrap();
        let value = module_registry.resolve_symbol(module_id, symbol_id);
        match value {
            Some(value) => Some(value),
            None => {
                module_registry.resolve_symbol(core_module, symbol_id)
            }
        }
    }


    fn build_simple_macro(&mut self,name: &str, module: u32) -> u32 {
        let symbol_id = self.symbol_table.write().intern(name);
        let expr = ValueRef::symbol(symbol_id);
        let mut module_registry = self.module_registry.write();
        module_registry.update_module(module, symbol_id, expr);
        symbol_id
    }

    pub fn preload_builtin_reader_macros(&mut self, module: u32) {
        
        
        let quote = self.build_simple_macro("quote", module);
    
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


// Future API
impl BlinkVM {
    pub fn spawn_goroutine(&self, func: ValueRef, args: Vec<ValueRef>) -> ValueRef {
        todo!()
    }

    pub fn future_from_rust_future(rust_future: Pin<Box<dyn Future<Output = ValueRef> + Send>>) -> ValueRef {
        todo!()
    }
}