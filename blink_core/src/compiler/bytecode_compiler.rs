use std::{collections::HashMap, sync::Arc};

use crate::{error::BlinkError, runtime::{BlinkVM, Bytecode, CompiledFunction, LabelPatch, Opcode}, value::unpack_immediate, ImmediateValue, ValueRef};

// The main bytecode compiler
pub struct BytecodeCompiler {
    vm: Arc<BlinkVM>,
    bytecode: Bytecode,
    constants: Vec<ValueRef>,
    next_register: u8,
    scope_stack: Vec<HashMap<u32, u8>>, // symbol_id -> register mapping
    next_label_id: u16,
    label_patches: Vec<LabelPatch>,
}

impl BytecodeCompiler {
    pub fn new(vm: Arc<BlinkVM>) -> Self {
        Self {
            vm,
            bytecode: Vec::new(),
            constants: Vec::new(),
            next_register: 0,
            scope_stack: vec![HashMap::new()], // Global scope
            next_label_id: 0,
            label_patches: Vec::new(),
        }
    }
    
    fn reset(&mut self) {
        self.bytecode.clear();
        self.constants.clear();
        self.next_register = 0;
        self.scope_stack.clear();
        self.scope_stack.push(HashMap::new());
        self.next_label_id = 0;
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
        let label = self.next_label_id;
        self.next_label_id += 1;
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
        self.label_patches.push(LabelPatch {
            bytecode_offset: patch_offset,
            label_id: label,
        });
        self.emit_i16(0); // Placeholder
    }
    
    fn emit_label(&mut self, label: u16) {
        let current_pos = self.bytecode.len() as i16;
        
        // Patch all jumps to this label
        for patch in &self.label_patches {
            if patch.label_id == label {
                let jump_pos = patch.bytecode_offset;
                let offset = current_pos - jump_pos as i16 - 2;
                
                // Write the offset back into bytecode
                let offset_bytes = offset.to_le_bytes();
                self.bytecode[jump_pos] = offset_bytes[0];
                self.bytecode[jump_pos + 1] = offset_bytes[1];
            }
        }
        
        // Remove processed patches
        self.label_patches.retain(|patch| patch.label_id != label);
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
    
    fn resolve_local_symbol(&self, symbol_id: u32) -> Option<u8> {
        for scope in self.scope_stack.iter().rev() {
            if let Some(&register) = scope.get(&symbol_id) {
                return Some(register);
            }
        }
        None
    }
    
    // MAIN COMPILATION METHODS
    
    fn compile_expression(&mut self, expr: ValueRef) -> Result<u8, String> {
        match expr {
            ValueRef::Immediate(_) => {
                let reg = self.alloc_register();
                self.emit_load_immediate(reg, expr);
                Ok(reg)
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
    
    fn compile_function_call(&mut self, items: &[ValueRef]) -> Result<u8, String> {
        if items.is_empty() {
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
                
                // Regular function call
                return self.compile_regular_function_call(symbol_id, &items[1..]);
            }
        }
        
        Err("Unsupported function call".to_string())
    }
    
    fn compile_special_form(&mut self, symbol_id: u32, args: &[ValueRef]) -> Result<u8, String> {
        let symbol_name = self.vm.symbol_table.read().get_symbol(symbol_id)
            .ok_or("Unknown symbol")?;
            
        match symbol_name.as_str() {
            "if" => self.compile_if(args),
            "let" => self.compile_let(args),
            "do" => self.compile_do(args),
            "quote" => self.compile_quote(args),
            _ => Err(format!("Special form '{}' not implemented", symbol_name)),
        }
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
        let symbol_name = self.vm.symbol_table.read().get_symbol(symbol_id)
            .ok_or("Unknown symbol")?;
            
        if !matches!(symbol_name.as_str(), "+" | "-" | "*" | "/") {
            return Err("Not an arithmetic operator".to_string());
        }
        
        // Handle zero arguments
        if args.is_empty() {
            let result_reg = self.alloc_register();
            let identity_value = match symbol_name.as_str() {
                "+" => ValueRef::number(0.0),  // Identity for addition
                "*" => ValueRef::number(1.0),  // Identity for multiplication
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
    
    fn compile_regular_function_call(&mut self, symbol_id: u32, args: &[ValueRef]) -> Result<u8, String> {
        let func_reg = self.alloc_register();
        
        // Load global function
        self.emit_u8(Opcode::LoadGlobal as u8);
        self.emit_u8(func_reg);
        self.emit_u32(symbol_id);
        
        // Compile arguments
        for arg in args {
            self.compile_expression(*arg)?;
        }
        
        let result_reg = self.alloc_register();
        
        // Emit call
        self.emit_u8(Opcode::Call as u8);
        self.emit_u8(func_reg);
        self.emit_u8(args.len() as u8);
        self.emit_u8(result_reg);
        
        Ok(result_reg)
    }
    
    fn is_special_form(&self, symbol_id: u32) -> bool {
        if let Some(symbol_name) = self.vm.symbol_table.read().get_symbol(symbol_id) {
            matches!(symbol_name.as_str(), "if" | "let" | "do" | "quote" | "def" | "fn")
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
            constants: self.constants.clone(),
            parameter_count: 0,
            register_count: self.next_register,
            module: 0,
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
                        "+"  => return self.compile_inline_fold_add(list_reg),
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
        self.emit_u8(result_reg);      // where to store result
        self.emit_u8(func_reg);        // function to call
        self.emit_u8(2);               // arg count
        
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
        self.emit_load_immediate(accumulator_reg, self.vm.error_value(BlinkError::eval("Division error")));
        
        self.emit_label(non_empty_label);
        Ok(accumulator_reg)
    }

}