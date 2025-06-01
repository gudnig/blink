use crate::env::Env;
use crate::error::LispError;
use crate::module::{ImportType, Module, ModuleRegistry, ModuleSource};
use crate::parser::{ ReaderContext};
use crate::telemetry::TelemetryEvent;
use crate::value::{bool_val, list_val, nil, BlinkValue, LispNode, SourceRange, Value};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub struct EvalContext {
    pub global_env: Arc<RwLock<Env>>,
    pub env: Arc<RwLock<Env>>,
    pub telemetry_sink: Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>,
    pub tracing_enabled: bool,
    pub current_module: Option<String>,
    pub module_registry: ModuleRegistry,
    pub file_to_modules: HashMap<PathBuf, Vec<String>>,
    pub current_file: Option<String>, // Is this needed?
    pub reader_macros: Arc<RwLock<ReaderContext>>,
}

impl EvalContext {
    pub fn new(parent: Arc<RwLock<Env>>) -> Self {
        EvalContext {
            global_env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            current_module: None,
            telemetry_sink: None,
            module_registry: ModuleRegistry::new(),
            current_file: None,
            tracing_enabled: false,
            reader_macros: Arc::new(RwLock::new(ReaderContext::new())),
            file_to_modules: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<BlinkValue> {
        self.env.read().get_with_registry(key, &self.module_registry)
    }

    pub fn set(&self, key: &str, val: BlinkValue) {
        self.env.write().set(key, val)
    }

    pub fn current_env(&self) -> Arc<RwLock<Env>> {
        self.env.clone()
    }

    pub fn register_module(&mut self, module: Module) {
        self.module_registry.register_module(module);
    }
    
    pub fn get_module(&self, name: &str) -> Option<Arc<RwLock<Module>>> {
        self.module_registry.get_module(name)
    }

    pub fn find_module_file(&self, module_name: &str) -> Option<PathBuf> {
        self.module_registry.find_module_file(module_name)
    }
}


fn get_pos(expr: &BlinkValue) -> Option<SourceRange> {
    expr.read().pos.clone()
}

pub fn eval(expr: BlinkValue, ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    match &expr.read().value {
        Value::Number(_) | Value::Bool(_) | Value::Str(_) | Value::Keyword(_) | Value::Nil => {
            Ok(expr.clone())
        }
        Value::Symbol(sym) => ctx.get(sym).ok_or_else(|| LispError::UndefinedSymbol {
            name: sym.clone(),
            pos: get_pos(&expr),
        }),
        Value::List(list) if list.is_empty() => Ok(nil()),
        Value::List(list) => {
            let head = &list[0];
            let head_val = &head.read().value;
            match head_val {
                Value::Symbol(s) => match s.as_str() {
                    "quote" => eval_quote(list),
                    "apply" => eval_apply(&list[1..], ctx),
                    "if" => eval_if(&list[1..], ctx),
                    "def" => eval_def(&list[1..], ctx),
                    "fn" => eval_fn(&list[1..], ctx),
                    "do" => eval_do(&list[1..], ctx),
                    "let" => eval_let(&list[1..], ctx),
                    "and" => eval_and(&list[1..], ctx),
                    "or" => eval_or(&list[1..], ctx),
                    "try" => eval_try(&list[1..], ctx),
                    "imp" => eval_imp(&list[1..], ctx),
                    "mod" => eval_mod(&list[1..], ctx),
                    "load" => eval_load(&list[1..], ctx),
                    "macro" => eval_macro(&list[1..], ctx),
                    "rmac" => eval_def_reader_macro(&list[1..], ctx),
                    "quasiquote" => eval_quasiquote(&list[1..], ctx),
                    "unquote" => Err(LispError::EvalError {
                        message: "unquote used outside quasiquote".into(),
                        pos: None,
                    }),
                    "unquote-splicing" => Err(LispError::EvalError {
                        message: "unquote-splicing used outside quasiquote".into(),
                        pos: None,
                    }),

                    _ => {
                        let func = trace_eval(list[0].clone(), ctx)?;
                        let mut args = Vec::new();
                        for arg in &list[1..] {
                            args.push(trace_eval(arg.clone(), ctx)?);
                        }
                        eval_func(func, args, ctx)
                    }
                },
                _ => {
                    let func = trace_eval(list[0].clone(), ctx)?;
                    let mut args = Vec::new();
                    for arg in &list[1..] {
                        args.push(trace_eval(arg.clone(), ctx)?);
                    }
                    eval_func(func, args, ctx)
                }
            }
        }
        _ => Err(LispError::EvalError {
            message: "Unknown expression type".into(),
            pos: get_pos(&expr),
        }),
    }
}

pub fn trace_eval(expr: BlinkValue, ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if ctx.tracing_enabled {
        let start = Instant::now();
        let res = eval(expr.clone(), ctx);
        if let Ok(val) = &res {
            if let Some(sink) = &ctx.telemetry_sink {
                sink(TelemetryEvent {
                    form: format!("{:?}", expr.read().value),
                    duration_us: start.elapsed().as_micros(),
                    result_type: val.read().value.type_tag().to_string(),
                    result_size: None,
                    source: expr.read().pos.clone(),
                });
            }
        }
        res
    } else {
        eval(expr, ctx)
    }
}

fn require_arity(args: &[BlinkValue], expected: usize, form_name: &str) -> Result<(), LispError> {
    if args.len() != expected {
        return Err(LispError::ArityMismatch {
            expected,
            got: args.len(),
            form: form_name.to_string(),
            pos: None,
        });
    }
    Ok(())
}

pub fn eval_quote(args: &Vec<BlinkValue>) -> Result<BlinkValue, LispError> {
    require_arity(&args[1..], 1, "quote")?;
    Ok(args[1].clone())
}

pub fn eval_func(
    func: BlinkValue,
    args: Vec<BlinkValue>,
    ctx: &mut EvalContext,
) -> Result<BlinkValue, LispError> {
    
    match &func.read().value {
        Value::NativeFunc(f) => f(args).map_err(|e| LispError::EvalError {
            message: e,
            pos: None,
        }),
        Value::Macro { params, body, env, is_variadic } => {
            // Check arity
            if *is_variadic {
                if args.len() < params.len() - 1 {
                    return Err(LispError::ArityMismatch {
                        expected: params.len() - 1,
                        got: args.len(),
                        form: "macro (at least)".into(),
                        pos: None,
                    });
                }
            } else if params.len() != args.len() {
                return Err(LispError::ArityMismatch {
                    expected: params.len(),
                    got: args.len(),
                    form: "macro".into(),
                    pos: None,
                });
            }
        
            // Create macro expansion environment
            let macro_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));
            
            // Bind macro parameters
            {
                let mut env_guard = macro_env.write();
                
                if *is_variadic {
                    // Bind regular parameters
                    for (i, param) in params.iter().take(params.len() - 1).enumerate() {
                        env_guard.set(param, args[i].clone());
                    }
                    
                    // Bind rest parameter to remaining arguments as a list
                    let rest_param = &params[params.len() - 1];
                    let rest_args = args.iter().skip(params.len() - 1).cloned().collect();
                    env_guard.set(rest_param, list_val(rest_args));
                } else {
                    // Bind all parameters normally
                    for (param, arg) in params.iter().zip(args.iter()) {
                        env_guard.set(param, arg.clone());
                    }
                }
            }
        
            // Save current context environment
            let old_env = ctx.env.clone();
            
            // Switch to macro environment for expansion
            ctx.env = macro_env;
        
            // Evaluate macro body to produce the expansion
            let mut expansion = nil();
            for form in body.iter() {
                expansion = trace_eval(form.clone(), ctx)?;
            }
        
            // Restore original environment
            ctx.env = old_env;
        
            // Evaluate the macro expansion in the original context
            trace_eval(expansion, ctx)
        }
        Value::FuncUserDefined { params, body, env } => {
            if params.len() != args.len() {
                return Err(LispError::ArityMismatch {
                    expected: params.len(),
                    got: args.len(),
                    form: "fn".into(),
                    pos: None,
                });
            }
            let local_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));
            {
                let mut env = local_env.write();
            
            

                for (param, val) in params.iter().zip(args) {
                    env.set(param, val);
                }
            }

            let old_env = ctx.env.clone();
            ctx.env = local_env;

            let mut result = nil();
            for form in body {
                result = trace_eval(form.clone(), ctx)?;
            }
            // TODO: Drop guard for env?
            ctx.env = old_env;
            Ok(result)
        }
        _ => Err(LispError::EvalError {
            message: "Not a function".into(),
            pos: get_pos(&func),
        }),
    }
}

fn eval_if(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() < 2 {
        return Err(LispError::EvalError {
            message: "if expects at least 2 arguments".into(),
            pos: None,
        });
    }
    let condition = trace_eval(args[0].clone(), ctx)?;
    let is_truthy = !matches!(condition.read().value, Value::Bool(false) | Value::Nil);
    if is_truthy {
        trace_eval(args[1].clone(), ctx)
    } else if args.len() > 2 {
        trace_eval(args[2].clone(), ctx)
    } else {
        Ok(nil())
    }
}

fn eval_def(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() != 2 {
        return Err(LispError::EvalError {
            message: "def expects exactly 2 arguments".into(),
            pos: None,
        });
    }
    let name = match &args[0].read().value {
        Value::Symbol(s) => s.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "def first argument must be a symbol".into(),
                pos: None,
            })
        }
    };
    let value = trace_eval(args[1].clone(), ctx)?;
    ctx.set(&name, value.clone());
    Ok(value)
}

fn eval_fn(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() < 2 {
        return Err(LispError::EvalError {
            message: "fn expects a parameter list and at least one body form".into(),
            pos: None,
        });
    }
    let params = match &args[0].read().value {
        Value::Vector(vs) => vs
            .iter()
            .filter_map(|v| {
                if let Value::Symbol(s) = &v.read().value {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect(),
        Value::List(xs) if !xs.is_empty() => {
            if let Value::Symbol(head) = &xs[0].read().value {
                if head == "vector" {
                    xs.iter()
                        .skip(1)
                        .filter_map(|v| {
                            if let Value::Symbol(s) = &v.read().value {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    return Err(LispError::EvalError {
                        message: "fn expects a vector of symbols as parameters".into(),
                        pos: None,
                    });
                }
            } else {
                return Err(LispError::EvalError {
                    message: "fn expects a vector of symbols as parameters".into(),
                    pos: None,
                });
            }
        }
        _ => {
            return Err(LispError::EvalError {
                message: "fn expects a vector of symbols as parameters".into(),
                pos: None,
            });
        }
    };

    Ok(BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::FuncUserDefined {
            params,
            body: args[1..].to_vec(),
            env: Arc::clone(&ctx.env),
        },
        pos: None,
    }))))
}
fn eval_do(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let mut result = nil();
    for form in args {
        result = trace_eval(form.clone(), ctx)?;
    }
    Ok(result)
}

fn eval_let(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() < 2 {
        return Err(LispError::EvalError {
            message: "let expects a binding list and at least one body form".into(),
            pos: None,
        });
    }
    let bindings_borrow = &args[0].read().value;
    let bindings = match bindings_borrow {
        Value::Vector(vs) => vs,
        _ => {
            return Err(LispError::EvalError {
                message: "let expects a vector of bindings".into(),
                pos: None,
            })
        }
    };
    if bindings.len() % 2 != 0 {
        return Err(LispError::EvalError {
            message: "let binding vector must have an even number of elements".into(),
            pos: None,
        });
    }
    let local_env = Arc::new(RwLock::new(Env::with_parent(ctx.env.clone())));
    for pair in bindings.chunks(2) {
        let key = match &pair[0].read().value {
            Value::Symbol(s) => s.clone(),
            _ => {
                return Err(LispError::EvalError {
                    message: "let binding keys must be symbols".into(),
                    pos: None,
                })
            }
        };
        let val = trace_eval(pair[1].clone(), ctx)?;
        local_env.write().set(&key, val);
    }
    let mut result = nil();
    for form in &args[1..] {
        result = trace_eval(form.clone(), ctx)?;
    }
    Ok(result)
}

fn eval_and(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let mut last = bool_val(true);
    for arg in args {
        last = trace_eval(arg.clone(), ctx)?;
        if matches!(last.read().value, Value::Bool(false) | Value::Nil) {
            break;
        }
    }
    Ok(last)
}

fn eval_or(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    for arg in args {
        let val = trace_eval(arg.clone(), ctx)?;
        if !matches!(val.read().value, Value::Bool(false) | Value::Nil) {
            return Ok(val);
        }
    }
    Ok(nil())
}

fn eval_try(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let res = eval(args[0].clone(), ctx);
    match res {
        Ok(val) => Ok(val),
        Err(_) if args.len() > 1 => trace_eval(args[1].clone(), ctx),
        Err(err) => Err(err),
    }
}

fn eval_apply(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() != 2 {
        return Err(LispError::ArityMismatch {
            expected: 2,
            got: args.len(),
            form: "apply".into(),
            pos: None,
        });
    }
    let func = trace_eval(args[0].clone(), ctx)?;
    let evaluated_list = trace_eval(args[1].clone(), ctx)?;
    let list_items = match &evaluated_list.read().value {
        Value::List(xs) => xs.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "apply expects a list as second argument".into(),
                pos: None,
            })
        }
    };
    eval_func(func, list_items, ctx)
}


fn load_native_library(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use libloading::Library;

    if args.len() != 1 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "load-native".into(),
            pos: None,
        });
    }

    let libname = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "load-native expects a string".into(),
                pos: None,
            })
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
    if ctx.module_registry.get_module(&libname).is_some() {
        println!("Reloading native library '{}'", libname);
        ctx.module_registry.remove_module(&libname);
        ctx.module_registry.remove_native_library(&lib_path);
    }

    // Load the library
    let lib = unsafe { Library::new(&filename) }.map_err(|e| LispError::EvalError {
        message: format!("Failed to load native lib '{}': {}", filename, e),
        pos: None,
    })?;

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
            let _arc_mod = ctx.module_registry.register_module(module);
            
            exports_set
        },
        Err(_) => {
            // Fall back to old registration function (no export tracking)
            let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
                lib.get(b"blink_register")
                    .map_err(|e| LispError::EvalError {
                        message: format!("Failed to find blink_register or blink_register_with_exports in '{}': {}", filename, e),
                        pos: None,
                    })?
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
            let _arc_mod = ctx.module_registry.register_module(module);
            
            exports
        }
    };

    // Store the library to prevent it from being unloaded
    ctx.module_registry.store_native_library(lib_path, lib);

    println!("Loaded native library '{}' with {} exports: {:?}", 
             libname, exports.len(), exports);

    Ok(nil())
}

fn load_native_code(
    args: &[BlinkValue],
    ctx: &mut EvalContext,
) -> Result<BlinkValue, LispError> {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use libloading::Library;
    use std::collections::HashSet;

    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    
    if args.len() < 1 || args.len() > 2 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "compile-plugin".into(),
            pos,
        });
    }

    let plugin_name = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "compile-plugin expects a string as first argument".into(),
                pos,
            });
        }
    };

    let mut plugin_path = format!("plugins/{}", plugin_name);
    let mut auto_import = false;

    // Parse options if provided
    if args.len() == 2 {
        let options_val = trace_eval(args[1].clone(), ctx)?;
        let options_borrowed = options_val.read();
        if let Value::Map(opt_map) = &options_borrowed.value {
            if let Some(path_val) = opt_map.get(":path").or_else(|| opt_map.get("path")) {
                if let Value::Str(path) = &path_val.read().value {
                    plugin_path = path.clone();
                }
            }
            if let Some(import_val) = opt_map.get(":import").or_else(|| opt_map.get("import")) {
                if matches!(&import_val.read().value, Value::Bool(true)) {
                    auto_import = true;
                }
            }
        } else {
            return Err(LispError::EvalError {
                message: "Second argument to compile-plugin must be a map".into(),
                pos,
            });
        }
    }

    // Check if plugin directory exists
    if !Path::new(&plugin_path).exists() {
        return Err(LispError::EvalError {
            message: format!("Plugin path '{}' does not exist", plugin_path),
            pos,
        });
    }

    // Compile the plugin
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&plugin_path)
        .status()
        .map_err(|e| LispError::EvalError {
            message: format!("Failed to build plugin: {}", e),
            pos: None,
        })?;

    if !status.success() {
        return Err(LispError::EvalError {
            message: "Plugin build failed".into(),
            pos: None,
        });
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
    fs::copy(&source, &dest).map_err(|e| LispError::EvalError {
        message: format!("Failed to copy compiled plugin: {}", e),
        pos: None,
    })?;

    // Load the library
    let lib = unsafe { Library::new(&dest) }.map_err(|e| LispError::EvalError {
        message: format!("Failed to load compiled plugin: {}", e),
        pos: None,
    })?;

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
            let _arc_mod = ctx.module_registry.register_module(module);
            
            exports_set
        },
        Err(_) => {
            // Fall back to old registration function (no export tracking)
            let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
                lib.get(b"blink_register")
                    .map_err(|e| LispError::EvalError {
                        message: format!("Failed to find blink_register or blink_register_with_exports: {}", e),
                        pos: None,
                    })?
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
            let _arc_mod = ctx.module_registry.register_module(module);
            
            exports
        }
    };

    // Store the library to prevent it from being unloaded
    ctx.module_registry.store_native_library(PathBuf::from(&dest), lib);

    println!("Compiled and loaded plugin '{}' with {} exports: {:?}", 
             plugin_name, exports.len(), exports);

    Ok(nil())
}

// Helper function to extract symbol names from an environment
fn extract_env_symbols(env: &Arc<RwLock<Env>>) -> HashSet<String> {
    env.read().vars.keys().cloned().collect()
}

fn eval_def_reader_macro(
    args: &[BlinkValue],
    ctx: &mut EvalContext,
) -> Result<BlinkValue, LispError> {
    if args.len() != 2 {
        return Err(LispError::ArityMismatch {
            expected: 2,
            got: args.len(),
            form: "def-reader-macro".into(),
            pos: None,
        });
    }

    let char_val = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "First argument to def-reader-macro must be a string".into(),
                pos: None,
            });
        }
    };

    let ch = char_val;

    let func = crate::eval::trace_eval(args[1].clone(), ctx)?;

    ctx.reader_macros
        .write()
        .reader_macros
        .insert(ch, func.clone());
    Ok(func)
}

/// Module declaration and context management
fn eval_mod(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.is_empty() {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: 0,
            form: "mod".into(),
            pos: None,
        });
    }
    
    // Parse flags
    let (flags, name_index) = parse_flags(args);
    if flags.len() < 1 {
        return Err(LispError::EvalError {
            message: "At least one flag is required.".into(),
            pos: None,
        });
    }
    
    // Extract module name
    if name_index >= args.len() {
        return Err(LispError::EvalError {
            message: "Missing module name after flags".into(),
            pos: None,
        });
    }
    
    let name = extract_name(&args[name_index])?;
    
    
    let (options, body_start) = parse_mod_options(&args[name_index + 1..])?;
    

    let should_declare = flags.contains("declare") || flags.is_empty();
    let should_enter = flags.contains("enter");
    
    let mut module = ctx.module_registry.get_module(&name);
    

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
        let module_arc = ctx.module_registry.register_module(new_module);
        module = Some(module_arc);        
    }
    if should_declare && options.contains_key("exports") {
        let mut module_guard = module.as_ref().unwrap().write();
        update_module_exports(&mut module_guard, &options["exports"])?;
    }

    if should_enter {
        if let Some(module) = module {
            ctx.current_module = Some(module.read().name.clone());
            ctx.env = module.read().env.clone();
        } else {
            return Err(LispError::ModuleError {
                message: "Module not found".into(),
                pos: None,
            });
        }
    }
    Ok(nil())
    
}

/// Helper to parse keyword flags at the beginning of args
fn parse_flags(args: &[BlinkValue]) -> (HashSet<String>, usize) {
    let mut flags = HashSet::new();
    let mut name_index = 0;
    
    for (i, arg) in args.iter().enumerate() {
        if let Value::Keyword(kw) = &arg.read().value {
            flags.insert(kw.clone());
            name_index = i + 1;
        } else {
            break;
        }
    }
    
    (flags, name_index)
}

/// Helper to check if an option is special (not for metadata)
fn is_special_option(key: &str) -> bool {
    matches!(key, "exports" | "parent")
}

/// Helper to update module exports
fn update_module_exports(module: &mut Module, exports_val: &BlinkValue) -> Result<(), LispError> {
    match &exports_val.read().value {
        Value::Keyword(kw) if kw == "all" => {
            let all_keys = module.env.read().vars.keys().cloned().collect();
            module.exports = all_keys;
            return Ok(());
        },
        Value::List(items) | Value::Vector(items) => {
            for item in items {
                if let Value::Symbol(name) = &item.read().value {
                    module.exports.insert(name.clone());
                } else {
                    return Err(LispError::EvalError {
                        message: "Exports must be a list of symbols".into(),
                        pos: None,
                    });
                }
            }
        },
        _ => return Err(LispError::EvalError {
            message: "Exports must be a list or vector".into(),
            pos: None,
        }),
    }
    
    Ok(())
}

fn eval_load(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let (source_type, source_value, options, _) = parse_load_args(args)?;
    
    match source_type.as_str() {
        "file" => {
            let file_path = PathBuf::from(&source_value);
            load_blink_file(file_path, ctx)?;
            Ok(nil())
        },
        
        "native" => {
            let path = PathBuf::from(&source_value);
            
            // Check if it's a Cargo project (has Cargo.toml) or a single library file
            if path.is_dir() {
                let cargo_toml = path.join("Cargo.toml");
                if cargo_toml.exists() {
                    // It's a Cargo project - compile it
                    load_native_code(&args, ctx)
                } else {
                    return Err(LispError::EvalError {
                        message: format!("Directory '{}' is not a valid Cargo project (no Cargo.toml found)", source_value),
                        pos: None,
                    });
                }
            } else if path.extension().map_or(false, |ext| {
                ext == "so" || ext == "dll" || ext == "dylib"
            }) {
                // It's a pre-built library file
                load_native_library(&args, ctx)
            } else {
                return Err(LispError::EvalError {
                    message: format!("'{}' is neither a Cargo project directory nor a native library file", source_value),
                    pos: None,
                });
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
        
        _ => Err(LispError::EvalError {
            message: format!("Unknown load source type: :{}", source_type),
            pos: None,
        }),
    }
}

/// Check if a source is an external module
fn is_external_source(source: &Option<ModuleSource>) -> bool {
    match source {
        Some(ModuleSource::Repl) => false,
        Some(ModuleSource::NativeDylib(_)) => true,
        Some(ModuleSource::Wasm(_)) => true,
        Some(ModuleSource::BlinkFile(_)) => false,
        Some(ModuleSource::BlinkPackage(_)) => true,
        Some(ModuleSource::Cargo(_)) => true,
        Some(ModuleSource::Git { .. }) => true,
        Some(ModuleSource::Url(_)) => true,
        Some(ModuleSource::BlinkDll(_)) => true,
        None => false,
    }
}

/// Get a string option
fn get_string_option(options: &HashMap<String, BlinkValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|val| {
        match &val.read().value {
            Value::Str(s) => Some(s.clone()),
            Value::Symbol(s) => Some(s.clone()),
            _ => None,
        }
    })
}


/// Extract a name from a BlinkValue
fn extract_name(value: &BlinkValue) -> Result<String, LispError> {
    match &value.read().value {
        Value::Symbol(s) => Ok(s.clone()),
        Value::Str(s) => Ok(s.clone()),
        _ => Err(LispError::EvalError {
            message: "Expected a symbol or string for name".into(),
            pos: None,
        }),
    }
}

/// Load and evaluate a Blink source file
fn load_blink_file(file_path: PathBuf, ctx: &mut EvalContext) -> Result<(), LispError> {
    // Don't evaluate if already loaded
    if ctx.module_registry.is_file_evaluated(&file_path) {
        return Ok(());
    }

    let contents = fs::read_to_string(&file_path).map_err(|e| LispError::EvalError {
        message: format!("Failed to read file: {}", e),
        pos: None,
    })?;
    let mut reader_ctx = ctx.reader_macros.clone();
    // Parse the file
    let forms = crate::parser::parse_all(&contents, &mut reader_ctx)?;
    
    // Set current file context
    let old_file = ctx.current_file.clone();
    let file_name = file_path.file_name()
        .ok_or(LispError::EvalError { message: "File name missing.".into(), pos: None })? 
        .to_str().ok_or(LispError::EvalError { message: "File name missing.".into(), pos: None })?.to_string();
    ctx.current_file = Some(file_name);
    
    // Evaluate all forms in the file
    for form in forms {
        eval(form, ctx)?;
    }
    
    // Restore previous file context
    ctx.current_file = old_file;
    
    // Mark file as evaluated
    ctx.module_registry.mark_file_evaluated(file_path);
    
    Ok(())
}

fn parse_import_args(args: &[BlinkValue]) -> Result<(ImportType, Option<HashMap<String, BlinkValue>>), LispError> {
    if args.is_empty() {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: 0,
            form: "imp".into(),
            pos: None,
        });
    }

    let first_arg = &args[0].read().value;
    
    match first_arg {
        // File import: (imp "module-name")
        Value::Str(module_name) => {
            
            Ok((ImportType::File(module_name.clone()), None))
        },
        
        // Symbol import: (imp [sym1 sym2] :from module)
        Value::List(list) if !list.is_empty() => {
            // Check if it's a vector form (list starting with 'vector)
            if let Value::Symbol(first) = &list[0].read().value {
                if first == "vector" {
                    let (import_type, options) = parse_symbol_import(&list[1..], &args[1..])?;
                    return Ok((import_type, Some(options)))
                }
            }
            // Otherwise, treat as regular list of symbols
            let (import_type, options) = parse_symbol_import(list, &args[1..])?;
            Ok((import_type, Some(options)))
        },
        
        // Vector import: (imp [sym1 sym2] :from module)  
        Value::Vector(symbols) => {
            let (import_type, options) = parse_symbol_import(symbols, &args[1..])?;
            Ok((import_type, Some(options)))
        },
        
        _ => Err(LispError::EvalError {
            message: "imp expects a string (file) or vector (symbols)".into(),
            pos: args[0].read().pos.clone(),
        }),
    }
}

fn parse_symbol_import(
    symbol_list: &[BlinkValue], 
    remaining_args: &[BlinkValue]
) -> Result<(ImportType, HashMap<String, BlinkValue>), LispError> {
    
    // Parse symbols and any aliases
    let mut symbols = Vec::new();
    let mut aliases = HashMap::new();
    
    let mut i = 0;
    while i < symbol_list.len() {
        match &symbol_list[i].read().value {
            Value::Symbol(name) if name == "*" => {
                symbols.push("*".to_string()); // Import all
                i += 1;
            },
            Value::Symbol(name) => {
                // Check for alias: [func1 :as f1]
                if i + 2 < symbol_list.len() {
                    if let Value::Keyword(kw) = &symbol_list[i + 1].read().value {
                        if kw == "as" {
                            if let Value::Symbol(alias) = &symbol_list[i + 2].read().value {
                                symbols.push(name.clone());
                                aliases.insert(name.clone(), alias.clone());
                                i += 3;
                                continue;
                            }
                        }
                    }
                }
                // Regular symbol
                symbols.push(name.clone());
                i += 1;
            },
            _ => return Err(LispError::EvalError {
                message: "Symbol list must contain symbols".into(),
                pos: symbol_list[i].read().pos.clone(),
            }),
        }
    }
    
    // Look for :from module-name
    let mut module_name = None;
    let mut options = HashMap::new();
    
    let mut j = 0;
    while j < remaining_args.len() {
        if let Value::Keyword(kw) = &remaining_args[j].read().value {
            match kw.as_str() {
                "from" => {
                    if j + 1 >= remaining_args.len() {
                        return Err(LispError::EvalError {
                            message: ":from requires a module name".into(),
                            pos: remaining_args[j].read().pos.clone(),
                        });
                    }
                    if let Value::Symbol(module) = &remaining_args[j + 1].read().value {
                        module_name = Some(module.clone());
                        j += 2;
                    } else {
                        return Err(LispError::EvalError {
                            message: ":from expects a module name".into(),
                            pos: remaining_args[j + 1].read().pos.clone(),
                        });
                    }
                },
                "reload" => {
                    options.insert("reload".to_string(), bool_val(true));
                    j += 1;
                },
                other => {
                    return Err(LispError::EvalError {
                        message: format!("Unknown import option: {}", other),
                        pos: remaining_args[j].read().pos.clone(),
                    });
                }
            }
        } else {
            return Err(LispError::EvalError {
                message: "Expected keyword after symbol list".into(),
                pos: remaining_args[j].read().pos.clone(),
            });
        }
    }
    
    let module = module_name.ok_or_else(|| LispError::EvalError {
        message: "Symbol import requires :from module-name".into(),
        pos: None,
    })?;
    
    Ok((ImportType::Symbols { symbols, module, aliases }, options))
}

fn parse_mod_options(args: &[BlinkValue]) -> Result<(HashMap<String, BlinkValue>, usize), LispError> {
    let mut options = HashMap::new();
    let mut i = 0;
    
    while i < args.len() {
        match &args[i].read().value {
            Value::Keyword(key) => {
                match key.as_str() {
                    "exports" => {
                        if i + 1 >= args.len() {
                            return Err(LispError::EvalError {
                                message: ":exports requires a list".into(),
                                pos: args[i].read().pos.clone(),
                            });
                        }
                        options.insert("exports".to_string(), args[i + 1].clone());
                        i += 2;
                    },
                    other => {
                        return Err(LispError::EvalError {
                            message: format!("Unknown option: {}", other),
                            pos: args[i].read().pos.clone(),
                        });
                    }
                }
            },
            _ => {
                // Not a keyword, so we've reached the body
                break;
            }
        }
    }
    
    Ok((options, i))
}

fn find_module_file(module_name: &str, ctx: &mut EvalContext) -> Result<PathBuf, LispError> {
    // 1. Check if module is already registered (we know which file it came from)
    if let Some(module) = ctx.module_registry.get_module(module_name) {
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
    
    Err(LispError::EvalError {
        message: format!("Module '{}' not found. Tried:\n  lib/{}.bl\n  lib/{}.bl\n  And parent directories", 
                        module_name, module_name, parts.join("-")),
        pos: None,
    })
}

// Helper function to check if a file contains a specific module declaration
fn file_contains_module(file_path: &PathBuf, module_name: &str) -> Result<bool, LispError> {
    let content = std::fs::read_to_string(file_path).map_err(|e| LispError::EvalError {
        message: format!("Failed to read file {:?}: {}", file_path, e),
        pos: None,
    })?;
    
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

fn import_symbols_into_env(
    symbols: &[String], 
    module_name: &str, 
    aliases: &HashMap<String, String>,
    ctx: &mut EvalContext
) -> Result<(), LispError> {
    let module = ctx.module_registry.get_module(module_name)
        .ok_or_else(|| LispError::EvalError {
            message: format!("Module '{}' not found", module_name),
            pos: None,
        })?;
    
    let module_read = module.read();
    
    // Handle import all (*)
    if symbols.len() == 1 && symbols[0] == ":all" {
        for export_name in &module_read.exports {
            let local_name = aliases.get(export_name).unwrap_or(export_name);
            
            // Create a live reference instead of copying the value
            let reference = create_module_reference(module_name, export_name);
            ctx.env.write().vars.insert(local_name.clone(), reference);
        }
        return Ok(());
    }
    
    // Import specific symbols
    for symbol_name in symbols {
        // Check if symbol is exported
        if !module_read.exports.contains(symbol_name) {
            return Err(LispError::EvalError {
                message: format!("Symbol '{}' is not exported by module '{}'", symbol_name, module_name),
                pos: None,
            });
        }
        
        let local_name = aliases.get(symbol_name).unwrap_or(symbol_name);
        
        // Create a live reference to the module symbol
        let reference = create_module_reference(module_name, symbol_name);
        ctx.env.write().vars.insert(local_name.clone(), reference);
    }
    
    Ok(())
}

// Helper to create live references to module symbols
fn create_module_reference(module_name: &str, symbol_name: &str) -> BlinkValue {
    BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::ModuleReference {
            module: module_name.to_string(),
            symbol: symbol_name.to_string(),
        },
        pos: None,
    })))
}

// Generic parse options function
fn parse_options(args: &[BlinkValue]) -> Result<(HashMap<String, BlinkValue>, usize), LispError> {
    let mut options = HashMap::new();
    let mut i = 0;

    while i < args.len() {
        match &args[i].read().value {
            // make sure that the next argument is a value
            Value::Keyword(key) => {
                if i + 1 >= args.len() {
                    return Err(LispError::EvalError {
                        message: format!("Option {} requires a value", key),
                        pos: args[i].read().pos.clone(),
                    });
                }
                options.insert(key.clone(), args[i + 1].clone());
                i += 2;
            },
            _ => {
                break;
            }
        }
    }

    Ok((options, i))
}

fn parse_load_args(args: &[BlinkValue]) -> Result<(String, String, HashMap<String, BlinkValue>, usize), LispError> {
    if args.is_empty() {
        return Err(LispError::ArityMismatch {
            expected: 2,
            got: 0,
            form: "load".into(),
            pos: None,
        });
    }
    
    // First argument should be a keyword indicating source type
    let source_type = match &args[0].read().value {
        Value::Keyword(kw) => kw.clone(),
        _ => return Err(LispError::EvalError {
            message: "load expects a keyword as first argument (:file, :native, :cargo, :dylib, :url, :git)".into(),
            pos: args[0].read().pos.clone(),
        }),
    };
    
    // Second argument should be the source value
    if args.len() < 2 {
        return Err(LispError::EvalError {
            message: format!("load {} requires a source argument", source_type),
            pos: None,
        });
    }
    
    let source_value = match &args[1].read().value {
        Value::Str(s) => s.clone(),
        _ => return Err(LispError::EvalError {
            message: "load source must be a string".into(),
            pos: args[1].read().pos.clone(),
        }),
    };
    
    // Parse any additional options
    let (options, s) = parse_options(&args[2..])?;
    
    Ok((source_type, source_value, options, s))
}

fn eval_imp(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let (import_type, _options) = parse_import_args(args)?;
    
    match import_type {
        ImportType::File(file_name) => {
            println!("Importing file: {}", file_name);
            // File import: (imp "module-name")
            let file_path = PathBuf::from(format!("lib/{}.blink", file_name));
            
            // Load the file if needed
            load_blink_file(file_path, ctx)?;
            
            // Make the file's modules available for qualified access
            // (they're already registered by eval_mod during file evaluation)
            Ok(nil())
        },
        
        ImportType::Symbols { symbols, module, aliases } => {
            // Check if module already exists
            let module_exists = ctx.module_registry.get_module(&module).is_some();
            
            if !module_exists {
                
                // Find which file contains the module
                let file_path = find_module_file(&module, ctx)?;
                
                // Load the file if needed (this registers all modules in the file)
                load_blink_file(file_path, ctx)?;
                
                // Verify the module is now available
                if ctx.module_registry.get_module(&module).is_none() {
                    return Err(LispError::EvalError {
                        message: format!("Module '{}' was not found in the loaded file", module),
                        pos: None,
                    });
                }
            }
            
            // Import the symbols into current environment
            import_symbols_into_env(&symbols, &module, &aliases, ctx)?;
            Ok(nil())
        }
    }
}



// Add these functions to eval.rs
fn eval_quasiquote(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    require_arity(args, 1, "quasiquote")?;
    expand_quasiquote(args[0].clone(), ctx)
}

fn expand_quasiquote(expr: BlinkValue, ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    match &expr.read().value {
        Value::List(items) if !items.is_empty() => {
            let first = &items[0];
            match &first.read().value {
                Value::Symbol(s) if s == "unquote" => {
                    if items.len() != 2 {
                        return Err(LispError::EvalError {
                            message: "unquote expects exactly one argument".into(),
                            pos: None,
                        });
                    }
                    trace_eval(items[1].clone(), ctx)
                }
                Value::Symbol(s) if s == "unquote-splicing" => {
                    return Err(LispError::EvalError {
                        message: "unquote-splicing not valid here".into(),
                        pos: None,
                    });
                }
                _ => {
                    let mut expanded_items = Vec::new();
                    for item in items {
                        if let Value::List(inner_items) = &item.read().value {
                            if !inner_items.is_empty() {
                                if let Value::Symbol(s) = &inner_items[0].read().value {
                                    if s == "unquote-splicing" {
                                        if inner_items.len() != 2 {
                                            return Err(LispError::EvalError {
                                                message: "unquote-splicing expects exactly one argument".into(),
                                                pos: None,
                                            });
                                        }
                                        let spliced = trace_eval(inner_items[1].clone(), ctx)?;
                                        if let Value::List(splice_items) = &spliced.read().value {
                                            expanded_items.extend(splice_items.clone());
                                        } else {
                                            return Err(LispError::EvalError {
                                                message: "unquote-splicing expects a list".into(),
                                                pos: None,
                                            });
                                        }
                                        continue;
                                    }
                                }
                            }
                        }
                        expanded_items.push(expand_quasiquote(item.clone(), ctx)?);
                    }
                    Ok(list_val(expanded_items))
                }
            }
        }
        Value::Vector(items) => {
            let mut expanded_items = Vec::new();
            for item in items {
                expanded_items.push(expand_quasiquote(item.clone(), ctx)?);
            }
            Ok(BlinkValue(Arc::new(RwLock::new(LispNode {
                value: Value::Vector(expanded_items),
                pos: expr.read().pos.clone(),
            }))))
        }
        _ => Ok(expr.clone()),
    }
}


fn eval_macro(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() < 2 {
        return Err(LispError::EvalError {
            message: "macro expects at least 2 arguments: params and body".into(),
            pos: None,
        });
    }

    let (params, is_variadic) = match &args[0].read().value {
        Value::Vector(vs) => {
            let mut params = Vec::new();
            let mut is_variadic = false;
            let mut i = 0;
            
            while i < vs.len() {
                if let Value::Symbol(s) = &vs[i].read().value {
                    if s == "&" {
                        // Next symbol is the rest parameter
                        if i + 1 < vs.len() {
                            if let Value::Symbol(rest_param) = &vs[i + 1].read().value {
                                params.push(rest_param.clone());
                                is_variadic = true;
                                break;
                            }
                        }
                        return Err(LispError::EvalError {
                            message: "& must be followed by a parameter name".into(),
                            pos: None,
                        });
                    } else {
                        params.push(s.clone());
                    }
                }
                i += 1;
            }
            (params, is_variadic)
        },
        Value::List(xs) if !xs.is_empty() => {
            if let Value::Symbol(head) = &xs[0].read().value {
                if head == "vector" {
                    let mut params = Vec::new();
                    let mut is_variadic = false;
                    let mut i = 1; // skip "vector"
                    
                    while i < xs.len() {
                        if let Value::Symbol(s) = &xs[i].read().value {
                            if s == "&" {
                                if i + 1 < xs.len() {
                                    if let Value::Symbol(rest_param) = &xs[i + 1].read().value {
                                        params.push(rest_param.clone());
                                        is_variadic = true;
                                        break;
                                    }
                                }
                                return Err(LispError::EvalError {
                                    message: "& must be followed by a parameter name".into(),
                                    pos: None,
                                });
                            } else {
                                params.push(s.clone());
                            }
                        }
                        i += 1;
                    }
                    (params, is_variadic)
                } else {
                    return Err(LispError::EvalError {
                        message: "macro expects a vector of symbols as parameters".into(),
                        pos: None,
                    });
                }
            } else {
                return Err(LispError::EvalError {
                    message: "macro expects a vector of symbols as parameters".into(),
                    pos: None,
                });
            }
        }
        _ => {
            return Err(LispError::EvalError {
                message: "macro expects a vector of symbols as parameters".into(),
                pos: None,
            });
        }
    };

    Ok(BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Macro {
            params,
            body: args[1..].to_vec(),
            env: Arc::clone(&ctx.env),
            is_variadic,
        },
        pos: None,
    }))))
}