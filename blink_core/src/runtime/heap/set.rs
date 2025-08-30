// blink_core/src/runtime/heap/set.rs

use std::hash::{DefaultHasher, Hash, Hasher};
use mmtk::util::ObjectReference;
use crate::runtime::{BlinkActivePlan, BlinkObjectModel, BlinkSlot, BlinkVM, TypeTag, is_object_shared, obj_lock, obj_unlock, ObjectLockGuard};
use crate::value::ValueRef;
use crate::collections::BlinkHashSet;

// Small mode configuration
const SMALL_MODE_CAPACITY: usize = 4;

// Swiss table control bytes
const CTRL_EMPTY: u8 = 0x80;
const CTRL_DELETED: u8 = 0xFE;
const CTRL_FULL_MASK: u8 = 0x80;

// Load factor for resizing (87% = 7/8)
const MAX_LOAD_FACTOR_NUMERATOR: usize = 7;
const MAX_LOAD_FACTOR_DENOMINATOR: usize = 8;

impl BlinkVM {
    
    // === ALLOCATION ===
    
    /// Allocate a new hashset with specified capacity
    /// If capacity <= SMALL_MODE_CAPACITY, creates in small mode
    pub fn alloc_hashset(&self, items: Vec<ValueRef>, capacity: Option<usize>) -> ObjectReference {
        let initial_capacity = capacity.unwrap_or_else(|| {
            if items.len() <= SMALL_MODE_CAPACITY {
                SMALL_MODE_CAPACITY
            } else {
                (items.len() * 2).next_power_of_two().max(8)
            }
        });
        
        self.with_mutator(|mutator| {
            let len = items.len();
            
            if len <= SMALL_MODE_CAPACITY && initial_capacity <= SMALL_MODE_CAPACITY {
                // Create in small mode
                self.alloc_small_hashset(mutator, items, initial_capacity)
            } else {
                // Create in large mode
                self.alloc_large_hashset(mutator, items, initial_capacity)
            }
        })
    }
    
    fn alloc_small_hashset(&self, mutator: &mut mmtk::Mutator<BlinkVM>, items: Vec<ValueRef>, capacity: usize) -> ObjectReference {
        let header_size = 
            std::mem::size_of::<u32>() +      // len
            std::mem::size_of::<u32>() +      // capacity (0 = small mode)
            std::mem::size_of::<bool>();      // mode flag (true = small)
            
        let inline_slots_size = SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, bool)>(); // (value, occupied)
        let total_size = header_size + inline_slots_size;
        
        let object_start = BlinkActivePlan::alloc_object(mutator, &TypeTag::Set, &total_size);
        
        
        unsafe {
            let header_ptr = object_start.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Write header
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, items.len() as u32);
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, 0); // capacity = 0 indicates small mode
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut bool, true); // small mode flag
            offset += std::mem::size_of::<bool>();
            
            // Initialize inline slots
            let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, bool);
            for i in 0..SMALL_MODE_CAPACITY {
                if i < items.len() {
                    std::ptr::write(slots_ptr.add(i), (items[i], true));
                } else {
                    std::ptr::write(slots_ptr.add(i), (ValueRef::nil(), false));
                }
            }
        }
        
        object_start
    }
    
    fn alloc_large_hashset(&self, mutator: &mut mmtk::Mutator<BlinkVM>, items: Vec<ValueRef>, capacity: usize) -> ObjectReference {
        let capacity = capacity.next_power_of_two().max(8);
        
        let header_size = 
            std::mem::size_of::<u32>() +      // len
            std::mem::size_of::<u32>() +      // capacity
            std::mem::size_of::<bool>() +     // mode flag (false = large)
            (SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, bool)>()); // ignored inline slots
            
        let ctrl_bytes_size = capacity;
        let values_size = capacity * std::mem::size_of::<ValueRef>();
        let total_size = header_size + ctrl_bytes_size + values_size;
        
        let object_start = BlinkActivePlan::alloc_object(mutator, &TypeTag::Set, &total_size);
        
        
        unsafe {
            let header_ptr = object_start.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Write header
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, items.len() as u32);
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, capacity as u32);
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut bool, false); // large mode flag
            offset += std::mem::size_of::<bool>();
            
            // Skip inline slots (they're ignored in large mode)
            offset += SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, bool)>();
            
            // Initialize Swiss table
            let ctrl_ptr = header_ptr.add(offset);
            let values_ptr = header_ptr.add(offset + ctrl_bytes_size) as *mut ValueRef;
            
            // Initialize control bytes to EMPTY
            std::ptr::write_bytes(ctrl_ptr, CTRL_EMPTY, capacity);
            
            // Insert items using Swiss table logic
            for item in items {
                let hash = self.hash_value(&item);
                let index = self.swiss_insert_slot_set(ctrl_ptr, capacity, hash, &item, values_ptr);
                std::ptr::write(values_ptr.add(index), item);
            }
        }
        
        object_start
    }
    
    // === ACCESSORS ===
    
    pub fn hashset_get_length(&self, hashset: ObjectReference) -> u32 {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *const u8;
            std::ptr::read_unaligned(header_ptr as *const u32)
        }
    }
    
    pub fn hashset_is_small_mode(&self, hashset: ObjectReference) -> bool {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *const u8;
            let offset = std::mem::size_of::<u32>() * 2; // skip len and capacity
            std::ptr::read_unaligned(header_ptr.add(offset) as *const bool)
        }
    }
    
    pub fn hashset_get_capacity(&self, hashset: ObjectReference) -> u32 {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *const u8;
            let offset = std::mem::size_of::<u32>();
            let capacity = std::ptr::read_unaligned(header_ptr.add(offset) as *const u32);
            if capacity == 0 {
                SMALL_MODE_CAPACITY as u32
            } else {
                capacity
            }
        }
    }
    
    // === LOOKUP ===
    
    pub fn hashset_contains(&self, hashset: ObjectReference, value: &ValueRef) -> bool {
        let _guard = if is_object_shared(hashset) {
            Some(ObjectLockGuard::new(hashset))
        } else {
            None
        };
        
        if self.hashset_is_small_mode(hashset) {
            self.hashset_contains_small(hashset, value)
        } else {
            self.hashset_contains_large(hashset, value)
        }
    }
    
    fn hashset_contains_small(&self, hashset: ObjectReference, value: &ValueRef) -> bool {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *const u8;
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *const (ValueRef, bool);
            
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied && slot_value == *value {
                    return true;
                }
            }
            false
        }
    }
    
    fn hashset_contains_large(&self, hashset: ObjectReference, value: &ValueRef) -> bool {
        unsafe {
            let (ctrl_ptr, values_ptr, capacity) = self.get_large_mode_pointers_set(hashset);
            let hash = self.hash_value(value);
            
            self.swiss_find_slot_set(ctrl_ptr, values_ptr, capacity, hash, value).is_some()
        }
    }
    
    // === INSERTION ===
    
    pub fn hashset_insert(&self, hashset: ObjectReference, value: ValueRef) -> Result<bool, String> {
        let _guard = if is_object_shared(hashset) {
            Some(ObjectLockGuard::new(hashset))
        } else {
            None
        };
        
        if self.hashset_is_small_mode(hashset) {
            let was_new = self.hashset_insert_small(hashset, value)?;
            
            // Check if we need to promote to large mode
            let len = self.hashset_get_length(hashset);
            if len as usize > SMALL_MODE_CAPACITY {
                self.promote_to_large_mode_set(hashset)?;
            }
            
            Ok(was_new)
        } else {
            self.hashset_insert_large(hashset, value)
        }
    }
    
    fn hashset_insert_small(&self, hashset: ObjectReference, value: ValueRef) -> Result<bool, String> {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *mut u8;
            let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
            
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, bool);
            
            // Check if value already exists
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied && slot_value == value {
                    return Ok(false); // Already exists
                }
            }
            
            // Find empty slot
            for i in 0..SMALL_MODE_CAPACITY {
                let (_, occupied) = std::ptr::read(slots_ptr.add(i));
                if !occupied {
                    std::ptr::write(slots_ptr.add(i), (value, true));
                    len += 1;
                    std::ptr::write_unaligned(header_ptr as *mut u32, len);
                    return Ok(true); // New insertion
                }
            }
            
            Err("Small hashset is full".to_string())
        }
    }
    
    fn hashset_insert_large(&self, hashset: ObjectReference, value: ValueRef) -> Result<bool, String> {
        // Check if resize is needed first
        let len = self.hashset_get_length(hashset) as usize;
        let capacity = self.hashset_get_capacity(hashset) as usize;
        
        if len * MAX_LOAD_FACTOR_DENOMINATOR >= capacity * MAX_LOAD_FACTOR_NUMERATOR {
            self.resize_large_hashset(hashset)?;
        }
        
        unsafe {
            let (ctrl_ptr, values_ptr, capacity) = self.get_large_mode_pointers_set(hashset);
            let hash = self.hash_value(&value);
            
            // Check if value exists
            if self.swiss_find_slot_set(ctrl_ptr, values_ptr, capacity, hash, &value).is_some() {
                return Ok(false); // Already exists
            }
            
            // Insert new value
            let index = self.swiss_insert_slot_set(ctrl_ptr, capacity, hash, &value, values_ptr);
            std::ptr::write(values_ptr.add(index), value);
            
            // Update length
            let header_ptr = hashset.to_raw_address().as_usize() as *mut u8;
            let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
            len += 1;
            std::ptr::write_unaligned(header_ptr as *mut u32, len);
            
            Ok(true) // New insertion
        }
    }
    
    // === REMOVAL ===
    
    pub fn hashset_remove(&self, hashset: ObjectReference, value: &ValueRef) -> bool {
        let _guard = if is_object_shared(hashset) {
            Some(ObjectLockGuard::new(hashset))
        } else {
            None
        };
        
        if self.hashset_is_small_mode(hashset) {
            self.hashset_remove_small(hashset, value)
        } else {
            self.hashset_remove_large(hashset, value)
        }
    }
    
    fn hashset_remove_small(&self, hashset: ObjectReference, value: &ValueRef) -> bool {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *mut u8;
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, bool);
            
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied && slot_value == *value {
                    std::ptr::write(slots_ptr.add(i), (ValueRef::nil(), false));
                    
                    // Update length
                    let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
                    len -= 1;
                    std::ptr::write_unaligned(header_ptr as *mut u32, len);
                    
                    return true;
                }
            }
            false
        }
    }
    
    fn hashset_remove_large(&self, hashset: ObjectReference, value: &ValueRef) -> bool {
        unsafe {
            let (ctrl_ptr, values_ptr, capacity) = self.get_large_mode_pointers_set(hashset);
            let hash = self.hash_value(value);
            
            if let Some(index) = self.swiss_find_slot_set(ctrl_ptr, values_ptr, capacity, hash, value) {
                // Mark as deleted
                std::ptr::write(ctrl_ptr.add(index), CTRL_DELETED);
                
                // Update length
                let header_ptr = hashset.to_raw_address().as_usize() as *mut u8;
                let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
                len -= 1;
                std::ptr::write_unaligned(header_ptr as *mut u32, len);
                
                true
            } else {
                false
            }
        }
    }
    
    // === ITERATION ===
    
    /// Get all values from the set as a vector
    pub fn hashset_to_vec(&self, hashset: ObjectReference) -> Vec<ValueRef> {
        let _guard = if is_object_shared(hashset) {
            Some(ObjectLockGuard::new(hashset))
        } else {
            None
        };
        
        if self.hashset_is_small_mode(hashset) {
            self.hashset_to_vec_small(hashset)
        } else {
            self.hashset_to_vec_large(hashset)
        }
    }
    
    fn hashset_to_vec_small(&self, hashset: ObjectReference) -> Vec<ValueRef> {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *const u8;
            let len = std::ptr::read_unaligned(header_ptr as *const u32) as usize;
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *const (ValueRef, bool);
            
            let mut values = Vec::with_capacity(len);
            
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied {
                    values.push(slot_value);
                }
            }
            
            values
        }
    }
    
    fn hashset_to_vec_large(&self, hashset: ObjectReference) -> Vec<ValueRef> {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *const u8;
            let len = std::ptr::read_unaligned(header_ptr as *const u32) as usize;
            let (ctrl_ptr, values_ptr, capacity) = self.get_large_mode_pointers_set(hashset);
            
            let mut values = Vec::with_capacity(len);
            
            for i in 0..capacity {
                let ctrl = std::ptr::read(ctrl_ptr.add(i));
                if ctrl != CTRL_EMPTY && ctrl != CTRL_DELETED {
                    let value = std::ptr::read(values_ptr.add(i));
                    values.push(value);
                }
            }
            
            values
        }
    }
    
    // === MODE PROMOTION ===
    
    fn promote_to_large_mode_set(&self, hashset: ObjectReference) -> Result<(), String> {
        self.with_mutator(|_mutator| {
            // For now, return an error and require external reallocation
            // This is complex because it requires allocating a new object
            Err("Set promotion requires external reallocation".to_string())
        })
    }
    
    // === RESIZE ===
    
    fn resize_large_hashset(&self, hashset: ObjectReference) -> Result<(), String> {
        // Similar to promotion, this requires allocating a new object
        Err("Set resize requires external reallocation".to_string())
    }
    
    // === SWISS TABLE HELPERS FOR SETS ===
    
    fn swiss_find_slot_set(&self, ctrl_ptr: *const u8, values_ptr: *const ValueRef, capacity: usize, hash: u64, value: &ValueRef) -> Option<usize> {
        let hash_7bit = (hash & 0x7F) as u8;
        let mut index = (hash as usize) & (capacity - 1);
        
        unsafe {
            loop {
                let ctrl = std::ptr::read(ctrl_ptr.add(index));
                
                if ctrl == CTRL_EMPTY {
                    return None; // Value not found
                }
                
                if ctrl == hash_7bit {
                    let slot_value = std::ptr::read(values_ptr.add(index));
                    if slot_value == *value {
                        return Some(index);
                    }
                }
                
                index = (index + 1) & (capacity - 1);
            }
        }
    }
    
    fn swiss_insert_slot_set(&self, ctrl_ptr: *mut u8, capacity: usize, hash: u64, value: &ValueRef, values_ptr: *const ValueRef) -> usize {
        let hash_7bit = (hash & 0x7F) as u8;
        let mut index = (hash as usize) & (capacity - 1);
        
        unsafe {
            loop {
                let ctrl = std::ptr::read(ctrl_ptr.add(index));
                
                if ctrl == CTRL_EMPTY || ctrl == CTRL_DELETED {
                    std::ptr::write(ctrl_ptr.add(index), hash_7bit);
                    return index;
                }
                
                // Check for existing value
                if ctrl == hash_7bit {
                    let slot_value = std::ptr::read(values_ptr.add(index));
                    if slot_value == *value {
                        return index; // Update existing (shouldn't happen for sets)
                    }
                }
                
                index = (index + 1) & (capacity - 1);
            }
        }
    }
    
    fn get_large_mode_pointers_set(&self, hashset: ObjectReference) -> (*mut u8, *mut ValueRef, usize) {
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *mut u8;
            let capacity = {
                let offset = std::mem::size_of::<u32>();
                std::ptr::read_unaligned(header_ptr.add(offset) as *const u32) as usize
            };
            
            let mut offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            offset += SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, bool)>(); // skip inline slots
            
            let ctrl_ptr = header_ptr.add(offset);
            let values_ptr = header_ptr.add(offset + capacity) as *mut ValueRef;
            
            (ctrl_ptr, values_ptr, capacity)
        }
    }
    
    // === CONVENIENCE METHODS ===
    
    pub fn hashset_is_empty(&self, hashset: ObjectReference) -> bool {
        self.hashset_get_length(hashset) == 0
    }
    
    pub fn hashset_clear(&self, hashset: ObjectReference) {
        let _guard = if is_object_shared(hashset) {
            Some(ObjectLockGuard::new(hashset))
        } else {
            None
        };
        
        unsafe {
            let header_ptr = hashset.to_raw_address().as_usize() as *mut u8;
            std::ptr::write_unaligned(header_ptr as *mut u32, 0); // Set length to 0
            
            if self.hashset_is_small_mode(hashset) {
                let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
                let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, bool);
                
                for i in 0..SMALL_MODE_CAPACITY {
                    std::ptr::write(slots_ptr.add(i), (ValueRef::nil(), false));
                }
            } else {
                let (ctrl_ptr, _, capacity) = self.get_large_mode_pointers_set(hashset);
                std::ptr::write_bytes(ctrl_ptr, CTRL_EMPTY, capacity);
            }
        }
    }
    
    /// Convert set to BlinkHashSet for compatibility with existing code
    pub fn hashset_to_blink_hash_set(&self, hashset: ObjectReference) -> BlinkHashSet {
        let values = self.hashset_to_vec(hashset);
        BlinkHashSet::from_iter(values)
    }
    
    /// Mark object as shared when passed to another goroutine
    pub fn mark_hashset_as_shared(&self, obj_ref: ObjectReference) {
        obj_lock(obj_ref);
        obj_unlock(obj_ref);
    }
}