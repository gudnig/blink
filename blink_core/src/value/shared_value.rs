use std::{collections::{HashMap, HashSet}, fmt::Display, hash::{Hash, Hasher}};

use crate::{collections::{BlinkHashMap, BlinkHashSet, ContextualValueRef, ValueContext}, error::BlinkError, eval::{EvalContext, EvalResult}, future::BlinkFuture, value::{IsolatedValue, Macro, ModuleRef, NativeFn, UserDefinedFn, ValueRef} };

#[derive(Debug)]
pub enum SharedValue {

    // Data that needs to be shared but eventaully will be moved to the GC heap
    List(Vec<ValueRef>),
    Vector(Vec<ValueRef>),
    Map(BlinkHashMap),
    Str(String),
    Set(BlinkHashSet),
    Error(BlinkError),
    
    
    // Runtime objects
    Future(BlinkFuture),
    NativeFunction(NativeFn),
    Module(ModuleRef),
    UserDefinedFunction(UserDefinedFn),
    Macro(Macro),
    //TODO env could be a reference to a shared value

}

impl SharedValue {
    pub fn display_with_context(&self, f: &mut std::fmt::Formatter<'_>, context: ValueContext) -> std::fmt::Result {
        match self {
            SharedValue::List(value_refs) => {
                        write!(f, "(")?;
                        for (i, value_ref) in value_refs.iter().enumerate() {
                            if i > 0 {
                                write!(f, " ")?;
                            }
                            // Create a ContextualValueRef to display nested values
                            let contextual = ContextualValueRef::new(value_ref.clone(), context.clone());
                            write!(f, "{}", contextual)?;
                        }
                        write!(f, ")")
                    }
            SharedValue::Vector(value_refs) => {
                        write!(f, "[")?;
                        for (i, value_ref) in value_refs.iter().enumerate() {
                            if i > 0 {
                                write!(f, " ")?;
                            }
                            let contextual = ContextualValueRef::new(value_ref.clone(), context.clone());
                            write!(f, "{}", contextual)?;
                        }
                        write!(f, "]")
                    }
            SharedValue::Map(hash_map) => {
                write!(f, "{}", hash_map)
                    }
            SharedValue::Str(s) => write!(f, "\"{}\"", s),
            SharedValue::Future(_) => write!(f, "#<future>"),
            SharedValue::NativeFunction(_) => write!(f, "#<native-fn>"),
            SharedValue::Module(_) => write!(f, "#<module>"),
            SharedValue::Set(blink_hash_set) => {
                write!(f, "{}", blink_hash_set)
            },
            
            SharedValue::Error(blink_error) => {
                write!(f, "#<error: ")?;
                blink_error.error_type.display_with_context(f, &context)?;
                write!(f, ": {}", blink_error.message)?;
                write!(f, ">")
            },
            SharedValue::UserDefinedFunction(user_defined_fn) => {
                write!(f, "#<user-defined-fn>")
            },
            SharedValue::Macro(_) => {
                write!(f, "#<macro>")
            },
        }
    }
}

impl SharedValue {
    pub fn type_tag(&self) -> &'static str  {
        match self {
            SharedValue::Str(_) => "string",
            SharedValue::Error(_) => "error",
            SharedValue::Future(_) => "future",
            SharedValue::NativeFunction(_) => "native-function",
            SharedValue::Module(_) => "module",
            SharedValue::UserDefinedFunction(_) => "user-defined-function",
            SharedValue::Macro(_) => "macro",
            _ => todo!(),
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            SharedValue::Error(_) => true,
            _ => false,
        }
    }


    pub fn hash_with_context<H: Hasher>(&self, state: &mut H, context: &ValueContext) {
        match self {
            SharedValue::Str(s) => {
                "string".hash(state);
                s.hash(state);
            }

            SharedValue::List(items) => {
                "list".hash(state);
                items.len().hash(state);
                for item in items {
                    item.hash_with_context(state, context);
                }
            }
            SharedValue::Vector(items) => {
                "vector".hash(state);
                items.len().hash(state);
                for item in items {
                    item.hash_with_context(state, context);
                }
            }
            SharedValue::Map(map) => {
                "map".hash(state);
                map.len().hash(state);
                // Maps are unordered, so we need order-independent hashing
                let mut pairs: Vec<_> = map.iter().collect();
                pairs.sort_by(|a, b| {
                    // Sort by hash of key for deterministic ordering
                    let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
                    let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
                    a.0.hash_with_context(&mut hasher_a, context);
                    b.0.hash_with_context(&mut hasher_b, context);
                    hasher_a.finish().cmp(&hasher_b.finish())
                });
                for (k, v) in pairs {
                    k.hash_with_context(state, context);
                    v.hash_with_context(state, context);
                }
            }
            SharedValue::Set(set) => {
                "set".hash(state);
                set.len().hash(state);
                // Sets are unordered, collect and sort for deterministic hashing
                let mut items: Vec<_> = set.iter().collect();
                items.sort_by(|a, b| {
                    let mut hasher_a = std::collections::hash_map::DefaultHasher::new();
                    let mut hasher_b = std::collections::hash_map::DefaultHasher::new();
                    a.hash_with_context(&mut hasher_a, context);
                    b.hash_with_context(&mut hasher_b, context);
                    hasher_a.finish().cmp(&hasher_b.finish())
                });
                for item in items {
                    item.hash_with_context(state, context);
                }
            }

            SharedValue::Error(e) => {
                "error".hash(state);
                // Hash the error message/type
                format!("{:?}", e).hash(state);
            }
            SharedValue::Future(_) => {
                "future".hash(state);
                // TODO: implement future hashing (maybe use an ID?)
                "future-placeholder".hash(state);
            }
            SharedValue::NativeFunction(f) => {
                "native-func".hash(state);
                match f {
                    NativeFn::Isolated(func) => {
                        "isolated-native-func".hash(state);
                        // Hash by memory address since we can't hash the trait object
                        (func.as_ref() as *const dyn Fn(Vec<IsolatedValue>) -> Result<IsolatedValue, String> as *const () as usize).hash(state);
                    }
                    NativeFn::Contextual(func) => {
                        "contextual-native-func".hash(state);
                        // Hash by memory address
                        (func.as_ref() as *const dyn Fn(Vec<ValueRef>, &mut EvalContext) -> EvalResult as *const () as usize).hash(state);
                    }
                }
            }
            SharedValue::Module(module_ref) => {
                "module-ref".hash(state);
                module_ref.module.hash(state);
                module_ref.symbol.hash(state);
            }
            SharedValue::UserDefinedFunction(func) => {
                "user-func".hash(state);
                func.params.hash(state);
                func.body.len().hash(state);
                for expr in &func.body {
                    expr.hash_with_context(state, context);
                }
                // Note: we're not hashing the environment for now
            }
            SharedValue::Macro(mac) => {
                "macro".hash(state);
                mac.params.hash(state);
                mac.is_variadic.hash(state);
                mac.body.len().hash(state);
                for expr in &mac.body {
                    expr.hash_with_context(state, context);
                }
                // Note: we're not hashing the environment for now
            }
        }
    }

    /// Check equality with another SharedValue
    pub fn eq_with_context(&self, other: &Self, context: &ValueContext) -> bool {
        match (self, other) {
            
            (SharedValue::Str(a), SharedValue::Str(b)) => a == b,
            (SharedValue::Error(a), SharedValue::Error(b)) => a.message == b.message,
            (SharedValue::List(a), SharedValue::List(b)) => {
                a.len() == b.len() && 
                a.iter().zip(b.iter()).all(|(x, y)| x.eq_with_context(y, context))
            }
            (SharedValue::Vector(a), SharedValue::Vector(b)) => {
                a.len() == b.len() && 
                a.iter().zip(b.iter()).all(|(x, y)| x.eq_with_context(y, context))
            }
            (SharedValue::Map(a), SharedValue::Map(b)) => {
                a.len() == b.len() && 
                a.iter().all(|(k, v)| {
                    b.iter().any(|(k2, v2)| {
                        k.eq_with_context(k2, context) && v.eq_with_context(v2, context)
                    })
                })
            }
            (SharedValue::Set(a), SharedValue::Set(b)) => {
                a.len() == b.len() && 
                a.iter().all(|item| {   
                    b.iter().any(|item2| item.eq_with_context(item2, context))
                })
            }
            
            (SharedValue::NativeFunction(a), SharedValue::NativeFunction(b)) => {
                // Function pointer equality
                std::ptr::eq(a as *const _, b as *const _)
            }
            
            (SharedValue::UserDefinedFunction(a), SharedValue::UserDefinedFunction(b)) => {
                // Functions equal if same params and body
                // (ignoring environment for now)
                a.params == b.params && 
                a.body.len() == b.body.len() &&
                a.body.iter().zip(b.body.iter()).all(|(x, y)| x.eq_with_context(y, context))
            }
            
            (SharedValue::Module(a), SharedValue::Module(b)) => {
                a.module == b.module && a.symbol == b.symbol
            }
            
            (SharedValue::Macro(a), SharedValue::Macro(b)) => {
                a.params == b.params && 
                a.is_variadic == b.is_variadic &&
                a.body.len() == b.body.len() &&
                a.body.iter().zip(b.body.iter()).all(|(x, y)| x.eq_with_context(y, context))
            }
            
            // Different types are never equal
            _ => false
        }
    }
}