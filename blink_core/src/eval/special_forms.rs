use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
};

use libloading::Library;
use mmtk::util::ObjectReference;
use parking_lot::RwLock;

use crate::{
    env::Env,
    error::BlinkError,
    eval::{eval_func, forward_eval, result::EvalResult, trace_eval, try_eval, EvalContext},
    module::{Module, SerializedModuleSource},
    runtime::AsyncContext,
    value::{unpack_immediate, Callable, GcPtr, HeapValue, ImmediateValue, Plugin, ValueRef},
};

fn require_arity(args: &[ValueRef], expected: usize, form_name: &str) -> Result<(), BlinkError> {
    if args.len() != expected {
        return Err(BlinkError::arity(expected, args.len(), form_name));
    }
    Ok(())
}



pub fn eval_quote(args: &Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(&args[1..], 1, "quote") {
        return EvalResult::Value(ctx.error_value(e));
    }
    EvalResult::Value(args[1])
}

pub fn eval_if(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(ctx.eval_error("if expects at least 2 arguments"));
    }
    let condition = try_eval!(trace_eval(args[0].clone(), ctx), ctx);
    let is_truthy = condition.is_truthy();
    if is_truthy {
        forward_eval!(trace_eval(args[1].clone(), ctx), ctx)
    } else if args.len() > 2 {
        forward_eval!(trace_eval(args[2].clone(), ctx), ctx)
    } else {
        EvalResult::Value(ValueRef::nil())
    }
}
pub fn eval_def(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "def"));
    }

    // Extract symbol name from first argument
    let sym = match ctx.get_symbol_id(args[0]) {
        Some(sym) => sym,
        None => {
            return EvalResult::Value(ctx.eval_error("def first argument must be a symbol"));
        }
    };

    // Evaluate the second argument (the value to bind)
    let value = try_eval!(trace_eval(args[1], ctx), ctx);
    ctx.exec_ctx.vm.update_module(ctx.current_module, sym, value);

    // Return the value that was bound
    EvalResult::Value(value)
}
pub fn eval_fn(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    // Parse name vs anonymous
    let (name, params_index) = if let Some(name_sym) = ctx.get_symbol_id(args[0]) {
        if args.len() < 3 {
            return EvalResult::Value(ctx.arity_error(3, args.len(), "named fn"));
        }
        (Some(name_sym), 1)
    } else {
        (None, 0)
    };

    let params = match ctx.get_vector_of_symbols(args[params_index]) {
        Ok(params) => params,
        Err(error_msg) => {
            return EvalResult::Value(ctx.eval_error(&error_msg));
        }
    };

    let env = if let Some(name_sym) = name {
        // Create environment with placeholder for self-reference
        let mut fn_env = Env::with_parent(ctx.env);
        fn_env.set(name_sym, ctx.nil_value()); // Placeholder
        ctx.vm.alloc_env(fn_env)
    } else {
        ctx.env
    };

    let user_fn = Callable {
        params,
        body: args[(params_index + 1)..].to_vec(),
        env,
        is_variadic: false,
    };

    let value_ref = ctx.user_defined_function_value(user_fn);

    // If named, update the placeholder with the actual function
    if let Some(name_sym) = name {
        ctx.vm.update_env_variable(env, name_sym, value_ref);
    }

    EvalResult::Value(value_ref)
}

pub fn eval_do(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.nil_value());
    }
    eval_do_inline(args.to_vec(), 0, ctx.nil_value(), ctx)
}

fn eval_do_inline(
    forms: Vec<ValueRef>,
    mut index: usize,
    mut result: ValueRef,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= forms.len() {
            return EvalResult::Value(result);
        }

        let eval_result = trace_eval(forms[index].clone(), ctx);
        match eval_result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                result = val;
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        eval_do_inline(forms, index + 1, v, ctx)
                    }),
                };
            }
        }
    }
}

pub fn eval_let(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(
            ctx.eval_error("let expects a binding vector and at least one body form"),
        );
    }

    let bindings_val = &args[0];
    let bindings = match ctx.get_vector_elements(*bindings_val) {
        Ok(bindings) => bindings,
        _ => {
            return EvalResult::Value(ctx.eval_error("let expects a vector of bindings"));
        }
    };

    if bindings.len() % 2 != 0 {
        return EvalResult::Value(
            ctx.eval_error("let binding vector must have an even number of elements"),
        );
    }

    // Start processing bindings with current context
    eval_let_bindings_one_by_one(bindings, 0, args[1..].to_vec(), ctx)
}

fn eval_let_bindings_one_by_one(
    bindings: Vec<ValueRef>,
    mut index: usize,
    body: Vec<ValueRef>,
    ctx: &mut EvalContext,
) -> EvalResult {
    if index >= bindings.len() {
        // All bindings processed, evaluate body
        return eval_do_inline(body, 0, ctx.nil_value(), ctx);
    }

    let key = match ctx.get_symbol_id(bindings[index]) {
        Some(key) => key,
        None => return EvalResult::Value(ctx.eval_error("let binding keys must be symbols")),
    };

    let result = trace_eval(bindings[index + 1].clone(), ctx);
    match result {
        EvalResult::Value(val) => {
            if val.is_error() {
                return EvalResult::Value(val);
            }
            
            // Create new environment with this binding
            let mut new_env = Env::with_parent(ctx.env);
            new_env.set(key, val);
            let new_env_ref = ctx.vm.alloc_env(new_env);
            
            // Update context environment
            ctx.env = new_env_ref;
            
            // Continue with next binding
            eval_let_bindings_one_by_one(bindings, index + 2, body, ctx)
        }
        
        EvalResult::Suspended { future, resume: _ } => {
            return EvalResult::Suspended {
                future,
                resume: Box::new(move |val, ctx| {
                    if val.is_error() {
                        return EvalResult::Value(val);
                    }
                    
                    // Create new environment with this binding
                    let mut new_env = Env::with_parent(ctx.env);
                    new_env.set(key, val);
                    let new_env_ref = ctx.vm.alloc_env(new_env);
                    ctx.env = new_env_ref;
                    
                    eval_let_bindings_one_by_one(bindings, index + 2, body, ctx)
                }),
            };
        }
    }
}

pub fn eval_and(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.bool_value(true));
    }
    eval_and_inline(args.to_vec(), 0, ctx.bool_value(true), ctx)
}

fn eval_and_inline(
    args: Vec<ValueRef>,
    mut index: usize,
    mut last: ValueRef,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= args.len() {
            return EvalResult::Value(last);
        }

        let result = trace_eval(args[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                last = val;
                // Short-circuit on falsy values
                if !last.is_truthy() {
                    return EvalResult::Value(last);
                }
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        // Short-circuit check in resume too
                        if !v.is_truthy() {
                            return EvalResult::Value(v);
                        }
                        eval_and_inline(args, index + 1, v, ctx)
                    }),
                };
            }
        }
    }
}

pub fn eval_or(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.nil_value());
    }
    eval_or_inline(args.to_vec(), 0, ctx)
}

fn eval_or_inline(args: Vec<ValueRef>, mut index: usize, ctx: &mut EvalContext) -> EvalResult {
    loop {
        if index >= args.len() {
            return EvalResult::Value(ctx.nil_value());
        }

        let result = trace_eval(args[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                // Short-circuit on truthy values
                if !val.is_truthy() {
                    return EvalResult::Value(val);
                }
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        // Short-circuit check in resume too
                        if !v.is_truthy() {
                            return EvalResult::Value(v);
                        }
                        eval_or_inline(args, index + 1, ctx)
                    }),
                };
            }
        }
    }
}

pub fn eval_try(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "try"));
    }
    let res = try_eval!(trace_eval(args[0].clone(), ctx), ctx);
    if res.is_error() {
        forward_eval!(trace_eval(args[1].clone(), ctx), ctx)
    } else {
        EvalResult::Value(res)
    }
}

pub fn eval_apply(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "apply"));
    }
    let func = try_eval!(trace_eval(args[0].clone(), ctx), ctx);
    let evaluated_list = try_eval!(trace_eval(args[1].clone(), ctx), ctx);
    let list_items = match evaluated_list.get_list() {
        Some(list) => list,
        None => {
            return EvalResult::Value(ctx.eval_error("apply expects a list as second argument"))
        }
    };
    eval_func(func, list_items, ctx)
}
fn load_native_library(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    use libloading::Library;
    use std::path::PathBuf;

    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "load-native"));
    }

    let libname = match args[0].get_string() {
        Some(s) => s,
        None => return EvalResult::Value(ctx.eval_error("load-native expects a string")),
    };

    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let filename = format!("native/lib{}.{}", libname, ext);
    let lib_path = PathBuf::from(&filename);
    let lib_symbol_id = ctx.intern_symbol(&libname);
    let lib_symbol_id = ctx.get_symbol_id(lib_symbol_id).unwrap();

    // Remove existing module if it exists
    if ctx.get_module(lib_symbol_id).is_some() {
        ctx.remove_module(lib_symbol_id);
        ctx.remove_native_library(lib_symbol_id);
    }

    let lib = match unsafe { Library::new(&filename) } {
        Ok(lib) => lib,
        Err(e) => {
            return EvalResult::Value(
                ctx.eval_error(&format!("Failed to load native lib '{}': {}", filename, e)),
            );
        }
    };

    // Load using new Plugin system
    let plugin_register: libloading::Symbol<extern "C" fn() -> Plugin> = unsafe {
        match lib.get(b"blink_register") {
            Ok(register) => register,
            Err(e) => {
                return EvalResult::Value(ctx.eval_error(&format!(
                    "Plugin '{}' missing blink_register function: {}",
                    filename, e
                )));
            }
        }
    };

    let plugin = plugin_register();
    load_plugin_as_module(plugin, lib_symbol_id, lib_path, lib, ctx)
}

fn load_plugin_as_module(
    plugin: Plugin,
    lib_symbol_id: u32,
    lib_path: PathBuf,
    lib: Library,
    ctx: &mut EvalContext,
) -> EvalResult {

    let lib_path_symbol_id = ctx.intern_symbol(&lib_path.to_string_lossy());
    let lib_path_symbol_id = ctx.get_symbol_id(lib_path_symbol_id).unwrap();
    // Create module environment
    let mut module_env = Env::with_parent(ctx.env);

    // Register each function in the module environment
    let mut exports_set = HashMap::new();
    for (func_name, native_fn) in plugin.functions {
        // Convert function name to symbol ID for exports

        let func_symbol = ctx.symbol_value(&func_name);
        let func_symbol_id = ctx.get_symbol_id(func_symbol).unwrap();
        let value = ctx.native_function_value(native_fn);
        exports_set.insert(func_symbol_id, value);
    }

    let module_env_ref = ctx.vm.alloc_env(module_env);

    // Create and register the module
    let module = Module {
        name: lib_symbol_id,
        source: SerializedModuleSource::NativeDylib(lib_path_symbol_id),
        exports: exports_set,
        env: module_env_ref,
        ready: true,
    };

    ctx.register_module(&module);
    ctx.store_native_library(lib_symbol_id, lib);

    EvalResult::Value(ctx.nil_value())
}
