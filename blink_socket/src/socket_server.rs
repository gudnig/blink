use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use blink_core::env::Env;
use blink_core::eval::{eval, EvalContext};
use blink_core::parser::{parse, preload_builtin_reader_macros, tokenize, ReaderContext};
use blink_core::telemetry::BlinkMessage;

use anyhow::Result;
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use tokio::task::LocalSet;

pub async fn start_socket_server() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:5555").await?;
    println!("Socket server listening on 127.0.0.1:5555");

    let env = Rc::new(RefCell::new(Env::new()));
    blink_core::native_functions::register_builtins(&env);
    let mut ctx = EvalContext {
        env,
        current_module: None,
        plugins: Default::default(),
        telemetry_sink: None,
        tracing_enabled: true,
        reader_macros: Rc::new(RefCell::new(ReaderContext::new())),
    };

    preload_builtin_reader_macros(&mut ctx);
    let global_ctx = Arc::new(Mutex::new(ctx));
    let local = LocalSet::new(); // <-- move LocalSet OUTSIDE

    local.spawn_local(async move {
        loop {
            let (mut socket, addr) = listener.accept().await.expect("accept failed");
            println!("Client connected: {}", addr);

            let ctx = global_ctx.clone();

            tokio::task::spawn_local(async move {
                loop {
                    let mut len_buf = [0u8; 4];
                    if socket.read_exact(&mut len_buf).await.is_err() {
                        println!("Client {} disconnected", addr);
                        break;
                    }
                    let msg_len = u32::from_be_bytes(len_buf) as usize;

                    let mut msg_buf = vec![0u8; msg_len];
                    if socket.read_exact(&mut msg_buf).await.is_err() {
                        println!("Client {} disconnected", addr);
                        break;
                    }

                    let mut de = Deserializer::new(&msg_buf[..]);
                    let incoming: BlinkMessage = match Deserialize::deserialize(&mut de) {
                        Ok(msg) => msg,
                        Err(_) => {
                            println!("Failed to deserialize message from {}", addr);
                            continue;
                        }
                    };

                    match incoming {
                        BlinkMessage::Eval { id, code } => {
                            let mut ctx = ctx.lock().unwrap();

                            let ast = {
                                let mut rcx = ctx.reader_macros.borrow_mut();
                                let mut tokens = match tokenize(&code) {
                                    Ok(toks) => toks,
                                    Err(e) => {
                                        println!("Tokenize error: {}", e);
                                        return;
                                    }
                                };
                                match parse(&mut tokens, &mut *rcx) {
                                    Ok(ast) => ast,
                                    Err(e) => {
                                        println!("Parse error: {}", e);
                                        return;
                                    }
                                }
                            };

                            let result = eval(ast, &mut ctx).map_err(|e| e.to_string());

                            let response = match result {
                                Ok(val) => BlinkMessage::Result {
                                    id,
                                    value: val.to_string_repr(),
                                },
                                Err(msg) => BlinkMessage::Error { id, message: msg },
                            };

                            let mut buf = Vec::new();
                            if response.serialize(&mut Serializer::new(&mut buf)).is_ok() {
                                let len = (buf.len() as u32).to_be_bytes();
                                if socket.write_all(&len).await.is_err() {
                                    break;
                                }
                                if socket.write_all(&buf).await.is_err() {
                                    break;
                                }
                            }
                        }
                        _ => {
                            println!("Unsupported message type from {}", addr);
                        }
                    }
                }
            });
        }
    });

    local.await; // <-- at the end: drive LocalSet
    Ok(())
}
