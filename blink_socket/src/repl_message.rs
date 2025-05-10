use blink_core::{telemetry::TelemetryEvent, value::{SourcePos, SourceRange}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ReplRequest {
    Init { 
        id: String,
        session_id: Option<String> 
    },
    Close,
    Eval { id: String, code: String, pos: Option<SourcePos> },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum ReplResponse {
    Initialized {
        id: String,
    },
    EvalResult {
        id: String,
        value: String,
    },
    Error {
        id: String,
        message: String,
    },
    Telemetry {
        id: String,
        event: TelemetryEvent,
    },
    ProfileUpdate {
        id: String,
        calls: u64,
        avg_time_us: u128,
        max_time_us: u128,
    },
}
