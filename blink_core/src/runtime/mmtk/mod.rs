mod object_model;
mod scanning;
mod collection;
mod active_plan;
mod reference_glue;
mod slot;
mod gc_work;
mod upcalls;


use mmtk::vm::VMBinding;
pub use object_model::*;
pub use scanning::*;
pub use collection::*;
pub use active_plan::*;
pub use reference_glue::*;
pub use slot::*;
pub use gc_work::*;
pub use upcalls::*;

use crate::{runtime::BlinkVM, value::pack_number, value::ValueRef};


impl VMBinding for BlinkVM {
    type VMObjectModel = BlinkObjectModel;
    type VMScanning = BlinkScanning;
    type VMCollection = BlinkCollection;
    type VMActivePlan = BlinkActivePlan;
    type VMReferenceGlue = BlinkReferenceGlue;
    type VMSlot = BlinkSlot;
    type VMMemorySlice = BlinkMemorySlice;
    
    const ALIGNMENT_VALUE: usize = 0xdead_beef;
    const MIN_ALIGNMENT: usize = 8; // 8-byte alignment, typical for 64-bit systems
    const MAX_ALIGNMENT: usize = 64; // Maximum alignment, adjust as needed
    const USE_ALLOCATION_OFFSET: bool = true;
    const ALLOC_END_ALIGNMENT: usize = 1;
}


// Add this test to blink_core/src/runtime/mod.rs or create a new test file

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{pack_number, ValueRef};

    #[test]
    fn test_semispace_basic_allocation() {
        println!("=== Testing Semispace GC ===");
        
        // Create a VM with semispace GC
        let vm = BlinkVM::new();
        
        // Test basic allocation using your actual methods
        let str1 = vm.alloc_str("Hello, semispace GC!");
        println!("Allocated string 1: {:?}", str1);
        
        let str2 = vm.alloc_str("This is another test string");
        println!("Allocated string 2: {:?}", str2);
        
        // Test vector allocation
        let vec1 = vm.alloc_vec_or_list(vec![
            ValueRef::Immediate(pack_number(1.0)),
            ValueRef::Immediate(pack_number(2.0)),
            ValueRef::Immediate(pack_number(3.0)),
        ], false);
        println!("Allocated vector: {:?}", vec1);
        
        // Allocate more objects to potentially trigger GC
        println!("Allocating many strings...");
        for i in 0..50 {
            let test_str = vm.alloc_str(&format!("Test string number {}", i));
            if i % 10 == 0 {
                println!("Allocated string {}: {:?}", i, test_str);
            }
        }
        
        // Try to trigger GC manually
        println!("Triggering GC manually...");
        vm.trigger_gc();
        
        // Allocate after GC
        let post_gc_str = vm.alloc_str("Post-GC allocation test");
        println!("Allocated after GC: {:?}", post_gc_str);
        
        println!("Semispace test completed successfully!");
    }

    #[test]
    fn test_create_blink_values() {
        println!("=== Testing Blink Values with GC ===");
        
        let vm = std::sync::Arc::new(BlinkVM::new());
        
        // Test creating different types of Blink values
        let str_obj = vm.alloc_str("Hello, World!");
        println!("Created string: {:?}", str_obj);
        
        let vec_obj = vm.alloc_vec_or_list(vec![
            ValueRef::Immediate(pack_number(42.0)),
            ValueRef::Immediate(pack_number(24.0)),
        ], false);
        println!("Created vector: {:?}", vec_obj);
        
        let list_obj = vm.alloc_vec_or_list(vec![
            ValueRef::Immediate(pack_number(100.0)),
            ValueRef::Immediate(pack_number(200.0)),
        ], true);
        println!("Created list: {:?}", list_obj);
        
        // Allocate more to stress the system
        for i in 0..20 {
            let _temp_str = vm.alloc_str(&format!("Temporary {}", i));
            let _temp_vec = vm.alloc_vec_or_list(vec![ValueRef::Immediate(pack_number(i as f64))], false);
        }
        
        // Try GC after creating some objects
        println!("Triggering GC...");
        vm.trigger_gc();
        
        println!("After GC - original objects should still be valid:");
        println!("String: {:?}", str_obj);
        println!("Vector: {:?}", vec_obj);
        println!("List: {:?}", list_obj);
        
        println!("Blink values test completed successfully!");
    }

    #[test]
    fn test_gc_stress() {
        println!("=== GC Stress Test ===");
        
        let vm = BlinkVM::new();
        
        // Allocate lots of objects to force GC
        println!("Stress testing allocation...");
        for round in 0..5 {
            println!("Round {}", round);
            
            // Allocate many objects in this round
            for i in 0..100 {
                let _str = vm.alloc_str(&format!("Round {} item {}", round, i));
                let _vec = vm.alloc_vec_or_list(vec![
                    ValueRef::Immediate(pack_number((round * 100 + i) as f64)),
                    ValueRef::Immediate(pack_number((round * 200 + i) as f64)),
                ], false);
            }
            
            // Force GC after each round
            vm.trigger_gc();
        }
        
        println!("Stress test completed!");
    }
}

// Manual test function using your actual methods
pub fn test_semispace_manually() {
    println!("=== Manual Semispace GC Test ===");
    
    let vm = BlinkVM::new();
    
    println!("VM created successfully with semispace GC");
    
    // Test basic string allocation
    println!("Testing string allocation...");
    for i in 0..5 {
        let test_str = vm.alloc_str(&format!("Test string {}", i));
        println!("Allocated string {}: {:?}", i, test_str);
    }
    
    // Test vector allocation
    println!("Testing vector allocation...");
    for i in 0..3 {
        let test_vec = vm.alloc_vec_or_list(vec![
            ValueRef::Immediate(pack_number(i as f64)),
            ValueRef::Immediate(pack_number((i * 10) as f64)),
        ], false);
        println!("Allocated vector {}: {:?}", i, test_vec);
    }
    
    // Trigger GC
    println!("Triggering GC...");
    vm.trigger_gc();
    
    // Allocate more after GC
    println!("Testing allocation after GC...");
    for i in 0..3 {
        let post_gc_str = vm.alloc_str(&format!("Post-GC string {}", i));
        println!("Allocated post-GC string {}: {:?}", i, post_gc_str);
    }
    
    println!("Manual test completed!");
}