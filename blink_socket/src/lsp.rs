use std::sync::Arc;

use crate::{
    helpers::collect_symbols_from_forms, lsp_messages::{create_server_capabilities, CompletionItem, CompletionParams, Diagnostic, DiagnosticsParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentSymbolParams, GotoDefinitionParams, HoverParams, LspError, LspMessage, Position, Range}, session::{Document, Session, SymbolInfo, SymbolKind, SymbolSource}, session_manager::SessionManager
};
use anyhow::{anyhow, Context, Result};
use blink_core::{error::LispError, parser::parse_all, value::SourcePos, BlinkValue};
use serde_json::{json, Value};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Helper function to read LSP message

/// Enum representing possible results from the LSP handler

pub enum LspHandlerResult {
    /// A response message to send back to the client
    Response(LspMessage),
    /// Diagnostics to publish for a specific document
    PublishDiagnostics(DiagnosticsParams),
    /// Clear diagnostics for a specific document
    ClearDiagnostics(String),
    /// An error occurred during processing
    Error(String),
    /// Client requested exit
    Exit,
    /// No response needed (e.g., for notifications)
    NoResponse,
}



// Updated LspHandler implementation with error handling
pub struct LspHandler<R, W> {
    reader: BufReader<R>,
    writer: W,
    session: Option<Arc<Session>>
}


impl<R, W> LspHandler<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    pub fn new(reader: BufReader<R>, writer: W) -> Self {
        Self {
            reader,
            writer,
            session: None,
        }
    }

    pub async fn init(&mut self, session_manager: Arc<SessionManager>) -> Result<()> {
        let msg = self
            .read_message()
            .await?
            .context("Not received init message.")?;
        let id = msg.id.context("Missing id in initialize message.")?;
        let method = msg
            .method
            .as_deref()
            .context("Missing method in initialize message.")?;
        if method != "initialize" {
            return Err(anyhow!(format!("Expected initialize, got {:?}", method)));
        }
        let params = msg
            .params
            .context("Missing params in initialize message.")?;
        let session_id = params
            .get("initializationOptions")
            .and_then(|o| o.get("sessionId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let session = if let Some(session_id) = session_id {
            session_manager
                .get(&session_id)
                .await
                .with_context(|| format!("Session '{}' not found", session_id))?
        } else {
            let new_id = uuid::Uuid::new_v4().to_string();
            let session = Session::new(new_id.clone());
            session.features.write().lsp = true;
            let arc_session = Arc::new(session);
            session_manager.register(arc_session.clone()).await;
            arc_session
        };

        let response = self.handle_initialize(id).await;
        
        self.write_message(&response).await?;
        
        println!("Initialized with session: {:?}", session.id);
        self.session = Some(session);
        let session_message = self.after_initialize().await;
        
        if let Some(message) = session_message {
            println!("Sending session message: {:?}", message);
            self.write_message(&message).await?;
        }
        Ok(())
    }

    async fn write_message(&mut self, msg: &LspMessage) -> io::Result<()> {
        let content = serde_json::to_string(&msg)?;
        let header = format!("Content-Length: {}\r\n\r\n", content.len());

        self.writer.write_all(header.as_bytes()).await?;
        self.writer.write_all(content.as_bytes()).await?;
        self.writer.flush().await?;

        Ok(())
    }

    async fn read_message(&mut self) -> io::Result<Option<LspMessage>> {
        // Read header - using manual implementation since we need to handle line-by-line reading
        // without relying on AsyncBufReadExt::read_line
        let mut content_length: Option<usize> = None;
        let mut buf = Vec::new();
        let mut last_was_cr = false;

        // Read headers until we find an empty line (CR+LF followed by CR+LF)
        let mut headers_done = false;
        while !headers_done {
            let mut byte = [0u8; 1];
            match self.reader.read_exact(&mut byte).await {
                Ok(_) => {
                    if byte[0] == b'\r' {
                        last_was_cr = true;
                    } else if byte[0] == b'\n' && last_was_cr {
                        // End of line
                        let line = String::from_utf8_lossy(&buf).to_string();
                        buf.clear();

                        // If this was an empty line, it signals the end of headers
                        if line.is_empty() {
                            headers_done = true;
                        }
                        // Try to parse Content-Length header
                        else if line.starts_with("Content-Length:") {
                            if let Some(len_str) = line.split(':').nth(1) {
                                if let Ok(len) = len_str.trim().parse::<usize>() {
                                    content_length = Some(len);
                                }
                            }
                        }

                        last_was_cr = false;
                    } else {
                        if last_was_cr {
                            buf.push(b'\r');
                        }
                        buf.push(byte[0]);
                        last_was_cr = false;
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        return Ok(None); // Client disconnected
                    }
                    return Err(e);
                }
            }
        }

        // Read content
        if let Some(len) = content_length {
            let mut content = vec![0; len];
            self.reader.read_exact(&mut content).await?;

            // Parse as JSON
            match serde_json::from_slice::<LspMessage>(&content) {
                Ok(msg) => Ok(Some(msg)),
                Err(e) => {
                    eprintln!("Error parsing LSP message: {}", e);
                    Err(io::Error::new(io::ErrorKind::InvalidData, e))
                }
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing Content-Length header",
            ))
        }
    }

    pub async fn process(&mut self) -> Result<()> {
        loop {
            
            let Some(message) = self.read_message().await? else {
                return Err(anyhow!("No message received"));
            };
            let message_id = message.id.clone();

            let result = self.handle_message(message).await;
            match result {
                LspHandlerResult::Exit => {
                                return Ok(());
                            },
                LspHandlerResult::NoResponse => {
                                continue;
                            },
                _ => {
                    let messages = process_handler_result(result, message_id).await;
                    for message in messages {
                        self.write_message(&message).await?;
                    }
                }
            }
        }
    }

    async fn handle_message(&mut self, message: LspMessage) -> LspHandlerResult {
        let method = match message.method.as_deref() {
            Some(m) => m,
            None => return LspHandlerResult::Error("Missing method in message".into()),
        };
    
        // Requests: method + id
        if let Some(id) = message.id {
            match method {
                "initialize" => {
                    LspHandlerResult::Error("Already initialized (or invalid duplicate initialize request)".into())
                }
                "shutdown" => {
                    LspHandlerResult::Response(self.handle_shutdown(id).await)
                }
                "textDocument/completion" => {
                    match message.params {
                        Some(params) => LspHandlerResult::Response(self.handle_completion(id, params).await),
                        None => LspHandlerResult::Error("Missing params for completion request".into()),
                    }
                }
                "textDocument/hover" => {
                    match message.params {
                        Some(params) => LspHandlerResult::Response(self.handle_hover(id, params).await),
                        None => LspHandlerResult::Error("Missing params for hover request".into()),
                    }
                },
                "textDocument/documentSymbol" => {
                    match message.params {
                        Some(params) => match self.handle_document_symbol(id, params).await {
                            Ok(msg) => LspHandlerResult::Response(msg),
                            Err(e) => LspHandlerResult::Error(e.to_string()),
                        },
                        None => LspHandlerResult::Error("Missing params for documentSymbol request".into()),
                    }
                },
                "textDocument/definition" => {
                    match message.params {
                        Some(params) => match self.handle_definition(id, params).await {
                            Ok(msg) => LspHandlerResult::Response(msg),
                            Err(e) => LspHandlerResult::Error(e),
                        },
                        None => LspHandlerResult::Error("Missing params for definition request".into()),
                    }
                },
                _ if method.starts_with("$/") => LspHandlerResult::NoResponse,
                _ => LspHandlerResult::Error(format!("Request method not found: {}", method)),
            }
        } else {
            // Notifications: method only
            match method {
                "textDocument/didOpen" => {
                    match message.params {
                        Some(params) => match self.handle_text_document_did_open(params).await {
                            Ok(diags) => LspHandlerResult::PublishDiagnostics(diags),
                            Err(e) => LspHandlerResult::Error(e),
                        },
                        None => LspHandlerResult::Error("Missing params for didOpen notification".into()),
                    }
                }
                "textDocument/didChange" => {
                    match message.params {
                        Some(params) => match self.handle_text_document_did_change(params).await {
                            Ok(diags) => LspHandlerResult::PublishDiagnostics(diags),
                            Err(e) => LspHandlerResult::Error(e),
                        },
                        None => LspHandlerResult::Error("Missing params for didChange notification".into()),
                    }
                }
                "textDocument/didClose" => {
                    match message.params {
                        Some(params) => match self.handle_text_document_did_close(params).await {
                            Ok(uri) => LspHandlerResult::ClearDiagnostics(uri),
                            Err(e) => LspHandlerResult::Error(e),
                        },
                        None => LspHandlerResult::Error("Missing params for didClose notification".into()),
                    }
                },
                

                "initialized" => LspHandlerResult::NoResponse,
                "exit" => LspHandlerResult::Exit,
                _ if method.starts_with("$/") => LspHandlerResult::NoResponse,
                _ => LspHandlerResult::Error(format!("Notification method not found: {}", method)),
            }
        }
    }

    async fn handle_definition(&self, id: Value, params: Value) -> Result<LspMessage, String> {
        let session = self.session.as_ref().ok_or("Session not initialized")?;
        let parsed = serde_json::from_value::<GotoDefinitionParams>(params)
            .map_err(|e| format!("Failed to parse params: {}", e))?;
    
        let uri = parsed.text_document_position_params.text_document.uri;
        let position = parsed.text_document_position_params.position;
    
        let doc = session.documents.read();
        let text = doc
            .get(&uri)
            .map(|doc| &doc.text)
            .ok_or("Document not found")?;
    
        let Some(symbol_name) = find_symbol_at_position(text, &position) else {
            return Ok(LspMessage {
                jsonrpc: "2.0".to_string(),
                id: Some(id),
                method: None,
                params: None,
                result: Some(json!(null)),
                error: None,
            });
        };
    
        let symbols = session.symbols.read();
        if let Some(info) = symbols.get(&symbol_name) {
            if let Some(range) = &info.position {
                let lsp_range = Range::from(range.clone());
                return Ok(LspMessage {
                    jsonrpc: "2.0".to_string(),
                    id: Some(id),
                    method: None,
                    params: None,
                    result: Some(json!([{
                        "uri": uri,
                        "range": lsp_range
                    }])),
                    error: None,
                });
            }
        }
    
        Ok(LspMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(json!(null)),
            error: None,
        })
    }
    
    

    async fn handle_initialize(&mut self, id: Value) -> LspMessage {
        // Respond with server capabilities
        let capabilities = create_server_capabilities();

        LspMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(json!({
                "capabilities": capabilities,
                "serverInfo": {
                    "name": "Blink LSP Server",
                    "version": "0.1.0"
                },
                
            })),
            error: None,
        }
    }

    async fn after_initialize(&mut self) -> Option<LspMessage> {
        self.session.as_ref().map(|s| {
            LspMessage {
                jsonrpc: "2.0".to_string(),
                id: None,
                method: Some("$/blinkSessionInfo".to_string()),
                params: Some(json!({
                    "sessionId": s.id.clone()
                })),
                result: None,
                error: None,
            }
        }) 
    }

    async fn handle_shutdown(&self, id: Value) -> LspMessage {
        LspMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(json!(null)),
            error: None,
        }
    }

    async fn handle_text_document_did_open(
        &mut self,
        params: Value,
    ) -> Result<DiagnosticsParams, String> {
        
    
        let open_params = serde_json::from_value::<DidOpenTextDocumentParams>(params)
            .map_err(|e| format!("Failed to parse didOpen params: {}", e))?;
    
        let uri = open_params.text_document.uri;
        let text = open_params.text_document.text;
    
        // Clone early so we can reuse `uri` later
        let uri_clone = uri.clone();
        let source = SymbolSource::File(uri_clone.clone());
    
        match parse_all(&text) {
            Ok(forms) => {
                
                let new_doc = Document {
                    uri: uri_clone.clone(),
                    text,
                    forms,
                };

                
                if let Some(session) = &self.session {
                    {
                        let mut symbols = session.symbols.write();
                        collect_symbols_from_forms(&mut symbols, &new_doc.forms, source);
                    }
                    {
                        let mut documents = session.documents.write();
                        documents.insert(uri_clone.clone(), new_doc);
                    }
                } else {
                    return Err("Session not initialized".to_string());
                };
    
                Ok(DiagnosticsParams {
                    uri: uri_clone,
                    diagnostics: Vec::new(),
                })
            }
            Err(err) => {
                let diagnostic = error_to_diagnostic(&err, &uri);
    
                Ok(DiagnosticsParams {
                    uri,
                    diagnostics: vec![diagnostic],
                })
            }
        }
    }
    

    async fn handle_text_document_did_change(
        &mut self,
        params: Value,
    ) -> Result<DiagnosticsParams, String> {
        let Some(session) = self.session.clone() else {
            return Err("Session not initialized".to_string());
        };
        let change_params = serde_json::from_value::<DidChangeTextDocumentParams>(params)
            .map_err(|e| format!("Failed to parse didChange params: {}", e))?;

        let uri = change_params.text_document.uri;

        // Get the current document
        let current_doc = {
            let documents = session.documents.read();
                documents
                    .get(&uri)
                    .cloned() // <----- THIS ensures we don't hold the borrow
                    .ok_or_else(|| format!("Document not found: {}", uri))?
        };
        let mut current_text = current_doc.text.clone();

        // Apply changes
        for change in change_params.content_changes {
            if let Some(range) = change.range {
                // Incremental update (replace a range)
                let start_pos =
                    compute_position(&current_text, range.start.line, range.start.character);
                let end_pos = compute_position(&current_text, range.end.line, range.end.character);

                if start_pos <= end_pos && end_pos <= current_text.len() {
                    current_text = format!(
                        "{}{}{}",
                        &current_text[0..start_pos],
                        change.text,
                        &current_text[end_pos..]
                    );
                }
            } else {
                // Full document update
                current_text = change.text;
            }
        }
        let source = SymbolSource::File(uri.clone());
        let forms = parse_all(&current_text);
        match forms {
            Ok(forms) =>  // Update document in session
                    {
                        
                        let new_doc = Document {
                    
                            text: current_text.clone(),
                            forms,
                            uri,
                        };
                        let mut documents = session.documents.write();
                        let diag = DiagnosticsParams {
                            uri: new_doc.uri.clone(),
                            diagnostics: Vec::new(),
                        };
                        // Update symbols
                        let mut symbols = session.symbols.write();
                        collect_symbols_from_forms(&mut symbols, &new_doc.forms, source);
                        documents.insert(new_doc.uri.clone(), new_doc);
                        
                        

                        // No diagnostics to publish
                        Ok(diag)
                    },
            Err(e) => {
                // Create diagnostics for error
                let diagnostic = error_to_diagnostic(&e, &uri);

                Ok(DiagnosticsParams {
                    uri,
                    diagnostics: vec![diagnostic],
                })
            }
            
        }
    }

    async fn handle_text_document_did_close(&mut self, params: Value) -> Result<String, String> {
        let Some(session) = &self.session else {
            return Err("Session not initialized".to_string());
        };
        let close_params = serde_json::from_value::<DidCloseTextDocumentParams>(params)
            .map_err(|e| format!("Failed to parse didClose params: {}", e))?;

        let uri = close_params.text_document.uri;

        // Remove document from session
        {
            let mut documents = session.documents.write();
            documents.remove(&uri);
        }

        // Return the URI to clear diagnostics
        Ok(uri)
    }
    
    
    
    

    

    

    fn empty_completion_response(&self, id: Value) -> LspMessage {
        LspMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(json!(null)),
            error: None,
        }
    }

    fn get_prefix_at_position(&self, text: &str, position: &Position) -> Option<String> {
        let lines: Vec<&str> = text.lines().collect();
        let line = lines.get(position.line as usize)?;
        let char_index = position.character as usize;
    
        // Collect chars up to the cursor into a Vec so we can reverse safely
        let chars_before_cursor: Vec<char> = line.chars().take(char_index).collect();
    
        let is_symbol_char = |c: char| c.is_alphanumeric() || c == '-' || c == '_' || c == '?' || c == '!';
    
        let prefix: String = chars_before_cursor
            .iter()
            .rev()
            .take_while(|&&c| is_symbol_char(c))
            .copied()
            .collect::<Vec<char>>()
            .into_iter()
            .rev()
            .collect();
    
        if prefix.is_empty() {
            None
        } else {
            Some(prefix)
        }
    }
    
    

    async fn handle_completion(&self, id: Value, params: Value) -> LspMessage {
        let params = serde_json::from_value::<CompletionParams>(params).unwrap();

        let uri = params.text_document.uri;
        let position = params.position;
        

        let session = self.session.as_ref().unwrap();
        let document= {
            let docs = session.documents.read();
            match docs.get(&uri) {
                Some(text) => text.clone(),
                None => return self.empty_completion_response(id),
            }
        };

        let mut completion_items = Vec::new();
        let prefix_str = self.get_prefix_at_position(&document.text, &position);
    
        // Add built-in special forms
        for &special_form in &[
            "def", "fn", "if", "quote", "do", "let", "and", "or", "try", "imp", "apply",
        ] {
            completion_items.push(CompletionItem {
                label: special_form.to_string(),
                kind: Some(3), // Function
                detail: Some("Blink Special Form".into()),
                documentation: get_special_form_doc(special_form),
                insert_text: None,
            });
        }
    
        // Add symbols from the environment
        let session = self.session.as_deref().unwrap();
        let symbols = session.symbols.read();
        
        for (sym, ty) in symbols.iter() {
            if let Some(prefix) = &prefix_str {
                if !sym.starts_with(prefix) {
                    continue;
                }
            }
            
            completion_items.push(CompletionItem {
                label: sym.clone(),
                kind: Some(6), // Variable
                detail: Some(format!("Type: {}", ty.kind)),
                documentation: None,
                insert_text: None,
            });
        }
    
        // Add native functions
        for (func, doc) in get_native_function_docs() {
            if let Some(prefix) = &prefix_str {
                if !func.starts_with(prefix) {
                    continue;
                }
            }
            completion_items.push(CompletionItem {
                label: func.to_string(),
                kind: Some(3), // Function
                detail: Some("Built-in Function".into()),
                documentation: Some(doc),
                insert_text: None,
            });
        }
    
    
        LspMessage {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(json!({
                "isIncomplete": false,
                "items": completion_items
            })),
            error: None,
        }
    }
    

    async fn handle_hover(&self, id: Value, params: Value) -> LspMessage {
        let Some(session) = &self.session else {
            return create_empty_hover_response(id);
        };
        if let Ok(hover_params) = serde_json::from_value::<HoverParams>(params) {
            // Extract the document and position
            let uri = hover_params.text_document.uri;
            let position = hover_params.position;

            // Get document text
            let document = {
                let documents = session.documents.read();
                match documents.get(&uri) {
                    Some(text) => text.clone(),
                    None => return create_empty_hover_response(id),
                }
            };

            // Find the symbol at this position
            if let Some(symbol) = find_symbol_at_position(&document.text, &position) {
                // Check for special forms
                if let Some(doc) = get_special_form_doc(&symbol) {
                    create_hover_response(id, &doc)
                }
                // Check for native functions
                else if let Some(doc) = get_native_function_doc(&symbol) {
                    create_hover_response(id, &doc)
                }
                // Check user-defined symbols
                else if let Some(type_info) = session.symbols.read().get(&symbol) {
                    if let Some(rep) = &type_info.representation {
                        create_hover_response(id, &rep)
                    } else {
                        create_hover_response(id, &format!("**{}**\nType: {}", symbol, type_info.kind))
                    }
                }
                // Symbol not found
                else {
                    create_empty_hover_response(id)
                }
            } else {
                create_empty_hover_response(id)
            }
        } else {
            create_empty_hover_response(id)
        }
    }
    async fn handle_document_symbol(&self, id: Value, params: Value) -> Result<LspMessage, String> {

        
    
        let parsed: Result<DocumentSymbolParams, _> = serde_json::from_value(params);
        if let Ok(DocumentSymbolParams { text_document }) = parsed {

            
            let uri = text_document.uri;
            let session = self.session.as_ref().unwrap();

            let symbols = session.symbols.read();
            let result = symbols.iter()
                .filter_map(|(name, info)| {
                    let range: Range = info.position.as_ref().map(|pos| pos.clone().into()).unwrap_or(create_default_range());
                    info.position.as_ref().map(|pos| {
                        json!({
                            "name": name,
                            "kind": info.kind.to_lsp_symbol_kind(),
                            "range": range,
                            "selectionRange": range
            })
        })
    })
    .collect::<Vec<_>>();

    
            let json_result = serde_json::to_value(result).map_err(|e| format!("Failed to serialize document symbol result: {}", e))?;
            return Ok(LspMessage {
                jsonrpc: "2.0".to_string(),
                id: Some(id),
                method: None,
                params: None,
                result: Some(json_result),
                error: None,
            })
        }
    
        Err("Invalid documentSymbol parameters".to_string())
    }
    
}

fn find_symbol_at_position(text: &str, position: &Position) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();

    // Ensure position is within bounds
    if position.line as usize >= lines.len() {
        return None;
    }

    let line = lines[position.line as usize];

    // Ensure character position is within bounds
    if position.character as usize >= line.len() {
        return None;
    }

    // Find word boundaries
    let is_symbol_char =
        |c: char| c.is_alphanumeric() || c == '-' || c == '_' || c == '?' || c == '!';

    let start = line[..position.character as usize]
        .chars()
        .rev()
        .take_while(|&c| is_symbol_char(c))
        .count();

    let end = line[position.character as usize..]
        .chars()
        .take_while(|&c| is_symbol_char(c))
        .count();

    if start == 0 && end == 0 {
        return None;
    }

    let start_idx = position.character as usize - start;
    let end_idx = position.character as usize + end;

    Some(line[start_idx..end_idx].to_string())
}

fn get_special_form_doc(symbol: &str) -> Option<String> {
    match symbol {
        "def" => Some("**def** - Define a variable\n\n```blink\n(def name value)\n```\n\nAssigns `value` to `name` in the current environment.".to_string()),
        "fn" => Some("**fn** - Create a function\n\n```blink\n(fn [param1 param2 ...] body)\n```\n\nCreates a new function with the specified parameters and body expressions.".to_string()),
        "if" => Some("**if** - Conditional expression\n\n```blink\n(if condition then-expr else-expr)\n```\n\nEvaluates `condition` and returns `then-expr` if truthy, otherwise returns `else-expr`.".to_string()),
        "do" => Some("**do** - Sequence of expressions\n\n```blink\n(do expr1 expr2 ... exprN)\n```\n\nEvaluates each expression in order and returns the value of the last one.".to_string()),
        "let" => Some("**let** - Local bindings\n\n```blink\n(let [name1 value1, name2 value2 ...] body)\n```\n\nCreates local bindings that are available within the body expressions.".to_string()),
        "quote" => Some("**quote** - Prevent evaluation\n\n```blink\n(quote expr)\n'expr\n```\n\nReturns the expression without evaluating it.".to_string()),
        "and" => Some("**and** - Logical AND\n\n```blink\n(and expr1 expr2 ... exprN)\n```\n\nReturns the first falsy value or the last value if all are truthy.".to_string()),
        "or" => Some("**or** - Logical OR\n\n```blink\n(or expr1 expr2 ... exprN)\n```\n\nReturns the first truthy value or the last value if all are falsy.".to_string()),
        "try" => Some("**try** - Error handling\n\n```blink\n(try expr recovery-expr)\n```\n\nEvaluates `expr` and returns its value. If an error occurs, evaluates and returns `recovery-expr`.".to_string()),
        "apply" => Some("**apply** - Apply function to arguments\n\n```blink\n(apply fn arg-list)\n```\n\nApplies the function to the list of arguments.".to_string()),
        "imp" => Some("**imp** - Import module\n\n```blink\n(imp \"module-path\")\n```\n\nImports the specified module, evaluating all forms in it.".to_string()),
        "nimp" => Some("**nimp** - Native import\n\n```blink\n(nimp \"library-name\")\n```\n\nImports a native Rust library (.so/.dll) and registers its functions.".to_string()),
        _ => None,
    }
}

fn get_native_function_doc(symbol: &str) -> Option<String> {
    match symbol {
        "+" => Some("**+** - Addition\n\n```blink\n(+ num1 num2 ...)\n```\n\nAdds numbers together.".to_string()),
        "-" => Some("**-** - Subtraction\n\n```blink\n(- num1 num2 ...)\n```\n\nSubtracts numbers from the first one.".to_string()),
        "*" => Some("***** - Multiplication\n\n```blink\n(* num1 num2 ...)\n```\n\nMultiplies numbers together.".to_string()),
        "/" => Some("**/** - Division\n\n```blink\n(/ num1 num2 ...)\n```\n\nDivides the first number by the rest.".to_string()),
        "=" => Some("**=** - Equality\n\n```blink\n(= val1 val2 ...)\n```\n\nChecks if all values are equal.".to_string()),
        "not" => Some("**not** - Logical NOT\n\n```blink\n(not expr)\n```\n\nReturns true if expr is falsy, false otherwise.".to_string()),
        "map" => Some("**map** - Apply function to each item\n\n```blink\n(map fn coll)\n```\n\nApplies the function to each item in the collection.".to_string()),
        "reduce" => Some("**reduce** - Reduce collection to a value\n\n```blink\n(reduce fn init coll)\n```\n\nReduces the collection to a single value using the function.".to_string()),
        "list" => Some("**list** - Create a list\n\n```blink\n(list item1 item2 ...)\n```\n\nCreates a new list containing the given items.".to_string()),
        "vector" => Some("**vector** - Create a vector\n\n```blink\n(vector item1 item2 ...)\n```\n\nCreates a new vector containing the given items.".to_string()),
        "hash-map" => Some("**hash-map** - Create a map\n\n```blink\n(hash-map key1 val1 key2 val2 ...)\n```\n\nCreates a new hash map with the given keys and values.".to_string()),
        "print" => Some("**print** - Output values\n\n```blink\n(print val1 val2 ...)\n```\n\nPrints values to standard output.".to_string()),
        "type-of" => Some("**type-of** - Get type\n\n```blink\n(type-of value)\n```\n\nReturns the type of the given value as a string.".to_string()),
        "cons" => Some("**cons** - Prepend to list\n\n```blink\n(cons item list)\n```\n\nPrepends an item to the beginning of a list.".to_string()),
        "car" => Some("**car** - First item\n\n```blink\n(car list)\n```\n\nReturns the first item of a list.".to_string()),
        "cdr" => Some("**cdr** - Rest of list\n\n```blink\n(cdr list)\n```\n\nReturns all items except the first one from a list.".to_string()),
        "first" => Some("**first** - First item\n\n```blink\n(first list)\n```\n\nAlias for car. Returns the first item of a list.".to_string()),
        "rest" => Some("**rest** - Rest of list\n\n```blink\n(rest list)\n```\n\nAlias for cdr. Returns all items except the first one from a list.".to_string()),
        "get" => Some("**get** - Get by key/index\n\n```blink\n(get collection key [default])\n```\n\nGets the value at the key/index in the collection, or returns default if not found.".to_string()),
        _ => None,
    }
}

fn get_native_function_docs() -> Vec<(&'static str, String)> {
    vec![
        ("+", get_native_function_doc("+").unwrap()),
        ("-", get_native_function_doc("-").unwrap()),
        ("*", get_native_function_doc("*").unwrap()),
        ("/", get_native_function_doc("/").unwrap()),
        ("=", get_native_function_doc("=").unwrap()),
        ("not", get_native_function_doc("not").unwrap()),
        ("map", get_native_function_doc("map").unwrap()),
        ("reduce", get_native_function_doc("reduce").unwrap()),
        ("list", get_native_function_doc("list").unwrap()),
        ("vector", get_native_function_doc("vector").unwrap()),
        ("hash-map", get_native_function_doc("hash-map").unwrap()),
        ("print", get_native_function_doc("print").unwrap()),
        ("type-of", get_native_function_doc("type-of").unwrap()),
        ("cons", get_native_function_doc("cons").unwrap()),
        ("car", get_native_function_doc("car").unwrap()),
        ("cdr", get_native_function_doc("cdr").unwrap()),
        ("first", get_native_function_doc("first").unwrap()),
        ("rest", get_native_function_doc("rest").unwrap()),
        ("get", get_native_function_doc("get").unwrap()),
    ]
}

// Helper function to compute byte position in string from line/column
fn compute_position(text: &str, line: u32, character: u32) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let line_idx = line as usize;

    if line_idx >= lines.len() {
        return text.len();
    }

    let mut pos = 0;

    // Add length of all previous lines plus newlines
    for i in 0..line_idx {
        pos += lines[i].len() + 1; // +1 for newline character
    }

    // Add characters in current line
    let char_pos = character as usize;
    let line_len = lines[line_idx].len();

    pos + std::cmp::min(char_pos, line_len)
}

/// Create an empty hover response
fn create_empty_hover_response(id: Value) -> LspMessage {
    LspMessage {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        method: None,
        params: None,
        result: Some(json!(null)),
        error: None,
    }
}

/// Create a hover response with content
fn create_hover_response(id: Value, content: &str) -> LspMessage {
    LspMessage {
        jsonrpc: "2.0".to_string(),
        id: Some(id),
        method: None,
        params: None,
        result: Some(json!({
            "contents": {
                "kind": "markdown",
                "value": content
            }
        })),
        error: None,
    }
}

/// Get documentation for special forms
fn get_special_forms() -> Vec<(&'static str, String)> {
    vec![
        ("def", "**def** - Define a variable\n\n```blink\n(def name value)\n```\n\nAssigns `value` to `name` in the current environment.".to_string()),
        ("fn", "**fn** - Create a function\n\n```blink\n(fn [param1 param2 ...] body)\n```\n\nCreates a new function with the specified parameters and body expressions.".to_string()),
        ("if", "**if** - Conditional expression\n\n```blink\n(if condition then-expr else-expr)\n```\n\nEvaluates `condition` and returns `then-expr` if truthy, otherwise returns `else-expr`.".to_string()),
        ("do", "**do** - Sequence of expressions\n\n```blink\n(do expr1 expr2 ... exprN)\n```\n\nEvaluates each expression in order and returns the value of the last one.".to_string()),
        ("let", "**let** - Local bindings\n\n```blink\n(let [name1 value1, name2 value2 ...] body)\n```\n\nCreates local bindings that are available within the body expressions.".to_string()),
        ("quote", "**quote** - Prevent evaluation\n\n```blink\n(quote expr)\n'expr\n```\n\nReturns the expression without evaluating it.".to_string()),
        ("and", "**and** - Logical AND\n\n```blink\n(and expr1 expr2 ... exprN)\n```\n\nReturns the first falsy value or the last value if all are truthy.".to_string()),
        ("or", "**or** - Logical OR\n\n```blink\n(or expr1 expr2 ... exprN)\n```\n\nReturns the first truthy value or the last value if all are falsy.".to_string()),
        ("try", "**try** - Error handling\n\n```blink\n(try expr recovery-expr)\n```\n\nEvaluates `expr` and returns its value. If an error occurs, evaluates and returns `recovery-expr`.".to_string()),
        ("apply", "**apply** - Apply function to arguments\n\n```blink\n(apply fn arg-list)\n```\n\nApplies the function to the list of arguments.".to_string()),
        ("imp", "**imp** - Import module\n\n```blink\n(imp \"module-path\")\n```\n\nImports the specified module, evaluating all forms in it.".to_string()),
        ("nimp", "**nimp** - Native import\n\n```blink\n(nimp \"library-name\")\n```\n\nImports a native Rust library (.so/.dll) and registers its functions.".to_string()),
    ]
}

/// Get documentation for built-in functions
fn get_builtin_functions() -> Vec<(&'static str, String)> {
    vec![
        ("+", "**+** - Addition\n\n```blink\n(+ num1 num2 ...)\n```\n\nAdds numbers together.".to_string()),
        ("-", "**-** - Subtraction\n\n```blink\n(- num1 num2 ...)\n```\n\nSubtracts numbers from the first one.".to_string()),
        ("*", "***** - Multiplication\n\n```blink\n(* num1 num2 ...)\n```\n\nMultiplies numbers together.".to_string()),
        ("/", "**/** - Division\n\n```blink\n(/ num1 num2 ...)\n```\n\nDivides the first number by the rest.".to_string()),
        ("=", "**=** - Equality\n\n```blink\n(= val1 val2 ...)\n```\n\nChecks if all values are equal.".to_string()),
        ("not", "**not** - Logical NOT\n\n```blink\n(not expr)\n```\n\nReturns true if expr is falsy, false otherwise.".to_string()),
        ("map", "**map** - Apply function to each item\n\n```blink\n(map fn coll)\n```\n\nApplies the function to each item in the collection.".to_string()),
        ("reduce", "**reduce** - Reduce collection to a value\n\n```blink\n(reduce fn init coll)\n```\n\nReduces the collection to a single value using the function.".to_string()),
        ("list", "**list** - Create a list\n\n```blink\n(list item1 item2 ...)\n```\n\nCreates a new list containing the given items.".to_string()),
        ("vector", "**vector** - Create a vector\n\n```blink\n(vector item1 item2 ...)\n```\n\nCreates a new vector containing the given items.".to_string()),
        ("hash-map", "**hash-map** - Create a map\n\n```blink\n(hash-map key1 val1 key2 val2 ...)\n```\n\nCreates a new hash map with the given keys and values.".to_string()),
        ("print", "**print** - Output values\n\n```blink\n(print val1 val2 ...)\n```\n\nPrints values to standard output.".to_string()),
        ("type-of", "**type-of** - Get type\n\n```blink\n(type-of value)\n```\n\nReturns the type of the given value as a string.".to_string()),
        ("cons", "**cons** - Prepend to list\n\n```blink\n(cons item list)\n```\n\nPrepends an item to the beginning of a list.".to_string()),
        ("car", "**car** - First item\n\n```blink\n(car list)\n```\n\nReturns the first item of a list.".to_string()),
        ("cdr", "**cdr** - Rest of list\n\n```blink\n(cdr list)\n```\n\nReturns all items except the first one from a list.".to_string()),
        ("first", "**first** - First item\n\n```blink\n(first list)\n```\n\nAlias for car. Returns the first item of a list.".to_string()),
        ("rest", "**rest** - Rest of list\n\n```blink\n(rest list)\n```\n\nAlias for cdr. Returns all items except the first one from a list.".to_string()),
        ("get", "**get** - Get by key/index\n\n```blink\n(get collection key [default])\n```\n\nGets the value at the key/index in the collection, or returns default if not found.".to_string()),
    ]
}

/// Convert a Blink error to an LSP diagnostic
fn error_to_diagnostic(err: &blink_core::error::LispError, uri: &str) -> Diagnostic {
    use blink_core::error::LispError;

    let (message, range) = match err {
        LispError::TokenizerError { message, pos } => {
            (message.clone(), create_range_from_position(pos, 1))
        }
        LispError::ParseError { message, pos } => {
            (message.clone(), pos.clone().into())
        }
        LispError::EvalError { message, pos } => {
            let range = pos
                .as_ref()
                .map(|p| p.clone().into())
                .unwrap_or_else(create_default_range);
            (message.clone(), range)
        }
        LispError::ArityMismatch {
            expected,
            got,
            form,
            pos,
        } => {
            let message = format!(
                "Wrong number of arguments to '{}': expected {}, got {}",
                form, expected, got
            );
            let range = pos
                .as_ref()
                .map(|p|p.clone().into())
                .unwrap_or_else(create_default_range);
            (message, range)
        }
        LispError::UndefinedSymbol { name, pos } => {
            let message = format!("Undefined symbol: {}", name);
            let range = pos
                .as_ref()
                .map(|p| p.clone().into())
                .unwrap_or_else(create_default_range);
            (message, range)
        }
        LispError::UnexpectedToken { token, pos } => (
            format!("Unexpected token: {}", token),
            create_range_from_position(pos, token.len()),
        ),
    };

    Diagnostic {
        range,
        severity: Some(1), // Error = 1, Warning = 2, Info = 3, Hint = 4
        code: None,
        source: Some("blink-lsp".to_string()),
        message,
    }
}

/// Create a Range from a SourcePos
fn create_range_from_position(pos: &SourcePos, length: usize) -> Range {
    Range {
        start: Position {
            line: (pos.line - 1) as u32,     // LSP uses 0-based lines
            character: (pos.col - 1) as u32, // LSP uses 0-based columns
        },
        end: Position {
            line: (pos.line - 1) as u32,
            character: (pos.col - 1 + length) as u32,
        },
    }
}

/// Create a default Range (position 0,0)
fn create_default_range() -> Range {
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 1,
        },
    }
}

/// Convert an LspHandlerResult to a vec of LspMessages ready to be sent over the socket
pub async fn process_handler_result(
    result: LspHandlerResult,
    request_id: Option<Value>,
) -> Vec<LspMessage> {
    match result {
        LspHandlerResult::Response(response) => {
            // Direct response to a request
            vec![response]
        }
        LspHandlerResult::PublishDiagnostics(diag_params) => {
            // Create diagnostic notification
            vec![LspMessage {
                jsonrpc: "2.0".to_string(),
                id: None, // Notifications don't have IDs
                method: Some("textDocument/publishDiagnostics".to_string()),
                params: Some(json!({
                    "uri": diag_params.uri,
                    "diagnostics": diag_params.diagnostics
                })),
                result: None,
                error: None,
            }]
        }
        LspHandlerResult::ClearDiagnostics(uri) => {
            // Create diagnostic notification with empty diagnostics array
            vec![LspMessage {
                jsonrpc: "2.0".to_string(),
                id: None,
                method: Some("textDocument/publishDiagnostics".to_string()),
                params: Some(json!({
                    "uri": uri,
                    "diagnostics": []
                })),
                result: None,
                error: None,
            }]
        }
        LspHandlerResult::Error(error_msg) => {
            if let Some(id) = request_id {
                // If there was a request ID, send an error response
                vec![LspMessage {
                    jsonrpc: "2.0".to_string(),
                    id: Some(id),
                    method: None,
                    params: None,
                    result: None,
                    error: Some(LspError {
                        code: -32603, // Internal error
                        message: error_msg.clone(),
                        data: None,
                    }),
                }]
            } else {
                // Log the error but don't send anything if there was no request ID
                eprintln!("LSP handler error (no request ID): {}", error_msg);
                vec![]
            }
        }
        LspHandlerResult::Exit | LspHandlerResult::NoResponse => vec![],
    }
}
