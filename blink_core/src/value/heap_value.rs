use std::hash::{Hash, Hasher};

use crate::{
    collections::{BlinkHashMap, BlinkHashSet},
    error::BlinkError,
    eval::{EvalContext, EvalResult},
    future::BlinkFuture,
    value::{Callable, IsolatedValue, ModuleRef, NativeFn, ValueRef},
    Env,
};

#[derive(Debug)]
pub enum HeapValue {
    List(Vec<ValueRef>),
    Vector(Vec<ValueRef>),
    Map(BlinkHashMap),
    Str(String),
    Set(BlinkHashSet),
    Error(BlinkError),
    Function(Callable),
    Macro(Callable),
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
                for (key, value) in blink_hash_map.iter() {
                    key.hash(state);
                    value.hash(state);
                }
            }
            HeapValue::Set(blink_hash_set) => {
                "set".hash(state);
                blink_hash_set.len().hash(state);
                for item in blink_hash_set.iter() {
                    item.hash(state);
                }
            }
            HeapValue::Error(blink_error) => {
                "error".hash(state);
                blink_error.error_type.hash(state);
                blink_error.message.hash(state);
            }

            HeapValue::Function(user_defined_fn) => {
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
                env.vars.len().hash(state);
                for (key, value) in env.vars.iter() {
                    key.hash(state);
                    value.hash(state);
                }
            }
        }
    }
}

impl PartialEq for HeapValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (HeapValue::Str(s), HeapValue::Str(other_s)) => s == other_s,
            (HeapValue::List(value_refs), HeapValue::List(other_value_refs)) => value_refs == other_value_refs,
            (HeapValue::Vector(value_refs), HeapValue::Vector(other_value_refs)) => {
                value_refs.len() == other_value_refs.len() && value_refs.iter().zip(other_value_refs.iter()).all(|(a, b)| a == b)
            },
            (HeapValue::Map(blink_hash_map), HeapValue::Map(other_blink_hash_map)) => {
                blink_hash_map.len() == other_blink_hash_map.len() && blink_hash_map.iter().zip(other_blink_hash_map.iter()).all(|(a, b)| a == b)
            },
            (HeapValue::Set(blink_hash_set), HeapValue::Set(other_blink_hash_set)) => {
                blink_hash_set.len() == other_blink_hash_set.len() && blink_hash_set.iter().zip(other_blink_hash_set.iter()).all(|(a, b)| a == b)
            },
            (HeapValue::Error(blink_error), HeapValue::Error(other_blink_error)) => {
                panic!("Should have happened already")
            },
            (HeapValue::Function(user_defined_fn), HeapValue::Function(other_user_defined_fn)) => {
                panic!("Should have happened already")
                
            },
            (HeapValue::Macro(mac), HeapValue::Macro(other_mac)) => {
                panic!("Should have happened already")
            },
            (HeapValue::Future(blink_future), HeapValue::Future(other_blink_future)) => {
                panic!("Should have happened already")
            }
            (HeapValue::Env(env), HeapValue::Env(other_env)) => {
                panic!("Should have happened already")
            }
            _ => false,
        }
    }
}

impl Eq for HeapValue {}

impl HeapValue {
    pub fn type_tag(&self) -> &'static str {
        match self {
            HeapValue::List(_) => "list",
            HeapValue::Vector(_) => "vector",
            HeapValue::Map(_) => "map",
            HeapValue::Str(_) => "string",
            HeapValue::Set(_) => "set",
            HeapValue::Error(_) => "error",
            HeapValue::Function(_) => "function",
            HeapValue::Macro(_) => "macro",
            HeapValue::Future(_) => "future",
            HeapValue::Env(_) => "env",
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            HeapValue::Error(_) => true,
            _ => false,
        }
    }
}
