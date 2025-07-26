// use crate::{eval::EvalContext, future::BlinkFuture, value::ValueRef};

// pub enum EvalResult {
//     Value(ValueRef),

//     Suspended {
//         future: BlinkFuture,
//         resume: Box<dyn FnOnce(ValueRef, &mut EvalContext) -> EvalResult + Send>,
//     },
// }

// fn ok(val: ValueRef) -> EvalResult {
//     EvalResult::Value(val)
// }
