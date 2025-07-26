use std::sync::Arc;
use mmtk::util::ObjectReference;

use crate::{compiler::BytecodeCompiler, runtime::{BlinkVM, CompiledFunction, Opcode}, value::unpack_immediate, value::ImmediateValue, ValueRef};

// Updated call frame for byte-sized bytecode
#[derive(Clone, Debug)]
pub struct CallFrame {
    pub func: FunctionRef,
    pub pc: usize,              // Byte offset into bytecode, not instruction index
    pub reg_start: usize,
    pub reg_count: u8,
    pub current_module: u32,
}

#[derive(Clone, Debug)]
pub enum FunctionRef {
    CompiledFunction(ObjectReference),
    Native(usize),
    CompiledMacro(ObjectReference),
}

#[derive(Clone, Debug)]
pub struct ExecutionContext {
    pub vm: Arc<BlinkVM>,
    current_module: u32,
    register_stack: Vec<ValueRef>,
    call_stack: Vec<CallFrame>,
}

impl ExecutionContext {
    pub fn new(vm: Arc<BlinkVM>) -> Self {
        Self {
            vm,
            current_module: 0,
            register_stack: Vec::new(),
            call_stack: Vec::new(),
        }
    }

    pub fn get_stack_roots(&self) -> Vec<ObjectReference> {
        let mut roots = Vec::new();
        for frame in self.call_stack.iter() {
            match frame.func {
                FunctionRef::CompiledFunction(func) => {
                    roots.push(func);
                },
                FunctionRef::Native(_func) => {
                    // no op
                },
                FunctionRef::CompiledMacro(func) => {
                    roots.push(func);
                },
                
            }
        }
        for reg in self.register_stack.iter() {
            match reg {
                ValueRef::Heap(heap) => {
                    roots.push(heap.0);
                },
                _ => {}
            }
        }
        roots
    }
    
    /// Compile and execute expression immediately (for REPL)
    pub fn compile_and_execute(&mut self, expr: ValueRef) -> Result<ValueRef, String> {
        let mut compiler = BytecodeCompiler::new(self.vm.clone());
        let compiled = compiler.compile_for_storage(expr)?;
        self.execute_compiled_function(&compiled, &[])
    }
    
    /// Execute compiled function with arguments
    fn execute_compiled_function(&mut self, compiled_fn: &CompiledFunction, args: &[ValueRef]) -> Result<ValueRef, String> {
        if args.len() != compiled_fn.parameter_count as usize {
            return Err(format!("Function expects {} arguments, got {}", compiled_fn.parameter_count, args.len()));
        }
        
        let reg_start = self.register_stack.len();
        let total_registers = reg_start + compiled_fn.register_count as usize;
        self.register_stack.resize(total_registers, ValueRef::nil());
        
        // Place arguments in parameter registers
        for (i, &arg) in args.iter().enumerate() {
            self.register_stack[reg_start + i] = arg;
        }
        
        // Execute bytecode
        let result = self.execute_bytecode(&compiled_fn.bytecode, &compiled_fn.constants, reg_start);
        
        // Clean up registers
        self.register_stack.truncate(reg_start);
        
        result
    }
    
    /// Execute raw bytecode with register base offset
    fn execute_bytecode(&mut self, bytecode: &[u8], constants: &[ValueRef], reg_base: usize) -> Result<ValueRef, String> {
        let mut pc = 0;
        
        while pc < bytecode.len() {
            let opcode = Opcode::from_u8(bytecode[pc])?;
            pc += 1;
            
            match opcode {
                Opcode::LoadImm8 => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    let value = self.read_u8(bytecode, &mut pc)?;
                    self.register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                }
                
                Opcode::LoadImm16 => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    let value = self.read_u16(bytecode, &mut pc)?;
                    self.register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                }
                
                Opcode::LoadImm32 => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    let value = self.read_u32(bytecode, &mut pc)?;
                    self.register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                }
                
                Opcode::LoadImmConst => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    let const_idx = self.read_u8(bytecode, &mut pc)?;
                    if (const_idx as usize) < constants.len() {
                        self.register_stack[reg_base + reg as usize] = constants[const_idx as usize];
                    } else {
                        return Err(format!("Constant index {} out of bounds", const_idx));
                    }
                }
                
                Opcode::LoadLocal => {
                    let dest_reg = self.read_u8(bytecode, &mut pc)?;
                    let src_reg = self.read_u8(bytecode, &mut pc)?;
                    let value = self.register_stack[reg_base + src_reg as usize];
                    self.register_stack[reg_base + dest_reg as usize] = value;
                }
                
                Opcode::LoadGlobal => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    let symbol_id = self.read_u32(bytecode, &mut pc)?;
                    let module_id = self.call_stack[self.call_stack.len() - 1].current_module;
                    match self.vm.resolve_global_symbol(module_id, symbol_id) {
                        Some(value) => {
                            self.register_stack[reg_base + reg as usize] = value;
                        }
                        None => {
                            return Err(format!("Global symbol {} not found", symbol_id));
                        }
                    }
                }
                
                Opcode::StoreGlobal => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    let symbol_id = self.read_u32(bytecode, &mut pc)?;
                    let value = self.register_stack[reg_base + reg as usize];
                    let module_id = self.call_stack[self.call_stack.len() - 1].current_module;
                    self.vm.update_module(module_id, symbol_id, value);
                }
                
                Opcode::Add => {
                    let result_reg = self.read_u8(bytecode, &mut pc)?;
                    let left_reg = self.read_u8(bytecode, &mut pc)?;
                    let right_reg = self.read_u8(bytecode, &mut pc)?;
                    
                    let left = self.register_stack[reg_base + left_reg as usize];
                    let right = self.register_stack[reg_base + right_reg as usize];
                    
                    match self.perform_addition(left, right) {
                        Ok(result) => {
                            self.register_stack[reg_base + result_reg as usize] = result;
                        }
                        Err(e) => return Err(e),
                    }
                }
                
                Opcode::Sub => {
                    let result_reg = self.read_u8(bytecode, &mut pc)?;
                    let left_reg = self.read_u8(bytecode, &mut pc)?;
                    let right_reg = self.read_u8(bytecode, &mut pc)?;
                    
                    let left = self.register_stack[reg_base + left_reg as usize];
                    let right = self.register_stack[reg_base + right_reg as usize];
                    
                    match self.perform_subtraction(left, right) {
                        Ok(result) => {
                            self.register_stack[reg_base + result_reg as usize] = result;
                        }
                        Err(e) => return Err(e),
                    }
                }
                
                Opcode::Mul => {
                    let result_reg = self.read_u8(bytecode, &mut pc)?;
                    let left_reg = self.read_u8(bytecode, &mut pc)?;
                    let right_reg = self.read_u8(bytecode, &mut pc)?;
                    
                    let left = self.register_stack[reg_base + left_reg as usize];
                    let right = self.register_stack[reg_base + right_reg as usize];
                    
                    match self.perform_multiplication(left, right) {
                        Ok(result) => {
                            self.register_stack[reg_base + result_reg as usize] = result;
                        }
                        Err(e) => return Err(e),
                    }
                }
                
                Opcode::Div => {
                    let result_reg = self.read_u8(bytecode, &mut pc)?;
                    let left_reg = self.read_u8(bytecode, &mut pc)?;
                    let right_reg = self.read_u8(bytecode, &mut pc)?;
                    
                    let left = self.register_stack[reg_base + left_reg as usize];
                    let right = self.register_stack[reg_base + right_reg as usize];
                    
                    match self.perform_division(left, right) {
                        Ok(result) => {
                            self.register_stack[reg_base + result_reg as usize] = result;
                        }
                        Err(e) => return Err(e),
                    }
                }
                
                Opcode::Jump => {
                    let offset = self.read_i16(bytecode, &mut pc)?;
                    pc = (pc as i32 + offset as i32) as usize;
                }
                
                Opcode::JumpIfTrue => {
                    let test_reg = self.read_u8(bytecode, &mut pc)?;
                    let offset = self.read_i16(bytecode, &mut pc)?;
                    let test_value = self.register_stack[reg_base + test_reg as usize];
                    if test_value.is_truthy() {
                        pc = (pc as i32 + offset as i32) as usize;
                    }
                }
                
                Opcode::JumpIfFalse => {
                    let test_reg = self.read_u8(bytecode, &mut pc)?;
                    let offset = self.read_i16(bytecode, &mut pc)?;
                    let test_value = self.register_stack[reg_base + test_reg as usize];
                    if !test_value.is_truthy() {
                        pc = (pc as i32 + offset as i32) as usize;
                    }
                }
                
                Opcode::Call => {
                    let func_reg = self.read_u8(bytecode, &mut pc)?;
                    let arg_count = self.read_u8(bytecode, &mut pc)?;
                    let result_reg = self.read_u8(bytecode, &mut pc)?;
                    
                    // For now, return error - function calls need frame management
                    return Err("Function calls not implemented in simple executor".to_string());
                }
                
                Opcode::Return => {
                    let reg = self.read_u8(bytecode, &mut pc)?;
                    return Ok(self.register_stack[reg_base + reg as usize]);
                }
                
                Opcode::ReturnNil => {
                    return Ok(ValueRef::nil());
                }
                
                _ => {
                    return Err(format!("Unimplemented opcode: {:?}", opcode));
                }
            }
        }
        
        Ok(ValueRef::nil())
    }
    
    // BYTECODE READING HELPERS
    
    fn read_u8(&self, bytecode: &[u8], pc: &mut usize) -> Result<u8, String> {
        if *pc >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let value = bytecode[*pc];
        *pc += 1;
        Ok(value)
    }
    
    fn read_u16(&self, bytecode: &[u8], pc: &mut usize) -> Result<u16, String> {
        if *pc + 1 >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let bytes = [bytecode[*pc], bytecode[*pc + 1]];
        *pc += 2;
        Ok(u16::from_le_bytes(bytes))
    }
    
    fn read_u32(&self, bytecode: &[u8], pc: &mut usize) -> Result<u32, String> {
        if *pc + 3 >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let bytes = [bytecode[*pc], bytecode[*pc + 1], bytecode[*pc + 2], bytecode[*pc + 3]];
        *pc += 4;
        Ok(u32::from_le_bytes(bytes))
    }
    
    fn read_i16(&self, bytecode: &[u8], pc: &mut usize) -> Result<i16, String> {
        if *pc + 1 >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let bytes = [bytecode[*pc], bytecode[*pc + 1]];
        *pc += 2;
        Ok(i16::from_le_bytes(bytes))
    }
    
    // ARITHMETIC HELPERS (same as before)
    
    fn perform_addition(&self, left: ValueRef, right: ValueRef) -> Result<ValueRef, String> {
        // Same implementation as before
        let left_num = self.extract_number(left)?;
        let right_num = self.extract_number(right)?;
        Ok(ValueRef::number(left_num + right_num))
    }
    
    fn perform_subtraction(&self, left: ValueRef, right: ValueRef) -> Result<ValueRef, String> {
        let left_num = self.extract_number(left)?;
        let right_num = self.extract_number(right)?;
        Ok(ValueRef::number(left_num - right_num))
    }
    
    fn perform_multiplication(&self, left: ValueRef, right: ValueRef) -> Result<ValueRef, String> {
        let left_num = self.extract_number(left)?;
        let right_num = self.extract_number(right)?;
        Ok(ValueRef::number(left_num * right_num))
    }
    
    fn perform_division(&self, left: ValueRef, right: ValueRef) -> Result<ValueRef, String> {
        let left_num = self.extract_number(left)?;
        let right_num = self.extract_number(right)?;
        
        if right_num == 0.0 {
            return Err("Division by zero".to_string());
        }
        
        Ok(ValueRef::number(left_num / right_num))
    }
    
    fn extract_number(&self, value: ValueRef) -> Result<f64, String> {
        match value {
            ValueRef::Immediate(packed) => {
                if let ImmediateValue::Number(n) = unpack_immediate(packed) {
                    Ok(n)
                } else {
                    Err("Value is not a number".to_string())
                }
            }
            _ => Err("Value is not a number".to_string()),
        }
    }
}


// Bytecode disassembler for debugging
pub fn disassemble_bytecode(bytecode: &[u8], constants: &[ValueRef]) -> String {
    let mut result = String::new();
    let mut pc = 0;
    
    while pc < bytecode.len() {
        let start_pc = pc;
        
        if let Ok(opcode) = Opcode::from_u8(bytecode[pc]) {
            pc += 1;
            
            result.push_str(&format!("{:04x}: {:?}", start_pc, opcode));
            
            match opcode {
                Opcode::LoadImm8 => {
                    if pc + 1 < bytecode.len() {
                        let reg = bytecode[pc];
                        let value = bytecode[pc + 1];
                        result.push_str(&format!(" r{}, {}", reg, value));
                        pc += 2;
                    }
                }
                
                Opcode::LoadImmConst => {
                    if pc + 1 < bytecode.len() {
                        let reg = bytecode[pc];
                        let const_idx = bytecode[pc + 1];
                        if (const_idx as usize) < constants.len() {
                            result.push_str(&format!(" r{}, const[{}] = {:?}", 
                                                   reg, const_idx, constants[const_idx as usize]));
                        }
                        pc += 2;
                    }
                }
                
                Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div => {
                    if pc + 2 < bytecode.len() {
                        let result_reg = bytecode[pc];
                        let left_reg = bytecode[pc + 1];
                        let right_reg = bytecode[pc + 2];
                        result.push_str(&format!(" r{}, r{}, r{}", result_reg, left_reg, right_reg));
                        pc += 3;
                    }
                }
                
                Opcode::Return => {
                    if pc < bytecode.len() {
                        let reg = bytecode[pc];
                        result.push_str(&format!(" r{}", reg));
                        pc += 1;
                    }
                }
                
                Opcode::Jump => {
                    if pc + 1 < bytecode.len() {
                        let offset = i16::from_le_bytes([bytecode[pc], bytecode[pc + 1]]);
                        let target = (pc as i32 + 2 + offset as i32) as usize;
                        result.push_str(&format!(" {:04x} (offset {})", target, offset));
                        pc += 2;
                    }
                }
                
                _ => {
                    // Add other instruction formats as needed
                }
            }
        } else {
            result.push_str(&format!("{:04x}: INVALID 0x{:02x}", start_pc, bytecode[pc]));
            pc += 1;
        }
        
        result.push('\n');
    }
    
    result
}

// Testing helper
pub fn test_compact_bytecode() -> Result<(), String> {
    // This would test the compilation and execution pipeline:
    
    // 1. Test simple literal
    // let vm = Arc::new(BlinkVM::new());
    // let mut exec_ctx = ExecutionContext::new(vm.clone());
    // let result = exec_ctx.compile_and_execute(ValueRef::number(42.0))?;
    // assert_eq!(result, ValueRef::number(42.0));
    
    // 2. Test arithmetic
    // let plus_expr = parse_expression("(+ 1 2)")?;
    // let result = exec_ctx.compile_and_execute(plus_expr)?;
    // assert_eq!(result, ValueRef::number(3.0));
    
    // 3. Test string
    // let string_expr = ValueRef::string("hello");
    // let result = exec_ctx.compile_and_execute(string_expr)?;
    // assert_eq!(result, ValueRef::string("hello"));
    
    Ok(())
}