use std::{collections::HashMap, sync::Arc};

use crate::{runtime::{BlinkVM, Bytecode, CompiledFunction, LabelPatch, Opcode}, value::unpack_immediate, ImmediateValue, ValueRef};

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
        let reg = self.next_register;
        self.next_register += 1;
        if reg > 255 {
            panic!("Too many registers in function (max 256)");
        }
        reg
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
        
        if args.len() != 2 {
            return Err("Arithmetic operations require exactly 2 arguments".to_string());
        }
        
        let left_reg = self.compile_expression(args[0])?;
        let right_reg = self.compile_expression(args[1])?;
        let result_reg = self.alloc_register();
        
        let opcode = match symbol_name.as_str() {
            "+" => Opcode::Add,
            "-" => Opcode::Sub,
            "*" => Opcode::Mul,
            "/" => Opcode::Div,
            _ => unreachable!(),
        };
        
        self.emit_u8(opcode as u8);
        self.emit_u8(result_reg);
        self.emit_u8(left_reg);
        self.emit_u8(right_reg);
        
        Ok(result_reg)
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
}