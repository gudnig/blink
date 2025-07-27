use crate::error::{BlinkError, BlinkErrorType, ParseErrorType};
use crate::future::BlinkFuture;
use crate::module::{Module, SerializedModuleSource};
use crate::runtime::mmtk::ObjectHeader;
use crate::runtime::{BlinkActivePlan, BlinkObjectModel, CompiledFunction, TypeTag, GLOBAL_MMTK};
use crate::value::{Callable, GcPtr, ParsedValue, ParsedValueWithPos, SourceRange};
use crate::collections::{BlinkHashMap, BlinkHashSet};
use crate::env::Env;
use crate::{runtime::BlinkVM, value::ValueRef};
use mmtk::memory_manager::alloc;
use mmtk::util::{alloc, Address, OpaquePointer, VMThread};
use mmtk::MutatorContext;
use mmtk::{util::ObjectReference, Mutator, util::VMMutatorThread, memory_manager};
use parking_lot::{Condvar, Mutex};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::hash::{DefaultHasher, Hash, Hasher};

static GC_PARK: OnceLock<Arc<(Mutex<bool>, Condvar)>> = OnceLock::new();

pub fn init_gc_park() -> Arc<(Mutex<bool>, Condvar)> {
    GC_PARK.get_or_init(|| Arc::new((Mutex::new(false), Condvar::new()))).clone()
}
use crate::value::HeapValue;

static THREAD_IDS: OnceLock<Mutex<HashMap<std::thread::ThreadId, usize>>> = OnceLock::new();
static COUNTER: AtomicUsize = AtomicUsize::new(1);

impl BlinkVM {
    pub fn with_mutator<T>(&self, f: impl FnOnce(&mut Mutator<BlinkVM>) -> T) -> T {
        BlinkActivePlan::with_mutator(f)
    }
    
    
    // Handle the static lifetime requirement for MMTK
    fn get_static_mmtk(&self) -> &'static mmtk::MMTK<BlinkVM> {
        // UNSAFE: Required because bind_mutator expects 'static
        // This is safe as long as your VM instance lives for the program duration
        unsafe {
            let mmtk = GLOBAL_MMTK.get().expect("MMTK not initialized");
            std::mem::transmute::<&mmtk::MMTK<BlinkVM>, &'static mmtk::MMTK<BlinkVM>>(&*mmtk)
            
        }
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
            let data_start = BlinkActivePlan::alloc(mutator, &type_tag, &total_size);
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut ValueRef;
                std::ptr::copy_nonoverlapping(items.as_ptr(), data_ptr, items.len());
            }
            data_start
        })
    }

    pub fn alloc_user_defined_fn(&self, function: CompiledFunction) -> ObjectReference {
        self.alloc_callable(function)
    }


    pub fn alloc_callable(&self, function: CompiledFunction) -> ObjectReference {
        self.with_mutator(|mutator| {

            /* 
            pub struct CompiledFunction {
                pub bytecode: Bytecode,
                pub constants: Vec<ValueRef>,  // Constant pool for complex values
                pub parameter_count: u8,
                pub register_count: u8,
                pub module: u32,
            }
             */
            let constants_count = function.constants.len();
            let bytecode_len = function.bytecode.len();
            
            // GC-FRIENDLY LAYOUT: All ObjectReferences first!
            // [parameter_count: u8]
            // [register_count: u8]
            // [module_id: u32]
            // [constants_count: u32]
            // [constants: ValueRef...]
            // [bytecode_count: u32]
            // [bytecode: u8...]
            
            let total_size = 
            std::mem::size_of::<u32>() +                              // constants_count
            constants_count * std::mem::size_of::<ValueRef>() +       // constants
            std::mem::size_of::<u8>() +                               // parameter_count
            std::mem::size_of::<u8>() +                               // register_count  
            std::mem::size_of::<u32>() +                              // module
            std::mem::size_of::<u32>() +                              // bytecode_len
            bytecode_len;                                             // bytecode data
            
            
            let type_tag = TypeTag::UserDefinedFunction;
            let data_start = BlinkActivePlan::alloc(mutator, &type_tag, &total_size);


            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write constants count FIRST (needed for GC scanning)
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, constants_count as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write ALL constants together (ObjectReferences that GC needs to scan)
                for constant in &function.constants {
                    std::ptr::write_unaligned(data_ptr.add(offset) as *mut ValueRef, *constant);
                    offset += std::mem::size_of::<ValueRef>();
                }
                
                // Now write all non-reference data (GC won't scan past this point)
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u8, function.parameter_count);
                offset += std::mem::size_of::<u8>();
                
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u8, function.register_count);
                offset += std::mem::size_of::<u8>();
                
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, function.module);
                offset += std::mem::size_of::<u32>();
                
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, bytecode_len as u32);
                offset += std::mem::size_of::<u32>();
                
                // Write bytecode data
                std::ptr::copy_nonoverlapping(
                    function.bytecode.as_ptr(),
                    data_ptr.add(offset),
                    bytecode_len
                );
            }
            
            data_start
        })
    }



    pub fn alloc_parsed_value(&self, parsed: ParsedValueWithPos) -> ValueRef {
        let value_ref = match parsed.value {
            // Immediate values - pack directly
            ParsedValue::Number(n) => ValueRef::number(n),
            ParsedValue::Bool(b) => ValueRef::boolean(b),
            ParsedValue::Symbol(id) => ValueRef::symbol(id),
            ParsedValue::Keyword(id) => ValueRef::keyword(id),
            ParsedValue::Nil => ValueRef::nil(),

            // Complex values - alloc on gc heap
            // functions are allocated during execution
            // TODO: gradually allocate all values during execution
            ParsedValue::String(s) => self.string_value(&s),

            ParsedValue::List(items) => {
                let converted_items: Vec<ValueRef> = items
                    .into_iter()
                    .map(|item| self.alloc_parsed_value(item))
                    .collect();

                self.list_value(converted_items)
            }

            ParsedValue::Vector(items) => {
                let converted_items: Vec<ValueRef> = items
                    .into_iter()
                    .map(|item| self.alloc_parsed_value(item))
                    .collect();

                self.vector_value(converted_items)
            }

            ParsedValue::Map(pairs) => {
                let value_pairs = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        let key = self.alloc_parsed_value(k);
                        let value = self.alloc_parsed_value(v);
                        (key, value)
                    })
                    .collect();

                self.map_value(value_pairs)
            }
        };

        if let (Some(id), Some(pos)) = (value_ref.get_or_create_id(), parsed.pos) {
            self.value_metadata.write().set_position(id, pos);
        }

        value_ref
    }
    
    
    
    // Helper function to calculate SerializedModuleSource size
    fn calculate_serialized_source_size(&self, source: &SerializedModuleSource) -> usize {
        match source {
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
            
            SerializedModuleSource::Repl => {
                        *(ptr.add(offset) as *mut u8) = 0; // variant tag
                        offset += std::mem::size_of::<u8>();
                    }
            SerializedModuleSource::BlinkFile(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 1;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::NativeDylib(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 2;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::BlinkPackage(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 3;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::Cargo(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 4;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::Url(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 5;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::BlinkDll(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 6;
                        offset += std::mem::size_of::<u8>();
                        *(ptr.add(offset) as *mut u32) = *symbol_id;
                        offset += std::mem::size_of::<u32>();
                    }
            SerializedModuleSource::Wasm(symbol_id) => {
                        *(ptr.add(offset) as *mut u8) = 7;
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
        
        // NEW LAYOUT: All ObjectReferences at the beginning for easy scanning
        // [vars_count: u32]
        // [valueref_array: ValueRef...]          <- ObjectReferences #2 to #(vars_count+1)  
        // [var_keys: u32...]                     <- No ObjectReferences
         
        
        let refs_size = vars_count * std::mem::size_of::<ValueRef>();
        let counts_size = 3 * std::mem::size_of::<u32>(); // 3 counts
        let keys_size = vars_count * std::mem::size_of::<u32>();
        
        let total_size = counts_size + refs_size + keys_size;
        
        let data_start = BlinkActivePlan::alloc(mutator, &TypeTag::Env, &total_size);
        
        unsafe {
            let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            
            // Write counts
            std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, vars_count as u32);
            offset += std::mem::size_of::<u32>();
            
            // Write ALL ValueRefs together (ObjectReferences #2 to #(vars_count+1))
            let mut sorted_vars: Vec<_> = env.vars.iter().collect();
            sorted_vars.sort_by_key(|(k, _)| *k);
            
            for (_, value) in &sorted_vars {
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut ValueRef, *value);
                offset += std::mem::size_of::<ValueRef>();
            }
            
            // Now write all the non-reference data
            // Write var keys (corresponding to the ValueRefs above)
            for (key, _) in &sorted_vars {
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, *key);
                offset += std::mem::size_of::<u32>();
            }
            

        }
        
        data_start
    })
}

    
    

    pub fn alloc_str(&self, s: &str) -> ObjectReference {
        //println!("Thread {:?} requesting string allocation", std::thread::current().id());
        //TODO String interning
        self.with_mutator(|mutator| {
            //rintln!("Thread {:?} has mutator, allocating string", std::thread::current().id());
            
            // For strings, we might want to store the string data inline
            let string_bytes = s.as_bytes();
            let data_size = string_bytes.len();
            let total_size =  data_size;

            let data_start = BlinkActivePlan::alloc(mutator, &TypeTag::Str, &total_size);
            
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
            
            // Calculate sizes
            let metadata_size = 2 * std::mem::size_of::<usize>(); // bucket_count + item_count
            let buckets_size = bucket_count * std::mem::size_of::<u32>(); // bucket offsets
            let pairs_size = item_count * 2 * std::mem::size_of::<ValueRef>(); // key-value pairs
            let total_data_size = metadata_size + buckets_size + pairs_size;
            
            let data_start = BlinkActivePlan::alloc(mutator, &TypeTag::Map, &total_data_size);
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write metadata
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut usize, bucket_count);
                offset += std::mem::size_of::<usize>();
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut usize, item_count);
                offset += std::mem::size_of::<usize>();
                
                // Organize into buckets
                let mut buckets: Vec<Vec<(ValueRef, ValueRef)>> = vec![Vec::new(); bucket_count];
                for (key, val) in pairs {
                    let mut hasher = DefaultHasher::new();
                    key.hash(&mut hasher);
                    let hash_value = hasher.finish();
                    let bucket = (hash_value as usize) % bucket_count;
                    buckets[bucket].push((*key, *val));
                }
                
                // Write bucket offsets (using correct offset)
                let bucket_offsets_ptr = data_ptr.add(offset) as *mut u32;  // ← Fixed!
                let mut current_offset = 0u32;
                for (i, bucket) in buckets.iter().enumerate() {
                    std::ptr::write(bucket_offsets_ptr.add(i), current_offset);
                    current_offset += bucket.len() as u32;
                }
                offset += buckets_size;
                
                // Write pairs (using correct offset)
                let pairs_ptr = data_ptr.add(offset) as *mut ValueRef;  // ← Fixed!
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
            
            // Use consistent allocation method like other types
            let data_start = BlinkActivePlan::alloc(mutator, &TypeTag::Set, &total_data_size);
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write metadata
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut usize, bucket_count);
                offset += std::mem::size_of::<usize>();
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut usize, item_count);
                offset += std::mem::size_of::<usize>();
                
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
                let bucket_offsets_ptr = data_ptr.add(offset) as *mut u32;
                let mut current_offset = 0u32;
                for (i, bucket) in buckets.iter().enumerate() {
                    std::ptr::write(bucket_offsets_ptr.add(i), current_offset);
                    current_offset += bucket.len() as u32;
                }
                offset += buckets_size;
                
                // Write items
                let items_ptr = data_ptr.add(offset) as *mut ValueRef;
                let mut item_index = 0;
                for bucket in buckets {
                    for item in bucket {
                        std::ptr::write(items_ptr.add(item_index), item);
                        item_index += 1;
                    }
                }
            }
            
            data_start  // Return the ObjectReference directly
        })
    }

    pub fn alloc_error(&self, error: BlinkError) -> ObjectReference {
        self.with_mutator(|mutator| {
            let message_bytes = error.message.as_bytes();
            let message_len = message_bytes.len() as u32;
            
            // Calculate sizes
            let message_size = std::mem::size_of::<u32>() + message_bytes.len();
            let pos_size = std::mem::size_of::<Option<SourceRange>>();
            let error_type_size = Self::calculate_error_type_size(&error.error_type);
            
            let total_size = message_size + pos_size + error_type_size;
            let data_start = BlinkActivePlan::alloc(mutator, &TypeTag::Error, &total_size);
            
            unsafe {
                let data_ptr = data_start.to_raw_address().as_usize() as *mut u8;
                let mut offset = 0;
                
                // Write message length
                std::ptr::write_unaligned(data_ptr.add(offset) as *mut u32, message_len);
                offset += std::mem::size_of::<u32>();
                
                // Write message bytes
                std::ptr::copy_nonoverlapping(
                    message_bytes.as_ptr(),
                    data_ptr.add(offset),
                    message_bytes.len()
                );
                offset += message_bytes.len();
                
                // Write position
                std::ptr::write_unaligned(
                    data_ptr.add(offset) as *mut Option<SourceRange>,
                    error.pos
                );
                offset += std::mem::size_of::<Option<SourceRange>>();
                
                // Write error type data
                Self::write_error_type_data(data_ptr.add(offset), &error.error_type);
            }
            
            data_start
        })
    }

    fn calculate_error_type_size(error_type: &BlinkErrorType) -> usize {
        let discriminant_size = std::mem::size_of::<u8>();
        
        let variant_size = match error_type {
            BlinkErrorType::Tokenizer => 0,
            BlinkErrorType::Parse(parse_type) => {
                // Store ParseErrorType discriminant
                std::mem::size_of::<u8>()
            },
            BlinkErrorType::UndefinedSymbol { name } => {
                // Store name length + name bytes
                std::mem::size_of::<u32>() + name.as_bytes().len()
            },
            BlinkErrorType::Eval => 0,
            BlinkErrorType::ArityMismatch { expected, got, form } => {
                // Store expected, got, form_len, form_bytes
                std::mem::size_of::<usize>() + 
                std::mem::size_of::<usize>() + 
                std::mem::size_of::<u32>() + 
                form.as_bytes().len()
            },
            BlinkErrorType::UnexpectedToken { token } => {
                // Store token length + token bytes
                std::mem::size_of::<u32>() + token.as_bytes().len()
            },
            BlinkErrorType::UserDefined { data } => {
                // Store option discriminant + potential ObjectReference
                std::mem::size_of::<u8>() + 
                if data.is_some() { std::mem::size_of::<ObjectReference>() } else { 0 }
            },
        };
        
        discriminant_size + variant_size
    }
    

    unsafe fn write_error_type_data(ptr: *mut u8, error_type: &BlinkErrorType) {
        let mut offset = 0;
        
        // Write discriminant
        let discriminant = match error_type {
            BlinkErrorType::Tokenizer => 0u8,
            BlinkErrorType::Parse(_) => 1u8,
            BlinkErrorType::UndefinedSymbol { .. } => 2u8,
            BlinkErrorType::Eval => 3u8,
            BlinkErrorType::ArityMismatch { .. } => 4u8,
            BlinkErrorType::UnexpectedToken { .. } => 5u8,
            BlinkErrorType::UserDefined { .. } => 6u8,
        };
        
        std::ptr::write_unaligned(ptr.add(offset) as *mut u8, discriminant);
        offset += std::mem::size_of::<u8>();
        
        // Write variant data
        match error_type {
            BlinkErrorType::Tokenizer | BlinkErrorType::Eval => {
                // No additional data
            },
            BlinkErrorType::Parse(parse_type) => {
                let parse_discriminant = match parse_type {
                    ParseErrorType::UnexpectedEof => 0u8,
                    ParseErrorType::UnclosedDelimiter(_) => 1u8,
                    ParseErrorType::UnexpectedToken(_) => 2u8,
                    ParseErrorType::InvalidNumber(_) => 3u8,
                    ParseErrorType::InvalidString(_) => 4u8,
                };
                std::ptr::write_unaligned(ptr.add(offset) as *mut u8, parse_discriminant);
            },
            BlinkErrorType::UndefinedSymbol { name } => {
                let name_bytes = name.as_bytes();
                let name_len = name_bytes.len() as u32;
                
                std::ptr::write_unaligned(ptr.add(offset) as *mut u32, name_len);
                offset += std::mem::size_of::<u32>();
                
                std::ptr::copy_nonoverlapping(name_bytes.as_ptr(), ptr.add(offset), name_bytes.len());
            },
            BlinkErrorType::ArityMismatch { expected, got, form } => {
                std::ptr::write_unaligned(ptr.add(offset) as *mut usize, *expected);
                offset += std::mem::size_of::<usize>();
                
                std::ptr::write_unaligned(ptr.add(offset) as *mut usize, *got);
                offset += std::mem::size_of::<usize>();
                
                let form_bytes = form.as_bytes();
                let form_len = form_bytes.len() as u32;
                
                std::ptr::write_unaligned(ptr.add(offset) as *mut u32, form_len);
                offset += std::mem::size_of::<u32>();
                
                std::ptr::copy_nonoverlapping(form_bytes.as_ptr(), ptr.add(offset), form_bytes.len());
            },
            BlinkErrorType::UnexpectedToken { token } => {
                let token_bytes = token.as_bytes();
                let token_len = token_bytes.len() as u32;
                
                std::ptr::write_unaligned(ptr.add(offset) as *mut u32, token_len);
                offset += std::mem::size_of::<u32>();
                
                std::ptr::copy_nonoverlapping(token_bytes.as_ptr(), ptr.add(offset), token_bytes.len());
            },
            BlinkErrorType::UserDefined { data } => {
                match data {
                    Some(value_ref) => {
                        std::ptr::write_unaligned(ptr.add(offset) as *mut u8, 1u8); // Some
                        offset += std::mem::size_of::<u8>();
                        std::ptr::write_unaligned(ptr.add(offset) as *mut ValueRef, *value_ref);  // ← Fixed!
                    },
                    None => {
                        std::ptr::write_unaligned(ptr.add(offset) as *mut u8, 0u8); // None
                    }
                }
            },
        }
    }

    pub fn alloc_future(&self, future: BlinkFuture) -> ObjectReference {
        Self::fake_object_reference(0x70000)
    }

    pub fn update_env_variable(&self, env_ref: ObjectReference, symbol: u32, new_value: ValueRef) {
        unsafe {
            let data_ptr = env_ref.to_raw_address().as_usize() as *mut u8;
            let mut offset = 0;
            
            // Skip parent reference
            offset += std::mem::size_of::<Option<ObjectReference>>();
            
            // Read vars_count
            let vars_count = std::ptr::read_unaligned(data_ptr.add(offset) as *const u32) as usize;
            offset += std::mem::size_of::<u32>();
            
            // Skip other counts
            offset += std::mem::size_of::<u32>() * 2; // symbol_aliases_count + module_aliases_count
            
            // Now we're at the ValueRef array - this is what we need to update
            let values_start_ptr = data_ptr.add(offset) as *mut ValueRef;
            
            // Skip past all ValueRefs to get to the keys
            let keys_start_offset = offset + (vars_count * std::mem::size_of::<ValueRef>());
            let keys_start_ptr = data_ptr.add(keys_start_offset) as *const u32;
            
            // Read the keys array to find our symbol
            let keys_slice = std::slice::from_raw_parts(keys_start_ptr, vars_count);
            
            // Binary search to find the symbol (keys are sorted)
            match keys_slice.binary_search(&symbol) {
                Ok(index) => {
                    // Found it! Update the corresponding ValueRef
                    let value_ptr = values_start_ptr.add(index);
                    std::ptr::write(value_ptr, new_value);
                }
                Err(_) => {
                    // Symbol not found - this shouldn't happen if we set it up correctly
                    eprintln!("Warning: Symbol {} not found in environment", symbol);
                }
            }
        }
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
            HeapValue::Future(blink_future) => self.alloc_future(blink_future),
            HeapValue::Env(env) => self.alloc_env(env),
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