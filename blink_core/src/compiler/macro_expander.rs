use std::{collections::HashMap, sync::Arc};

use crate::{
    runtime::{BlinkVM, Macro},
    value::unpack_immediate,
    value::{HeapValue, ImmediateValue, ValueRef},
};

pub struct MacroExpander {
    vm: Arc<BlinkVM>,
    current_module: u32,
    expansion_depth: usize,
    max_expansion_depth: usize,
}

impl MacroExpander {
    pub fn new(vm: Arc<BlinkVM>, module: u32) -> Self {
        Self {
            vm,
            current_module: module,
            expansion_depth: 0,
            max_expansion_depth: 100,
        }
    }

    /// Main entry point: takes ValueRef, returns fully expanded ValueRef
    pub fn expand(&mut self, expr: ValueRef) -> Result<ValueRef, String> {
        let mut current = expr;
        let mut iterations = 0;

        // Keep expanding until no more changes (fixpoint)
        loop {
            let expanded = self.expand_once(current)?;

            // Check if anything changed
            if self.values_equal(expanded, current) {
                return Ok(current);
            }

            current = expanded;
            iterations += 1;

            if iterations > self.max_expansion_depth {
                return Err("Maximum macro expansion depth exceeded".to_string());
            }
        }
    }

    /// Single expansion pass
    fn expand_once(&mut self, expr: ValueRef) -> Result<ValueRef, String> {
        match expr {
            ValueRef::Immediate(_) => {
                // Immediates can't contain macros
                Ok(expr)
            }
            ValueRef::Heap(gc_ptr) => {
                match gc_ptr.to_heap_value() {
                    HeapValue::List(list_obj) => self.expand_list(&list_obj),
                    HeapValue::Vector(vec_obj) => {
                        // Expand elements in vectors too
                        let mut expanded_items = Vec::new();
                        for &item in &vec_obj {
                            expanded_items.push(self.expand_once(item)?);
                        }
                        Ok(self.vm.vector_value(expanded_items))
                    }
                    HeapValue::Map(map_obj) => {
                        // Expand keys and values in maps
                        let mut expanded_pairs = Vec::new();
                        for &(key, value) in &map_obj.iter().collect::<Vec<_>>() {
                            let expanded_key = self.expand_once(*key)?;
                            let expanded_value = self.expand_once(*value)?;
                            expanded_pairs.push((expanded_key, expanded_value));
                        }
                        Ok(self.vm.map_value(expanded_pairs))
                    }
                    _ => {
                        // Other heap values (strings, functions, etc.) don't contain macros
                        Ok(expr)
                    }
                }
            }
            ValueRef::Handle(_) => {
                // Native values can't contain macros
                Ok(expr)
            }
        }
    }

    /// Expand a list (might be a macro call)
    fn expand_list(&mut self, items: &[ValueRef]) -> Result<ValueRef, String> {
        if items.is_empty() {
            return Ok(self.vm.list_value(vec![]));
        }

        // Check if first element is a symbol that might be a macro
        if let ValueRef::Immediate(packed) = items[0] {
            if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                // Try to expand as macro
                if let Some(expanded) = self.try_expand_macro(symbol_id, &items[1..])? {
                    return Ok(expanded);
                }

                // Handle special forms that need special macro treatment
                if self.is_special_form(symbol_id) {
                    return self.expand_special_form(symbol_id, &items[1..]);
                }
            }
        }

        // Not a macro - expand all elements recursively
        let mut expanded_items = Vec::new();
        for &item in items {
            expanded_items.push(self.expand_once(item)?);
        }

        Ok(self.vm.list_value(expanded_items))
    }

    /// Try to expand a symbol as a macro
    fn try_expand_macro(
        &mut self,
        symbol_id: u32,
        args: &[ValueRef],
    ) -> Result<Option<ValueRef>, String> {
        // Look up symbol in module registry
        let symbol_value = match self
            .vm
            .module_registry
            .read()
            .resolve_symbol(self.current_module, symbol_id)
        {
            Some(val) => val,
            None => return Ok(None), // Symbol not found
        };

        // Check if it's a macro
        if let ValueRef::Heap(gc_ptr) = symbol_value {
            if let HeapValue::Macro(macro_def) = gc_ptr.to_heap_value() {
                // Execute macro expansion
                let expanded = self.expand_macro_call(&macro_def, args)?;

                println!(
                    "DEBUG: Macro {} expanded from ({} ...) to {}",
                    self.vm
                        .symbol_table
                        .read()
                        .get_symbol(symbol_id)
                        .unwrap_or_default(),
                    self.vm
                        .symbol_table
                        .read()
                        .get_symbol(symbol_id)
                        .unwrap_or_default(),
                    expanded
                );

                return Ok(Some(expanded));
            }
        }

        Ok(None)
    }

    /// Expand a macro call
    fn expand_macro_call(
        &mut self,
        macro_def: &Macro,
        args: &[ValueRef],
    ) -> Result<ValueRef, String> {
        // Validate arity
        macro_def.validate_arity(args.len())?;

        // Create parameter bindings
        let mut bindings = HashMap::new();

        // Bind regular parameters
        for (i, &param_id) in macro_def.regular_params().iter().enumerate() {
            bindings.insert(param_id, args[i]);
        }

        // Bind variadic parameter (if any)
        if let Some(variadic_param) = macro_def.variadic_param() {
            let variadic_args: Vec<ValueRef> = args
                .iter()
                .skip(macro_def.regular_params().len())
                .cloned()
                .collect();
            let variadic_list = self.vm.list_value(variadic_args);
            bindings.insert(variadic_param, variadic_list);
        }

        // Expand macro body using substitution
        if macro_def.body.len() == 1 {
            self.substitute_in_ast(macro_def.body[0], &bindings)
        } else {
            // Multiple body forms - wrap in 'do
            let do_symbol = {
                let mut symbol_table = self.vm.symbol_table.write();
                ValueRef::symbol(symbol_table.intern("do"))
            };

            let mut expanded_body = vec![do_symbol];
            for &body_expr in &macro_def.body {
                expanded_body.push(self.substitute_in_ast(body_expr, &bindings)?);
            }

            Ok(self.vm.list_value(expanded_body))
        }
    }

    /// Check if two values are equal (for fixpoint detection)
    fn values_equal(&self, a: ValueRef, b: ValueRef) -> bool {
        // Simple equality check - you might want something more sophisticated
        a == b
    }

    /// Check if symbol is a special form
    fn is_special_form(&self, symbol_id: u32) -> bool {
        if let Some(symbol_name) = self.vm.symbol_table.read().get_symbol(symbol_id) {
            matches!(
                symbol_name.as_str(),
                "def"
                    | "fn"
                    | "if"
                    | "let"
                    | "quote"
                    | "do"
                    | "try"
                    | "macro"
                    | "go"
                    | "deref"
                    | "and"
                    | "or"
            )
        } else {
            false
        }
    }

    fn substitute_in_ast(
        &mut self,
        expr: ValueRef,
        bindings: &HashMap<u32, ValueRef>,
    ) -> Result<ValueRef, String> {
        match expr {
            ValueRef::Immediate(packed) => {
                if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                    // If this symbol is a parameter, substitute it
                    if let Some(&replacement) = bindings.get(&symbol_id) {
                        Ok(replacement)
                    } else {
                        Ok(expr) // Not a parameter, keep as-is
                    }
                } else {
                    Ok(expr) // Numbers, bools, etc. - no substitution needed
                }
            }
            ValueRef::Heap(gc_ptr) => {
                match gc_ptr.to_heap_value() {
                    HeapValue::List(list_obj) => self.substitute_in_list(&list_obj, bindings),
                    HeapValue::Vector(vec_obj) => {
                        let mut substituted = Vec::new();
                        for &item in &vec_obj {
                            substituted.push(self.substitute_in_ast(item, bindings)?);
                        }
                        Ok(self.vm.vector_value(substituted))
                    }
                    HeapValue::Map(map_obj) => {
                        let mut substituted_pairs = Vec::new();
                        for &(key, value) in &map_obj.iter().collect::<Vec<_>>() {
                            let new_key = self.substitute_in_ast(*key, bindings)?;
                            let new_value = self.substitute_in_ast(*value, bindings)?;
                            substituted_pairs.push((new_key, new_value));
                        }
                        Ok(self.vm.map_value(substituted_pairs))
                    }
                    _ => Ok(expr), // Strings, functions, etc. - no substitution needed
                }
            }
            ValueRef::Handle(_) => Ok(expr), // Native functions - no substitution
        }
    }

    /// Handle list substitution with special quasiquote support
    fn substitute_in_list(
        &mut self,
        items: &[ValueRef],
        bindings: &HashMap<u32, ValueRef>,
    ) -> Result<ValueRef, String> {
        if items.is_empty() {
            return Ok(self.vm.list_value(vec![]));
        }

        // Check for quasiquote special forms first
        if let ValueRef::Immediate(packed) = items[0] {
            if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                let symbol_opt = self.vm.symbol_table.read().get_symbol(symbol_id);
                if let Some(symbol_name) = symbol_opt {
                    match symbol_name.as_str() {
                        "quasiquote" => {
                            if items.len() == 2 {
                                return self.expand_quasiquote(items[1], bindings);
                            }
                        }
                        "unquote" => {
                            if items.len() == 2 {
                                // ~expr - substitute the expression
                                return self.substitute_in_ast(items[1], bindings);
                            }
                        }
                        "unquote-splicing" => {
                            // ~@expr should only appear inside quasiquote
                            return Err("unquote-splicing used outside quasiquote".to_string());
                        }
                        _ => {} // Regular list
                    }
                }
            }
        }

        // Regular list - substitute all elements
        let mut substituted = Vec::new();
        for &item in items {
            substituted.push(self.substitute_in_ast(item, bindings)?);
        }

        Ok(self.vm.list_value(substituted))
    }

    /// Expand quasiquote template with unquote and unquote-splicing support
    fn expand_quasiquote(
        &mut self,
        template: ValueRef,
        bindings: &HashMap<u32, ValueRef>,
    ) -> Result<ValueRef, String> {
        match template {
            ValueRef::Immediate(packed) => {
                if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                    // In quasiquote, symbols are quoted unless explicitly unquoted
                    // Check if this symbol should be substituted (but it won't be unless unquoted)
                    Ok(template)
                } else {
                    Ok(template) // Numbers, bools, etc. are literal
                }
            }
            ValueRef::Heap(gc_ptr) => {
                match gc_ptr.to_heap_value() {
                    HeapValue::List(list_obj) => self.expand_quasiquote_list(&list_obj, bindings),
                    HeapValue::Vector(vec_obj) => {
                        let mut expanded = Vec::new();
                        for &item in &vec_obj {
                            expanded.push(self.expand_quasiquote(item, bindings)?);
                        }
                        Ok(self.vm.vector_value(expanded))
                    }
                    _ => Ok(template), // Strings, etc. are literal
                }
            }
            ValueRef::Handle(_) => Ok(template),
        }
    }

    /// Expand quasiquote list with unquote and splicing support
    fn expand_quasiquote_list(
        &mut self,
        items: &[ValueRef],
        bindings: &HashMap<u32, ValueRef>,
    ) -> Result<ValueRef, String> {
        if items.is_empty() {
            return Ok(self.vm.list_value(vec![]));
        }

        let mut result = Vec::new();
        let mut i = 0;

        while i < items.len() {
            let item = items[i];

            // Check for unquote or unquote-splicing
            if let Some(list_items) = item.get_list() {
                if !list_items.is_empty() {
                    if let ValueRef::Immediate(packed) = list_items[0] {
                        if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                            let symbol_opt = self.vm.symbol_table.read().get_symbol(symbol_id);
                            if let Some(symbol_name) = symbol_opt {
                                match symbol_name.as_str() {
                                    "unquote" => {
                                        // ~expr - evaluate and substitute
                                        if list_items.len() == 2 {
                                            let substituted =
                                                self.substitute_in_ast(list_items[1], bindings)?;
                                            result.push(substituted);
                                            i += 1;
                                            continue;
                                        }
                                    }
                                    "unquote-splicing" => {
                                        // ~@expr - evaluate and splice
                                        if list_items.len() == 2 {
                                            let substituted =
                                                self.substitute_in_ast(list_items[1], bindings)?;

                                            // The substituted value should be a list to splice
                                            if let Some(splice_items) = substituted.get_list() {
                                                result.extend_from_slice(&splice_items);
                                            } else if let Some(splice_items) = substituted.get_vec()
                                            {
                                                result.extend_from_slice(&splice_items);
                                            } else {
                                                return Err(
                                                    "unquote-splicing requires a list or vector"
                                                        .to_string(),
                                                );
                                            }
                                            i += 1;
                                            continue;
                                        }
                                    }
                                    _ => {} // Regular list
                                }
                            }
                        }
                    }
                }
            }

            // Regular item - recursively expand within quasiquote context
            result.push(self.expand_quasiquote(item, bindings)?);
            i += 1;
        }

        Ok(self.vm.list_value(result))
    }

    /// Handle let bindings carefully during expansion
    fn expand_let_bindings(&mut self, bindings: ValueRef) -> Result<ValueRef, String> {
        if let Some(binding_vec) = bindings.get_vec() {
            let mut expanded_bindings = Vec::new();

            // Bindings come in pairs: [symbol value symbol value ...]
            for chunk in binding_vec.chunks(2) {
                if chunk.len() != 2 {
                    return Err("let bindings must be pairs".to_string());
                }

                // Symbol doesn't get expanded, but value does
                expanded_bindings.push(chunk[0]); // binding symbol
                expanded_bindings.push(self.expand_once(chunk[1])?); // binding value
            }

            Ok(self.vm.vector_value(expanded_bindings))
        } else {
            Err("let bindings must be a vector".to_string())
        }
    }

    /// Update expand_special_form to handle quasiquote properly
    fn expand_special_form(
        &mut self,
        symbol_id: u32,
        args: &[ValueRef],
    ) -> Result<ValueRef, String> {
        let symbol_name = self
            .vm
            .symbol_table
            .read()
            .get_symbol(symbol_id)
            .unwrap_or_default();

        match symbol_name.as_str() {
            "quote" => {
                // Quoted content doesn't get expanded
                let mut result = vec![ValueRef::symbol(symbol_id)];
                result.extend_from_slice(args);
                Ok(self.vm.list_value(result))
            }
            "quasiquote" => {
                // Quasiquote needs special handling but isn't a macro call
                // This should be handled during macro template expansion, not here
                // For now, treat it like quote
                let mut result = vec![ValueRef::symbol(symbol_id)];
                result.extend_from_slice(args);
                Ok(self.vm.list_value(result))
            }
            "unquote" | "unquote-splicing" => {
                // These should only appear inside quasiquote
                Err(format!("{} used outside quasiquote", symbol_name))
            }
            "let" => {
                if args.len() < 2 {
                    return Err("let expects at least 2 arguments".to_string());
                }

                // Expand binding values but not symbols
                let bindings = self.expand_let_bindings(args[0])?;

                // Expand body
                let mut expanded_body = Vec::new();
                for &body_expr in &args[1..] {
                    expanded_body.push(self.expand_once(body_expr)?);
                }

                let mut result = vec![ValueRef::symbol(symbol_id), bindings];
                result.extend(expanded_body);
                Ok(self.vm.list_value(result))
            }
            "fn" => {
                if args.len() < 2 {
                    return Err("fn expects at least 2 arguments".to_string());
                }

                // Params don't get expanded, body does
                let mut result = vec![ValueRef::symbol(symbol_id), args[0]];
                for &body_expr in &args[1..] {
                    result.push(self.expand_once(body_expr)?);
                }

                Ok(self.vm.list_value(result))
            }
            _ => {
                // Default: expand all arguments
                let mut result = vec![ValueRef::symbol(symbol_id)];
                for &arg in args {
                    result.push(self.expand_once(arg)?);
                }
                Ok(self.vm.list_value(result))
            }
        }
    }
}
