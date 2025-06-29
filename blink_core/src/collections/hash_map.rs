use std::{collections::HashMap, fmt::Display};

use crate::{collections::{ContextualValueRef, ValueContext}, value::ValueRef};

#[derive(Debug)]
pub struct BlinkHashMap {
    context: ValueContext,
    map: HashMap<ContextualValueRef, ValueRef>
}

impl Display for BlinkHashMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (k, v)) in self.map.iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            let v_contextual = ContextualValueRef::new(v.clone(), self.context.clone());
            write!(f, "{} {}", k, v_contextual)?;
        }
        write!(f, "}}")
    }
}

impl BlinkHashMap {
    pub fn new(context: ValueContext) -> Self {
        Self {
            context: context.clone(),
            map: HashMap::new(),
        }
    }

    pub fn with_capacity(capacity: usize, context: ValueContext) -> Self {
        Self {
            context: context.clone(),
            map: HashMap::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, key: ValueRef, value: ValueRef) -> Option<ValueRef> {
        let contextual_key = ContextualValueRef::new(key, self.context.clone());
        self.map.insert(contextual_key, value)
    }

    pub fn get(&self, key: &ValueRef) -> Option<&ValueRef> {
        let contextual_key = ContextualValueRef::new(*key, self.context.clone());
        self.map.get(&contextual_key)
    }

    pub fn get_mut(&mut self, key: &ValueRef) -> Option<&mut ValueRef> {
        let contextual_key = ContextualValueRef::new(*key, self.context.clone());
        self.map.get_mut(&contextual_key)
    }

    pub fn remove(&mut self, key: &ValueRef) -> Option<ValueRef> {
        let contextual_key = ContextualValueRef::new(*key, self.context.clone());
        self.map.remove(&contextual_key)
    }

    pub fn contains_key(&self, key: &ValueRef) -> bool {
        let contextual_key = ContextualValueRef::new(*key, self.context.clone());
        self.map.contains_key(&contextual_key)
    }


    // Standard interface
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
    pub fn clear(&mut self) { self.map.clear() }

    // Iterators that return ValueRef pairs
    pub fn iter(&self) -> impl Iterator<Item = (&ValueRef, &ValueRef)> {
        self.map.iter().map(|(k, v)| (k.value(), v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ValueRef, &mut ValueRef)> {
        self.map.iter_mut().map(|(k, v)| (k.value(), v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &ValueRef> {
        self.map.keys().map(|k| k.value())
    }

    pub fn values(&self) -> impl Iterator<Item = &ValueRef> {
        self.map.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut ValueRef> {
        self.map.values_mut()
    }

    // Language runtime specific methods
    pub fn get_or_nil(&self, key: &ValueRef) -> ValueRef {
        self.get(key).cloned().unwrap_or_else(|| ValueRef::nil())
    }

    pub fn try_get_string(&self, key: &ValueRef) -> Option<String> {
        self.get(key).and_then(|v| {
            // Extract string from ValueRef using context
            match v {
                ValueRef::Shared(idx) => {
                    self.context.arena().read().get(*idx)
                        .and_then(|shared_val| {
                            // Assuming SharedValue has a string variant
                            // match shared_val.as_ref() {
                            //     SharedValue::Str(s) => Some(s.clone()),
                            //     _ => None,
                            // }
                            None // Placeholder
                        })
                }
                _ => None,
            }
        })
    }

    pub fn context(&self) -> &ValueContext {
        &self.context
    }

    // Convenient constructors for common operations
    pub fn from_pairs<I>(pairs: I, context: ValueContext) -> Self 
    where 
        I: IntoIterator<Item = (ValueRef, ValueRef)>
    {
        let mut map = Self::new(context);
        for (k, v) in pairs {
            map.insert(k, v);
        }
        map
    }
}

// Implement common traits for BlinkHashMap
impl Clone for BlinkHashMap {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            map: self.map.clone(),
        }
    }
}