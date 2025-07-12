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
    module::{ImportType, Module, SerializedModuleSource},
    runtime::AsyncContext,
    value::{unpack_immediate, Callable, GcPtr, HeapValue, ImmediateValue, Plugin, ValueRef},
};

fn require_arity(args: &[ValueRef], expected: usize, form_name: &str) -> Result<(), BlinkError> {
    if args.len() != expected {
        return Err(BlinkError::arity(expected, args.len(), form_name));
    }
    Ok(())
}

// Helper function to extract symbol names from an environment
fn extract_env_symbols(env: &Env) -> Vec<u32> {
    env.vars.iter().map(|(key, _)| *key).collect()
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

    // Bind the symbol to the value in the current module
    // Always get read the latest version of the module
    let current_module_ref = ctx.get_module(ctx.current_module);
    if let Some(module_ref) = current_module_ref {
        let module = GcPtr::new(module_ref).read_module();
        let mut module_env = GcPtr::new(module.env).read_env();
        module_env.set(sym, value);
        
        // Copy on write
        let new_module_env_ref = ctx.vm.alloc_env(module_env);
        let new_module = Module {
            name: module.name,
            env: new_module_env_ref,
            exports: module.exports,
            source: module.source,
            ready: module.ready,
        };
        // This allocates a new module and registers it in the module registry
        // Gc should then clean up the old module
        ctx.register_module(&new_module);
        

    } else {
        return EvalResult::Value(ctx.eval_error("Current module not found"));
    }

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
    let user_fn = Callable {
        params,
        body: args[1..].to_vec(),
        env: ctx.env,
        is_variadic: false, //TODO: handle variadic functions
    };

    let value_ref = ctx.user_defined_function_value(user_fn);

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
    let mut exports_set = Vec::new();
    for (func_name, native_fn) in plugin.functions {
        // Convert function name to symbol ID for exports

        let func_symbol = ctx.symbol_value(&func_name);
        let func_symbol_id = ctx.get_symbol_id(func_symbol).unwrap();
        exports_set.push(func_symbol_id);

        // Create ValueRef that wraps the NativeFn
        let function_value = ctx.native_function_value(native_fn);

        // Set in environment
        module_env.set(func_symbol_id, function_value);
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

fn import_symbols_into_env(
    symbols: &[ValueRef],
    module_name: u32,
    aliases: &HashMap<u32, u32>,
    ctx: &mut EvalContext,
) -> Result<(), BlinkError> {
    // new env that points to parent
    let mut import_frame = Env::with_parent(ctx.env);
    
    let module_ref = ctx
        .get_module(module_name)
        .ok_or_else(|| BlinkError::eval(format!("Module '{}' not found", module_name)))?;

    let module_read = GcPtr::new(module_ref).read_module();

    // Handle import all (*) - check first symbol only
    if symbols.len() == 1 {
        if let Some(first_name) = ctx.get_symbol_name(symbols[0].clone()) {
            if first_name == ":all" {
                for export_name in &module_read.exports {
                    let local_name = aliases.get(export_name).unwrap_or(export_name);
                    let reference = ctx.module_value(module_name, *export_name);
                    import_frame.set(*local_name, reference);
                }
                let import_frame_ref = ctx.vm.alloc_env(import_frame);
                ctx.env = import_frame_ref;
                return Ok(());
            }
        }
    }

    // Import specific symbols
    for symbol_ref in symbols {
        let symbol_name = ctx
            .get_symbol_id(*symbol_ref)
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
        import_frame.set(*local_name, reference);
    }

    let import_frame_ref = ctx.vm.alloc_env(import_frame);
    ctx.env = import_frame_ref;

    Ok(())
}

fn load_native_code(args: &[ValueRef], ctx: &mut EvalContext) -> Result<EvalResult, BlinkError> {
    use libloading::Library;
    use std::collections::HashSet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    let pos = args.get(0).and_then(|v| ctx.get_pos(*v));

    if args.len() < 1 || args.len() > 2 {
        return Err(BlinkError::arity(1, args.len(), "compile-plugin").with_pos(pos));
    }

    let plugin_name = match args[0].get_string() {
        Some(plugin_name) => plugin_name,
        None => {
            return Err(
                BlinkError::eval("compile-plugin expects a string as first argument").with_pos(pos),
            );
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
                if v.is_error() {
                    return Err(BlinkError::eval("Second argument must be a map").with_pos(pos));
                }

                v
            }
            suspended => return Ok(suspended),
        };
        if let Some(opt_map) = options_val.get_map() {
            for (key, value) in opt_map {
                if let Some(kw_name) = ctx.get_keyword_name(key) {
                    match &*kw_name {
                        "path" => {
                            if let Some(path) = value.get_string() {
                                plugin_path = path;
                            }
                        }
                        "import" => {
                            if let Some(b) = ctx.get_bool(value) {
                                auto_import = b;
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else {
            return Err(BlinkError::eval("Second argument must be a map").with_pos(pos));
        }
    }

    // Check if plugin directory exists
    if !Path::new(&plugin_path).exists() {
        return Err(
            BlinkError::eval(format!("Plugin path '{}' does not exist", plugin_path)).with_pos(pos),
        );
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
    let dest_symbol_id = ctx.intern_symbol(&dest);
    let dest_symbol_id = ctx.get_symbol_id(dest_symbol_id).unwrap();

    // Create native directory and copy library
    fs::create_dir_all("native").ok();
    fs::copy(&source, &dest).map_err(|e| {
        BlinkError::eval(format!("Failed to copy compiled plugin: {}", e)).with_pos(pos)
    })?;

    // Load the library
    let lib = unsafe { Library::new(&dest) }.map_err(|e| {
        BlinkError::eval(format!("Failed to load compiled plugin: {}", e)).with_pos(pos)
    })?;

    // Try the new registration function first (with exports)
    let exports = match unsafe {
        lib.get::<unsafe extern "C" fn(&mut Env) -> Vec<String>>(b"blink_register_with_exports")
    } {
        Ok(register_with_exports) => {
            // Create a new environment for the module
            let mut module_env = Env::with_parent(ctx.get_global_env());
            let mut exports_set: Vec<u32> = Vec::new();
            let exported_names = unsafe { register_with_exports(&mut module_env) };
            for name in exported_names {
                let symbol = ctx.intern_symbol(&name);
                let symbol_id = ctx.get_symbol_id(symbol).unwrap();
                exports_set.push(symbol_id);
            }

            // Register the module with known exports
            let module_env_ref = ctx.vm.alloc_env(module_env);

            let module = Module {
                name: plugin_symbol_id,
                source: SerializedModuleSource::NativeDylib(dest_symbol_id),
                exports: exports_set.clone(),
                env: module_env_ref,
                ready: true,
            };
            let _module_ref = ctx.register_module(&module);

            exports_set
        }
        Err(_) => {
            // Fall back to old registration function (no export tracking)
            let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
                lib.get(b"blink_register").map_err(|e| {
                    BlinkError::eval(format!(
                        "Failed to find blink_register or blink_register_with_exports: {}",
                        e
                    ))
                    .with_pos(pos)
                })?
            };

            // Create a new environment for the module°
            let mut module_env = Env::with_parent(ctx.get_global_env());
            unsafe { register(&mut module_env) };

            // We don't know the exports, so we'll have to extract them from the environment
            let export_symbols = extract_env_symbols(&module_env);
            let module_env_ref = ctx.vm.alloc_env(module_env);
            let exports = export_symbols
                .iter()
                .map(|s| ValueRef::symbol(*s))
                .collect::<Vec<ValueRef>>();
            let module = Module {
                name: plugin_symbol_id,
                source: SerializedModuleSource::NativeDylib(dest_symbol_id),
                exports: export_symbols.clone(),
                env: module_env_ref,
                ready: true,
            };
            // Register the module
            let module_ref = ctx.register_module(&module);
            if auto_import {
                import_symbols_into_env(&exports, plugin_symbol_id, &HashMap::new(), ctx)?;
            }

            export_symbols
        }
    };
    let exports = exports
        .iter()
        .map(|s| ValueRef::symbol(*s))
        .collect::<Vec<ValueRef>>();
    // Store the library to prevent it from being unloaded
    ctx.store_native_library(dest_symbol_id, lib);
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
                if val.is_error() {
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
                        eval_file_forms_inline(forms, index + 1, ctx)
                    }),
                };
            }
        }
    }
}

// Load and evaluate a Blink source file
fn eval_blink_file(file_path_symbol_id: u32, ctx: &mut EvalContext) -> EvalResult {
    let file_path = match ctx.get_symbol_name_from_id(file_path_symbol_id) {
        Some(file_path) => PathBuf::from(file_path),
        None => {
            return EvalResult::Value(ctx.eval_error("File name missing."));
        }
    };
    
    
    

    // Don't evaluate if already loaded
    if ctx.is_file_evaluated(file_path_symbol_id) {
        return EvalResult::Value(ctx.nil_value());
    }

    let contents = match fs::read_to_string(&file_path) {
        Ok(contents) => contents,
        Err(e) => {
            return EvalResult::Value(ctx.eval_error(&format!("Failed to read file: {}", e)));
        }
    };
    // Parse the file
    let parsed_forms = crate::parser::parse_all(
        &contents,
        &mut ctx.vm.reader_macros.write(),
        &mut ctx.vm.symbol_table.write(),
    );

    let parsed_forms = match parsed_forms {
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
                ctx.intern_symbol_id(file_name)
            } else {
                return EvalResult::Value(ctx.eval_error("File name missing."));
            }
        }
        None => {
            return EvalResult::Value(ctx.eval_error("File name missing."));
        }
    };
    ctx.current_file = Some(file_name);

    // Evaluate all forms in the file
    try_eval!(eval_file_forms_inline(forms, 0, ctx), ctx);

    // Restore previous file context
    ctx.current_file = old_file;

    // Mark file as evaluated
    ctx.mark_file_evaluated(file_path_symbol_id);

    EvalResult::Value(ctx.nil_value())
}

fn parse_load_args(
    args: &[ValueRef],
    ctx: &mut EvalContext,
) -> Result<(String, String), BlinkError> {
    if args.is_empty() {
        return Err(BlinkError::arity(2, 0, "load"));
    }

    // First argument should be a keyword indicating source type
    let source_type = ctx.get_keyword_name(args[0].clone()).ok_or_else(|| {
        BlinkError::eval(
            "load expects a keyword as first argument (:file, :native, :cargo, :dylib, :url, :git)",
        )
    })?;

    // Second argument should be the source value
    if args.len() < 2 {
        return Err(BlinkError::eval(format!(
            "load {} requires a source argument",
            source_type
        )));
    }

    let source_value = match args[1].get_string() {
        Some(s) => s,
        None => return Err(BlinkError::eval("load source must be a string")),
    };

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
            let file_path_symbol_id = ctx.intern_symbol_id(&source_value);
            let loaded = eval_blink_file(file_path_symbol_id, ctx);
            try_eval!(loaded, ctx);
            EvalResult::Value(ctx.nil_value())
        }

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
                    return EvalResult::Value(ctx.eval_error(&format!(
                        "Directory '{}' is not a valid Cargo project (no Cargo.toml found)",
                        source_value
                    )));
                }
            } else if path
                .extension()
                .map_or(false, |ext| ext == "so" || ext == "dll" || ext == "dylib")
            {
                // It's a pre-built library file
                load_native_library(&args, ctx)
            } else {
                return EvalResult::Value(ctx.eval_error(&format!(
                    "'{}' is neither a Cargo project directory nor a native library file",
                    source_value
                )));
            }
        }

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
        _ => EvalResult::Value(
            ctx.eval_error(&format!("Unknown load source type: :{}", source_type)),
        ),
    }
}

fn parse_import_args(
    args: &[ValueRef],
    ctx: &mut EvalContext,
) -> Result<(ImportType, Option<HashMap<String, ValueRef>>), BlinkError> {
    if args.is_empty() {
        return Err(BlinkError::arity(1, 0, "imp"));
    }

    let first_arg = args[0];
    let heap = first_arg.read_heap_value();
    if let Some(heap) = heap {
        match heap {
            // File import: (imp "module-name")
            HeapValue::Str(s) => Ok((ImportType::File(s.clone()), None)),
            // Vector import: (imp [sym1 sym2] :from module)
            HeapValue::Vector(symbols) => {
                let (import_type, options) = parse_symbol_import(&symbols, &args[1..], ctx)?;
                Ok((import_type, Some(options)))
            }
            _ => Err(BlinkError::eval(
                "imp expects a string (file) or vector (symbols)",
            )),
        }
    } else {
        Err(BlinkError::eval(
            "imp expects a string (file) or vector (symbols)",
        ))
    }
}

fn parse_symbol_import(
    symbol_list: &[ValueRef],
    remaining_args: &[ValueRef],
    ctx: &mut EvalContext,
) -> Result<(ImportType, HashMap<String, ValueRef>), BlinkError> {
    // Parse symbols and any aliases
    let mut symbols = Vec::new();
    let aliases = HashMap::new();

    let i = 0;
    while i < symbol_list.len() {
        match &symbol_list[i] {
            ValueRef::Immediate(packed) => {
                let unpacked = unpack_immediate(*packed);
                if let ImmediateValue::Symbol(symbol_id) = unpacked {
                    symbols.push(symbol_id);
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
                }
                other => {
                    return Err(BlinkError::eval(format!(
                        "Unknown import option: {}",
                        other
                    )));
                }
            }
        } else {
            return Err(BlinkError::eval("Expected keyword after symbol list"));
        }
    }

    let module =
        module_name.ok_or_else(|| BlinkError::eval("Symbol import requires :from module-name"))?;

    Ok((
        ImportType::Symbols {
            symbols,
            module,
            aliases,
        },
        options,
    ))
}

pub fn eval_def_reader_macro(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "def-reader-macro"));
    }

    let char_val = match &args[0].get_string() {
        Some(s) => s.clone(),
        None => {
            return EvalResult::Value(
                ctx.eval_error("First argument to def-reader-macro must be a string"),
            );
        }
        _ => {
            return EvalResult::Value(
                ctx.eval_error("First argument to def-reader-macro must be a string"),
            );
        }
    };

    let ch = char_val;

    let func = try_eval!(trace_eval(args[1].clone(), ctx), ctx);
    let func_symbol = ctx.intern_symbol(&ch);
    let func_symbol_id = ctx.get_symbol_id(func_symbol).unwrap();
    ctx.set_symbol(func_symbol_id, func);
    ctx.vm
        .reader_macros
        .write()
        .reader_macros
        .insert(ch, func_symbol_id);
    EvalResult::Value(func)
}

/// Helper to update module exports
fn update_module_exports(
    module: &mut Module,
    exports_val: &ValueRef,
    ctx: &mut EvalContext,
) -> Result<(), BlinkError> {
    let exports = exports_val.get_vec();

    if let Some(exports) = exports {
        for export in exports {
            let sym = ctx.get_symbol_id(export);
            if let Some(sym) = sym {
                module.exports.push(sym);
            }
        }
    } else if let Some(kw) = exports_val.get_keyword() {
        let kw_str = ctx.get_keyword_name(*exports_val);
        if kw_str == Some("all".to_string()) {
            let all_keys = GcPtr::new(module.env).read_env().vars.iter().map(|(key, _)| *key).collect();
            module.exports = all_keys;
            return Ok(());
        }
    } else {
        return Err(BlinkError::eval("Exports must be a list of symbols"));
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
    if let Some(name) = ctx.get_symbol_name(*value) {
        Ok(name.clone())
    } else {
        Err(BlinkError::eval("Expected a symbol for name"))
    }
}

fn parse_mod_options(
    args: &[ValueRef],
    ctx: &mut EvalContext,
) -> Result<(HashMap<String, ValueRef>, usize), BlinkError> {
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
                }
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

    let name: u32 = match ctx.get_symbol_id(args[name_index]) {
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

    let module_ref = ctx.get_module(name);
    let mut module: Module;
    let repl_symbol_id = ctx.intern_symbol_id("<repl>");

    let module = match (module_ref, should_declare) {
        (Some(module_ref), true) => {
            // Existing module but declaring so possibly adding exports
            module = GcPtr::new(module_ref).read_module();
            
            let current_file = ctx
                .current_file
                .clone()
                .unwrap_or_else(|| repl_symbol_id);
            let source = if current_file == repl_symbol_id {
                SerializedModuleSource::Repl
            } else {
                SerializedModuleSource::BlinkFile(current_file)
            };

            if options.contains_key("exports") {
            match update_module_exports(&mut module, &options["exports"], ctx) {
                    Ok(_) => (),
                    Err(e) => {
                        return EvalResult::Value(ctx.error_value(e));
                    }
                }
            };

            // need to allocate a new module with the new exports/source
            let new_module = Module {
                name: module.name,
                env: module.env,
                exports: module.exports,
                source: source,
                ready: true,
            };

            let module_ref = ctx.register_module(&new_module);

            new_module
        
            
        }
        (Some(module_ref), false) => {
            // Just entering existing module
            GcPtr::new(module_ref).read_module()
            
        }
        (None, true) => { 
            // New module
            let module_env = Env::with_parent(ctx.get_global_env());
            let module_env_ref = ctx.vm.alloc_env(module_env);

            let current_file = ctx
                .current_file
                .clone()
                .unwrap_or_else(|| repl_symbol_id);
            let source = if current_file == repl_symbol_id {
                SerializedModuleSource::Repl
            } else {
                SerializedModuleSource::BlinkFile(current_file)
            };

            let mut module = Module {
                name: name,
                env: module_env_ref,
                exports: vec![],
                source: source,
                ready: true,
            };

            if options.contains_key("exports") {
                match update_module_exports(&mut module, &options["exports"], ctx) {
                    Ok(_) => (),
                    Err(e) => {
                        return EvalResult::Value(ctx.error_value(e));
                    }
                }
            };




            let _module_ref = ctx.register_module(&module);

            module
        }
        (None, false) => {
            // Trying to enter non-existent module
            return EvalResult::Value(ctx.eval_error(&format!("Module {} not found", ctx.get_symbol_name(args[name_index]).unwrap_or("Unknown".to_string()))));
        }
    };

    

    if should_enter {
        ctx.current_module = module.name;
        ctx.env = module.env;
    }

    EvalResult::Value(ctx.nil_value())
}

fn find_module_file(module_name: &str, ctx: &mut EvalContext) -> Result<u32, BlinkError> {
    // 1. Check if module is already registered (we know which file it came from)
    let module_symbol_id = ctx.intern_symbol(module_name);
    let module_symbol_id = ctx.get_symbol_id(module_symbol_id).unwrap();

    if let Some(module) = ctx.get_module(module_symbol_id) {
        let module = GcPtr::new(module).read_module();
        if let SerializedModuleSource::BlinkFile(ref path) = module.source {
            return Ok(*path);
        }
    }

    // 2. Try direct file mapping first (most common case)
    let direct_path = PathBuf::from(format!("lib/{}.blink", module_name));
    if direct_path.exists() {
        let path_id = ctx.intern_symbol_id(&direct_path.to_string_lossy());
        return Ok(path_id);
    }

    // 3. Try parent directory approach (for multi-module files)
    let parts: Vec<&str> = module_name.split('/').collect();
    for i in (1..parts.len()).rev() {
        let parent_path = parts[..i].join("/");
        let candidate = PathBuf::from(format!("lib/{}.blink", parent_path));

        if candidate.exists() {
            // Check if this file actually contains our target module
            if file_contains_module(&candidate, module_name)? {
                let path_id = ctx.intern_symbol_id(&candidate.to_string_lossy());
                return Ok(path_id);
            }
        }
    }

    // 4. Search common patterns
    let search_candidates = vec![
        format!("lib/{}.bl", parts.join("-")), // math/utils -> math-utils.blink
        format!("lib/{}.bl", parts.last().unwrap()), // math/utils -> utils.blink
        format!("lib/{}/mod.bl", parts[0]),    // math/utils -> math/mod.blink
    ];

    for candidate_str in search_candidates {
        let candidate = PathBuf::from(candidate_str);
        if candidate.exists() && file_contains_module(&candidate, module_name)? {
            let path_id = ctx.intern_symbol_id(&candidate.to_string_lossy());
            return Ok(path_id);
        }
    }

    Err(BlinkError::eval(format!(
        "Module '{}' not found. Tried:\n  lib/{}.bl\n  lib/{}.bl\n  And parent directories",
        module_name,
        module_name,
        parts.join("-")
    )))
}

// Helper function to check if a file contains a specific module declaration
fn file_contains_module(file_path: &PathBuf, module_name: &str) -> Result<bool, BlinkError> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| BlinkError::eval(format!("Failed to read file {:?}: {}", file_path, e)))?;

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
    // TODO revisit the way we find modules/files

    let parsed = parse_import_args(args, ctx);
    let (import_type, _options) = match parsed {
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
            let file_path_symbol_id = ctx.intern_symbol_id(&file_path.to_string_lossy());
            let loaded = eval_blink_file(file_path_symbol_id, ctx);
            try_eval!(loaded, ctx);

            // Make the file's modules available for qualified access
            // (they're already registered by eval_mod during file evaluation)
            EvalResult::Value(ctx.nil_value())
        }

        ImportType::Symbols {
            symbols,
            module,
            aliases,
        } => {
            // Check if module already exists
            let module_exists = ctx.get_module(module).is_some();
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
                if ctx.get_module(module).is_none() {
                    return EvalResult::Value(ctx.eval_error(&format!(
                        "Module '{}' was not found in the loaded file",
                        module
                    )));
                }
            }

            let symbol_values = symbols
                .iter()
                .map(|s| ValueRef::symbol(*s))
                .collect::<Vec<ValueRef>>();

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
    if let Some(heap_value) = expr.read_heap_value() {
        match heap_value {
            HeapValue::List(items) if !items.is_empty() => {
                let first = items[0];
                // Check if first item is 'unquote or 'unquote-splicing
                if let Some(symbol_name) = ctx.get_symbol_name(first) {
                    match symbol_name.as_str() {
                        "unquote" => {
                            if items.len() != 2 {
                                return EvalResult::Value(
                                    ctx.eval_error("unquote expects exactly one argument"),
                                );
                            }
                            // Evaluate the unquoted form
                            forward_eval!(trace_eval(items[1], ctx), ctx)
                        }

                        "unquote-splicing" => {
                            return EvalResult::Value(
                                ctx.eval_error("unquote-splicing not valid here"),
                            );
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
            HeapValue::Vector(items) => {
                expand_quasiquote_items_inline(items.clone(), 0, Vec::new(), true, ctx)
            }
            _ => EvalResult::Value(expr),
        }
    } else {
        EvalResult::Value(expr)
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
                let value = ctx.vector_value(expanded_items);
                return EvalResult::Value(value);
            } else {
                let value = ctx.list_value(expanded_items);
                return EvalResult::Value(value);
            }
        }

        let item = items[index];

        // Check for unquote-splicing
        if let Some(heap_value) = item.read_heap_value() {
            if let HeapValue::List(inner_items) = heap_value {
                if let Some(symbol_name) = ctx.get_symbol_name(inner_items[0]) {
                    if symbol_name == "unquote-splicing" {
                        if inner_items.len() != 2 {
                            return EvalResult::Value(
                                ctx.eval_error("unquote-splicing expects exactly one argument"),
                            );
                        }
                        let result = trace_eval(inner_items[1], ctx);
                        match result {
                            EvalResult::Value(spliced) => {
                                if spliced.is_error() {
                                    return EvalResult::Value(spliced);
                                }

                                // Extract list items to splice in
                                if let Some(splice_items) = spliced.get_vec() {
                                    expanded_items.extend(splice_items);
                                } else if let Some(splice_items) = spliced.get_list() {
                                    expanded_items.extend(splice_items);
                                } else {
                                    return EvalResult::Value(
                                        ctx.eval_error("unquote-splicing expects a list"),
                                    );
                                }

                                index += 1;
                                continue;
                            }
                            EvalResult::Suspended { future, resume: _ } => {
                                return EvalResult::Suspended {
                                    future,
                                    resume: Box::new(move |v, ctx| {
                                        if v.is_error() {
                                            return EvalResult::Value(v);
                                        }

                                        if let Some(splice_items) = v.get_vec() {
                                            expanded_items.extend(splice_items);
                                            expand_quasiquote_items_inline(
                                                items,
                                                index + 1,
                                                expanded_items,
                                                is_vector,
                                                ctx,
                                            )
                                        } else if let Some(splice_items) = v.get_list() {
                                            expanded_items.extend(splice_items);
                                            expand_quasiquote_items_inline(
                                                items,
                                                index + 1,
                                                expanded_items,
                                                is_vector,
                                                ctx,
                                            )
                                        } else {
                                            EvalResult::Value(
                                                ctx.eval_error("unquote-splicing expects a list"),
                                            )
                                        }
                                    }),
                                };
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
                if expanded.is_error() {
                    return EvalResult::Value(expanded);
                }
                expanded_items.push(expanded);
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        expanded_items.push(v);
                        expand_quasiquote_items_inline(
                            items,
                            index + 1,
                            expanded_items,
                            is_vector,
                            ctx,
                        )
                    }),
                };
            }
        }
    }
}

pub fn eval_macro(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(
            ctx.eval_error("macro expects at least 2 arguments: params and body"),
        );
    }

    let (params, is_variadic) = match extract_macro_params(args[0], ctx) {
        Ok(result) => result,
        Err(err_ref) => return EvalResult::Value(err_ref),
    };

    // Create the macro value and store in arena
    let macro_data = Callable {
        params,
        body: args[1..].to_vec(),
        env: ctx.env.clone(),
        is_variadic,
    };

    let macro_ref = ctx.macro_value(macro_data);
    EvalResult::Value(macro_ref)
}

fn extract_macro_params(
    param_expr: ValueRef,
    ctx: &mut EvalContext,
) -> Result<(Vec<u32>, bool), ValueRef> {
    // Use the existing method from your EvalContext
    match ctx.get_vector_of_symbols(param_expr) {
        Ok(symbol_names) => parse_variadic_params(symbol_names, ctx),
        Err(err_msg) => Err(ctx.eval_error(&err_msg)),
    }
}

fn parse_variadic_params(
    symbol_ids: Vec<u32>,
    ctx: &mut EvalContext,
) -> Result<(Vec<u32>, bool), ValueRef> {
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
            let future_opt = future_val.get_future();
            if let Some(future) = future_opt {
                let res = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(future.clone())
                });
                EvalResult::Value(res)
            } else {
                EvalResult::Value(ctx.eval_error("deref can only be used on futures"))
            }
        }
        AsyncContext::Goroutine(_) => {
            let future_opt = future_val.get_future();

            if let Some(future) = future_opt {
                let future_clone = future.clone();
                EvalResult::Suspended {
                    future: future_clone,
                    resume: Box::new(|resolved, _ctx| EvalResult::Value(resolved)),
                }
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

    // Mediated through runtime
    // TODO: Implement this
    // ctx.goroutine_scheduler.spawn_with_context(goroutine_ctx, move |ctx| {
    //     trace_eval(expr, ctx)
    // });

    EvalResult::Value(ctx.nil_value())
}
