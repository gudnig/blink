use blink_core::repl::start_repl;

#[tokio::main]
async fn main() {
    println!("Blink REPL");
    start_repl().await;
}
