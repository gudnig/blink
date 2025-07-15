use serde::{Deserialize, Serialize};

use crate::{runtime::SymbolTable, value::SourceRange};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedValueWithPos {
    pub value: ParsedValue,
    pub pos: Option<SourceRange>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ParsedValue {
    // Immediate values
    Number(f64),
    Bool(bool),
    Symbol(u32),
    Keyword(u32),
    Nil,
    
    
    String(String),
    List(Vec<ParsedValueWithPos>),    
    Vector(Vec<ParsedValueWithPos>),  
    Map(Vec<(ParsedValueWithPos, ParsedValueWithPos)>), 
}

impl ParsedValueWithPos {
    pub fn new(value: ParsedValue, pos: Option<SourceRange>) -> Self {
        Self { value, pos }
    }

    pub fn display_with_symbol_table(&self, symbol_table: &SymbolTable) -> String {
        match &self.value {
            ParsedValue::Number(n) => n.to_string(),
            ParsedValue::Bool(b) => b.to_string(),
            ParsedValue::Symbol(s) => symbol_table.get_symbol(*s).unwrap_or("Unknown".to_string()),
            ParsedValue::Keyword(id) => {
                let symbol_name = symbol_table.get_symbol(*id);
                if let Some(symbol_name) = symbol_name {
                    return symbol_name.to_string();
                }
                "Unknown".to_string()
            },
            ParsedValue::Nil => "nil".to_string(),
            ParsedValue::String(s) => s.to_string(),
            ParsedValue::List(parsed_values) => {
                let mut out = String::new();
                out.push_str("(");
                for value in parsed_values {
                    out.push_str(&value.display_with_symbol_table(symbol_table));
                }
                out.push_str(")");
                out
            },
            ParsedValue::Vector(parsed_values) => {
                let mut out = String::new();
                out.push_str("[");
                for value in parsed_values {
                    out.push_str(&value.display_with_symbol_table(symbol_table));
                }
                out.push_str("]");
                out
            },
            ParsedValue::Map(items) => {
                let mut out = String::new();
                out.push_str("{");
                for (key, value) in items {
                    out.push_str(&key.display_with_symbol_table(symbol_table));
                    out.push_str(" => ");
                    out.push_str(&value.display_with_symbol_table(symbol_table));
                }
                out.push_str("}");
                out
            },
        }
    }
}