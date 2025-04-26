mod socket_server;

#[tokio::main]
async fn main() {
    println!("🔌 Starting Blink socket server...");
    socket_server::start_socket_server()
        .await
        .expect("Failed to start socket server");
}
