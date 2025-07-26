use std::sync::Arc;

use crate::{
    error::BlinkError, eval::EvalResult, future::BlinkFuture, runtime::{BlinkVM, ContextualBoundary, ValueBoundary, GLOBAL_VM}, value::{GcPtr, IsolatedValue, SourceRange, ValueRef}, BlinkHashMap, BlinkHashSet
};

pub type IsolatedNativeFn =
    Box<dyn Fn(Vec<IsolatedValue>) -> Result<IsolatedValue, String> + Send + Sync>;
pub type ContextualNativeFn =
    Box<dyn Fn(Vec<ValueRef>, &mut NativeContext) -> EvalResult + Send + Sync>;

pub enum NativeFn {
    Isolated(IsolatedNativeFn),
    Contextual(ContextualNativeFn),
}

impl std::fmt::Debug for NativeFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeFn::Isolated(_) => write!(f, "NativeFn::Isolated(<function>)"),
            NativeFn::Contextual(_) => write!(f, "NativeFn::Contextual(<function>)"),
        }
    }
}

impl NativeFn {

    pub fn call(&self, args: Vec<ValueRef>) -> EvalResult {
        let vm = GLOBAL_VM.get().unwrap().clone();
        match self {
            NativeFn::Isolated(f) => {
                let mut boundary = ContextualBoundary::new(vm.clone());

                // Extract to isolated values
                let isolated_args: Result<Vec<_>, _> = args
                    .iter()
                    .map(|arg| boundary.extract_isolated(*arg))
                    .collect();
                let isolated_args = isolated_args.map_err(|e| BlinkError::eval(e.to_string()));

                if let Err(e) = isolated_args {
                    return EvalResult::Value(vm.eval_error(&e.to_string()));
                }
                let isolated_args = isolated_args.unwrap();
                // Call function
                let result = f(isolated_args);

                match result {
                    Ok(result) => {
                        // Convert back
                        EvalResult::Value(boundary.alloc_from_isolated(result))
                    }
                    Err(e) => EvalResult::Value(vm.eval_error(&e.to_string())),
                }
            }

            NativeFn::Contextual(f) => {
                let mut boundary = NativeContext::new(&vm);
                f(args, &mut boundary)
            }
        }
    }
}


/// Lightweight context for native functions - provides safe access to VM operations
/// without exposing execution state or environments
pub struct NativeContext<'a> {
    vm: &'a Arc<BlinkVM>,
}

impl<'a> NativeContext<'a> {
    pub fn new(vm: &'a Arc<BlinkVM>) -> Self {
        Self { vm }
    }

    pub fn get_pos(&self, value: ValueRef) -> Option<SourceRange> {
       self.vm.get_pos(value)
    }

    // === VALUE CREATION ===
    
    /// Create a number value
    pub fn number(&self, n: f64) -> ValueRef {
        ValueRef::number(n)
    }
    
    /// Create a boolean value
    pub fn boolean(&self, b: bool) -> ValueRef {
        ValueRef::boolean(b)
    }
    
    /// Create nil value
    pub fn nil(&self) -> ValueRef {
        ValueRef::nil()
    }
    
    /// Allocate a string value
    pub fn string(&self, s: &str) -> ValueRef {
        let object_ref = self.vm.alloc_str(s);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    
    /// Allocate a list value
    pub fn list(&self, items: Vec<ValueRef>) -> ValueRef {
        let object_ref = self.vm.alloc_vec_or_list(items, true);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    
    /// Allocate a vector value
    pub fn vector(&self, items: Vec<ValueRef>) -> ValueRef {
        let object_ref = self.vm.alloc_vec_or_list(items, false);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    
    /// Allocate a hash map value
    pub fn hash_map(&self, pairs: Vec<(ValueRef, ValueRef)>) -> ValueRef {
        let map = BlinkHashMap::from_pairs(pairs);
        let object_ref = self.vm.alloc_blink_hash_map(map);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    
    /// Allocate a hash set value
    pub fn hash_set(&self, items: Vec<ValueRef>) -> ValueRef {
        let set = BlinkHashSet::from_iter(items);
        let object_ref = self.vm.alloc_blink_hash_set(set);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    /// Allocate a future value
    pub fn future(&self) -> ValueRef {
        let future = BlinkFuture::new();
        let object_ref = self.vm.alloc_future(future);
        ValueRef::Heap(GcPtr::new(object_ref))
    }

    pub fn bool(&self, b: bool) -> ValueRef {
        ValueRef::boolean(b)
    }

    // === SYMBOL OPERATIONS ===
    
    /// Intern a symbol and return its ValueRef
    pub fn symbol(&self, name: &str) -> ValueRef {
        let symbol_id = self.vm.symbol_table.write().intern(name);
        ValueRef::symbol(symbol_id)
    }
    
    /// Intern a keyword and return its ValueRef
    pub fn keyword(&self, name: &str) -> ValueRef {
        let full_name = format!(":{}", name);
        let id = self.vm.symbol_table.write().intern(&full_name);
        ValueRef::keyword(id)
    }
    
    /// Get the string name of a symbol ID
    pub fn symbol_name(&self, symbol_id: u32) -> Option<String> {
        self.vm.symbol_table
            .read()
            .get_symbol(symbol_id)
            .map(|s| s.to_string())
    }
    
    /// Get symbol ID from a symbol ValueRef
    pub fn get_symbol_id(&self, value: ValueRef) -> Option<u32> {
        if let ValueRef::Immediate(packed) = value {
            if let crate::value::ImmediateValue::Symbol(id) = crate::value::unpack_immediate(packed) {
                return Some(id);
            }
        }
        None
    }

    // === ERROR HANDLING ===

    pub fn error(&self, error: BlinkError) -> ValueRef {
        let object_ref = self.vm.alloc_error(error);
        ValueRef::Heap(GcPtr::new(object_ref))
    }
    
    /// Create an error value
    pub fn eval_error(&self, message: &str) -> ValueRef {
        self.vm.eval_error(message)
    }
    
    /// Create an arity error value
    pub fn arity_error(&self, expected: usize, actual: usize, function_name: &str) -> ValueRef {
        let msg = format!("{} expects {} arguments, got {}", function_name, expected, actual);
        self.eval_error(&msg)
    }
    
    /// Create a type error value
    pub fn type_error(&self, expected: &str, actual: &str, function_name: &str) -> ValueRef {
        let msg = format!("{} expects {}, got {}", function_name, expected, actual);
        self.eval_error(&msg)
    }

    // === VALUE INSPECTION ===
    
    /// Check if a value is nil
    pub fn is_nil(&self, value: ValueRef) -> bool {
        matches!(value, ValueRef::Immediate(packed) 
            if matches!(crate::value::unpack_immediate(packed), crate::value::ImmediateValue::Nil))
    }
    
    /// Check if a value is truthy (everything except nil and false)
    pub fn is_truthy(&self, value: ValueRef) -> bool {
        value.is_truthy()
    }
    
    /// Get the type name of a value
    pub fn type_name(&self, value: ValueRef) -> &'static str {
        value.type_name()
    }
    
    /// Extract number from ValueRef
    pub fn get_number(&self, value: ValueRef) -> Option<f64> {
        if let ValueRef::Immediate(packed) = value {
            if let crate::value::ImmediateValue::Number(n) = crate::value::unpack_immediate(packed) {
                return Some(n);
            }
        }
        None
    }
    
    /// Extract boolean from ValueRef
    pub fn get_boolean(&self, value: ValueRef) -> Option<bool> {
        if let ValueRef::Immediate(packed) = value {
            if let crate::value::ImmediateValue::Bool(b) = crate::value::unpack_immediate(packed) {
                return Some(b);
            }
        }
        None
    }
    
    /// Extract string from ValueRef
    pub fn get_string(&self, value: ValueRef) -> Option<String> {
        value.get_string()
    }
    
    /// Extract list from ValueRef
    pub fn get_list(&self, value: ValueRef) -> Option<Vec<ValueRef>> {
        value.get_list()
    }
    
    /// Extract vector from ValueRef
    pub fn get_vector(&self, value: ValueRef) -> Option<Vec<ValueRef>> {
        value.get_vec()
    }

    // === COLLECTION UTILITIES ===
    
    /// Get the length of a collection (list, vector, string, map, set)
    pub fn count(&self, value: ValueRef) -> Option<usize> {
        match value {
            ValueRef::Heap(gc_ptr) => {
                match gc_ptr.to_heap_value() {
                    crate::value::HeapValue::List(items) => Some(items.len()),
                    crate::value::HeapValue::Vector(items) => Some(items.len()),
                    crate::value::HeapValue::Str(s) => Some(s.chars().count()),
                    crate::value::HeapValue::Map(map) => Some(map.len()),
                    crate::value::HeapValue::Set(set) => Some(set.len()),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    
    /// Check if a collection is empty
    pub fn is_empty(&self, value: ValueRef) -> bool {
        self.count(value).map_or(false, |c| c == 0)
    }
    
    /// Get first element of a list/vector
    pub fn first(&self, value: ValueRef) -> ValueRef {
        if let Some(list) = value.get_list() {
            list.first().copied().unwrap_or(self.nil())
        } else if let Some(vec) = value.get_vec() {
            vec.first().copied().unwrap_or(self.nil())
        } else {
            self.nil()
        }
    }
    
    /// Get rest of a list/vector (all elements except first)
    pub fn rest(&self, value: ValueRef) -> ValueRef {
        if let Some(list) = value.get_list() {
            let rest: Vec<_> = list.iter().skip(1).copied().collect();
            self.list(rest)
        } else if let Some(vec) = value.get_vec() {
            let rest: Vec<_> = vec.iter().skip(1).copied().collect();
            self.list(rest) // Note: returns list, not vector (consistent with Clojure)
        } else {
            self.list(vec![]) // Empty list for non-collections
        }
    }

    // === UTILITY FUNCTIONS ===
    
    /// Helper to validate argument count
    pub fn require_arity(&self, args: &[ValueRef], expected: usize, function_name: &str) -> Result<(), ValueRef> {
        if args.len() != expected {
            Err(self.arity_error(expected, args.len(), function_name))
        } else {
            Ok(())
        }
    }
    
    /// Helper to validate minimum argument count
    pub fn require_min_arity(&self, args: &[ValueRef], min: usize, function_name: &str) -> Result<(), ValueRef> {
        if args.len() < min {
            Err(self.arity_error(min, args.len(), &format!("{} (at least)", function_name)))
        } else {
            Ok(())
        }
    }
    
    /// Helper to extract numbers from all arguments
    pub fn extract_numbers(&self, args: &[ValueRef], function_name: &str) -> Result<Vec<f64>, ValueRef> {
        args.iter()
            .map(|&arg| self.get_number(arg)
                .ok_or_else(|| self.type_error("number", self.type_name(arg), function_name)))
            .collect()
    }
}