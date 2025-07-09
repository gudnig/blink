
use std::{collections::HashMap, ffi::c_void, path::PathBuf, ptr, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use mmtk::{
    util::{options::PlanSelector, Address}, MMTKBuilder, MMTK
    
};
use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::{
    env::Env, module::ModuleRegistry, parser::ReaderContext, runtime::{
        GoroutineId, HandleRegistry, SymbolTable, ValueMetadataStore
    }, telemetry::TelemetryEvent
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
        if Self::is_apple_silicon() {
            println!("Apple Silicon detected - applying mmap workarounds");
            Self::new_with_apple_silicon_mmap_fix()
        } else {
            Self::new_standard()
        }
    }
    
    fn is_apple_silicon() -> bool {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            // Also check if pthread_jit_write_protect is supported
            unsafe { pthread_jit_write_protect_supported_np() != 0 }
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            false
        }
    }
    
    fn new_with_apple_silicon_mmap_fix() -> Self {
        println!("=== Apple Silicon MMTK with mmap workarounds ===");
        
        // First, let's try to create a test JIT allocation to see if the workaround works
        Self::test_apple_silicon_jit_allocation();
        
        // Now try MMTK with very conservative settings
        let mut builder = MMTKBuilder::new();
        
        // Minimal configuration
        builder.options.plan.set(PlanSelector::NoGC);
        builder.options.threads.set(1);
        builder.options.stress_factor.set(0);
        builder.options.no_finalizer.set(true);
        builder.options.no_reference_types.set(true);
        
        // Critical: Don't set custom vm_space settings on Apple Silicon
        // Let MMTK handle its own memory allocation patterns
        
        // Try with smallest possible heap
        use mmtk::util::options::GCTriggerSelector;
        builder.options.gc_trigger.set(GCTriggerSelector::FixedHeapSize(1024 * 1024)); // 1MB
        
        let mmtk = mmtk::memory_manager::mmtk_init(&builder);
        println!("✓ MMTK initialized successfully with Apple Silicon workarounds!");
        Self::construct_vm(mmtk)
    }
    
    fn test_apple_silicon_jit_allocation() {
        println!("Testing Apple Silicon JIT allocation...");
        
        unsafe {
            let size = 4096; // One page
            let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
            let flags = MAP_PRIVATE | MAP_ANON | MAP_JIT;
            
            let addr = libc::mmap(
                ptr::null_mut(),
                size,
                prot,
                flags,
                -1,
                0
            );
            
            if addr == libc::MAP_FAILED {
                eprintln!("✗ Basic JIT mmap failed - Apple Silicon support broken");
                return;
            }
            
            println!("✓ Basic JIT mmap succeeded");
            
            // Test write protection toggle
            pthread_jit_write_protect_np(0); // Enable writing
            
            // Try to write some data
            let data_ptr = addr as *mut u8;
            *data_ptr = 0x42;
            
            pthread_jit_write_protect_np(1); // Enable execution
            sys_icache_invalidate(addr, size);
            
            // Verify the write worked
            pthread_jit_write_protect_np(0);
            let value = *data_ptr;
            pthread_jit_write_protect_np(1);
            
            if value == 0x42 {
                println!("✓ Apple Silicon JIT write protection works correctly");
            } else {
                eprintln!("✗ Apple Silicon JIT write protection failed");
            }
            
            libc::munmap(addr, size);
        }
    }
    
    fn construct_vm(mmtk: Box<mmtk::MMTK<BlinkVM>>) -> Self {
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
    
    fn new_standard() -> Self {
        // Standard MMTK initialization for non-Apple Silicon
        let mut builder = MMTKBuilder::new();
        builder.options.plan.set(PlanSelector::NoGC);
        
        let mmtk = mmtk::memory_manager::mmtk_init(&builder);
        
        Self::construct_vm(mmtk)
    }
}