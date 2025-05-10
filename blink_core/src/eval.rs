use crate::env::Env;
use crate::error::LispError;
use crate::parser::ReaderContext;
use crate::telemetry::TelemetryEvent;
use crate::value::{bool_val, nil, BlinkValue, LispNode, SourceRange, Value};
use libloading::Library;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

pub struct EvalContext {
    pub env: Arc<RwLock<Env>>,
    pub current_module: Option<String>,
    pub plugins: HashMap<String, Arc<Library>>,
    pub telemetry_sink: Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>,
    pub tracing_enabled: bool,
    pub reader_macros: Arc<RwLock<ReaderContext>>,
}

impl EvalContext {
    pub fn new(parent: Arc<RwLock<Env>>) -> Self {
        EvalContext {
            env: Arc::new(RwLock::new(Env::with_parent(parent))),
            current_module: None,
            plugins: HashMap::new(),
            telemetry_sink: None,
            tracing_enabled: false,
            reader_macros: Arc::new(RwLock::new(ReaderContext::new())),
        }
    }

    pub fn get(&self, key: &str) -> Option<BlinkValue> {
        self.env.read().get(key)
    }

    pub fn set(&self, key: &str, val: BlinkValue) {
        self.env.write().set(key, val)
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
                    "quo" => eval_quote(list),
                    "apply" => eval_apply(&list[1..], ctx),
                    "if" => eval_if(&list[1..], ctx),
                    "def" => eval_def(&list[1..], ctx),
                    "fn" => eval_fn(&list[1..], ctx),
                    "do" => eval_do(&list[1..], ctx),
                    "let" => eval_let(&list[1..], ctx),
                    "and" => eval_and(&list[1..], ctx),
                    "or" => eval_or(&list[1..], ctx),
                    "try" => eval_try(&list[1..], ctx),
                    "imp" => eval_import(&list[1..], ctx),
                    "nimp" => eval_native_import(&list[1..], ctx),
                    "ncom" => eval_compile_plugin(&list[1..], ctx),
                    "rmac" => eval_def_reader_macro(&list[1..], ctx),

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
            let mut env = local_env.write();
            for (param, val) in params.iter().zip(args) {
                env.set(param, val);
            }
            let mut result = nil();
            for form in body {
                result = trace_eval(form.clone(), ctx)?;
            }
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

fn eval_import(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    use std::fs;

    if args.len() != 1 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "import".into(),
            pos: None,
        });
    }

    let path = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "import expects a string as module path".into(),
                pos: None,
            })
        }
    };

    let code =
        fs::read_to_string(format!("lib/{}.blink", path)).map_err(|_| LispError::EvalError {
            message: format!("Failed to read module: {}", path),
            pos: None,
        })?;

    let forms = crate::parser::parse_all(&code)?;
    let old_module = ctx.current_module.clone();
    ctx.current_module = Some(path);

    for form in forms {
        trace_eval(form, ctx)?;
    }

    ctx.current_module = old_module;
    Ok(nil())
}

fn eval_native_import(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    if args.len() != 1 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "native-import".into(),
            pos: None,
        });
    }

    let libname = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return Err(LispError::EvalError {
                message: "native-import expects a string".into(),
                pos: None,
            })
        }
    };

    let filename = format!("native/lib{}.so", libname);

    let lib = unsafe { Library::new(&filename) }.map_err(|e| LispError::EvalError {
        message: format!("Failed to load native lib: {}", e),
        pos: None,
    })?;

    let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
        lib.get(b"blink_register")
            .map_err(|e| LispError::EvalError {
                message: format!("Failed to find blink_register: {}", e),
                pos: None,
            })?
    };

    unsafe { register(&mut *ctx.env.write()) };
    ctx.plugins.insert(libname, Arc::new(lib));

    Ok(nil())
}

fn eval_compile_plugin(
    args: &[BlinkValue],
    ctx: &mut EvalContext,
) -> Result<BlinkValue, LispError> {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

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
    let mut _auto_import = false;

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
                    _auto_import = true;
                }
            }
        } else {
            return Err(LispError::EvalError {
                message: "Second argument to compile-plugin must be a map".into(),
                pos,
            });
        }
    }

    if !Path::new(&plugin_path).exists() {
        return Err(LispError::EvalError {
            message: format!("Plugin path '{}' does not exist", plugin_path),
            pos,
        });
    }

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

    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let source = format!("{}/target/release/lib{}.{}", plugin_path, plugin_name, ext);
    let dest = format!("native/lib{}.so", plugin_name);

    fs::create_dir_all("native").ok();
    fs::copy(&source, &dest).map_err(|e| LispError::EvalError {
        message: format!("Failed to copy compiled plugin: {}", e),
        pos: None,
    })?;

    // Load and register immediately
    let lib = unsafe { Library::new(&dest) }.map_err(|e| LispError::EvalError {
        message: format!("Failed to load compiled plugin: {}", e),
        pos: None,
    })?;

    let register: libloading::Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
        lib.get(b"blink_register")
            .map_err(|e| LispError::EvalError {
                message: format!("Failed to find blink_register: {}", e),
                pos: None,
            })?
    };

    unsafe { register(&mut *ctx.env.write()) };

    ctx.plugins.insert(plugin_name, Arc::new(lib));

    Ok(nil())
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
