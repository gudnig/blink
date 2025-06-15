use std::{collections::HashMap, path::PathBuf, sync::Arc};

use parking_lot::RwLock;

use crate::{async_context::AsyncContext, error::BlinkError, goroutine::TokioGoroutineScheduler, metadata::ValueMetadataStore, module::ModuleRegistry, parser::ReaderContext, shared_arena::SharedArena, symbol_table::SymbolTable, telemetry::TelemetryEvent, value::SourceRange, value_ref::{unpack_immediate, ImmediateValue, SharedValue, ValueRef}, Env};

#[derive(Clone)]
pub struct EvalContext {
    pub global_env: Arc<RwLock<Env>>,
    pub env: Arc<RwLock<Env>>,
    pub telemetry_sink: Arc<Option<Box<dyn Fn(TelemetryEvent) + Send + Sync + 'static>>>,
    pub module_registry: Arc<RwLock<ModuleRegistry>>,
    pub file_to_modules: Arc<HashMap<PathBuf, Vec<String>>>,
    pub goroutine_scheduler: Arc<TokioGoroutineScheduler>,
    pub reader_macros: Arc<RwLock<ReaderContext>>,
    pub value_metadata: Arc<RwLock<ValueMetadataStore>>,
    pub current_file: Option<String>,
    pub current_module: Option<String>,
    pub async_ctx: AsyncContext,
    pub tracing_enabled: bool,
    pub symbol_table: Arc<RwLock<SymbolTable>>,
    pub shared_arena: Arc<RwLock<SharedArena>>,
}

impl EvalContext {
    pub fn new(parent: Arc<RwLock<Env>>) -> Self {
        EvalContext {
            global_env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            env: Arc::new(RwLock::new(Env::with_parent(parent.clone()))),
            current_module: None,
            telemetry_sink: Arc::new(None),
            module_registry: Arc::new(RwLock::new(ModuleRegistry::new())),
            value_metadata: Arc::new(RwLock::new(ValueMetadataStore::new())),
            current_file: None,
            symbol_table: Arc::new(RwLock::new(SymbolTable::new())),
            tracing_enabled: false,
            reader_macros: Arc::new(RwLock::new(ReaderContext::new())),
            file_to_modules: Arc::new(HashMap::new()),
            async_ctx: AsyncContext::default(),
            goroutine_scheduler: Arc::new(TokioGoroutineScheduler::new()),
            shared_arena: Arc::new(RwLock::new(SharedArena::new())),
        }
    }

    pub fn type_tag(&self, value: ValueRef) -> String {
        let arena = self.shared_arena.read();
        value.type_tag(&arena).to_string()
    }

    

    pub fn get(&self, key: &str) -> Option<ValueRef> {
        let module_registry = self.module_registry.read();
        self.env.read().get_with_registry(key, &module_registry)
    }

    pub fn set(&self, key: &str, val: ValueRef) {
        self.env.write().set(key, val)
    }

    pub fn intern_symbol(&self, name: &str) -> ValueRef {
        let symbol_id = self.symbol_table.write().intern(name);
        ValueRef::symbol(symbol_id)
    }
    
    pub fn resolve_symbol_name(&self, symbol_id: u32) -> Option<String> {
        self.symbol_table.read().get_symbol(symbol_id).map(|s| s.to_string())
    }

    pub fn intern_keyword(&mut self, name: &str) -> ValueRef {
        // Store with ":" prefix in symbol table
        let full_name = format!(":{}", name);
        let id = self.symbol_table.write().intern(&full_name);
        ValueRef::keyword(id)  // Use a different immediate tag
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
            env,
            async_ctx: self.async_ctx.clone(),
            goroutine_scheduler: self.goroutine_scheduler.clone(),
            reader_macros: self.reader_macros.clone(),
            file_to_modules: self.file_to_modules.clone(),
            module_registry: self.module_registry.clone(),
            value_metadata: self.value_metadata.clone(),
            telemetry_sink: self.telemetry_sink.clone(),
            symbol_table: self.symbol_table.clone(),
            global_env: self.global_env.clone(),
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


    pub fn resolve_module_symbol(&self, module_alias: &str, symbol: &str) -> Result<ValueRef, BlinkError> {
        // Step 1: Look up module alias -> full module name (acquire and release lock)
        let module_name = {
            let env = self.env.read();
        env.available_modules.get(module_alias).cloned()
    }.ok_or_else(|| BlinkError::eval(format!("Module alias '{}' not found", module_alias)))?;
    
    // Step 2: Get the module from registry (new lock scope)
    let module = {
        let module_registry = self.module_registry.read();
        module_registry.get_module(&module_name).clone() // Clone the Arc
    }.ok_or_else(|| BlinkError::eval(format!("Module '{}' not found", module_name)))?;
    
    // Step 3: Look up symbol in module environment (new lock scope)
    let result = {
        let module_guard = module.read();
        let env_guard = module_guard.env.read();
        env_guard.get_local(symbol)
    };
    
    result.ok_or_else(|| BlinkError::eval(format!("Symbol '{}' not found in module '{}'", symbol, module_name)))
}

   pub fn resolve_symbol(&self, name: &str) -> Result<ValueRef, BlinkError> {
        // Check for qualified names (module/symbol)
        if let Some((module_alias, symbol)) = name.split_once('/') {
            self.resolve_module_symbol(module_alias, symbol)
        } else {
            // Check local environment
            let env = self.env.read();
            env.get_local(name)
                .ok_or_else(|| BlinkError::undefined_symbol(name))
        }
    }

    
    pub fn set_symbol(&mut self, name: &str, value: ValueRef) {
        self.env.write().set(name, value);
    }

    pub fn get_symbol_name(&self, val: ValueRef) -> Option<String> {
        match val {
            ValueRef::Immediate(packed) => {
                if let ImmediateValue::Symbol(id) = unpack_immediate(packed) {
                    self.symbol_table.read().get_symbol(id).map(|s| s.to_string())
                } else {
                    None
                }
            }
            ValueRef::Shared(idx) => {
                if let Some(shared) = self.shared_arena.read().get(idx) {
                    match shared.as_ref() {
                        SharedValue::Symbol(name) => Some(name.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_shared_value(&self, value: ValueRef) -> Option<Arc<SharedValue>> {
        match value {
            ValueRef::Shared(idx) => {
                self.shared_arena.read().get(idx).map(|v| v.clone())
            }
            _ => None,
        }
    }

    /// Extract a vector of symbol names from a ValueRef
    /// Handles both [sym1 sym2] and (vector sym1 sym2) forms
    pub fn get_vector_of_symbols(&self, val: ValueRef) -> Result<Vec<String>, String> {
        match val {
            ValueRef::Shared(idx) => {
                if let Some(shared) = self.shared_arena.read().get(idx) {
                    match shared.as_ref() {
                        SharedValue::Vector(items) => {
                            self.extract_symbol_names(items)
                        }
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
    fn extract_symbol_names(&self, items: &[ValueRef]) -> Result<Vec<String>, String> {
        let mut params = Vec::new();
        
        for &item in items {
            match self.get_symbol_name(item) {
                Some(name) => params.push(name),
                None => return Err("fn parameter list must contain only symbols".to_string()),
            }
        }
        
        Ok(params)
    }
}