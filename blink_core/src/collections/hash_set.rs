use std::collections::HashSet;

use crate::{collections::{ContextualValueRef, ValueContext}, value::ValueRef};

/// Native HashSet for your language runtime - ValueRef values
#[derive(Debug)]
pub struct BlinkHashSet {
    context: ValueContext,
    set: HashSet<ContextualValueRef>,
}

impl BlinkHashSet {
    pub fn new(context: ValueContext) -> Self {
        Self {
            context: context.clone(),
            set: HashSet::new(),
        }
    }

    pub fn with_capacity(capacity: usize, context: ValueContext) -> Self {
        Self {
            context: context.clone(),
            set: HashSet::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, value: ValueRef) -> bool {
        let contextual_value = ContextualValueRef::new(value, self.context.clone());
        self.set.insert(contextual_value)
    }

    pub fn contains(&self, value: &ValueRef) -> bool {
        let contextual_value = ContextualValueRef::new(*value, self.context.clone());
        self.set.contains(&contextual_value)
    }

    pub fn remove(&mut self, value: &ValueRef) -> bool {
        let contextual_value = ContextualValueRef::new(*value, self.context.clone());
        self.set.remove(&contextual_value)
    }

    // Standard interface
    pub fn len(&self) -> usize { self.set.len() }
    pub fn is_empty(&self) -> bool { self.set.is_empty() }
    pub fn clear(&mut self) { self.set.clear() }

    // Iterators
    pub fn iter(&self) -> impl Iterator<Item = &ValueRef> {
        self.set.iter().map(|v| v.value())
    }

    // Set operations
    pub fn union<'a>(&'a self, other: &'a BlinkHashSet) -> impl Iterator<Item = &ValueRef> + 'a {
        self.iter().chain(other.iter().filter(|v| !self.contains(v)))
    }

    pub fn intersection<'a>(&'a self, other: &'a BlinkHashSet) -> impl Iterator<Item = &ValueRef> + 'a {
        self.iter().filter(|v| other.contains(v))
    }

    pub fn difference<'a>(&'a self, other: &'a BlinkHashSet) -> impl Iterator<Item = &ValueRef> + 'a {
        self.iter().filter(|v| !other.contains(v))
    }

    pub fn is_subset(&self, other: &BlinkHashSet) -> bool {
        self.iter().all(|v| other.contains(v))
    }

    pub fn is_superset(&self, other: &BlinkHashSet) -> bool {
        other.is_subset(self)
    }

    pub fn is_disjoint(&self, other: &BlinkHashSet) -> bool {
        self.iter().all(|v| !other.contains(v))
    }

    // Language runtime specific methods
    pub fn extend<I>(&mut self, iter: I) 
    where 
        I: IntoIterator<Item = ValueRef>
    {
        for value in iter {
            self.insert(value);
        }
    }

    pub fn context(&self) -> &ValueContext {
        &self.context
    }

    // Convenient constructors
    pub fn from_iter<I>(iter: I, context: ValueContext) -> Self 
    where 
        I: IntoIterator<Item = ValueRef>
    {
        let mut set = Self::new(context);
        set.extend(iter);
        set
    }

    pub fn from_values(values: &[ValueRef], context: ValueContext) -> Self {
        Self::from_iter(values.iter().copied(), context)
    }

    // Language-specific helpers
    pub fn contains_number(&self, n: f64) -> bool {
        self.contains(&ValueRef::number(n))
    }

    pub fn contains_bool(&self, b: bool) -> bool {
        self.contains(&ValueRef::boolean(b))
    }

    pub fn contains_nil(&self) -> bool {
        self.contains(&ValueRef::nil())
    }

    pub fn insert_if_truthy(&mut self, value: ValueRef) -> bool {
        if value.is_truthy() {
            self.insert(value)
        } else {
            false
        }
    }

    // Convert to Vec for ordered operations
    pub fn to_vec(&self) -> Vec<ValueRef> {
        self.iter().copied().collect()
    }

    // Retain elements matching predicate
    pub fn retain<F>(&mut self, mut f: F) 
    where 
        F: FnMut(&ValueRef) -> bool
    {
        let to_remove: Vec<ValueRef> = self.iter()
            .filter(|v| !f(v))
            .copied()
            .collect();
        
        for value in to_remove {
            self.remove(&value);
        }
    }
}

// Implement common traits for BlinkHashSet
impl Clone for BlinkHashSet {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            set: self.set.clone(),
        }
    }
}

// Set operations between BlinkHashSets
impl BlinkHashSet {
    /// Create a new set with the union of both sets
    pub fn union_with(&self, other: &BlinkHashSet) -> BlinkHashSet {
        let mut result = self.clone();
        for value in other.iter() {
            result.insert(*value);
        }
        result
    }

    /// Create a new set with the intersection of both sets
    pub fn intersection_with(&self, other: &BlinkHashSet) -> BlinkHashSet {
        let mut result = BlinkHashSet::new(self.context.clone());
        for value in self.iter() {
            if other.contains(value) {
                result.insert(*value);
            }
        }
        result
    }

    /// Create a new set with values in self but not in other
    pub fn difference_with(&self, other: &BlinkHashSet) -> BlinkHashSet {
        let mut result = BlinkHashSet::new(self.context.clone());
        for value in self.iter() {
            if !other.contains(value) {
                result.insert(*value);
            }
        }
        result
    }

    /// Create a new set with values in either set but not both
    pub fn symmetric_difference_with(&self, other: &BlinkHashSet) -> BlinkHashSet {
        let mut result = BlinkHashSet::new(self.context.clone());
        
        // Add values from self that aren't in other
        for value in self.iter() {
            if !other.contains(value) {
                result.insert(*value);
            }
        }
        
        // Add values from other that aren't in self
        for value in other.iter() {
            if !self.contains(value) {
                result.insert(*value);
            }
        }
        
        result
    }
}