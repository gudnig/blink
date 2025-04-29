mod lsp;
mod repl_message;
mod session;
mod session_manager;

use clap::Parser;

use crate::session_manager::SessionManager;
use std::{io, sync::Arc};
use tokio::net::{TcpListener, TcpStream};

/// Blink Daemon options
#[derive(Parser)]
struct Opts {
    /// Port to listen on
    #[arg(short, long, default_value = "7010")]
    port: u16,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let opts = Opts::parse();

    let addr = format!("127.0.0.1:{}", opts.port);

    let manager = Arc::new(SessionManager::new());
    let listener = TcpListener::bind("127.0.0.1:7010").await?;

    println!("Blink Daemon listening on 127.0.0.1:7010");

    loop {
        let (socket, addr) = listener.accept().await?;
        let manager = manager.clone();

        tokio::spawn(async move {
            handle_connection(socket, manager).await;
        });
    }
}

async fn handle_connection(socket: TcpStream, manager: Arc<SessionManager>) {
    let mut socket = socket;

    match detect_protocol(&mut socket).await {
        Ok(msg_type) => match msg_type {
            MsgType::Lsp => {}
            MsgType::Blink => {}
        },
        Err(e) => {
            eprintln!("Protocol detection failed: {:?}", e);
        }
    }
}

pub enum MsgType {
    Lsp,
    Blink,
}

async fn detect_protocol(socket: &mut TcpStream) -> io::Result<MsgType> {
    let mut buf = [0; 4];
    socket.peek(&mut buf).await?;

    if buf.starts_with(b"{") || buf.starts_with(b"C") {
        // `{` = JSON; `C` = Content-Length header
        Ok(MsgType::Lsp)
    } else {
        Ok(MsgType::Blink)
    }
}
