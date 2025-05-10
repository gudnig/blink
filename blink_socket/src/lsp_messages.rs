use blink_core::value::{SourcePos, SourceRange};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Represents a JSON-RPC 2.0 message for the Language Server Protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspMessage {
    /// The JSON-RPC protocol version (always "2.0")
    pub jsonrpc: String,

    /// The message ID used to match requests with responses
    /// - Not present in notifications
    /// - Present in requests (sent by client)
    /// - Present in responses (sent by server, must match request ID)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,

    /// The method name (e.g., "textDocument/hover")
    /// - Present in requests and notifications
    /// - Not present in responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,

    /// The method parameters
    /// - Present in requests and notifications
    /// - Not present in responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    /// The result of a successful request
    /// - Present in successful responses
    /// - Not present in requests, notifications, or error responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Error information
    /// - Present in error responses
    /// - Not present in successful responses, requests, or notifications
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LspError>,
}

/// Represents an error in an LSP response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspError {
    /// The error code
    pub code: i32,

    /// A human-readable error message
    pub message: String,

    /// Additional error data (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionOptions {
    pub resolve_provider: bool,
    pub trigger_characters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub text_document_sync: u8,
    pub completion_provider: Option<CompletionOptions>,
    pub hover_provider: Option<bool>,
    pub definition_provider: Option<bool>,
    pub references_provider: Option<bool>,
    pub document_symbol_provider: Option<bool>,
    pub workspace_symbol_provider: Option<bool>,
    pub document_formatting_provider: Option<bool>,
}

pub fn create_server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Text Document Sync:
        // 0 = None: no syncing
        // 1 = Full: full document sent on changes
        // 2 = Incremental: only changes are sent
        text_document_sync: 2, // Incremental sync for better performance

        // Completion: provides autocomplete suggestions
        completion_provider: Some(CompletionOptions {
            // Whether the server resolves additional information for completions
            resolve_provider: true,

            // Characters that trigger completion - relevant for Blink
            trigger_characters: vec![
                "(".to_string(), // Function/form invocation
                " ".to_string(), // Space within forms
                "-".to_string(), // Part of symbol names
                ":".to_string(), // Keywords
            ],
        }),

        // Hover: provides documentation on hover
        hover_provider: Some(true),

        // Definition: jump to definition
        // Enabled as a future capability - currently limited implementation
        definition_provider: Some(true),

        // References: find references to symbols
        // Disabled for now, can enable when implemented
        references_provider: Some(false),

        // Document Symbols: outline of document symbols
        // Important for Blink for showing defined functions and variables
        document_symbol_provider: Some(true),

        // Workspace Symbols: search symbols across workspace
        // Disabled for now, enable when module/project support is added
        workspace_symbol_provider: Some(false),

        // Document Formatting: code formatting
        // Disabled for now, enable when formatter is implemented
        document_formatting_provider: Some(false),
    }
}

// LSP structures needed for message handling
#[derive(Clone,Debug, serde::Serialize, serde::Deserialize)]
pub struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: i32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentContentChangeEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_length: Option<u32>,
    pub text: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidChangeTextDocumentParams {
    pub text_document: VersionedTextDocumentIdentifier,
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidCloseTextDocumentParams {
    pub text_document: TextDocumentIdentifier,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidOpenTextDocumentParams {
    pub text_document: TextDocumentItem,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentItem {
    pub uri: String,
    pub language_id: String,
    pub version: i32,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl From<SourcePos> for Position {
    fn from(pos: SourcePos) -> Self {
        Position {
            line: (pos.line - 1) as u32,
            character: (pos.col - 1) as u32,
        }
    }
}


impl From<SourceRange> for Range {
    fn  from(range: SourceRange) -> Self {
        Range { start: range.start.into(), end: range.end.into() }
    }
}

impl From<Position> for SourcePos {
    fn from(pos: Position) -> Self {
        SourcePos { line: pos.line as usize, col: pos.character as usize }
    }
}

impl From<Range> for SourceRange {
    fn from(range: Range) -> Self {
        SourceRange { start: range.start.into(), end: range.end.into() }
    }
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<CompletionContext>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionContext {
    pub trigger_kind: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_character: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GotoDefinitionParams {
    #[serde(flatten)]
    pub text_document_position_params: TextDocumentPositionParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentPositionParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}



#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbolParams {
    pub text_document: TextDocumentIdentifier,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: u32,
    pub range: Range,
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<DocumentSymbol>>,
}




/// LSP diagnostic structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Option<i32>,
    pub code: Option<String>,
    pub source: Option<String>,
    pub message: String,
}

/// Parameters for publishing diagnostics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsParams {
    pub uri: String,
    pub diagnostics: Vec<Diagnostic>,
}