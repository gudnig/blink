use std::{io::BufRead, sync::Arc};

use anyhow::Context;
use blink_core::{eval::{self, EvalContext}, native_functions::register_builtins, parser::{parse, preload_builtin_reader_macros, tokenize_at}, value::SourcePos, BlinkValue, Env};
use parking_lot::RwLock;
use rmp_serde::{from_slice, to_vec, Deserializer, Serializer};
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::{
    helpers::collect_symbols_from_forms, repl_message::{ReplRequest, ReplResponse}, session::{Session, SymbolSource}, session_manager::SessionManager
};



pub struct ReplHandler<R, W> {
    reader: BufReader<R>,
    writer: W,
    session: Option<Arc<Session>>,
}

impl<R, W> ReplHandler<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn read_message(&mut self) -> io::Result<Option<ReplRequest>> {
        self.read_msgpack_frame::<ReplRequest>().await
    }
    async fn write_message(&mut self, msg: &ReplResponse) -> io::Result<()> {
        self.write_msgpack_frame( msg).await
    }

    pub async fn read_msgpack_frame<T>(&mut self) -> io::Result<Option<T>>
    where
        T: DeserializeOwned,
    
    {
        // Step 1: read bytes async
        let mut len_buf = [0u8; 4];
        if self.reader.read_exact(&mut len_buf).await.is_err() {
            return Ok(None); // EOF or error
        }
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf).await?;

        // Step 2: deserialize *synchronously* and *immediately*
        let msg_result = {
            // This block isolates any internal !Send temporaries
            from_slice::<T>(&buf)
        };

        let msg = msg_result.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(msg))
    }


    pub async fn write_msgpack_frame(&mut self, msg: &ReplResponse) -> io::Result<()>
    {
        println!("DEBUG: Type is {}", std::any::type_name::<ReplResponse>());
        println!("DEBUG: JSON representation = {}", serde_json::to_string(msg).unwrap());
        let mut buf = Vec::new();
        {
            let mut serializer = Serializer::new(&mut buf).with_struct_map(); // force map encoding
            msg.serialize(&mut serializer).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        }
        let len = (buf.len() as u32).to_be_bytes();
        self.writer.write_all(&len).await?;
        self.writer.write_all(&buf).await?;
        self.writer.flush().await?;
        Ok(())
    }
    

    pub fn new(reader: BufReader<R>, writer: W) -> Self {
        Self {
            reader,
            writer,
            session: None,
        }
    }

    pub async fn init(&mut self, session_manager: Arc<SessionManager>) -> anyhow::Result<()> {
        let message = self.read_message().await?.context("No message received")?;
        let response = match message {
            ReplRequest::Init { id, session_id } => {
                let session = if let Some(session_id) = session_id {
                    session_manager
                        .get(&session_id)
                        .await
                        .with_context(|| format!("Session '{}' not found", session_id))?
                } else {
                    let new_id = uuid::Uuid::new_v4().to_string();
                    let arc_session = Arc::new(Session::new(new_id.clone()));

                    

                    
                    // Register the session
                    session_manager.register(arc_session.clone()).await;
                    println!("Session pointer at init: {:?}", Arc::as_ptr(&arc_session));

                    self.session = Some(arc_session.clone());
                    arc_session

                    
                };
                {
                    let mut ctx = session.eval_ctx.write();
                    // Check if the context is null
                    if ctx.is_none() {
                        
                        let global_env = Arc::new(RwLock::new(Env::new()));

                        register_builtins(&global_env);
                        let mut eval_ctx = EvalContext::new(global_env.clone());
                        preload_builtin_reader_macros(&mut eval_ctx);
                        *ctx = Some(Box::new(eval_ctx));
                    }
                }
                session.features.write().repl = true;
                self.session = Some(session);
                ReplResponse::Initialized {
                    id,
                }
            }
            _ => anyhow::bail!("Invalid request"),
        };
        println!("--------------------------------");
        println!("Sending REPL response: {:?}", &response);
        self.write_message(&response).await?;
        Ok(())
    }
    pub async fn process(&mut self) -> anyhow::Result<()> {
        let session = self.session.as_ref().cloned().context("No session found")?;
        println!("Session pointer at process: {:?}", Arc::as_ptr(&session));
        loop {
            let message = self.read_message().await?.context("No message received")?;
            match message {
                ReplRequest::Eval { id, code, pos } => {
                    let response = self.handle_eval(id, code, pos)?;
                    
                    self.write_message(&response).await?;
                }
                ReplRequest::Close => {
                    break;
                }
                _ => anyhow::bail!("Invalid request"),
            }
        }
        Ok(())
    }

    
    

    pub fn handle_eval(&mut self, id: String, code: String, pos: Option<SourcePos>) -> anyhow::Result<ReplResponse> {
        println!("Received code literal: {:?}", &code);
            
        let source_pos = pos.unwrap_or(SourcePos { line: 0, col: 0 });
                    let session = self.session.as_ref().cloned().context("No session found")?;
                    let mut ctx_guard = session.eval_ctx.write();
                    let ctx = ctx_guard
                        .as_deref_mut()
                        .ok_or(anyhow::anyhow!("No eval context found."))?;

                    let ast = {
                        let mut rctx = ctx.reader_macros.write(); // this lock is scoped
                        let mut tokens = tokenize_at(&code, Some(source_pos))?;
                        parse(&mut tokens, &mut rctx).context("Failed to parse code")?
                    }; // `rctx` dropped here
                    println!("Ast {:?}", ast);

                    let result = eval::eval(ast, ctx);

                    let response = match result {
                        Ok(value) => {
                            // Should I push symbols to the session here
                            let ses = self.session.as_ref().unwrap();
                            
                            let resp = ReplResponse::EvalResult {
                                id,
                                value: format!("{}", value.clone()),
                            };
                            {
                                let mut symbols = ses.symbols.write();
                                collect_symbols_from_forms(&mut symbols, &vec![value], SymbolSource::Repl);
                            }
                            resp
                            
                            
                        }
                        Err(e) => {
                            ReplResponse::Error {
                                id,
                                message: e.to_string(),
                            }
                        }
                        _ => anyhow::bail!("Invalid request"),
                    };
            Ok(response)
        
    }
}
