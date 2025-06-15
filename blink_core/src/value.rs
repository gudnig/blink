use crate::env::Env;
use crate::{error::{BlinkError}};
use crate::future::BlinkFuture;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::hash::{Hash, Hasher};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct SourcePos {
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for SourcePos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.col)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct SourceRange {
    pub start: SourcePos,
    pub end: SourcePos,
}

impl std::fmt::Display for SourceRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}