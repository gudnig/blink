
mod helpers;
mod result;
mod special_forms;

use std::{sync::Arc, time::Instant};

pub use crate::runtime::EvalContext;
use mmtk::util::ObjectReference;
pub use result::EvalResult;
use parking_lot::RwLock;

use crate::{
    env::Env, eval::special_forms::{
            eval_and, eval_apply, eval_def, eval_def_reader_macro, eval_deref, eval_do, eval_fn, eval_go, eval_if, eval_imp, eval_let, eval_load, eval_macro, eval_mod, eval_or, eval_quasiquote, eval_quote, eval_try
        }, telemetry::TelemetryEvent, value::{unpack_immediate, Callable, ImmediateValue, ValueRef}, value::HeapValue
};

macro_rules! try_eval {
    ($expr:expr, $ctx:expr) => {
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
    ($expr:expr, $ctx:expr) => {
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

// Make them available to submodules
pub(crate) use {forward_eval, try_eval};

pub fn eval(expr: ValueRef, ctx: &mut EvalContext) -> EvalResult {
    match expr {
        ValueRef::Heap(gc_ptr) => {
            // Fetch from arena using the index
            let heap_value = gc_ptr.to_heap_value();
            match heap_value {
                HeapValue::List(list) if list.is_empty() => {
                        EvalResult::Value(ValueRef::nil())
                    }
                    HeapValue::List(list) => eval_list(&list, ctx),
                    _ => {
                        // Everything else is self-evaluating
                        EvalResult::Value(expr)
                    }
                }
        }

        ValueRef::Immediate(packed) => {
            match unpack_immediate(packed) {
                ImmediateValue::Number(_)
                            | ImmediateValue::Bool(_)
                            | ImmediateValue::Keyword(_)
                            | ImmediateValue::Nil => {
                                // Self-evaluating
                                EvalResult::Value(expr)
                            }
                ImmediateValue::Symbol(symbol_id) => {
                                println!("Resolving symbol: {:?}", symbol_id);
                                match ctx.resolve_symbol(symbol_id) {
                                    Ok(val) => EvalResult::Value(val),
                                    Err(err) => EvalResult::Value(ctx.error_value(err)),
                                }
                            }
            }
        }
        ValueRef::Native(tagged_ptr) => {
            match expr.get_native_fn() {
                Some(native_fn) => {
                    native_fn.call(Vec::new(), ctx)
                }
                None => EvalResult::Value(ctx.eval_error("Not a native function")),
            }
        }
    }
}

pub fn trace_eval(expr: ValueRef, ctx: &mut EvalContext) -> EvalResult {
    if ctx.tracing_enabled {
        let start = Instant::now();
        let val = try_eval!(eval(expr, ctx), ctx);
        if let Some(sink) = &ctx.vm.telemetry_sink {
            sink(TelemetryEvent {
                form: format!("{:?}", expr),
                duration_us: start.elapsed().as_micros(),
                result_type: val.type_tag().to_string(),
                result_size: None,
                source: ctx.get_pos(expr),
            });
        }
        EvalResult::Value(val)
    } else {
        eval(expr, ctx)
    }
}

pub fn eval_list(list: &Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    let head = &list[0];

    // Check if head is a symbol (immediate value)
    if let ValueRef::Immediate(packed) = head {
        if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(*packed) {
            let symbol_name = {
                let symbol_table = ctx.vm.symbol_table.read();
                match symbol_table.get_symbol(symbol_id) {
                    Some(name) => Some(name.to_string()), // Clone to avoid holding lock
                    None => None,
                }
            };
            if symbol_name.is_none() {
                return EvalResult::Value(
                    ctx.eval_error(&format!("Invalid symbol ID: {}", symbol_id)),
                );
            }
            let symbol_name = symbol_name.unwrap();
            match symbol_name.as_str() {
                "quote" => return eval_quote(list, ctx),
                "apply" => return eval_apply(&list[1..], ctx),
                "if" => return eval_if(&list[1..], ctx),
                "def" => return eval_def(&list[1..], ctx),
                "fn" => return eval_fn(&list[1..], ctx),
                "do" => return eval_do(&list[1..], ctx),
                "let" => return eval_let(&list[1..], ctx),
                "and" => return eval_and(&list[1..], ctx),
                "or" => return eval_or(&list[1..], ctx),
                "try" => return eval_try(&list[1..], ctx),
                "imp" => return eval_imp(&list[1..], ctx),
                "mod" => return eval_mod(&list[1..], ctx),
                "load" => return eval_load(&list[1..], ctx),
                "macro" => return eval_macro(&list[1..], ctx),
                "rmac" => return eval_def_reader_macro(&list[1..], ctx),
                "quasiquote" => return eval_quasiquote(&list[1..], ctx),
                "unquote" => {
                    return EvalResult::Value(ctx.eval_error("unquote used outside quasiquote"))
                }
                "unquote-splicing" => {
                    return EvalResult::Value(
                        ctx.eval_error("unquote-splicing used outside quasiquote"),
                    )
                }
                "go" => return eval_go(&list[1..], ctx),
                "deref" => return eval_deref(&list[1..], ctx),
                _ => {
                    // Not a special form - treat as function call
                    // This is where symbol resolution happens!
                    return eval_symbol_and_call(symbol_id, &list[1..], ctx);
                }
            }
        }
    }

    // Head is not a symbol - evaluate it and call as function
    eval_function_call_inline(list.clone(), 0, Vec::new(), None, ctx)
}

fn eval_symbol_and_call(symbol_id: u32, args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    // Resolve symbol to actual function
    let function_val = match ctx.resolve_symbol(symbol_id) {
        Ok(val) => val,
        Err(e) => return EvalResult::Value(ctx.error_value(e)),
    };

    // Create args list with function first
    let mut all_args = vec![function_val];
    all_args.extend_from_slice(args);

    eval_function_call_inline(all_args, 1, Vec::new(), Some(function_val), ctx)
}

fn eval_function_call_inline(
    list: Vec<ValueRef>,
    mut index: usize,
    mut evaluated_args: Vec<ValueRef>,
    func: Option<ValueRef>,
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

fn eval_macro_body_inline(
    body: Vec<ValueRef>,
    mut index: usize,
    mut expansion: ValueRef,
    original_env: ObjectReference,
    mut ctx: EvalContext,
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
    body: Vec<ValueRef>,
    mut index: usize,
    mut result: ValueRef,
    original_env: ObjectReference,
    mut ctx: EvalContext,
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

pub fn eval_func(func: ValueRef, args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    match &func {
        ValueRef::Heap(gc_ptr) => {
            let func = gc_ptr.to_heap_value();

            match func {


                HeapValue::Macro(Callable {
                    params,
                    body,
                    env,
                    is_variadic,
                }) => {
                    // Arity check...
                    if is_variadic {
                        if args.len() < params.len() - 1 {
                            return EvalResult::Value(ctx.arity_error(
                                params.len() - 1,
                                args.len(),
                                "macro (at least)",
                            ));
                        }
                    } else if params.len() != args.len() {
                        return EvalResult::Value(ctx.arity_error(
                            params.len(),
                            args.len(),
                            "macro",
                        ));
                    }

                    // Create macro environment and bind parameters
                    let mut macro_env = Env::with_parent(env);
                    
                    if is_variadic {
                        for (i, param) in params.iter().take(params.len() - 1).enumerate() {
                            macro_env.set(*param, args[i].clone());
                        }
                        let rest_param = &params[params.len() - 1];
                        let rest_args = args.iter().skip(params.len() - 1).cloned().collect();
                        macro_env.set(*rest_param, ctx.list_value(rest_args));
                    } else {
                        for (param, arg) in params.iter().zip(args.iter()) {
                            macro_env.set(*param, arg.clone());
                        }
                    }
                    let macro_env_ref = ctx.vm.alloc_env(macro_env);
                

                    let old_env = ctx.env;
                    let macro_ctx = ctx.with_env(macro_env_ref);

                    // Evaluate macro body with proper suspension handling
                    eval_macro_body_inline(body.clone(), 0, ctx.nil_value(), old_env, macro_ctx)
                }

                HeapValue::Function(Callable { params, body, env, is_variadic }) => {
                    if params.len() != args.len() {
                        return EvalResult::Value(ctx.arity_error(params.len(), args.len(), "fn"));
                    }

                    let mut local_env = Env::with_parent(env);
                    
                    for (param, val) in params.iter().zip(args) {
                        local_env.set(*param, val);
                    }
                    let local_env_ref = ctx.vm.alloc_env(local_env);

                    let old_env = ctx.env;
                    let local_ctx = ctx.with_env(local_env_ref);

                    eval_function_body_inline(body.clone(), 0, ctx.nil_value(), old_env, local_ctx)
                }
                _ => EvalResult::Value(ctx.eval_error("Not a function")),
            }
        }
        ValueRef::Native(_) => {
            func.call_native(args, ctx)
        }

        _ => EvalResult::Value(ctx.eval_error("Not a function")),
    }
}
