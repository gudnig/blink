use blink_core::repl::start_repl;

#[tokio::main]
async fn main() {
    start_repl().await;
}
