use std::fmt;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SourcePos {
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for SourcePos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.col)
    }
}

#[derive(Debug)]
pub enum LispError {
    TokenizerError {
        message: String,
        pos: SourcePos,
    },
    ParseError {
        message: String,
        pos: SourcePos,
    },
    EvalError {
        message: String,
        pos: Option<SourcePos>, // optional if eval doesn’t know pos
    },
    ArityMismatch {
        expected: usize,
        got: usize,
        form: String,
        pos: Option<SourcePos>,
    },
    UndefinedSymbol {
        name: String,
        pos: Option<SourcePos>,
    },
    UnexpectedToken {
        token: String,
        pos: SourcePos,
    },
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
        }
    }
}

impl std::error::Error for LispError {}
