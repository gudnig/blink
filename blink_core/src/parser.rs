use parking_lot::RwLock;

use crate::env::Env;
use crate::error::LispError;
use crate::eval::EvalContext;
use crate::value::{bool_val_at, keyword_at, list_val, map_val_at, num_at, str_val, sym, sym_at, vector_val_at, BlinkValue, SourcePos, SourceRange};
use crate::value::{LispNode, Value};

use std::collections::HashMap;

use std::sync::Arc;

pub struct ReaderContext {
    pub reader_macros: HashMap<String, BlinkValue>,
}

impl ReaderContext {
    pub fn new() -> Self {
        ReaderContext {
            reader_macros: HashMap::new(),
        }
    }
}

fn expand_macro_body(form: BlinkValue, env: Arc<RwLock<Env>>) -> Result<BlinkValue, LispError> {
    match &form.read().value {
        Value::Symbol(s) => {
            let available_modules = env.read().available_modules.clone();
            // If symbol exists in the macro local env, replace it
            if let Some(val) = env.read().get_local(s) {
                Ok(val)
            } else {
                Ok(form.clone()) // leave symbol unchanged
            }
        }

        Value::List(forms) => {
            // Recurse into each form inside the list
            let mut expanded = Vec::new();
            for f in forms {
                expanded.push(expand_macro_body(f.clone(), env.clone())?);
            }
            Ok(list_val(expanded))
        }

        // All other literals stay as-is
        _ => Ok(form.clone()),
    }
}

fn apply_reader_macro(macro_fn: BlinkValue, form: BlinkValue) -> Result<BlinkValue, LispError> {
    match &macro_fn.read().value {
        Value::FuncUserDefined { params, body, env } => {
            if params.len() != 1 {
                return Err(LispError::EvalError {
                    message: "Reader macro must take exactly 1 argument".into(),
                    pos: None,
                });
            }

            // Create fresh local env
            let local_env = Arc::new(RwLock::new(Env::with_parent(env.clone())));

            // Bind param "x" -> the parsed form (like "foo")
            local_env.write().set(&params[0], form);

            let macro_body = body.get(0).ok_or_else(|| LispError::EvalError {
                message: "Reader macro has no body".into(),
                pos: None,
            })?;

            // Expand macro body inside this local env
            expand_macro_body(macro_body.clone(), local_env)
        }
        Value::NativeFunc(f) => f(vec![form]).map_err(|e| LispError::EvalError {
            message: e,
            pos: None,
        }),
        _ => Err(LispError::EvalError {
            message: "Reader macro must be a function".into(),
            pos: None,
        }),
    }
}

pub fn tokenize(code: &str) -> Result<Vec<(String, SourcePos)>, LispError> {
    tokenize_at(code, None)
}

pub fn tokenize_at(
    code: &str,
    pos: Option<SourcePos>,
) -> Result<Vec<(String, SourcePos)>, LispError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_str = false;

    let mut line = 1;
    let mut col = 0;

    if let Some(pos) = pos {
        line = pos.line;
        col = pos.col;
    }

    for c in code.chars() {
        col += 1;
        if c == '\n' {
            line += 1;
            col = 0;
        }

        match c {
            '(' | ')' | '[' | ']' | '{' | '}' => {
                if !current.is_empty() {
                    
                    tokens.push((current.clone(), SourcePos { line, col }));
                    current.clear();
                }
                tokens.push((c.to_string(), SourcePos { line, col }));
            }
            '"' => {
                current.push(c);
                if in_str {
                    tokens.push((current.clone(), SourcePos { line, col }));
                    current.clear();
                }
                in_str = !in_str;
            }
            c if c.is_whitespace() && !in_str => {
                if !current.is_empty() {
                    tokens.push((current.clone(), SourcePos { line, col }));
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }

    if in_str {
        return Err(LispError::TokenizerError {
            message: "Unterminated string literal".into(),
            pos: SourcePos { line, col },
        });
    }

    if !current.is_empty() {
        tokens.push((current, SourcePos { line, col }));
    }

    Ok(tokens)
}

pub fn atom(token: &str, pos: Option<SourcePos>) -> BlinkValue {
    

    if token.starts_with('"') && token.ends_with('"') {
        str_val(&token[1..token.len() - 1])
    } else if let Ok(n) = token.parse::<f64>() {
        num_at(n, pos)
    } else if token == "true" {
        bool_val_at(true, pos)
    } else if token == "false" {
        bool_val_at(false, pos)
    } else if token.starts_with(':') {
        keyword_at(&token[1..], pos)
    } else {
        sym_at(token, pos)
    }
}

pub fn parse(
    tokens: &mut Vec<(String, SourcePos)>,
    rcx: &mut Arc<RwLock<ReaderContext>>,
) -> Result<BlinkValue, LispError> {
    if tokens.is_empty() {
        return Err(LispError::UnexpectedToken {
            pos: SourcePos { line: 0, col: 0 },
            token: "EOF".into(),
        });
    }

    let (token, start) = tokens.remove(0);

    match token.as_str() {
        "(" => {
            let mut list = Vec::new();
            
            let mut end = start.clone();

            while let Some((tok, next_pos)) = tokens.first() {
                if tok == ")" {
                    end = next_pos.clone(); // position of ')'
                    tokens.remove(0); // consume ')'
                    break;
                }
                let item = parse(tokens, rcx)?;
                if let Some(item_end) = item.read().pos.as_ref().map(|r| r.end.clone()) {
                    end = item_end;
                }
                list.push(item);
            }

            let range = SourceRange { start, end };
            Ok(BlinkValue(Arc::new(RwLock::new(LispNode {
                value: Value::List(list),
                pos: Some(range),
            }))))
        }


        "[" => {
            let mut elements = Vec::new();
            
            let mut end = start.clone();
            while let Some((t, next_pos)) = tokens.first() {
                if t == "]" {
                    end = next_pos.clone(); // position of ']'
                    tokens.remove(0); // consume ]

                    return Ok(vector_val_at(elements, Some(start)));
                }
                let item = parse(tokens, rcx)?;
                if let Some(item_end) = item.read().pos.as_ref().map(|r| r.end.clone()) {
                    end = item_end;
                }
                elements.push(item); 
            }
            let pos = SourceRange { start: start.clone(), end };

            Err(LispError::ParseError {
                message: "Unclosed vector literal".into(),
                pos,
            })
        }

        "{" => {
            let mut entries: Vec<BlinkValue> = Vec::new();
            let mut end = start.clone();
            
            while let Some((t, next_pos)) = tokens.first() {
                if t == "}" {
                    end = next_pos.clone(); // position of '}'
                    tokens.remove(0); // consume }
                    
                    // Convert entries to HashMap
                    if entries.len() % 2 != 0 {
                        return Err(LispError::ParseError {
                            message: "Map literal must have even number of elements (key-value pairs)".into(),
                            pos: SourceRange { start: start.clone(), end },
                        });
                    }
                    
                    let mut map = HashMap::new();
                    for pair in entries.chunks(2) {
                        let key = pair[0].clone();
                        let value = pair[1].clone();
                        map.insert(key, value);
                    }
                    
                    return Ok(BlinkValue(Arc::new(RwLock::new(LispNode {
                        value: Value::Map(map),
                        pos: Some(SourceRange { start, end }),
                    }))));
                }
                
                // Parse one element (key or value)
                let element = parse(tokens, rcx)?;
                if let Some(element_end) = element.read().pos.as_ref().map(|r| r.end.clone()) {
                    end = element_end;
                }
                entries.push(element);
            }
            
            // If we get here, we never found the closing }
            let pos = SourceRange { start: start.clone(), end };
            Err(LispError::ParseError {
                message: "Unclosed map literal".into(),
                pos,
            })
        }

        

        ")" | "]" | "}" => Err(LispError::UnexpectedToken { token, pos: start }),

        _ => {
            let mut matched_macro: Option<(String, BlinkValue)> = None;
            {
                let rcx_read = rcx.read();
                for (prefix, macro_fn) in rcx_read.reader_macros.iter() {
                    if token.starts_with(prefix) {
                        if let Some((best_prefix, _)) = &matched_macro {
                            if prefix.len() > best_prefix.len() {
                                matched_macro = Some((prefix.clone(), macro_fn.clone()));
                            }
                        } else {
                            matched_macro = Some((prefix.clone(), macro_fn.clone()));
                        }
                    }
                }
            }
        
            if let Some((prefix, macro_fn)) = matched_macro {
                let rest = &token[prefix.len()..];
            
                let target_form = if rest.is_empty() {
                    if tokens.is_empty() {
                        return Err(LispError::ParseError {
                            message: "Unexpected EOF after reader macro".into(),
                            pos: SourceRange {
                                start: start.clone(),
                                end: start.clone(),
                            },
                        });
                    }
                    parse(tokens, rcx)?
                } else {
                    let mut rest_tokens = tokenize(rest)?;
                    parse(&mut rest_tokens, rcx)?
                };
            
                let node = apply_reader_macro(macro_fn, target_form.clone())?;
            
                let target_range = target_form.read().pos.clone();
                let end = target_range.map(|r| r.end).unwrap_or_else(|| start.clone());
            
                node.write().pos = Some(SourceRange {
                    start: start.clone(),
                    end,
                });
            
                return Ok(node);
            }
            
        
            Ok(atom(&token, Some(start)))
        }
    }
}

pub fn parse_all(code: &str, ctx: &mut Arc<RwLock<ReaderContext>>) -> Result<Vec<BlinkValue>, LispError> {
    let mut tokens = tokenize(code)?;
    let mut forms = Vec::new();

    while !tokens.is_empty() {
        let form = parse(&mut tokens, ctx)?;
        forms.push(form);
    }

    Ok(forms)
}

pub fn preload_builtin_reader_macros(ctx: &mut EvalContext) {
    fn build_simple_macro(name: &str) -> BlinkValue {
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::FuncUserDefined {
                params: vec!["x".to_string()],
                body: vec![list_val(vec![
                    sym(name), // 'quote, 'quasiquote, etc.
                    sym("x"),  // just reference the param symbol "x" at runtime
                ])],
                env: Arc::new(RwLock::new(Env::new())), // empty environment
            },
            pos: None,
        })))
    }

    let mut rm = ctx.reader_macros.write();

    // Single character reader macros
    rm.reader_macros
        .insert("\'".into(), build_simple_macro("quo"));
    rm.reader_macros
        .insert("`".into(), build_simple_macro("quasiquote"));
    rm.reader_macros
        .insert("~".into(), build_simple_macro("unquote"));

    rm.reader_macros
    .insert("~@".into(), build_simple_macro("unquote-splicing"));

    rm.reader_macros
    .insert("@".into(), build_simple_macro("deref"));

    
}
