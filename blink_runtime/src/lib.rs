pub use blink_core::{Env, BlinkValue, Value}; 
pub fn register_fn(
    env: &mut Env,
    name: &str,
    f: fn(Vec<BlinkValue>) -> Result<BlinkValue, String>
) {
    use std::rc::Rc;
    use std::cell::RefCell;
    let node = blink_core::LispNode {
        value: Value::NativeFunc(f),
        pos: None,
    };
    env.set(name, BlinkValue(Rc::new(RefCell::new(node))));
}
