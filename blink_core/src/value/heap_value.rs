use std::{fmt::{self, Display}, hash::{Hash, Hasher}};

use crate::{
    collections::{BlinkHashMap, BlinkHashSet}, env::Env, error::BlinkError, future::BlinkFuture, module::Module, runtime::{ClosureObject, CompiledFunction}, value::{Callable, IsolatedValue, ModuleRef, NativeFn, ValueRef}
};

#[derive(Debug)]
pub enum HeapValue {
    List(Vec<ValueRef>),
    Vector(Vec<ValueRef>),
    Map(BlinkHashMap),
    Str(String),
    Set(BlinkHashSet),
    Error(BlinkError),
    Function(CompiledFunction),
    Macro(CompiledFunction),
    Closure(ClosureObject),
    Future(BlinkFuture),
    Env(Env),
}

impl Display for HeapValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeapValue::Str(s) => write!(f, "{}", s),
            HeapValue::List(value_refs) => {
                                                        write!(f, "(")?;
                                                        for value_ref in value_refs {
                                                            write!(f, "{} ", value_ref)?;
                                                        }
                                                        write!(f, ")")?;
                                                        Ok(())
                                                    },
            HeapValue::Vector(value_refs) => {
                                                        write!(f, "[")?;
                                                        for value_ref in value_refs {
                                                            write!(f, "{} ", value_ref)?;
                                                        }
                                                        write!(f, "]")?;
                                                        Ok(())
                                                    },
            HeapValue::Map(blink_hash_map) => {
                                                        write!(f, "{{")?;
                                                        for (key, value) in blink_hash_map.iter() {
                                                            write!(f, "{}: {}\n", key, value)?;
                                                        }
                                                        write!(f, "}}")?;
                                                        Ok(())
                                                    },
            HeapValue::Set(blink_hash_set) => {
                                                        write!(f, "#{{")?;
                                                        for value in blink_hash_set.iter() {
                                                            write!(f, "{} ", value)?;
                                                        }
                                                        write!(f, "}}")?;
                                                        Ok(())
                                                    },
            HeapValue::Function(callable) => {
                                                        write!(f, "function")?;
                                                        Ok(())
                                                    },
            HeapValue::Future(blink_future) => {
                                                        write!(f, "future")?;
                                                        Ok(())
                                                    },
            HeapValue::Env(env) => {
                                                        write!(f, "env")?;
                                                        Ok(())
                                                    },
            HeapValue::Error(blink_error) => {
                                                        write!(f, "error: {} {}", blink_error.error_type, blink_error.message)?;
                                                        Ok(())
                                                    },
            HeapValue::Closure(closure_object) => {
                                                        write!(f, "closure")?;
                                                        Ok(())
                                                    },
            HeapValue::Macro(compiled_function) => {
                                                        write!(f, "macro")?;
                                                        Ok(())
                                                    },
        }
    }
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
                                user_defined_fn.parameter_count.hash(state);
                                user_defined_fn.bytecode.len().hash(state);
                                user_defined_fn.constants.len().hash(state);
                                for constant in &user_defined_fn.constants {
                                    constant.hash(state);
                                }
                                user_defined_fn.bytecode.hash(state);
                            }
            HeapValue::Macro(macro_fn) => {
                                "macro".hash(state);
                                macro_fn.parameter_count.hash(state);
                                macro_fn.bytecode.len().hash(state);
                                macro_fn.constants.len().hash(state);
                                for constant in &macro_fn.constants {
                                    constant.hash(state);
                                }
                                macro_fn.bytecode.hash(state);
                            }
            HeapValue::Closure(closure_object) => {
                                "closure".hash(state);
                                closure_object.template.hash(state);
                                closure_object.upvalues.len().hash(state);
                                for upvalue in &closure_object.upvalues {
                                    upvalue.hash(state);
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
            (HeapValue::Error(_arc_mod), HeapValue::Error(_)) => {
                panic!("Should have happened already")
            },
            (HeapValue::Function(_), HeapValue::Function(_)) => {
                panic!("Should have happened already")
                
            },
            (HeapValue::Future(_), HeapValue::Future(_)) => {
                panic!("Should have happened already")
            }
            (HeapValue::Env(_), HeapValue::Env(_)) => {
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
            HeapValue::Closure(_) => "closure",
            HeapValue::Future(_) => "future",
            HeapValue::Env(_) => "env",
            HeapValue::Macro(_) => "macro",
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            HeapValue::Error(_) => true,
            _ => false,
        }
    }
}
