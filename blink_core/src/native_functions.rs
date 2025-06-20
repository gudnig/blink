
use crate::value::bool_val;
use crate::env::Env;
use crate::error::{BlinkError};
use crate::future::BlinkFuture;
use crate::value::{
     bool_val_at, list_val, list_val_at, map_val_at, nil, num_at, str_val_at,
    vector_val_at, BlinkValue, Value,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

pub fn native_add(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    let sum: f64 = args
        .into_iter()
        .map(|arg| {
            let node = arg.read();
            match &node.value {
                Value::Number(n) => Ok(*n),
                _ => Err("+ expects numbers".to_string()),
            }
        })
        .collect::<Result<Vec<f64>, _>>().map_err(|e| BlinkError::eval(e.to_string()))?
        .into_iter()
        .sum();

    Ok(num_at(sum, pos.map(|pos| pos.start)))
}

pub fn native_sub(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    let mut nums: Vec<f64> = args
        .into_iter()
        .map(|v| {
            let node = v.read();
            match &node.value {
                Value::Number(n) => Ok(*n),
                _ => Err("- expects numbers".to_string()),
            }
        })
        .collect::<Result<_, _>>().map_err(|e| BlinkError::eval(e.to_string()))?;

    let first = nums.remove(0);
    let result = if nums.is_empty() {
        -first
    } else {
        nums.into_iter().fold(first, |a, b| a - b)
    };

    Ok(num_at(result, pos.map(|pos| pos.start)))
}

pub fn native_mul(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    let mut product = 1.0;
    for arg in args {
        let node = arg.read();
        match &node.value {
            Value::Number(n) => product *= n,
            _ => return Err(BlinkError::eval("* expects numbers")),
        }
    }
    Ok(num_at(product, pos.map(|pos| pos.start)))
}

pub fn native_div(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    let mut nums: Vec<f64> = args
        .into_iter()
        .map(|v| {
            let node = v.read();
            match &node.value {
                Value::Number(n) => Ok(*n),
                _ => Err("/ expects numbers".to_string()),
            }
        })
        .collect::<Result<_, _>>().map_err(|e| BlinkError::eval(e.to_string()))?;

    let first = nums.remove(0);
    let result = if nums.is_empty() {
        1.0 / first
    } else {
        nums.into_iter().fold(first, |a, b| a / b)
    };
    Ok(num_at(result, pos.map(|pos| pos.start)))
}

pub fn native_eq(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    
    if args.len() < 2 {
        return Ok(bool_val_at(true, pos.map(|pos| pos.start)));
    }

    // Check if all args are equal to the first
    let all_equal = args.windows(2).all(|pair| pair[0] == pair[1]);
    
    Ok(bool_val_at(all_equal, pos.map(|pos| pos.start)))
}
pub fn native_not(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    
    if args.len() != 1 {
        return Err(BlinkError::arity(1, args.len(), "not"));
    }
    let result = match &args[0].read().value {
        Value::Bool(b) => !*b,
        Value::Nil => true,
        _ => false,
    };
    Ok(bool_val(result))
}

pub fn native_map(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 2 {
        return Err(BlinkError::arity(2, args.len(), "map"));
    }
    let func = args[0].clone();
    let list = match &args[1].read().value {
        Value::List(xs) => xs.clone(),
        _ => return Err(BlinkError::eval("map expects a list as second argument")),
    };
    let mut results = Vec::new();
    for val in list {
        if let Value::NativeFunc(f) = &func.read().value {
            results.push(f(vec![val])?);
        } else {
            return Err(BlinkError::eval("map only works on native functions for now"));
        }
    }
    Ok(list_val(results))
}



pub fn native_list(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    Ok(list_val(args))
}

pub fn native_vector(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    let pos = args.get(0).and_then(|v| v.read().pos.clone());
    Ok(vector_val_at(args, pos.map(|pos| pos.start)))
}

pub fn native_map_construct(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() % 2 != 0 {
        return Err(BlinkError::eval("map expects an even number of arguments"));
    }

    let pos = args.get(0).and_then(|v| v.read().pos.clone());

    let mut map = HashMap::new();
    let mut it = args.into_iter();

    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        map.insert(k, v);
    }

    Ok(map_val_at(map, pos.map(|pos| pos.start)))
}

pub fn native_print(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    for val in args {
        print!("{} ", val.read().value);
    }
    println!();
    Ok(nil())
}

pub fn native_type_of(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 1 {
        return Err(BlinkError::arity(1, args.len(), "type-of"));
    }

    let arg = &args[0];
    let type_name = arg.read().value.type_tag();
    let pos = arg.read().pos.clone();

    Ok(str_val_at(type_name, pos.map(|pos| pos.start)))
}

pub fn native_cons(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 2 {
        return Err(BlinkError::arity(2, args.len(), "cons"));
    }
    let mut new_list = vec![args[0].clone()];
    match &args[1].read().value {
        Value::List(rest) => new_list.extend(rest.clone()),
        _ => return Err(BlinkError::eval("second argument to cons must be a list")),
    }
    Ok(list_val(new_list))
}

pub fn native_car(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 1 {
        return Err(BlinkError::arity(1, args.len(), "car"));
    }

    let arg_ref = args[0].read();
    match &arg_ref.value {
        Value::List(xs) => xs.get(0).cloned().ok_or_else(|| BlinkError::eval("car on empty list")),
        _ => Err(BlinkError::eval("car expects a list")),
    }
}

pub fn native_cdr(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 1 {
        return Err(BlinkError::arity(1, args.len(), "cdr"));
    }

    let arg_ref = args[0].read();
    match &arg_ref.value {
        Value::List(xs) => Ok(list_val(xs.iter().skip(1).cloned().collect())),
        _ => Err(BlinkError::eval("cdr expects a list")),
    }
}

pub fn native_get(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(BlinkError::arity(2, args.len(), "get"));
    }

    let target_val = &args[0];
    let key_val = &args[1];
    let fallback_val = args.get(2).cloned();

    let key_pos = key_val.read().pos.clone(); // for potential error reporting
    let target_ref = target_val.read();
    let target = &target_ref.value;

    match target {
        Value::Vector(vec) => {
            if let Value::Number(n) = key_val.read().value {
                let idx = n.clone() as usize;
                if let Some(val) = vec.get(idx).cloned() {
                    Ok(val)
                } else if let Some(default) = fallback_val {
                    Ok(default)
                } else {
                    Err(BlinkError::eval(format!("Index {} out of bounds{}", idx, key_pos.map(|p| format!(" at {}", p)).unwrap_or_default())))
                }
            } else {
                Err(BlinkError::eval("get on vector expects numeric index"))
            }
        }

        Value::Map(map) => {
            

            if let Some(val) = map.get(key_val) {
                Ok(val.clone())
            } else if let Some(default) = fallback_val {
                Ok(default)
            } else {
                Err(BlinkError::eval(format!("Key '{}' not found in map{}", key_val, key_pos.map(|p| format!(" at {}", p)).unwrap_or_default())))
            }
        }

        _ => Err(BlinkError::eval("get only works on vector or map")),
    }
}

fn native_future(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 0 {
        return Err(BlinkError::arity(0, args.len(), "future"));
    }
    Ok(BlinkValue(Arc::new(RwLock::new(LispNode {
        value: Value::Future(BlinkFuture::new()),
        pos: None,
    }))))
}

fn native_complete_future(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 2 {
        return Err(BlinkError::arity(2, args.len(), "complete"));
    }
    match &args[0].read().value {
        Value::Future(future) => {
            future.complete(args[1].clone()).map_err(|e| BlinkError::eval(e.to_string()))?;
            Ok(nil())
        }
        _ => Err(BlinkError::eval("complete expects a future")),
    }
}

fn native_fail_future(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 2 {
        return Err(BlinkError::arity(2, args.len(), "fail"));
    }
    match (&args[0].read().value, &args[1].read().value)     {
        (Value::Future(future), Value::Str(s)) => {
            future.fail(s.clone()).map_err(|e| BlinkError::eval(e.to_string()))?;
            Ok(nil())
        }
        _ => Err(BlinkError::eval("fail expects a future")),
    }
}

fn native_error(args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
    if args.len() != 1 {
        return Err(BlinkError::arity(1, args.len(), "error"));
    }
    Ok(args[0].clone())
}

use crate::value::LispNode;

pub fn register_builtins(env: &Arc<RwLock<Env>>) {
    let mut e = env.write();

    macro_rules! reg {
        ($name:expr, $func:expr) => {
            e.set(
                $name,
                BlinkValue(Arc::new(RwLock::new(LispNode {
                    value: Value::NativeFunc($func),
                    pos: None,
                    
                }))),
            );
        };
    }

    reg!("+", native_add);
    reg!("-", native_sub);
    reg!("*", native_mul);
    reg!("/", native_div);
    reg!("=", native_eq);
    reg!("not", native_not);
    reg!("map", native_map);
    
    reg!("list", native_list);
    reg!("vector", native_vector);
    reg!("hash-map", native_map_construct);
    reg!("print", native_print);
    reg!("type-of", native_type_of);
    reg!("cons", native_cons);
    reg!("car", native_car);
    reg!("cdr", native_cdr);
    reg!("first", native_car);
    reg!("rest", native_cdr);
    reg!("get", native_get);    

    // TODO: Error module
    reg!("err", native_error);


    // TODO: async module
    reg!("future", native_future);
    reg!("complete", native_complete_future);
    reg!("fail", native_fail_future);
}
