use crate::error::SourcePos;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct TelemetryEvent {
    pub form: String,
    pub duration_us: u128,
    pub result_type: String,
    pub result_size: Option<usize>,
    pub source: Option<SourcePos>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BlinkMessage {
    Eval {
        id: String,
        code: String,
    },
    Result {
        id: String,
        value: String,
    },
    Error {
        id: String,
        message: String,
    },
    Telemetry {
        id: String,
        form: String,
        duration_us: u128,
        result_type: String,
        result_size: Option<usize>,
        source: Option<(usize, usize)>,
    },
    ProfileUpdate {
        id: String,
        calls: u64,
        avg_time_us: u128,
        max_time_us: u128,
    },
}
