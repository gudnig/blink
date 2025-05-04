use std::sync::Arc;

pub use blink_core::{BlinkValue, Env, Value};
use parking_lot::RwLock;
pub fn register_fn(
    env: &mut Env,
    name: &str,
    f: fn(Vec<BlinkValue>) -> Result<BlinkValue, String>,
) {
    let node = blink_core::LispNode {
        value: Value::NativeFunc(f),
        pos: None,
    };
    env.set(name, BlinkValue(Arc::new(RwLock::new(node))));
}
