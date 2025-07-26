use std::collections::HashMap;

// TODO: strategic symbol assignment for array index access
pub struct SymbolTable {
    // Simple symbols: "foo", "bar", "add"
    strings: Vec<String>,
    lookup: HashMap<String, u32>,
    
    // Qualified symbols: (module_id, symbol_id) -> qualified_id
    qualified_lookup: HashMap<(u32, u32), u32>,
    qualified_symbols: Vec<(u32, u32)>, // For reverse lookup
    
    // Separate ID counters to prevent overlap
    next_simple_id: u32,
    next_qualified_id: u32,
}

impl SymbolTable {
    // Reserve the first 2^31 IDs for simple symbols, rest for qualified
    const QUALIFIED_ID_OFFSET: u32 = 0x80000000;
    
    pub fn new() -> Self {
        SymbolTable {
            strings: Vec::new(),
            lookup: HashMap::new(),
            qualified_lookup: HashMap::new(),
            qualified_symbols: Vec::new(),
            next_simple_id: 32,
            next_qualified_id: Self::QUALIFIED_ID_OFFSET,
        }
    }
    
    pub fn print_all(&self) {
        for (symbol, id) in self.lookup.iter() {
            println!("{}: {}", symbol, id);
        }
        for ((module_id, symbol_id), qualified_id) in self.qualified_lookup.iter() {
            let module_name = self.get_symbol(*module_id).unwrap_or("?".to_string());
            let symbol_name = self.get_symbol(*symbol_id).unwrap_or("?".to_string());
            println!("{}/{}: {}", module_name, symbol_name, qualified_id);
        }
    }
    
    pub fn intern(&mut self, name: &str) -> u32 {
        if name.contains("/") {
            let parts: Vec<&str> = name.split('/').collect();
            if parts.len() != 2 {
                panic!("Invalid qualified symbol format: {}", name);
            }
            
            // Intern the parts as simple symbols first
            let module_id = self.intern_simple(parts[0]);
            let symbol_id = self.intern_simple(parts[1]);
            
            // Then create the qualified symbol
            self.intern_qualified(module_id, symbol_id)
        } else {
            self.intern_simple(name)
        }
    }
    
    fn intern_simple(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.lookup.get(name) {
            id
        } else {
            let id = self.next_simple_id;
            self.next_simple_id += 1;
            
            // Ensure we don't overflow into qualified ID space
            if self.next_simple_id >= Self::QUALIFIED_ID_OFFSET {
                panic!("Too many simple symbols - exceeded reserved ID space");
            }
            
            self.strings.push(name.to_string());
            self.lookup.insert(name.to_string(), id);
            id
        }
    }

    pub fn intern_special_form(&mut self, id: u32, name: &str) -> u32 {
        self.strings.push(name.to_string());
        self.lookup.insert(name.to_string(), id);
        id
    }
    
    pub fn intern_qualified(&mut self, module_id: u32, symbol_id: u32) -> u32 {
        // Verify both IDs are simple symbols
        if self.is_qualified(module_id) || self.is_qualified(symbol_id) {
            panic!("Cannot create qualified symbol from other qualified symbols");
        }
        
        let key = (module_id, symbol_id);
        if let Some(&id) = self.qualified_lookup.get(&key) {
            id
        } else {
            let id = self.next_qualified_id;
            self.next_qualified_id += 1;
            
            // Store the mapping both ways
            self.qualified_lookup.insert(key, id);
            self.qualified_symbols.push(key);
            id
        }
    }
    
    pub fn get_symbol(&self, id: u32) -> Option<String> {
        if self.is_qualified(id) {
            let (module_id, symbol_id) = self.get_qualified(id).unwrap();
            let module_name = self.get_symbol(module_id).unwrap_or("?".to_string());
            let symbol_name = self.get_symbol(symbol_id).unwrap_or("?".to_string());
            Some(format!("{}/{}", module_name, symbol_name))
        } else {
            self.strings.get(id as usize).map(|s| s.clone())
        }
    }
    
    pub fn get_qualified(&self, id: u32) -> Option<(u32, u32)> {
        if !self.is_qualified(id) {
            return None;
        }
        
        // Calculate index in qualified_symbols vector
        let qualified_index = (id - Self::QUALIFIED_ID_OFFSET) as usize;
        self.qualified_symbols.get(qualified_index).copied()
    }
    
    pub fn is_qualified(&self, id: u32) -> bool {
        id >= Self::QUALIFIED_ID_OFFSET
    }
    
    // Helper for error messages and debugging
    pub fn display_symbol(&self, id: u32) -> String {
        if let Some(name) = self.get_symbol(id) {
            name.to_string()
        } else if let Some((module_id, symbol_id)) = self.get_qualified(id) {
            format!("{}/{}", 
                    self.get_symbol(module_id).unwrap_or("?".to_string()),
                    self.get_symbol(symbol_id).unwrap_or("?".to_string()))
        } else {
            format!("symbol#{}", id)
        }
    }
    
    // Additional helper methods
    pub fn get_all_simple_symbols(&self) -> impl Iterator<Item = (u32, &str)> {
        self.strings.iter().enumerate().map(|(i, s)| (i as u32, s.as_str()))
    }
    
    pub fn get_all_qualified_symbols(&self) -> impl Iterator<Item = (u32, (u32, u32))> + '_ {
        self.qualified_lookup.iter().map(|((m, s), &id)| (id, (*m, *s)))
    }
    
    // Lookup by string (for backward compatibility)
    pub fn lookup_symbol(&self, name: &str) -> Option<u32> {
        if name.contains("/") {
            let parts: Vec<&str> = name.split('/').collect();
            if parts.len() == 2 {
                let module_id = self.lookup.get(parts[0])?;
                let symbol_id = self.lookup.get(parts[1])?;
                self.qualified_lookup.get(&(*module_id, *symbol_id)).copied()
            } else {
                None
            }
        } else {
            self.lookup.get(name).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simple_symbols() {
        let mut table = SymbolTable::new();
        
        let foo = table.intern("foo");
        let bar = table.intern("bar");
        
        assert!(!table.is_qualified(foo));
        assert!(!table.is_qualified(bar));
        assert_eq!(table.get_symbol(foo), Some("foo".to_string()));
        assert_eq!(table.get_symbol(bar), Some("bar".to_string()));
    }
    
    #[test]
    fn test_qualified_symbols() {
        let mut table = SymbolTable::new();
        
        let math_add = table.intern("math/add");
        
        assert!(table.is_qualified(math_add));
        
        if let Some((module_id, symbol_id)) = table.get_qualified(math_add) {
            assert_eq!(table.get_symbol(module_id), Some("math".to_string()));
            assert_eq!(table.get_symbol(symbol_id), Some("add".to_string()));
        } else {
            panic!("Should be qualified");
        }
        
        assert_eq!(table.display_symbol(math_add), "math/add");
    }
    
    #[test]
    fn test_mixed_order() {
        let mut table = SymbolTable::new();
        
        // Create in mixed order
        let foo = table.intern("foo");
        let math_add = table.intern("math/add");
        let bar = table.intern("bar");
        let core_map = table.intern("core/map");
        
        // All simple symbols should not be qualified
        assert!(!table.is_qualified(foo));
        assert!(!table.is_qualified(bar));
        
        // All qualified symbols should be qualified
        assert!(table.is_qualified(math_add));
        assert!(table.is_qualified(core_map));
        
        // Check that "math" and "add" were created as simple symbols
        let math_id = table.lookup_symbol("math").unwrap();
        let add_id = table.lookup_symbol("add").unwrap();
        assert!(!table.is_qualified(math_id));
        assert!(!table.is_qualified(add_id));
    }
    
    #[test]
    fn test_deduplication() {
        let mut table = SymbolTable::new();
        
        let foo1 = table.intern("foo");
        let foo2 = table.intern("foo");
        assert_eq!(foo1, foo2);
        
        let math_add1 = table.intern("math/add");
        let math_add2 = table.intern("math/add");
        assert_eq!(math_add1, math_add2);
    }
}