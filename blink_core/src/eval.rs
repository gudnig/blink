use crate::async_context::AsyncContext;
use crate::env::Env;
use crate::error::{BlinkError, LispError};
use crate::future::BlinkFuture;
use crate::goroutine::{GoroutineId, TokioGoroutineScheduler};
use crate::module::{ImportType, Module, ModuleRegistry, ModuleSource};
use crate::parser::{ ReaderContext};
use crate::value::str_val;
use crate::telemetry::TelemetryEvent;
use crate::value::{bool_val, keyword_at, list_val, nil, BlinkValue, LispNode, SourceRange, Value};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct EvalContext {
    pub global_env: Arc<RwLock<Env>>,
    pub env: Arc<RwLock<Env>>,
    pub telemetry_sink: Arc<Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>>,
    pub module_registry: Arc<RwLock<ModuleRegistry>>,
    pub file_to_modules: Arc<HashMap<PathBuf, Vec<String>>>,
    pub goroutine_scheduler: Arc<TokioGoroutineScheduler>,
    pub reader_macros: Arc<RwLock<ReaderContext>>,

    pub current_file: Option<String>,
    pub current_module: Option<String>,
    pub async_ctx: AsyncContext,
    pub tracing_enabled: bool,
}

impl EvalContext {
    pub fn new(parent: Arc<RwLock<Env>>) -> Self {
        EvalContext {
            global_env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            current_module: None,
            telemetry_sink: Arc::new(None),
            module_registry: Arc::new(RwLock::new(ModuleRegistry::new())),
            current_file: None,
            tracing_enabled: false,
            reader_macros: Arc::new(RwLock::new(ReaderContext::new())),
            file_to_modules: Arc::new(HashMap::new()),
            async_ctx: AsyncContext::default(),
            goroutine_scheduler: Arc::new(TokioGoroutineScheduler::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<BlinkValue> {
        let module_registry = self.module_registry.read();
        self.env.read().get_with_registry(key, &module_registry)
    }

    pub fn set(&self, key: &str, val: BlinkValue) {
        self.env.write().set(key, val)
    }

    pub fn with_env(&self, env: Arc<RwLock<Env>>) -> Self {
        EvalContext {
            env,
            async_ctx: self.async_ctx.clone(),
            goroutine_scheduler: self.goroutine_scheduler.clone(),
            reader_macros: self.reader_macros.clone(),
            file_to_modules: self.file_to_modules.clone(),
            module_registry: self.module_registry.clone(),
            telemetry_sink: self.telemetry_sink.clone(),
            global_env: self.global_env.clone(),
            current_file: self.current_file.clone(),
            current_module: self.current_module.clone(),
            tracing_enabled: self.tracing_enabled,
        }
    }

    
}

pub enum EvalResult {
    Value(BlinkValue),

    Suspended {
        future: BlinkFuture,
        resume: Box<dyn FnOnce(BlinkValue, &mut EvalContext) -> EvalResult + Send>,
    },
}

fn ok(val: BlinkValue) -> EvalResult {
    EvalResult::Value(val)
}

macro_rules! try_eval {
    ($expr:expr) => {
        match $expr {
            EvalResult::Value(v) => {
                if v.is_error() {
                    return EvalResult::Value(v);
                }
                v
            }
            suspended @ EvalResult::Suspended { .. } => return suspended,
        }
    };
}


macro_rules! forward_eval {
    ($expr:expr) => {
        match $expr {
            EvalResult::Value(v) => {
                if v.is_error() {
                    return EvalResult::Value(v);
                }
                EvalResult::Value(v)
            }
            suspended => suspended,
        }
    };
}


fn get_pos(expr: &BlinkValue) -> Option<SourceRange> {
    expr.read().pos.clone()
}

pub fn eval(expr: BlinkValue, ctx: &mut EvalContext) -> EvalResult {
    let read_expr = &expr.read();
    
    match &read_expr.value {
        Value::Number(_) | Value::Bool(_) | Value::Str(_) | Value::Keyword(_) | Value::Nil => {
            EvalResult::Value(expr.clone())
        }
        Value::Symbol(sym) => ctx.get(sym)
            .map(|sym| EvalResult::Value(sym))
            .unwrap_or( EvalResult::Value(  BlinkError::undefined_symbol(sym).with_pos(get_pos(&expr)).into_blink_value())),
        Value::List(list) if list.is_empty() => EvalResult::Value(nil()),
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
                    "unquote" => EvalResult::Value(BlinkError::eval("unquote used outside quasiquote").into_blink_value()),
                    "unquote-splicing" => EvalResult::Value(BlinkError::eval("unquote-splicing used outside quasiquote").into_blink_value()),
                    "go" => eval_go(&list[1..], ctx),
                    "deref" => eval_deref(&list[1..], ctx),

                    _ => {
                        eval_function_call_inline(list.clone(), 0, Vec::new(), None, ctx)
                        
                    }
                },
                _ => {
                    eval_function_call_inline(list.clone(), 0, Vec::new(), None, ctx)
                }
            }
        }
        _ => EvalResult::Value(BlinkError::eval("Unknown expression type").with_pos(get_pos(&expr)).into_blink_value()),
    }
}

pub fn trace_eval(expr: BlinkValue, ctx: &mut EvalContext) -> EvalResult {
    if ctx.tracing_enabled {
        let start = Instant::now();
        let val = try_eval!(eval(expr.clone(), ctx));
        if let Some(sink) = &*ctx.telemetry_sink {
            sink(TelemetryEvent {
                form: format!("{:?}", expr.read().value),
                duration_us: start.elapsed().as_micros(),
                result_type: val.read().value.type_tag().to_string(),
                result_size: None,
                source: expr.read().pos.clone(),
            });
        }
        EvalResult::Value(val)
    } else {
        eval(expr, ctx)
    }
}

fn require_arity(args: &[BlinkValue], expected: usize, form_name: &str) -> Result<(), BlinkError> {
    if args.len() != expected {
        return Err(BlinkError::arity(expected, args.len(), form_name));
    }
    Ok(())
}

pub fn eval_quote(args: &Vec<BlinkValue>) -> EvalResult {
    if let Err(e) = require_arity(&args[1..], 1, "quote") {
        return EvalResult::Value(e.into_blink_value());
    }
    EvalResult::Value(args[1].clone())
}

fn eval_function_call_inline(
    list: Vec<BlinkValue>,
    mut index: usize,
    mut evaluated_args: Vec<BlinkValue>,
    func: Option<BlinkValue>,
    ctx: &mut EvalContext,
) -> EvalResult {
    // First evaluate the function if we haven't yet
    if func.is_none() {
        let result = trace_eval(list[0].clone(), ctx);
        match result {
            EvalResult::Value(f) => {
                if f.is_error() {
                    return EvalResult::Value(f);
                }
                return eval_function_call_inline(list, 1, evaluated_args, Some(f), ctx);
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        eval_function_call_inline(list, 1, evaluated_args, Some(v), ctx)
                    }),
                };
            }
        }
    }

    // Now evaluate arguments one by one
    loop {
        if index >= list.len() {
            // All arguments evaluated, call the function
            return eval_func(func.unwrap(), evaluated_args, ctx);
        }

        let result = trace_eval(list[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                evaluated_args.push(val);
                index += 1; 
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        evaluated_args.push(v);
                        eval_function_call_inline(list, index + 1, evaluated_args, func, ctx)
                    }),
                };
            }
        }
    }
}

pub fn eval_func(
    func: BlinkValue,
    args: Vec<BlinkValue>,
    ctx: &mut EvalContext,
) -> EvalResult {
    match &func.read().value {
        Value::NativeFunc(f) => f(args)
            .map_or_else(
                |e| EvalResult::Value(BlinkError::eval(e.to_string()).with_pos(get_pos(&func)).into_blink_value()),
                |v| EvalResult::Value(v))
        ,
        
        Value::Macro { params, body, env, is_variadic } => {
            // Arity check...
            if *is_variadic {
                if args.len() < params.len() - 1 {
                    return EvalResult::Value(BlinkError::arity(params.len() - 1, args.len(), "macro (at least)").into_blink_value());
                }
            } else if params.len() != args.len() {
                return EvalResult::Value(BlinkError::arity(params.len(), args.len(), "macro").into_blink_value());
            }

            // Create macro environment and bind parameters
            let macro_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));
            {
                let mut env_guard = macro_env.write();
                if *is_variadic {
                    for (i, param) in params.iter().take(params.len() - 1).enumerate() {
                        env_guard.set(param, args[i].clone());
                    }
                    let rest_param = &params[params.len() - 1];
                    let rest_args = args.iter().skip(params.len() - 1).cloned().collect();
                    env_guard.set(rest_param, list_val(rest_args));
                } else {
                    for (param, arg) in params.iter().zip(args.iter()) {
                        env_guard.set(param, arg.clone());
                    }
                }
            }

            let old_env = ctx.env.clone();
            let macro_ctx = ctx.with_env(macro_env);
            
            // Evaluate macro body with proper suspension handling
            eval_macro_body_inline(body.clone(), 0, nil(), old_env, macro_ctx)
        }
        
        Value::FuncUserDefined { params, body, env } => {
            if params.len() != args.len() {
                return EvalResult::Value(BlinkError::arity(params.len(), args.len(), "fn").into_blink_value());
            }

            let local_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));
            {
                let mut env_guard = local_env.write();
                for (param, val) in params.iter().zip(args) {
                    env_guard.set(param, val);
                }
            }

            let old_env = ctx.env.clone();
            let local_ctx = ctx.with_env(local_env);
            
            eval_function_body_inline(body.clone(), 0, nil(), old_env, local_ctx)
        }
        
        _ => EvalResult::Value(BlinkError::eval("Not a function").with_pos(get_pos(&func)).into_blink_value()),
    }
}

fn eval_macro_body_inline(
    body: Vec<BlinkValue>, 
    mut index: usize, 
    mut expansion: BlinkValue,
    original_env: Arc<RwLock<Env>>,
    mut ctx: EvalContext
) -> EvalResult {
    loop {
        if index >= body.len() {
            // Switch back to original environment and evaluate the expansion
            ctx.env = original_env;
            return trace_eval(expansion, &mut ctx);
        }

        let result = trace_eval(body[index].clone(), &mut ctx);
        match result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                expansion = val;
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if v.is_error() {
                            return EvalResult::Value(v);
                        }
                        eval_macro_body_inline(body, index + 1, v, original_env, ctx.clone())
                    }),
                };
            }
        }
    }
}

fn eval_function_body_inline(
    body: Vec<BlinkValue>, 
    mut index: usize, 
    mut result: BlinkValue,
    original_env: Arc<RwLock<Env>>,
    mut ctx: EvalContext
) -> EvalResult {
    loop {
        if index >= body.len() {
            // Restore environment and return result
            ctx.env = original_env;
            return EvalResult::Value(result);
        }

        let eval_result = trace_eval(body[index].clone(), &mut ctx);
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
                        eval_function_body_inline(body, index + 1, v, original_env, ctx.clone())
                    }),
                };
            }
        }
    }
}

fn eval_if(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(BlinkError::eval("if expects at least 2 arguments").into_blink_value());
    }
    let condition = try_eval!(trace_eval(args[0].clone(), ctx));
    let is_truthy = !matches!(condition.read().value, Value::Bool(false) | Value::Nil);
    if is_truthy {
        forward_eval!(trace_eval(args[1].clone(), ctx))
    } else if args.len() > 2 {
        forward_eval!(trace_eval(args[2].clone(), ctx))
    } else {
        EvalResult::Value(nil())
    }
}

fn eval_def(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(BlinkError::arity(2, args.len(), "def").into_blink_value());
    }
    let name = match &args[0].read().value {
        Value::Symbol(s) => s.clone(),
        _ => {
            return EvalResult::Value(BlinkError::eval("def first argument must be a symbol").into_blink_value());
        }
    };
    let value = try_eval!(trace_eval(args[1].clone(), ctx));
    ctx.set(&name, value.clone());
    EvalResult::Value(value)
}

fn eval_fn(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(BlinkError::arity(2, args.len(), "fn").into_blink_value());
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
                    return EvalResult::Value(BlinkError::eval("fn expects a vector of symbols as parameters").into_blink_value());
                }
            } else {
                return EvalResult::Value(BlinkError::eval("fn expects a vector of symbols as parameters").into_blink_value());
            }
        }
        _ => {
            return EvalResult::Value(BlinkError::eval("fn expects a vector of symbols as parameters").into_blink_value());
        }
    };

    EvalResult::Value(BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::FuncUserDefined {
            params,
            body: args[1..].to_vec(),
            env: Arc::clone(&ctx.env),
        },
        pos: None,
    }))))
}
fn eval_do(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(nil());
    }
    eval_do_inline(args.to_vec(), 0, nil(), ctx)
}

fn eval_do_inline(
    forms: Vec<BlinkValue>, 
    mut index: usize, 
    mut result: BlinkValue, 
    ctx: &mut EvalContext
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

fn eval_let_bindings_inline(
    bindings: Vec<BlinkValue>,
    mut index: usize,
    env: Arc<RwLock<Env>>,
    body: Vec<BlinkValue>,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= bindings.len() {
            return eval_do_inline(body, 0, nil(), ctx);
        }

        let key_val = &bindings[index];
        let val_expr = &bindings[index + 1];

        let key = match &key_val.read().value {
            Value::Symbol(s) => s.clone(),
            _ => {
                return EvalResult::Value(
                    BlinkError::eval("let binding keys must be symbols").into_blink_value(),
                );
            }
        };

        let result = trace_eval(val_expr.clone(), ctx);
        match result {
            EvalResult::Value(v) => {
                if v.is_error() {
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
                        if v.is_error() {
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


fn eval_let(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(BlinkError::eval("let expects a binding vector and at least one body form").into_blink_value());
    }

    let bindings_val = &args[0];
    
    let bindings = match &bindings_val.read().value {


        
        Value::Vector(vs) => vs.clone(),
         _ => {
            // need to print value type here
            return EvalResult::Value(BlinkError::eval("let expects a vector of bindings").into_blink_value());
        }
    };

    if bindings.len() % 2 != 0 {
        return EvalResult::Value(BlinkError::eval("let binding vector must have an even number of elements").into_blink_value());
    }

    let local_env = Arc::new(RwLock::new(Env::with_parent(ctx.env.clone())));
    let mut local_ctx = ctx.with_env(local_env.clone());

    eval_let_bindings_inline(bindings, 0, local_env, args[1..].to_vec(), &mut local_ctx)
}



fn eval_and(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(bool_val(true));
    }
    eval_and_inline(args.to_vec(), 0, bool_val(true), ctx)
}

fn eval_and_inline(
    args: Vec<BlinkValue>, 
    mut index: usize, 
    mut last: BlinkValue, 
    ctx: &mut EvalContext
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
                if matches!(last.read().value, Value::Bool(false) | Value::Nil) {
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
                        if matches!(v.read().value, Value::Bool(false) | Value::Nil) {
                            return EvalResult::Value(v);
                        }
                        eval_and_inline(args, index + 1, v, ctx)
                    }),
                };
            }
        }
    }
}

fn eval_or(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(nil());
    }
    eval_or_inline(args.to_vec(), 0, ctx)
}

fn eval_or_inline(
    args: Vec<BlinkValue>, 
    mut index: usize, 
    ctx: &mut EvalContext
) -> EvalResult {
    loop {
        if index >= args.len() {
            return EvalResult::Value(nil());
        }

        let result = trace_eval(args[index].clone(), ctx);
        match result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                // Short-circuit on truthy values
                if !matches!(val.read().value, Value::Bool(false) | Value::Nil) {
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
                        if !matches!(v.read().value, Value::Bool(false) | Value::Nil) {
                            return EvalResult::Value(v);
                        }
                        eval_or_inline(args, index + 1, ctx)
                    }),
                };
            }
        }
    }
}

fn eval_try(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(BlinkError::arity(2, args.len(), "try").into_blink_value());
    }
    let res = try_eval!(trace_eval(args[0].clone(), ctx));
    if res.is_error() {
        forward_eval!(trace_eval(args[1].clone(), ctx))
    } else {
        EvalResult::Value(res)
    }
}

fn eval_apply(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(BlinkError::arity(2, args.len(), "apply").into_blink_value());
    }
    let func = try_eval!(trace_eval(args[0].clone(), ctx));
    let evaluated_list = try_eval!(trace_eval(args[1].clone(), ctx));
    let list_items = match &evaluated_list.read().value {
        Value::List(xs) => xs.clone(),
        _ => {
            return EvalResult::Value(BlinkError::eval("apply expects a list as second argument").into_blink_value());
        }
    };
    eval_func(func, list_items, ctx)
}


fn load_native_library(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use libloading::Library;

    if args.len() != 1 {
        return EvalResult::Value(BlinkError::arity(1, args.len(), "load-native").into_blink_value());
    }

    let libname = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return EvalResult::Value(BlinkError::eval("load-native expects a string").into_blink_value());
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
            return EvalResult::Value(BlinkError::eval(format!("Failed to load native lib '{}': {}", filename, e)).into_blink_value());
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
                        return EvalResult::Value(BlinkError::eval(format!("Failed to find blink_register or blink_register_with_exports in '{}': {}", filename, e)).into_blink_value());
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



    EvalResult::Value(nil())
}

fn load_native_code(
    args: &[BlinkValue],
    ctx: &mut EvalContext,
) -> Result<EvalResult, BlinkError> {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use libloading::Library;
    use std::collections::HashSet;

    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    
    if args.len() < 1 || args.len() > 2 {
        return Err(BlinkError::arity(1, args.len(), "compile-plugin").with_pos(pos));
    }

    let plugin_name = match &args[0].read().value {
        Value::Str(s) => s.clone(),
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
                let is_err = {
                    let read_v = v.read();
                    matches!(&read_v.value, Value::Error(_))
                };
        
                if is_err {
                    let read_v = v.read();
                    if let Value::Error(e) = &read_v.value {
                        return Err(e.clone());
                    }
                }
        
                v
            }
            suspended => return Ok(suspended),
        };
        let options_borrowed = options_val.read();
        if let Value::Map(opt_map) = &options_borrowed.value {
            if let Some(path_val) = opt_map.get(&keyword_at(":path", None)).or_else(|| opt_map.get(&str_val("path"))) {
                if let Value::Str(path) = &path_val.read().value {
                    plugin_path = path.clone();
                }
            }
            if let Some(import_val) = opt_map.get(&keyword_at(":import", None)).or_else(|| opt_map.get(&str_val("import"))) {
                if matches!(&import_val.read().value, Value::Bool(true)) {
                    auto_import = true;
                }
            }
        } else {
            return Err(BlinkError::eval("Second argument to compile-plugin must be a map").with_pos(pos));
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

    

    Ok(EvalResult::Value(nil()))
}

// Helper function to extract symbol names from an environment
fn extract_env_symbols(env: &Arc<RwLock<Env>>) -> HashSet<String> {
    env.read().vars.keys().cloned().collect()
}

fn eval_def_reader_macro(
    args: &[BlinkValue],
    ctx: &mut EvalContext,
) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(BlinkError::arity(2, args.len(), "def-reader-macro").into_blink_value());
    }

    let char_val = match &args[0].read().value {
        Value::Str(s) => s.clone(),
        _ => {
            return EvalResult::Value(BlinkError::eval("First argument to def-reader-macro must be a string").into_blink_value());
        }
    };

    let ch = char_val;

    let func = try_eval!(trace_eval(args[1].clone(), ctx));

    ctx.reader_macros
        .write()
        .reader_macros
        .insert(ch, func.clone());
    EvalResult::Value(func)
}

/// Module declaration and context management
fn eval_mod(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(BlinkError::arity(1, 0, "mod").into_blink_value());
    }
    
    // Parse flags
    let (flags, name_index) = parse_flags(args);
    if flags.len() < 1 {
        return EvalResult::Value(BlinkError::eval("At least one flag is required.").into_blink_value());
    }
    
    // Extract module name
    if name_index >= args.len() {
        return EvalResult::Value(BlinkError::eval("Missing module name after flags").into_blink_value());
    }
    
    let name = match extract_name(&args[name_index]) {
        Ok(name) => name,
        Err(e) => {
            return EvalResult::Value(e.into_blink_value());
        }
    };
    
    
    let (options, body_start) = match parse_mod_options(&args[name_index + 1..]) {
        Ok(res) => res,
        Err(e) => {
            return EvalResult::Value(e.into_blink_value());
        }
    };
    

    let should_declare = flags.contains("declare") || flags.is_empty();
    let should_enter = flags.contains("enter");
    
    let mut module = ctx.module_registry.read().get_module(&name);
    

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
        match update_module_exports(&mut module_guard, &options["exports"]) {
            Ok(_) => (),
            Err(e) => {
                return EvalResult::Value(e.into_blink_value());
            }
        };
    }

    if should_enter {
        if let Some(module) = module {
            ctx.current_module = Some(module.read().name.clone());
            ctx.env = module.read().env.clone();
        } else {
            return EvalResult::Value(BlinkError::eval("Module not found").into_blink_value());
        }
    }
    EvalResult::Value(nil())
    
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
fn update_module_exports(module: &mut Module, exports_val: &BlinkValue) -> Result<(), BlinkError> {
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
                    return Err(BlinkError::eval("Exports must be a list of symbols"));
                }
            }
        },
        _ => return Err(BlinkError::eval("Exports must be a list or vector")),
    }
    
    Ok(())
}

fn eval_load(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    let (source_type, source_value, options, _) = match parse_load_args(args) {
        Ok(res) => res,
        Err(e) => {
            return EvalResult::Value(e.into_blink_value());
        }
    };
    
    match source_type.as_str() {
        "file" => {
            let file_path = PathBuf::from(&source_value);
            let loaded = eval_blink_file(file_path, ctx);
            try_eval!(loaded);
            EvalResult::Value(nil())
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
                            return EvalResult::Value(e.into_blink_value());
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
        
        _ => EvalResult::Value(BlinkError::eval(format!("Unknown load source type: :{}", source_type)).into_blink_value()),
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
fn extract_name(value: &BlinkValue) -> Result<String, BlinkError> {
    match &value.read().value {
        Value::Symbol(s) => Ok(s.clone()),
        Value::Str(s) => Ok(s.clone()),
        _ => Err(BlinkError::eval("Expected a symbol or string for name")),
    }
}

/// Load and evaluate a Blink source file
fn eval_blink_file(file_path: PathBuf, ctx: &mut EvalContext) -> EvalResult {
    // Don't evaluate if already loaded
    if ctx.module_registry.read().is_file_evaluated(&file_path) {
        return EvalResult::Value(nil());
    }

    let contents = match fs::read_to_string(&file_path) {
        Ok(contents) => contents,
        Err(e) => {
            return EvalResult::Value(BlinkError::eval(format!("Failed to read file: {}", e)).into_blink_value());
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
                return EvalResult::Value(BlinkError::eval("File name missing.").into_blink_value());
            }
        },
        None => {
            return EvalResult::Value(BlinkError::eval("File name missing.").into_blink_value());
        },
    };
    ctx.current_file = Some(file_name);
    
    // Evaluate all forms in the file
    try_eval!(eval_file_forms_inline(forms, 0, ctx));
    
    // Restore previous file context
    ctx.current_file = old_file;
    
    // Mark file as evaluated
    ctx.module_registry.write().mark_file_evaluated(file_path);
    
    EvalResult::Value(nil())
}

fn eval_file_forms_inline(
    forms: Vec<BlinkValue>,
    mut index: usize,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= forms.len() {
            return EvalResult::Value(nil());
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

fn parse_import_args(args: &[BlinkValue]) -> Result<(ImportType, Option<HashMap<String, BlinkValue>>), BlinkError> {
    if args.is_empty() {
        return Err(BlinkError::arity(1, 0, "imp"));
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
        
        _ => Err( BlinkError::eval("imp expects a string (file) or vector (symbols)")),
    }
}

fn parse_symbol_import(
    symbol_list: &[BlinkValue], 
    remaining_args: &[BlinkValue]
) -> Result<(ImportType, HashMap<String, BlinkValue>), BlinkError> {
    
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
            _ => return Err(BlinkError::eval("Symbol list must contain symbols")),
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
                        return Err(BlinkError::eval(":from requires a module name"));
                    }
                    if let Value::Symbol(module) = &remaining_args[j + 1].read().value {
                        module_name = Some(module.clone());
                        j += 2;
                    } else {
                        return Err(BlinkError::eval(":from expects a module name"));
                    }
                },
                "reload" => {
                    options.insert("reload".to_string(), bool_val(true));
                    j += 1;
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

fn parse_mod_options(args: &[BlinkValue]) -> Result<(HashMap<String, BlinkValue>, usize), BlinkError> {
    let mut options = HashMap::new();
    let mut i = 0;
    
    while i < args.len() {
        match &args[i].read().value {
            Value::Keyword(key) => {
                match key.as_str() {
                    "exports" => {
                        if i + 1 >= args.len() {
                            return Err(BlinkError::eval(":exports requires a list"));
                        }
                        options.insert("exports".to_string(), args[i + 1].clone());
                        i += 2;
                    },
                    other => {
                        return Err(BlinkError::eval(format!("Unknown option: {}", other)));
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

fn find_module_file(module_name: &str, ctx: &mut EvalContext) -> Result<PathBuf, BlinkError> {
    // 1. Check if module is already registered (we know which file it came from)
    if let Some(module) = ctx.module_registry.read().get_module(module_name) {
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
            let reference = create_module_reference(module_name, export_name);
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
fn parse_options(args: &[BlinkValue]) -> Result<(HashMap<String, BlinkValue>, usize), BlinkError> {
    let mut options = HashMap::new();
    let mut i = 0;

    while i < args.len() {
        match &args[i].read().value {
            // make sure that the next argument is a value
            Value::Keyword(key) => {
                if i + 1 >= args.len() {
                    return Err(BlinkError::eval(format!("Option {} requires a value", key)));
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

fn parse_load_args(args: &[BlinkValue]) -> Result<(String, String, HashMap<String, BlinkValue>, usize), BlinkError> {
    if args.is_empty() {
        return Err(BlinkError::arity(2, 0, "load"));
    }
    
    // First argument should be a keyword indicating source type
    let source_type = match &args[0].read().value {
        Value::Keyword(kw) => kw.clone(),
        _ => return Err(BlinkError::eval("load expects a keyword as first argument (:file, :native, :cargo, :dylib, :url, :git)")),
    };
    
    // Second argument should be the source value
    if args.len() < 2 {
        return Err(BlinkError::eval(format!("load {} requires a source argument", source_type)));
    }
    
    let source_value = match &args[1].read().value {
        Value::Str(s) => s.clone(),
        _ => return Err(BlinkError::eval("load source must be a string")),
    };
    
    // Parse any additional options
    let (options, s) = parse_options(&args[2..])?;
    
    Ok((source_type, source_value, options, s))
}

fn eval_imp(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    let parsed = parse_import_args(args);
    let  (import_type, _options) = match parsed {
        Ok(parsed) => parsed,
        Err(e) => {
            return EvalResult::Value(e.into_blink_value());
        }
    };
    
    match import_type {
        ImportType::File(file_name) => {
            // File import: (imp "module-name")
            let file_path = PathBuf::from(format!("lib/{}.blink", file_name));
            
            // Load the file if needed
            let loaded = eval_blink_file(file_path, ctx);
            try_eval!(loaded);
            
            // Make the file's modules available for qualified access
            // (they're already registered by eval_mod during file evaluation)
            EvalResult::Value(nil())
        },
        
        ImportType::Symbols { symbols, module, aliases } => {
            // Check if module already exists
            let module_exists = ctx.module_registry.read().get_module(&module).is_some();
            
            if !module_exists {
                
                // Find which file contains the module
                let file_path = find_module_file(&module, ctx);
                if let Err(e) = file_path {
                    return EvalResult::Value(e.into_blink_value());
                }
                let file_path = file_path.unwrap();
                
                // Load the file if needed (this registers all modules in the file)
                let loaded = eval_blink_file(file_path, ctx);
                try_eval!(loaded);
                
                // Verify the module is now available
                if ctx.module_registry.read().get_module(&module).is_none() {
                    return EvalResult::Value(BlinkError::eval(format!("Module '{}' was not found in the loaded file", module)).into_blink_value());
                }
            }
            
            // Import the symbols into current environment
            match import_symbols_into_env(&symbols, &module, &aliases, ctx) {
                Ok(_) => (),
                Err(e) => {
                    return EvalResult::Value(e.into_blink_value());
                }
            };
            EvalResult::Value(nil())
        }
    }
}



// Add these functions to eval.rs
fn eval_quasiquote(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(args, 1, "quasiquote") {
        return EvalResult::Value(e.into_blink_value());
    }
    expand_quasiquote(args[0].clone(), ctx)
}
fn expand_quasiquote(expr: BlinkValue, ctx: &mut EvalContext) -> EvalResult {
    match &expr.read().value {
        Value::List(items) if !items.is_empty() => {
            let first = &items[0];
            match &first.read().value {
                Value::Symbol(s) if s == "unquote" => {
                    if items.len() != 2 {
                        return EvalResult::Value(BlinkError::eval("unquote expects exactly one argument").into_blink_value());
                    }
                    // Just forward the eval of the unquoted form
                    forward_eval!(trace_eval(items[1].clone(), ctx))
                }

                Value::Symbol(s) if s == "unquote-splicing" => {
                    return EvalResult::Value(BlinkError::eval("unquote-splicing not valid here").into_blink_value());
                }

                _ => {
                    // Use list version for lists
                    expand_quasiquote_items_inline(items.clone(), 0, Vec::new(), false, expr.read().pos.clone(), ctx)
                }
            }
        }

        Value::Vector(items) => {
            // Use vector version for vectors
            expand_quasiquote_items_inline(items.clone(), 0, Vec::new(), true, expr.read().pos.clone(), ctx)
        }

        _ => EvalResult::Value(expr.clone()),
    }
}

fn expand_quasiquote_items_inline(
    items: Vec<BlinkValue>,  
    mut index: usize,
    mut expanded_items: Vec<BlinkValue>,
    is_vector: bool,
    original_pos: Option<SourceRange>,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= items.len() {
            // Return the appropriate type
            if is_vector {
                return EvalResult::Value(BlinkValue(Arc::new(RwLock::new(LispNode {
                    value: Value::Vector(expanded_items),
                    pos: original_pos,
                }))));
            } else {
                return EvalResult::Value(list_val(expanded_items));
            }
        }

        let item = items[index].clone();

        // Check for unquote-splicing
        if let Value::List(inner_items) = &item.read().value {
            if !inner_items.is_empty() {
                if let Value::Symbol(s) = &inner_items[0].read().value {
                    if s == "unquote-splicing" {
                        if inner_items.len() != 2 {
                            return EvalResult::Value(BlinkError::eval("unquote-splicing expects exactly one argument").into_blink_value());
                        }

                        let result = trace_eval(inner_items[1].clone(), ctx);
                        match result {
                            EvalResult::Value(spliced) => {
                                if spliced.is_error() {
                                    return EvalResult::Value(spliced);
                                }
                                if let Value::List(splice_items) = &spliced.read().value {
                                    expanded_items.extend(splice_items.clone());
                                } else {
                                    return EvalResult::Value(BlinkError::eval("unquote-splicing expects a list").into_blink_value());
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
                                        if let Value::List(splice_items) = &v.read().value {
                                            expanded_items.extend(splice_items.clone());
                                            expand_quasiquote_items_inline(items, index + 1, expanded_items, is_vector, original_pos, ctx)
                                        } else {
                                            EvalResult::Value(BlinkError::eval("unquote-splicing expects a list").into_blink_value())
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
                        expand_quasiquote_items_inline(items, index + 1, expanded_items, is_vector, original_pos, ctx)
                    }),
                };
            }
        }
    }
}


fn eval_macro(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 {
        return EvalResult::Value(BlinkError::eval("macro expects at least 2 arguments: params and body").into_blink_value());
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
                        return EvalResult::Value(BlinkError::eval("& must be followed by a parameter name").into_blink_value());
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
                                return EvalResult::Value(BlinkError::eval("& must be followed by a parameter name").into_blink_value());
                            } else {
                                params.push(s.clone());
                            }
                        }
                        i += 1;
                    }
                    (params, is_variadic)
                } else {
                    return EvalResult::Value(BlinkError::eval("macro expects a vector of symbols as parameters").into_blink_value());
                }
            } else {
                return EvalResult::Value(BlinkError::eval("macro expects a vector of symbols as parameters").into_blink_value());
            }
        }
        _ => {
            return EvalResult::Value(BlinkError::eval("macro expects a vector of symbols as parameters").into_blink_value());
        }
    };

    EvalResult::Value(BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Macro {
            params,
            body: args[1..].to_vec(),
            env: Arc::clone(&ctx.env),
            is_variadic,
        },
        pos: None,
    }))))
}


// Special handling for deref
fn eval_deref(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(args, 1, "deref") {
        return EvalResult::Value(e.into_blink_value());
    }
    let future_val = try_eval!(trace_eval(args[0].clone(), ctx));

    match ctx.async_ctx {
        AsyncContext::Blocking => {
            match &future_val.read().value {
                Value::Future(future) => {
                    // Use block_in_place to safely block
                    let res = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(future.clone())
                            
                    });
                    EvalResult::Value(res)
                }
                _ => EvalResult::Value(BlinkError::eval("deref can only be used on futures").into_blink_value())
            }
            
        },
        AsyncContext::Goroutine(_) => {
            match &future_val.read().value {
                Value::Future(future) => {
                    let future_clone = future.clone();
                    
                    EvalResult::Suspended { future: future_clone, resume: Box::new(|resolved, _ctx| {
                        EvalResult::Value(resolved)
                    }) }
        
                    
                }
                _ => EvalResult::Value(BlinkError::eval("deref can only be used on futures").into_blink_value())
            }
        }
        
    }
}

pub fn eval_go(args: &[BlinkValue], ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(BlinkError::arity(1, args.len(), "go").into_blink_value());
    }

    let goroutine_ctx = ctx.clone();
    let expr = args[0].clone();

    ctx.goroutine_scheduler.spawn_with_context(goroutine_ctx, move |ctx| {
        trace_eval(expr, ctx)
    });

    EvalResult::Value(nil())
}
