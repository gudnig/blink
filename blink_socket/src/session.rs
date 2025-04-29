use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use blink_core::eval::EvalContext;

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

pub struct Session {
    pub id: String,
    pub features: RwLock<SessionFeatures>,
    pub documents: RwLock<HashMap<String, String>>,
    pub eval_ctx: RwLock<Option<Arc<EvalContext>>>, // Only filled after REPL attached
}
