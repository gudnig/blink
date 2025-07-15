use parking_lot::RwLock;

use crate::error::{BlinkError, ParseErrorType};
use crate::eval::EvalContext;
use crate::runtime::{BlinkVM, SymbolTable};
use crate::value::{Callable, ParsedValue, ParsedValueWithPos, SourcePos, SourceRange};
use crate::env::Env;
use crate::value::ValueRef;
use std::collections::HashMap;
use std::sync::Arc;


pub struct ReaderContext {
    pub reader_macros: HashMap<String, u32>, // prefix -> symbol_id
}

impl ReaderContext {
    pub fn new() -> Self {
        ReaderContext {
            reader_macros: HashMap::new(),
        }
    }
    
    pub fn add_reader_macro(&mut self, prefix: String, symbol_id: u32) {
        self.reader_macros.insert(prefix, symbol_id);
    }
}

pub fn tokenize(code: &str) -> Result<Vec<(String, SourcePos)>, BlinkError> {
    tokenize_at(code, None)
}

pub fn tokenize_at(
    code: &str,
    pos: Option<SourcePos>,
) -> Result<Vec<(String, SourcePos)>, BlinkError> {
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
        return Err(BlinkError::tokenizer("Unterminated string literal", SourcePos { line, col }));
    }

    if !current.is_empty() {
        tokens.push((current, SourcePos { line, col }));
    }

    Ok(tokens)
}

pub fn parse_symbol_token(token: &str, symbol_table: &mut SymbolTable) -> u32 {
    
    if let Some((module_part, symbol_part)) = token.split_once('/') {
        // Qualified symbol like "math/add"
        let module_id = symbol_table.intern(module_part);
        let symbol_id = symbol_table.intern(symbol_part);
        let id = symbol_table.intern_qualified(module_id, symbol_id);
    
        id  
    } else {
        // Simple symbol like "add"
        let id = symbol_table.intern(token);
    
        id
    }
}

// Trait for symbol table operations that the parser needs
pub trait SymbolTableTrait {
    fn intern(&mut self, name: &str) -> u32;
    fn intern_qualified(&mut self, module_id: u32, symbol_id: u32) -> u32;
}


fn apply_reader_macro(symbol_id: u32, form: ParsedValueWithPos) -> ParsedValueWithPos {
    let pos = form.pos;
    let symbol = ParsedValueWithPos { value: ParsedValue::Symbol(symbol_id), pos: pos };
    ParsedValueWithPos {
        value: ParsedValue::List(vec![symbol, form]),
        pos: pos,
    }
    
}

pub fn parse(
    tokens: &mut Vec<(String, SourcePos)>,
    reader_ctx: &ReaderContext,
    symbol_table: &mut SymbolTable,
) -> Result<ParsedValueWithPos, BlinkError> {
    if tokens.is_empty() {
        return Err(BlinkError::unexpected_token("EOF", SourcePos { line: 0, col: 0 }));
    }

    let (token, start_pos) = tokens.remove(0);

    match token.as_str() {
        "(" => {
            let mut list = Vec::new();
            let mut end_pos = start_pos;
            
            while let Some((tok, next_pos)) = tokens.first() {
                if tok == ")" {
                    end_pos = *next_pos;
                    tokens.remove(0); // consume ')'
                    break;
                }
                let item = parse(tokens, reader_ctx, symbol_table)?;
                // Update end position to the last item's end
                if let Some(item_end) = item.pos.as_ref().map(|r| r.end) {
                    end_pos = item_end;
                }
                list.push(item);
            }

            let range = SourceRange { start: start_pos, end: end_pos };
            Ok(ParsedValueWithPos::new(ParsedValue::List(list), Some(range)))
        }

        "[" => {
            let mut elements = Vec::new();
            let mut end_pos = start_pos;
            
            while let Some((t, next_pos)) = tokens.first() {
                if t == "]" {
                    end_pos = *next_pos;
                    tokens.remove(0); // consume ']'
                    
                    let range = SourceRange { start: start_pos, end: end_pos };
                    return Ok(ParsedValueWithPos::new(ParsedValue::Vector(elements), Some(range)));
                }
                let item = parse(tokens, reader_ctx, symbol_table)?;
                if let Some(item_end) = item.pos.as_ref().map(|r| r.end) {
                    end_pos = item_end;
                }
                elements.push(item); 
            }
            
            let pos = SourceRange { start: start_pos, end: end_pos };
            Err(BlinkError::parse_unclosed_delimiter("Unclosed vector literal", "[", pos))
        }

        "{" => {
            let mut pairs: Vec<(ParsedValueWithPos, ParsedValueWithPos)> = Vec::new();
            let mut end_pos = start_pos;
            
            while let Some((t, next_pos)) = tokens.first() {
                if t == "}" {
                    end_pos = *next_pos;
                    tokens.remove(0); // consume '}'
                    
                    let range = SourceRange { start: start_pos, end: end_pos };
                    return Ok(ParsedValueWithPos::new(ParsedValue::Map(pairs), Some(range)));
                }
                
                // Parse key
                let key = parse(tokens, reader_ctx, symbol_table)?;
                if let Some(key_end) = key.pos.as_ref().map(|r| r.end) {
                    end_pos = key_end;
                }
                
                // Ensure we have a value
                if tokens.is_empty() || tokens.first().map(|(t, _)| t) == Some(&"}".to_string()) {
                    let pos = SourceRange { start: start_pos, end: end_pos };
                    return Err(BlinkError::parse_invalid_number("Map literal must have even number of elements (key-value pairs)", pos));
                }
                
                // Parse value
                let value = parse(tokens, reader_ctx, symbol_table)?;
                if let Some(value_end) = value.pos.as_ref().map(|r| r.end) {
                    end_pos = value_end;
                }
                
                pairs.push((key, value));
            }
            
            // Unclosed map
            let pos = SourceRange { start: start_pos, end: end_pos };
            Err(BlinkError::parse_unclosed_delimiter("Unclosed map literal", "}", pos))
        }

        ")" | "]" | "}" => {
            let end_pos = calculate_token_end(&token, &start_pos);
            let range = SourceRange { start: start_pos, end: end_pos };
            Err(BlinkError::unexpected_token(&token, start_pos).with_pos(Some(range)))
        }

        _ => {
            // Check for reader macros
            let mut matched_macro: Option<(String, u32)> = None;
            for (prefix, &symbol_id) in &reader_ctx.reader_macros {
                if token.starts_with(prefix) {
                    if let Some((best_prefix, _)) = &matched_macro {
                        if prefix.len() > best_prefix.len() {
                            matched_macro = Some((prefix.clone(), symbol_id));
                        }
                    } else {
                        matched_macro = Some((prefix.clone(), symbol_id));
                    }
                }
            }
        
            if let Some((prefix, symbol_id)) = matched_macro {
                let rest = &token[prefix.len()..];
            
                let target_form = if rest.is_empty() {
                    if tokens.is_empty() {
                        let pos = SourceRange { start: start_pos, end: start_pos };
                        return Err(BlinkError::parse_unexpected_eof(pos));
                    }
                    parse(tokens, reader_ctx, symbol_table)?
                } else {
                    let mut rest_tokens = tokenize(rest)?;
                    parse(&mut rest_tokens, reader_ctx, symbol_table)?
                };

                let end_pos = target_form.pos.as_ref()
                    .map(|r| r.end)
                    .unwrap_or_else(|| calculate_token_end(&token, &start_pos));

                let expanded = apply_reader_macro(symbol_id, target_form);
                
                return Ok(expanded);
            }
            
            Ok(atom_with_pos(&token, start_pos, symbol_table))
        }
    }
}

// Helper function to calculate where a token ends
fn calculate_token_end(token: &str, start_pos: &SourcePos) -> SourcePos {
    if token.contains('\n') {
        // Handle multi-line tokens (like multi-line strings)
        let lines: Vec<&str> = token.split('\n').collect();
        SourcePos {
            line: start_pos.line + lines.len() - 1,
            col: if lines.len() > 1 {
                lines.last().unwrap().len() + 1
            } else {
                start_pos.col + token.len()
            },
        }
    } else {
        SourcePos {
            line: start_pos.line,
            col: start_pos.col + token.len(),
        }
    }
}

// Updated atom function to return ParsedValueWithPos
fn atom_with_pos(token: &str, start_pos: SourcePos, symbol_table: &mut SymbolTable) -> ParsedValueWithPos {
    let end_pos = calculate_token_end(token, &start_pos);
    let range = Some(SourceRange { start: start_pos, end: end_pos });

    let value = if token.starts_with('"') && token.ends_with('"') {
        ParsedValue::String(token[1..token.len() - 1].to_string())
    } else if let Ok(n) = token.parse::<f64>() {
        ParsedValue::Number(n)
    } else if token == "true" {
        ParsedValue::Bool(true)
    } else if token == "false" {
        ParsedValue::Bool(false)
    } else if token == "nil" {
        ParsedValue::Nil
    } else if token.starts_with(':') {
        let id = symbol_table.intern(&token);
        ParsedValue::Keyword(id)
    } else {
        let id = parse_symbol_token(token, symbol_table);
        ParsedValue::Symbol(id)
    };

    ParsedValueWithPos::new(value, range)
}

pub fn parse_all(
    code: &str, 
    reader_ctx: &mut ReaderContext,
    symbol_table: &mut SymbolTable
) -> Result<Vec<ParsedValueWithPos>, BlinkError> {
    let mut tokens = tokenize(code)?;
    let mut forms = Vec::new();

    while !tokens.is_empty() {
        let form = parse(&mut tokens, reader_ctx, symbol_table)?;
        forms.push(form);
    }

    Ok(forms)
}
