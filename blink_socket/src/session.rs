use core::fmt;
use std::{collections::HashMap, fmt::Display, sync::Arc, time::Instant};

use blink_core::{ eval::EvalContext, runtime::SymbolTable, value::{ParsedValue, ParsedValueWithPos, SourceRange}};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::{lsp_messages::LspMessage, repl_message::ReplResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionFeatures {
    pub lsp: bool,
    pub repl: bool,
    pub telemetry: bool,
}

impl Default for SessionFeatures {
    fn default() -> Self {
        SessionFeatures {
            lsp: false,
            repl: false,
            telemetry: false,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    Repl(ReplResponse),
    Lsp(LspMessage),
}

pub struct ClientConnection {
    pub connected_at: Instant,
    pub last_activity: Instant,
    pub sender: tokio::sync::mpsc::Sender<ClientMessage>,
}

#[derive(Clone)]
pub struct SymbolInfo {
    pub kind: SymbolKind,            // e.g., Function, Macro, Var
    pub defined_in: SymbolSource,    // Repl, File(uri), Import
    pub position: Option<SourceRange>, // For LSP range
    pub representation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Number,
    String,
    Bool,
    Keyword,
    SymbolRef, // Renamed from Symbol to clarify it's a reference
    Variable,
    Function, 
    Macro,
    List,
    Vector,
    Map,
    Set,
    Error,
    Future,
    Module,
    Nil,
    Unknown,
}

impl SymbolKind {
    pub fn to_lsp_symbol_kind(&self) -> u32 {
        match self {
            SymbolKind::Function => 12,
            SymbolKind::Macro => 12,
            SymbolKind::Variable => 13,
            SymbolKind::Number | SymbolKind::String | SymbolKind::Bool => 14,
            SymbolKind::Map | SymbolKind::List | SymbolKind::Vector => 6,
            _ => 255,
        }
    }
}


impl Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone)]
pub enum SymbolSource {
    Repl,
    File(String), // URI
    Imported(String), // module name
}

#[derive(Clone)]

pub struct Document {
    pub uri: String,
    pub text: String,
    pub forms: Vec<ParsedValueWithPos>,
}

pub struct Session {
    pub id: String,
    pub features: RwLock<SessionFeatures>,
    pub documents: RwLock<HashMap<String, Document>>,
    // Only filled after REPL attached
    pub eval_ctx: Arc<RwLock<Option<Box<EvalContext>>>>,
    pub connected_at: RwLock<Instant>,
    pub last_activity: RwLock<Instant>,
    pub symbols: RwLock<HashMap<String, SymbolInfo>>,
    pub symbol_table: Arc<RwLock<SymbolTable>>,
}

impl Session {
    pub fn new(id: String, symbol_table: Arc<RwLock<SymbolTable>>) -> Self {
        Self {
            id: id,
            features: RwLock::new(SessionFeatures::default()),
            documents: RwLock::new(HashMap::new()),
            eval_ctx: Arc::new(RwLock::new(None)),
            symbols: RwLock::new(HashMap::new()),   
            connected_at: RwLock::new(Instant::now()),
            last_activity: RwLock::new(Instant::now()),
            symbol_table: symbol_table,
            
        }
    }
}
