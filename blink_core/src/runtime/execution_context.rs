use std::sync::Arc;
use mmtk::util::ObjectReference;

use crate::{compiler::BytecodeCompiler, error::BlinkError, runtime::{BlinkVM, ClosureObject, CompiledFunction, Opcode, TypeTag}, value::{unpack_immediate, GcPtr, ImmediateValue}, ValueRef};

// Updated call frame for byte-sized bytecode
#[derive(Clone, Debug)]
pub struct CallFrame {
    pub func: FunctionRef,
    pub pc: usize,              // Byte offset into bytecode, not instruction index
    pub reg_start: usize,
    pub reg_count: u8,
    pub current_module: u32,
}

#[derive(Debug)]

enum InstructionResult {
    Continue,
    Return,
    Call(CallFrame),
    SetupSelfReference(u8),
    CreateClosure {                    // Renamed from CreateClosureWithUpvalues
        dest_register: u8,
        template_register: u8,
        captures: Vec<(u8, u32)>,      // Upvalue capture info included
    },
    LoadUpvalue {
        dest_register: u8,
        upvalue_index: u8,
    },
    StoreUpvalue {
        upvalue_index: u8,
        src_register: u8,
    },
}

#[derive(Clone, Debug)]
pub enum FunctionRef {
    Closure(ClosureObject, Option<ObjectReference>),
    CompiledFunction(CompiledFunction, Option<ObjectReference>),
    Native(usize),
}

#[derive(Clone, Debug)]
struct PendingUpvalue {
    index: u8,
    value: ValueRef,
    symbol_id: u32,
}


#[derive(Clone, Debug)]
pub struct ExecutionContext {
    pub vm: Arc<BlinkVM>,
    current_module: u32,
    register_stack: Vec<ValueRef>,
    call_stack: Vec<CallFrame>,
}

impl ExecutionContext {
    pub fn new(vm: Arc<BlinkVM>, current_module: u32) -> Self {
        Self {
            vm,
            current_module,
            register_stack: Vec::new(),
            call_stack: Vec::new(),
        }
    }

    pub fn get_stack_roots(&self) -> Vec<ObjectReference> {
        let mut roots = Vec::new();
        for frame in self.call_stack.iter() {
            match &frame.func {
                FunctionRef::Closure(closure_object, obj_ref) => {
                    if let Some(obj_ref) = obj_ref {
                        roots.push(*obj_ref);
                    }
                },
                FunctionRef::CompiledFunction(_func, obj_ref) => {
                    if let Some(obj_ref) = obj_ref {
                        roots.push(*obj_ref);
                    }
                },
                FunctionRef::Native(_func) => {
                    // no op
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
    
    pub fn compile_and_execute(&mut self, expr: ValueRef) -> Result<ValueRef, BlinkError> {
        let mut compiler = BytecodeCompiler::new(self.vm.clone(), self.current_module);
        let compiled = compiler.compile_for_storage(expr).map_err(|e| BlinkError::eval(e))?;
        
        let reg_count = compiled.register_count;
        // Setup initial frame
        let initial_frame = CallFrame {
            func: FunctionRef::CompiledFunction(compiled, None), // No GC object for REPL
            pc: 0,
            reg_start: self.register_stack.len(),
            reg_count: reg_count,
            current_module: self.current_module,
        };
        
        // Allocate registers for the expression
        for _ in 0..reg_count {
            self.register_stack.push(ValueRef::nil());
        }
        
        self.call_stack.push(initial_frame);
        
        // Execute frame loop
        let res = self.execute().map_err(|e| BlinkError::eval(e));
        println!("Before function call - register_stack.len(): {}", self.register_stack.len());
        res
    }
    
    // Main execution loop - processes all frames until stack is empty
    pub fn execute(&mut self) -> Result<ValueRef, String> {
        while !self.call_stack.is_empty() {
            println!("Before function call - register_stack.len(): {}", self.register_stack.len());
            // Get current frame (don't pop yet)
            let mut current_frame = if let Some(frame) = self.call_stack.last().cloned() {
                frame
            } else {
                break;
            };
            
            if let FunctionRef::CompiledFunction(compiled_fn, obj_ref) = &current_frame.func {
                // Check for end of function
                if current_frame.pc >= compiled_fn.bytecode.len() {
                    // Function completed naturally
                    let completed_frame = self.call_stack.pop().unwrap();
                    let return_value = self.register_stack[completed_frame.reg_start];
                    
                    // Clean up registers
                    self.register_stack.truncate(completed_frame.reg_start);
                    
                    if self.call_stack.is_empty() {
                        return Ok(return_value);
                    }
                    
                    // Store return value in caller's register 0
                    if let Some(caller_frame) = self.call_stack.last() {
                        self.register_stack[caller_frame.reg_start] = return_value;
                    }
                    continue;
                }
                
                let opcode = Opcode::from_u8(compiled_fn.bytecode[current_frame.pc])?;
                current_frame.pc += 1;

                
                
                let instruction_result = Self::execute_instruction(
                    &mut self.register_stack, 
                    &self.vm.as_ref(), 
                    current_frame.current_module, 
                    opcode, 
                    &compiled_fn.bytecode, 
                    &compiled_fn.constants, 
                    current_frame.reg_start, 
                    &mut current_frame.pc
                );

                // In the main execution loop, when an error occurs:
                if let Err(error) = instruction_result {
                    // Clean up incomplete function calls
                    while !self.call_stack.is_empty() {
                        let incomplete_frame = self.call_stack.pop().unwrap();
                        self.register_stack.truncate(incomplete_frame.reg_start);
                    }
                    return Err(error);
                };

                let instruction_result = instruction_result?;

                
                match instruction_result {
                    InstructionResult::Continue => {
                        // Update the frame in the stack
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
                    InstructionResult::Return => {
                        // Get return value from register 0 of completed frame
                        let completed_frame = self.call_stack.pop().unwrap();
                        let return_value = self.register_stack[completed_frame.reg_start];
                        
                        // Clean up registers used by completed frame
                        self.register_stack.truncate(completed_frame.reg_start);
                        
                        // If no more frames, we're done
                        if self.call_stack.is_empty() {
                            return Ok(return_value);
                        }
                        
                        // Store return value in caller's register 0
                        if let Some(caller_frame) = self.call_stack.last() {
                            self.register_stack[caller_frame.reg_start] = return_value;
                        }
                    }
                    InstructionResult::Call(new_frame) => {
                        // Update current frame PC, then push new frame
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                        self.call_stack.push(new_frame);
                    }
                    InstructionResult::SetupSelfReference(reg) => {
                        // Handle self-reference setup here where we have access to function context
                        if let Some(obj_ref) = obj_ref {
                            let function_value = ValueRef::Heap(GcPtr::new(*obj_ref));
                            self.register_stack[current_frame.reg_start + reg as usize] = function_value;
                        } else {
                            return Err("SetupSelfReference: no function object available".to_string());
                        }
                        
                        // Update frame PC and continue
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
                   

                    InstructionResult::SetupSelfReference(self_ref_reg) => {
                        if let Some(obj_ref) = obj_ref {
                            let function_value = ValueRef::Heap(GcPtr::new(*obj_ref));
                            self.register_stack[current_frame.reg_start + self_ref_reg as usize] = function_value;
                        } else {
                            return Err("SetupSelfReference: no function object available".to_string());
                        }
                        
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
                    
                
                    
                    InstructionResult::LoadUpvalue { dest_register, upvalue_index } => {
                        if let FunctionRef::Closure(_, Some(obj_ref)) = &current_frame.func {  // Changed here
                            let closure = GcPtr(*obj_ref).read_closure();
                            if let Some(upvalue) = closure.upvalues.get(upvalue_index as usize) {
                                self.register_stack[current_frame.reg_start + dest_register as usize] = *upvalue;
                            } else {
                                return Err(format!("Upvalue index {} out of bounds", upvalue_index));
                            }
                        } else {
                            return Err("LoadUpvalue called on non-closure function".to_string());
                        }
                        
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
                    
                    InstructionResult::StoreUpvalue { upvalue_index, src_register } => {
                        let value = self.register_stack[current_frame.reg_start + src_register as usize];
                        
                        if let FunctionRef::Closure(_, Some(obj_ref)) = &current_frame.func {  // Changed here
                            GcPtr(*obj_ref).set_upvalue(upvalue_index as usize, value)?;
                        } else {
                            return Err("StoreUpvalue called on non-closure function".to_string());
                        }
                        
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
                    
                    
                    InstructionResult::CreateClosure { dest_register, template_register, captures } => {
                        // Get template
                        let template_value = self.register_stack[current_frame.reg_start + template_register as usize];
                        let template_obj_ref = if let ValueRef::Heap(heap_ptr) = template_value {
                            heap_ptr.0
                        } else {
                            return Err("Template must be a heap object".to_string());
                        };
                        
                        // Capture upvalues directly from registers
                        let mut upvalues = Vec::new();
                        for (parent_reg, _symbol_id) in captures {
                            let captured_value = self.register_stack[current_frame.reg_start + parent_reg as usize];
                            upvalues.push(captured_value);
                        }
                        
                        // Create closure
                        let closure_obj = ClosureObject {
                            template: template_obj_ref,
                            upvalues,
                        };
                        
                        let closure_ref = self.vm.alloc_closure(closure_obj);
                        self.register_stack[current_frame.reg_start + dest_register as usize] = 
                            ValueRef::Heap(GcPtr::new(closure_ref));
                        
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
                }
                
            } else {
                // TODO: Implement native function calls
                todo!()
            }
        }
        
        Ok(ValueRef::nil())
    }
    
    fn execute_instruction(
        register_stack: &mut Vec<ValueRef>,
        vm: &BlinkVM,
        current_module: u32,
        opcode: Opcode,
        bytecode: &[u8],
        constants: &[ValueRef],
        reg_base: usize,
        pc: &mut usize
    ) -> Result<InstructionResult, String> {
        match opcode {
            Opcode::LoadImm8 => {
                let reg = Self::read_u8(bytecode, pc)?;
                let value = Self::read_u8(bytecode, pc)?;
                register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                Ok(InstructionResult::Continue)
            }
            
            Opcode::LoadImm16 => {
                let reg = Self::read_u8(bytecode, pc)?;
                let value = Self::read_u16(bytecode, pc)?;
                register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                Ok(InstructionResult::Continue)
            }
            
            Opcode::LoadImm32 => {
                let reg = Self::read_u8(bytecode, pc)?;
                let value = Self::read_u32(bytecode, pc)?;
                register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                Ok(InstructionResult::Continue)
            }
            
            Opcode::LoadImmConst => {
                let reg = Self::read_u8(bytecode, pc)?;
                let const_idx = Self::read_u8(bytecode, pc)?;
                if (const_idx as usize) < constants.len() {
                    register_stack[reg_base + reg as usize] = constants[const_idx as usize];
                } else {
                    return Err(format!("Constant index {} out of bounds", const_idx));
                }
                Ok(InstructionResult::Continue)
            }
            
            Opcode::LoadLocal => {

                let dest_reg = Self::read_u8(bytecode, pc)?;
                let src_reg = Self::read_u8(bytecode, pc)?;
                let value = register_stack[reg_base + src_reg as usize];
                if let ValueRef::Immediate(packed) = value {
                    let imm = unpack_immediate(packed);
                    println!("LoadLocal: immediate value: {}", imm);
                } else if let ValueRef::Heap(heap) = value {
                    let type_tag = heap.type_tag();
                    println!("LoadLocal: {:?} heap value: {}", type_tag, value);
                    
                }
                println!("LoadLocal: copying from register {} to register {}, value: {:?}", 
                src_reg, dest_reg, value);
                register_stack[reg_base + dest_reg as usize] = value;
                Ok(InstructionResult::Continue)
            }

            Opcode::LoadGlobal => {
                let dest_reg = Self::read_u8(bytecode, pc)?;     // Register to store result
                let symbol_id = Self::read_u32(bytecode, pc)?;   // Symbol ID to look up
                
                // Look up the global symbol (not use it as register index!)
                match vm.resolve_global_symbol(current_module, symbol_id) {
                    Some(value) => {
                        register_stack[reg_base + dest_reg as usize] = value;  // Use dest_reg, not symbol_id
                    }
                    None => {
                        let symbol = vm.symbol_table.read().get_symbol(symbol_id);
                        return Err(format!("Global symbol {} not found", symbol.unwrap_or("Unknown symbol.".to_string())));
                    }
                }
                Ok(InstructionResult::Continue)
            }
            
            Opcode::LoadGlobal => {
                let reg = Self::read_u8(bytecode, pc)?;
                let symbol_id = Self::read_u32(bytecode, pc)?;
                let module_id = current_module; // Use context module
                match vm.resolve_global_symbol(module_id, symbol_id) {
                    Some(value) => {
                        register_stack[reg_base + reg as usize] = value;
                    }
                    None => {
                        let symbol = vm.symbol_table.read().get_symbol(symbol_id);
                        return Err(format!("Global symbol {} not found", symbol.unwrap_or("Unknown symbol.".to_string())));
                    }
                }
                Ok(InstructionResult::Continue)
            }
            
            Opcode::StoreGlobal => {
                let reg = Self::read_u8(bytecode, pc)?;
                let symbol_id = Self::read_u32(bytecode, pc)?;
                let value = register_stack[reg_base + reg as usize];
                let module_id = current_module;
                vm.update_module(module_id, symbol_id, value);
                Ok(InstructionResult::Continue)
            }
            
            // Arithmetic operations
            Opcode::Add => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;
                
                let left = register_stack[reg_base + left_reg as usize];
                let right = register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                let result = ValueRef::number(left_num + right_num);
                println!("Add: {} + {} = {}, storing in register {}", left_num, right_num, left_num + right_num, result_reg);
    
                register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            
            Opcode::Sub => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;
                
                let left = register_stack[reg_base + left_reg as usize];
                let right = register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                let result = ValueRef::number(left_num - right_num);
                register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            
            Opcode::Mul => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;
                
                let left = register_stack[reg_base + left_reg as usize];
                let right = register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                let result = ValueRef::number(left_num * right_num);
                register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            
            Opcode::Div => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;
                
                let left = register_stack[reg_base + left_reg as usize];
                let right = register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                
                if right_num == 0.0 {
                    return Err("Division by zero".to_string());
                }
                
                let result = ValueRef::number(left_num / right_num);
                register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            
            // Control flow
            Opcode::Jump => {
                let offset = Self::read_i16(bytecode, pc)?;
                *pc = (*pc as i32 + offset as i32) as usize;
                Ok(InstructionResult::Continue)
            }
            
            Opcode::JumpIfTrue => {
                let test_reg = Self::read_u8(bytecode, pc)?;
                let offset = Self::read_i16(bytecode, pc)?;
                let test_value = register_stack[reg_base + test_reg as usize];
                if test_value.is_truthy() {
                    *pc = (*pc as i32 + offset as i32) as usize;
                }
                Ok(InstructionResult::Continue)
            }
            
            Opcode::JumpIfFalse => {
                let test_reg = Self::read_u8(bytecode, pc)?;
                let offset = Self::read_i16(bytecode, pc)?;
                let test_value = register_stack[reg_base + test_reg as usize];
                if !test_value.is_truthy() {
                    *pc = (*pc as i32 + offset as i32) as usize;
                }
                Ok(InstructionResult::Continue)
            }
            
            // Function operations
            Opcode::Call => {
                let func_reg = Self::read_u8(bytecode, pc)?;
                let arg_count = Self::read_u8(bytecode, pc)?;
                let _result_reg = Self::read_u8(bytecode, pc)?; // Ignored - always use reg 0
                
                let func_value = register_stack[reg_base + func_reg as usize];
                
                let frame = Self::setup_function_call(register_stack, current_module,func_value, func_reg, arg_count, reg_base)?;
                Ok(InstructionResult::Call(frame))
            }
            
            Opcode::Return => {
                let reg = Self::read_u8(bytecode, pc)?;
                let return_value = register_stack[reg_base + reg as usize];
                println!("Return: moving value from register {} to register 0: {:?}", reg, return_value);
                register_stack[reg_base] = return_value;
                Ok(InstructionResult::Return)
            }
            
            Opcode::ReturnNil => {
                register_stack[reg_base] = ValueRef::nil();
                Ok(InstructionResult::Return)
            }
            
            // Comparison operations
            Opcode::Lt => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;
                
                let left = register_stack[reg_base + left_reg as usize];
                let right = register_stack[reg_base + right_reg as usize];
                
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                
                let result = if left_num < right_num {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };
                
                register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            
            Opcode::Eq => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;
                
                let left = register_stack[reg_base + left_reg as usize];
                let right = register_stack[reg_base + right_reg as usize];
                
                let result = if left == right {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };
                
                register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }

            Opcode::SetupSelfReference => {
                let self_ref_reg = Self::read_u8(bytecode, pc)?;
                Ok(InstructionResult::SetupSelfReference(self_ref_reg))
            }

             
            Opcode::CreateClosure => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let template_reg = Self::read_u8(bytecode, pc)?;
                let upvalue_count = Self::read_u8(bytecode, pc)?;
                
                let mut captures = Vec::new();
                for _ in 0..upvalue_count {
                    let parent_reg = Self::read_u8(bytecode, pc)?;
                    let symbol_id = Self::read_u32(bytecode, pc)?;
                    captures.push((parent_reg, symbol_id));
                }
                
                Ok(InstructionResult::CreateClosure {
                    dest_register: dest_reg,
                    template_register: template_reg,
                    captures,
                })
            }
            
            Opcode::LoadUpvalue => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let upvalue_index = Self::read_u8(bytecode, pc)?;
                
                Ok(InstructionResult::LoadUpvalue {
                    dest_register: dest_reg,
                    upvalue_index,
                })
            }
            
            Opcode::StoreUpvalue => {
                let upvalue_index = Self::read_u8(bytecode, pc)?;
                let src_reg = Self::read_u8(bytecode, pc)?;
                
                Ok(InstructionResult::StoreUpvalue {
                    upvalue_index,
                    src_register: src_reg,
                })
            }
            
            _ => {
                Err(format!("Unimplemented opcode: {:?}", opcode))
            }
        }
    }
    
    fn setup_function_call(register_stack: &mut Vec<ValueRef>, current_module: u32, func_value: ValueRef, func_reg: u8, arg_count: u8, caller_reg_base: usize) -> Result<CallFrame, String> {
        let (func_ref, module) = match func_value {
            ValueRef::Heap(heap) => {
                let type_tag = heap.type_tag();
                let obj_ref = heap.0;
                match type_tag {
                    TypeTag::UserDefinedFunction => {
                        let compiled_func = heap.read_callable();
                        let module = compiled_func.module;
                        (FunctionRef::CompiledFunction(compiled_func, Some(obj_ref)), module)
                    },
                    _ => return Err(format!("Invalid function value: {:?}", func_value)),
                }
            }
            ValueRef::Native(native) => {
                (FunctionRef::Native(native), current_module)
            }
            _ => return Err(format!("Invalid function value: {:?}", func_value)),
        };
        
        match func_ref {
            FunctionRef::CompiledFunction(compiled_fn, obj_ref) => {
                        // Allocate registers for new frame
                        let reg_start = register_stack.len();
                        let reg_count = compiled_fn.register_count;
                
                        // Resize register stack - include register 0 for return value
                        for _ in 0..reg_count {
                            register_stack.push(ValueRef::nil());
                        }
                
                        let param_start = compiled_fn.register_start;
                        for i in 0..arg_count.min(compiled_fn.parameter_count) {
                            let arg_value = register_stack[caller_reg_base + func_reg as usize + 1 + i as usize];
                            println!("Copying argument {} from caller register {} to parameter register {}: {:?}", i, caller_reg_base + 1 + i as usize, reg_start + (param_start as usize) + i as usize, arg_value);
                            register_stack[reg_start + (param_start as usize) + i as usize] = arg_value;
                        }
                        
                
                        let frame = CallFrame {
                            func: FunctionRef::CompiledFunction(compiled_fn, obj_ref),
                            pc: 0,
                            reg_start,
                            reg_count,
                            current_module: module,
                        };
                
                        Ok(frame)
                    }
            FunctionRef::Native(native_fn) => {
                        // Execute native function immediately
                        // TODO: Implement native function calls
                        Err("Native functions not implemented".to_string())
                    }   
            FunctionRef::Closure(closure_object, object_reference) => todo!(),
        }
    }
    
    // BYTECODE READING HELPERS
    
    fn read_u8(bytecode: &[u8], pc: &mut usize) -> Result<u8, String> {
        if *pc >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let value = bytecode[*pc];
        *pc += 1;
        Ok(value)
    }
    
    fn read_u16(bytecode: &[u8], pc: &mut usize) -> Result<u16, String> {
        if *pc + 1 >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let bytes = [bytecode[*pc], bytecode[*pc + 1]];
        *pc += 2;
        Ok(u16::from_le_bytes(bytes))
    }
    
    fn read_u32(bytecode: &[u8], pc: &mut usize) -> Result<u32, String> {
        if *pc + 3 >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let bytes = [bytecode[*pc], bytecode[*pc + 1], bytecode[*pc + 2], bytecode[*pc + 3]];
        *pc += 4;
        Ok(u32::from_le_bytes(bytes))
    }
    
    fn read_i16(bytecode: &[u8], pc: &mut usize) -> Result<i16, String> {
        if *pc + 1 >= bytecode.len() {
            return Err("Unexpected end of bytecode".to_string());
        }
        let bytes = [bytecode[*pc], bytecode[*pc + 1]];
        *pc += 2;
        Ok(i16::from_le_bytes(bytes))
    }
    
    
    
    fn extract_number(value: ValueRef) -> Result<f64, String> {
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