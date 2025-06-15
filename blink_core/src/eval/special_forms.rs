use std::{collections::{HashMap, HashSet}, fs, path::PathBuf, sync::Arc};

use parking_lot::RwLock;

use crate::{error::BlinkError, eval::{eval_func, forward_eval, result::EvalResult, trace_eval, try_eval, EvalContext}, module::{Module, ModuleSource}, value_ref::{ModuleRef, SharedValue, UserDefinedFn, ValueRef}, Env};

fn require_arity(args: &[ValueRef], expected: usize, form_name: &str) -> Result<(), BlinkError> {
    if args.len() != expected {
        return Err(BlinkError::arity(expected, args.len(), form_name));
    }
    Ok(())
}

// Helper function to extract symbol names from an environment
fn extract_env_symbols(env: &Arc<RwLock<Env>>) -> HashSet<String> {
    env.read().vars.keys().cloned().collect()
}

pub fn eval_quote(args: &Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(&args[1..], 1, "quote") {
        return EvalResult::Value(ctx.error_value(e));
    }
    EvalResult::Value(args[1].clone())
}

fn eval_if(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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
        EvalResult::Value(ctx.nil_value())
    }
}
fn eval_def(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "def"));
    }
    
    // Extract symbol name from first argument
    let name = match ctx.get_symbol_name(args[0]) {
        Some(name) => name,
        None => {
            return EvalResult::Value(ctx.eval_error("def first argument must be a symbol"));
        }
    };
    
    // Evaluate the second argument (the value to bind)
    let value = try_eval!(trace_eval(args[1], ctx), ctx);
    
    // Bind the symbol to the value in the current environment
    ctx.set_symbol(&name, value);
    
    // Return the value that was bound
    EvalResult::Value(value)
}
fn eval_fn(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "fn"));
    }
    
    // Extract parameter names from the first argument (should be a vector of symbols)
    let params = match ctx.get_vector_of_symbols(args[0]) {
        Ok(params) => params,
        Err(error_msg) => {
            return EvalResult::Value(ctx.eval_error(&error_msg));
        }
    };
    
    // Create the user-defined function
    let user_fn = UserDefinedFn {
        params,
        body: args[1..].to_vec(),
        env: Arc::clone(&ctx.env),
    };
    
    // Store in shared arena
    let shared_fn = SharedValue::UserDefinedFunction(user_fn);
    let value_ref = ctx.shared_arena.write().alloc(shared_fn);
    
    EvalResult::Value(value_ref)
}

fn eval_do(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.nil_value());
    }
    eval_do_inline(args.to_vec(), 0, ctx.nil_value(), ctx)
}

fn eval_do_inline(
    forms: Vec<ValueRef>, 
    mut index: usize, 
    mut result: ValueRef, 
    ctx: &mut EvalContext
) -> EvalResult {
    loop {
        if index >= forms.len() {
            return EvalResult::Value(result);
        }

        let eval_result = trace_eval(forms[index].clone(), ctx);
        match eval_result {
            EvalResult::Value(val) => {
                if ctx.is_err(&val) {
                    return EvalResult::Value(val);
                }
                result = val;
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if ctx.is_err(&v) {
                            return EvalResult::Value(v);
                        }
                        eval_do_inline(forms, index + 1, v, ctx)
                    }),
                };
            }
        }
    }
}

fn eval_let_bindings_inline(
    bindings: Vec<ValueRef>,
    mut index: usize,
    env: Arc<RwLock<Env>>,
    body: Vec<ValueRef>,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= bindings.len() {
            return eval_do_inline(body, 0, ctx.nil_value(), ctx);
        }

        let key_val = &bindings[index];
        let val_expr = &bindings[index + 1];

        let key = match ctx.get_symbol_name(*key_val) {
            Some(key) => key,
            None => {
                return EvalResult::Value(ctx.eval_error("let binding keys must be symbols"));
            }
        };

        let result = trace_eval(val_expr.clone(), ctx);
        match result {
            EvalResult::Value(v) => {
                if ctx.is_err(&v) {
                    return EvalResult::Value(v);
                }
                env.write().set(&key, v);
                index += 2;
            }

            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        
                        // Do what the deref resume would do: just use v directly
                        if ctx.is_err(&v) {
                            return EvalResult::Value(v);
                        }
                        
                        // Set the binding and continue
                        env.write().set(&key, v);

                        ctx.env = env.clone();


                        eval_let_bindings_inline(bindings, index + 2, env, body, ctx)
                    }),
                };
            }
        }
    }
}


fn eval_let(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(ctx.eval_error("let expects a binding vector and at least one body form"));
    }

    let bindings_val = &args[0];
    
    let bindings = match ctx.get_vector_elements(*bindings_val) {
        Ok(bindings) => bindings,
         _ => {
            // need to print value type here
            return EvalResult::Value(ctx.eval_error("let expects a vector of bindings"));
        }
    };

    if bindings.len() % 2 != 0 {
        return EvalResult::Value(ctx.eval_error("let binding vector must have an even number of elements"));
    }

    let local_env = Arc::new(RwLock::new(Env::with_parent(ctx.env.clone())));
    let mut local_ctx = ctx.with_env(local_env.clone());

    eval_let_bindings_inline(bindings, 0, local_env, args[1..].to_vec(), &mut local_ctx)
}



fn eval_and(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.bool_value(true));
    }
    eval_and_inline(args.to_vec(), 0, ctx.bool_value(true), ctx)
}

fn eval_and_inline(
    args: Vec<ValueRef>, 
    mut index: usize, 
    mut last: ValueRef, 
    ctx: &mut EvalContext
) -> EvalResult {
    loop {
        if index >= args.len() {
            return EvalResult::Value(last);
        }

        let result = trace_eval(args[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if ctx.is_err(&val) {
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
                        if ctx.is_err(&v) {
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

fn eval_or(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.nil_value());
    }
    eval_or_inline(args.to_vec(), 0, ctx)
}

fn eval_or_inline(
    args: Vec<ValueRef>, 
    mut index: usize, 
    ctx: &mut EvalContext
) -> EvalResult {
    loop {
        if index >= args.len() {
            return EvalResult::Value(ctx.nil_value());
        }

        let result = trace_eval(args[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if ctx.is_err(&val) {
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
                        if ctx.is_err(&v) {
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

fn eval_try(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "try"));
    }
    let res = try_eval!(trace_eval(args[0].clone(), ctx), ctx);
    if ctx.is_err(&res) {
        forward_eval!(trace_eval(args[1].clone(), ctx), ctx)
    } else {
        EvalResult::Value(res)
    }
}

fn eval_apply(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "apply"));
    }
    let func = try_eval!(trace_eval(args[0].clone(), ctx), ctx);
    let evaluated_list = try_eval!(trace_eval(args[1].clone(), ctx), ctx);
    let list_items = match &evaluated_list {
        ValueRef::Shared(idx) => {
            let shared_value = {
                let shared_arena = ctx.shared_arena.read();
                let res = shared_arena.get(*idx);
                res.map(|v| v.clone())
            };
            if let Some(shared) = shared_value {
                match shared.as_ref() {
                    SharedValue::List(xs) => xs.clone(),
                    _ => {
                        return EvalResult::Value(ctx.eval_error("apply expects a list as second argument"));
                    }
                }
            } else {
                return EvalResult::Value(ctx.eval_error("apply expects a list as second argument"));
            }
        },
        _ => {
            return EvalResult::Value(ctx.eval_error("apply expects a list as second argument"));
        }
    };
    eval_func(func, list_items, ctx)
}


fn load_native_library(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use libloading::Library;

    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "load-native"));
    }

    let libname = match &args[0] {
        ValueRef::Shared(idx) => {
            let shared_value = {
                let shared_arena = ctx.shared_arena.read();
                let res = shared_arena.get(*idx);
                res.map(|v| v.clone())
            };
            if let Some(shared) = shared_value {

                match shared.as_ref() {
                    SharedValue::Str(s) => s.clone(),
                    _ => {
                        return EvalResult::Value(ctx.eval_error("load-native expects a string"));
                    }
                } 
            } else  {
                return EvalResult::Value(ctx.eval_error("load-native expects a string"));
            }
        },
        _ => {
            return EvalResult::Value(ctx.eval_error("load-native expects a string"));
        }
    };

    // Determine the correct file extension
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let filename = format!("native/lib{}.{}", libname, ext);
    let lib_path = PathBuf::from(&filename);

    // Remove existing module if it exists (force reload)
    if ctx.module_registry.read().get_module(&libname).is_some() {
        
        ctx.module_registry.write().remove_module(&libname);
        ctx.module_registry.write().remove_native_library(&lib_path);
    }

    let lib = match unsafe { Library::new(&filename) } {
        Ok(lib) => lib,
        Err(e) => {
            return EvalResult::Value(ctx.eval_error(&format!("Failed to load native lib '{}': {}", filename, e)));
        }
    };

    // Try the new registration function first (with exports)
    let exports = match unsafe { lib.get::<unsafe extern "C" fn(&mut Env) -> Vec<String>>(b"blink_register_with_exports") } {
        Ok(register_with_exports) => {
            // Create a new environment for the module
            let module_env = Arc::new(RwLock::new(Env::with_parent(ctx.global_env.clone())));
            let exported_names = unsafe { register_with_exports(&mut *module_env.write()) };
            
            // Register the module with known exports
            let exports_set: HashSet<String> = exported_names.into_iter().collect();
            let module = Module {
                name: libname.clone(),
                source: ModuleSource::NativeDylib(lib_path.clone()),
                exports: exports_set.clone(),
                env: module_env,
                ready: true,
            };
            let _arc_mod = ctx.module_registry.write().register_module(module);
            
            exports_set
        },
        Err(_) => {
            // Fall back to old registration function (no export tracking)
            let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
                match lib.get(b"blink_register") {
                    Ok(register) => register,
                    Err(e) => {
                        return EvalResult::Value(ctx.eval_error(&format!("Failed to find blink_register or blink_register_with_exports in '{}': {}", filename, e)));
                    }
                }
            };
            
            // Create a new environment for the module
            let module_env = Arc::new(RwLock::new(Env::with_parent(ctx.global_env.clone())));
            unsafe { register(&mut *module_env.write()) };
            
            // Extract exports from the environment (since old function doesn't return them)
            let exports = extract_env_symbols(&module_env);
            
            // Register the module
            let module = Module {
                name: libname.clone(),
                source: ModuleSource::NativeDylib(lib_path.clone()),
                exports: exports.clone(),
                env: module_env,
                ready: true,
            };
            let _arc_mod = ctx.module_registry.write().register_module(module);
            
            exports
        }
    };

    // Store the library to prevent it from being unloaded
    ctx.module_registry.write().store_native_library(lib_path, lib);



    EvalResult::Value(ctx.nil_value())
}



fn import_symbols_into_env(
    symbols: &[String], 
    module_name: &str, 
    aliases: &HashMap<String, String>,
    ctx: &mut EvalContext
) -> Result<(), BlinkError> {
    let module = ctx.module_registry.read().get_module(module_name)
        .ok_or_else(|| BlinkError::eval(format!("Module '{}' not found", module_name)))?;
    
    let module_read = module.read();
    
    // Handle import all (*)
    if symbols.len() == 1 && symbols[0] == ":all" {
        for export_name in &module_read.exports {
            let local_name = aliases.get(export_name).unwrap_or(export_name);
            
            // Create a live reference instead of copying the value
            let reference = ctx.module_value(module_name, export_name);
            ctx.env.write().vars.insert(local_name.clone(), reference);
        }
        return Ok(());
    }
    
    // Import specific symbols
    for symbol_name in symbols {
        // Check if symbol is exported
        if !module_read.exports.contains(symbol_name) {
            return Err(BlinkError::eval(format!("Symbol '{}' is not exported by module '{}'", symbol_name, module_name)));
        }
        
        let local_name = aliases.get(symbol_name).unwrap_or(symbol_name);
        
        // Create a live reference to the module symbol
        let reference = ctx.module_value(module_name, symbol_name);
        ctx.env.write().vars.insert(local_name.clone(), reference);
    }
    
    Ok(())
}

fn load_native_code(
    args: &[ValueRef],
    ctx: &mut EvalContext,
) -> Result<EvalResult, BlinkError> {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use libloading::Library;
    use std::collections::HashSet;

    let pos = args.get(0).and_then(|v| ctx.get_pos(*v));
    
    if args.len() < 1 || args.len() > 2 {
        return Err(BlinkError::arity(1, args.len(), "compile-plugin").with_pos(pos));
    }

    let plugin_name = match &args[0] {
        ValueRef::Shared(idx) => {
            let shared_value = {
                let shared_arena = ctx.shared_arena.read();
                let res = shared_arena.get(*idx);
                res.map(|v| v.clone())
            };
            if let Some(shared) = shared_value {
                match shared.as_ref() {
                    SharedValue::Str(s) => s.clone(),
                    _ => {
                        return Err(BlinkError::eval("compile-plugin expects a string as first argument").with_pos(pos));
                    }
                }
            } else {
                return Err(BlinkError::eval("compile-plugin expects a string as first argument").with_pos(pos));
            }
        },
        _ => {
            return Err(BlinkError::eval("compile-plugin expects a string as first argument").with_pos(pos));
        }
    };

    let mut plugin_path = format!("plugins/{}", plugin_name);
    let mut auto_import = false;

    // Parse options if provided
    if args.len() == 2 {        
        let options_val = match trace_eval(args[1].clone(), ctx) {
            EvalResult::Value(v) => {
                if ctx.is_err(&v) {
                    let error = ctx.get_err(&v);
                    return Err(error);
                }
        
                v
            }
            suspended => return Ok(suspended),
        };
        if let ValueRef::Shared(idx) = options_val {
            if let Some(()) = ctx.with_map(options_val, |opt_map| {
                for (key, value) in opt_map {
                    if let Some(kw_name) = ctx.get_keyword_name(*key) {
                        match &*kw_name {
                            "path" => {
                                if let Some(path) = ctx.get_string(*value) {
                                    plugin_path = path;
                                }
                            }
                            "import" => {
                                if let Some(b) = ctx.get_bool(*value) {
                                    auto_import = b;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }) {
                // Map was successfully processed
            } else {
                return Err(BlinkError::eval("Second argument must be a map").with_pos(pos));
            }
        }
    }

    // Check if plugin directory exists
    if !Path::new(&plugin_path).exists() {
        return Err(BlinkError::eval(format!("Plugin path '{}' does not exist", plugin_path)).with_pos(pos));
    }

    // Compile the plugin
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&plugin_path)
        .status()
        .map_err(|e| BlinkError::eval(format!("Failed to build plugin: {}", e)).with_pos(pos))?;

    if !status.success() {
        return Err(BlinkError::eval("Plugin build failed").with_pos(pos));
    }

    // Determine library extension
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let source = format!("{}/target/release/lib{}.{}", plugin_path, plugin_name, ext);
    let dest = format!("native/lib{}.{}", plugin_name, ext);

    // Create native directory and copy library
    fs::create_dir_all("native").ok();
    fs::copy(&source, &dest).map_err(|e| BlinkError::eval(format!("Failed to copy compiled plugin: {}", e)).with_pos(pos))?;

    // Load the library
    let lib = unsafe { Library::new(&dest) }.map_err(|e| BlinkError::eval(format!("Failed to load compiled plugin: {}", e)).with_pos(pos))?;

    // Try the new registration function first (with exports)
    let exports = match unsafe { lib.get::<unsafe extern "C" fn(&mut Env) -> Vec<String>>(b"blink_register_with_exports") } {
        Ok(register_with_exports) => {
            // Create a new environment for the module
            let module_env = Arc::new(RwLock::new(Env::with_parent(ctx.global_env.clone())));
            let exported_names = unsafe { register_with_exports(&mut *module_env.write()) };
            
            // Register the module with known exports
            let exports_set: HashSet<String> = exported_names.into_iter().collect();
            let module = Module {
                name: plugin_name.clone(),
                source: ModuleSource::NativeDylib(PathBuf::from(&dest)),
                exports: exports_set.clone(),
                env: module_env,
                ready: true,
            };
            let _arc_mod = ctx.module_registry.write().register_module(module);
            
            exports_set
        },
        Err(_) => {
            // Fall back to old registration function (no export tracking)
            let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
                lib.get(b"blink_register")
                    .map_err(|e| BlinkError::eval(format!("Failed to find blink_register or blink_register_with_exports: {}", e)).with_pos(pos))?
            };
            
            // Create a new environment for the module
            let module_env = Arc::new(RwLock::new(Env::with_parent(ctx.global_env.clone())));
            unsafe { register(&mut *module_env.write()) };
            
            // We don't know the exports, so we'll have to extract them from the environment
            let exports = extract_env_symbols(&module_env);
            let module = Module {
                name: plugin_name.clone(),
                source: ModuleSource::NativeDylib(PathBuf::from(&dest)),
                exports: exports.clone(),
                env: module_env,
                ready: true,
            };
            // Register the module
            let _arc_mod = ctx.module_registry.write().register_module(module);
            if auto_import {
                import_symbols_into_env(&exports.clone().into_iter().collect::<Vec<String>>(), &plugin_name, &HashMap::new(), ctx)?;
            }
            
            exports
        }
    };

    // Store the library to prevent it from being unloaded
    ctx.module_registry.write().store_native_library(PathBuf::from(&dest), lib);
    import_symbols_into_env(&exports.clone().into_iter().collect::<Vec<String>>(), &plugin_name, &HashMap::new(), ctx)?;

    

    Ok(EvalResult::Value(ctx.nil_value()))
}

// Load and evaluate a Blink source file
fn eval_blink_file(file_path: PathBuf, ctx: &mut EvalContext) -> EvalResult {
    // Don't evaluate if already loaded
    if ctx.module_registry.read().is_file_evaluated(&file_path) {
        return EvalResult::Value(ctx.nil_value());
    }

    let contents = match fs::read_to_string(&file_path) {
        Ok(contents) => contents,
        Err(e) => {
            return EvalResult::Value(ctx.eval_error(&format!("Failed to read file: {}", e)));
        }
    };
    let mut reader_ctx = ctx.reader_macros.clone();
    // Parse the file
    let forms = crate::parser::parse_all(&contents, &mut reader_ctx);


    let forms = match forms {
        Ok(forms) => forms,
        Err(e) => {
            return EvalResult::Value(e.into_blink_value());
        }
    };
    
    // Set current file context
    let old_file = ctx.current_file.clone();
    let file_name = match file_path.file_name() {
        Some(file_name) => {
            if let Some(file_name) = file_name.to_str() {
                file_name.to_string()
            } else {
                return EvalResult::Value(ctx.eval_error("File name missing."));
            }
        },
        None => {
            return EvalResult::Value(ctx.eval_error("File name missing."));
        },
    };
    ctx.current_file = Some(file_name);
    
    // Evaluate all forms in the file
    try_eval!(eval_file_forms_inline(forms, 0, ctx), ctx);
    
    // Restore previous file context
    ctx.current_file = old_file;
    
    // Mark file as evaluated
    ctx.module_registry.write().mark_file_evaluated(file_path);
    
    EvalResult::Value(ctx.nil_value())
}


fn parse_load_args(args: &[ValueRef], ctx: &mut EvalContext) -> Result<(String, String), BlinkError> {
    if args.is_empty() {
        return Err(BlinkError::arity(2, 0, "load"));
    }
    
    // First argument should be a keyword indicating source type
    let source_type = ctx.get_keyword_name(args[0].clone()).ok_or_else(|| BlinkError::eval("load expects a keyword as first argument (:file, :native, :cargo, :dylib, :url, :git)"))?;
    
    
    // Second argument should be the source value
    if args.len() < 2 {
        return Err(BlinkError::eval(format!("load {} requires a source argument", source_type)));
    }
    
    let source_value = ctx.get_string(args[1].clone()).ok_or_else(|| BlinkError::eval("load source must be a string"))?;

    Ok((source_type, source_value))
}


fn eval_load(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    let (source_type, source_value) = match parse_load_args(args, ctx) {
        Ok(res) => res,
        Err(e) => {
            return EvalResult::Value(ctx.error_value(e));
        }
    };
    
    match source_type.as_str() {
        "file" => {
            let file_path = PathBuf::from(&source_value);
            let loaded = eval_blink_file(file_path, ctx);
            try_eval!(loaded, ctx);
            EvalResult::Value(ctx.nil_value())
        },
        
        "native" => {
            let path = PathBuf::from(&source_value);
            
            // Check if it's a Cargo project (has Cargo.toml) or a single library file
            if path.is_dir() {
                let cargo_toml = path.join("Cargo.toml");
                if cargo_toml.exists() {
                    // It's a Cargo project - compile it
                    match load_native_code(&args, ctx) {
                        Ok(result) => result,
                        Err(e) => {
                            return EvalResult::Value(ctx.error_value(e));
                        }
                    }
                } else {
                    return EvalResult::Value(BlinkError::eval(format!("Directory '{}' is not a valid Cargo project (no Cargo.toml found)", source_value)).into_blink_value());
                }
            } else if path.extension().map_or(false, |ext| {
                ext == "so" || ext == "dll" || ext == "dylib"
            }) {
                // It's a pre-built library file
                load_native_library(&args, ctx)
            } else {
                return EvalResult::Value(BlinkError::eval(format!("'{}' is neither a Cargo project directory nor a native library file", source_value)).into_blink_value());
            }
        },
        
        // "cargo" => {
        //     // Direct cargo crate compilation
        //     compile_cargo_crate(&source_value, ctx)
        // },
        
        // "dylib" | "dll" | "so" => {
        //     // Direct library loading
        //     load_single_native_library(&source_value, ctx)
        // },
        
        // "url" => {
        //     load_url_module(source_value, ctx)
        // },
        
        // "git" => {
        //     load_git_module(source_value, options, ctx)
        // },
        
        _ => EvalResult::Value(ctx.eval_error(&format!("Unknown load source type: :{}", source_type))),
    }
}