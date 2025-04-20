mod value;
mod env;
mod parser;
mod eval;
mod native_functions;
mod repl;
mod error;

fn main() {
    repl::start_repl();
}