use std::{collections::HashMap, sync::Arc};

use crate::{
    error::BlinkError,
    runtime::{BlinkVM, CompiledFunction, LabelPatch, Macro, Opcode},
    value::{unpack_immediate, GcPtr, ImmediateValue, ValueRef},
};

#[derive(Debug, Clone)]
struct LoopFrame {
    start_label: u16,
    binding_count: u8,
    binding_registers: Vec<u8>, // Registers holding loop bindings
}

// The main bytecode compiler
pub struct BytecodeCompiler {
    vm: Arc<BlinkVM>,
    bytecode: Vec<u8>,
    constants: Vec<ValueRef>,
    next_register: u8,
    scope_stack: Vec<HashMap<u32, u8>>, // symbol_id -> register
    current_module: u32,                // Add this field

    loop_stack: Vec<LoopFrame>,
    // For closure support
    upvalue_stack: Vec<HashMap<u32, u8>>, // symbol_id -> upvalue_index
    captured_symbols: Vec<(u32, u8)>,     // (symbol_id, parent_register)

    // Label management
    next_label: u16,
    label_positions: HashMap<u16, usize>,
    label_patches: Vec<LabelPatch>,
}

#[derive(Debug)]
enum SplicePart {
    Item(ValueRef),         // Regular item (gets quoted if symbol)
    Splice(ValueRef),       // Expression to be spliced
    UnquotedItem(ValueRef), // From unquote (never gets quoted)
}

impl BytecodeCompiler {
    pub fn new(vm: Arc<BlinkVM>, current_module: u32) -> Self {
        Self {
            vm,
            bytecode: Vec::new(),
            constants: Vec::new(),
            next_register: 1, // Register 0 reserved for return value
            scope_stack: Vec::new(),
            current_module,
            loop_stack: Vec::new(),
            upvalue_stack: Vec::new(),
            captured_symbols: Vec::new(),
            next_label: 0,
            label_positions: HashMap::new(),
            label_patches: Vec::new(),
        }
    }

    fn reset(&mut self) {
        self.bytecode.clear();
        self.constants.clear();
        self.next_register = 0;
        self.scope_stack.clear();
        self.scope_stack.push(HashMap::new());
        self.next_label = 0;
        self.label_patches.clear();
    }

    fn alloc_register(&mut self) -> u8 {
        // Start allocation from register 1, not 0
        self.next_register += 1;
        if self.next_register == 1 {
            self.next_register = 2; // Skip register 0
        }
        self.next_register - 1
    }

    fn resolve_symbol_for_compilation(&self, symbol_id: u32) -> Result<ValueRef, String> {
        // First check local scope bindings
        for scope in self.scope_stack.iter().rev() {
            if let Some(&reg) = scope.get(&symbol_id) {
                // This is a local binding, not a macro
                return Err("Local binding".to_string());
            }
        }

        // Check module-level definitions (where macros would be)
        if let Some(value) = self
            .vm
            .module_registry
            .read()
            .resolve_symbol(self.current_module, symbol_id)
        {
            return Ok(value);
        }

        Err("Symbol not found".to_string())
    }

    // INSTRUCTION EMISSION

    fn emit_u8(&mut self, value: u8) {
        self.bytecode.push(value);
    }

    fn emit_u16(&mut self, value: u16) {
        self.bytecode.extend_from_slice(&value.to_le_bytes());
    }

    fn emit_u32(&mut self, value: u32) {
        self.bytecode.extend_from_slice(&value.to_le_bytes());
    }

    fn emit_i16(&mut self, value: i16) {
        self.bytecode.extend_from_slice(&value.to_le_bytes());
    }

    // CONSTANT POOL MANAGEMENT

    fn add_constant(&mut self, value: ValueRef) -> u8 {
        // Check if constant already exists
        for (i, &existing) in self.constants.iter().enumerate() {
            if existing == value {
                return i as u8;
            }
        }

        // Add new constant
        let index = self.constants.len();

        if index > 255 {
            panic!("Too many constants (max 256)");
        }
        self.constants.push(value);

        index as u8
    }

    // HIGH-LEVEL INSTRUCTION EMISSION

    fn emit_load_immediate(&mut self, reg: u8, value: ValueRef) {
        match value {
            ValueRef::Immediate(packed) => {
                let imm = unpack_immediate(packed);

                match imm {
                    ImmediateValue::Number(n) if n.fract() == 0.0 && n >= 0.0 && n <= 255.0 => {
                        // Small integer - emit directly
                        self.emit_u8(Opcode::LoadImm8 as u8);
                        self.emit_u8(reg);
                        self.emit_u8(n as u8);
                    }
                    ImmediateValue::Number(n) if n.fract() == 0.0 && n >= 0.0 && n <= 65535.0 => {
                        // Medium integer - emit as 16-bit
                        self.emit_u8(Opcode::LoadImm16 as u8);
                        self.emit_u8(reg);
                        self.emit_u16(n as u16);
                    }
                    _ => {
                        // Complex immediate - use constant pool
                        let const_idx = self.add_constant(value);

                        self.emit_u8(Opcode::LoadImmConst as u8);
                        self.emit_u8(reg);
                        self.emit_u8(const_idx);
                    }
                }
            }
            _ => {
                // Heap/Native value - use constant pool
                let const_idx = self.add_constant(value);

                self.emit_u8(Opcode::LoadImmConst as u8);
                self.emit_u8(reg);
                self.emit_u8(const_idx);
            }
        }
    }

    // LABEL MANAGEMENT

    fn alloc_label(&mut self) -> u16 {
        let label = self.next_label;
        self.next_label += 1;
        label
    }

    fn emit_jump_if_true(&mut self, test_reg: u8, label: u16) {
        self.emit_u8(Opcode::JumpIfTrue as u8);
        self.emit_u8(test_reg);
        let patch_offset = self.bytecode.len();
        self.label_patches.push(LabelPatch {
            bytecode_offset: patch_offset,
            label_id: label,
        });
        self.emit_i16(0); // Placeholder
    }

    fn emit_jump_if_false(&mut self, test_reg: u8, label: u16) {
        self.emit_u8(Opcode::JumpIfFalse as u8);
        self.emit_u8(test_reg);
        let patch_offset = self.bytecode.len();
        self.label_patches.push(LabelPatch {
            bytecode_offset: patch_offset,
            label_id: label,
        });
        self.emit_i16(0); // Placeholder
    }

    fn emit_jump(&mut self, label: u16) {
        self.emit_u8(Opcode::Jump as u8);
        let patch_offset = self.bytecode.len();

        // Check if this label was already emitted
        if let Some(&target_pos) = self.label_positions.get(&label) {
            // Label already exists - patch immediately
            let offset = target_pos as i16 - (patch_offset as i16 + 2);

            self.emit_i16(offset);
        } else {
            // Label not yet emitted - add to patches list
            self.label_patches.push(LabelPatch {
                bytecode_offset: patch_offset,
                label_id: label,
            });

            self.emit_i16(0); // Placeholder
        }
    }

    fn emit_label(&mut self, label: u16) {
        let current_pos = self.bytecode.len();

        // Store the label position for later use
        self.label_positions.insert(label, current_pos);

        // Patch all existing jumps to this label
        for patch in &self.label_patches {
            if patch.label_id == label {
                let jump_pos = patch.bytecode_offset;
                let offset = current_pos as i16 - (jump_pos as i16 + 2);

                // Write the offset back into bytecode
                let offset_bytes = offset.to_le_bytes();
                self.bytecode[jump_pos] = offset_bytes[0];
                self.bytecode[jump_pos + 1] = offset_bytes[1];
            }
        }

        // Remove processed patches
        let before_count = self.label_patches.len();
        self.label_patches.retain(|patch| patch.label_id != label);
        let after_count = self.label_patches.len();
    }

    // SCOPE MANAGEMENT

    fn enter_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        self.scope_stack.pop();
    }

    fn bind_local_symbol(&mut self, symbol_id: u32, register: u8) {
        if let Some(current_scope) = self.scope_stack.last_mut() {
            current_scope.insert(symbol_id, register);
        }
    }

    fn resolve_any_local_symbol(&self, symbol_id: u32) -> Option<u8> {
        for scope in self.scope_stack.iter().rev() {
            if let Some(&register) = scope.get(&symbol_id) {
                return Some(register);
            }
        }
        None
    }

    fn resolve_local_symbol(&self, symbol_id: u32) -> Option<u8> {
        // Only look in the CURRENT scope (for free variable detection)
        if let Some(current_scope) = self.scope_stack.last() {
            current_scope.get(&symbol_id).copied()
        } else {
            None
        }
    }

    // MAIN COMPILATION METHODS

    fn compile_expression(&mut self, expr: ValueRef) -> Result<u8, String> {
        match expr {
            ValueRef::Immediate(packed) => {
                let imm = unpack_immediate(packed);
                match imm {
                    ImmediateValue::Symbol(symbol_id) => self.compile_symbol_reference(symbol_id),
                    _ => {
                        let reg = self.alloc_register();
                        self.emit_load_immediate(reg, expr);
                        Ok(reg)
                    }
                }
            }
            ValueRef::Heap(_) => {
                if let Some(list_items) = expr.get_list() {
                    self.compile_function_call(&list_items)
                } else {
                    let reg = self.alloc_register();
                    self.emit_load_immediate(reg, expr);
                    Ok(reg)
                }
            }
            ValueRef::Native(_) => {
                let reg = self.alloc_register();
                self.emit_load_immediate(reg, expr);
                Ok(reg)
            }
        }
    }

    fn compile_loop(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() < 2 {
            return Err("loop expects at least 2 arguments: bindings and body".to_string());
        }

        let bindings = args[0].get_vec().ok_or("loop bindings must be a vector")?;

        if bindings.len() % 2 != 0 {
            return Err("loop bindings must have even number of elements".to_string());
        }

        let binding_count = bindings.len() / 2;
        let mut binding_registers = Vec::new();
        let mut binding_symbols = Vec::new();

        self.enter_scope();

        // STEP 1: First, allocate and bind ALL variables to their loop registers
        // This ensures variable lookups always resolve to the loop registers
        for i in 0..binding_count {
            let symbol_idx = i * 2;

            // Get symbol
            let symbol_id = if let ValueRef::Immediate(packed) = bindings[symbol_idx] {
                if let ImmediateValue::Symbol(sym_id) = unpack_immediate(packed) {
                    sym_id
                } else {
                    return Err("loop binding names must be symbols".to_string());
                }
            } else {
                return Err("loop binding names must be symbols".to_string());
            };

            // Allocate the loop binding register FIRST
            let binding_reg = self.alloc_register();

            // Bind the symbol to the loop register IMMEDIATELY
            // This ensures all references to this symbol use the loop register
            self.bind_local_symbol(symbol_id, binding_reg);

            binding_registers.push(binding_reg);
            binding_symbols.push(symbol_id);
        }

        // STEP 2: Now compile initial values and store them in the loop registers
        for i in 0..binding_count {
            let value_idx = i * 2 + 1;
            let binding_reg = binding_registers[i];

            // Compile initial value - this might use temporary registers
            let value_reg = self.compile_expression(bindings[value_idx])?;

            // Move the initial value to the loop register
            if value_reg != binding_reg {
                self.emit_u8(Opcode::LoadLocal as u8);
                self.emit_u8(binding_reg); // destination (loop register)
                self.emit_u8(value_reg); // source (temporary register)
            }
        }

        // Create loop frame
        let start_label = self.alloc_label();
        let loop_frame = LoopFrame {
            start_label,
            binding_count: binding_count as u8,
            binding_registers: binding_registers.clone(),
        };
        self.loop_stack.push(loop_frame);

        // Emit the loop start label
        self.emit_label(start_label);

        let mut result_reg = self.alloc_register();

        // STEP 3: Compile body expressions - now all variable lookups use loop registers
        for (i, &expr) in args[1..].iter().enumerate() {
            result_reg = self.compile_expression(expr)?;

            if i == args[1..].len() - 1 {
                if let Some(_) = self.check_tail_call(expr)? {
                    break;
                }
            }
        }

        // Clean up
        self.loop_stack.pop();
        self.exit_scope();

        Ok(result_reg)
    }

    // In your compile_recur method, add debug info about the values:
    fn compile_recur(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        let loop_frame = self
            .loop_stack
            .last()
            .cloned()
            .ok_or("recur used outside of loop")?;

        if args.len() != loop_frame.binding_count as usize {
            return Err(format!(
                "recur expects {} arguments, got {}",
                loop_frame.binding_count,
                args.len()
            ));
        }

        // Compile new values for loop bindings
        let mut new_value_regs = Vec::new();
        for (i, &arg) in args.iter().enumerate() {
            let value_reg = self.compile_expression(arg)?;
            new_value_regs.push(value_reg);
        }

        // Update binding registers with new values
        for (i, &new_value_reg) in new_value_regs.iter().enumerate() {
            let binding_reg = loop_frame.binding_registers[i];

            // This should move the new value to the binding register
            self.emit_u8(Opcode::LoadLocal as u8);
            self.emit_u8(binding_reg); // destination
            self.emit_u8(new_value_reg); // source
        }

        self.emit_jump(loop_frame.start_label);

        Ok(0)
    }

    fn compile_fn(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() < 2 {
            return Err("fn expects at least 2 arguments: [name] parameters and body".to_string());
        }

        // Parse name vs anonymous function
        let (function_name, params_index) = if args.len() >= 3 {
            // Check if first arg is a symbol (potential function name)
            if let ValueRef::Immediate(packed) = args[0] {
                if let ImmediateValue::Symbol(name_symbol) = unpack_immediate(packed) {
                    // First arg is a symbol, treat as named function
                    (Some(name_symbol), 1)
                } else {
                    // First arg is not a symbol, treat as anonymous
                    (None, 0)
                }
            } else {
                // First arg is not immediate, treat as anonymous
                (None, 0)
            }
        } else {
            // Only 2 args, must be anonymous: (fn [params] body)
            (None, 0)
        };

        // Parse parameter list
        let params = args[params_index]
            .get_vec()
            .ok_or("fn parameter list must be a vector")?;

        let param_symbols: Result<Vec<u32>, String> = params
            .iter()
            .map(|p| match p {
                ValueRef::Immediate(packed) => {
                    if let ImmediateValue::Symbol(sym_id) = unpack_immediate(*packed) {
                        Ok(sym_id)
                    } else {
                        Err("fn parameters must be symbols".to_string())
                    }
                }
                _ => Err("fn parameters must be symbols".to_string()),
            })
            .collect();
        let param_symbols = param_symbols?;

        // Validate parameter count
        if param_symbols.len() > 255 {
            return Err("fn cannot have more than 255 parameters".to_string());
        }

        // Save current compilation state
        let saved_bytecode = std::mem::take(&mut self.bytecode);
        let saved_constants = std::mem::take(&mut self.constants);
        let saved_register_count = self.next_register;
        let saved_labels = std::mem::take(&mut self.label_positions);
        let saved_patches = std::mem::take(&mut self.label_patches);
        let saved_next_label = self.next_label;

        // Reset for function compilation
        self.next_register = 1; // Register 0 reserved for return value
        self.next_label = 0;

        // Enter function scope

        self.enter_scope();

        self.debug_closure_compilation("After entering function scope");

        // If named function, bind the name to itself for recursion
        // We'll use a special register slot that gets set up at function call time
        if let Some(name_symbol) = function_name {
            // Reserve a register for the function self-reference
            let self_ref_reg = self.alloc_register();
            self.bind_local_symbol(name_symbol, self_ref_reg);

            // At function entry, the function object will be loaded into this register
            // This happens in the VM when the function is called
        }

        // Bind parameters to registers (params start after self-reference if named)
        let param_start_reg = if function_name.is_some() { 2 } else { 1 };
        for (i, &param_symbol) in param_symbols.iter().enumerate() {
            let target_reg = (param_start_reg + i) as u8;

            self.bind_local_symbol(param_symbol, target_reg);
        }

        self.debug_closure_compilation("After binding parameters");
        self.next_register = (param_start_reg + param_symbols.len()) as u8;

        // Analyze closure requirements - no need for upvalue array register anymore
        let body_exprs = &args[(params_index + 1)..];
        self.analyze_closures(body_exprs)?;

        self.debug_closure_compilation("After analyzing closures");

        // For named functions, emit instruction to load self-reference
        if let Some(_name_symbol) = function_name {
            // The VM will handle setting up the self-reference register
            // when the function is called. We emit a special opcode here.
            self.emit_u8(Opcode::SetupSelfReference as u8); // Use enum
            self.emit_u8(1); // Self-reference register
        }

        // Compile function body expressions
        let mut result_reg = self.alloc_register(); // Default return value

        for (i, &expr) in body_exprs.iter().enumerate() {
            result_reg = self.compile_expression(expr)?;

            // Check for tail call optimization on final expression
            if i == body_exprs.len() - 1 {
                if let Some(_tail_call_reg) = self.check_tail_call(expr)? {
                    // Already emitted TailCall - function will return
                    self.exit_scope();

                    // Extract function compilation results
                    let function_bytecode = std::mem::take(&mut self.bytecode);
                    let function_constants = std::mem::take(&mut self.constants);
                    let function_registers = self.next_register;

                    // Restore parent compilation state
                    self.bytecode = saved_bytecode;
                    self.constants = saved_constants;
                    self.next_register = saved_register_count;
                    self.label_positions = saved_labels;
                    self.label_patches = saved_patches;
                    self.next_label = saved_next_label;

                    let compiled_fn = CompiledFunction {
                        bytecode: function_bytecode,
                        constants: function_constants,
                        parameter_count: param_symbols.len() as u8,
                        register_count: function_registers,
                        module: self.current_module,
                        register_start: param_start_reg as u8,
                        has_self_reference: function_name.is_some(),
                    };

                    return self.create_closure_object(compiled_fn);
                }
            }
        }

        // Regular return
        self.emit_u8(Opcode::Return as u8);
        self.emit_u8(result_reg);
        self.exit_scope();

        // Extract function compilation results
        let function_bytecode = std::mem::take(&mut self.bytecode);
        let function_constants = std::mem::take(&mut self.constants);
        let function_registers = self.next_register;

        // Restore parent compilation state
        self.bytecode = saved_bytecode;
        self.constants = saved_constants;
        self.next_register = saved_register_count;
        self.label_positions = saved_labels;
        self.label_patches = saved_patches;
        self.next_label = saved_next_label;

        let compiled_fn = CompiledFunction {
            bytecode: function_bytecode,
            constants: function_constants,
            parameter_count: param_symbols.len() as u8,
            register_count: function_registers,
            module: self.current_module,
            register_start: param_start_reg as u8,
            has_self_reference: function_name.is_some(),
        };

        self.create_closure_object(compiled_fn)
    }

    fn analyze_closures(&mut self, exprs: &[ValueRef]) -> Result<(), String> {
        // Walk the AST to find free variables that need to be captured as upvalues
        for &expr in exprs {
            self.find_free_variables(expr)?;
        }
        Ok(())
    }

    fn extract_parameter_symbols(&self, params: ValueRef) -> Result<Vec<u32>, String> {
        let params_vec = if let Some(params) = params.get_vec() {
            params
        } else {
            return Err("Parameters must be a vector".to_string());
        };

        let param_symbols: Result<Vec<u32>, String> = params_vec
            .iter()
            .map(|p| match p {
                ValueRef::Immediate(packed) => {
                    if let ImmediateValue::Symbol(sym_id) = unpack_immediate(*packed) {
                        Ok(sym_id)
                    } else {
                        Err("Parameters must be symbols".to_string())
                    }
                }
                _ => Err("Parameters must be symbols".to_string()),
            })
            .collect();

        param_symbols
    }

    /// Detect variadic parameters and return the final parameter list
    /// Handles patterns like [a b & rest] -> returns [a, b, rest] with is_variadic=true
    fn detect_variadic_params(&self, param_symbols: &[u32]) -> (Vec<u32>, bool) {
        let mut final_params = Vec::new();
        let mut is_variadic = false;

        let mut i = 0;
        while i < param_symbols.len() {
            let symbol_name = self
                .vm
                .symbol_table
                .read()
                .get_symbol(param_symbols[i])
                .unwrap_or_default();

            if symbol_name == "&" {
                // Found variadic marker
                if i + 1 < param_symbols.len() {
                    // Add the rest parameter
                    final_params.push(param_symbols[i + 1]);
                    is_variadic = true;
                    break; // Stop processing after rest parameter
                } else {
                    // This should be an error, but return what we have
                    break;
                }
            } else {
                // Regular parameter
                final_params.push(param_symbols[i]);
            }
            i += 1;
        }

        (final_params, is_variadic)
    }

    fn compile_macro(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() < 2 {
            return Err("macro expects at least 2 arguments: params and body".to_string());
        }

        let params = args[0];
        let body = &args[1..];

        // Parse parameters (same logic as fn)
        let param_symbols = self.extract_parameter_symbols(params)?;
        let (variadic_params, is_variadic) = self.detect_variadic_params(&param_symbols);

        // Create MacroDefinition with raw AST
        let macro_def = Macro {
            params: variadic_params,
            body: body.to_vec(), // Store raw AST - no compilation!
            is_variadic,
            module: self.current_module,
        };

        // Allocate to GC heap
        let macro_obj = self.vm.alloc_macro(macro_def);
        let macro_value = ValueRef::Heap(GcPtr::new(macro_obj));

        // Store in constants and return register
        let constant_idx = self.add_constant(macro_value);
        let result_reg = self.alloc_register();

        self.emit_u8(Opcode::LoadImmConst as u8);
        self.emit_u8(result_reg);
        self.emit_u8(constant_idx);

        Ok(result_reg)
    }

    fn find_free_variables(&mut self, expr: ValueRef) -> Result<(), String> {
        println!("🔍 Analyzing expression: {}", expr);

        match expr {
            ValueRef::Immediate(packed) => {
                if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                    let symbol_name = self
                        .vm
                        .symbol_table
                        .read()
                        .get_symbol(symbol_id)
                        .unwrap_or_default();
                    println!("   → Found symbol: {} (id: {})", symbol_name, symbol_id);

                    let local_result = self.resolve_local_symbol(symbol_id);
                    println!("     Local scope result: {}", local_result.unwrap_or(0));

                    if self.resolve_local_symbol(symbol_id).is_none() {
                        if let Some(parent_reg) = self.resolve_in_parent_scopes(symbol_id) {
                            if !self
                                .captured_symbols
                                .iter()
                                .any(|(sym, _)| *sym == symbol_id)
                            {
                                println!(
                                    "     ✅ CAPTURING as upvalue: {} at register {}",
                                    symbol_name, parent_reg
                                );
                                self.captured_symbols.push((symbol_id, parent_reg));
                                // Store both!
                            }
                        } else {
                            println!("     → Not in parent scopes, might be global");
                        }
                    } else {
                        println!("     → Found in local scope, no capture needed");
                    }
                } else {
                    println!("   → Non-symbol immediate: {}", unpack_immediate(packed));
                }
            }
            ValueRef::Heap(_) => {
                if let Some(list_items) = expr.get_list() {
                    println!("   → Analyzing list with {} items", list_items.len());
                    // Recursively analyze ALL list elements
                    for (i, &item) in list_items.iter().enumerate() {
                        println!("     Analyzing list item {}: {}", i, item);
                        self.find_free_variables(item)?;
                    }
                } else if let Some(vec_items) = expr.get_vec() {
                    println!("   → Analyzing vector with {} items", vec_items.len());
                    // Recursively analyze vector elements
                    for (i, &item) in vec_items.iter().enumerate() {
                        println!("     Analyzing vector item {}: {}", i, item);
                        self.find_free_variables(item)?;
                    }
                } else {
                    println!("   → Heap value (not list/vector)");
                }
            }
            _ => {
                println!("   → Other value type");
            }
        }
        Ok(())
    }

    fn debug_closure_compilation(&self, context: &str) {
        println!("=== Debug: {} ===", context);
        println!("Scope stack levels: {}", self.scope_stack.len());

        for (level, scope) in self.scope_stack.iter().enumerate() {
            println!("Scope level {}: {} bindings", level, scope.len());
            for (&symbol_id, &register) in scope {
                let symbol_name = self
                    .vm
                    .symbol_table
                    .read()
                    .get_symbol(symbol_id)
                    .unwrap_or_default();
                println!(
                    "  {} (id: {}) -> register {}",
                    symbol_name, symbol_id, register
                );
            }
        }

        println!("Captured symbols: {} total", self.captured_symbols.len());
        for &(symbol_id, _) in &self.captured_symbols {
            let symbol_name = self
                .vm
                .symbol_table
                .read()
                .get_symbol(symbol_id)
                .unwrap_or_default();
            println!("  Captured: {} (id: {})", symbol_name, symbol_id);

            if let Some(parent_reg) = self.resolve_in_parent_scopes(symbol_id) {
                println!("    Found in parent scope at register: {}", parent_reg);
            } else {
                println!("    ERROR: NOT FOUND in parent scopes!");
            }
        }
        println!("=====================================");
    }

    fn resolve_in_parent_scopes(&self, symbol_id: u32) -> Option<u8> {
        // Look through LOCAL scope stack to find symbol in parent scopes
        // We need to find the REGISTER where the value is stored, not an upvalue index

        if self.scope_stack.len() <= 1 {
            return None; // No parent scopes
        }

        // Look through all parent scopes (excluding current scope)
        // Go from most recent parent to oldest
        for scope in self.scope_stack.iter().rev().skip(1) {
            if let Some(&register) = scope.get(&symbol_id) {
                return Some(register);
            }
        }

        None
    }

    fn resolve_upvalue(&self, symbol_id: u32) -> Option<u8> {
        self.captured_symbols
            .iter()
            .position(|(sym, _)| *sym == symbol_id)
            .map(|pos| pos as u8)
    }

    fn create_closure_object(&mut self, compiled_fn: CompiledFunction) -> Result<u8, String> {
        println!(
            "DEBUG: create_closure_object called with {} captured symbols",
            self.captured_symbols.len()
        );

        if self.captured_symbols.is_empty() {
            println!("DEBUG: Taking simple function path (no upvalues)");
            // Simple function - no upvalues
            let func_obj = self.vm.alloc_user_defined_fn(compiled_fn);
            let result_reg = self.alloc_register();
            self.emit_load_immediate(result_reg, ValueRef::Heap(GcPtr::new(func_obj)));
            Ok(result_reg)
        } else {
            // Closure - emit single instruction with all upvalue capture info
            println!("DEBUG: Taking closure path with upvalues");
            // First, allocate the template CompiledFunction
            let template_obj = self.vm.alloc_user_defined_fn(compiled_fn);
            let template_reg = self.alloc_register();
            self.emit_load_immediate(template_reg, ValueRef::Heap(GcPtr::new(template_obj)));

            // Collect upvalue capture information
            // Collect upvalue capture information
            let mut upvalue_captures = Vec::new();
            for (symbol_id, parent_reg) in &self.captured_symbols {
                println!(
                    "DEBUG: Using stored info: symbol {} at register {}",
                    symbol_id, parent_reg
                );
                upvalue_captures.push((*parent_reg, *symbol_id));
            }

            println!(
                "DEBUG: Final upvalue_captures: {} items",
                upvalue_captures.len()
            );

            let result_reg = self.alloc_register();

            // Emit single instruction with all capture info
            self.emit_u8(Opcode::CreateClosure as u8);
            self.emit_u8(result_reg); // destination register
            self.emit_u8(template_reg); // template function register
            self.emit_u8(upvalue_captures.len() as u8); // number of upvalues

            // Emit capture info for each upvalue
            for (parent_reg, symbol_id) in upvalue_captures {
                self.emit_u8(parent_reg); // where to get the value
                self.emit_u32(symbol_id); // symbol for debugging
            }

            Ok(result_reg)
        }
    }

    fn try_compile_comparison(&mut self, symbol_id: u32, args: &[ValueRef]) -> Result<u8, String> {
        let symbol_name = self
            .vm
            .symbol_table
            .read()
            .get_symbol(symbol_id)
            .ok_or("Unknown symbol")?;

        match symbol_name.as_str() {
            "=" => self.compile_equality_chain(args),
            "<" => self.compile_ordered_chain(args, Opcode::Lt),
            ">" => self.compile_ordered_chain(args, Opcode::Gt),
            "<=" => self.compile_ordered_chain(args, Opcode::LtEq),
            ">=" => self.compile_ordered_chain(args, Opcode::GtEq),
            _ => Err("Not a comparison operator".to_string()),
        }
    }

    fn compile_equality_chain(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() < 2 {
            return Err("= expects at least 2 arguments".to_string());
        }

        if args.len() == 2 {
            // Simple binary comparison
            let left_reg = self.compile_expression(args[0])?;
            let right_reg = self.compile_expression(args[1])?;
            let result_reg = self.alloc_register();

            self.emit_u8(Opcode::Eq as u8);
            self.emit_u8(result_reg);
            self.emit_u8(left_reg);
            self.emit_u8(right_reg);

            return Ok(result_reg);
        } else {
            // Chain multiple comparisons with AND logic
            let first_reg = self.compile_expression(args[0])?;
            let mut current_result = self.alloc_register();
            self.emit_load_immediate(current_result, ValueRef::boolean(true));

            for arg in &args[1..] {
                let arg_reg = self.compile_expression(*arg)?;
                let cmp_result = self.alloc_register();
                let and_result = self.alloc_register();

                // Compare first_reg == arg_reg
                self.emit_u8(Opcode::Eq as u8);
                self.emit_u8(cmp_result);
                self.emit_u8(first_reg);
                self.emit_u8(arg_reg);

                // AND with previous result
                // (You'd need to implement logical AND bytecode or use conditionals)
                // For now, could fall back to native function for multi-arg =
            }

            return Ok(current_result);
        }
    }

    fn compile_ordered_chain(&mut self, args: &[ValueRef], base_op: Opcode) -> Result<u8, String> {
        if args.len() == 2 {
            // Binary case
            let left_reg = self.compile_expression(args[0])?;
            let right_reg = self.compile_expression(args[1])?;
            let result_reg = self.alloc_register();

            self.emit_u8(base_op as u8);
            self.emit_u8(result_reg);
            self.emit_u8(left_reg);
            self.emit_u8(right_reg);

            return Ok(result_reg);
        }

        // Multi-argument: implement the same short-circuit logic as your `and`
        let result_reg = self.alloc_register();
        let false_label = self.alloc_label();
        let end_label = self.alloc_label();

        for i in 0..(args.len() - 1) {
            let left_reg = self.compile_expression(args[i])?;
            let right_reg = self.compile_expression(args[i + 1])?;
            let cmp_reg = self.alloc_register();

            // Compare args[i] > args[i+1]
            self.emit_u8(base_op as u8);
            self.emit_u8(cmp_reg);
            self.emit_u8(left_reg);
            self.emit_u8(right_reg);

            // If comparison is false, short-circuit to false
            self.emit_jump_if_false(cmp_reg, false_label);
        }

        // All comparisons passed
        self.emit_load_immediate(result_reg, ValueRef::boolean(true));
        self.emit_jump(end_label);

        // At least one comparison failed
        self.emit_label(false_label);
        self.emit_load_immediate(result_reg, ValueRef::boolean(false));

        self.emit_label(end_label);
        Ok(result_reg)
    }

    fn compile_cond(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.is_empty() {
            return Err("cond expects at least one condition-expression pair".to_string());
        }

        if args.len() % 2 != 0 {
            return Err("cond expects pairs of condition-expression".to_string());
        }

        let result_reg = self.alloc_register();
        let end_label = self.alloc_label();
        let mut next_condition_labels = Vec::new();

        // Generate labels for each condition check
        for _ in 0..(args.len() / 2) {
            next_condition_labels.push(self.alloc_label());
        }

        // Process each condition-expression pair
        for i in (0..args.len()).step_by(2) {
            let condition_index = i / 2;

            // Check if this is an :else clause
            if let ValueRef::Immediate(packed) = args[i] {
                if let ImmediateValue::Keyword(keyword_id) = unpack_immediate(packed) {
                    let keyword = self
                        .vm
                        .symbol_table
                        .read()
                        .get_symbol(keyword_id)
                        .unwrap_or_default();
                    if keyword == "else" {
                        // This is the else clause - just compile the expression
                        let else_reg = self.compile_expression(args[i + 1])?;
                        self.emit_u8(Opcode::LoadLocal as u8);
                        self.emit_u8(result_reg);
                        self.emit_u8(else_reg);
                        self.emit_jump(end_label);
                        break;
                    }
                }
            }

            // Regular condition-expression pair
            let condition_reg = self.compile_expression(args[i])?;

            // If this isn't the last pair, jump to next condition on false
            if condition_index < next_condition_labels.len() - 1 {
                self.emit_jump_if_false(condition_reg, next_condition_labels[condition_index + 1]);
            } else {
                // Last condition - jump to end (setting nil) if false
                let nil_label = self.alloc_label();
                self.emit_jump_if_false(condition_reg, nil_label);

                // Condition true - evaluate expression
                let expr_reg = self.compile_expression(args[i + 1])?;
                self.emit_u8(Opcode::LoadLocal as u8);
                self.emit_u8(result_reg);
                self.emit_u8(expr_reg);
                self.emit_jump(end_label);

                // Condition false - load nil
                self.emit_label(nil_label);
                self.emit_load_immediate(result_reg, ValueRef::nil());
                self.emit_jump(end_label);
                break;
            }

            // Condition true - evaluate expression and jump to end
            let expr_reg = self.compile_expression(args[i + 1])?;
            self.emit_u8(Opcode::LoadLocal as u8);
            self.emit_u8(result_reg);
            self.emit_u8(expr_reg);
            self.emit_jump(end_label);

            // Emit label for next condition (if not the last)
            if condition_index < next_condition_labels.len() - 1 {
                self.emit_label(next_condition_labels[condition_index + 1]);
            }
        }

        self.emit_label(end_label);
        Ok(result_reg)
    }

    fn compile_symbol_reference(&mut self, symbol_id: u32) -> Result<u8, String> {
        let symbol_name = self
            .vm
            .symbol_table
            .read()
            .get_symbol(symbol_id)
            .unwrap_or_default();

        // Try CURRENT scope first (not any local scope)
        if let Some(local_reg) = self.resolve_local_symbol(symbol_id) {
            return Ok(local_reg);
        }

        // Try upvalues BEFORE parent scopes
        if let Some(upvalue_idx) = self.resolve_upvalue(symbol_id) {
            let result_reg = self.alloc_register();
            self.emit_u8(Opcode::LoadUpvalue as u8);
            self.emit_u8(result_reg);
            self.emit_u8(upvalue_idx);
            return Ok(result_reg);
        }

        // Then try parent scopes (for non-captured symbols)
        if let Some(local_reg) = self.resolve_any_local_symbol(symbol_id) {
            return Ok(local_reg);
        }

        // Only allocate result_reg if we need it for upvalues/globals
        let result_reg = self.alloc_register();

        // Fall back to global

        self.emit_u8(Opcode::LoadGlobal as u8);
        self.emit_u8(result_reg);
        self.emit_u32(symbol_id);
        Ok(result_reg)
    }

    fn try_compile_logical_operator(
        &mut self,
        symbol_id: u32,
        args: &[ValueRef],
    ) -> Result<u8, String> {
        let symbol_name = self
            .vm
            .symbol_table
            .read()
            .get_symbol(symbol_id)
            .ok_or("Unknown symbol")?;
        match symbol_name.as_str() {
            "and" => self.compile_and(args),
            "or" => self.compile_or(args),
            "not" => self.compile_not(args),
            _ => Err("Not a logical operator".to_string()),
        }
    }

    fn compile_not(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.is_empty() {
            let result_reg = self.alloc_register();
            self.emit_load_immediate(result_reg, ValueRef::boolean(false));
            return Ok(result_reg);
        }

        if args.len() != 1 {}

        let arg_reg = self.compile_expression(args[0])?;
        let result_reg = self.alloc_register();
        self.emit_u8(Opcode::Not as u8);
        self.emit_u8(result_reg);
        self.emit_u8(arg_reg);
        return Ok(result_reg);
    }

    fn compile_and(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.is_empty() {
            let result_reg = self.alloc_register();
            self.emit_load_immediate(result_reg, ValueRef::boolean(true));
            return Ok(result_reg);
        }

        let result_reg = self.alloc_register();
        let false_label = self.alloc_label();
        let end_label = self.alloc_label();

        for (i, &arg) in args.iter().enumerate() {
            let arg_reg = self.compile_expression(arg)?;

            if i == args.len() - 1 {
                // Last argument - its value becomes the result
                self.emit_u8(Opcode::LoadLocal as u8);
                self.emit_u8(result_reg);
                self.emit_u8(arg_reg);
            } else {
                // Not last - check if falsy and jump out
                self.emit_jump_if_false(arg_reg, false_label);
            }
        }

        self.emit_jump(end_label);

        // False path
        self.emit_label(false_label);
        self.emit_load_immediate(result_reg, ValueRef::boolean(false));

        self.emit_label(end_label);
        Ok(result_reg)
    }

    fn compile_or(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.is_empty() {
            let result_reg = self.alloc_register();
            self.emit_load_immediate(result_reg, ValueRef::boolean(false));
            return Ok(result_reg);
        }

        let result_reg = self.alloc_register();
        let true_label = self.alloc_label();
        let end_label = self.alloc_label();

        for (i, &arg) in args.iter().enumerate() {
            let arg_reg = self.compile_expression(arg)?;

            if i == args.len() - 1 {
                // Last argument - its value becomes the result
                self.emit_u8(Opcode::LoadLocal as u8);
                self.emit_u8(result_reg);
                self.emit_u8(arg_reg);
            } else {
                // Not last - check if truthy and jump out
                self.emit_jump_if_true(arg_reg, true_label);
            }
        }

        self.emit_jump(end_label);

        // True path
        self.emit_label(true_label);
        self.emit_load_immediate(result_reg, ValueRef::boolean(false));

        self.emit_label(end_label);
        Ok(result_reg)
    }

    fn check_tail_call(&mut self, expr: ValueRef) -> Result<Option<u8>, String> {
        // Check if expression is a function call that can be tail-optimized
        if let Some(list_items) = expr.get_list() {
            if !list_items.is_empty() {
                if let ValueRef::Immediate(packed) = list_items[0] {
                    if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                        // Don't tail-optimize special forms or arithmetic operators
                        if !self.is_special_form(symbol_id)
                            && !self.is_arithmetic_operator(symbol_id)
                        {
                            // This is a regular function call - emit as tail call
                            let func_reg = self.alloc_register();

                            // Load function
                            if let Some(local_reg) = self.resolve_any_local_symbol(symbol_id) {
                                self.emit_u8(Opcode::LoadLocal as u8);
                                self.emit_u8(func_reg);
                                self.emit_u8(local_reg);
                            } else if let Some(upvalue_idx) = self.resolve_upvalue(symbol_id) {
                                self.emit_u8(Opcode::LoadUpvalue as u8);
                                self.emit_u8(func_reg);
                                self.emit_u8(upvalue_idx);
                            } else {
                                self.emit_u8(Opcode::LoadGlobal as u8);
                                self.emit_u8(func_reg);
                                self.emit_u32(symbol_id);
                            }

                            // Compile arguments
                            let args = &list_items[1..];
                            for arg in args {
                                self.compile_expression(*arg)?;
                            }

                            // Emit tail call (no result register - direct return)
                            self.emit_u8(Opcode::TailCall as u8);
                            self.emit_u8(func_reg);
                            self.emit_u8(args.len() as u8);

                            return Ok(Some(func_reg));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    fn compile_special_form(&mut self, symbol_id: u32, args: &[ValueRef]) -> Result<u8, String> {
        let symbol_name = self
            .vm
            .symbol_table
            .read()
            .get_symbol(symbol_id)
            .ok_or("Unknown symbol")?;

        match symbol_name.as_str() {
            "def" => self.compile_def(args),
            "if" => self.compile_if(args),
            "let" => self.compile_let(args),
            "do" => self.compile_do(args),
            "quote" => self.compile_quote(args),
            "fn" => self.compile_fn(args),
            "loop" => self.compile_loop(args),
            "recur" => self.compile_recur(args),
            "cond" => self.compile_cond(args),
            "macro" => self.compile_macro(args),
            "quasiquote" => self.compile_quasiquote(args),
            "unquote" => self.compile_unquote(args),
            "unquote-splicing" => self.compile_unquote_splicing(args),
            "future" => self.compile_future(args),
            "complete" => self.compile_complete(args),
            //"go" => self.compile_go(args),
            _ => Err(format!("Special form '{}' not implemented", symbol_name)),
        }
    }

    fn compile_future(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if !args.is_empty() {
            return Err("future expects no arguments".to_string());
        }

        let result_reg = self.alloc_register();

        // Emit CreateFuture opcode
        self.emit_u8(Opcode::CreateFuture as u8);
        self.emit_u8(result_reg);

        Ok(result_reg)
    }

    fn compile_complete(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() != 2 {
            return Err("complete expects exactly 2 arguments: future and value".to_string());
        }

        // Compile future reference
        let future_reg = self.compile_expression(args[0])?;

        // Compile value to complete with
        let value_reg = self.compile_expression(args[1])?;

        let result_reg = self.alloc_register();

        // Emit CompleteFuture opcode
        self.emit_u8(Opcode::CompleteFuture as u8);
        self.emit_u8(result_reg); // Result register (returns success/error)
        self.emit_u8(future_reg); // Future to complete
        self.emit_u8(value_reg); // Value to complete with

        Ok(result_reg)
    }

    fn compile_def(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() != 2 {
            return Err("def expects exactly 2 arguments: name and value".to_string());
        }

        // First argument must be a symbol (the name to define)
        let symbol_id = if let ValueRef::Immediate(packed) = args[0] {
            if let ImmediateValue::Symbol(sym_id) = unpack_immediate(packed) {
                sym_id
            } else {
                return Err("def: first argument must be a symbol".to_string());
            }
        } else {
            return Err("def: first argument must be a symbol".to_string());
        };

        // Compile the value expression
        let value_reg = self.compile_expression(args[1])?;

        // Store the value globally
        self.emit_u8(Opcode::StoreGlobal as u8);
        self.emit_u8(value_reg); // register first
        self.emit_u32(symbol_id); // symbol_id second

        // Return the value that was stored
        Ok(value_reg)
    }

    fn compile_function_call(&mut self, items: &[ValueRef]) -> Result<u8, String> {
        if items.is_empty() {
            // TODO empty should return nil
            return Err("Empty function call".to_string());
        }

        if let ValueRef::Immediate(packed) = items[0] {
            if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                // Check for special forms
                if self.is_special_form(symbol_id) {
                    return self.compile_special_form(symbol_id, &items[1..]);
                }

                // Check for arithmetic operators
                if let Ok(result) = self.try_compile_arithmetic(symbol_id, &items[1..]) {
                    return Ok(result);
                }

                // Check for comparison
                if let Ok(result) = self.try_compile_comparison(symbol_id, &items[1..]) {
                    return Ok(result);
                }

                if let Ok(result) = self.try_compile_logical_operator(symbol_id, &items[1..]) {
                    return Ok(result);
                }

                // Regular function call
                return self.compile_regular_function_call(symbol_id, &items[1..]);
            }
        }

        Err("Unsupported function call".to_string())
    }

    fn compile_if(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() < 2 || args.len() > 3 {
            return Err("if expects 2 or 3 arguments".to_string());
        }

        let condition_reg = self.compile_expression(args[0])?;
        let result_reg = self.alloc_register();

        let else_label = self.alloc_label();
        let end_label = self.alloc_label();

        self.emit_jump_if_false(condition_reg, else_label);

        // Then branch
        let then_reg = self.compile_expression(args[1])?;
        self.emit_u8(Opcode::LoadLocal as u8);
        self.emit_u8(result_reg);
        self.emit_u8(then_reg);
        self.emit_jump(end_label);

        // Else branch
        self.emit_label(else_label);
        if args.len() == 3 {
            let else_reg = self.compile_expression(args[2])?;
            self.emit_u8(Opcode::LoadLocal as u8);
            self.emit_u8(result_reg);
            self.emit_u8(else_reg);
        } else {
            self.emit_load_immediate(result_reg, ValueRef::nil());
        }

        self.emit_label(end_label);
        Ok(result_reg)
    }

    fn compile_let(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() < 2 {
            return Err("let expects at least 2 arguments".to_string());
        }

        let bindings = if let Some(bindings_vec) = args[0].get_vec() {
            bindings_vec
        } else {
            return Err("let first argument must be a vector".to_string());
        };

        if bindings.len() % 2 != 0 {
            return Err("let bindings must be pairs".to_string());
        }

        self.enter_scope();

        // Compile bindings
        for i in (0..bindings.len()).step_by(2) {
            let symbol_id = if let ValueRef::Immediate(packed) = bindings[i] {
                if let ImmediateValue::Symbol(id) = unpack_immediate(packed) {
                    id
                } else {
                    return Err("let binding names must be symbols".to_string());
                }
            } else {
                return Err("let binding names must be symbols".to_string());
            };

            let value_reg = self.compile_expression(bindings[i + 1])?;
            self.bind_local_symbol(symbol_id, value_reg);
        }

        // Compile body
        let mut result_reg = self.alloc_register();
        self.emit_load_immediate(result_reg, ValueRef::nil());

        for body_expr in &args[1..] {
            result_reg = self.compile_expression(*body_expr)?;
        }

        self.exit_scope();
        Ok(result_reg)
    }

    fn compile_do(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        let mut result_reg = self.alloc_register();
        self.emit_load_immediate(result_reg, ValueRef::nil());

        for expr in args {
            result_reg = self.compile_expression(*expr)?;
        }

        Ok(result_reg)
    }

    fn compile_quote(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() != 1 {
            return Err("quote expects 1 argument".to_string());
        }

        let reg = self.alloc_register();
        self.emit_load_immediate(reg, args[0]);
        Ok(reg)
    }

    fn try_compile_arithmetic(&mut self, symbol_id: u32, args: &[ValueRef]) -> Result<u8, String> {
        let symbol_name = self
            .vm
            .symbol_table
            .read()
            .get_symbol(symbol_id)
            .ok_or("Unknown symbol")?;

        if !matches!(symbol_name.as_str(), "+" | "-" | "*" | "/") {
            return Err("Not an arithmetic operator".to_string());
        }

        // Handle zero arguments
        if args.is_empty() {
            let result_reg = self.alloc_register();
            let identity_value = match symbol_name.as_str() {
                "+" => ValueRef::number(0.0), // Identity for addition
                "*" => ValueRef::number(1.0), // Identity for multiplication
                "-" | "/" => return Err(format!("{} requires at least 1 argument", symbol_name)),
                _ => unreachable!(),
            };
            self.emit_load_immediate(result_reg, identity_value);
            return Ok(result_reg);
        }

        // Handle single argument
        if args.len() == 1 {
            match symbol_name.as_str() {
                "+" | "*" => {
                    // For + and *, single argument just returns itself
                    return self.compile_expression(args[0]);
                }
                "-" => {
                    // For -, single argument is negation
                    let operand_reg = self.compile_expression(args[0])?;
                    let zero_reg = self.alloc_register();
                    let result_reg = self.alloc_register();

                    self.emit_load_immediate(zero_reg, ValueRef::number(0.0));
                    self.emit_u8(Opcode::Sub as u8);
                    self.emit_u8(result_reg);
                    self.emit_u8(zero_reg);
                    self.emit_u8(operand_reg);

                    return Ok(result_reg);
                }
                "/" => {
                    // For /, single argument is reciprocal
                    let operand_reg = self.compile_expression(args[0])?;
                    let one_reg = self.alloc_register();
                    let result_reg = self.alloc_register();

                    self.emit_load_immediate(one_reg, ValueRef::number(1.0));
                    self.emit_u8(Opcode::Div as u8);
                    self.emit_u8(result_reg);
                    self.emit_u8(one_reg);
                    self.emit_u8(operand_reg);

                    return Ok(result_reg);
                }
                _ => unreachable!(),
            }
        }

        // Handle multiple arguments by chaining binary operations
        let opcode = match symbol_name.as_str() {
            "+" => Opcode::Add,
            "-" => Opcode::Sub,
            "*" => Opcode::Mul,
            "/" => Opcode::Div,
            _ => unreachable!(),
        };

        // Compile first argument as initial accumulator
        let mut accumulator_reg = self.compile_expression(args[0])?;

        // Chain subsequent arguments
        for arg in &args[1..] {
            let arg_reg = self.compile_expression(*arg)?;
            let result_reg = self.alloc_register();

            self.emit_u8(opcode as u8);
            self.emit_u8(result_reg);
            self.emit_u8(accumulator_reg);
            self.emit_u8(arg_reg);

            accumulator_reg = result_reg;
        }

        Ok(accumulator_reg)
    }

    fn compile_regular_function_call(
        &mut self,
        symbol_id: u32,
        args: &[ValueRef],
    ) -> Result<u8, String> {
        let func_reg = self.alloc_register();

        // Load global function
        self.emit_u8(Opcode::LoadGlobal as u8);
        self.emit_u8(func_reg);
        self.emit_u32(symbol_id);

        // Compile arguments into consecutive registers
        let mut arg_registers = Vec::new();
        for arg in args {
            let arg_reg = self.compile_expression(*arg)?; // This puts result in register 0

            // Save the result before the next expression overwrites register 0
            let saved_reg = self.alloc_register();
            if arg_reg != saved_reg {
                self.emit_u8(Opcode::LoadLocal as u8);
                self.emit_u8(saved_reg);
                self.emit_u8(arg_reg); // Copy from register 0 to saved_reg
            }
            arg_registers.push(saved_reg); // Use the saved register, not arg_reg
        }

        // Move arguments to consecutive positions
        for (i, &arg_reg) in arg_registers.iter().enumerate() {
            let target_reg = func_reg + 1 + i as u8; // Right after func_reg
            if arg_reg != target_reg {
                self.emit_u8(Opcode::LoadLocal as u8);
                self.emit_u8(target_reg);
                self.emit_u8(arg_reg);
            }
        }

        let result_reg = self.alloc_register();

        // Emit call - arguments are now in consecutive registers starting at first_arg_reg
        self.emit_u8(Opcode::Call as u8);
        self.emit_u8(func_reg);
        self.emit_u8(args.len() as u8);
        self.emit_u8(result_reg);

        Ok(0) // ← Return register 0, where Call actually puts the result
    }

    fn is_special_form(&self, symbol_id: u32) -> bool {
        if let Some(symbol_name) = self.vm.symbol_table.read().get_symbol(symbol_id) {
            matches!(
                symbol_name.as_str(),
                "if" | "let"
                    | "do"
                    | "quote"
                    | "def"
                    | "fn"
                    | "loop"
                    | "recur"
                    | "cond"
                    | "macro"
                    | "quasiquote"
                    | "unquote"
                    | "unquote-splicing"
                    | "future"
                    | "complete"
                    | "go"
            )
        } else {
            false
        }
    }

    fn is_arithmetic_operator(&self, symbol_id: u32) -> bool {
        if let Some(symbol_name) = self.vm.symbol_table.read().get_symbol(symbol_id) {
            matches!(symbol_name.as_str(), "+" | "-" | "*" | "/")
        } else {
            false
        }
    }

    // MAIN COMPILATION ENTRY POINTS

    pub fn compile_for_storage(&mut self, expr: ValueRef) -> Result<CompiledFunction, String> {
        self.reset();

        let result_reg = self.compile_expression(expr)?;

        // Emit return
        self.emit_u8(Opcode::Return as u8);
        self.emit_u8(result_reg);

        Ok(CompiledFunction {
            bytecode: self.bytecode.clone(),
            constants: self.constants.clone(), // Make sure this is actually copying
            parameter_count: 0,
            register_count: self.next_register,
            module: 0,
            register_start: 0,
            has_self_reference: false,
        })
    }

    fn compile_apply(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() != 2 {
            return Err("apply expects 2 arguments".to_string());
        }

        let func_reg = self.compile_expression(args[0])?;
        let list_reg = self.compile_expression(args[1])?;

        // Check if it's a known arithmetic operator we can inline
        if let ValueRef::Immediate(packed) = args[0] {
            if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                let symbol = self.vm.symbol_table.read().get_symbol(symbol_id);
                if let Some(symbol_name) = symbol {
                    match symbol_name.as_str() {
                        "+" => return self.compile_inline_fold_add(list_reg),
                        "-" => return self.compile_inline_fold_sub(list_reg),
                        "*" => return self.compile_inline_fold_mul(list_reg),
                        "/" => return self.compile_inline_fold_div(list_reg),
                        _ => {}
                    }
                }
            }
        }

        // Fall back to general apply
        self.compile_general_apply(func_reg, list_reg)
    }

    fn get_fold_opcode(&self, op_name: &str) -> Option<Opcode> {
        match op_name {
            "+" => Some(Opcode::AddImm8),
            "*" => Some(Opcode::MulImm8),
            "-" => Some(Opcode::SubImm8),
            "/" => Some(Opcode::DivImm8),
            _ => None,
        }
    }

    fn compile_general_apply(&mut self, func_reg: u8, args_list_reg: u8) -> Result<u8, String> {
        let result_reg = self.alloc_register();
        let length_reg = self.alloc_register();
        let index_reg = self.alloc_register();
        let current_arg_reg = self.alloc_register();
        let accumulator_reg = self.alloc_register();
        let condition_reg = self.alloc_register();

        // Get the length of the argument list
        self.emit_u8(Opcode::GetLength as u8);
        self.emit_u8(length_reg);
        self.emit_u8(args_list_reg);

        // Initialize loop: index = 0
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(0); // index = 0

        // Initialize accumulator based on function type (we'll determine at runtime)
        // For now, start with nil - the runtime will handle initialization
        self.emit_load_immediate(accumulator_reg, ValueRef::nil());

        let loop_start_label = self.alloc_label();
        let loop_end_label = self.alloc_label();
        let first_iteration_label = self.alloc_label();

        // Check if list is empty
        self.emit_u8(Opcode::Eq as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(length_reg);
        self.emit_u8(index_reg); // Compare length with 0 (index starts at 0)
        self.emit_jump_if_true(condition_reg, loop_end_label);

        // Special handling for first element (different for each operator)
        self.emit_jump(first_iteration_label);

        // Loop start (for 2nd+ iterations)
        self.emit_label(loop_start_label);

        // Test: if index >= length, exit loop
        self.emit_u8(Opcode::Lt as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(index_reg);
        self.emit_u8(length_reg);
        self.emit_jump_if_false(condition_reg, loop_end_label);

        // Get current argument: args_list[index]
        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(current_arg_reg);
        self.emit_u8(args_list_reg);
        self.emit_u8(index_reg);

        // Prepare arguments for function call: [accumulator, current_arg]
        self.emit_u8(Opcode::PrepareArgs as u8);
        self.emit_u8(2); // arg count
        self.emit_u8(accumulator_reg); // first arg
        self.emit_u8(current_arg_reg); // second arg

        // Call function with accumulator and current argument
        self.emit_u8(Opcode::CallDynamic as u8);
        self.emit_u8(result_reg); // where to store result
        self.emit_u8(func_reg); // function to call
        self.emit_u8(2); // arg count

        // Move result back to accumulator for next iteration
        self.emit_u8(Opcode::LoadLocal as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(result_reg);

        // Increment index
        self.emit_u8(Opcode::LoopIncr as u8);
        self.emit_u8(index_reg);

        // Jump back to loop start
        self.emit_jump(loop_start_label);

        // First iteration handling
        self.emit_label(first_iteration_label);

        // Get first element
        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(args_list_reg);
        self.emit_u8(index_reg); // index is 0

        // Increment index for next iteration
        self.emit_u8(Opcode::LoopIncr as u8);
        self.emit_u8(index_reg);

        // Jump to main loop
        self.emit_jump(loop_start_label);

        // Loop end - result is in accumulator
        self.emit_label(loop_end_label);

        // Handle empty list case - call function with no args to get identity
        let identity_label = self.alloc_label();
        let final_label = self.alloc_label();

        // Check if we processed any elements (index > 0)
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(current_arg_reg);
        self.emit_u8(0);

        self.emit_u8(Opcode::Gt as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(index_reg);
        self.emit_u8(current_arg_reg);
        self.emit_jump_if_true(condition_reg, final_label);

        // Empty list case - get identity value by calling with no args
        self.emit_u8(Opcode::PrepareArgs as u8);
        self.emit_u8(0); // no args

        self.emit_u8(Opcode::CallDynamic as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(func_reg);
        self.emit_u8(0); // no args

        self.emit_label(final_label);

        // Copy accumulator to result register
        self.emit_u8(Opcode::LoadLocal as u8);
        self.emit_u8(result_reg);
        self.emit_u8(accumulator_reg);

        Ok(result_reg)
    }

    fn compile_inline_fold_add(&mut self, list_reg: u8) -> Result<u8, String> {
        let length_reg = self.alloc_register();
        let index_reg = self.alloc_register();
        let accumulator_reg = self.alloc_register();
        let current_reg = self.alloc_register();
        let condition_reg = self.alloc_register();

        // Get list length
        self.emit_u8(Opcode::GetLength as u8);
        self.emit_u8(length_reg);
        self.emit_u8(list_reg);

        // Initialize index = 0
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(0);

        // Initialize accumulator = 0 (identity for addition)
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(0);

        let loop_start_label = self.alloc_label();
        let loop_end_label = self.alloc_label();

        // Loop start
        self.emit_label(loop_start_label);

        // Check: index < length
        self.emit_u8(Opcode::Lt as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(index_reg);
        self.emit_u8(length_reg);
        self.emit_jump_if_false(condition_reg, loop_end_label);

        // Get current element: list[index]
        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(current_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // accumulator = accumulator + current
        self.emit_u8(Opcode::Add as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(accumulator_reg);
        self.emit_u8(current_reg);

        // index++
        self.emit_u8(Opcode::AddImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(index_reg);
        self.emit_u8(1);

        // Jump back to loop start
        self.emit_jump(loop_start_label);

        // Loop end
        self.emit_label(loop_end_label);

        Ok(accumulator_reg)
    }

    fn compile_inline_fold_sub(&mut self, list_reg: u8) -> Result<u8, String> {
        // Similar pattern but different for subtraction:
        // - First element becomes initial accumulator (no identity value)
        // - Start loop from index 1, not 0

        let length_reg = self.alloc_register();
        let index_reg = self.alloc_register();
        let accumulator_reg = self.alloc_register();
        let current_reg = self.alloc_register();
        let condition_reg = self.alloc_register();

        // Get list length
        self.emit_u8(Opcode::GetLength as u8);
        self.emit_u8(length_reg);
        self.emit_u8(list_reg);

        // Check for empty list
        let empty_label = self.alloc_label();
        let non_empty_label = self.alloc_label();

        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(0);

        self.emit_u8(Opcode::Eq as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(length_reg);
        self.emit_u8(condition_reg);
        self.emit_jump_if_true(condition_reg, empty_label);

        // Non-empty: get first element as initial accumulator
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(0);

        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // Start index from 1
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(1);

        let loop_start_label = self.alloc_label();
        let loop_end_label = self.alloc_label();

        // Loop for remaining elements
        self.emit_label(loop_start_label);

        self.emit_u8(Opcode::Lt as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(index_reg);
        self.emit_u8(length_reg);
        self.emit_jump_if_false(condition_reg, loop_end_label);

        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(current_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // accumulator = accumulator - current
        self.emit_u8(Opcode::Sub as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(accumulator_reg);
        self.emit_u8(current_reg);

        self.emit_u8(Opcode::AddImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(index_reg);
        self.emit_u8(1);

        self.emit_jump(loop_start_label);

        self.emit_label(loop_end_label);
        self.emit_jump(non_empty_label);

        // Empty list case - error for subtraction
        self.emit_label(empty_label);
        // You'd emit an error here or return some default

        self.emit_label(non_empty_label);
        Ok(accumulator_reg)
    }

    fn compile_inline_fold_mul(&mut self, list_reg: u8) -> Result<u8, String> {
        let length_reg = self.alloc_register();
        let index_reg = self.alloc_register();
        let accumulator_reg = self.alloc_register();
        let current_reg = self.alloc_register();
        let condition_reg = self.alloc_register();

        // Get list length
        self.emit_u8(Opcode::GetLength as u8);
        self.emit_u8(length_reg);
        self.emit_u8(list_reg);

        // Initialize index = 0
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(0);

        // Initialize accumulator = 1 (identity for multiplication)
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(1);

        let loop_start_label = self.alloc_label();
        let loop_end_label = self.alloc_label();

        // Loop start
        self.emit_label(loop_start_label);

        // Check: index < length
        self.emit_u8(Opcode::Lt as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(index_reg);
        self.emit_u8(length_reg);
        self.emit_jump_if_false(condition_reg, loop_end_label);

        // Get current element: list[index]
        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(current_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // accumulator = accumulator * current
        self.emit_u8(Opcode::Mul as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(accumulator_reg);
        self.emit_u8(current_reg);

        // index++
        self.emit_u8(Opcode::AddImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(index_reg);
        self.emit_u8(1);

        // Jump back to loop start
        self.emit_jump(loop_start_label);

        // Loop end
        self.emit_label(loop_end_label);

        Ok(accumulator_reg)
    }

    fn compile_inline_fold_div(&mut self, list_reg: u8) -> Result<u8, String> {
        // Division is left-associative like subtraction:
        // (/ a b c) = ((a / b) / c), NOT a / (b / c)
        // First element becomes initial accumulator, then divide by subsequent elements

        let length_reg = self.alloc_register();
        let index_reg = self.alloc_register();
        let accumulator_reg = self.alloc_register();
        let current_reg = self.alloc_register();
        let condition_reg = self.alloc_register();
        let zero_reg = self.alloc_register();

        // Get list length
        self.emit_u8(Opcode::GetLength as u8);
        self.emit_u8(length_reg);
        self.emit_u8(list_reg);

        // Load zero for comparisons
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(zero_reg);
        self.emit_u8(0);

        // Check for empty list - error for division
        let empty_error_label = self.alloc_label();
        let non_empty_label = self.alloc_label();
        let single_element_label = self.alloc_label();
        let multi_element_label = self.alloc_label();

        self.emit_u8(Opcode::Eq as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(length_reg);
        self.emit_u8(zero_reg);
        self.emit_jump_if_true(condition_reg, empty_error_label);

        // Check for single element: (/ x) = (/ 1 x) = 1/x (reciprocal)
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(1);

        self.emit_u8(Opcode::Eq as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(length_reg);
        self.emit_u8(condition_reg);
        self.emit_jump_if_true(condition_reg, single_element_label);

        // Multiple elements: first becomes accumulator, divide by rest
        self.emit_jump(multi_element_label);

        // Single element case: (/ x) = 1/x
        self.emit_label(single_element_label);

        // Get the single element
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(0);

        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(current_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // Check for division by zero
        self.emit_u8(Opcode::Eq as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(current_reg);
        self.emit_u8(zero_reg);
        self.emit_jump_if_true(condition_reg, empty_error_label); // Reuse error label

        // Calculate 1/x
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(1);

        self.emit_u8(Opcode::Div as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(accumulator_reg);
        self.emit_u8(current_reg);

        self.emit_jump(non_empty_label);

        // Multiple elements case: (/ a b c) = ((a / b) / c)
        self.emit_label(multi_element_label);

        // Get first element as initial accumulator
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(0);

        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // Start loop from index 1 (skip first element)
        self.emit_u8(Opcode::LoadImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(1);

        let loop_start_label = self.alloc_label();
        let loop_end_label = self.alloc_label();
        let division_check_label = self.alloc_label();

        // Loop for remaining elements
        self.emit_label(loop_start_label);

        // Check: index < length
        self.emit_u8(Opcode::Lt as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(index_reg);
        self.emit_u8(length_reg);
        self.emit_jump_if_false(condition_reg, loop_end_label);

        // Get current element
        self.emit_u8(Opcode::GetElement as u8);
        self.emit_u8(current_reg);
        self.emit_u8(list_reg);
        self.emit_u8(index_reg);

        // Check for division by zero
        self.emit_u8(Opcode::Eq as u8);
        self.emit_u8(condition_reg);
        self.emit_u8(current_reg);
        self.emit_u8(zero_reg);
        self.emit_jump_if_true(condition_reg, empty_error_label);

        // accumulator = accumulator / current
        self.emit_u8(Opcode::Div as u8);
        self.emit_u8(accumulator_reg);
        self.emit_u8(accumulator_reg);
        self.emit_u8(current_reg);

        // index++
        self.emit_u8(Opcode::AddImm8 as u8);
        self.emit_u8(index_reg);
        self.emit_u8(index_reg);
        self.emit_u8(1);

        // Jump back to loop start
        self.emit_jump(loop_start_label);

        self.emit_label(loop_end_label);
        self.emit_jump(non_empty_label);

        // Error case - empty list or division by zero
        self.emit_label(empty_error_label);
        // You could emit an error instruction here, or load NaN, or throw exception
        // For now, let's load a special error value
        self.emit_load_immediate(
            accumulator_reg,
            self.vm.error_value(BlinkError::eval("Division error")),
        );

        self.emit_label(non_empty_label);
        Ok(accumulator_reg)
    }

    // QUASIQUOTE and UNQUOTE support
    fn compile_quasiquote(&mut self, args: &[ValueRef]) -> Result<u8, String> {
        if args.len() != 1 {
            return Err("quasiquote expects exactly 1 argument".to_string());
        }

        if self.has_unquotes(args[0]) {
            let processed = self.process_quasiquote(args[0], 1)?;

            self.compile_expression(processed)
        } else {
            self.compile_quote(args)
        }
    }

    fn has_unquotes(&self, expr: ValueRef) -> bool {
        match expr {
            ValueRef::Immediate(_) => false,
            ValueRef::Native(_) => false,
            ValueRef::Heap(_) => {
                if let Some(list_items) = expr.get_list() {
                    self.list_has_unquotes(&list_items)
                } else if let Some(vec_items) = expr.get_vec() {
                    self.list_has_unquotes(&vec_items)
                } else {
                    false
                }
            }
        }
    }

    fn list_has_unquotes(&self, items: &[ValueRef]) -> bool {
        if items.is_empty() {
            return false;
        }

        // Check if this list IS an unquote/unquote-splicing
        if let ValueRef::Immediate(packed) = items[0] {
            if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                if let Some(symbol_name) = self.vm.symbol_table.read().get_symbol(symbol_id) {
                    if matches!(
                        symbol_name.as_str(),
                        "unquote" | "unquote-splicing" | "quasiquote"
                    ) {
                        return true;
                    }
                }
            }
        }

        // Check nested items recursively
        for item in items {
            if self.has_unquotes(*item) {
                return true;
            }
        }

        false
    }

    fn compile_unquote(&mut self, _args: &[ValueRef]) -> Result<u8, String> {
        Err("unquote used outside quasiquote".to_string())
    }

    fn compile_unquote_splicing(&mut self, _args: &[ValueRef]) -> Result<u8, String> {
        Err("unquote-splicing used outside quasiquote".to_string())
    }

    /// Process a quasiquoted expression, handling nested quasiquotes
    /// depth: current nesting level of quasiquotes (1 = top level)
    fn process_quasiquote(&mut self, expr: ValueRef, depth: i32) -> Result<ValueRef, String> {
        match expr {
            ValueRef::Immediate(packed) => {
                // For immediate values (symbols, numbers, etc), just return them quoted
                Ok(expr)
            }
            ValueRef::Heap(_) => {
                if let Some(list_items) = expr.get_list() {
                    self.process_quasiquote_list(&list_items, depth)
                } else if let Some(vec_items) = expr.get_vec() {
                    // Handle vectors specially
                    let processed_items = self.process_quasiquote_list(&vec_items, depth)?;
                    if let Some(processed_list) = processed_items.get_list() {
                        // Convert the list back to a vector
                        Ok(self.create_quoted_vector(processed_list))
                    } else {
                        Ok(processed_items)
                    }
                } else {
                    // For other heap values (maps, strings, etc), return as-is
                    Ok(expr)
                }
            }
            ValueRef::Native(_) => Ok(expr),
        }
    }

    fn process_quasiquote_list(
        &mut self,
        items: &[ValueRef],
        depth: i32,
    ) -> Result<ValueRef, String> {
        if items.is_empty() {
            // Empty list - just return a quoted empty list
            return Ok(self.create_quoted_list(vec![]));
        }

        // Check if first item is a special quasiquote form
        if let ValueRef::Immediate(packed) = items[0] {
            if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                let symbol_name = self.vm.symbol_table.read().get_symbol(symbol_id);
                match symbol_name.as_deref() {
                    Some("quasiquote") => {
                        // Nested quasiquote - increase depth
                        if items.len() != 2 {
                            return Err("quasiquote expects exactly 1 argument".to_string());
                        }
                        let processed = self.process_quasiquote(items[1], depth + 1)?;
                        return Ok(self.create_quoted_list(vec![items[0], processed]));
                    }
                    Some("unquote") => {
                        if depth == 1 {
                            // This is the unquote we should evaluate
                            if items.len() != 2 {
                                return Err("unquote expects exactly 1 argument".to_string());
                            }
                            return Ok(items[1]); // Return the unquoted expression for evaluation
                        } else {
                            // Nested unquote - decrease depth
                            let processed = self.process_quasiquote(items[1], depth - 1)?;
                            return Ok(self.create_quoted_list(vec![items[0], processed]));
                        }
                    }
                    Some("unquote-splicing") => {
                        return Err("unquote-splicing not allowed at top level of list".to_string());
                    }
                    _ => {
                        // Regular symbol, fall through to process normally
                    }
                }
            }
        }

        // Process all items, collecting them and checking for splicing
        let mut result_parts = Vec::new();

        for item in items {
            // Check for unquote-splicing and unquote
            if let Some(list_items) = item.get_list() {
                if list_items.len() == 2 {
                    if let ValueRef::Immediate(packed) = list_items[0] {
                        if let ImmediateValue::Symbol(symbol_id) = unpack_immediate(packed) {
                            let symbol_name = self.vm.symbol_table.read().get_symbol(symbol_id);
                            if symbol_name.as_deref() == Some("unquote-splicing") {
                                if depth == 1 {
                                    // This is actual splicing
                                    result_parts.push(SplicePart::Splice(list_items[1]));
                                    continue;
                                } else {
                                    // Nested unquote-splicing - decrease depth
                                    let processed =
                                        self.process_quasiquote(list_items[1], depth - 1)?;
                                    let new_item =
                                        self.create_quoted_list(vec![list_items[0], processed]);
                                    result_parts.push(SplicePart::Item(new_item));
                                    continue;
                                }
                            }
                            // ADD THIS: Handle regular unquote
                            if symbol_name.as_deref() == Some("unquote") {
                                if depth == 1 {
                                    // This is an unquote - mark as unquoted (no quoting!)
                                    result_parts.push(SplicePart::UnquotedItem(list_items[1]));
                                    continue;
                                } else {
                                    // Nested unquote - decrease depth
                                    let processed =
                                        self.process_quasiquote(list_items[1], depth - 1)?;
                                    let new_item =
                                        self.create_quoted_list(vec![list_items[0], processed]);
                                    result_parts.push(SplicePart::Item(new_item));
                                    continue;
                                }
                            }
                        }
                    }
                }
            }

            // Regular item - process recursively
            let processed = self.process_quasiquote(*item, depth)?;

            let final_item = processed;
            result_parts.push(SplicePart::Item(final_item));
        }
        self.build_spliced_list(result_parts)
    }

    fn create_quoted_list(&mut self, items: Vec<ValueRef>) -> ValueRef {
        // Create a direct list structure for macro expansion
        // This should create (if condition body nil) not (list 'if condition body 'nil)
        ValueRef::Heap(GcPtr::new(
            self.vm.alloc_list_from_items(items), // true = list
        ))
    }

    fn create_quoted_vector(&mut self, items: Vec<ValueRef>) -> ValueRef {
        // Create a simple quoted vector literal
        ValueRef::Heap(GcPtr::new(self.vm.alloc_list_from_items(items)))
    }

    fn build_spliced_list(&mut self, parts: Vec<SplicePart>) -> Result<ValueRef, String> {
        // Check if we actually need splicing
        let has_splice = parts.iter().any(|p| matches!(p, SplicePart::Splice(_)));

        if !has_splice {
            // No splicing - just build a regular quoted list
            let items: Vec<ValueRef> = parts
                .into_iter()
                .map(|p| match p {
                    SplicePart::Item(item) => item,
                    SplicePart::Splice(_) => unreachable!(),
                    SplicePart::UnquotedItem(item) => item,
                })
                .collect();
            return Ok(self.create_quoted_list(items));
        }

        // We have splicing - need to build a runtime expression
        // Transform into: (concat (list item1) spliced-expr (list item3) ...)

        let concat_symbol = self.vm.symbol_table.write().intern("concat");
        let concat_symbol_val = ValueRef::symbol(concat_symbol);
        let list_symbol = self.vm.symbol_table.write().intern("list");
        let list_symbol_val = ValueRef::symbol(list_symbol);

        let mut concat_args = vec![concat_symbol_val];
        let mut current_items = Vec::new();

        for part in parts {
            match part {
                SplicePart::Item(item) => {
                    current_items.push(item);
                }
                SplicePart::Splice(expr) => {
                    // Flush any accumulated items as a list
                    if !current_items.is_empty() {
                        let mut list_call = vec![list_symbol_val];
                        list_call.extend(current_items.drain(..));
                        concat_args.push(ValueRef::Heap(GcPtr::new(
                            self.vm.alloc_list_from_items(list_call),
                        )));
                    }
                    // Add the spliced expression directly
                    concat_args.push(expr);
                }
                SplicePart::UnquotedItem(item) => current_items.push(item),
            }
        }

        // Flush any remaining items
        if !current_items.is_empty() {
            let mut list_call = vec![list_symbol_val];
            list_call.extend(current_items);
            concat_args.push(ValueRef::Heap(GcPtr::new(
                self.vm.alloc_list_from_items(list_call),
            )));
        }

        // Return the concat expression
        Ok(ValueRef::Heap(GcPtr::new(
            self.vm.alloc_list_from_items(concat_args),
        )))
    }
}
