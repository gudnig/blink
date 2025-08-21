mod env;
mod error;
mod eval;
mod native_functions;
mod parser;
mod repl;
mod telemetry;
mod value;
mod module;
mod future;
mod collections;
mod runtime;
mod compiler;

#[tokio::main]
async fn main() {
    repl::start_repl();
}
