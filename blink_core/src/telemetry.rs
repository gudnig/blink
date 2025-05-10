use crate::value::SourceRange;
use serde::{ Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TelemetryEvent {
    pub form: String,
    pub duration_us: u128,
    pub result_type: String,
    pub result_size: Option<usize>,
    pub source: Option<SourceRange>,
}
