use std::collections::{HashMap, HashSet};

use blink_core::BlinkValue;

use crate::session::{SymbolInfo, SymbolKind, SymbolSource};

pub fn get_symbol_kind(value: &BlinkValue) -> SymbolKind {
    let value = &value.read().value;
    match value {
        blink_core::value::Value::Bool(_) => SymbolKind::Bool,
        blink_core::value::Value::Number(_) => SymbolKind::Number,
        blink_core::value::Value::Str(_) => SymbolKind::String,
        blink_core::value::Value::Symbol(_) => SymbolKind::SymbolRef,
        blink_core::value::Value::Keyword(_) => SymbolKind::Keyword,
        blink_core::value::Value::Vector(_) => SymbolKind::Vector,
        blink_core::value::Value::Map(_) => SymbolKind::Map,
        blink_core::value::Value::List(_) => {
            if let blink_core::value::Value::List(vec) = value {
                let first_elem = vec.get(0);
                if let Some(elem) =first_elem {
                    let elem_read = elem.read();
                    if let blink_core::value::Value::Symbol(sym) =  &elem_read.value {
                        if sym == "hash-map" {
                            return SymbolKind::Map
                        }                    
                    }
                }
            }
            SymbolKind::List
        },
        blink_core::value::Value::NativeFunc(_) => SymbolKind::Function,
        blink_core::value::Value::FuncUserDefined { .. } => SymbolKind::Function,
        _ => SymbolKind::Unknown,
    }
}

pub fn get_var_representation(value: &BlinkValue, kind: &SymbolKind, name: &str, depth: usize) -> String {
    if depth > 10 {
        return "...".to_string();
    }
    let v = value.read();
    match kind {
        
        SymbolKind::Variable => {
            
            if let blink_core::value::Value::List(elements) = &v.value {
                if elements.len() >= 3 {
                    let head = &elements[0].read().value;
                    if let blink_core::value::Value::Symbol(sym) = head {
                        if sym == "def" {

                            let value_expr = &elements[2];
                            let inner_name = &elements[1].read().value;
                            if let blink_core::value::Value::Symbol(inner_name) = inner_name {
                                let inner_kind = get_symbol_kind(value_expr);
                                let inner = get_var_representation(value_expr, &inner_kind, inner_name, depth + 1);
                                return format!("{} => {}", name, inner);
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
            if let blink_core::value::Value::List(v) = &v.value {
                if v.len() >= 2 {
                    if let blink_core::value::Value::Symbol(sym) = &v[0].read().value {
                        if sym == "hash-map" {
                            let mut out = String::new();
                            out.push_str(&format!("{} => {{", name));
                            let mut is_key = true;
                            for korv in v[1..].iter() { 
                                let korv_read = korv.read();
                                let v = &korv_read.value;
                                if is_key {
                                out.push_str(&format!("{} =>", v));
                                } else {
                                    out.push_str(&format!(" {}\n", v));
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


pub fn collect_symbols_from_forms(symbols: &mut HashMap<String, SymbolInfo>, forms: &Vec<BlinkValue>, defined_in: SymbolSource) {

        
    // Extract defined symbols and their types from the parsed forms
    for form in forms {
        
        {
            let form_read = form.clone().read().value.clone();
        

            println!("form_read: {:?}", form_read);
        }
        let form_read = form.read();
        if let blink_core::value::Value::List(elements) = &form_read.value {if elements.len() >= 3 {
                let first_elem = &elements[0].read().value;

                if let blink_core::value::Value::Symbol(sym) = first_elem {
                    if sym == "def" {
                        let symbol_pos = elements[1].read().pos.clone();
                        if let blink_core::value::Value::Symbol(name) =
                            &elements[1].read().value
                        {
                            let type_info = get_symbol_kind(&elements[2]);
                            let representation = get_var_representation(&elements[2], &type_info, name, 0);
                            let symbol_info = SymbolInfo { 
                                kind: type_info,
                                defined_in: defined_in.clone(),
                                position: symbol_pos,
                                representation: Some(representation),
                            };
                            
                            

                            symbols.insert(name.clone(), symbol_info);
                        }
                    }
                }
            }
            
        }
    }
}