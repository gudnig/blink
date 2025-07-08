
use std::{collections::HashMap, path::PathBuf, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use mmtk::{
    util::options::PlanSelector, 
    MMTK, 
    MMTKBuilder,
    
};
use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::{
    env::Env, module::ModuleRegistry, parser::ReaderContext, runtime::{
        GoroutineId, HandleRegistry, SymbolTable, ValueMetadataStore
    }, telemetry::TelemetryEvent
};

pub struct BlinkVM {
    pub mmtk: Box<MMTK<BlinkVM>>,
    pub symbol_table: RwLock<SymbolTable>,
    pub global_env: Arc<RwLock<Env>>,
    pub telemetry_sink: Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>,
    pub module_registry: RwLock<ModuleRegistry>,
    pub file_to_modules: RwLock<HashMap<PathBuf, Vec<String>>>,
    pub reader_macros: RwLock<ReaderContext>,
    pub value_metadata: RwLock<ValueMetadataStore>,
    pub handle_registry: RwLock<HandleRegistry>,
}

impl Default for BlinkVM {
    fn default() -> Self {
        Self::new()
    }
}

impl BlinkVM {
    pub fn new() -> Self {
        // Create MMTK builder and configure it
        
        let mut builder = MMTKBuilder::new();
        
        // Set the GC plan - you can choose different plans:
        // - PlanSelector::NoGC: No garbage collection (useful for testing)
        // - PlanSelector::SemiSpace: Simple copying collector
        // - PlanSelector::MarkSweep: Mark and sweep collector
        // - PlanSelector::Immix: High-performance collector
        builder.options.plan.set(PlanSelector::NoGC); // Start with NoGC for development
        // You can also set other options:
        // builder.options.stress_factor = Some(1); // For testing
        // builder.options.gc_trigger = mmtk::util::options::GCTriggerSelector::FixedHeapSize(1024 * 1024);
        
        // Build the MMTK instance
        let mmtk = mmtk::memory_manager::mmtk_init(&builder);
        
        
        Self {
            mmtk,
            symbol_table: RwLock::new(SymbolTable::new()),
            global_env: Arc::new(RwLock::new(Env::new())),
            telemetry_sink: None,
            module_registry: RwLock::new(ModuleRegistry::new()),
            file_to_modules: RwLock::new(HashMap::new()),
            reader_macros: RwLock::new(ReaderContext::new()),
            value_metadata: RwLock::new(ValueMetadataStore::new()),
            handle_registry: RwLock::new(HandleRegistry::new()),

        }

        

    }

    

}