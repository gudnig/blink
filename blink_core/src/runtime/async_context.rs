use std::sync::atomic::{AtomicU64};

use crate::runtime::GoroutineId;

#[derive(Clone, Debug, PartialEq)]
pub enum AsyncContext {
    Blocking,
    Goroutine(GoroutineId),
}


impl AsyncContext {

    
    
    pub fn is_blocking(&self) -> bool {
        matches!(self, AsyncContext::Blocking)
    }
    
    pub fn is_goroutine(&self) -> bool {
        matches!(self, AsyncContext::Goroutine(_))
    }
    
    pub fn goroutine_id(&self) -> Option<GoroutineId> {
        match self {
            AsyncContext::Goroutine(id) => Some(*id),
            _ => None,
        }
    }
    
    pub fn context_name(&self) -> &'static str {
        match self {
            AsyncContext::Blocking => "blocking",
            AsyncContext::Goroutine(_) => "goroutine",
        }
    }

    
    
}

impl Default for AsyncContext {
    fn default() -> Self {
        AsyncContext::Blocking
    }
}