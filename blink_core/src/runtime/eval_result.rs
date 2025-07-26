use crate::{runtime::CallFrame, value::ValueRef};



pub struct SupendedState {
    pub call_stack: Vec<CallFrame>,
    pub register_stack: Vec<ValueRef>,
    pub current_module: u32,
    pub suspended_frame_idx: usize,
    pub suspended_pc: usize,
}

pub enum EvalResult {
    Value(ValueRef),
    Suspended(SupendedState),
}
