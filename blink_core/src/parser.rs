use crate::error::{BlinkError, ParseErrorType};
use crate::value::{SourcePos, SourceRange, ParsedValue};
use std::collections::HashMap;

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

pub fn parse_symbol_token(token: &str, symbol_table: &mut dyn SymbolTableTrait) -> u32 {
    if let Some((module_part, symbol_part)) = token.split_once('/') {
        // Qualified symbol like "math/add"
        let module_id = symbol_table.intern(module_part);
        let symbol_id = symbol_table.intern(symbol_part);
        symbol_table.intern_qualified(module_id, symbol_id)
    } else {
        // Simple symbol like "add"
        symbol_table.intern(token)
    }
}

// Trait for symbol table operations that the parser needs
pub trait SymbolTableTrait {
    fn intern(&mut self, name: &str) -> u32;
    fn intern_qualified(&mut self, module_id: u32, symbol_id: u32) -> u32;
}

pub fn atom(token: &str, pos: Option<SourcePos>, symbol_table: &mut dyn SymbolTableTrait) -> ParsedValue {
    if token.starts_with('"') && token.ends_with('"') {
        ParsedValue::String(token[1..token.len() - 1].to_string())
    } else if let Ok(n) = token.parse::<f64>() {
        ParsedValue::Number(n)
    } else if token == "true" {
        ParsedValue::Bool(true)
    } else if token == "false" {
        ParsedValue::Bool(false)
    } else if token.starts_with(':') {
        let keyword_id = symbol_table.intern(&token[1..]);
        ParsedValue::Keyword(keyword_id)
    } else {
        let symbol_id = parse_symbol_token(token, symbol_table);
        ParsedValue::Symbol(symbol_id)
    }
}

fn apply_reader_macro(symbol_id: u32, form: ParsedValue) -> ParsedValue {
    ParsedValue::List(vec![
        ParsedValue::Symbol(symbol_id),
        form
    ])
}

pub fn parse(
    tokens: &mut Vec<(String, SourcePos)>,
    reader_ctx: &ReaderContext,
    symbol_table: &mut dyn SymbolTableTrait,
) -> Result<ParsedValue, BlinkError> {
    if tokens.is_empty() {
        return Err(BlinkError::unexpected_token("EOF", SourcePos { line: 0, col: 0 }));
    }

    let (token, start) = tokens.remove(0);

    match token.as_str() {
        "(" => {
            let mut list = Vec::new();
            
            while let Some((tok, _next_pos)) = tokens.first() {
                if tok == ")" {
                    tokens.remove(0); // consume ')'
                    break;
                }
                let item = parse(tokens, reader_ctx, symbol_table)?;
                list.push(item);
            }

            Ok(ParsedValue::List(list))
        }

        "[" => {
            let mut elements = Vec::new();
            
            while let Some((t, _next_pos)) = tokens.first() {
                if t == "]" {
                    tokens.remove(0); // consume ']'
                    return Ok(ParsedValue::Vector(elements));
                }
                let item = parse(tokens, reader_ctx, symbol_table)?;
                elements.push(item); 
            }
            
            let pos = SourceRange { start: start.clone(), end: start };
            Err(BlinkError::parse_unclosed_delimiter("Unclosed vector literal", "[", pos))
        }

        "{" => {
            let mut entries: Vec<ParsedValue> = Vec::new();
            
            while let Some((t, _next_pos)) = tokens.first() {
                if t == "}" {
                    tokens.remove(0); // consume '}'
                    
                    // Convert entries to key-value pairs
                    if entries.len() % 2 != 0 {
                        let pos = SourceRange { start: start.clone(), end: start };
                        return Err(BlinkError::parse_invalid_number("Map literal must have even number of elements (key-value pairs)", pos));
                    }
                    
                    let pairs: Vec<(ParsedValue, ParsedValue)> = entries
                        .chunks(2)
                        .map(|pair| (pair[0].clone(), pair[1].clone()))
                        .collect();
                    
                    return Ok(ParsedValue::Map(pairs));
                }
                
                // Parse one element (key or value)
                let element = parse(tokens, reader_ctx, symbol_table)?;
                entries.push(element);
            }
            
            // If we get here, we never found the closing }
            let pos = SourceRange { start: start.clone(), end: start };
            Err(BlinkError::parse_unclosed_delimiter("Unclosed map literal", "}", pos))
        }

        ")" | "]" | "}" => Err(BlinkError::unexpected_token(&token, start)),

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
                        let pos = SourceRange { start: start.clone(), end: start };
                        return Err(BlinkError::parse_unexpected_eof(pos));
                    }
                    parse(tokens, reader_ctx, symbol_table)?
                } else {
                    let mut rest_tokens = tokenize(rest)?;
                    parse(&mut rest_tokens, reader_ctx, symbol_table)?
                };
            
                return Ok(apply_reader_macro(symbol_id, target_form));
            }
            
            Ok(atom(&token, Some(start), symbol_table))
        }
    }
}

pub fn parse_all(
    code: &str, 
    reader_ctx: &ReaderContext,
    symbol_table: &mut dyn SymbolTableTrait
) -> Result<Vec<ParsedValue>, BlinkError> {
    let mut tokens = tokenize(code)?;
    let mut forms = Vec::new();

    while !tokens.is_empty() {
        let form = parse(&mut tokens, reader_ctx, symbol_table)?;
        forms.push(form);
    }

    Ok(forms)
}