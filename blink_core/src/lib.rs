pub mod value;
pub mod env;
pub mod parser;
pub mod eval;
pub mod native_functions;
pub mod error;
pub mod repl; // optional — not needed by plugins

pub use env::Env;
pub use value::{BlinkValue, Value, LispNode, str_val, num, bool_val}; // add what your plugins use

