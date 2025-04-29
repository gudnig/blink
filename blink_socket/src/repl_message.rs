use blink_core::telemetry::TelemetryEvent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ReplRequest {
    Init { session_id: Option<String> },
    Eval { id: String, code: String },
}

pub enum ReplResponse {
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
