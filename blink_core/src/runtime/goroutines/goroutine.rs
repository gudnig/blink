use crate::{value::GcPtr, CallFrame, FunctionRef, TypeTag, ValueRef};


pub type GoroutineId = u32;

// === Goroutine State ===

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoroutineState {
    Ready,     // Can be scheduled
    Running,   // Currently executing
    Blocked,   // Waiting on future/channel
    Completed, // Finished execution
}

#[derive(Debug)]
pub struct Goroutine {
    pub id: u32,
    pub state: GoroutineState,
    pub call_stack: Vec<crate::runtime::execution_context::CallFrame>,
    pub register_stack: Vec<ValueRef>,
    pub current_module: u32,
    pub instruction_pointer: usize,
}

impl Goroutine {
    pub fn new(id: u32, initial_function: ValueRef) -> Result<Self, String> {
        // Create initial call frame from the function
        let call_frame = Self::create_initial_frame(initial_function)?;
        let current_module = call_frame.current_module;

        // Allocate registers for the initial function
        let mut register_stack = Vec::new();
        let reg_count = match &call_frame.func {
            crate::runtime::execution_context::FunctionRef::CompiledFunction(compiled_fn, _) => {
                compiled_fn.register_count as usize
            }
            crate::runtime::execution_context::FunctionRef::Closure(closure_obj, _) => {
                let template_fn = GcPtr::new(closure_obj.template).read_callable();
                template_fn.register_count as usize
            }
            crate::runtime::execution_context::FunctionRef::Native(_) => {
                1 // Native functions need at least 1 register for return value
            }
        };

        // Pre-allocate registers with nil values (same as normal execution)
        for _ in 0..reg_count {
            register_stack.push(ValueRef::nil());
        }

        Ok(Self {
            id,
            state: GoroutineState::Ready,
            call_stack: vec![call_frame],
            register_stack,
            current_module,
            instruction_pointer: 0,
        })
    }


    fn create_initial_frame(func_value: ValueRef) -> Result<CallFrame, String> {
        match func_value {
            ValueRef::Heap(heap) => {
                let type_tag = heap.type_tag();
                let obj_ref = heap.0;
                match type_tag {
                    TypeTag::UserDefinedFunction | TypeTag::Macro => {
                        let compiled_func = heap.read_callable();
                        let module = compiled_func.module;
                        Ok(CallFrame {
                            func: FunctionRef::CompiledFunction(compiled_func, Some(obj_ref)),
                            pc: 0,
                            reg_start: 0,
                            reg_count: 0, // Will be set when registers are allocated
                            current_module: module,
                        })
                    }
                    TypeTag::Closure => {
                        let closure_obj = heap.read_closure();
                        let template_fn = GcPtr::new(closure_obj.template).read_callable();
                        let module = template_fn.module;
                        Ok(CallFrame {
                            func: FunctionRef::Closure(closure_obj, Some(obj_ref)),
                            pc: 0,
                            reg_start: 0,
                            reg_count: 0, // Will be set when registers are allocated
                            current_module: module,
                        })
                    }
                    _ => Err(format!(
                        "Invalid function value for goroutine: {:?}",
                        func_value
                    )),
                }
            }
            ValueRef::Handle(native) => {
                if func_value.is_native_fn() {
                    return Err(format!(
                        "Invalid function value for goroutine: {:?}",
                        func_value
                    ));
                }
                Ok(CallFrame {
                    func: FunctionRef::Native(native),
                    pc: 0,
                    reg_start: 0,
                    reg_count: 0,      // Will be set when registers are allocated
                    current_module: 0, // Native functions don't have modules
                })
            }
            _ => Err(format!(
                "Invalid function value for goroutine: {:?}",
                func_value
            )),
        }
    }
}