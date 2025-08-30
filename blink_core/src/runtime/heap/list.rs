// blink_core/src/runtime/heap/list.rs

use mmtk::util::{Address, ObjectReference};
use crate::runtime::{BlinkActivePlan, BlinkSlot, TypeTag};
use crate::value::ValueRef;
use crate::runtime::{is_object_shared, ObjectLockGuard};

impl crate::runtime::BlinkVM {
    
    /// Allocate a new empty list
    pub fn alloc_list(&self) -> ObjectReference {
        self.with_mutator(|mutator| {
            // List header: [length: usize][flags: usize][head: ObjectReference][tail: ObjectReference]
            // flags: bit 0 = has_head, bit 1 = has_tail
            let header_size = 
                std::mem::size_of::<usize>() +                   // length
                std::mem::size_of::<usize>() +                   // flags (has_head, has_tail)
                std::mem::size_of::<ObjectReference>() +         // head
                std::mem::size_of::<ObjectReference>();          // tail
            
            let list_ref = BlinkActivePlan::alloc_object(mutator, &TypeTag::List, &header_size);
            // No need for initialize_object_header - happens in ObjectHeader::new
            
            unsafe {
                let header_ptr = list_ref.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Initialize length to 0 (now at offset 0)
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut usize, 0);
                offset += std::mem::size_of::<usize>();
                
                // Initialize flags to 0 (no head, no tail)
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut usize, 0);
                offset += std::mem::size_of::<usize>();
                
                // Initialize head to a dummy value (will be ignored when has_head is 0)
                // We'll use the list itself as a sentinel since it's guaranteed to be valid
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut ObjectReference, list_ref);
                offset += std::mem::size_of::<ObjectReference>();
                
                // Initialize tail to a dummy value (will be ignored when has_tail is 0)
                std::ptr::write_unaligned(header_ptr.add(offset) as *mut ObjectReference, list_ref);
            }
            
            list_ref
        })
    }

    /// Allocate a list from a vector of items
    pub fn alloc_list_from_items(&self, items: Vec<ValueRef>) -> ObjectReference {
        let list = self.alloc_list();
        
        // Add items in reverse order to maintain order when prepending
        for item in items.into_iter().rev() {
            let _ = self.list_prepend(list, item);
        }
        
        list
    }
    
    /// Allocate a new list node  
    fn alloc_list_node(&self, value: ValueRef, next: ObjectReference, has_next: bool) -> ObjectReference {
        self.with_mutator(|mutator| {
            // Node: [flags: usize][value: ValueRef][next: ObjectReference]
            // flags: bit 0 = has_next
            let node_size = 
                std::mem::size_of::<usize>() +                   // flags (has_next)
                std::mem::size_of::<ValueRef>() +                // value
                std::mem::size_of::<ObjectReference>();          // next
            
            let node_ref = BlinkActivePlan::alloc_object(mutator, &TypeTag::ListNode, &node_size);
            // No need for initialize_object_header - happens in ObjectHeader::new
            
            unsafe {
                let node_ptr = node_ref.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write flags
                let flags = if has_next { 1 } else { 0 };
                std::ptr::write_unaligned(node_ptr.add(offset) as *mut usize, flags);
                offset += std::mem::size_of::<usize>();
                
                // Write value
                std::ptr::write_unaligned(node_ptr.add(offset) as *mut ValueRef, value);
                offset += std::mem::size_of::<ValueRef>();
                
                // Write next pointer
                std::ptr::write_unaligned(node_ptr.add(offset) as *mut ObjectReference, next);
                
                // Set up write barriers only if has_next is true
                if has_next {
                    let next_slot_addr = Address::from_ptr(node_ptr.add(offset) as *const ObjectReference);
                    mutator.barrier.object_reference_write(
                        node_ref,
                        BlinkSlot::ObjectRef(next_slot_addr),
                        next
                    );
                }
                
                // Value barrier if needed
                if let ValueRef::Heap(gc_ptr) = value {
                    let value_slot_addr = Address::from_ptr(node_ptr.add(std::mem::size_of::<usize>()) as *const ValueRef);
                    mutator.barrier.object_reference_write(
                        node_ref,
                        BlinkSlot::ValueRef(value_slot_addr),
                        gc_ptr.0
                    );
                }
            }
            
            node_ref
        })
    }
    
    /// Read list header components
    unsafe fn read_list_header(&self, list: ObjectReference) -> (bool, ObjectReference, bool, ObjectReference, usize) {
        let header_ptr = list.to_raw_address().as_usize() as *const u8;
        let mut offset = 0;
        
        let length = std::ptr::read_unaligned(header_ptr.add(offset) as *const usize);
        offset += std::mem::size_of::<usize>();
        
        let flags = std::ptr::read_unaligned(header_ptr.add(offset) as *const usize);
        let has_head = (flags & 1) != 0;
        let has_tail = (flags & 2) != 0;
        offset += std::mem::size_of::<usize>();
        
        let head = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
        offset += std::mem::size_of::<ObjectReference>();
        
        let tail = std::ptr::read_unaligned(header_ptr.add(offset) as *const ObjectReference);
        
        (has_head, head, has_tail, tail, length)
    }
    
    /// Update list header
    unsafe fn update_list_header(&self, list: ObjectReference, has_head: bool, head: ObjectReference, has_tail: bool, tail: ObjectReference, length: usize) {
        self.with_mutator(|mutator| {
            let header_ptr = list.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Update length (now at offset 0)
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut usize, length);
            offset += std::mem::size_of::<usize>();
            
            // Update flags
            let mut flags = 0;
            if has_head { flags |= 1; }
            if has_tail { flags |= 2; }
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut usize, flags);
            offset += std::mem::size_of::<usize>();
            
            // Update head
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut ObjectReference, head);
            if has_head {
                let head_slot_addr = Address::from_ptr(header_ptr.add(offset) as *const ObjectReference);
                mutator.barrier.object_reference_write(
                    list,
                    BlinkSlot::ObjectRef(head_slot_addr),
                    head
                );
            }
            offset += std::mem::size_of::<ObjectReference>();
            
            // Update tail
            std::ptr::write_unaligned(header_ptr.add(offset) as *mut ObjectReference, tail);
            if has_tail {
                let tail_slot_addr = Address::from_ptr(header_ptr.add(offset) as *const ObjectReference);
                mutator.barrier.object_reference_write(
                    list,
                    BlinkSlot::ObjectRef(tail_slot_addr),
                    tail
                );
            }
        });
    }
    
    /// Read list node components
    unsafe fn read_node(&self, node: ObjectReference) -> (ValueRef, bool, ObjectReference) {
        let node_ptr = node.to_raw_address().as_usize() as *const u8;
        let mut offset = 0;
        
        let flags = std::ptr::read_unaligned(node_ptr.add(offset) as *const usize);
        let has_next = (flags & 1) != 0;
        offset += std::mem::size_of::<usize>();
        
        let value = std::ptr::read_unaligned(node_ptr.add(offset) as *const ValueRef);
        offset += std::mem::size_of::<ValueRef>();
        
        let next = std::ptr::read_unaligned(node_ptr.add(offset) as *const ObjectReference);
        
        (value, has_next, next)
    }
    
    /// Update node's next pointer
    unsafe fn update_node_next(&self, node: ObjectReference, has_next: bool, new_next: ObjectReference) {
        self.with_mutator(|mutator| {
            let node_ptr = node.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Update flags
            let flags = if has_next { 1 } else { 0 };
            std::ptr::write_unaligned(node_ptr.add(offset) as *mut usize, flags);
            offset += std::mem::size_of::<usize>();
            
            // Skip value
            offset += std::mem::size_of::<ValueRef>();
            
            // Update next pointer
            std::ptr::write_unaligned(node_ptr.add(offset) as *mut ObjectReference, new_next);
            
            // Write barrier only if has_next is true
            if has_next {
                let next_slot_addr = Address::from_ptr(node_ptr.add(offset) as *const ObjectReference);
                mutator.barrier.object_reference_write(
                    node,
                    BlinkSlot::ObjectRef(next_slot_addr),
                    new_next
                );
            }
        });
    }
    
    /// Get the length of a list
    pub fn list_length(&self, list: ObjectReference) -> usize {
        unsafe {
            let (_, _, _, _, length) = self.read_list_header(list);
            length
        }
    }
    
    /// Check if list is empty
    pub fn list_is_empty(&self, list: ObjectReference) -> bool {
        self.list_length(list) == 0
    }
    
    /// Prepend an element to the front of the list (cons operation) - O(1)
    pub fn list_prepend(&self, list: ObjectReference, value: ValueRef) -> Result<(), String> {
        let _guard = if is_object_shared(list) {
            Some(ObjectLockGuard::new(list))
        } else {
            None
        };
        
        unsafe {
            let (has_head, head, has_tail, tail, length) = self.read_list_header(list);
            
            let new_node = self.alloc_list_node(value, head, has_head);
            
            if !has_head {
                // List was empty, new node is both head and tail
                self.update_list_header(list, true, new_node, true, new_node, 1);
            } else {
                // Update list header with new head
                self.update_list_header(list, true, new_node, has_tail, tail, length + 1);
            }
        }
        
        Ok(())
    }
    
    /// Append an element to the end of the list - O(1) with tail pointer
    pub fn list_append(&self, list: ObjectReference, value: ValueRef) -> Result<(), String> {
        let _guard = if is_object_shared(list) {
            Some(ObjectLockGuard::new(list))
        } else {
            None
        };
        
        unsafe {
            let (has_head, head, has_tail, tail, length) = self.read_list_header(list);
            
            let new_node = self.alloc_list_node(value, list, false); // Use list as sentinel
            
            if !has_tail {
                // List was empty, new node is both head and tail
                self.update_list_header(list, true, new_node, true, new_node, 1);
            } else {
                // Update old tail's next pointer to point to new node
                self.update_node_next(tail, true, new_node);
                // Update list header with new tail
                self.update_list_header(list, has_head, head, true, new_node, length + 1);
            }
        }
        
        Ok(())
    }
    
    /// Get the first element of the list - O(1)
    pub fn list_first(&self, list: ObjectReference) -> Result<ValueRef, String> {
        unsafe {
            let (has_head, head, _, _, _) = self.read_list_header(list);
            
            if has_head {
                let (value, _, _) = self.read_node(head);
                Ok(value)
            } else {
                Ok(ValueRef::nil())
            }
        }
    }
    
    /// Get the last element of the list - O(1) with tail pointer
    pub fn list_last(&self, list: ObjectReference) -> Result<ValueRef, String> {
        unsafe {
            let (_, _, has_tail, tail, _) = self.read_list_header(list);
            
            if has_tail {
                let (value, _, _) = self.read_node(tail);
                Ok(value)
            } else {
                Ok(ValueRef::nil())
            }
        }
    }
    
    /// Remove and return the first element - O(1)
    pub fn list_pop_front(&self, list: ObjectReference) -> Result<ValueRef, String> {
        let _guard = if is_object_shared(list) {
            Some(ObjectLockGuard::new(list))
        } else {
            None
        };
        
        unsafe {
            let (has_head, head, has_tail, tail, length) = self.read_list_header(list);
            
            if has_head {
                let (value, has_next, next) = self.read_node(head);
                
                if !has_next {
                    // List becomes empty
                    self.update_list_header(list, false, list, false, list, 0);
                } else {
                    // Update head to next node, tail stays the same
                    self.update_list_header(list, true, next, has_tail, tail, length - 1);
                }
                
                Ok(value)
            } else {
                Err("Cannot pop from empty list".to_string())
            }
        }
    }
    
    /// Remove and return the last element - O(n) (we traverse to find penultimate)
    pub fn list_pop_back(&self, list: ObjectReference) -> Result<ValueRef, String> {
        let _guard = if is_object_shared(list) {
            Some(ObjectLockGuard::new(list))
        } else {
            None
        };
        
        unsafe {
            let (has_head, head, has_tail, tail, length) = self.read_list_header(list);
            
            if has_tail {
                let (value, _, _) = self.read_node(tail);
                
                if head == tail {
                    // Single element list becomes empty
                    self.update_list_header(list, false, list, false, list, 0);
                } else {
                    // Need to find the penultimate node - O(n) traversal
                    let mut current = head;
                    let mut prev: Option<ObjectReference> = None;
                    
                    while current != tail {
                        let (_, _, next) = self.read_node(current);
                        prev = Some(current);
                        current = next;
                    }
                    
                    // Update penultimate node to point to nothing
                    if let Some(prev_node) = prev {
                        self.update_node_next(prev_node, false, list);
                        self.update_list_header(list, has_head, head, true, prev_node, length - 1);
                    }
                }
                
                Ok(value)
            } else {
                Err("Cannot pop from empty list".to_string())
            }
        }
    }
    
    /// Get the rest of the list (all elements except the first) as a new list - O(1) structure sharing
    pub fn list_rest(&self, list: ObjectReference) -> Result<ObjectReference, String> {
        unsafe {
            let (has_head, head, has_tail, tail, length) = self.read_list_header(list);
            
            if has_head {
                let (_, has_next, next) = self.read_node(head);
                
                if !has_next {
                    // Single element list, return empty list
                    Ok(self.alloc_list())
                } else {
                    // Validate that next is a valid object reference
                    if next.to_raw_address().as_usize() == 0 {
                        return Err("Invalid next pointer: null reference".to_string());
                    }
                    
                    // Create new list that shares structure starting from next
                    let new_list = self.alloc_list();
                    self.update_list_header(new_list, true, next, has_tail, tail, length - 1);
                    Ok(new_list)
                }
            } else {
                // Empty list, return empty list
                Ok(self.alloc_list())
            }
        }
    }
    
    /// Convert list to vector for easier iteration
    pub fn list_to_vec(&self, list: ObjectReference) -> Vec<ValueRef> {
        let mut result = Vec::new();
        
        unsafe {
            let (has_head, head, _, _, _) = self.read_list_header(list);
            if !has_head {
                return result;
            }
            
            let mut current = head;
            loop {
                let (value, has_next, next) = self.read_node(current);
                result.push(value);
                if !has_next {
                    break;
                }
                current = next;
            }
        }
        
        result
    }
    
    /// Convert list to vector for macro expansion compatibility
    pub fn list_to_vec_for_macro(&self, list: ObjectReference) -> Vec<ValueRef> {
        self.list_to_vec(list)
    }
    
    /// Get nth element - O(n)
    pub fn list_nth(&self, list: ObjectReference, index: usize) -> Result<ValueRef, String> {
        unsafe {
            let (has_head, head, _, _, length) = self.read_list_header(list);
            
            if !has_head || index >= length {
                return Err(format!("Index {} out of bounds for list of length {}", index, length));
            }
            
            let mut current = head;
            for _ in 0..index {
                let (_, has_next, next) = self.read_node(current);
                if !has_next {
                    return Err("Unexpected end of list".to_string());
                }
                current = next;
            }
            
            let (value, _, _) = self.read_node(current);
            Ok(value)
        }
    }
    
    /// Clear all elements from the list
    pub fn list_clear(&self, list: ObjectReference) {
        let _guard = if is_object_shared(list) {
            Some(ObjectLockGuard::new(list))
        } else {
            None
        };
        
        unsafe {
            self.update_list_header(list, false, list, false, list, 0);
        }
    }
}