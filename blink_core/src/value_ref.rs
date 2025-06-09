pub enum ValueRef {
    Immediate(u64),           // packed small values
    Gc(GcPtr),               // GC heap objects (coming later)
    Shared(SharedValueRef),   // reference counted boundary objects
}

// Your old Value enum lives inside SharedValue now
pub enum SharedValue {
    
    // Lisp data that needs to be shared
    List(Vec<ValueRef>),
    Map(HashMap<ValueRef, ValueRef>),
    Str(String),
    
    // Runtime objects
    Future(BlinkFuture),
    NativeFunction(NativeFn),
    Module(ModuleData),
    
    // Legacy: until we implement GC heap
    Number(f64),
    Bool(bool),
}

pub type SharedValueRef = Arc<RwLock<SharedValue>>;