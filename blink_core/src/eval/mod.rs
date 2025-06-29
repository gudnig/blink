
mod helpers;
mod result;
mod special_forms;

use std::{sync::Arc, time::Instant};

pub use crate::runtime::EvalContext;
pub use result::EvalResult;
pub use helpers::*;
use parking_lot::RwLock;

use crate::{
    error::BlinkError, eval::special_forms::{
            eval_and, eval_apply, eval_def, eval_def_reader_macro, eval_deref, eval_do, eval_fn, eval_go, eval_if, eval_imp, eval_let, eval_load, eval_macro, eval_mod, eval_or, eval_quasiquote, eval_quote, eval_try
        }, runtime::{ContextualBoundary, ValueBoundary}, telemetry::TelemetryEvent, value::{unpack_immediate, ImmediateValue, IsolatedValue, Macro, SharedValue, UserDefinedFn, ValueRef}
        , env::Env
};

macro_rules! try_eval {
    ($expr:expr, $ctx:expr) => {
        match $expr {
            EvalResult::Value(v) => {
                if $ctx.is_err(&v) {
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
                if $ctx.is_err(&v) {
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
        ValueRef::Shared(idx) => {
            // Fetch from arena using the index
            let shared_value = {
                let arena = ctx.shared_arena.read();
                arena.get(idx).map(|idx| idx.clone())
            };
            if let Some(shared_value) = shared_value {
                match shared_value.as_ref() {
                    SharedValue::List(list) if list.is_empty() => {
                        EvalResult::Value(ValueRef::nil())
                    }
                    SharedValue::List(list) => eval_list(&list, ctx),
                    _ => {
                        // Everything else is self-evaluating
                        EvalResult::Value(expr)
                    }
                }
            } else {
                // Invalid/stale arena index
                EvalResult::Value(ctx.eval_error("Invalid value reference"))
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
                    match ctx.resolve_symbol(symbol_id) {
                        Ok(val) => EvalResult::Value(val),
                        Err(err) => EvalResult::Value(ctx.error_value(err)),
                    }
                }
            }
        }

        ValueRef::Gc(_gc_ptr) => {
            // TODO: Handle GC values when implemented
            todo!("GC values not yet implemented")
        }
    }
}

pub fn trace_eval(expr: ValueRef, ctx: &mut EvalContext) -> EvalResult {
    if ctx.tracing_enabled {
        let start = Instant::now();
        let val = try_eval!(eval(expr, ctx), ctx);
        if let Some(sink) = &*ctx.telemetry_sink {
            sink(TelemetryEvent {
                form: format!("{:?}", expr),
                duration_us: start.elapsed().as_micros(),
                result_type: ctx.type_tag(val),
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
                let symbol_table = ctx.symbol_table.read();
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
                if ctx.is_err(&f) {
                    return EvalResult::Value(f);
                }
                return eval_function_call_inline(list, 1, evaluated_args, Some(f), ctx);
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if ctx.is_err(&v) {
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
                if ctx.is_err(&val) {
                    return EvalResult::Value(val);
                }
                evaluated_args.push(val);
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if ctx.is_err(&v) {
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
    original_env: Arc<RwLock<Env>>,
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
                if ctx.is_err(&val) {
                    return EvalResult::Value(val);
                }
                expansion = val;
                index += 1;
            }
            EvalResult::Suspended { future, resume: _ } => {
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |v, ctx| {
                        if ctx.is_err(&v) {
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
    original_env: Arc<RwLock<Env>>,
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
                        eval_function_body_inline(body, index + 1, v, original_env, ctx.clone())
                    }),
                };
            }
        }
    }
}

pub fn eval_func(func: ValueRef, args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    match &func {
        ValueRef::Shared(idx) => {
            let func = {
                let arena = ctx.shared_arena.read();
                arena.get(*idx).unwrap().clone()
            };

            match func.as_ref() {
                SharedValue::NativeFunction(f) => {
                    f.call(args, ctx)
                },

                // destructuring the macro
                SharedValue::Macro(Macro {
                    params,
                    body,
                    env,
                    is_variadic,
                }) => {
                    // Arity check...
                    if *is_variadic {
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
                    let macro_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));
                    {
                        let mut env_guard = macro_env.write();
                        if *is_variadic {
                            for (i, param) in params.iter().take(params.len() - 1).enumerate() {
                                env_guard.set(*param, args[i].clone());
                            }
                            let rest_param = &params[params.len() - 1];
                            let rest_args = args.iter().skip(params.len() - 1).cloned().collect();
                            env_guard.set(*rest_param, ctx.list_value(rest_args));
                        } else {
                            for (param, arg) in params.iter().zip(args.iter()) {
                                env_guard.set(*param, arg.clone());
                            }
                        }
                    }

                    let old_env = ctx.env.clone();
                    let macro_ctx = ctx.with_env(macro_env);

                    // Evaluate macro body with proper suspension handling
                    eval_macro_body_inline(body.clone(), 0, ctx.nil_value(), old_env, macro_ctx)
                }

                SharedValue::UserDefinedFunction(UserDefinedFn { params, body, env }) => {
                    if params.len() != args.len() {
                        return EvalResult::Value(ctx.arity_error(params.len(), args.len(), "fn"));
                    }

                    let local_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));
                    {
                        let mut env_guard = local_env.write();
                        for (param, val) in params.iter().zip(args) {
                            env_guard.set(*param, val);
                        }
                    }

                    let old_env = ctx.env.clone();
                    let local_ctx = ctx.with_env(local_env);

                    eval_function_body_inline(body.clone(), 0, ctx.nil_value(), old_env, local_ctx)
                }
                _ => EvalResult::Value(ctx.eval_error("Not a function")),
            }
        }

        _ => EvalResult::Value(ctx.eval_error("Not a function")),
    }
}
