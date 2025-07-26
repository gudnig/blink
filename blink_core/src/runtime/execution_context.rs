use std::sync::Arc;

use mmtk::util::ObjectReference;

use crate::{eval::EvalResult, runtime::{BlinkVM, ContextualBoundary, SpecialFormId, TypeTag, ValueBoundary}, value::{self, unpack_immediate, ContextualNativeFn, GcPtr, IsolatedNativeFn, NativeContext, NativeFn}, Env, HeapValue, ImmediateValue, ValueRef};


#[derive(Clone)]
struct ExecutionSnapshot {
    call_stack: Vec<CallFrame>,
    register_stack: Vec<ValueRef>,
    current_module: u32,
    pc: usize,
}


#[derive(Clone, Debug)]
pub enum FunctionRef {
    UserDefined(ObjectReference),       // GC-managed Callable
    Native(usize),               // Raw pointer to boxed function
    SpecialForm(SpecialFormId),        // Just an enum ID
    Macro(ObjectReference)
}

static DEFAULT_REG_COUNT: usize = 1024;
static MAX_REGISTERS: usize = 1024 * 1024;




#[derive(Clone, Debug)]
pub struct CallFrame {
    pub func: FunctionRef,
    pub pc: usize,                   // Program counter for bytecode
    pub reg_start: usize,            // Index into register_stack
    pub reg_count: u8,               // Number of registers for this frame
    pub current_module: u32,
    pub symbol_bindings: Vec<(u32, u8)>, // (symbol_id, register_offset)
    // For bytecode: upvalues will be stored differently
    pub upvalues: Vec<ValueRef>,     // Captured variables (Lua-style)
}


impl CallFrame {
    fn new(func: FunctionRef, pc: usize, reg_start: usize, reg_count: u8, current_module: u32, symbol_bindings: Vec<(u32, u8)>) -> Self {
        CallFrame { func, pc, reg_start, reg_count, current_module, symbol_bindings, upvalues: vec![] }
    }


}


#[derive(Clone, Debug)]
pub struct ExecutionContext {
    pub vm: Arc<BlinkVM>,
    current_module: u32,
    register_stack: Vec<ValueRef>,
    call_stack: Vec<CallFrame>,
    register_count: usize,
}

impl ExecutionContext {
    pub fn new(vm: Arc<BlinkVM>) -> Self {
        ExecutionContext { vm, current_module: 0, register_stack: vec![], call_stack: vec![], register_count: 0 }
    }
    pub fn get_stack_roots(&self) -> Vec<ObjectReference> {
        let mut roots = vec![];
        for frame in self.call_stack.iter() {
            match frame.func {
                FunctionRef::UserDefined(func)  | FunctionRef::Macro(func) => roots.push(func),
                _ => (),
            }
        }

        for value in self.register_stack.iter() {
            if let ValueRef::Heap(gc_ptr) = value {
                roots.push(gc_ptr.0);
            }
        }
        roots
    }
}