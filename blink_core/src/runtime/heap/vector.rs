// blink_core/src/runtime/heap/vector.rs

use crate::runtime::{is_object_shared, obj_lock, obj_unlock, BlinkActivePlan, BlinkObjectModel, BlinkSlot, ObjectLockGuard, TypeTag };
use crate::{runtime::BlinkVM, value::ValueRef};


use mmtk::util::{ Address};
use mmtk::MutatorContext;
use mmtk::{util::ObjectReference, Mutator};

use std::ptr;

impl BlinkVM {
    // === Read Operations (No barriers needed) ===
    
    /// Get vector length - fast, no locking needed for metadata reads
    pub fn vector_get_length(&self, vector: ObjectReference) -> u32 {
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
            ptr::read_unaligned(header_ptr as *const u32)
        }
    }
    
    /// Get vector capacity
    pub fn vector_get_capacity(&self, vector: ObjectReference) -> u32 {
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
            let offset = std::mem::size_of::<u32>(); // Skip length
            std::ptr::read_unaligned(header_ptr.add(offset) as *const u32)
        }
    }
    
    /// Get element at index - conditional locking
    pub fn vector_get_at(&self, vector: ObjectReference, index: u32) -> Result<ValueRef, String> {
        // Conditional locking based on sharing
        let _guard = if is_object_shared(vector) {
            Some(ObjectLockGuard::new(vector))
        } else {
            None
        };
        
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            let length = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
            offset += std::mem::size_of::<u32>() * 2; // Skip length and capacity
            
            if index >= length {
                return Err(format!("Index {} out of bounds for vector of length {}", index, length));
            }
            
            let data_ref = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
            let data_ptr = data_ref.to_raw_address().as_usize() as *mut u8;
            let item_ptr = data_ptr.add(index as usize * std::mem::size_of::<ValueRef>());
            
            Ok(std::ptr::read_unaligned(item_ptr as *const ValueRef))
        }
    }
    
    // === Write Operations (Need barriers and locking) ===
    
    /// Update element at index - proper barriers and conditional locking
    pub fn vector_update_at(&self, vector: ObjectReference, index: u32, element: ValueRef) -> Result<(), String> {
        self.with_mutator(|mutator| {
            // Conditional locking
            let _guard = if is_object_shared(vector) {
                Some(ObjectLockGuard::new(vector))
            } else {
                None
            };
            
            unsafe {
                let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                let length = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>() * 2; // Skip length and capacity
                
                if index >= length {
                    return Err(format!("Index {} out of bounds for vector of length {}", index, length));
                }
                
                let data_ref = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
                let data_ptr = data_ref.to_raw_address().as_usize() as *mut u8;
                let item_ptr = data_ptr.add(index as usize * std::mem::size_of::<ValueRef>());
                
                // Write the element
                std::ptr::write_unaligned(item_ptr as *mut ValueRef, element);
                
                // Barrier only if storing a heap reference
                if let ValueRef::Heap(gc_ptr) = element {
                    let slot_addr = Address::from_mut_ptr(item_ptr);
                    mutator.barrier().object_reference_write(
                        data_ref,  // Source object (data array)
                        BlinkSlot::ValueRef(slot_addr), 
                        gc_ptr.0   // Target reference being stored
                    );
                }
                
                Ok(())
            }
        })
    }
    
    /// Push element to end of vector
    pub fn vector_push(&self, vector: ObjectReference, element: ValueRef) -> Result<(), String> {
        self.with_mutator(|mutator| {
            let _guard = if is_object_shared(vector) {
                Some(ObjectLockGuard::new(vector))
            } else {
                None
            };
            
            unsafe {
                let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                let current_length = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                
                let current_capacity = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                
                // Check if we need to resize
                if current_length >= current_capacity {
                    let new_capacity = (current_capacity * 2).max(8);
                    self.vector_ensure_capacity_internal(vector, new_capacity, mutator)?;
                }
                
                // Get (possibly new) data reference
                let data_ref = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
                let data_ptr = data_ref.to_raw_address().as_usize() as *mut u8;
                let item_ptr = data_ptr.add(current_length as usize * std::mem::size_of::<ValueRef>());
                
                // Write the element
                std::ptr::write_unaligned(item_ptr as *mut ValueRef, element);
                
                // Update length
                std::ptr::write_unaligned(header_ptr as *mut u32, current_length + 1);
                
                // Barrier for the new element
                if let ValueRef::Heap(gc_ptr) = element {
                    let slot_addr = Address::from_mut_ptr(item_ptr);
                    mutator.barrier().object_reference_write(
                        data_ref, 
                        BlinkSlot::ValueRef(slot_addr), 
                        gc_ptr.0
                    );
                }
                
                Ok(())
            }
        })
    }
    
    /// Pop element from end of vector
    pub fn vector_pop(&self, vector: ObjectReference) -> Result<Option<ValueRef>, String> {
        let _guard = if is_object_shared(vector) {
            Some(ObjectLockGuard::new(vector))
        } else {
            None
        };
        
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
            let current_length = std::ptr::read_unaligned(header_ptr as *const u32);
            
            if current_length == 0 {
                return Ok(None);
            }
            
            let new_length = current_length - 1;
            
            // Get the element before removing it
            let element = self.vector_get_at(vector, new_length)?;
            
            // Update length
            std::ptr::write_unaligned(header_ptr as *mut u32, new_length);
            
            Ok(Some(element))
        }
    }
    
    /// Resize vector - complex operation with proper data copying
    pub fn vector_resize(&self, vector: ObjectReference, new_size: u32, fill_value: ValueRef) -> Result<(), String> {
        self.with_mutator(|mutator| {
            let _guard = if is_object_shared(vector) {
                Some(ObjectLockGuard::new(vector))
            } else {
                None
            };
            
            unsafe {
                let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                let current_length = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                
                let current_capacity = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
                offset += std::mem::size_of::<u32>();
                
                // If shrinking, just update length
                if new_size <= current_length {
                    std::ptr::write_unaligned(header_ptr as *mut u32, new_size);
                    return Ok(());
                }
                
                // If growing beyond capacity, reallocate
                if new_size > current_capacity {
                    self.vector_ensure_capacity_internal(vector, new_size, mutator)?;
                }
                
                // Fill new elements
                let data_ref = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
                let data_ptr = data_ref.to_raw_address().as_usize() as *mut u8;
                
                for i in current_length..new_size {
                    let item_ptr = data_ptr.add(i as usize * std::mem::size_of::<ValueRef>());
                    std::ptr::write_unaligned(item_ptr as *mut ValueRef, fill_value);
                    
                    // Barrier for each new element
                    if let ValueRef::Heap(gc_ptr) = fill_value {
                        let slot_addr = Address::from_mut_ptr(item_ptr);
                        mutator.barrier().object_reference_write(
                            data_ref, 
                            BlinkSlot::ValueRef(slot_addr), 
                            gc_ptr.0
                        );
                    }
                }
                
                // Update length
                std::ptr::write_unaligned(header_ptr as *mut u32, new_size);
                
                Ok(())
            }
        })
    }
    
    // === Internal Helper Methods ===
    
    /// Ensure vector has at least the specified capacity
    fn vector_ensure_capacity_internal(&self, vector: ObjectReference, new_capacity: u32, mutator: &mut Mutator<BlinkVM>) -> Result<(), String> {
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            let current_length = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
            offset += std::mem::size_of::<u32>();
            
            let current_capacity = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
            offset += std::mem::size_of::<u32>();
            
            if new_capacity <= current_capacity {
                return Ok(()); // Already sufficient capacity
            }
            
            // Allocate new data array
            let new_data_size = new_capacity as usize * std::mem::size_of::<ValueRef>();
            let new_data_ref = BlinkActivePlan::alloc(mutator, &new_data_size);
            let new_data_ptr = new_data_ref.to_raw_address().as_usize() as *mut u8;
            
            // Copy existing elements
            let old_data_ref = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
            let old_data_ptr = old_data_ref.to_raw_address().as_usize() as *const u8;
            
            for i in 0..current_length {
                let old_item_ptr = old_data_ptr.add(i as usize * std::mem::size_of::<ValueRef>());
                let new_item_ptr = new_data_ptr.add(i as usize * std::mem::size_of::<ValueRef>());
                let element = std::ptr::read_unaligned(old_item_ptr as *const ValueRef);
                std::ptr::write_unaligned(new_item_ptr as *mut ValueRef, element);
                
                // Barrier for copied elements
                if let ValueRef::Heap(gc_ptr) = element {
                    let slot_addr = Address::from_mut_ptr(new_item_ptr);
                    mutator.barrier().object_reference_write(
                        new_data_ref, 
                        BlinkSlot::ValueRef(slot_addr), 
                        gc_ptr.0
                    );
                }
            }
            
            // Update header with new data reference and capacity
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, new_capacity);
            offset += std::mem::size_of::<u32>();
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut ObjectReference, new_data_ref);
            
            // Barrier for data reference update
            let data_ref_slot_addr = vector.to_raw_address() + (2 * std::mem::size_of::<u32>());
            mutator.barrier().object_reference_write(
                vector, 
                BlinkSlot::ObjectRef(data_ref_slot_addr), 
                new_data_ref
            );
            
            Ok(())
        }
    }
    
    /// Mark object as shared when passed to another goroutine
    pub fn mark_object_as_shared(&self, obj_ref: ObjectReference) {
        // This will transition the object to thin-lock state
        obj_lock(obj_ref);
        obj_unlock(obj_ref);
        // Object is now marked as having been accessed by multiple goroutines
    }
    
    // === Convenience Methods ===
    
    /// Check if vector is empty
    pub fn vector_is_empty(&self, vector: ObjectReference) -> bool {
        self.vector_get_length(vector) == 0
    }
    
    /// Clear all elements from vector
    pub fn vector_clear(&self, vector: ObjectReference) {
        let _guard = if is_object_shared(vector) {
            Some(ObjectLockGuard::new(vector))
        } else {
            None
        };
        
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *mut u8;
            std::ptr::write_unaligned(header_ptr as *mut u32, 0); // Set length to 0
        }
    }

    pub fn alloc_vec(&self, items: Vec<ValueRef>, capacity: Option<usize>) -> ObjectReference {
        
        self.with_mutator(|mutator| {
            let capacity = capacity.unwrap_or(items.len().max(8));
            

            let header_size =
                std::mem::size_of::<u32>() +      // length
                std::mem::size_of::<u32>() +      // capacity  
                std::mem::size_of::<ObjectReference>(); // data_ptr

            let data_size = capacity * std::mem::size_of::<ValueRef>();
            
            
            let type_tag = TypeTag::Vector;
            
            // Allocate header object
            
            let object_start = BlinkActivePlan::alloc_object(mutator, &type_tag, &header_size);

            // Allocate separate data array
            let data_start = BlinkActivePlan::alloc(mutator, &data_size);
            
            // Initialize metadata for both allocations
            
            
            unsafe {
                let header_ptr = object_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;

                // Write length
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, items.len() as u32);
                offset += std::mem::size_of::<u32>();

                // Write capacity
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, capacity as u32);
                offset += std::mem::size_of::<u32>();

                // Write data pointer
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut ObjectReference, data_start);

                // Write data array
                let data_ptr = data_start.to_raw_address().as_usize() as *mut ValueRef;
                for (i, item) in items.iter().enumerate() {
                    std::ptr::write(data_ptr.add(i), *item);
                }
                
                // Zero out unused capacity
                for i in items.len()..capacity {
                    std::ptr::write(data_ptr.add(i), ValueRef::nil());
                }
            }
            
            object_start
        })
    }
    
    // Debug version of vector access
    pub fn vector_get_length_debug(&self, vector: ObjectReference) -> u32 {
        println!("🔍 DEBUG: Getting length of vector: {:?}", vector);
        
        unsafe {
            let header_ptr = vector.to_raw_address().as_usize() as *const u8;
            println!("🔍 DEBUG: Header ptr: {:p}", header_ptr);
            
            let length = std::ptr::read_unaligned(header_ptr as *const u32);
            println!("🔍 DEBUG: Length read: {}", length);
            
            length
        }
    }
}