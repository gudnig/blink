
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{BlinkError, BlinkErrorType};
use crate::eval::{eval_func, EvalContext, EvalResult};
use crate::future::BlinkFuture;
use crate::value::{unpack_immediate, Callable, ImmediateValue, NativeFn, ValueRef};
use crate::env::Env;


pub fn native_add(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.number_value(0.0)); // Additive identity
    }
    
    let mut sum = 0.0;
    for arg in args {
        if let Some(val) = ctx.get_number(arg) {
            sum += val;
        } else {
            return EvalResult::Value(ctx.eval_error(&format!("+ expects numbers, got {}", arg.type_tag())));
        }
    }
    EvalResult::Value(ctx.number_value(sum))
}

pub fn native_sub(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.number_value(0.0)); // Subtractive identity
    }
    if args.len() == 1 {
        // Unary minus: (- x) => -x
        if let Some(val) = ctx.get_number(args[0]) {
            return EvalResult::Value(ctx.number_value(-val));
        } else {
            return EvalResult::Value(ctx.eval_error(&format!("- expects numbers, got {}", args[0].type_tag())));
        }
    }
    
    // Binary/n-ary: (- a b c) => a - b - c
    let result = ctx.get_number(args[0]);
    if result.is_none() {
        return EvalResult::Value(ctx.eval_error(&format!("- expects numbers, got {}", args[0].type_tag())));
    }
    let mut result = result.unwrap();
    for arg in &args[1..] {
        let val = ctx.get_number(*arg);
        if val.is_none() {
            return EvalResult::Value(ctx.eval_error(&format!("- expects numbers, got {}", arg.type_tag())));
        }
        let val = val.unwrap();
        result -= val;
    }
    EvalResult::Value(ctx.number_value(result))
}

pub fn native_mul(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.number_value(1.0)); // Multiplicative identity
    }
    
    let mut product = 1.0;
    for arg in args {
        if let Some(val) = ctx.get_number(arg) {
            product *= val;
        } else {
            return EvalResult::Value(ctx.eval_error(&format!("* expects numbers, got {}", arg.type_tag())));
        }
    }
    EvalResult::Value(ctx.number_value(product))
}

pub fn native_div(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.eval_error("/ expects at least one argument"));
    }
    
    let mut val = if let Some(n) = ctx.get_number(args[0]) {
        n
    } else {
        return EvalResult::Value(ctx.eval_error("/ expects numbers"));
    };

    for arg in args.iter().skip(1) {
        if let Some(n) = ctx.get_number(*arg) {
            val /= n;
        } else {
            return EvalResult::Value(ctx.eval_error("/ expects numbers"));
        }
    }
    EvalResult::Value(ctx.number_value(val))
}

pub fn native_eq(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    let result = if let Some((first, rest)) = args.split_first() {
        rest.iter().all(|arg| arg == first)
    } else {
        true
    };

    EvalResult::Value(ctx.bool_value(result))
}

pub fn native_not(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "not"));
    }
    let result = match args[0] {
        ValueRef::Immediate(packed) => {
            let unpacked = unpack_immediate(packed);
            match unpacked {
                ImmediateValue::Bool(b) => !b,
                ImmediateValue::Nil => true,
                _ => false,
            }
        }
        _ => false,
    };
    EvalResult::Value(ctx.bool_value(result))
}



pub fn native_list(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    EvalResult::Value(ctx.list_value(args))
}

pub fn native_vector(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    EvalResult::Value(ctx.vector_value(args))
}

pub fn native_map_construct(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() % 2 != 0 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "map"));
    }

    let pairs = args.chunks(2).map(|chunk| (chunk[0], chunk[1])).collect();

    EvalResult::Value(ctx.map_value(pairs))
    
    
}

pub fn native_print(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    for val in args {
        print!("{} ", val);
    }
    println!();
    EvalResult::Value(ctx.nil_value())
}

pub fn native_type_of(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "type-of"));
    }

    let arg = &args[0];
    let type_name = arg.type_name();

    EvalResult::Value(ctx.string_value(type_name))
}

pub fn native_cons(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "cons"));
    }
    let mut new_list = vec![args[0]];
    let old_list =  if let Some(v)  = args[0].get_list(){
        v 
    } else if let Some(v) = args[1].get_vec(){
        v
    } else {
        return EvalResult::Value(ctx.eval_error("second argument to cons must be a list or vector"));
    };
    new_list.extend(old_list);
    EvalResult::Value(ctx.list_value(new_list))
}

pub fn native_first(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "first"));
    }
    
    match args[0].get_list() {
        Some(list) => {
            if list.is_empty() {
                EvalResult::Value(ctx.nil_value())
            } else {
                EvalResult::Value(list[0])
            }
        },
        None => match args[0].get_vec() {
            Some(vec) => {
                if vec.is_empty() {
                    EvalResult::Value(ctx.nil_value())
                } else {
                    EvalResult::Value(vec[0])
                }
            },
            None => EvalResult::Value(ctx.eval_error("first expects a list or vector"))
        }
    }
}

pub fn native_rest(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "rest"));
    }
    
    match args[0].get_list() {
        Some(list) => {
            let rest_items: Vec<ValueRef> = list.iter().skip(1).cloned().collect();
            EvalResult::Value(ctx.list_value(rest_items))
        },
        None => match args[0].get_vec() {
            Some(vec) => {
                let rest_items: Vec<ValueRef> = vec.iter().skip(1).cloned().collect();
                EvalResult::Value(ctx.list_value(rest_items)) // Note: returns list, not vector
            },
            None => EvalResult::Value(ctx.eval_error("rest expects a list or vector"))
        }
    }
}

pub fn native_empty_q(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "empty?"));
    }
    
    let is_empty = match args[0].get_list() {
        Some(list) => list.is_empty(),
        None => match args[0].get_vec() {
            Some(vec) => vec.is_empty(),
            None => {
                return EvalResult::Value(ctx.eval_error("empty? expects a list or vector"));
            }
        }
    };
    
    EvalResult::Value(ctx.bool_value(is_empty))
}

pub fn native_count(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "count"));
    }
    
    let count = match args[0].get_list() {
        Some(list) => list.len(),
        None => match args[0].get_vec() {
            Some(vec) => vec.len(),
            None => {
                return EvalResult::Value(ctx.eval_error("count expects a list or vector"));
            }
        }
    };
    
    EvalResult::Value(ctx.number_value(count as f64))
}

pub fn native_get(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 2 || args.len() > 3 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "get"));
    }

    let target_val = &args[0];
    let key_val = &args[1];
    let fallback_val = args.get(2).cloned();

    
    let target_ref = target_val;    

    if let Some(target) = target_val.get_vec(){
        if let Some(n) = ctx.get_number(*key_val){
            let idx = n as usize;
            if let Some(val) = target.get(idx) {
                return EvalResult::Value(*val);
            } else if let Some(default) = fallback_val {
                return EvalResult::Value(default);
            } else {
                return EvalResult::Value(ctx.nil_value());
            }
        } else {
            return EvalResult::Value(ctx.eval_error("get expects a number as second argument"));
        }
    } else if let Some(target) = target_val.get_list(){
        if let Some(n) = ctx.get_number(*key_val){
            let idx = n as usize;
            if let Some(val) = target.get(idx) {
                return EvalResult::Value(*val);
            } else if let Some(default) = fallback_val {
                return EvalResult::Value(default);
            } else {
                return EvalResult::Value(ctx.nil_value());
            }
        } else {
            return EvalResult::Value(ctx.eval_error("get expects a number as second argument"));
        }
    } else if let Some(target) = target_val.get_map(){
        let res = target.get(key_val);
        if let Some(val) = res {
            return EvalResult::Value(*val);
        } else if let Some(default) = fallback_val {
            return EvalResult::Value(default);
        } else {
            return EvalResult::Value(ctx.nil_value());
        }
    } else {
        return EvalResult::Value(ctx.eval_error("get expects a list, vector, or map"));
    }
}

pub fn native_map(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "map"));
    }
    
    let func = args[0];
    let collection = {
        if let Some(list) = args[1].get_list() {
            list
        } else if let Some(vec) = args[1].get_vec() {
            vec
        } else {
            return EvalResult::Value(ctx.eval_error("map expects a list or vector"));
        }
    };
    
    // Start the mapping process
    map_inline(func, collection, 0, Vec::new(), ctx)
}

fn map_inline(
    func: ValueRef,
    items: Vec<ValueRef>, 
    mut index: usize,
    mut results: Vec<ValueRef>,
    ctx: &mut EvalContext
) -> EvalResult {
    loop {
        if index >= items.len() {
            // All items processed - return the results
            let result_list = ctx.list_value(results);
            return EvalResult::Value(result_list);
        }

        // Apply function to current item
        let result = eval_func(func, vec![items[index]], ctx);
        match result {
            EvalResult::Value(val) => {
                if val.is_error() {
                    return EvalResult::Value(val);
                }
                results.push(val);
                index += 1;
                // Continue loop
            }
            
            EvalResult::Suspended { future, resume: _ } => {
                // Function suspended - capture continuation
                return EvalResult::Suspended {
                    future,
                    resume: Box::new(move |resolved_val, ctx| {
                        if resolved_val.is_error() {
                            return EvalResult::Value(resolved_val);
                        }
                        
                        // Add the resolved value and continue mapping
                        results.push(resolved_val);
                        map_inline(func, items, index + 1, results, ctx)
                    }),
                };
            }
        }
    }
}

fn native_future(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 0 {
        return EvalResult::Value(ctx.arity_error(0, args.len(), "future"));
    }

    EvalResult::Value(ctx.future_value(BlinkFuture::new()))
}

fn native_complete_future(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "complete"));
    }
    if let Some(future) = args[0].get_future(){
        future.complete(args[1]).map_err(|e| BlinkError::eval(e.to_string()));
        return EvalResult::Value(ctx.nil_value());
    } else {
        return EvalResult::Value(ctx.eval_error("complete expects a future"));
    }
}

fn native_error(args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
    if args.len() < 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "error"));
    }

    let message = if let Some(message) = args[0].get_string() {
        message
    } else {
        "".to_string()
    };

    let data = if let Some(data) = args.get(1) {
        Some(*data)
    } else {
        None
    };

    let pos = ctx.get_pos(args[0]);

    let error = BlinkError {
        pos,
        message,
        error_type: BlinkErrorType::UserDefined { data },
    };
    EvalResult::Value(ctx.error_value(error))
}

pub fn register_builtins(ctx: &mut EvalContext) {

    let reg = |s: &str, f: fn(Vec<ValueRef>, &mut EvalContext) -> EvalResult, ctx: &mut EvalContext| -> ValueRef {
        let sym = ctx.intern_symbol(s);
        let sym = ctx.get_symbol_id(sym).unwrap();
        let native_fn = NativeFn::Contextual(Box::new(f));
        let val = ctx.native_function_value(native_fn);
        ctx.set_symbol(sym, val);
        val
    };


    reg("+", native_add, ctx);
    reg("-", native_sub, ctx);
    reg("*", native_mul, ctx);
    reg("/", native_div, ctx);
    reg("=", native_eq, ctx);
    reg("not", native_not, ctx);

    
    reg("list", native_list, ctx);
    reg("vector", native_vector, ctx);
    reg("hash-map", native_map_construct, ctx);
    reg("map", native_map, ctx);
    reg("print", native_print, ctx);
    reg("type-of", native_type_of, ctx);
    reg("cons", native_cons, ctx);
    reg("first", native_first, ctx);
    reg("rest", native_rest, ctx);
    reg("get", native_get, ctx);    

    // TODO: Error module
    reg("err", native_error, ctx);


    // TODO: async module
    reg("future", native_future, ctx);
    reg("complete", native_complete_future, ctx);

}

pub fn register_builtin_macros(ctx: &mut EvalContext) {
    

    
    let current_env = ctx.env.clone();
    
    let if_sym_val = ctx.symbol_value("if");
    let do_sym_val = ctx.symbol_value("do");
    let let_sym_val = ctx.symbol_value("let");
    let fn_sym_val = ctx.symbol_value("fn");
    let def_sym_val = ctx.symbol_value("def");

    let not_sym_val = ctx.symbol_value("not");
    let count_sym_val = ctx.symbol_value("count");
    let cons_sym_val = ctx.symbol_value("cons");
    let list_sym_val = ctx.symbol_value("list");
    let first_sym_val = ctx.symbol_value("first");
    let rest_sym_val = ctx.symbol_value("rest");
    let empty_sym_val = ctx.symbol_value("empty?");
    let nil_sym_val = ctx.symbol_value("nil");
    let true_sym_val = ctx.symbol_value("true");
    let eq_sym_val = ctx.symbol_value("=");

    let condition_sym_val = ctx.symbol_value("condition");
    let when_sym_val = ctx.symbol_value("when");
    let unless_sym_val = ctx.symbol_value("unless");
    let forms_sym_val = ctx.symbol_value("forms");
    let and_sym_val = ctx.symbol_value("and");
    let or_sym_val = ctx.symbol_value("or");

    let condition_sym = ctx.get_symbol_id(condition_sym_val).unwrap();
    let when_sym = ctx.get_symbol_id(when_sym_val).unwrap();
    let unless_sym = ctx.get_symbol_id(unless_sym_val).unwrap();
    let forms_sym = ctx.get_symbol_id(forms_sym_val).unwrap();
    let and_sym = ctx.get_symbol_id(and_sym_val).unwrap();
    let or_sym = ctx.get_symbol_id(or_sym_val).unwrap();

    
    let body_sym_val = ctx.symbol_value("body");
    
    
    // when - expands to (if condition (do ...))
    let cons_expr = ctx.list_value(vec![cons_sym_val, do_sym_val, body_sym_val]);
    let when_body = vec![if_sym_val, condition_sym_val, cons_expr];
    
    let when_macro = Callable {
        params: vec![condition_sym],  
        is_variadic: true,       
        body: when_body,        
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };
    
    let macro_value = ctx.macro_value(when_macro);
    ctx.set_symbol(when_sym, macro_value);

    // unless - expands to (if (not condition) (do ...))
    let not_expr = ctx.list_value(vec![not_sym_val, condition_sym_val]);
    let unless_body = vec![if_sym_val, not_expr, cons_expr];
    let unless_macro = Callable {
        params: vec![condition_sym],
        is_variadic: true,
        body: unless_body,
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };
    let macro_value = ctx.macro_value(unless_macro);
    ctx.set_symbol(unless_sym, macro_value);


   // and - expands to nested ifs
    let empty_check = ctx.list_value(vec![empty_sym_val, forms_sym_val]);
    let count_check = ctx.list_value(vec![count_sym_val, forms_sym_val]);
    let one = ctx.number_value(1.0);
    let single_check = ctx.list_value(vec![eq_sym_val, count_check, one]);
    let first_form = ctx.list_value(vec![first_sym_val, forms_sym_val]);
    let rest_forms = ctx.list_value(vec![rest_sym_val, forms_sym_val]);
    let recursive_and = ctx.list_value(vec![and_sym_val, rest_forms]);

    // Build the innermost if first
    let inner_if = ctx.list_value(vec![
        if_sym_val,
        first_form,
        recursive_and,
        first_form
    ]);

    // Build the middle if
    let middle_if = ctx.list_value(vec![
        if_sym_val, 
        single_check, 
        first_form, 
        inner_if
    ]);

    // Build the outermost if (the complete expansion)
    let and_body = ctx.list_value(vec![
        if_sym_val, 
        empty_check, 
        true_sym_val,
        middle_if
    ]);

    let and_macro = Callable {
        params: vec![forms_sym],
        is_variadic: true,
        body: vec![and_body], // Single expansion expression
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };

    let macro_value = ctx.macro_value(and_macro);
    ctx.set_symbol(and_sym, macro_value);


    // or - expands to nested ifs: (if (empty? forms) nil (if (first forms) (first forms) (or (rest forms))))
    let first_form = ctx.list_value(vec![first_sym_val, forms_sym_val]);
    let rest_forms = ctx.list_value(vec![rest_sym_val, forms_sym_val]);
    let recursive_or = ctx.list_value(vec![or_sym_val, rest_forms]);
    
    let inner_or_if = ctx.list_value(vec![
        if_sym_val,
        first_form.clone(),  // condition
        first_form,          // then (return the truthy value)
        recursive_or         // else (recurse on rest)
    ]);
    
    let or_body = ctx.list_value(vec![
        if_sym_val,
        empty_check,
        nil_sym_val,
        inner_or_if
    ]);

    let or_macro = Callable {
        params: vec![forms_sym],
        is_variadic: true,
        body: vec![or_body],
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };

    let macro_value = ctx.macro_value(or_macro);
    ctx.set_symbol(or_sym, macro_value);
    // cond - expands to nested ifs
    // defn - expands to (def name (fn ...))
    // -> and ->> - threading macros
}

pub fn register_complex_macros(ctx: &mut EvalContext) {
    
    let current_env = ctx.env.clone();
    
    // Pre-allocate all the symbols and values we'll need
    let if_sym_val = ctx.symbol_value("if");
    let cons_sym_val = ctx.symbol_value("cons");
    let list_sym_val = ctx.symbol_value("list");
    let first_sym_val = ctx.symbol_value("first");
    let rest_sym_val = ctx.symbol_value("rest");
    let empty_sym_val = ctx.symbol_value("empty?");
    let nil_sym_val = ctx.symbol_value("nil");
    let def_sym_val = ctx.symbol_value("def");
    let fn_sym_val = ctx.symbol_value("fn");
    let let_sym_val = ctx.symbol_value("let");
    let list_check_sym_val = ctx.symbol_value("list?");

    let cond_sym_val = ctx.symbol_value("cond");
    let clauses_sym_val = ctx.symbol_value("clauses");
    // cond macro - recursive expansion
    let cond_sym = ctx.get_symbol_id(cond_sym_val).unwrap();
    let clauses_sym = ctx.get_symbol_id(clauses_sym_val).unwrap();
    
    
    
    // Build the macro body step by step to avoid nested ctx borrows
    let empty_check = ctx.list_value(vec![empty_sym_val.clone(), clauses_sym_val.clone()]);
    let first_clause = ctx.list_value(vec![first_sym_val.clone(), clauses_sym_val.clone()]);
    let rest_clauses = ctx.list_value(vec![rest_sym_val.clone(), clauses_sym_val.clone()]);
    let second_clause = ctx.list_value(vec![first_sym_val.clone(), rest_clauses.clone()]);
    let remaining_clauses = ctx.list_value(vec![rest_sym_val.clone(), rest_clauses]);
    let recursive_cond = ctx.list_value(vec![cond_sym_val, remaining_clauses]);
    
    let inner_if = ctx.list_value(vec![
        if_sym_val.clone(),
        first_clause,
        second_clause,
        recursive_cond
    ]);
    
    let cond_body = ctx.list_value(vec![
        if_sym_val.clone(),
        empty_check,
        nil_sym_val.clone(),
        inner_if
    ]);

    let cond_macro = Callable {
        params: vec![clauses_sym],
        is_variadic: true,
        body: vec![cond_body],
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };

    let cond_macro_val = ctx.macro_value(cond_macro);
    ctx.set_symbol(cond_sym, cond_macro_val);

    // defn macro - simple expansion
    let defn_sym_val = ctx.symbol_value("defn");
    let name_sym_val = ctx.symbol_value("name");
    let args_sym_val = ctx.symbol_value("args");
    let body_sym_val = ctx.symbol_value("body");
    
    let defn_sym = ctx.get_symbol_id(defn_sym_val).unwrap();
    let name_sym = ctx.get_symbol_id(name_sym_val).unwrap();
    let args_sym = ctx.get_symbol_id(args_sym_val).unwrap();
    let body_sym = ctx.get_symbol_id(body_sym_val).unwrap();
    
    // Build step by step
    let cons_args_body = ctx.list_value(vec![cons_sym_val.clone(), args_sym_val, body_sym_val]);
    let fn_expr = ctx.list_value(vec![
        cons_sym_val.clone(),
        fn_sym_val,
        cons_args_body
    ]);
    
    let defn_body = ctx.list_value(vec![
        def_sym_val,
        name_sym_val,
        fn_expr
    ]);

    let defn_macro = Callable {
        params: vec![name_sym, args_sym],
        is_variadic: true,
        body: vec![defn_body],
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };

    let defn_macro_val = ctx.macro_value(defn_macro);
    ctx.set_symbol(defn_sym, defn_macro_val);

    // -> (thread-first) macro

    
    let x_sym_val = ctx.symbol_value("x");
    let forms_sym_val = ctx.symbol_value("forms");
    let form_sym_val = ctx.symbol_value("form");
    let rest_forms_sym_val = ctx.symbol_value("rest-forms");
    let threaded_sym_val = ctx.symbol_value("threaded");
    
    let thread_first_sym_val = ctx.symbol_value("->");

    let thread_first_sym = ctx.get_symbol_id(thread_first_sym_val).unwrap();
    let x_sym = ctx.get_symbol_id(x_sym_val).unwrap();
    let forms_sym = ctx.get_symbol_id(forms_sym_val).unwrap();
    let form_sym = ctx.get_symbol_id(form_sym_val).unwrap();
    let rest_forms_sym = ctx.get_symbol_id(rest_forms_sym_val).unwrap();
    let threaded_sym = ctx.get_symbol_id(threaded_sym_val).unwrap();
    
    // Build all the sub-expressions step by step
    let empty_forms_check = ctx.list_value(vec![empty_sym_val.clone(), forms_sym_val.clone()]);
    let first_forms = ctx.list_value(vec![first_sym_val.clone(), forms_sym_val.clone()]);
    let rest_forms_expr = ctx.list_value(vec![rest_sym_val.clone(), forms_sym_val.clone()]);
    let first_form = ctx.list_value(vec![first_sym_val.clone(), form_sym_val.clone()]);
    let rest_form = ctx.list_value(vec![rest_sym_val.clone(), form_sym_val.clone()]);
    let list_check = ctx.list_value(vec![list_check_sym_val, form_sym_val.clone()]);
    
    let cons_x_rest = ctx.list_value(vec![cons_sym_val.clone(), x_sym_val.clone(), rest_form]);
    let threaded_list = ctx.list_value(vec![cons_sym_val.clone(), first_form, cons_x_rest]);
    let simple_thread = ctx.list_value(vec![list_sym_val.clone(), form_sym_val.clone(), x_sym_val.clone()]);
    
    let threading_if = ctx.list_value(vec![
        if_sym_val.clone(),
        list_check,
        threaded_list,
        simple_thread
    ]);
    
    let recursive_thread = ctx.list_value(vec![
        thread_first_sym_val.clone(),
        threaded_sym_val.clone(),
        rest_forms_sym_val.clone()
    ]);
    
    let let_bindings = ctx.list_value(vec![
        form_sym_val, first_forms,
        rest_forms_sym_val, rest_forms_expr,
        threaded_sym_val, threading_if
    ]);
    
    let let_body = ctx.list_value(vec![
        let_sym_val.clone(),
        let_bindings,
        recursive_thread
    ]);
    
    let thread_first_body = ctx.list_value(vec![
        if_sym_val.clone(),
        empty_forms_check,
        x_sym_val.clone(),
        let_body
    ]);

    let thread_first_macro = Callable {
        params: vec![x_sym, forms_sym],
        is_variadic: true,
        body: vec![thread_first_body],
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };

    let thread_first_macro_val = ctx.macro_value(thread_first_macro);
    ctx.set_symbol(thread_first_sym, thread_first_macro_val);

    // ->> (thread-last) macro - similar but threads as last argument
    let thread_last_sym_val = ctx.symbol_value("->>");
    let thread_last_sym = ctx.get_symbol_id(thread_last_sym_val).unwrap();
    
    // ->> (thread-last) macro - similar but threads as last argument
    let thread_last_sym_val = ctx.symbol_value("->>");
    let concat_sym_val = ctx.symbol_value("concat"); // You'll need this function

    // Pre-build all the pieces
    let x_list = ctx.list_value(vec![list_sym_val.clone(), x_sym_val.clone()]);
    let form_plus_x = ctx.list_value(vec![concat_sym_val, form_sym_val.clone(), x_list]);
    let list_check_form = ctx.list_value(vec![list_check_sym_val.clone(), form_sym_val.clone()]);
    let simple_thread_last = ctx.list_value(vec![list_sym_val.clone(), form_sym_val.clone(), x_sym_val.clone()]);

    // Build the conditional for thread-last
    let thread_last_if = ctx.list_value(vec![
        if_sym_val.clone(),
        list_check_form,
        form_plus_x,
        simple_thread_last
    ]);
    
    let thread_last_recursive = ctx.list_value(vec![
        thread_last_sym_val.clone(),
        threaded_sym_val.clone(),
        rest_forms_sym_val.clone()
    ]);
    
    let thread_last_let_bindings = ctx.list_value(vec![
        form_sym_val.clone(), first_forms.clone(),
        rest_forms_sym_val.clone(), rest_forms_expr.clone(),
        threaded_sym_val.clone(), thread_last_if
    ]);
    
    let thread_last_let_body = ctx.list_value(vec![
        let_sym_val.clone(),
        thread_last_let_bindings,
        thread_last_recursive
    ]);
    
    let thread_last_body = ctx.list_value(vec![
        if_sym_val,
        empty_forms_check.clone(),
        x_sym_val.clone(),
        thread_last_let_body
    ]);

    let thread_last_macro = Callable {
        params: vec![x_sym, forms_sym],
        is_variadic: true,
        body: vec![thread_last_body],
        env: Arc::new(RwLock::new(Env::with_parent(current_env.clone()))),
    };

    let thread_last_macro_val = ctx.macro_value(thread_last_macro);
    ctx.set_symbol(thread_last_sym, thread_last_macro_val);
}