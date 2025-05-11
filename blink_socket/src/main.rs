mod lsp;
mod lsp_messages;
mod repl_message;
mod repl;   
mod session;
mod session_manager;
mod helpers;
use clap::Parser;
use lsp::LspHandler;
use repl::ReplHandler;


use crate::session_manager::SessionManager;
use std::sync::Arc;
use tokio::{ io::{BufReader, BufWriter}, net::TcpListener};

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

    let manager = Arc::new(SessionManager::new());
    let repl_manager = manager.clone();

    let repl_port = opts.port;
    let lsp_port = opts.port + 1;

    let repl_listener = TcpListener::bind(("127.0.0.1", repl_port)).await?;
    let lsp_listener = TcpListener::bind(("127.0.0.1", lsp_port)).await?;

    println!("Blink REPL listening on 127.0.0.1:{}", repl_port);
    println!("Blink LSP listening on 127.0.0.1:{}", lsp_port);

    // Spawn REPL server
    tokio::spawn(async move {
        loop {
            match repl_listener.accept().await {
                Ok((socket, addr)) => {
                    println!("REPL client {} connected.", addr);
                    let (reader, writer) = socket.into_split();
                    let reader = BufReader::new(reader);
                    let writer = BufWriter::new(writer);
                    
                    let mut handler = ReplHandler::new(reader, writer);
                    let result = handler.init(repl_manager.clone()).await;
                    if result.is_err() {
                        eprintln!("Failed to initialize REPL handler: {:?}", result.err().unwrap());
                        continue;
                    }
                    
                    tokio::spawn(async move {
                        let result =    handler.process().await;
                        if result.is_err() {
                            eprintln!("REPL handler process error: {:?}", result.err().unwrap());
                        }
                    });
                }
                Err(e) => eprintln!("REPL accept error: {:?}", e),
            }
        }
    });

    // Spawn LSP server
    loop {
        match lsp_listener.accept().await {
            Ok((socket, addr)) => {
                println!("LSP client {} connected.", addr);
                let (reader, writer) = socket.into_split();
                let reader = BufReader::new(reader);
                let writer = BufWriter::new(writer);
                
                let mut handler = LspHandler::new(reader, writer);
                let result = handler.init(manager.clone()).await;
                if result.is_err() {
                    eprintln!("Failed to initialize LSP handler: {:?}", result.err().unwrap());
                    continue;
                }
                tokio::spawn(async move {
                    let result = handler.process().await;
                    if result.is_err() {
                        eprintln!("LSP handler process error: {:?}", result.err().unwrap());
                    }
                });
            }
            Err(e) => eprintln!("LSP accept error: {:?}", e),
        }
    }
}

