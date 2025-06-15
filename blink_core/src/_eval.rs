use crate::async_context::AsyncContext;
use crate::env::Env;
use crate::error::{BlinkError};
use crate::eval::context::EvalContext;
use crate::future::BlinkFuture;
use crate::goroutine::{ TokioGoroutineScheduler};
use crate::metadata::ValueMetadataStore;
use crate::module::{ImportType, Module, ModuleRegistry, ModuleSource};
use crate::parser::{ ReaderContext};
use crate::shared_arena::SharedArena;
use crate::symbol_table::SymbolTable;
use crate::value::str_val;
use crate::telemetry::TelemetryEvent;
use crate::value::{bool_val, keyword_at, list_val, nil, LispNode, SourceRange, Value};
use crate::value_ref::{unpack_immediate, ImmediateValue, Macro, SharedValue, ValueRef};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

mod context;
mod helpers;




















fn eval_def_reader_macro(
    args: &[ValueRef],
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
fn eval_mod(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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
fn parse_flags(args: &[ValueRef]) -> (HashSet<String>, usize) {
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
fn update_module_exports(module: &mut Module, exports_val: &ValueRef) -> Result<(), BlinkError> {
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
fn get_string_option(options: &HashMap<String, ValueRef>, key: &str) -> Option<String> {
    options.get(key).and_then(|val| {
        match &val.read().value {
            Value::Str(s) => Some(s.clone()),
            Value::Symbol(s) => Some(s.clone()),
            _ => None,
        }
    })
}


/// Extract a name from a ValueRef
fn extract_name(value: &ValueRef) -> Result<String, BlinkError> {
    match &value.read().value {
        Value::Symbol(s) => Ok(s.clone()),
        Value::Str(s) => Ok(s.clone()),
        _ => Err(BlinkError::eval("Expected a symbol or string for name")),
    }
}


fn eval_file_forms_inline(
    forms: Vec<ValueRef>,
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

fn parse_import_args(args: &[ValueRef]) -> Result<(ImportType, Option<HashMap<String, ValueRef>>), BlinkError> {
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
    symbol_list: &[ValueRef], 
    remaining_args: &[ValueRef]
) -> Result<(ImportType, HashMap<String, ValueRef>), BlinkError> {
    
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

fn parse_mod_options(args: &[ValueRef]) -> Result<(HashMap<String, ValueRef>, usize), BlinkError> {
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






fn eval_imp(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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
fn eval_quasiquote(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
    if let Err(e) = require_arity(args, 1, "quasiquote") {
        return EvalResult::Value(e.into_blink_value());
    }
    expand_quasiquote(args[0].clone(), ctx)
}
fn expand_quasiquote(expr: ValueRef, ctx: &mut EvalContext) -> EvalResult {
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
    items: Vec<ValueRef>,  
    mut index: usize,
    mut expanded_items: Vec<ValueRef>,
    is_vector: bool,
    original_pos: Option<SourceRange>,
    ctx: &mut EvalContext,
) -> EvalResult {
    loop {
        if index >= items.len() {
            // Return the appropriate type
            if is_vector {
                return EvalResult::Value(ValueRef(Arc::new(RwLock::new(LispNode {
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


fn eval_macro(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

    EvalResult::Value(ValueRef(Arc::new(RwLock::new(LispNode {
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
fn eval_deref(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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

pub fn eval_go(args: &[ValueRef], ctx: &mut EvalContext) -> EvalResult {
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
