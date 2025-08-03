
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{BlinkError, BlinkErrorType};
use crate::runtime::{EvalResult, GLOBAL_VM};
use crate::value::{unpack_immediate, ImmediateValue, NativeContext, ValueRef};


pub fn native_add(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.number(0.0)); // Additive identity
    }
    
    let mut sum = 0.0;
    for arg in args {
        if let Some(val) = ctx.get_number(arg) {
            sum += val;
        } else {
            return EvalResult::Value(ctx.eval_error(&format!("+ expects numbers, got {}", arg.type_tag())));
        }
    }
    EvalResult::Value(ctx.number(sum))
}

pub fn native_sub(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.number(0.0)); // Subtractive identity
    }
    if args.len() == 1 {
        // Unary minus: (- x) => -x
        if let Some(val) = ctx.get_number(args[0]) {
            return EvalResult::Value(ctx.number(-val));
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
    EvalResult::Value(ctx.number(result))
}

pub fn native_mul(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.is_empty() {
        return EvalResult::Value(ctx.number(1.0)); // Multiplicative identity
    }
    
    let mut product = 1.0;
    for arg in args {
        if let Some(val) = ctx.get_number(arg) {
            product *= val;
        } else {
            return EvalResult::Value(ctx.eval_error(&format!("* expects numbers, got {}", arg.type_tag())));
        }
    }
    EvalResult::Value(ctx.number(product))
}

pub fn native_div(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
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
    EvalResult::Value(ctx.number(val))
}

pub fn native_eq(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    let result = if let Some((first, rest)) = args.split_first() {
        rest.iter().all(|arg| arg == first)
    } else {
        true
    };

    EvalResult::Value(ctx.bool(result))
}

pub fn native_not(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    
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
    EvalResult::Value(ctx.bool(result))
}



pub fn native_list(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    EvalResult::Value(ctx.list(args))
}

pub fn native_vector(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    EvalResult::Value(ctx.vector(args))
}

pub fn native_map_construct(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() % 2 != 0 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "map"));
    }

    let pairs = args.chunks(2).map(|chunk| (chunk[0], chunk[1])).collect();

    EvalResult::Value(ctx.hash_map(pairs))
    
    
}

pub fn native_print(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    for val in args {
        print!("{} ", val);
    }
    println!();
    EvalResult::Value(ctx.nil())
}

pub fn native_type_of(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "type-of"));
    }

    let arg = &args[0];
    let type_name = arg.type_name();

    EvalResult::Value(ctx.string(type_name))
}

pub fn native_cons(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "cons"));
    }
    let mut new_list = vec![args[1]];
    let old_list =  if let Some(v)  = args[0].get_list(){
        v 
    } else if let Some(v) = args[1].get_vec(){
        v
    } else {
        return EvalResult::Value(ctx.eval_error("second argument to cons must be a list or vector"));
    };
    new_list.extend(old_list);
    EvalResult::Value(ctx.list(new_list))
}

pub fn native_concat(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    
    
    let mut lists = vec![];
    for arg in args {
        if let Some(v) = arg.get_list(){
            lists.push(v);
        } else if let Some(v) = arg.get_vec(){
            lists.push(v);
        }
    }
    let new_list = lists.concat();
    
    EvalResult::Value(ctx.list(new_list))
}

pub fn native_first(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "first"));
    }
    
    match args[0].get_list() {
        Some(list) => {
            if list.is_empty() {
                EvalResult::Value(ctx.nil())
            } else {
                EvalResult::Value(list[0])
            }
        },
        None => match args[0].get_vec() {
            Some(vec) => {
                if vec.is_empty() {
                    EvalResult::Value(ctx.nil())
                } else {
                    EvalResult::Value(vec[0])
                }
            },
            None => EvalResult::Value(ctx.eval_error("first expects a list or vector"))
        }
    }
}

pub fn native_rest(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(1, args.len(), "rest"));
    }
    
    match args[0].get_list() {
        Some(list) => {
            let rest_items: Vec<ValueRef> = list.iter().skip(1).cloned().collect();
            EvalResult::Value(ctx.list(rest_items))
        },
        None => match args[0].get_vec() {
            Some(vec) => {
                let rest_items: Vec<ValueRef> = vec.iter().skip(1).cloned().collect();
                EvalResult::Value(ctx.list(rest_items)) // Note: returns list, not vector
            },
            None => EvalResult::Value(ctx.eval_error("rest expects a list or vector"))
        }
    }
}

pub fn native_empty_q(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
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
    
    EvalResult::Value(ctx.bool(is_empty))
}

pub fn native_count(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
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
    
    EvalResult::Value(ctx.number(count as f64))
}

pub fn native_gc_stress(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 1 {
        return EvalResult::Value(ctx.arity_error(0, args.len(), "gc-stress"));
    }

    let n = ctx.get_number(args[0]);
    if n.is_none() {
        return EvalResult::Value(ctx.eval_error("gc-stress expects a number"));
    }
    let n = n.unwrap() as usize;

    for _ in 0..n {
        let mut strings = Vec::new();
        for _ in 0..1000 {
            let str = ctx.string("hello this is a long string, so very long abcdefg hleloa asd adsg asf as asd asd as das asd adsa sdasdssdaf dsfdsas as ada sda sdasd asd asd asd asfd agdasd asf ");
            
            strings.push(str);
        }
    
        let x = ctx.list(strings);
    }
    
    EvalResult::Value(ValueRef::nil())
}

pub fn native_report_gc_stats(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 0 {
        return EvalResult::Value(ctx.arity_error(0, args.len(), "report-gc-stats"));
    }
    
    let vm = GLOBAL_VM.get().unwrap().clone();
    vm.print_gc_stats();
    EvalResult::Value(ValueRef::nil())
}

pub fn native_get(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
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
                return EvalResult::Value(ctx.nil());
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
                return EvalResult::Value(ctx.nil());
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
            return EvalResult::Value(ctx.nil());
        }
    } else {
        return EvalResult::Value(ctx.eval_error("get expects a list, vector, or map"));
    }
}


pub fn native_future(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 0 {
        return EvalResult::Value(ctx.arity_error(0, args.len(), "future"));
    }

    EvalResult::Value(ctx.future())
}

pub fn native_complete_future(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
    if args.len() != 2 {
        return EvalResult::Value(ctx.arity_error(2, args.len(), "complete"));
    }
    if let Some(future) = args[0].get_future(){
        future.complete(args[1]).map_err(|e| BlinkError::eval(e.to_string()));
        return EvalResult::Value(ctx.nil());
    } else {
        return EvalResult::Value(ctx.eval_error("complete expects a future"));
    }
}

pub fn native_error(args: Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult {
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
    EvalResult::Value(ctx.error(error))
}

