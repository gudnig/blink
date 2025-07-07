use std::{collections::{HashMap, HashSet}, fs, path::PathBuf, sync::Arc};

use libloading::Library;
use parking_lot::RwLock;

use crate::{env::Env, error::BlinkError, eval::{eval_func, forward_eval, result::EvalResult, trace_eval, try_eval, EvalContext}, module::{ImportType, Module, ModuleSource}, runtime::AsyncContext, value::{unpack_immediate, ImmediateValue, Macro, Plugin, UserDefinedFn, ValueRef}};

fn require_arity(args: &[ValueRef], expected: usize, form_name: &str) -> Result<(), BlinkError> {
    if args.len() != expected {
        return Err(BlinkError::arity(expected, args.len(), form_name));
    }
    Ok(())
}

// Helper function to extract symbol names from an environment
fn extract_env_symbols(env: &Arc<RwLock<Env>>) -> HashSet<u32> {
    env.read().vars.keys().cloned().collect()
}

pub fn eval_quote(args: &Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(&args[1..], 1, "quote") {
        return EvalResult::Value(ctx.error_value(e));
    }
    EvalResult::Value(args[1].clone())
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
        EvalResult::Value(ctx.nil_value())
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
    
    // Bind the symbol to the value in the current environment
    ctx.set_symbol(sym, value);
    
    // Return the value that was bound
    EvalResult::Value(value)
}
pub fn eval_fn(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

        let key = match ctx.get_symbol_id(*key_val) {
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
                env.write().set(key, v);
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
                        env.write().set(key, v);

                        ctx.env = env.clone();


                        eval_let_bindings_inline(bindings, index + 2, env, body, ctx)
                    }),
                };
            }
        }
    }
}


pub fn eval_let(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

pub fn eval_or(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

pub fn eval_try(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

pub fn eval_apply(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

    let libname = match ctx.get_string(args[0]) {
        Some(s) => s,
        None => return EvalResult::Value(ctx.eval_error("load-native expects a string")),
    };

    let ext = if cfg!(target_os = "macos") { "dylib" }
    else if cfg!(target_os = "windows") { "dll" }
    else { "so" };

    let filename = format!("native/lib{}.{}", libname, ext);
    let lib_path = PathBuf::from(&filename);
    let lib_symbol_id = ctx.intern_symbol(&libname);
    let lib_symbol_id = ctx.get_symbol_id(lib_symbol_id).unwrap();

    // Remove existing module if it exists
    if ctx.module_registry.read().get_module(lib_symbol_id).is_some() {
        ctx.module_registry.write().remove_module(lib_symbol_id);
        ctx.module_registry.write().remove_native_library(&lib_path);
    }

    let lib = match unsafe { Library::new(&filename) } {
        Ok(lib) => lib,
        Err(e) => {
            return EvalResult::Value(ctx.eval_error(&format!("Failed to load native lib '{}': {}", filename, e)));
        }
    };

    // Load using new Plugin system
    let plugin_register: libloading::Symbol<extern "C" fn() -> Plugin> = unsafe {
        match lib.get(b"blink_register") {
            Ok(register) => register,
            Err(e) => {
                return EvalResult::Value(ctx.eval_error(&format!("Plugin '{}' missing blink_register function: {}", filename, e)));
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
    ctx: &mut EvalContext
) -> EvalResult {
    use std::collections::HashSet;

    // Create module environment
    let module_env = Arc::new(RwLock::new(Env::with_parent(ctx.global_env.clone())));
    
    // Register each function in the module environment
    let mut exports_set = HashSet::new();
    for (func_name, native_fn) in plugin.functions {
        // Convert function name to symbol ID for exports
        
        let func_symbol = ctx.symbol_value(&func_name);
        let func_symbol_id = ctx.get_symbol_id(func_symbol).unwrap();
        exports_set.insert(func_symbol_id);
        
        // Create ValueRef that wraps the NativeFn
        let function_value = ctx.native_function_value(native_fn);
        
        // Set in environment
        module_env.write().set(func_symbol_id, function_value);
    }

    // Create and register the module
    let module = Module {
        name: lib_symbol_id,
        source: ModuleSource::NativeDylib(lib_path.clone()),
        exports: exports_set,
        env: module_env,
        ready: true,
    };

    ctx.module_registry.write().register_module(module);
    ctx.module_registry.write().store_native_library(lib_path, lib);
    
    EvalResult::Value(ctx.nil_value())
}

fn import_symbols_into_env(
    symbols: &[ValueRef], //TODO this should be a vector of u32s
    module_name: u32,
    aliases: &HashMap<u32, u32>,
    ctx: &mut EvalContext
) -> Result<(), BlinkError> {
    let module = ctx.module_registry.read().get_module(module_name)
        .ok_or_else(|| BlinkError::eval(format!("Module '{}' not found", module_name)))?;
    
    let module_read = module.read();
    
    // Handle import all (*) - check first symbol only
    if symbols.len() == 1 {
        if let Some(first_name) = ctx.get_symbol_name(symbols[0].clone()) {
            if first_name == ":all" {
                for export_name in &module_read.exports {
                    let local_name = aliases.get(export_name).unwrap_or(export_name);
                    let reference = ctx.module_value(module_name, *export_name);
                    ctx.env.write().vars.insert(*local_name, reference);
                }
                return Ok(());
            }
        }
    }
    
    // Import specific symbols
    for symbol_ref in symbols {
        let symbol_name = ctx.get_symbol_id(*symbol_ref)
            .ok_or_else(|| BlinkError::eval("Non-symbol argument to import"))?;
        
        // Check if symbol is exported
        if !module_read.exports.contains(&symbol_name) {
            return Err(BlinkError::eval(format!(
                "Symbol '{}' is not exported by module '{}'", 
                symbol_name, module_name
            )));
        }
        
        let local_name = aliases.get(&symbol_name).unwrap_or(&symbol_name);
        let reference = ctx.module_value(module_name, symbol_name);
        ctx.env.write().vars.insert(*local_name, reference);
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

    let plugin_name =  match args[0] {
        ValueRef::Shared(idx) => {
            let shared_value = {
                let shared_arena = ctx.shared_arena.read();
                let res = shared_arena.get(idx);
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

    let plugin_symbol_id = ctx.intern_symbol(&plugin_name);
    let plugin_symbol_id = ctx.get_symbol_id(plugin_symbol_id).unwrap();

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
                for (key, value) in opt_map.iter() {
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
            let mut exports_set: HashSet<u32> = HashSet::new();
            let exported_names = unsafe {
                register_with_exports(&mut *module_env.write())
             };
             for name in exported_names {
                let symbol = ctx.intern_symbol(&name);
                let symbol_id = ctx.get_symbol_id(symbol).unwrap();
                exports_set.insert(symbol_id);
             }
            
            // Register the module with known exports
            
            let module = Module {
                name: plugin_symbol_id,
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
            let export_symbols =extract_env_symbols(&module_env);
            let exports = export_symbols.iter().map(|s| ValueRef::symbol(*s)).collect::<Vec<ValueRef>>();
            let module = Module {
                name: plugin_symbol_id,
                source: ModuleSource::NativeDylib(PathBuf::from(&dest)),
                exports: export_symbols.clone(),
                env: module_env,
                ready: true,
            };
            // Register the module
            let _arc_mod = ctx.module_registry.write().register_module(module);
            if auto_import {
                import_symbols_into_env(&exports, plugin_symbol_id, &HashMap::new(), ctx)?;
            }
            
            export_symbols
        }
    };
    let exports = exports.iter().map(|s| ValueRef::symbol(*s)).collect::<Vec<ValueRef>>();
    // Store the library to prevent it from being unloaded
    ctx.module_registry.write().store_native_library(PathBuf::from(&dest), lib);
    import_symbols_into_env(&exports, plugin_symbol_id, &HashMap::new(), ctx)?;

    

    Ok(EvalResult::Value(ctx.nil_value()))
}

fn eval_file_forms_inline(
    forms: Vec<ValueRef>,
    mut index: usize,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= forms.len() {
            return EvalResult::Value(ctx.nil_value());
        }

        let result = trace_eval(forms[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if ctx.is_err(&val) {
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
                        eval_file_forms_inline(forms, index + 1, ctx)
                    }),
                };
            }
        }
    }
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
    let parsed_forms = crate::parser::parse_all(&contents, &mut reader_ctx.write(), &mut ctx.symbol_table.write());


    let parsed_forms = match   parsed_forms {
        Ok(forms) => forms,
        Err(e) => {
            return EvalResult::Value(ctx.error_value(e));
        }
    };

    let mut forms = Vec::new();
    for form in parsed_forms {
        let value = ctx.alloc_parsed_value(form);
        forms.push(value);
    }
    
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


pub fn eval_load(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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
                    return EvalResult::Value(ctx.eval_error(&format!("Directory '{}' is not a valid Cargo project (no Cargo.toml found)", source_value)));
                }
            } else if path.extension().map_or(false, |ext| {
                ext == "so" || ext == "dll" || ext == "dylib"
            }) {
                // It's a pre-built library file
                load_native_library(&args, ctx)
            } else {
                return EvalResult::Value(ctx.eval_error(&format!("'{}' is neither a Cargo project directory nor a native library file", source_value)));
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

fn parse_import_args(args: &[ValueRef], ctx: &mut EvalContext) -> Result<(ImportType, Option<HashMap<String, ValueRef>>), BlinkError> {
    if args.is_empty() {
        return Err(BlinkError::arity(1, 0, "imp"));
    }

    let first_arg = args[0];
    
    match first_arg {
        // Check for string (file import)
        ValueRef::Shared(idx) => {
            

            if let Some(shared) = ctx.get_shared_value(ValueRef::Shared(idx)).map(|v| v.clone()) {
                match shared.as_ref() {
                    // File import: (imp "module-name")
                    SharedValue::Str(module_name) => {
                        Ok((ImportType::File(module_name.clone()), None))
                    }
                    
                    // Vector import: (imp [sym1 sym2] :from module)
                    SharedValue::Vector(symbols) => {
                        let (import_type, options) = parse_symbol_import(&symbols, &args[1..],  ctx)?;
                        Ok((import_type, Some(options)))
                    }
                    
                    // List import: (imp (vector sym1 sym2) :from module) or regular list
                    SharedValue::List(list) if !list.is_empty() => {
                        // Check if it's a vector form (list starting with 'vector)
                        if let Some(first_symbol_name) = ctx.get_symbol_name(list[0]) {
                            if first_symbol_name == "vector" {
                                let (import_type, options) = parse_symbol_import(&list[1..], &args[1..], ctx)?;
                                return Ok((import_type, Some(options)));
                            }
                        }
                        
                        // Otherwise, treat as regular list of symbols
                        let (import_type, options) = parse_symbol_import(&list, &args[1..], ctx)?;
                        Ok((import_type, Some(options)))
                    }
                    
                    _ => Err(BlinkError::eval("imp expects a string (file) or vector (symbols)")),
                }
            } else {
                Err(BlinkError::eval("Invalid reference in imp"))
            }
        }
        
        _ => Err(BlinkError::eval("imp expects a string (file) or vector (symbols)")),
    }
}

fn parse_symbol_import(
    symbol_list: &[ValueRef], 
    remaining_args: &[ValueRef],
    ctx: &mut EvalContext
) -> Result<(ImportType, HashMap<String, ValueRef>), BlinkError> {
    
    // Parse symbols and any aliases
    let mut symbols = Vec::new();
    let aliases = HashMap::new();
    
    let mut i = 0;
    while i < symbol_list.len() {
        match &symbol_list[i] {
            
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(*packed);
                if let ImmediateValue::Symbol(symbol_id) = unpacked {
                    symbols.push(symbol_id) ;
                } else {
                    return Err(BlinkError::eval("Symbol list must contain symbols"));
                }
            }
            
            _ => return Err(BlinkError::eval("Symbol list must contain symbols")),
        }
    }
    
    // Look for :from module-name
    let mut module_name = None;
    let options = HashMap::new();
    
    let mut j = 0;
    while j < remaining_args.len() {
        if let Some(kw) = ctx.get_keyword_name(remaining_args[j]) {
            match kw.as_str() {
                "from" => {
                    if j + 1 >= remaining_args.len() {
                        return Err(BlinkError::eval(":from requires a module name"));
                    }
                    if let Some(module) = ctx.get_symbol_id(remaining_args[j + 1]) {
                        module_name = Some(module.clone());
                        j += 2;
                    } else {
                        return Err(BlinkError::eval(":from expects a module name"));
                    }
                },
                other => {
                    return Err(BlinkError::eval(format!("Unknown import option: {}", other)));
                }
            }
        } else {
            return Err(BlinkError::eval("Expected keyword after symbol list"));
        }
    }
    
    let module = module_name.ok_or_else(|| BlinkError::eval("Symbol import requires :from module-name"))?;
    
    Ok((ImportType::Symbols { symbols, module, aliases }, options))
}

pub fn eval_def_reader_macro(
    args: &[ValueRef],
    ctx: &mut EvalContext,
) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "def-reader-macro"));
    }

    let char_val = match &args[0] {
        ValueRef::Shared(idx) => {
            let shared = ctx.get_shared_value(ValueRef::Shared(*idx));
            if let Some(shared) = shared {
                match shared.as_ref() {
                    SharedValue::Str(s) => s.clone(),
                    _ => {
                        return EvalResult::Value(ctx.eval_error("First argument to def-reader-macro must be a string"));
                    }
                }
            } else {
                return EvalResult::Value(ctx.eval_error("First argument to def-reader-macro must be a string"));
            }
        }
        _ => {
            return EvalResult::Value(ctx.eval_error("First argument to def-reader-macro must be a string"));
        }
    };

    let ch = char_val;

    let func = try_eval!(trace_eval(args[1].clone(), ctx), ctx);
    let func_symbol = ctx.intern_symbol(&ch);
    let func_symbol_id = ctx.get_symbol_id(func_symbol).unwrap();
    ctx.set_symbol(func_symbol_id, func);
    ctx.reader_macros
        .write()
        .reader_macros
        .insert(ch, func_symbol_id);
    EvalResult::Value(func)
}

/// Helper to update module exports
fn update_module_exports(module: &mut Module, exports_val: &ValueRef, ctx: &mut EvalContext) -> Result<(), BlinkError> {
    match &exports_val {
        ValueRef::Shared(idx) => {
            let list = ctx.get_shared_value(ValueRef::Shared(*idx));
            if let Some(list) = list {
                match list.as_ref() {
                    SharedValue::List(items) => {
                        for item in items {
                            let sym  = ctx.get_symbol_id(*item);
                            if let Some(sym) = sym {
                                module.exports.insert(sym);
                            }
                        }
                    }
                    _ => return Err(BlinkError::eval("Exports must be a list of symbols")),
                }
            }
        }, 
        ValueRef::Immediate(packed) => {
            let kw_str = ctx.get_keyword_name(ValueRef::Immediate(*packed));
            if let Some(kw_str) = kw_str {
                if kw_str == "all" {
                    let all_keys = module.env.read().vars.keys().cloned().collect();
                    module.exports = all_keys;
                    return Ok(());
                }
            }            
        }
        ValueRef::Gc(_gc_ptr) => todo!(),
    }
    
    Ok(())
}

/// Helper to parse keyword flags at the beginning of args
fn parse_flags(args: &[ValueRef], ctx: &mut EvalContext) -> (HashSet<String>, usize) {
    let mut flags = HashSet::new();
    let mut name_index = 0;
    
    for (i, arg) in args.iter().enumerate() {
        if let Some(kw) = ctx.get_keyword_name(*arg) {
            flags.insert(kw.clone());
            name_index = i + 1;
        } else {
            break;
        }
    }
    
    (flags, name_index)
}

/// Extract a name from a ValueRef
fn extract_name(value: &ValueRef, ctx: &mut EvalContext) -> Result<String, BlinkError> {

    if let Some( name) = ctx.get_symbol_name(*value) {
        Ok(name.clone())
    } else {
        Err(BlinkError::eval("Expected a symbol for name"))
    }    
}

fn parse_mod_options(args: &[ValueRef], ctx: &mut EvalContext) -> Result<(HashMap<String, ValueRef>, usize), BlinkError> {
    let mut options = HashMap::new();
    let mut i = 0;
    
    while i < args.len() {
        // Check if current arg is a keyword
        if let Some(keyword) = ctx.get_keyword_name(args[i]) {
            match keyword.as_str() {
                "exports" => {
                    if i + 1 >= args.len() {
                        return Err(BlinkError::eval(":exports requires a list"));
                    }
                    options.insert("exports".to_string(), args[i + 1]); // No clone needed
                    i += 2;
                },
                other => {
                    return Err(BlinkError::eval(format!("Unknown option: :{}", other)));
                }
            }
        } else {
            // Not a keyword, so we've reached the body
            break;
        }
    }
    
    Ok((options, i))
}



/// Module declaration and context management
pub fn eval_mod(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.arity_error(1, 0, "mod"));
    }
    
    // Parse flags
    let (flags, name_index) = parse_flags(args, ctx);
    if flags.len() < 1 {
        return EvalResult::Value(ctx.eval_error("At least one flag is required."));
    }
    
    // Extract module name
    if name_index >= args.len() {
        return EvalResult::Value(ctx.eval_error("Missing module name after flags"));
    }
    
    let name = match ctx.get_symbol_id(args[name_index]) {
        Some(name) => name,
        None => {
            return EvalResult::Value(ctx.eval_error("Expected a symbol for module name"));
        }
    };
    
    
    let (options, body_start) = match parse_mod_options(&args[name_index + 1..], ctx) {
        Ok(res) => res,
        Err(e) => {
            return EvalResult::Value(ctx.error_value(e));
        }
    };
    

    let should_declare = flags.contains("declare") || flags.is_empty();
    let should_enter = flags.contains("enter");
    
    let mut module = ctx.module_registry.read().get_module(name);
    

    if should_declare && module.is_none() {
        let current_file = ctx.current_file.clone().unwrap_or_else(|| "<repl>".to_string());
        let source = if current_file == "<repl>" {
            ModuleSource::Repl
        } else {
            ModuleSource::BlinkFile(PathBuf::from(current_file))
        };
        let module_env = Arc::new(RwLock::new(Env::with_parent(ctx.global_env.clone())));
        let new_module = Module {
            name: name.clone(),
            // using the global env as parent
            env: module_env,
            exports: HashSet::new(),
            source: source,
            ready: true,
        };
        let module_arc = ctx.module_registry.write().register_module(new_module);
        module = Some(module_arc);        
    }
    if should_declare && options.contains_key("exports") {
        let mut module_guard = module.as_ref().unwrap().write();
        match update_module_exports(&mut module_guard, &options["exports"], ctx) {
            Ok(_) => (),
            Err(e) => {
                return EvalResult::Value(ctx.error_value(e));
            }
        };
    }

    if should_enter {
        if let Some(module) = module {
            ctx.current_module = Some(module.read().name.clone());
            ctx.env = module.read().env.clone();
        } else {
            return EvalResult::Value(ctx.eval_error("Module not found"));
        }
    }
    EvalResult::Value(ctx.nil_value())
    
}



fn find_module_file(module_name: &str, ctx: &mut EvalContext) -> Result<PathBuf, BlinkError> {
    // 1. Check if module is already registered (we know which file it came from)
    let module_symbol_id = ctx.intern_symbol(module_name);
    let module_symbol_id = ctx.get_symbol_id(module_symbol_id).unwrap();

    if let Some(module) = ctx.module_registry.read().get_module(module_symbol_id) {
        let module_read = module.read();
        if let ModuleSource::BlinkFile(ref path) = module_read.source {
            return Ok(path.clone());
        }
    }
    
    // 2. Try direct file mapping first (most common case)
    let direct_path = PathBuf::from(format!("lib/{}.blink", module_name));
    if direct_path.exists() {
        return Ok(direct_path);
    }
    
    // 3. Try parent directory approach (for multi-module files)
    let parts: Vec<&str> = module_name.split('/').collect();
    for i in (1..parts.len()).rev() {
        let parent_path = parts[..i].join("/");
        let candidate = PathBuf::from(format!("lib/{}.blink", parent_path));
        
        if candidate.exists() {
            // Check if this file actually contains our target module
            if file_contains_module(&candidate, module_name)? {
                return Ok(candidate);
            }
        }
    }
    
    // 4. Search common patterns
    let search_candidates = vec![
        format!("lib/{}.bl", parts.join("-")),           // math/utils -> math-utils.blink
        format!("lib/{}.bl", parts.last().unwrap()),     // math/utils -> utils.blink
        format!("lib/{}/mod.bl", parts[0]),              // math/utils -> math/mod.blink
    ];
    
    for candidate_str in search_candidates {
        let candidate = PathBuf::from(candidate_str);
        if candidate.exists() && file_contains_module(&candidate, module_name)? {
            return Ok(candidate);
        }
    }
    
    Err(BlinkError::eval(format!("Module '{}' not found. Tried:\n  lib/{}.bl\n  lib/{}.bl\n  And parent directories", 
                        module_name, module_name, parts.join("-"))))
}

// Helper function to check if a file contains a specific module declaration
fn file_contains_module(file_path: &PathBuf, module_name: &str) -> Result<bool, BlinkError> {
    let content = std::fs::read_to_string(file_path).map_err(|e| BlinkError::eval(format!("Failed to read file {:?}: {}", file_path, e)))?;
    
    // Quick scan for module declaration (this is a simple approach)
    // More robust would be to actually parse, but this is faster for searching
    let search_patterns = vec![
        format!("(mod {}", module_name),
        format!("(mod :declare {}", module_name),
        format!("(mod :enter {}", module_name),
    ];
    
    for pattern in search_patterns {
        if content.contains(&pattern) {
            return Ok(true);
        }
    }
    
    Ok(false)
}






pub fn eval_imp(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    let parsed = parse_import_args(args, ctx);
    let  (import_type, _options) = match parsed {
        Ok(parsed) => parsed,
        Err(e) => {
            return EvalResult::Value(ctx.error_value(e));
        }
    };
    
    match import_type {
        ImportType::File(file_name) => {
            // File import: (imp "module-name")
            let file_path = PathBuf::from(format!("lib/{}.blink", file_name));
            
            // Load the file if needed
            let loaded = eval_blink_file(file_path, ctx);
            try_eval!(loaded, ctx);
            
            // Make the file's modules available for qualified access
            // (they're already registered by eval_mod during file evaluation)
            EvalResult::Value(ctx.nil_value())
        },
        
        ImportType::Symbols { symbols, module, aliases } => {
            // Check if module already exists
            let module_exists = ctx.module_registry.read().get_module(module).is_some();
            let module_name = match ctx.resolve_symbol_name(module) {
                Some(name) => name,
                None => {
                    return EvalResult::Value(ctx.eval_error("Module not found"));
                }
            };
            
            if !module_exists {
                
                // Find which file contains the module
                let file_path = find_module_file(&module_name, ctx);
                if let Err(e) = file_path {
                    return EvalResult::Value(ctx.error_value(e));
                }
                let file_path = file_path.unwrap();
                
                // Load the file if needed (this registers all modules in the file)
                let loaded = eval_blink_file(file_path, ctx);
                try_eval!(loaded, ctx);
                
                // Verify the module is now available
                if ctx.module_registry.read().get_module(module).is_none() {
                    return EvalResult::Value(ctx.eval_error(&format!("Module '{}' was not found in the loaded file", module)));
                }
            }

            let symbol_values = symbols.iter().map(|s| ValueRef::symbol(*s)).collect::<Vec<ValueRef>>();
            
            // Import the symbols into current environment
            match import_symbols_into_env(&symbol_values, module, &aliases, ctx) {
                Ok(_) => (),
                Err(e) => {
                    return EvalResult::Value(ctx.error_value(e));
                }
            };
            EvalResult::Value(ctx.nil_value())
        }
    }
}

pub fn eval_quasiquote(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "quasiquote"));
    }
    expand_quasiquote(args[0], ctx)
}

fn expand_quasiquote(expr: ValueRef, ctx: &mut EvalContext) -> EvalResult {
    match expr {
        ValueRef::Shared(idx) => {
            let shared = ctx.shared_arena.read().get(idx).map(|s| s.clone());
            if let Some(shared) = shared {
                match shared.as_ref() {
                    SharedValue::List(items) if !items.is_empty() => {
                        let first = items[0];
                        
                        // Check if first item is 'unquote or 'unquote-splicing
                        if let Some(symbol_name) = ctx.get_symbol_name(first) {
                            match symbol_name.as_str() {
                                "unquote" => {
                                    if items.len() != 2 {
                                        return EvalResult::Value(ctx.eval_error("unquote expects exactly one argument"));
                                    }
                                    // Evaluate the unquoted form
                                    forward_eval!(trace_eval(items[1], ctx), ctx)
                                }
                                
                                "unquote-splicing" => {
                                    return EvalResult::Value(ctx.eval_error("unquote-splicing not valid here"));
                                }
                                
                                _ => {
                                    // Regular list - expand recursively
                                    expand_quasiquote_items_inline(items.clone(), 0, Vec::new(), false, ctx)
                                }
                            }
                        } else {
                            // First item is not a symbol - expand recursively
                            expand_quasiquote_items_inline(items.clone(), 0, Vec::new(), false, ctx)
                        }
                    }
                    
                    SharedValue::Vector(items) => {
                        // Expand vector recursively
                        expand_quasiquote_items_inline(items.clone(), 0, Vec::new(), true, ctx)
                    }
                    
                    // Other shared values are self-quoting
                    _ => EvalResult::Value(expr),
                }
            } else {
                EvalResult::Value(ctx.eval_error("Invalid reference in quasiquote"))
            }
        }
        
        // Immediate values and nil are self-quoting
        _ => EvalResult::Value(expr),
    }
}

fn expand_quasiquote_items_inline(
    items: Vec<ValueRef>,
    mut index: usize,
    mut expanded_items: Vec<ValueRef>,
    is_vector: bool,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= items.len() {
            // Create the appropriate collection type
            if is_vector {
                let vector = SharedValue::Vector(expanded_items);
                let value = ctx.shared_arena.write().alloc(vector);
                return EvalResult::Value(value);
            } else {
                let list = SharedValue::List(expanded_items);
                let value = ctx.shared_arena.write().alloc(list);
                return EvalResult::Value(value);
            }
        }

        let item = items[index];

        // Check for unquote-splicing
        if let ValueRef::Shared(item_idx) = item {
            let shared = ctx.shared_arena.read().get(item_idx).map(|s| s.clone());
            if let Some(shared) = shared {
                if let SharedValue::List(inner_items) = shared.as_ref() {
                    if !inner_items.is_empty() {
                        if let Some(symbol_name) = ctx.get_symbol_name(inner_items[0]) {
                            if symbol_name == "unquote-splicing" {
                                if inner_items.len() != 2 {
                                    return EvalResult::Value(ctx.eval_error("unquote-splicing expects exactly one argument"));
                                }

                                let result = trace_eval(inner_items[1], ctx);
                                match result {
                                    EvalResult::Value(spliced) => {
                                        if ctx.is_err(&spliced) {
                                            return EvalResult::Value(spliced);
                                        }
                                        
                                        // Extract list items to splice in
                                        if let Some(splice_items) = ctx.get_vec_or_list_items(spliced) {
                                            expanded_items.extend(splice_items);
                                        } else {
                                            return EvalResult::Value(ctx.eval_error("unquote-splicing expects a list"));
                                        }
                                        
                                        index += 1;
                                        continue;
                                    }
                                    EvalResult::Suspended { future, resume: _ } => {
                                        return EvalResult::Suspended {
                                            future,
                                            resume: Box::new(move |v, ctx| {
                                                if ctx.is_err(&v) {
                                                    return EvalResult::Value(v);
                                                }
                                                
                                                if let Some(splice_items) = ctx.get_vec_or_list_items(v) {
                                                    expanded_items.extend(splice_items);
                                                    expand_quasiquote_items_inline(items, index + 1, expanded_items, is_vector, ctx)
                                                } else {
                                                    EvalResult::Value(ctx.eval_error("unquote-splicing expects a list"))
                                                }
                                            }),
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Regular item expansion
        let result = expand_quasiquote(item, ctx);
        match result {
            EvalResult::Value(expanded) => {
                if ctx.is_err(&expanded) {
                    return EvalResult::Value(expanded);
                }
                expanded_items.push(expanded);
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if ctx.is_err(&v) {
                            return EvalResult::Value(v);
                        }
                        expanded_items.push(v);
                        expand_quasiquote_items_inline(items, index + 1, expanded_items, is_vector, ctx)
                    }),
                };
            }
        }
    }
}

pub fn eval_macro(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(ctx.eval_error("macro expects at least 2 arguments: params and body"));
    }

    let (params, is_variadic) = match extract_macro_params(args[0], ctx) {
        Ok(result) => result,
        Err(err_ref) => return EvalResult::Value(err_ref),
    };

    // Create the macro value and store in arena
    let macro_data = Macro {
        params,
        body: args[1..].to_vec(),
        env: ctx.env.clone(),
        is_variadic,
    };
    
    let macro_ref = ctx.shared_arena.write().alloc(SharedValue::Macro(macro_data));
    EvalResult::Value(macro_ref)
}

fn extract_macro_params(param_expr: ValueRef, ctx: &mut EvalContext) -> Result<(Vec<u32>, bool), ValueRef> {
    // Use the existing method from your EvalContext
    match ctx.get_vector_of_symbols(param_expr) {
        Ok(symbol_names) => {
            parse_variadic_params(symbol_names, ctx)
        }
        Err(err_msg) => {
            Err(ctx.eval_error(&err_msg))
        }
    }
}

fn parse_variadic_params(symbol_ids: Vec<u32>, ctx: &mut EvalContext) -> Result<(Vec<u32>, bool), ValueRef> {
    let mut params = Vec::new();
    let mut is_variadic = false;
    let mut i = 0;

    while i < symbol_ids.len() {
        let symbol_id = &symbol_ids[i];
        let symbol_name = ctx.resolve_symbol_name(*symbol_id).unwrap();
        
        if symbol_name == "&" {
            // Next symbol is the rest parameter
            if i + 1 < symbol_ids.len() {
                params.push(symbol_ids[i + 1].clone());
                is_variadic = true;
                break;
            } else {
                return Err(ctx.eval_error("& must be followed by a parameter name"));
            }
        } else {
            params.push(symbol_id.clone());
        }
        
        i += 1;
    }

    Ok((params, is_variadic))
}

pub fn eval_deref(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(args, 1, "deref") {
        return EvalResult::Value(ctx.error_value(e));
    }
    let future_val = try_eval!(trace_eval(args[0].clone(), ctx), ctx);

    match ctx.async_ctx {
        AsyncContext::Blocking => {
            let future_opt = ctx.get_future(future_val);
            if let Some(future) = future_opt {
                let res = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(future.clone())
                        
                });
                EvalResult::Value(res)
            } else {
                EvalResult::Value(ctx.eval_error("deref can only be used on futures"))
            }
            
        },
        AsyncContext::Goroutine(_) => {
            let future_opt = ctx.get_future(future_val);

            if let Some(future) = future_opt {
                let future_clone = future.clone();
                EvalResult::Suspended { future: future_clone, resume: Box::new(|resolved, _ctx| {
                    EvalResult::Value(resolved)
                }) }
            } else {
                EvalResult::Value(ctx.eval_error("deref can only be used on futures"))
            }
            
        }
        
    }
}

pub fn eval_go(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "go"));
    }

    let goroutine_ctx = ctx.clone();
    let expr = args[0].clone();

    ctx.goroutine_scheduler.spawn_with_context(goroutine_ctx, move |ctx| {
        trace_eval(expr, ctx)
    });

    EvalResult::Value(ctx.nil_value())
}