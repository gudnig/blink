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
mod async_context;
mod goroutine;

#[tokio::main]
async fn main() {
    repl::start_repl();
}
