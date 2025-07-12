use crate::error::{BlinkError, BlinkErrorType};
use crate::future::BlinkFuture;
use crate::module::{Module, SerializedModuleSource};
use crate::runtime::mmtk::ObjectHeader;
use crate::runtime::{BlinkObjectModel, TypeTag};
use crate::value::{Callable, GcPtr, SourceRange};
use crate::collections::{BlinkHashMap, BlinkHashSet};
use crate::env::Env;
use crate::{runtime::BlinkVM, value::ValueRef};
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

    fn fake_object_reference(id: usize) -> ObjectReference {
        // Use a base address that's definitely non-zero
        let base_addr = 0x10000; // 64KB base
        let addr = base_addr + id;
        ObjectReference::from_raw_address(unsafe { Address::from_usize(addr) })
            .expect("Failed to create fake ObjectReference")
    }

    pub fn alloc_vec_or_list(&self, items: Vec<ValueRef>, is_list: bool) -> ObjectReference {
        self.with_mutator(|mutator| {
            let vec_data_size = items.len() * std::mem::size_of::<ValueRef>();
            let total_data_size = vec_data_size;
            let total_size = total_data_size;
            let type_tag = if is_list { TypeTag::List } else { TypeTag::Vector };
            let data_start = BlinkObjectModel::alloc_with_type(mutator, type_tag, total_size);
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut ValueRef;
                std::ptr::copy_nonoverlapping(items.as_ptr(), data_ptr, items.len());
            }
            data_start
        })
    }

    pub fn alloc_macro(&self, mac: Callable) -> ObjectReference {
        self.alloc_callable(mac, true)
    }

    pub fn alloc_user_defined_fn(&self, function: Callable) -> ObjectReference {
        self.alloc_callable(function, false)
    }


    pub fn alloc_callable(&self, function: Callable, is_macro: bool) -> ObjectReference {
        //[params_count][param1][param2]...[body_count][expr1][expr2]...[env_ref][is_variadic]
        self.with_mutator(|mutator| {
            let params_count = function.params.len();
            let body_count = function.body.len();
            
            // Layout: [params_count][params...][body_count][body...][env_ref][is_variadic]
            let params_size = std::mem::size_of::<u32>() + // count
                             (params_count * std::mem::size_of::<u32>()); // param symbol IDs
            
            let body_size = std::mem::size_of::<u32>() + // count
                           (body_count * std::mem::size_of::<ValueRef>()); // body expressions
            
            let env_size = std::mem::size_of::<ObjectReference>(); // env reference
            let variadic_size = std::mem::size_of::<bool>(); // is_variadic flag
            
            let total_size = params_size + body_size + env_size + variadic_size;
            
            let data_start = BlinkObjectModel::alloc_with_type(
                mutator, 
                if is_macro { TypeTag::Macro } else { TypeTag::UserDefinedFunction }, 
                total_size
            );
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write params count
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, params_count as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write parameter IDs
                for param_id in &function.params {
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, *param_id);
                    offset += std::mem::size_of::<u32>();
                }
                
                // Write body count
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, body_count as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write body expressions
                for expr in &function.body {
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut ValueRef, *expr);
                    offset += std::mem::size_of::<ValueRef>();
                }
                
                // Write environment reference
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut ObjectReference, function.env);
                offset += std::mem::size_of::<ObjectReference>();
                
                // Write variadic flag
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut bool, function.is_variadic);
            }
            
            data_start
        })
    }
    

    pub fn alloc_module(&self, module: &Module) -> ObjectReference {
        self.with_mutator(|mutator| {
            let exports_count = module.exports.len();
            
            // Layout: [name][env_ref][exports_count][exports...][source_data][ready]
            let name_size = std::mem::size_of::<u32>();
            let env_size = std::mem::size_of::<ObjectReference>();
            let exports_size = std::mem::size_of::<u32>() + // count
                              (exports_count * std::mem::size_of::<u32>()); // exports
            let source_size = std::mem::size_of::<SerializedModuleSource>();
            let ready_size = std::mem::size_of::<bool>();
            
            let total_size = name_size + env_size + exports_size + source_size + ready_size;
            
            let data_start = BlinkObjectModel::alloc_with_type(mutator, TypeTag::Module, total_size);
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write name
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, module.name);
                offset += std::mem::size_of::<u32>();
                
                // Write env reference
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut ObjectReference, module.env);
                offset += std::mem::size_of::<ObjectReference>();
                
                // Write exports count
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, exports_count as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write exports
                for export in &module.exports {
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, *export);
                    offset += std::mem::size_of::<u32>();
                }
                
                // Write source data
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut SerializedModuleSource, module.source.clone());
                offset += std::mem::size_of::<SerializedModuleSource>();
                
                // Write ready flag
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut bool, module.ready);
            }
            
            data_start
        })
    }
    
    
    // Helper function to calculate SerializedModuleSource size
    fn calculate_serialized_source_size(&self, source: &SerializedModuleSource) -> usize {
        match source {
            SerializedModuleSource::Global => std::mem::size_of::<u8>(), // Just the variant tag
            SerializedModuleSource::Repl => std::mem::size_of::<u8>(), // Just the variant tag
            SerializedModuleSource::BlinkFile(_) => {
                std::mem::size_of::<u8>() + // variant tag
                std::mem::size_of::<u32>()  // symbol ID
            }
            SerializedModuleSource::NativeDylib(_) => {
                std::mem::size_of::<u8>() + std::mem::size_of::<u32>()
            }
            SerializedModuleSource::BlinkPackage(_) => {
                std::mem::size_of::<u8>() + std::mem::size_of::<u32>()
            }
            SerializedModuleSource::Cargo(_) => {
                std::mem::size_of::<u8>() + std::mem::size_of::<u32>()
            }
            // SerializedModuleSource::Git { repo, reference } => {
            //     std::mem::size_of::<u8>() + // variant tag
            //     std::mem::size_of::<u32>() + // repo symbol ID
            //     std::mem::size_of::<u8>() + // has_reference flag
            //     if reference.is_some() { std::mem::size_of::<u32>() } else { 0 } // optional reference symbol ID
            // }
            SerializedModuleSource::Url(_) => {
                std::mem::size_of::<u8>() + std::mem::size_of::<u32>()
            }
            SerializedModuleSource::BlinkDll(_) => {
                std::mem::size_of::<u8>() + std::mem::size_of::<u32>()
            }
            SerializedModuleSource::Wasm(_) => {
                std::mem::size_of::<u8>() + std::mem::size_of::<u32>()
            }
        }
    }
    
    // Helper function to write SerializedModuleSource to memory
    unsafe fn write_serialized_source(&self, ptr: *mut u8, source: &SerializedModuleSource) -> usize {
        let mut offset = 0;
        
        match source {
            SerializedModuleSource::Global => {
                *(ptr.add(offset) as *mut u8) = 0;
                offset += std::mem::size_of::<u8>();
            }
            SerializedModuleSource::Repl => {
                        *(ptr.add(offset) as *mut u8) = 1; // variant tag
                        offset += std::mem::size_of::<u8>();
                    }
            SerializedModuleSource::BlinkFile(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 2;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::NativeDylib(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 3;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::BlinkPackage(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 4;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::Cargo(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 5;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::Url(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 7;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::BlinkDll(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 8;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::Wasm(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 9;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            
                    }
        
        offset
    }


    pub fn alloc_env(&self, env: Env) -> ObjectReference {
        self.with_mutator(|mutator| {
            let vars_count = env.vars.len();
            let modules_count = env.symbol_aliases.len();
            
            // Layout: [vars_count][var_pairs...][modules_count][module_pairs...][parent_ref]
            let vars_size = std::mem::size_of::<u32>() + // count
                           (vars_count * (std::mem::size_of::<u32>() + std::mem::size_of::<ValueRef>()));
            
            let modules_size = std::mem::size_of::<u32>() + // count  
                              (modules_count * std::mem::size_of::<u32>() * 2); // pairs
            
            let parent_size = std::mem::size_of::<Option<ObjectReference>>();
            let total_size = vars_size + modules_size + parent_size;
            
            let data_start = BlinkObjectModel::alloc_with_type(mutator, TypeTag::Env, total_size);
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write vars count
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, vars_count as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write vars in sorted order for faster lookup
                let mut sorted_vars: Vec<_> = env.vars.iter().collect();
                sorted_vars.sort_by_key(|(k, _)| *k);
                
                for (key, value) in sorted_vars {
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, *key);
                    offset += std::mem::size_of::<u32>();
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut ValueRef, *value);
                    offset += std::mem::size_of::<ValueRef>();
                }
                
                // Write modules count
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, modules_count as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write modules in sorted order for faster lookup
                let mut sorted_modules: Vec<_> = env.symbol_aliases.iter().collect();
                sorted_modules.sort_by_key(|(k, _)| *k);
                
                for (alias, module) in sorted_modules {
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, *alias);
                    offset += std::mem::size_of::<u32>();
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, *module);
                    offset += std::mem::size_of::<u32>();
                }
                
                // Write parent reference
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut Option<ObjectReference>, env.parent);
            }
            
            data_start
        })
    }
    
    

    pub fn alloc_str(&self, s: &str) -> ObjectReference {
        
        //TODO String interning
        self.with_mutator(|mutator| {
            // For strings, we might want to store the string data inline
            let string_bytes = s.as_bytes();
            let data_size = string_bytes.len();
            let total_size =  data_size;

            let data_start = BlinkObjectModel::alloc_with_type(mutator, TypeTag::Str, total_size);
            
            unsafe {
                let  base_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                
                std::ptr::copy_nonoverlapping(string_bytes.as_ptr(), base_ptr, data_size);
            }

            data_start

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
            
            let buckets_size = bucket_count * std::mem::size_of::<u32>(); // bucket start indices
            let pairs_size = item_count * 2 * std::mem::size_of::<ValueRef>();
            let total_data_size = buckets_size + pairs_size;
            
            let data_start = BlinkObjectModel::alloc_with_type(mutator, TypeTag::Map, total_data_size);
            
            unsafe {
                
                // Write metadata
                let bucket_count_ptr = data_start.to_raw_address().as_usize() as *mut usize;
                std::ptr::write(bucket_count_ptr, bucket_count);
                let item_count_ptr = data_start.to_raw_address().as_usize() as *mut usize;
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
                let bucket_offsets_ptr = data_start.to_raw_address().as_usize() as *mut u32;
                let mut current_offset = 0u32;
                for (i, bucket) in buckets.iter().enumerate() {
                    std::ptr::write(bucket_offsets_ptr.add(i), current_offset);
                    current_offset += bucket.len() as u32;
                }
                
                // Write pairs
                let pairs_ptr = data_start.to_raw_address().as_usize() as *mut ValueRef;
                let mut pair_index = 0;
                for bucket in buckets {
                    for (key, val) in bucket {
                        std::ptr::write(pairs_ptr.add(pair_index * 2), key);
                        std::ptr::write(pairs_ptr.add(pair_index * 2 + 1), val);
                        pair_index += 1;
                    }
                }
            }
            
            data_start
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
        self.with_mutator(|mutator| {
            // Serialize the error message as a string (we'll intern it)
            let message_id = self.symbol_table.write().intern(&error.message);
            
            // Calculate sizes for each component
            let message_size = std::mem::size_of::<u32>(); // interned string ID
            let pos_size = std::mem::size_of::<Option<SourceRange>>();
            let error_type_size = std::mem::size_of::<u8>() + // discriminant
                                  std::mem::size_of::<u32>(); // enough space for the largest variant data
            
            let total_size = message_size + pos_size + error_type_size;
            
            let data_start = BlinkObjectModel::alloc_with_type(mutator, TypeTag::Error, total_size);
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write message (as interned string ID)
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, message_id);
                offset += std::mem::size_of::<u32>();
                
                // Write position
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut Option<SourceRange>, error.pos);
                offset += std::mem::size_of::<Option<SourceRange>>();
                
                // Write error type (simplified - just store discriminant for now)
                let error_type_discriminant = match error.error_type {
                    BlinkErrorType::Tokenizer => 0u8,
                    BlinkErrorType::Parse(_) => 1u8,
                    BlinkErrorType::UndefinedSymbol { .. } => 2u8,
                    BlinkErrorType::Eval => 3u8,
                    BlinkErrorType::ArityMismatch { .. } => 4u8,
                    BlinkErrorType::UnexpectedToken { .. } => 5u8,
                    BlinkErrorType::UserDefined { .. } => 6u8,
                };
                
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u8, error_type_discriminant);
                offset += std::mem::size_of::<u8>();
                
                // For now, just write a placeholder for the variant data
                // In a full implementation, you'd serialize the specific error type data
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, 0u32);
            }
            
            data_start
        })
    }

    pub fn alloc_future(&self, future: BlinkFuture) -> ObjectReference {
        Self::fake_object_reference(0x70000)
    }

    pub fn alloc_val(&self, val: HeapValue) -> ObjectReference {
        match val {
            HeapValue::List(list) => self.alloc_vec_or_list(list, true),
            HeapValue::Str(str) => self.alloc_str(&str),
            HeapValue::Map(map) => self.alloc_blink_hash_map(map),
            HeapValue::Vector(value_refs) => self.alloc_vec_or_list(value_refs, false),
            HeapValue::Set(blink_hash_set) => self.alloc_blink_hash_set(blink_hash_set),
            HeapValue::Error(blink_error) => self.alloc_error(blink_error),
            HeapValue::Function(callable) => self.alloc_user_defined_fn(callable),
            HeapValue::Macro(mac) => self.alloc_macro(mac),
            HeapValue::Future(blink_future) => self.alloc_future(blink_future),
            HeapValue::Env(env) => self.alloc_env(env),
            HeapValue::Module(module) => self.alloc_module(&module),
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