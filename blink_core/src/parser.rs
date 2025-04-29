use parking_lot::RwLock;

use crate::env::Env;
use crate::error::{LispError, SourcePos};
use crate::eval::EvalContext;
use crate::value::{bool_val_at, keyword_at, list_val, num_at, str_val, sym, sym_at, BlinkValue};
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
            // If symbol exists in the macro local env, replace it
            if let Some(val) = env.read().get(s) {
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
    println!("Parsing atom: {}", token);

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
    rcx: &mut ReaderContext,
) -> Result<BlinkValue, LispError> {
    if tokens.is_empty() {
        return Err(LispError::UnexpectedToken {
            pos: SourcePos { line: 0, col: 0 },
            token: "EOF".into(),
        });
    }

    let (token, pos) = tokens.remove(0);

    match token.as_str() {
        "(" => {
            let mut list = Vec::new();
            while let Some((tok, _)) = tokens.first() {
                if tok == ")" {
                    tokens.remove(0); // consume ')'
                    return Ok(list_val(list));
                }
                list.push(parse(tokens, rcx)?); // <-- FIX
            }
            Err(LispError::ParseError {
                message: "Unclosed list, missing ')'".into(),
                pos,
            })
        }

        "[" => {
            let mut elements = Vec::new();
            while let Some((t, _)) = tokens.first() {
                if t == "]" {
                    tokens.remove(0); // consume ]
                    return Ok(list_val(
                        std::iter::once(sym("vector"))
                            .chain(elements.into_iter())
                            .collect(),
                    ));
                }
                elements.push(parse(tokens, rcx)?); // <-- FIX
            }
            Err(LispError::ParseError {
                message: "Unclosed vector literal".into(),
                pos,
            })
        }

        "{" => {
            let mut entries = Vec::new();
            while let Some((t, _)) = tokens.first() {
                if t == "}" {
                    tokens.remove(0); // consume }
                    return Ok(list_val(
                        std::iter::once(sym("hash-map"))
                            .chain(entries.into_iter())
                            .collect(),
                    ));
                }
                entries.push(parse(tokens, rcx)?); // <-- FIX
                entries.push(parse(tokens, rcx)?); // <-- FIX
            }
            Err(LispError::ParseError {
                message: "Unclosed map literal".into(),
                pos,
            })
        }

        ")" | "]" | "}" => Err(LispError::UnexpectedToken { token, pos }),

        _ => {
            // Check for reader macros by full prefix matching
            let mut matched_macro: Option<(String, BlinkValue)> = None;

            for (prefix, macro_fn) in rcx.reader_macros.iter() {
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

            if let Some((prefix, macro_fn)) = matched_macro {
                let rest = &token[prefix.len()..];

                // Special case: if rest is empty, must parse next token
                let target_form = if rest.is_empty() {
                    if tokens.is_empty() {
                        return Err(LispError::ParseError {
                            message: "Unexpected EOF after reader macro".into(),
                            pos,
                        });
                    }
                    parse(tokens, rcx)?
                } else {
                    let mut rest_tokens = tokenize(&rest)?;
                    println!("Rest tokens: {:?}", rest_tokens);

                    parse(&mut rest_tokens, rcx)?
                };

                return apply_reader_macro(macro_fn, target_form);
            }

            Ok(atom(&token, Some(pos)))
        }
    }
}

pub fn parse_all(code: &str) -> Result<Vec<BlinkValue>, LispError> {
    let mut tokens = tokenize(code)?;
    let mut forms = Vec::new();
    let mut reader_ctx = ReaderContext::new();

    while !tokens.is_empty() {
        let form = parse(&mut tokens, &mut reader_ctx)?;
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

    // (optional later: I could add ~@ for unquote-splicing if tokenized specially)
}
