use std::hash::{Hash, Hasher};

use crate::{env::Env, collections::{BlinkHashMap, BlinkHashSet}, error::BlinkError, eval::{EvalContext, EvalResult}, future::BlinkFuture, value::{IsolatedValue, Macro, ModuleRef, NativeFn, UserDefinedFn, ValueRef} };

#[derive(Debug)]
pub enum HeapValue {

    
    List(Vec<ValueRef>),
    Vector(Vec<ValueRef>),
    Map(BlinkHashMap),
    Str(String),
    Set(BlinkHashSet),

    Error(BlinkError),
    NativeFunction(NativeFn),
    Module(ModuleRef),
    UserDefinedFunction(UserDefinedFn),
    Macro(Macro),
    
    Future(BlinkFuture),
    Env(Env),
    
}


impl Hash for HeapValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            HeapValue::Str(s) => {
                "string".hash(state);
                s.hash(state);
            }
            HeapValue::List(value_refs) => {
                "list".hash(state);
                value_refs.len().hash(state);
                for item in value_refs {
                    item.hash(state);
                }
            }
            HeapValue::Vector(value_refs) => {
                "vector".hash(state);
                value_refs.len().hash(state);
                for item in value_refs {
                    item.hash(state);
                }
            }
            HeapValue::Map(blink_hash_map) => {
                "map".hash(state);
                blink_hash_map.len().hash(state);
                for (key, value) in blink_hash_map {
                    key.hash(state);
                    value.hash(state);
                }
            }
            HeapValue::Set(blink_hash_set) => {
                "set".hash(state);
                blink_hash_set.len().hash(state);
                for item in blink_hash_set {
                    item.hash(state);
                }
            }
            HeapValue::Error(blink_error) => {
                "error".hash(state);
                blink_error.error_type.hash(state);
                blink_error.message.hash(state);
            }
            HeapValue::NativeFunction(native_fn) => {
                "native-function".hash(state);
                match native_fn {
                    NativeFn::Isolated(func) => {
                        "isolated-native-function".hash(state);
                        (func.as_ref() as *const dyn Fn(Vec<IsolatedValue>) -> Result<IsolatedValue, String> as *const () as usize).hash(state);
                    }
                    NativeFn::Contextual(func) => {
                        "contextual-native-function".hash(state);
                        (func.as_ref() as *const dyn Fn(Vec<ValueRef>, &mut EvalContext) -> EvalResult as *const () as usize).hash(state);
                    }
                }
            }
            HeapValue::UserDefinedFunction(user_defined_fn) => {
                "user-defined-function".hash(state);
                user_defined_fn.params.hash(state);
                user_defined_fn.body.len().hash(state);
                for expr in &user_defined_fn.body {
                    expr.hash(state);
                }
            }
            HeapValue::Macro(mac) => {
                "macro".hash(state);
                mac.params.hash(state);
                mac.is_variadic.hash(state);
                mac.body.len().hash(state);
                for expr in &mac.body {
                    expr.hash(state);
                }
            }
            HeapValue::Future(blink_future) => {
                todo!()
            }
            HeapValue::Env(env) => {
                "env".hash(state);
                env.len().hash(state);
                env.hash(state);
            }
            HeapValue::Module(module) => {
                "module".hash(state);
                module.hash(state);
            }
        };
}
}

impl HeapValue {
    pub fn type_tag(&self) -> &'static str  {
        match self {
            HeapValue::Str(_) => "string",
            HeapValue::Error(_) => "error",
            HeapValue::Future(_) => "future",
            HeapValue::NativeFunction(_) => "native-function",
            HeapValue::Module(_) => "module",
            HeapValue::UserDefinedFunction(_) => "user-defined-function",
            HeapValue::Macro(_) => "macro",
            _ => todo!(),
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            HeapValue::Error(_) => true,
            _ => false,
        }
    }
}