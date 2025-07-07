use std::{collections::HashMap, path::PathBuf, sync::Arc};

use mmtk::Mutator;
use parking_lot::RwLock;

use crate::env::Env;
use crate::{
    collections::{ContextualValueRef, ValueContext},
    error::BlinkError,
    module::ModuleRegistry,
    parser::ReaderContext,
    runtime::{
        AsyncContext, BlinkVM, HandleRegistry, SharedArena, SymbolTable, TokioGoroutineScheduler,
        ValueMetadataStore,
    },
    telemetry::TelemetryEvent,
    value::{
        unpack_immediate, ImmediateValue, ModuleRef, ParsedValue, ParsedValueWithPos, SharedValue,
        SourceRange, ValueRef,
    },
};

#[derive(Clone)]
pub struct EvalContext<'vm> {
    pub vm: &'vm BlinkVM,
    pub mutator: Mutator<BlinkVM>,
    pub env: Arc<RwLock<Env>>,

    pub current_file: Option<String>,
    pub current_module: Option<u32>,
    pub async_ctx: AsyncContext,
    pub tracing_enabled: bool,
}

impl<'vm> EvalContext<'vm> {


    pub fn alloc_parsed_value(&mut self, parsed: ParsedValueWithPos) -> ValueRef {
        let value_ref = match parsed.value {
            // Immediate values - pack directly
            ParsedValue::Number(n) => ValueRef::number(n),
            ParsedValue::Bool(b) => ValueRef::boolean(b),
            ParsedValue::Symbol(id) => ValueRef::symbol(id),
            ParsedValue::Keyword(id) => ValueRef::keyword(id),
            ParsedValue::Nil => ValueRef::nil(),

            // Complex values - allocate in shared arena
            ParsedValue::String(s) => self.string_value(&s),

            ParsedValue::List(items) => {
                let converted_items: Vec<ValueRef> = items
                    .into_iter()
                    .map(|item| self.alloc_parsed_value(item))
                    .collect();

                self.list_value(converted_items)
            }

            ParsedValue::Vector(items) => {
                let converted_items: Vec<ValueRef> = items
                    .into_iter()
                    .map(|item| self.alloc_parsed_value(item))
                    .collect();

                self.vector_value(converted_items)
            }

            ParsedValue::Map(pairs) => {
                let value_pairs = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        let key = self.alloc_parsed_value(k);
                        let value = self.alloc_parsed_value(v);
                        (key, value)
                    })
                    .collect();

                self.map_value(value_pairs)
            }
        };

        if let (Some(id), Some(pos)) = (value_ref.get_or_create_id(), parsed.pos) {
            self.value_metadata.write().set_position(id, pos);
        }

        value_ref
    }

    pub fn resolve_module_symbol(
        &self,
        module_alias: u32,
        symbol: u32,
    ) -> Result<ValueRef, BlinkError> {
        // Step 1: Look up module alias -> full module name (acquire and release lock)
        let module_name = {
            let env = self.env.read();
            env.available_modules.get(&module_alias).cloned()
        }
        .ok_or_else(|| {
            let symbol_table = self.symbol_table.read();
            let alias_name = symbol_table.get_symbol(module_alias).unwrap_or("?");
            BlinkError::eval(format!("Module alias '{}' not found", alias_name))
        })?;

        // Step 2: Get the module from registry (new lock scope)
        let module = {
            let module_registry = self.module_registry.read();
            module_registry.get_module(module_name).map(|m| m.clone())
        }
        .ok_or_else(|| {
            let symbol_table = self.symbol_table.read();
            let module_name_str = symbol_table.get_symbol(module_name).unwrap_or("?");
            BlinkError::eval(format!("Module '{}' not found", module_name_str))
        })?;

        // Step 3: Look up symbol in module environment (new lock scope)
        let result = {
            let module_guard = module.read();
            let env_guard = module_guard.env.read();
            env_guard.get_local(symbol)
        };

        result.ok_or_else(|| {
            let symbol_table = self.symbol_table.read();
            let symbol_name = symbol_table.get_symbol(symbol).unwrap_or("?");
            let module_name_str = symbol_table.get_symbol(module_name).unwrap_or("?");
            BlinkError::eval(format!(
                "Symbol '{}' not found in module '{}'",
                symbol_name, module_name_str
            ))
        })
    }

    pub fn resolve_symbol(&self, symbol_id: u32) -> Result<ValueRef, BlinkError> {
        // Check if this is a qualified symbol (interned during parsing)
        if self.symbol_table.read().is_qualified(symbol_id) {
            if let Some((module_id, symbol_id)) = self.symbol_table.read().get_qualified(symbol_id)
            {
                self.resolve_module_symbol(module_id, symbol_id)
            } else {
                Err(BlinkError::eval("Invalid qualified symbol"))
            }
        } else {
            // Simple symbol lookup
            let env = self.env.read();
            env.get_local(symbol_id).ok_or_else(|| {
                let symbol_table = self.symbol_table.read();
                let name = symbol_table.get_symbol(symbol_id).unwrap_or("?");
                BlinkError::undefined_symbol(name)
            })
        }
    }

    // Alternative: if you want to support string-based lookup for backward compatibility
    pub fn resolve_symbol_by_name(&self, name: &str) -> Result<ValueRef, BlinkError> {
        // Check for qualified names (module/symbol)
        if let Some((module_alias, symbol)) = name.split_once('/') {
            let module_alias_id = self.symbol_table.write().intern(module_alias);
            let symbol_id = self.symbol_table.write().intern(symbol);
            self.resolve_module_symbol(module_alias_id, symbol_id)
        } else {
            // Check local environment
            let symbol_id = self.symbol_table.write().intern(name);
            self.resolve_symbol(symbol_id)
        }
    }

    // Update the old string-based methods to use the new u32-based ones
    pub fn get(&self, key: &str) -> Option<ValueRef> {
        let symbol_id = self.symbol_table.write().intern(key);
        let module_registry = self.module_registry.read();
        self.env
            .read()
            .get_with_registry(symbol_id, &module_registry)
    }

    pub fn set(&self, key: &str, val: ValueRef) {
        let symbol_id = self.symbol_table.write().intern(key);
        self.env.write().set(symbol_id, val)
    }

    // New preferred methods that work with symbol IDs directly
    pub fn get_symbol(&self, symbol_id: u32) -> Option<ValueRef> {
        let module_registry = self.module_registry.read();
        self.env
            .read()
            .get_with_registry(symbol_id, &module_registry)
    }

    pub fn set_symbol(&self, symbol_id: u32, value: ValueRef) {
        self.env.write().set(symbol_id, value);
    }

    // Helper for creating module references during import
    pub fn create_module_reference(&self, module_id: u32, symbol_id: u32) -> ValueRef {
        let module_ref = SharedValue::Module(ModuleRef {
            module: module_id,
            symbol: symbol_id,
        });
        self.shared_arena.write().alloc(module_ref)
    }
}

impl EvalContext<'_> {
    pub fn new(
        parent: Arc<RwLock<Env>>,
        symbol_table: Arc<RwLock<SymbolTable>>,
        vm: BlinkVM,
    ) -> Self {
        EvalContext {
            vm,
            mutator: vm.mutator(),
            env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            current_module: None,
            current_file: None,
            async_ctx: AsyncContext::default(),
            tracing_enabled: false,
            shared_arena: Arc::new(RwLock::new(SharedArena::new())),
        }
    }

    pub fn type_tag(&self, value: ValueRef) -> String {
        let arena = self.shared_arena.read();
        value.type_tag(&arena).to_string()
    }

    pub fn runtime(&self) -> &tokio::runtime::Handle {
        &self.goroutine_scheduler.runtime // assuming it has this field
    }

    pub fn intern_symbol(&self, name: &str) -> ValueRef {
        let symbol_id = self.symbol_table.write().intern(name);
        ValueRef::symbol(symbol_id)
    }

    pub fn resolve_symbol_name(&self, symbol_id: u32) -> Option<String> {
        self.symbol_table
            .read()
            .get_symbol(symbol_id)
            .map(|s| s.to_string())
    }

    pub fn intern_keyword(&mut self, name: &str) -> ValueRef {
        // Store with ":" prefix in symbol table
        let full_name = format!(":{}", name);
        let id = self.symbol_table.write().intern(&full_name);
        ValueRef::keyword(id) // Use a different immediate tag
    }

    pub fn get_keyword_name(&self, val: ValueRef) -> Option<String> {
        if let ValueRef::Immediate(packed) = val {
            if let ImmediateValue::Keyword(id) = unpack_immediate(packed) {
                let symbol_table = self.symbol_table.read();
                let full_name = symbol_table.get_symbol(id);
                // Strip the ":" prefix and convert to owned String
                full_name.map(|s| s.strip_prefix(":").map(|s| s.to_string()))?
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn with_env(&self, env: Arc<RwLock<Env>>) -> Self {
        EvalContext {
            vm: self.vm,
            mutator: self.mutator,
            env,
            async_ctx: self.async_ctx.clone(),
            current_file: self.current_file.clone(),
            current_module: self.current_module.clone(),
            tracing_enabled: self.tracing_enabled,
            shared_arena: self.shared_arena.clone(),
        }
    }

    pub fn get_pos(&self, expr: ValueRef) -> Option<SourceRange> {
        let store = self.value_metadata.read();
        let id = expr.get_or_create_id();
        id.and_then(|id| store.get_position(id))
    }

    pub fn get_symbol_name_from_id(&self, id: u32) -> Option<String> {
        self.symbol_table
            .read()
            .get_symbol(id)
            .map(|s| s.to_string())
    }

    pub fn get_symbol_name(&self, val: ValueRef) -> Option<String> {
        match val {
            ValueRef::Immediate(packed) => {
                if let ImmediateValue::Symbol(id) = unpack_immediate(packed) {
                    self.symbol_table
                        .read()
                        .get_symbol(id)
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_shared_value(&self, value: ValueRef) -> Option<Arc<SharedValue>> {
        match value {
            ValueRef::Shared(idx) => self.shared_arena.read().get(idx).map(|v| v.clone()),
            _ => None,
        }
    }

    /// Extract a vector of symbol names from a ValueRef
    /// Handles both [sym1 sym2] and (vector sym1 sym2) forms
    pub fn get_vector_of_symbols(&self, val: ValueRef) -> Result<Vec<u32>, String> {
        match val {
            ValueRef::Shared(idx) => {
                if let Some(shared) = self.shared_arena.read().get(idx) {
                    match shared.as_ref() {
                        SharedValue::Vector(items) => self.extract_symbol_names(&items),
                        SharedValue::List(items) if !items.is_empty() => {
                            // Check if it's (vector sym1 sym2 ...)
                            if let Some(head_name) = self.get_symbol_name(items[0]) {
                                if head_name == "vector" {
                                    self.extract_symbol_names(&items[1..])
                                } else {
                                    Err("fn expects a vector of symbols as parameters".to_string())
                                }
                            } else {
                                Err("fn expects a vector of symbols as parameters".to_string())
                            }
                        }
                        _ => Err("fn expects a vector of symbols as parameters".to_string()),
                    }
                } else {
                    Err("Invalid reference".to_string())
                }
            }
            _ => Err("fn expects a vector of symbols as parameters".to_string()),
        }
    }

    /// Helper to extract symbol names from a slice of ValueRefs
    fn extract_symbol_names(&self, items: &[ValueRef]) -> Result<Vec<u32>, String> {
        let mut params = Vec::new();

        for &item in items {
            match self.get_symbol_id(item) {
                Some(sym) => params.push(sym),
                None => return Err("fn parameter list must contain only symbols".to_string()),
            }
        }

        Ok(params)
    }
}
