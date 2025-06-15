use std::collections::HashMap;

pub struct SymbolTable {
    strings: Vec<String>,
    lookup: HashMap<String, u32>,
    next_id: u32,
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            strings: Vec::new(),
            lookup: HashMap::new(),
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
    
    pub fn get_symbol(&self, id: u32) -> Option<&str> {
        self.strings.get(id as usize).map(|s| s.as_str())
    }
}