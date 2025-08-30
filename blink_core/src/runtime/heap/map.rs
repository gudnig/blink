use std::hash::{DefaultHasher, Hash, Hasher};
use mmtk::util::ObjectReference;
use crate::runtime::{BlinkActivePlan, BlinkObjectModel, BlinkSlot, BlinkVM, TypeTag, is_object_shared, obj_lock, obj_unlock, ObjectLockGuard};
use crate::value::ValueRef;
use crate::collections::BlinkHashMap;

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
    
    /// Allocate a new hashmap with specified capacity
    /// If capacity <= SMALL_MODE_CAPACITY, creates in small mode
    pub fn alloc_hashmap(&self, pairs: Vec<(ValueRef, ValueRef)>, capacity: Option<usize>) -> ObjectReference {
        let initial_capacity = capacity.unwrap_or_else(|| {
            if pairs.len() <= SMALL_MODE_CAPACITY {
                SMALL_MODE_CAPACITY
            } else {
                (pairs.len() * 2).next_power_of_two().max(8)
            }
        });
        
        self.with_mutator(|mutator| {
            let len = pairs.len();
            
            if len <= SMALL_MODE_CAPACITY && initial_capacity <= SMALL_MODE_CAPACITY {
                // Create in small mode
                self.alloc_small_hashmap(mutator, pairs, initial_capacity)
            } else {
                // Create in large mode
                self.alloc_large_hashmap(mutator, pairs, initial_capacity)
            }
        })
    }
    
    fn alloc_small_hashmap(&self, mutator: &mut mmtk::Mutator<BlinkVM>, pairs: Vec<(ValueRef, ValueRef)>, capacity: usize) -> ObjectReference {
        let header_size = 
            std::mem::size_of::<u32>() +      // len
            std::mem::size_of::<u32>() +      // capacity (0 = small mode)
            std::mem::size_of::<bool>();      // mode flag (true = small)
            
        let inline_slots_size = SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, ValueRef, bool)>(); // (key, value, occupied)
        let total_size = header_size + inline_slots_size;
        
        let object_start = BlinkActivePlan::alloc_object(mutator, &TypeTag::Map, &total_size);
        
        
        unsafe {
            let header_ptr = object_start.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Write header
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, pairs.len() as u32);
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, 0); // capacity = 0 indicates small mode
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut bool, true); // small mode flag
            offset += std::mem::size_of::<bool>();
            
            // Initialize inline slots
            let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, ValueRef, bool);
            for i in 0..SMALL_MODE_CAPACITY {
                if i < pairs.len() {
                    std::ptr::write(slots_ptr.add(i), (pairs[i].0, pairs[i].1, true));
                } else {
                    std::ptr::write(slots_ptr.add(i), (ValueRef::nil(), ValueRef::nil(), false));
                }
            }
        }
        
        object_start
    }
    
    fn alloc_large_hashmap(&self, mutator: &mut mmtk::Mutator<BlinkVM>, pairs: Vec<(ValueRef, ValueRef)>, capacity: usize) -> ObjectReference {
        let capacity = capacity.next_power_of_two().max(8);
        
        let header_size = 
            std::mem::size_of::<u32>() +      // len
            std::mem::size_of::<u32>() +      // capacity
            std::mem::size_of::<bool>() +     // mode flag (false = large)
            (SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, ValueRef, bool)>()); // ignored inline slots
            
        let ctrl_bytes_size = capacity;
        let keys_size = capacity * std::mem::size_of::<ValueRef>();
        let values_size = capacity * std::mem::size_of::<ValueRef>();
        let total_size = header_size + ctrl_bytes_size + keys_size + values_size;
        
        let object_start = BlinkActivePlan::alloc_object(mutator, &TypeTag::Map, &total_size);
        
        
        unsafe {
            let header_ptr = object_start.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Write header
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, pairs.len() as u32);
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut u32, capacity as u32);
            offset += std::mem::size_of::<u32>();
            
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut bool, false); // large mode flag
            offset += std::mem::size_of::<bool>();
            
            // Skip inline slots (they're ignored in large mode)
            offset += SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, ValueRef, bool)>();
            
            // Initialize Swiss table
            let ctrl_ptr = header_ptr.add(offset);
            let keys_ptr = header_ptr.add(offset + ctrl_bytes_size) as *mut ValueRef;
            let values_ptr = header_ptr.add(offset + ctrl_bytes_size + keys_size) as *mut ValueRef;
            
            // Initialize control bytes to EMPTY
            std::ptr::write_bytes(ctrl_ptr, CTRL_EMPTY, capacity);
            
            // Insert pairs using Swiss table logic
            for (key, value) in pairs {
                let hash = self.hash_value(&key);
                let index = self.swiss_insert_slot(ctrl_ptr, capacity, hash, &key, keys_ptr);
                std::ptr::write(keys_ptr.add(index), key);
                std::ptr::write(values_ptr.add(index), value);
            }
        }
        
        object_start
    }
    
    // === ACCESSORS ===
    
    pub fn hashmap_get_length(&self, hashmap: ObjectReference) -> u32 {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *const u8;
            std::ptr::read_unaligned(header_ptr as *const u32)
        }
    }
    
    pub fn hashmap_is_small_mode(&self, hashmap: ObjectReference) -> bool {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *const u8;
            let offset = std::mem::size_of::<u32>() * 2; // skip len and capacity
            std::ptr::read_unaligned(header_ptr.add(offset) as *const bool)
        }
    }
    
    pub fn hashmap_get_capacity(&self, hashmap: ObjectReference) -> u32 {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *const u8;
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
    
    pub fn hashmap_get(&self, hashmap: ObjectReference, key: &ValueRef) -> Option<ValueRef> {
        let _guard = if is_object_shared(hashmap) {
            Some(ObjectLockGuard::new(hashmap))
        } else {
            None
        };
        
        if self.hashmap_is_small_mode(hashmap) {
            self.hashmap_get_small(hashmap, key)
        } else {
            self.hashmap_get_large(hashmap, key)
        }
    }
    
    fn hashmap_get_small(&self, hashmap: ObjectReference, key: &ValueRef) -> Option<ValueRef> {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *const u8;
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *const (ValueRef, ValueRef, bool);
            
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_key, slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied && slot_key == *key {
                    return Some(slot_value);
                }
            }
            None
        }
    }
    
    fn hashmap_get_large(&self, hashmap: ObjectReference, key: &ValueRef) -> Option<ValueRef> {
        unsafe {
            let (ctrl_ptr, keys_ptr, values_ptr, capacity) = self.get_large_mode_pointers(hashmap);
            let hash = self.hash_value(key);
            
            if let Some(index) = self.swiss_find_slot(ctrl_ptr, keys_ptr, capacity, hash, key) {
                Some(std::ptr::read(values_ptr.add(index)))
            } else {
                None
            }
        }
    }
    
    // === INSERTION ===
    
    pub fn hashmap_insert(&self, hashmap: ObjectReference, key: ValueRef, value: ValueRef) -> Result<Option<ValueRef>, String> {
        let _guard = if is_object_shared(hashmap) {
            Some(ObjectLockGuard::new(hashmap))
        } else {
            None
        };
        
        if self.hashmap_is_small_mode(hashmap) {
            let old_value = self.hashmap_insert_small(hashmap, key, value)?;
            
            // Check if we need to promote to large mode
            let len = self.hashmap_get_length(hashmap);
            if len as usize > SMALL_MODE_CAPACITY {
                self.promote_to_large_mode(hashmap)?;
            }
            
            Ok(old_value)
        } else {
            self.hashmap_insert_large(hashmap, key, value)
        }
    }
    
    fn hashmap_insert_small(&self, hashmap: ObjectReference, key: ValueRef, value: ValueRef) -> Result<Option<ValueRef>, String> {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *mut u8;
            let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
            
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, ValueRef, bool);
            
            // Check if key already exists
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_key, slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied && slot_key == key {
                    std::ptr::write(slots_ptr.add(i), (key, value, true));
                    return Ok(Some(slot_value));
                }
            }
            
            // Find empty slot
            for i in 0..SMALL_MODE_CAPACITY {
                let (_, _, occupied) = std::ptr::read(slots_ptr.add(i));
                if !occupied {
                    std::ptr::write(slots_ptr.add(i), (key, value, true));
                    len += 1;
                    std::ptr::write_unaligned(header_ptr as *mut u32, len);
                    return Ok(None);
                }
            }
            
            Err("Small hashmap is full".to_string())
        }
    }
    
    fn hashmap_insert_large(&self, hashmap: ObjectReference, key: ValueRef, value: ValueRef) -> Result<Option<ValueRef>, String> {
        // Check if resize is needed first
        let len = self.hashmap_get_length(hashmap) as usize;
        let capacity = self.hashmap_get_capacity(hashmap) as usize;
        
        if len * MAX_LOAD_FACTOR_DENOMINATOR >= capacity * MAX_LOAD_FACTOR_NUMERATOR {
            self.resize_large_hashmap(hashmap)?;
        }
        
        unsafe {
            let (ctrl_ptr, keys_ptr, values_ptr, capacity) = self.get_large_mode_pointers(hashmap);
            let hash = self.hash_value(&key);
            
            // Check if key exists
            if let Some(index) = self.swiss_find_slot(ctrl_ptr, keys_ptr, capacity, hash, &key) {
                let old_value = std::ptr::read(values_ptr.add(index));
                std::ptr::write(values_ptr.add(index), value);
                return Ok(Some(old_value));
            }
            
            // Insert new key
            let index = self.swiss_insert_slot(ctrl_ptr, capacity, hash, &key, keys_ptr);
            std::ptr::write(keys_ptr.add(index), key);
            std::ptr::write(values_ptr.add(index), value);
            
            // Update length
            let header_ptr = hashmap.to_raw_address().as_usize() as *mut u8;
            let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
            len += 1;
            std::ptr::write_unaligned(header_ptr as *mut u32, len);
            
            Ok(None)
        }
    }
    
    // === REMOVAL ===
    
    pub fn hashmap_remove(&self, hashmap: ObjectReference, key: &ValueRef) -> Option<ValueRef> {
        let _guard = if is_object_shared(hashmap) {
            Some(ObjectLockGuard::new(hashmap))
        } else {
            None
        };
        
        if self.hashmap_is_small_mode(hashmap) {
            self.hashmap_remove_small(hashmap, key)
        } else {
            self.hashmap_remove_large(hashmap, key)
        }
    }
    
    fn hashmap_remove_small(&self, hashmap: ObjectReference, key: &ValueRef) -> Option<ValueRef> {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *mut u8;
            let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, ValueRef, bool);
            
            for i in 0..SMALL_MODE_CAPACITY {
                let (slot_key, slot_value, occupied) = std::ptr::read(slots_ptr.add(i));
                if occupied && slot_key == *key {
                    std::ptr::write(slots_ptr.add(i), (ValueRef::nil(), ValueRef::nil(), false));
                    
                    // Update length
                    let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
                    len -= 1;
                    std::ptr::write_unaligned(header_ptr as *mut u32, len);
                    
                    return Some(slot_value);
                }
            }
            None
        }
    }
    
    fn hashmap_remove_large(&self, hashmap: ObjectReference, key: &ValueRef) -> Option<ValueRef> {
        unsafe {
            let (ctrl_ptr, keys_ptr, values_ptr, capacity) = self.get_large_mode_pointers(hashmap);
            let hash = self.hash_value(key);
            
            if let Some(index) = self.swiss_find_slot(ctrl_ptr, keys_ptr, capacity, hash, key) {
                let old_value = std::ptr::read(values_ptr.add(index));
                
                // Mark as deleted
                std::ptr::write(ctrl_ptr.add(index), CTRL_DELETED);
                
                // Update length
                let header_ptr = hashmap.to_raw_address().as_usize() as *mut u8;
                let mut len = std::ptr::read_unaligned(header_ptr as *const u32);
                len -= 1;
                std::ptr::write_unaligned(header_ptr as *mut u32, len);
                
                Some(old_value)
            } else {
                None
            }
        }
    }
    
    // === MODE PROMOTION ===
    
    fn promote_to_large_mode(&self, hashmap: ObjectReference) -> Result<(), String> {
        self.with_mutator(|mutator| {
            // Extract current pairs from small mode
            let mut pairs = Vec::new();
            unsafe {
                let header_ptr = hashmap.to_raw_address().as_usize() as *const u8;
                let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
                let slots_ptr = header_ptr.add(offset) as *const (ValueRef, ValueRef, bool);
                
                for i in 0..SMALL_MODE_CAPACITY {
                    let (key, value, occupied) = std::ptr::read(slots_ptr.add(i));
                    if occupied {
                        pairs.push((key, value));
                    }
                }
            }
            
            // Allocate new large mode hashmap
            let new_capacity = 8; // Start with minimum large capacity
            let new_hashmap = self.alloc_large_hashmap(mutator, pairs, new_capacity);
            
            // Copy the new object content over the old object
            // This is a complex operation that requires careful memory management
            // For now, we'll return an error and require external reallocation
            Err("Promotion requires external reallocation".to_string())
        })
    }
    
    // === RESIZE ===
    
    fn resize_large_hashmap(&self, hashmap: ObjectReference) -> Result<(), String> {
        // Similar to promotion, this requires allocating a new object
        // and copying data, which is complex in this GC context
        Err("Resize requires external reallocation".to_string())
    }
    
    // === SWISS TABLE HELPERS ===
    
    pub fn hash_value(&self, value: &ValueRef) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
    
    fn swiss_find_slot(&self, ctrl_ptr: *const u8, keys_ptr: *const ValueRef, capacity: usize, hash: u64, key: &ValueRef) -> Option<usize> {
        let hash_7bit = (hash & 0x7F) as u8;
        let mut index = (hash as usize) & (capacity - 1);
        
        unsafe {
            loop {
                let ctrl = std::ptr::read(ctrl_ptr.add(index));
                
                if ctrl == CTRL_EMPTY {
                    return None; // Key not found
                }
                
                if ctrl == hash_7bit {
                    let slot_key = std::ptr::read(keys_ptr.add(index));
                    if slot_key == *key {
                        return Some(index);
                    }
                }
                
                index = (index + 1) & (capacity - 1);
            }
        }
    }
    
    fn swiss_insert_slot(&self, ctrl_ptr: *mut u8, capacity: usize, hash: u64, key: &ValueRef, keys_ptr: *const ValueRef) -> usize {
        let hash_7bit = (hash & 0x7F) as u8;
        let mut index = (hash as usize) & (capacity - 1);
        
        unsafe {
            loop {
                let ctrl = std::ptr::read(ctrl_ptr.add(index));
                
                if ctrl == CTRL_EMPTY || ctrl == CTRL_DELETED {
                    std::ptr::write(ctrl_ptr.add(index), hash_7bit);
                    return index;
                }
                
                // Check for existing key
                if ctrl == hash_7bit {
                    let slot_key = std::ptr::read(keys_ptr.add(index));
                    if slot_key == *key {
                        return index; // Update existing
                    }
                }
                
                index = (index + 1) & (capacity - 1);
            }
        }
    }
    
    fn get_large_mode_pointers(&self, hashmap: ObjectReference) -> (*mut u8, *mut ValueRef, *mut ValueRef, usize) {
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *mut u8;
            let capacity = {
                let offset = std::mem::size_of::<u32>();
                std::ptr::read_unaligned(header_ptr.add(offset) as *const u32) as usize
            };
            
            let mut offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
            offset += SMALL_MODE_CAPACITY * std::mem::size_of::<(ValueRef, ValueRef, bool)>(); // skip inline slots
            
            let ctrl_ptr = header_ptr.add(offset);
            let keys_ptr = header_ptr.add(offset + capacity) as *mut ValueRef;
            let values_ptr = header_ptr.add(offset + capacity + capacity * std::mem::size_of::<ValueRef>()) as *mut ValueRef;
            
            (ctrl_ptr, keys_ptr, values_ptr, capacity)
        }
    }
    
    // === CONVENIENCE METHODS ===
    
    pub fn hashmap_is_empty(&self, hashmap: ObjectReference) -> bool {
        self.hashmap_get_length(hashmap) == 0
    }
    
    pub fn hashmap_clear(&self, hashmap: ObjectReference) {
        let _guard = if is_object_shared(hashmap) {
            Some(ObjectLockGuard::new(hashmap))
        } else {
            None
        };
        
        unsafe {
            let header_ptr = hashmap.to_raw_address().as_usize() as *mut u8;
            std::ptr::write_unaligned(header_ptr as *mut u32, 0); // Set length to 0
            
            if self.hashmap_is_small_mode(hashmap) {
                let offset = std::mem::size_of::<u32>() * 2 + std::mem::size_of::<bool>();
                let slots_ptr = header_ptr.add(offset) as *mut (ValueRef, ValueRef, bool);
                
                for i in 0..SMALL_MODE_CAPACITY {
                    std::ptr::write(slots_ptr.add(i), (ValueRef::nil(), ValueRef::nil(), false));
                }
            } else {
                let (ctrl_ptr, _, _, capacity) = self.get_large_mode_pointers(hashmap);
                std::ptr::write_bytes(ctrl_ptr, CTRL_EMPTY, capacity);
            }
        }
    }
    
    /// Mark object as shared when passed to another goroutine
    pub fn mark_hashmap_as_shared(&self, obj_ref: ObjectReference) {
        obj_lock(obj_ref);
        obj_unlock(obj_ref);
    }
}