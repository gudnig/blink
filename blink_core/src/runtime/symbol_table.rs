use std::collections::HashMap;

pub struct SymbolTable {
    // Simple symbols: "foo", "bar", "add"
    strings: Vec<String>,
    lookup: HashMap<String, u32>,
    
    // Qualified symbols: (module_id, symbol_id) -> qualified_id
    qualified_lookup: HashMap<(u32, u32), u32>,
    qualified_symbols: Vec<(u32, u32)>, // For reverse lookup
    
    next_id: u32,
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            strings: Vec::new(),
            lookup: HashMap::new(),
            qualified_lookup: HashMap::new(),
            qualified_symbols: Vec::new(),
            next_id: 0,
        }
    }
    
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.lookup.get(name) {
            id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.strings.push(name.to_string());
            self.lookup.insert(name.to_string(), id);
            id
        }
    }
    
    pub fn intern_qualified(&mut self, module_id: u32, symbol_id: u32) -> u32 {
        let key = (module_id, symbol_id);
        if let Some(&id) = self.qualified_lookup.get(&key) {
            id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.qualified_lookup.insert(key, id);
            self.qualified_symbols.push(key);
            id
        }
    }
    
    pub fn get_symbol(&self, id: u32) -> Option<&str> {
        self.strings.get(id as usize).map(|s| s.as_str())
    }
    
    pub fn get_qualified(&self, id: u32) -> Option<(u32, u32)> {
        // Find the qualified symbol by scanning (could optimize with reverse map)
        let qualified_index = id as usize - self.strings.len();
        self.qualified_symbols.get(qualified_index).copied()
    }
    
    pub fn is_qualified(&self, id: u32) -> bool {
        (id as usize) >= self.strings.len()
    }
    
    // Helper for error messages and debugging
    pub fn display_symbol(&self, id: u32) -> String {
        if let Some(name) = self.get_symbol(id) {
            name.to_string()
        } else if let Some((module_id, symbol_id)) = self.get_qualified(id) {
            format!("{}/{}", 
                    self.get_symbol(module_id).unwrap_or("?"),
                    self.get_symbol(symbol_id).unwrap_or("?"))
        } else {
            format!("symbol#{}", id)
        }
    }
}