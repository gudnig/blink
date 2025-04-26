mod env;
mod error;
mod eval;
mod native_functions;
mod parser;
mod repl;
mod telemetry;
mod value;

fn main() {
    repl::start_repl();
}
