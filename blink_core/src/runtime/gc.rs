use crate::error::BlinkError;
use crate::runtime::mmtk::ObjectHeader;
use crate::runtime::TypeTag;
use crate::value::Callable;
use crate::{BlinkHashMap, BlinkHashSet};
use crate::{runtime::BlinkVM, ValueRef};
use mmtk::util::{Address, OpaquePointer, VMThread};
use mmtk::MutatorContext;
use mmtk::{util::ObjectReference, Mutator, util::VMMutatorThread, memory_manager};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::hash::{DefaultHasher, Hash, Hasher};
use crate::value::HeapValue;

thread_local! {
    static MUTATOR: RefCell<Option<Box<Mutator<BlinkVM>>>> = RefCell::new(None);
    static THREAD_TLS: RefCell<Option<VMMutatorThread>> = RefCell::new(None);
}
static THREAD_IDS: OnceLock<Mutex<HashMap<std::thread::ThreadId, usize>>> = OnceLock::new();
static COUNTER: AtomicUsize = AtomicUsize::new(1);

impl BlinkVM {
    pub fn with_mutator<T>(&self, f: impl FnOnce(&mut Mutator<BlinkVM>) -> T) -> T {
        MUTATOR.with(|m| {
            let mut mutator_ref = m.borrow_mut();
            if mutator_ref.is_none() {
                // Create VMMutatorThread using current thread as identifier
                let tls = Self::create_vm_mutator_thread();
                
                // Get static reference to MMTK (required by bind_mutator)
                let static_mmtk = self.get_static_mmtk();
                
                // CORRECT: Bind mutator with VMMutatorThread
                let mutator = mmtk::memory_manager::bind_mutator(static_mmtk, tls);
                *mutator_ref = Some(mutator);
            }

            let mutator = mutator_ref.as_mut().unwrap();
            f(mutator.as_mut())
        })
    }
    

    fn create_vm_mutator_thread() -> VMMutatorThread {
        let thread_id = std::thread::current().id();
        let map = THREAD_IDS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut map = map.lock();
        
        let unique_id = *map.entry(thread_id).or_insert_with(|| {
            COUNTER.fetch_add(1, Ordering::Relaxed)
        });
        
        let address = unsafe { Address::from_usize(unique_id) };
        let opaque = OpaquePointer::from_address(address);
        
        let vm_thread = VMThread(opaque);
        VMMutatorThread(vm_thread)
    }
    
    
    // Handle the static lifetime requirement for MMTK
    fn get_static_mmtk(&self) -> &'static mmtk::MMTK<BlinkVM> {
        // UNSAFE: Required because bind_mutator expects 'static
        // This is safe as long as your VM instance lives for the program duration
        unsafe {
            std::mem::transmute::<&mmtk::MMTK<BlinkVM>, &'static mmtk::MMTK<BlinkVM>>(&*self.mmtk)
        }
    }
    
    // Explicit initialization for worker threads
    pub fn init_mutator_for_thread(&self) -> Result<(), String> {
        MUTATOR.with(|m| {
            let mut mutator_ref = m.borrow_mut();
            if mutator_ref.is_some() {
                return Err("Mutator already initialized for this thread".to_string());
            }
            
            let tls = Self::create_vm_mutator_thread();
            let static_mmtk = self.get_static_mmtk();
            
            let mutator = memory_manager::bind_mutator(static_mmtk, tls);
            *mutator_ref = Some(mutator);
            
            println!("Initialized mutator for thread: {:?}", tls);
            Ok(())
        })
    }
    
    pub fn destroy_mutator_for_thread(&self) {
        MUTATOR.with(|m| {
            let mut mutator_ref = m.borrow_mut();
            if let Some(mutator) = mutator_ref.take() {
                // Clean up the mutator
                drop(mutator);
                println!("Destroyed mutator for current thread");
            }
        });
    }
    
    pub fn has_thread_local_mutator(&self) -> bool {
        MUTATOR.with(|m| m.borrow().is_some())
    }

    pub fn alloc_vec(&self, items: Vec<ValueRef>) -> ObjectReference {
        self.with_mutator(|mutator| {
            let vec_header_size = std::mem::size_of::<usize>() * 2; // len + capacity
            let vec_data_size = items.len() * std::mem::size_of::<ValueRef>();
            let total_data_size = vec_header_size + vec_data_size;
            let total_size = std::mem::size_of::<ObjectHeader>() + total_data_size;

            let address = mutator.alloc(total_size, 8, 0, mmtk::AllocationSemantics::Default);

            unsafe {
                // Convert Address to raw pointer
                let base_ptr = address.as_usize() as *mut u8;

                // Write header
                let header_ptr = base_ptr as *mut ObjectHeader;
                std::ptr::write(
                    header_ptr,
                    ObjectHeader::new(TypeTag::List, total_data_size),
                );

                // Write vector metadata (length and capacity)
                let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
                let len_ptr = data_start as *mut usize;
                std::ptr::write(len_ptr, items.len());

                let cap_ptr = data_start.add(std::mem::size_of::<usize>()) as *mut usize;
                std::ptr::write(cap_ptr, items.len()); // capacity = length for now

                // Write vector data
                let vec_data_ptr =
                    data_start.add(std::mem::size_of::<usize>() * 2) as *mut ValueRef;

                for (i, item) in items.into_iter().enumerate() {
                    std::ptr::write(vec_data_ptr.add(i), item);
                }
            }

            ObjectReference::from_raw_address(address).unwrap()
        })
    }


    pub fn alloc_user_defined_fn(&self, function: Callable) -> ObjectReference {
        todo!()
    }

    pub fn alloc_macro(&self, mac: Callable) -> ObjectReference {
        todo!()
    }
    
    

    pub fn alloc_str(&self, s: &str) -> ObjectReference {

        //TODO String interning
        self.with_mutator(|mutator| {
            // For strings, we might want to store the string data inline
            let string_bytes = s.as_bytes();
            let data_size = string_bytes.len() + std::mem::size_of::<u32>(); // length + data
            let total_size = std::mem::size_of::<ObjectHeader>() + data_size;

            let address = mutator.alloc(total_size, 8, 0, mmtk::AllocationSemantics::Default);

            unsafe {
                // Convert Address to raw pointer
                let base_ptr = address.as_usize() as *mut u8;

                // Write header
                let header_ptr = base_ptr as *mut ObjectHeader;
                std::ptr::write(header_ptr, ObjectHeader::new(TypeTag::Str, data_size));

                // Write string length
                let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
                let len_ptr = data_start as *mut u32;
                std::ptr::write(len_ptr, string_bytes.len() as u32);

                // Write string data
                let str_data_ptr = data_start.add(std::mem::size_of::<u32>()) as *mut u8;
                std::ptr::copy_nonoverlapping(
                    string_bytes.as_ptr(),
                    str_data_ptr,
                    string_bytes.len(),
                );
            }

            ObjectReference::from_raw_address(address).unwrap()
        })
    }

    pub fn alloc_blink_hash_map(&self, map: BlinkHashMap) -> ObjectReference {
        let pairs: Vec<(&ValueRef, &ValueRef)> = map.iter().collect();
        self.alloc_map(pairs)
    }

    

    pub fn alloc_map(&self, pairs: Vec<(&ValueRef, &ValueRef)>) -> ObjectReference {
        self.with_mutator(|mutator| {
            let bucket_count = Self::calculate_bucket_count(pairs.len());
            let item_count = pairs.len();
            
            let header_size = std::mem::size_of::<usize>() * 2; // bucket_count + item_count
            let buckets_size = bucket_count * std::mem::size_of::<u32>(); // bucket start indices
            let pairs_size = item_count * 2 * std::mem::size_of::<ValueRef>();
            let total_data_size = header_size + buckets_size + pairs_size;
            
            let address = mutator.alloc(
                std::mem::size_of::<ObjectHeader>() + total_data_size,
                8, 0, mmtk::AllocationSemantics::Default
            );
            
            unsafe {
                let base_ptr = address.as_usize() as *mut u8;
                let header_ptr = base_ptr as *mut ObjectHeader;
                std::ptr::write(header_ptr, ObjectHeader::new(TypeTag::Map, total_data_size));
                
                let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
                
                // Write metadata
                let bucket_count_ptr = data_start as *mut usize;
                std::ptr::write(bucket_count_ptr, bucket_count);
                let item_count_ptr = data_start.add(std::mem::size_of::<usize>()) as *mut usize;
                std::ptr::write(item_count_ptr, item_count);
                
                // Organize into buckets

                let mut hasher = DefaultHasher::new();
                let mut buckets: Vec<Vec<(ValueRef, ValueRef)>> = vec![Vec::new(); bucket_count];
                for (key, val) in pairs {
                    let mut hasher = DefaultHasher::new();
                    key.hash(&mut hasher);
                    let hash_value = hasher.finish();
                    let bucket = (hash_value as usize) % bucket_count;
                    buckets[bucket].push((*key, *val));
                }
                
                // Write bucket offsets
                let bucket_offsets_ptr = data_start.add(header_size) as *mut u32;
                let mut current_offset = 0u32;
                for (i, bucket) in buckets.iter().enumerate() {
                    std::ptr::write(bucket_offsets_ptr.add(i), current_offset);
                    current_offset += bucket.len() as u32;
                }
                
                // Write pairs
                let pairs_ptr = data_start.add(header_size + buckets_size) as *mut ValueRef;
                let mut pair_index = 0;
                for bucket in buckets {
                    for (key, val) in bucket {
                        std::ptr::write(pairs_ptr.add(pair_index * 2), key);
                        std::ptr::write(pairs_ptr.add(pair_index * 2 + 1), val);
                        pair_index += 1;
                    }
                }
            }
            
            ObjectReference::from_raw_address(address).unwrap()
        })
    }

    pub fn alloc_blink_hash_set(&self, set: BlinkHashSet) -> ObjectReference {
        let items: Vec<&ValueRef> = set.iter().collect();
        self.alloc_set(items)
    }
    
    pub fn alloc_set(&self, items: Vec<&ValueRef>) -> ObjectReference {
        self.with_mutator(|mutator| {
            let bucket_count = Self::calculate_bucket_count(items.len());
            let item_count = items.len();
            
            let header_size = std::mem::size_of::<usize>() * 2;
            let buckets_size = bucket_count * std::mem::size_of::<u32>();
            let items_size = item_count * std::mem::size_of::<ValueRef>();
            let total_data_size = header_size + buckets_size + items_size;
            
            let address = mutator.alloc(
                std::mem::size_of::<ObjectHeader>() + total_data_size,
                8, 0, mmtk::AllocationSemantics::Default
            );
            
            unsafe {
                let base_ptr = address.as_usize() as *mut u8;
                let header_ptr = base_ptr as *mut ObjectHeader;
                std::ptr::write(header_ptr, ObjectHeader::new(TypeTag::Set, total_data_size));
                
                let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
                
                // Write metadata
                let bucket_count_ptr = data_start as *mut usize;
                std::ptr::write(bucket_count_ptr, bucket_count);
                let item_count_ptr = data_start.add(std::mem::size_of::<usize>()) as *mut usize;
                std::ptr::write(item_count_ptr, item_count);
                
                // Organize into buckets
                let mut buckets: Vec<Vec<ValueRef>> = vec![Vec::new(); bucket_count];
                for item in items {
                    let mut hasher = DefaultHasher::new();
                    item.hash(&mut hasher);
                    let hash_value = hasher.finish();
                    let bucket = (hash_value as usize) % bucket_count;
                    buckets[bucket].push(*item);
                }
                
                // Write bucket offsets
                let bucket_offsets_ptr = data_start.add(header_size) as *mut u32;
                let mut current_offset = 0u32;
                for (i, bucket) in buckets.iter().enumerate() {
                    std::ptr::write(bucket_offsets_ptr.add(i), current_offset);
                    current_offset += bucket.len() as u32;
                }
                
                // Write items
                let items_ptr = data_start.add(header_size + buckets_size) as *mut ValueRef;
                let mut item_index = 0;
                for bucket in buckets {
                    for item in bucket {
                        std::ptr::write(items_ptr.add(item_index), item);
                        item_index += 1;
                    }
                }
            }
            
            ObjectReference::from_raw_address(address).unwrap()
        })
    }

    

    pub fn alloc_error(&self, error: BlinkError) -> ObjectReference {
        todo!()
    }

    pub fn alloc_callable(&self, function: Callable) -> ObjectReference {
        todo!()
    }

    pub fn alloc_val(&self, val: HeapValue) -> ObjectReference {
        match val {
            HeapValue::List(list) => self.alloc_vec(list),
            HeapValue::Str(str) => self.alloc_str(&str),
            HeapValue::Map(map) => self.alloc_blink_hash_map(map),
            HeapValue::Vector(value_refs) => self.alloc_vec(value_refs),
            HeapValue::Set(blink_hash_set) => self.alloc_blink_hash_set(blink_hash_set),
            HeapValue::Error(blink_error) => self.alloc_error(blink_error),
            HeapValue::Function(callable) => todo!(),
            HeapValue::Macro(_) => todo!(),
            HeapValue::Future(blink_future) => todo!(),
            HeapValue::Env(env) => todo!(),
        }
    }
    
    
    fn calculate_bucket_count(item_count: usize) -> usize {
        if item_count == 0 {
            1
        } else {
            (item_count * 2).next_power_of_two().max(8)
        }
    }
    
    


}

fn read_header(address: ObjectReference) -> ObjectHeader {
    unsafe {
        let base_ptr = address.to_raw_address().as_usize() as *mut u8;
        let header_ptr = base_ptr as *mut ObjectHeader;
        std::ptr::read(header_ptr)
    }
}

impl ValueRef {
    pub fn read_heap_value(&self) -> Option<HeapValue> {

        match self {
            ValueRef::Immediate(_) => None,
            ValueRef::Heap(gc_ptr) => {
                        Some(gc_ptr.to_heap_value())
            }
            ValueRef::Native(_) => todo!(),
        }
    }


    pub fn read_vec(&self, address: ObjectReference) -> Vec<ValueRef> {
        unsafe {
            let base_ptr = address.to_raw_address().as_usize() as *mut u8;
            let header_ptr = base_ptr as *mut ObjectHeader;
            let header = std::ptr::read(header_ptr);
            
            // Verify this is actually a vector/list
            debug_assert_eq!(header.type_tag, TypeTag::List as i8);
            
            let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
            
            // Read vector metadata
            let len_ptr = data_start as *const usize;
            let len = std::ptr::read(len_ptr);
            
            let cap_ptr = data_start.add(std::mem::size_of::<usize>()) as *const usize;
            let _capacity = std::ptr::read(cap_ptr); // Read but don't need to use
            
            // Read vector data
            let vec_data_ptr = data_start.add(std::mem::size_of::<usize>() * 2) as *const ValueRef;
            let mut items = Vec::with_capacity(len);
            
            for i in 0..len {
                let item = std::ptr::read(vec_data_ptr.add(i));
                items.push(item);
            }
            
            items
        }
    }

    

    pub fn read_string(&self, address: ObjectReference) -> String {
        unsafe {
            let base_ptr = address.to_raw_address().as_usize() as *mut u8;
            let header_ptr = base_ptr as *mut ObjectHeader;
            let header = std::ptr::read(header_ptr);
            
            // Verify this is actually a string
            debug_assert_eq!(header.type_tag, TypeTag::Str as i8);
            
            let data_start = base_ptr.add(std::mem::size_of::<ObjectHeader>());
            
            // Read string length
            let len_ptr = data_start as *const u32;
            let len = std::ptr::read(len_ptr) as usize;
            
            // Read string data
            let str_data_ptr = data_start.add(std::mem::size_of::<u32>()) as *const u8;
            let mut bytes = Vec::with_capacity(len);
            
            std::ptr::copy_nonoverlapping(str_data_ptr, bytes.as_mut_ptr(), len);
            bytes.set_len(len);
            
            // Convert bytes back to String
            // This assumes the original string was valid UTF-8
            String::from_utf8_unchecked(bytes)
        }
    }
}