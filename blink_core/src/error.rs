use std::fmt::{self, Display};

use crate::{collections::{ContextualValueRef, ValueContext}, value::{  SourcePos, SourceRange, ValueRef}};

#[derive(Debug, Clone)]
pub struct BlinkError {
    pub message: String,
    pub pos: Option<SourceRange>,
    pub error_type: BlinkErrorType,
}

#[derive(Debug, Clone)]
pub enum ParseErrorType {
    UnclosedDelimiter(String), 
    UnexpectedToken(String),
    InvalidNumber(String),
    InvalidString(String),
    UnexpectedEof,
    
}

#[derive(Debug, Clone)]
pub enum BlinkErrorType {
    Tokenizer,    
    Parse(ParseErrorType),
    UndefinedSymbol{
        name: String,
    },
    Eval,
    ArityMismatch{
        expected: usize,
        got: usize,
        form: String,
    },
    UnexpectedToken{
        token: String
    },
    UserDefined {
        data: Option<ValueRef>
    }
}

impl BlinkErrorType {
    pub fn display_with_context(&self, f: &mut fmt::Formatter<'_>, context: &ValueContext) -> fmt::Result {
        match self {
            BlinkErrorType::Tokenizer => write!(f, "Tokenizer error"),
            BlinkErrorType::Parse(parse_error_type) => {
                match parse_error_type {
                    ParseErrorType::UnclosedDelimiter(message) => write!(f, "Unclosed delimiter: {}", message),
                    ParseErrorType::UnexpectedToken(token) => write!(f, "Unexpected token: {}", token),
                    ParseErrorType::InvalidNumber(message) => write!(f, "Invalid number: {}", message),
                    ParseErrorType::InvalidString(message) => write!(f, "Invalid string: {}", message),
                    ParseErrorType::UnexpectedEof => write!(f, "Unexpected EOF"),
                }
            },
            BlinkErrorType::UndefinedSymbol { name } => write!(f, "Undefined symbol: {}", name),
            BlinkErrorType::Eval => write!(f, "Eval error"),
            BlinkErrorType::ArityMismatch { expected, got, form } => write!(f, "Arity mismatch in '{}': expected {}, got {}", form, expected, got),
            BlinkErrorType::UnexpectedToken { token } => write!(f, "Unexpected token: {}", token),
            BlinkErrorType::UserDefined { data } => {
                if let Some(data) = data {
                    let contextual = ContextualValueRef::new(data.clone(), context.clone());
                    write!(f, "User defined error: {}", contextual)
                } else {
                    write!(f, "User defined error")
                }
            },
        }
    }
}

impl BlinkError {
    pub fn tokenizer(message: impl Into<String>, pos: SourcePos) -> Self {
        Self {
            message: message.into(),
            pos: Some(SourceRange { start: pos, end: pos }),
            error_type: BlinkErrorType::Tokenizer,
        }
    }

    pub fn unexpected_token(token: &str, pos: SourcePos) -> Self {
        Self {
            message: format!("Unexpected token '{}'", token),
            pos: Some(SourceRange { start: pos, end: pos }),
            error_type: BlinkErrorType::UnexpectedToken { token: token.to_string() },
        }
    }

    pub fn undefined_symbol(name: &str) -> Self {
        Self {
            message: format!("Undefined symbol '{}'", name),
            pos: None,
            error_type: BlinkErrorType::UndefinedSymbol { name: name.to_string() },
        }
    }
    
    pub fn parse(message: impl Into<String>, pos: SourceRange, error_type: ParseErrorType) -> Self {
        Self {
            message: message.into(),
            pos: Some(pos),
            error_type: BlinkErrorType::Parse(error_type),
        }
    }

    pub fn parse_unclosed_delimiter(message: &str, delimiter: &str, pos: SourceRange) -> Self {
        Self::parse(message, pos, ParseErrorType::UnclosedDelimiter(delimiter.into()))
    }

    pub fn parse_unexpected_token(token: &str, pos: SourceRange) -> Self {
        Self::parse(format!("Unexpected token '{}'", token), pos, ParseErrorType::UnexpectedToken(token.into()))
    }

    pub fn parse_invalid_number(message: &str, pos: SourceRange) -> Self {
        Self::parse(message, pos, ParseErrorType::InvalidNumber(message.into()))
    }

    pub fn parse_invalid_string(message: &str, pos: SourceRange) -> Self {
        Self::parse(message, pos, ParseErrorType::InvalidString(message.into()))
    }

    pub fn parse_unexpected_eof(pos: SourceRange) -> Self {
        Self::parse("Unexpected EOF", pos, ParseErrorType::UnexpectedEof)
    }

    pub fn eval(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            pos: None,
            error_type: BlinkErrorType::Eval,
        }
    }
    
    pub fn arity(expected: usize, got: usize, form: &str) -> Self {
        Self {
            message: format!("Wrong number of arguments to '{}': expected {}, got {}", form.clone(), expected, got),
            pos: None,
            error_type: BlinkErrorType::ArityMismatch { expected, got, form: form.into() },
        }
    }
    
    pub fn with_pos(mut self, pos: Option<SourceRange>) -> Self {
        self.pos = pos;
        self
    }
}


#[derive(Debug, Clone)]
pub enum LispError {
    TokenizerError {
        message: String,
        pos: SourcePos,
    },
    ParseError {
        message: String,
        pos: SourceRange,
    },
    EvalError {
        message: String,
        pos: Option<SourceRange>, // optional if eval doesn’t know pos
    },
    ArityMismatch {
        expected: usize,
        got: usize,
        form: String,
        pos: Option<SourceRange>,
    },
    UndefinedSymbol {
        name: String,
        pos: Option<SourceRange>,
    },
    UnexpectedToken {
        token: String,
        pos: SourcePos,
    },
    ModuleError {
        message: String,
        pos: Option<SourceRange>,
    },
    UserDefined {
        message: String,
        pos: Option<SourceRange>,
        data: Option<ValueRef>
    }
}


impl fmt::Display for BlinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.error_type {
            BlinkErrorType::Tokenizer => write!(f, "Tokenizer error: {}", self.message),
            BlinkErrorType::Parse(error_type) => {
                match error_type {
                    ParseErrorType::UnclosedDelimiter(message) => write!(f, "Unclosed delimiter: {}", message),
                    ParseErrorType::UnexpectedToken(token) => write!(f, "Unexpected token: {}", token),
                    ParseErrorType::InvalidNumber(message) => write!(f, "Invalid number: {}", message),
                    ParseErrorType::InvalidString(message) => write!(f, "Invalid string: {}", message),
                    ParseErrorType::UnexpectedEof => write!(f, "Unexpected EOF"),
                }
            },
            BlinkErrorType::Eval => write!(f, "Eval error: {}", self.message),
            BlinkErrorType::ArityMismatch { expected, got, form } => write!(f, "Arity mismatch in '{}': expected {}, got {}", form, expected, got),
            BlinkErrorType::UndefinedSymbol { name } => write!(f, "Undefined symbol '{}'", name),
            BlinkErrorType::UnexpectedToken { token } => write!(f, "Unexpected token '{}'", token),
            BlinkErrorType::UserDefined {  data: _ } => write!(f, "User defined error: {}", self.message),
        };
        if let Some(pos) = self.pos {
            write!(f, " at {}", pos);
        }
        
        Ok(())
    }
}

impl fmt::Display for LispError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use LispError::*;
        match self {
            TokenizerError { message, pos } => write!(f, "Tokenizer error at {}: {}", pos, message),
            ParseError { message, pos } => write!(f, "Parse error at {}: {}", pos, message),
            EvalError { message, pos } => match pos {
                        Some(p) => write!(f, "Eval error at {}: {}", p, message),
                        None => write!(f, "Eval error: {}", message),
                    },
            ArityMismatch {
                        expected,
                        got,
                        form,
                        pos,
                    } => match pos {
                        Some(p) => write!(
                            f,
                            "Arity mismatch in '{}' at {}: expected {}, got {}",
                            form, p, expected, got
                        ),
                        None => write!(
                            f,
                            "Arity mismatch in '{}': expected {}, got {}",
                            form, expected, got
                        ),
                    },
            UndefinedSymbol { name, pos } => match pos {
                        Some(p) => write!(f, "Undefined symbol '{}' at {}", name, p),
                        None => write!(f, "Undefined symbol '{}'", name),
                    },
            UnexpectedToken { token, pos } => write!(f, "Unexpected token '{}' at {}", token, pos),
            ModuleError { message, pos } => match pos {
                Some(p) => write!(f, "Module error at {}: {}", p, message),
                None => write!(f, "Module error: {}", message),
            },
            UserDefined { message, pos, data: _ } => match pos {
                Some(p) => write!(f, "User defined error at {}: {}", p, message),
                None => write!(f, "User defined error: {}", message),
            },
        }
    }
}

impl std::error::Error for LispError {}
