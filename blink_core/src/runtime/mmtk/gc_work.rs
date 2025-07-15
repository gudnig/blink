use mmtk::scheduler::{GCWork, GCWorker};
use mmtk::util::Address;
use mmtk::vm::RootsWorkFactory;
use mmtk::MMTK;

use crate::runtime::{BlinkSlot, BlinkVM, GLOBAL_VM};

// Create a work packet for scanning your VM roots
pub struct ScanBlinkVMRoots<F: RootsWorkFactory<BlinkSlot>> {
    factory: F,
}

impl<F: RootsWorkFactory<BlinkSlot>> ScanBlinkVMRoots<F> {
    pub fn new(factory: F) -> Self {
        Self { factory }
    }
}

impl<F: RootsWorkFactory<BlinkSlot>> GCWork<BlinkVM> for ScanBlinkVMRoots<F> {
    fn do_work(&mut self, _worker: &mut GCWorker<BlinkVM>, _mmtk: &'static MMTK<BlinkVM>) {
        let vm =GLOBAL_VM.get().expect("BlinkVM not initialized");
        let mut root_slots = Vec::new();
            
        // Scan global environment
        if let Some(global_env) = vm.global_env {
            root_slots.push(BlinkSlot::ObjectRef(Address::from_ptr(&global_env)));
        }
        
        // Scan tracked GC roots
        let gc_roots = vm.gc_roots.read();
        for root in gc_roots.iter() {
            root_slots.push(BlinkSlot::ObjectRef(Address::from_ptr(root)));
        }
        
        // Scan module registry
        let modules = vm.modules();
        for module_ref in modules {
            
            root_slots.push(BlinkSlot::ObjectRef(Address::from_ptr(&module_ref)));
        }
        
        // Create work packets for processing these roots
        if !root_slots.is_empty() {
            self.factory.create_process_roots_work(root_slots);
        }
    }
}