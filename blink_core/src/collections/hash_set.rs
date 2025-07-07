use std::{collections::HashSet, fmt::Display};

use crate::value::ValueRef;

/// Native HashSet for your language runtime - ValueRef values
#[derive(Debug)]
pub struct BlinkHashSet {
    set: HashSet<ValueRef>,
}

impl Display for BlinkHashSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{{")?;
        for (i, value) in self.set.iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
     
            write!(f, "{}", value)?;
        }
        write!(f, "}}")
    }
}

impl BlinkHashSet {
    pub fn new() -> Self {
        Self {
            set: HashSet::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            set: HashSet::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, value: ValueRef) -> bool {
        self.set.insert(value)
    }

    pub fn contains(&self, value: &ValueRef) -> bool {
        self.set.contains(value)
    }

    pub fn remove(&mut self, value: &ValueRef) -> bool {
        self.set.remove(value)
    }

    // Standard interface
    pub fn len(&self) -> usize { self.set.len() }
    pub fn is_empty(&self) -> bool { self.set.is_empty() }
    pub fn clear(&mut self) { self.set.clear() }

    // Iterators
    pub fn iter(&self) -> impl Iterator<Item = &ValueRef> {
        self.set.iter()
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

    // Convenient constructors
    pub fn from_iter<I>(iter: I) -> Self 
    where 
        I: IntoIterator<Item = ValueRef>
    {
        let mut set = Self::new();
        set.extend(iter);
        set
    }

    pub fn from_values(values: &[ValueRef]) -> Self {
        Self::from_iter(values.iter().copied())
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
        let mut result = BlinkHashSet::new();
        for value in self.iter() {
            if other.contains(value) {
                result.insert(*value);
            }
        }
        result
    }

    /// Create a new set with values in self but not in other
    pub fn difference_with(&self, other: &BlinkHashSet) -> BlinkHashSet {
        let mut result = BlinkHashSet::new();
        for value in self.iter() {
            if !other.contains(value) {
                result.insert(*value);
            }
        }
        result
    }

    /// Create a new set with values in either set but not both
    pub fn symmetric_difference_with(&self, other: &BlinkHashSet) -> BlinkHashSet {
        let mut result = BlinkHashSet::new();
        
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