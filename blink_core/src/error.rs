use std::{fmt, sync::Arc};

use parking_lot::RwLock;

use crate::{value::{SourcePos, SourceRange, BlinkValue, LispNode, Value}};



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
        data: Option<BlinkValue>
    }
}

impl LispError {
    pub fn into_blink_value(self) -> BlinkValue {
        let pos = match &self {
            LispError::TokenizerError { pos, .. } => Some(SourceRange {start: pos.clone(), end: pos.clone()}),
            LispError::ParseError { pos, .. } => Some(pos.clone()),
            LispError::EvalError { pos, .. } => pos.clone(),
            LispError::ArityMismatch { pos, .. } => pos.clone(),
            LispError::UndefinedSymbol { pos, .. } => pos.clone(),
            LispError::UnexpectedToken { token, pos } => Some(SourceRange {start: pos.clone(), end: pos.clone()}),
            LispError::ModuleError { message, pos } => pos.clone(),
            LispError::UserDefined { message, pos, data } => pos.clone(),
        };
        BlinkValue(Arc::new(RwLock::new(LispNode {
            value: Value::Error(self),
            pos: pos,
        }))) 
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
            UserDefined { message, pos, data } => match pos {
                Some(p) => write!(f, "User defined error at {}: {}", p, message),
                None => write!(f, "User defined error: {}", message),
            },
        }
    }
}

impl std::error::Error for LispError {}
