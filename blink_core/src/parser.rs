use crate::error::{LispError, SourcePos};
use crate::value::{ bool_val_at, keyword_at, list_val, num_at, str_val, sym, sym_at, BlinkValue};

pub fn tokenize(code: &str) -> Result<Vec<(String, SourcePos)>, LispError> {
    tokenize_at(code, None)
}

pub fn tokenize_at(code: &str, pos: Option<SourcePos>) -> Result<Vec<(String, SourcePos)>, LispError> {
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




pub fn parse(tokens: &mut Vec<(String, SourcePos)>) -> Result<BlinkValue, LispError> {
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
                list.push(parse(tokens)?);
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
                    return Ok(list_val(std::iter::once(sym("vector")).chain(elements.into_iter()).collect()));
                }
                elements.push(parse(tokens)?);
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
                    return Ok(list_val(std::iter::once(sym("hash-map")).chain(entries.into_iter()).collect()));
                }
                entries.push(parse(tokens)?); // key
                entries.push(parse(tokens)?); // val
            }
            Err(LispError::ParseError {
                message: "Unclosed map literal".into(),
                pos,
            })
        }


        "'" => {
            let quoted = parse(tokens)?;
            Ok(list_val(vec![sym("quote"), quoted]))
        }

        ")" | "]" | "}" => Err(LispError::UnexpectedToken {
            token,
            pos,
        }),

        _ => Ok(atom(&token, Some(pos))),
    }
}


pub fn parse_all(code: &str) -> Result<Vec<BlinkValue>, LispError> {
    let mut tokens = tokenize(code)?;
    let mut forms = Vec::new();

    while !tokens.is_empty() {
        let form = parse(&mut tokens)?; // your existing parse()
        forms.push(form);
    }

    Ok(forms)
}