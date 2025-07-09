use std::collections::{HashMap, HashSet};

use blink_core::{ eval::EvalContext, runtime::SymbolTable, value::{unpack_immediate, ImmediateValue, ParsedValue, ParsedValueWithPos}, ValueRef};

use crate::session::{SymbolInfo, SymbolKind, SymbolSource};

pub fn get_symbol_kind(value: &ParsedValue) -> SymbolKind {
    
    match value {
        ParsedValue::Number(_) => SymbolKind::Number,
        ParsedValue::Bool(_) => SymbolKind::Bool,
        ParsedValue::Symbol(_) => SymbolKind::SymbolRef,
        ParsedValue::Keyword(_) => SymbolKind::Keyword,
        ParsedValue::Nil => SymbolKind::Nil,
        ParsedValue::String(_) => SymbolKind::String,
        ParsedValue::List(_) => SymbolKind::List,
        ParsedValue::Vector(_) => SymbolKind::Vector,
        ParsedValue::Map(_) => SymbolKind::Map,
    }
}

pub fn get_var_representation(value: &ParsedValueWithPos, kind: &SymbolKind, name: &str, depth: usize, symbol_table: &SymbolTable) -> String {
    if depth > 10 {
        return "...".to_string();
    }
    
    match kind {
        
        SymbolKind::Variable => {
            
            if let ParsedValue::List(elements) = &value.value {
                if elements.len() >= 3 {
                    let head = &elements[0];
                    if let ParsedValue::Symbol(sym) = head.value {
                        if symbol_table.get_symbol(sym) == Some("def") {

                            let value_expr = &elements[2];
                            let inner_name = &elements[1];
                            if let ParsedValue::Symbol(inner_name) = inner_name.value {
                                let symbol_name = symbol_table.get_symbol(inner_name);
                                if let Some(symbol_name) = symbol_name {
                                    let inner_kind = get_symbol_kind(&value_expr.value);
                                    let inner = get_var_representation(value_expr, &inner_kind, &symbol_name, depth + 1, symbol_table);
                                    return format!("{} => {}", name, inner);
                                }
                            }
                        }
                    }
                }
            }
            format!("{} => {}", name, "Unknown")
        },
        SymbolKind::List => {
            
            format!("{} => {}", name, "Unknown")
            
        },
        SymbolKind::Map => {
            if let ParsedValue::List(elements) = &value.value {
                if elements.len() >= 2 {
                    if let ParsedValue::Symbol(sym) = elements[0].value {
                        let symbol_name = symbol_table.get_symbol(sym);
                        if symbol_name == Some("hash-map") {
                            let mut out = String::new();
                            out.push_str(&format!("{} => {{", name));
                            let mut is_key = true;
                            for korv in elements[1..].iter() { 
                                
                                
                                if is_key {
                                    out.push_str(&format!("{} =>", korv.display_with_symbol_table(symbol_table)));
                                } else {
                                    out.push_str(&format!(" {}\n", korv.display_with_symbol_table(symbol_table)));
                                }
                                is_key = !is_key;
                            }
                            out.push('}');
                            return out;
                            
                        }
                    }
                }
            }
            format!("{} => {}", name, "Map")
        }
        _ => format!("{}", kind)
        
    }
}


pub fn collect_symbols_from_forms(symbols: &mut HashMap<String, SymbolInfo>, forms: &Vec<ParsedValueWithPos>, defined_in: SymbolSource, symbol_table: &SymbolTable) {

        
    // Extract defined symbols and their types from the parsed forms
    for form in forms {

        match &form.value {
            ParsedValue::List(elements) => {
                if elements.len() >= 3 {
                    let first_elem = &elements[0];
                    if let ParsedValue::Symbol(id) = first_elem.value {
                        let symbol = symbol_table.get_symbol(id);
                        if let Some(symbol) = symbol {
                            if symbol == "def" {
                                let symbol = &elements[1];
                                if let ParsedValue::Symbol(id) = symbol.value {
                                    let name = symbol_table.get_symbol(id);
                                    if let Some(name) = name {
                                        let symbol_pos = elements[1].pos;
                                        let type_info = get_symbol_kind(&elements[2].value);
                                        let representation = get_var_representation(&elements[2], &type_info, &name, 0, symbol_table);
                                        let symbol_info = SymbolInfo { 
                                            kind: type_info,
                                            defined_in: defined_in.clone(),
                                            position: symbol_pos,
                                            representation: Some(representation),
                                        };
                                        symbols.insert(name.to_string(), symbol_info);
                                    }
                                }
                            }
                        }
                    }
                }
            },
            _ => {}
        }
    }
}