use crate::session::Session;
use blink_core::eval::EvalContext;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Thread-safe manager for active sessions.
#[derive(Default)]
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    saved_repl_sessions: RwLock<HashMap<String, Arc<RwLock<Option<Box<EvalContext>>>>>>

}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            saved_repl_sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new session
    pub async fn register(&self, session: Arc<Session>) {
        let mut sessions = self.sessions.write();
        sessions.insert(session.id.clone(), session);
    }

    /// Lookup an existing session
    pub async fn get(&self, session_id: &str) -> Option<Arc<Session>> {
        let sessions = self.sessions.read();
        sessions.get(session_id).cloned()
    }

    /// Remove a session
    pub async fn remove(&self, session_id: &str) {
        let mut sessions = self.sessions.write();
        sessions.remove(session_id);
    }

    /// List all sessions (optional, for debugging)
    pub async fn list_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.read();
        sessions.keys().cloned().collect()
    }
    pub async fn persist(&self, id: &str) -> Result<(), String> {
        let session = self.get(id).await.ok_or("Session not found.")?;
        

        let ctx = session.eval_ctx.clone();
        let mut saved = self.saved_repl_sessions.write();

        
        saved.insert(id.into(), ctx);

        Ok(())
    }

    pub async fn get_persisted(&self, id: &str) -> Option<Arc<RwLock<Option<Box<EvalContext>>>>> {
        let saved = self.saved_repl_sessions.read();
        saved.get(id).cloned()
    }
}
