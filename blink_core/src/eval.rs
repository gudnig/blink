use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use libloading::Library;

use crate::value::{bool_val, list_val, nil, str_val, LispNode, BlinkValue, Value};
use crate::env::Env;
use crate::parser::parse_all;
use crate::error::{LispError, SourcePos};

pub struct EvalContext {
    pub env: Rc<RefCell<Env>>,
    pub current_module: Option<String>,
    pub plugins: HashMap<String, Rc<Library>>, // keyed by plugin name
}

impl EvalContext {
    pub(crate) fn new(p0: &mut Env) -> Self {
        EvalContext {
            env: Rc::new(RefCell::new(Env::with_parent(Rc::new(RefCell::new(p0.clone()))))),
            current_module: None,
            plugins: HashMap::new()
        }
    }

    pub fn get(&self, key: &str) -> Option<BlinkValue> {
        self.env.borrow().get(key)
    }

    pub fn set(&self, key: &str, val: BlinkValue) {
        self.env.borrow_mut().set(key, val)
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
fn get_pos(expr: &BlinkValue) -> Option<SourcePos> {
    expr.borrow().pos.clone()
}


pub fn eval(expr: BlinkValue, ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let pos = get_pos(&expr);

    match &expr.borrow().value {
        Value::Number(_)
        | Value::Bool(_)
        | Value::Str(_)
        | Value::Keyword(_)
        | Value::Nil => Ok(expr.clone()),

        Value::Symbol(sym) => {
            ctx.get(sym).ok_or_else(|| LispError::UndefinedSymbol {
                name: sym.clone(),
                pos,
            })
        }

        Value::List(list) if list.is_empty() => Ok(nil()),

        Value::List(list) => {
            let head = &list[0];
            let head_val = &head.borrow().value;

            match head_val {
                Value::Symbol(s) => match s.as_str() {
                    "quote" => {
                        require_arity(&list[1..], 1, "quote")?;
                        Ok(list[1].clone())
                    }

                    "if" => {
                        let cond_val = eval(list[1].clone(), ctx)?;
                        let result = match &cond_val.borrow().value {
                            Value::Bool(true) => eval(list[2].clone(), ctx),
                            _ => {
                                if list.len() > 3 {
                                    eval(list[3].clone(), ctx)
                                } else {
                                    Ok(nil())
                                }
                            }
                        };
                        result
                    }

                    "def" => {
                        require_arity(&list[1..], 2, "def")?;
                        if let Value::Symbol(name) = &list[1].borrow().value {
                            let value = eval(list[2].clone(), ctx)?;
                            let full_name = if let Some(ns) = &ctx.current_module {
                                format!("{}/{}", ns, name)
                            } else {
                                name.clone()
                            };
                            ctx.set(&full_name, value.clone());
                            Ok(value)
                        } else {
                            Err(LispError::EvalError {
                                message: "def expects a symbol as the first argument".into(),
                                pos,
                            })
                        }
                    },
                    "apply" => {
                        require_arity(&list[1..], 2, "apply")?;

                        let func_val = eval(list[1].clone(), ctx)?;
                        let evaluated_arglist = eval(list[2].clone(), ctx)?;

                        let args = match &evaluated_arglist.borrow().value {
                            Value::List(xs) => xs.clone(),
                            _ => {
                                return Err(LispError::EvalError {
                                    message: "apply expects a list as second argument".into(),
                                    pos,
                                });
                            }
                        };

                        eval_func_with_resolved_func(func_val, &args, ctx)
                    }

                    "fn" => special_fn(&list[1..], ctx),
                    "import" => special_import(&list[1..], ctx),
                    "do" => special_do(&list[1..], ctx),
                    "let" => special_let(&list[1..], ctx),
                    "and" => special_and(&list[1..], ctx),
                    "or" => special_or(&list[1..], ctx),
                    "try" => special_try(&list[1..], ctx),
                    "native-import" => special_native_import(&list[1..], ctx),
                    "compile-plugin" => special_compile_plugin(&list[1..], ctx),
                    _ => {
                        let func_val = eval(list[0].clone(), ctx)?;
                        eval_func_with_resolved_func(func_val, &list[1..], ctx)
                    }
                },

                _ => {
                    let func_val = eval(list[0].clone(), ctx)?;
                    eval_func_with_resolved_func(func_val, &list[1..], ctx)
                }
            }
        }

        _ => Err(LispError::EvalError {
            message: "Unknown expression type".into(),
            pos,
        }),
    }
}


pub fn safe_eval(expr: BlinkValue, ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    catch_unwind(AssertUnwindSafe(|| eval(expr, ctx))).unwrap_or_else(|e| {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown internal panic".to_string()
        };
        Err(LispError::EvalError {
            message: format!("Internal panic: {}", msg),
            pos: None,
        })
    })
}

pub fn eval_func_with_resolved_func(func_val: BlinkValue, args_raw: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    match &func_val.borrow().value {
        Value::NativeFunc(f) => {
            let args = args_raw
                .iter()
                .map(|arg| eval(arg.clone(), ctx))
                .collect::<Result<Vec<_>, _>>()?;

            f(args).map_err(|e| LispError::EvalError { message: e, pos: None })
        }

        Value::FuncUserDefined { params, body, env: closure_env } => {
            if params.len() != args_raw.len() {
                return Err(LispError::ArityMismatch {
                    expected: params.len(),
                    got: args_raw.len(),
                    form: "fn".into(),
                    pos: None,
                });
            }

            let args = args_raw
                .iter()
                .map(|arg| eval(arg.clone(), ctx))
                .collect::<Result<Vec<_>, _>>()?;

            let local_env = Rc::new(RefCell::new(Env::with_parent(Rc::clone(&closure_env))));
            for (name, val) in params.iter().zip(args.iter()) {
                local_env.borrow_mut().set(name, val.clone());
            }

            let mut result = nil();
            for form in body {
                result = eval(form.clone(), &mut EvalContext {
                    env: Rc::clone(&local_env),
                    current_module: ctx.current_module.clone(),
                    plugins: ctx.plugins.clone(),
                })?;
            }

            Ok(result)
        }

        _ => Err(LispError::EvalError {
            message: "First element is not a function".into(),
            pos: None,
        }),
    }
}

fn expect_vector_form(val: &BlinkValue, form_name: &str, pos: Option<SourcePos>) -> Result<Vec<BlinkValue>, LispError> {
    match &val.borrow().value {
        Value::Vector(vs) => Ok(vs.clone()),

        Value::List(vs) if !vs.is_empty() => {
            if let Value::Symbol(tag) = &vs[0].borrow().value {
                if tag == "vector" {
                    Ok(vs[1..].to_vec())
                } else {
                    Err(LispError::EvalError {
                        message: format!("{} expects a vector as its argument", form_name),
                        pos,
                    })
                }
            } else {
                Err(LispError::EvalError {
                    message: format!("{} expects a vector as its argument", form_name),
                    pos,
                })
            }
        }

        _ => Err(LispError::EvalError {
            message: format!("{} expects a vector as its argument", form_name),
            pos,
        }),
    }
}


fn special_fn(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let fn_pos = args.get(0).map(get_pos).flatten(); // Clone early

    if args.len() < 2 {
        return Err(LispError::ArityMismatch {
            expected: 2,
            got: args.len(),
            form: "fn".into(),
            pos: fn_pos.clone(),
        });
    }

    let param_list = expect_vector_form(&args[0], "fn", fn_pos.clone())?
        .into_iter()
        .map(|v| match &v.borrow().value {
            Value::Symbol(s) => Ok(s.clone()),
            _ => Err(LispError::EvalError {
                message: "Function parameters must be symbols".into(),
                pos: v.borrow().pos.clone(),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let body_forms = args[1..].to_vec();

    Ok(BlinkValue(Rc::new(RefCell::new(LispNode {
        value: Value::FuncUserDefined {
            params: param_list,
            body: body_forms,
            env: Rc::clone(&ctx.env),
        },
        pos: fn_pos,
    }))))
    
}



fn special_import(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let pos = args.get(0).map(get_pos).flatten();
    if args.len() != 1 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "import".into(),
            pos: None,
        });
    }

    let modname = match &args[0].borrow().value {
        Value::Str(s) => s.clone(),
        _ => return Err(LispError::EvalError {
            message: "import expects a string module name".into(),
            pos: None,
        }),
    };

    let path = format!("lib/{}.blink", modname);
    let code = fs::read_to_string(&path)
        .map_err(|_| LispError::EvalError {
            message: format!("Failed to read module file: {}", path),
            pos: None,
        })?;

    let forms = parse_all(&code)?;
    let old_module = ctx.current_module.clone();
    ctx.current_module = Some(modname.clone());

    for form in forms {
        eval(form, ctx)?;
    }

    ctx.current_module = old_module;
    Ok(list_val(vec![
        BlinkValue(Rc::new(RefCell::new(LispNode::new( Value::Keyword("import/success".into()), pos.clone())))),
        BlinkValue(Rc::new(RefCell::new(LispNode::new(Value::Str(modname), pos)))),
    ]))
}

fn special_let(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let let_pos = args.get(0).map(get_pos).flatten();

    if args.len() < 2 {
        return Err(LispError::EvalError {
            message: "let expects a binding vector and at least one body form".into(),
            pos: let_pos.clone(),
        });
    }

    let bindings = expect_vector_form(&args[0], "let", let_pos.clone())?;

    if bindings.len() % 2 != 0 {
        return Err(LispError::EvalError {
            message: "let bindings must come in pairs".into(),
            pos: let_pos.clone(),
        });
    }

    let local_env = Rc::new(RefCell::new(Env::with_parent(Rc::clone(&ctx.env))));

    for pair in bindings.chunks(2) {
        let key = &pair[0];
        let val_expr = &pair[1];
        let val = eval(val_expr.clone(), ctx)?;

        match &key.borrow().value {
            Value::Symbol(name) => local_env.borrow_mut().set(&name, val),
            _ => {
                return Err(LispError::EvalError {
                    message: "let binding name must be a symbol".into(),
                    pos: key.borrow().pos.clone(),
                });
            }
        }
    }

    let mut result = nil();
    for form in &args[1..] {
        result = eval(form.clone(), &mut EvalContext {
            env: Rc::clone(&local_env),
            current_module: ctx.current_module.clone(),
            plugins: ctx.plugins.clone(),
        })?;
    }

    Ok(result)
}


fn special_do(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let mut result = nil();
    for form in args {
        result = eval(form.clone(), ctx)?;
    }
    Ok(result)
}

pub fn special_and(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let mut last = bool_val(true);

    for arg in args {
        let val = eval(arg.clone(), ctx)?;
        let is_false = matches!(&val.borrow().value, Value::Bool(false) | Value::Nil);
        if is_false {
            return Ok(val);
        }
        last = val;
    }

    Ok(last)
}



pub fn special_or(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    for arg in args {
        let val = eval(arg.clone(), ctx)?;
        let is_truthy = !matches!(&val.borrow().value, Value::Bool(false) | Value::Nil);
        if is_truthy {
            return Ok(val);
        }
    }

    Ok(nil())
}


fn special_try(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let try_pos = args.get(0).map(get_pos).flatten();

    if args.len() != 2 {
        return Err(LispError::ArityMismatch {
            expected: 2,
            got: args.len(),
            form: "try".into(),
            pos: try_pos.clone(),
        });
    }

    let result = safe_eval(args[0].clone(), ctx);

    match result {
        Ok(val) => {
            // If it's a map with :error key, treat it as user-tagged error
            if let Value::Map(m) = &val.borrow().value {
                if m.contains_key(":error") {
                    return eval(
                        list_val(vec![args[1].clone(), val.clone()]),
                        ctx,
                    );
                }
            }
            Ok(val)
        }

        Err(err) => {
            // Convert LispError into a map: {:error "msg"}
            let err_map = std::collections::HashMap::from([(
                ":error".to_string(),
                str_val(&format!("{}", err)),
            )]);
            let err_val = Rc::new(RefCell::new(LispNode {
                value: Value::Map(err_map),
                pos: try_pos.clone(),
            }));

            eval(
                list_val(vec![args[1].clone(), BlinkValue(err_val)]),

                ctx,
            )
        }
    }
}

fn special_native_import(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    use libloading::{Library, Symbol};

    let pos = args.get(0).map(get_pos).flatten();
    let pos_clone = pos.clone();

    if args.len() != 1 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "native-import".into(),
            pos,
        });
    }

    let arg0_ref = args[0].borrow();
    let libname = match &arg0_ref.value {
        Value::Str(s) => s.clone(),
        _ => return Err(LispError::EvalError {
            message: "native-import expects a string".into(),
            pos,
        }),
    };

    let filename = format!("native/lib{}.so", libname);

    let lib = unsafe { Library::new(&filename) }
        .map_err(|e| LispError::EvalError {
            message: format!("Failed to load native lib '{}': {}", filename, e),
            pos,
        })?;

    let func: Symbol<unsafe extern "C" fn(&mut Env)> = unsafe {
        lib.get(b"blink_register")
    }.map_err(|e| LispError::EvalError {
        message: format!("Failed to find 'blink_register' in '{}': {}", filename, e),
        pos: pos_clone,
    })?;

    unsafe { func(&mut *ctx.env.borrow_mut()) };
    ctx.plugins.insert(libname.clone(), Rc::new(lib));


    Ok(str_val(&format!("native-imported: {}", libname)))
}

fn special_compile_plugin(args: &[BlinkValue], ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    use std::process::Command;
    use std::fs;
    use std::path::Path;

    let pos = args.get(0).map(|v| v.borrow().pos.clone()).flatten();

    let pos_clone = pos.clone();

    if args.len() < 1 || args.len() > 2 {
        return Err(LispError::ArityMismatch {
            expected: 1,
            got: args.len(),
            form: "compile-plugin".into(),
            pos,
        });
    }

    let name = match &args[0].borrow().value {
        Value::Str(s) => s.clone(),
        _ => return Err(LispError::EvalError {
            message: "compile-plugin expects a string as the first argument".into(),
            pos,
        }),
    };

    // Default options
    let mut plugin_path = format!("plugins/{}", name);
    let mut auto_import = false;
    let mut _reload = false; // placeholder
    let mut _verbose = false;

    // Parse optional map
    // Parse optional map (evaluated)
    if args.len() == 2 {
        let options_val = eval(args[1].clone(), ctx)?;

        {
            let borrowed = options_val.borrow();
            if let Value::Map(opt_map) = &borrowed.value {
                if let Some(path_val) = opt_map.get(":path").or_else(|| opt_map.get("path")) {
                    if let Value::Str(s) = &path_val.borrow().value {
                        plugin_path = s.clone();
                    }
                }
                if let Some(import_val) = opt_map.get(":import").or_else(|| opt_map.get("import")) {
                    if matches!(&import_val.borrow().value, Value::Bool(true)) {
                        auto_import = true;
                    }
                }
                if let Some(reload_val) = opt_map.get(":reload").or_else(|| opt_map.get("reload")) {
                    if matches!(&reload_val.borrow().value, Value::Bool(true)) {
                        _reload = true;
                    }
                }
                if let Some(verb_val) = opt_map.get(":verbose").or_else(|| opt_map.get("verbose")) {
                    if matches!(&verb_val.borrow().value, Value::Bool(true)) {
                        _verbose = true;
                    }
                }
            } else {
                return Err(LispError::EvalError {
                    message: "Second argument to compile-plugin must evaluate to a map".into(),
                    pos: pos_clone,
                });
            }
        }
        
    }


    if !Path::new(&plugin_path).exists() {
        return Err(LispError::EvalError {
            message: format!("Plugin path '{}' not found", plugin_path),
            pos: pos_clone,
        });
    }

    let build_status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&plugin_path)
        .status();

    match build_status {
        Ok(status) if status.success() => {
            let ext = if cfg!(target_os = "macos") {
                "dylib"
            } else if cfg!(target_os = "windows") {
                "dll"
            } else {
                "so"
            };
            
            let source = format!("{}/target/release/lib{}.{}", plugin_path, name, ext);
            
            let dest = format!("native/lib{}.so", name);

            fs::create_dir_all("native").ok();
            fs::copy(&source, &dest).map_err(|e| LispError::EvalError {
                message: format!("Failed to copy compiled plugin: {}", e),
                pos,
            })?;

            // optionally import
            if auto_import {
                special_native_import(&[str_val(&name)], ctx)?;
            }

            Ok(str_val(&format!(":compile-plugin/success [{}]", name)))
        }

        Ok(status) => Err(LispError::EvalError {
            message: format!("cargo build failed with status: {}", status),
            pos,
        }),

        Err(e) => Err(LispError::EvalError {
            message: format!("Failed to invoke cargo: {}", e),
            pos,
        }),
    }
}