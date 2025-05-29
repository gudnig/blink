use std::sync::Arc;
use parking_lot::RwLock;

pub use blink_core::{BlinkValue, Env, Value, LispNode};

/// Register a single function and return its name for export tracking
pub fn register_fn(
    env: &mut Env,
    name: &str,
    f: fn(Vec<BlinkValue>) -> Result<BlinkValue, String>,
) -> String {
    let node = LispNode {
        value: Value::NativeFunc(f),
        pos: None,
    };
    env.set(name, BlinkValue(Arc::new(RwLock::new(node))));
    name.to_string()
}

/// Register multiple functions at once and return all their names
pub fn register_functions(
    env: &mut Env,
    functions: &[(&str, fn(Vec<BlinkValue>) -> Result<BlinkValue, String>)]
) -> Vec<String> {
    functions.iter()
        .map(|(name, func)| {
            register_fn(env, name, *func)
        })
        .collect()
}

/// Register a single value (non-function) and return its name
pub fn register_value(
    env: &mut Env,
    name: &str,
    value: BlinkValue,
) -> String {
    env.set(name, value);
    name.to_string()
}

/// Register multiple values at once
pub fn register_values(
    env: &mut Env,
    values: &[(&str, BlinkValue)]
) -> Vec<String> {
    values.iter()
        .map(|(name, value)| {
            register_value(env, name, value.clone())
        })
        .collect()
}

/// Convenience macro for registering multiple functions
#[macro_export]
macro_rules! register_module {
    ($env:expr, $($name:expr => $func:expr),+ $(,)?) => {
        {
            let functions: &[(&str, fn(Vec<BlinkValue>) -> Result<BlinkValue, String>)] = &[
                $(($name, $func as fn(Vec<BlinkValue>) -> Result<BlinkValue, String>)),+
            ];
            $crate::register_functions($env, functions)
        }
    };
}
/// Convenience macro for declaring a native module
#[macro_export]
macro_rules! native_module {
    (
        $(#[$attr:meta])*
        fn $register_name:ident($env:ident: &mut Env) -> Vec<String> {
            $($name:expr => $func:expr),+ $(,)?
        }
    ) => {
        $(#[$attr])*
        #[no_mangle]
        pub extern "C" fn $register_name($env: &mut $crate::Env) -> Vec<String> {
            $crate::register_module!($env, $($name => $func),+)
        }
    };
}

/// Create common Blink values for native modules
pub mod values {
    use super::*;
    
    pub fn number(n: f64) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Number(n),
            pos: None,
        })))
    }
    
    pub fn string(s: &str) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Str(s.to_string()),
            pos: None,
        })))
    }
    
    pub fn boolean(b: bool) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Bool(b),
            pos: None,
        })))
    }
    
    pub fn nil() -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Nil,
            pos: None,
        })))
    }
    
    pub fn symbol(s: &str) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Symbol(s.to_string()),
            pos: None,
        })))
    }
    
    pub fn keyword(k: &str) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Keyword(k.to_string()),
            pos: None,
        })))
    }
    
    pub fn list(items: Vec<BlinkValue>) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::List(items),
            pos: None,
        })))
    }
    
    pub fn vector(items: Vec<BlinkValue>) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Vector(items),
            pos: None,
        })))
    }
}

/// Helper functions for extracting values from BlinkValue
pub mod extract {
    use super::*;
    
    pub fn number(val: &BlinkValue) -> Result<f64, String> {
        match &val.read().value {
            Value::Number(n) => Ok(*n),
            _ => Err("Expected number".to_string()),
        }
    }
    
    pub fn string(val: &BlinkValue) -> Result<String, String> {
        match &val.read().value {
            Value::Str(s) => Ok(s.clone()),
            _ => Err("Expected string".to_string()),
        }
    }
    
    pub fn boolean(val: &BlinkValue) -> Result<bool, String> {
        match &val.read().value {
            Value::Bool(b) => Ok(*b),
            _ => Err("Expected boolean".to_string()),
        }
    }
    
    pub fn symbol(val: &BlinkValue) -> Result<String, String> {
        match &val.read().value {
            Value::Symbol(s) => Ok(s.clone()),
            _ => Err("Expected symbol".to_string()),
        }
    }
    
    pub fn list(val: &BlinkValue) -> Result<Vec<BlinkValue>, String> {
        match &val.read().value {
            Value::List(items) => Ok(items.clone()),
            _ => Err("Expected list".to_string()),
        }
    }
    
    pub fn vector(val: &BlinkValue) -> Result<Vec<BlinkValue>, String> {
        match &val.read().value {
            Value::Vector(items) => Ok(items.clone()),
            _ => Err("Expected vector".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_register_functions() {
        let mut env = Env::new();
        
        fn test_add(args: Vec<BlinkValue>) -> Result<BlinkValue, String> {
            Ok(values::number(42.0))
        }
        
        fn test_multiply(args: Vec<BlinkValue>) -> Result<BlinkValue, String> {
            Ok(values::number(100.0))
        }
        
        let exports = register_functions(&mut env, &[
            ("add", test_add),
            ("multiply", test_multiply),
        ]);
        
        assert_eq!(exports, vec!["add", "multiply"]);
        assert!(env.get_local("add").is_some());
        assert!(env.get_local("multiply").is_some());
    }
    
    
    #[test]
    fn test_macro_registration() {
        let mut env = Env::new();
        
        fn dummy_func(_args: Vec<BlinkValue>) -> Result<BlinkValue, String> {
            Ok(values::nil())
        }
        
        let exports = register_module!(&mut env,
            "test1" => dummy_func,
            "test2" => dummy_func,
        );
        
        assert_eq!(exports.len(), 2);
        assert!(exports.contains(&"test1".to_string()));
        assert!(exports.contains(&"test2".to_string()));
    }
}