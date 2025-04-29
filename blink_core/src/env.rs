use crate::value::BlinkValue;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Env {
    vars: HashMap<String, BlinkValue>,
    parent: Option<Arc<RwLock<Env>>>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: Arc<RwLock<Env>>) -> Self {
        Env {
            vars: HashMap::new(),
            parent: Some(parent),
        }
    }

    pub fn set(&mut self, key: &str, val: BlinkValue) {
        self.vars.insert(key.to_string(), val);
    }

    pub fn get(&self, key: &str) -> Option<BlinkValue> {
        match self.vars.get(key) {
            Some(val) => Some(val.clone()),
            None => self.parent.as_ref()?.read().get(key),
        }
    }
}
