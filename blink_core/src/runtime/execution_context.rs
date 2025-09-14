use crate::compiler::{BytecodeCompiler, MacroExpander};
use crate::runtime::{BlinkRuntime, SuspendedContinuation};
use crate::{
    error::BlinkError,
    runtime::{
        blink_runtime::GLOBAL_RUNTIME, BlinkVM, ClosureObject, CompiledFunction,
        ContextualBoundary, EvalResult, Opcode, TypeTag, ValueBoundary,
    },
    value::{
        unpack_immediate, ContextualNativeFn, GcPtr, ImmediateValue, IsolatedNativeFn,
        NativeContext, ValueRef,
    },
};
use mmtk::util::ObjectReference;
use std::sync::Arc;
use crate::value::FutureHandle;

// Updated call frame for byte-sized bytecode
#[derive(Clone, Debug)]
pub struct CallFrame {
    pub func: FunctionRef,
    pub pc: usize, // Byte offset into bytecode, not instruction index
    pub reg_start: usize,
    pub reg_count: u8,
    pub current_module: u32,
}

#[derive(Debug)]
//(def when (macro [condition body] (if true '(if condition body nil) nil)))
enum InstructionResult {
    Continue,
    Return,
    Call(CallFrame),
    SetupSelfReference(u8),
    CreateClosure {
        dest_register: u8,
        template_register: u8,
        captures: Vec<(u8, u32)>, // Upvalue capture info included
    },
    LoadUpvalue {
        dest_register: u8,
        upvalue_index: u8,
    },
    StoreUpvalue {
        upvalue_index: u8,
        src_register: u8,
    },
    Suspend
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
    pub current_module: u32,
    pub register_stack: Vec<ValueRef>,
    pub call_stack: Vec<CallFrame>,
    pub current_goroutine_id: Option<u32>, // Track the current goroutine ID
}

impl ExecutionContext {
    pub fn new(vm: Arc<BlinkVM>, current_module: u32) -> Self {
        Self {
            vm: vm.clone(),
            current_module,
            
            register_stack: Vec::new(),
            call_stack: Vec::new(),
            current_goroutine_id: None, // Default to no goroutine (main thread execution)
        }
    }

    pub fn compile_and_execute(&mut self, expr: ValueRef) -> Result<ValueRef, BlinkError> {
        let mut macro_expander = MacroExpander::new(self.vm.clone(), self.current_module);

        let expanded = macro_expander
            .expand(*&expr)
            .map_err(|e| BlinkError::eval(e))?;
        let mut compiler = BytecodeCompiler::new(self.vm.clone(), self.current_module);
        let compiled = compiler
            .compile_for_storage(expanded)
            .map_err(|e| BlinkError::eval(e))?;

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

        res
    }

    /// Execute a single step (one instruction) and return whether to continue
    pub fn execute_single_step(&mut self) -> Result<bool, String> {
        if self.call_stack.is_empty() {
            return Ok(false); // No more work to do
        }

        // Get current frame (don't pop yet)
        let mut current_frame = if let Some(frame) = self.call_stack.last().cloned() {
            frame
        } else {
            return Ok(false);
        };

        match &current_frame.func {
            FunctionRef::CompiledFunction(compiled_fn, obj_ref) => {
                // Check for end of function
                if current_frame.pc >= compiled_fn.bytecode.len() {
                    self.handle_function_completion()?;
                    return Ok(!self.call_stack.is_empty());
                }

                let opcode = Opcode::from_u8(compiled_fn.bytecode[current_frame.pc])?;
                current_frame.pc += 1;

                let instruction_result = self.execute_instruction(
                    opcode,
                    &compiled_fn.bytecode,
                    &compiled_fn.constants,
                    current_frame.reg_start,
                    &mut current_frame.pc,
                )?;

                self.handle_instruction_result(instruction_result, current_frame)?;
                Ok(true)
            }
            FunctionRef::Native(tagged_ptr) => {
                // Native functions complete in one step
                self.execute_native_function(*tagged_ptr)?;
                Ok(!self.call_stack.is_empty())
            }
            FunctionRef::Closure(closure_obj, obj_ref) => {
                let template_fn = GcPtr::new(closure_obj.template).read_callable();

                if current_frame.pc >= template_fn.bytecode.len() {
                    self.handle_function_completion()?;
                    return Ok(!self.call_stack.is_empty());
                }

                let opcode = Opcode::from_u8(template_fn.bytecode[current_frame.pc])?;
                current_frame.pc += 1;

                let instruction_result = self.execute_instruction(

                    opcode,
                    &template_fn.bytecode,
                    &template_fn.constants,
                    current_frame.reg_start,
                    &mut current_frame.pc,
                )?;

                self.handle_instruction_result(instruction_result, current_frame)?;
                Ok(true)
            }
        }
    }

    /// Handle completion of a function (natural end or explicit return)
    fn handle_function_completion(&mut self) -> Result<(), String> {
        let completed_frame = self.call_stack.pop().unwrap();
        let return_value = self.register_stack[completed_frame.reg_start];

        // Clean up registers
        self.register_stack.truncate(completed_frame.reg_start);

        if !self.call_stack.is_empty() {
            // Store return value in caller's register 0
            if let Some(caller_frame) = self.call_stack.last() {
                self.register_stack[caller_frame.reg_start] = return_value;
            }
        }
        Ok(())
    }

    /// Execute a native function and handle its completion
    fn execute_native_function(&mut self, tagged_ptr: usize) -> Result<(), String> {
        // Get arguments from registers (skip register 0 which is for return value)
        let current_frame = self.call_stack.last().unwrap();
        let arg_count = current_frame.reg_count as usize - 1; // Subtract 1 for return register
        let mut args = Vec::with_capacity(arg_count);

        for i in 0..arg_count {
            args.push(self.register_stack[current_frame.reg_start + 1 + i]);
        }

        // Decode tagged pointer and call appropriate function type
        let ptr = tagged_ptr & !1; // Clear the tag bit
        let return_value = if tagged_ptr & 1 == 0 {
            // Tag 0 = Isolated function
            let boxed_fn_ptr = ptr as *const IsolatedNativeFn;
            let boxed_fn = unsafe { &*boxed_fn_ptr };

            // Convert args to isolated values and call
            let mut boundary = ContextualBoundary::new(self.vm.clone());
            let isolated_args: Result<Vec<_>, _> = args
                .iter()
                .map(|arg| boundary.extract_isolated(*arg))
                .collect();

            match isolated_args {
                Ok(isolated_args) => match boxed_fn(isolated_args) {
                    Ok(result) => boundary.alloc_from_isolated(result),
                    Err(e) => self.vm.eval_error(&e.to_string()),
                },
                Err(e) => self.vm.eval_error(&e.to_string()),
            }
        } else {
            // Tag 1 = Contextual function
            let boxed_fn_ptr = ptr as *const ContextualNativeFn;
            let boxed_fn = unsafe { &*boxed_fn_ptr };
            let mut ctx = NativeContext::new(&self.vm);

            // Call function and extract value (ignore suspension for now)
            match boxed_fn(args, &mut ctx) {
                EvalResult::Value(val) => val,
                EvalResult::Suspended { .. } => {
                    // Convert suspension to error for now
                    self.vm
                        .eval_error("Native function suspension not supported")
                }
            }
        };

        // Native function completed - pop frame and handle return
        self.handle_function_completion()?;

        // Update return value after frame cleanup
        if !self.call_stack.is_empty() {
            if let Some(caller_frame) = self.call_stack.last() {
                self.register_stack[caller_frame.reg_start] = return_value;
            }
        }

        Ok(())
    }

    /// Handle the result of executing an instruction
    fn handle_instruction_result(
        &mut self,
        instruction_result: InstructionResult,
        mut current_frame: CallFrame,
    ) -> Result<(), String> {
        match instruction_result {
            InstructionResult::Continue => {
                        // Update the frame in the stack
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
            InstructionResult::Return => {
                        self.handle_function_completion()?;
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
                        if let FunctionRef::CompiledFunction(_, Some(obj_ref))
                        | FunctionRef::Closure(_, Some(obj_ref)) = &current_frame.func
                        {
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
            InstructionResult::LoadUpvalue {
                        dest_register,
                        upvalue_index,
                    } => {
                        match &current_frame.func {
                            FunctionRef::Closure(closure_obj, _) => {
                                if let Some(upvalue) = closure_obj.upvalues.get(upvalue_index as usize) {
                                    self.register_stack[current_frame.reg_start + dest_register as usize] =
                                        *upvalue;
                                } else {
                                    return Err(format!("Upvalue index {} out of bounds", upvalue_index));
                                }
                            }
                            FunctionRef::CompiledFunction(_, Some(obj_ref)) => {
                                let closure = GcPtr(*obj_ref).read_closure();
                                if let Some(upvalue) = closure.upvalues.get(upvalue_index as usize) {
                                    self.register_stack[current_frame.reg_start + dest_register as usize] =
                                        *upvalue;
                                } else {
                                    return Err(format!("Upvalue index {} out of bounds", upvalue_index));
                                }
                            }
                            _ => return Err("LoadUpvalue called on non-closure function".to_string()),
                        }

                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
            InstructionResult::StoreUpvalue {
                        upvalue_index,
                        src_register,
                    } => {
                        let value = self.register_stack[current_frame.reg_start + src_register as usize];

                        match &current_frame.func {
                            FunctionRef::Closure(_, Some(obj_ref))
                            | FunctionRef::CompiledFunction(_, Some(obj_ref)) => {
                                GcPtr(*obj_ref).set_upvalue(upvalue_index as usize, value)?;
                            }
                            _ => {
                                return Err(
                                    "StoreUpvalue called on function without object reference".to_string()
                                )
                            }
                        }

                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.pc = current_frame.pc;
                        }
                    }
            InstructionResult::CreateClosure {
                        dest_register,
                        template_register,
                        captures,
                    } => {
                        // Get template
                        let template_value =
                            self.register_stack[current_frame.reg_start + template_register as usize];
                        let template_obj_ref = if let ValueRef::Heap(heap_ptr) = template_value {
                            heap_ptr.0
                        } else {
                            return Err("Template must be a heap object".to_string());
                        };

                        // Capture upvalues directly from registers
                        let mut upvalues = Vec::new();
                        for (parent_reg, _symbol_id) in captures {
                            let captured_value =
                                self.register_stack[current_frame.reg_start + parent_reg as usize];
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
            InstructionResult::Suspend => {




                        // For single-step execution, we should return an error to signal suspension
                        return Err("SUSPENDED".to_string());
                    }

        }
        Ok(())
    }

    // Main execution loop - processes all frames until stack is empty
    pub fn execute(&mut self) -> Result<ValueRef, String> {
        while !self.call_stack.is_empty() {
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

                let instruction_result = self.execute_instruction(
                    opcode,
                    &compiled_fn.bytecode,
                    &compiled_fn.constants,
                    current_frame.reg_start,
                    &mut current_frame.pc,
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

                self.handle_instruction_result(instruction_result, current_frame)?;


            } else if let FunctionRef::Native(tagged_ptr) = &current_frame.func {
                // Handle native function execution using tagged pointer

                // Get arguments from registers (skip register 0 which is for return value)
                let arg_count = current_frame.reg_count as usize - 1; // Subtract 1 for return register
                let mut args = Vec::with_capacity(arg_count);

                for i in 0..arg_count {
                    args.push(self.register_stack[current_frame.reg_start + 1 + i]);
                }

                // Decode tagged pointer and call appropriate function type
                let ptr = tagged_ptr & !1; // Clear the tag bit
                let return_value = if tagged_ptr & 1 == 0 {
                    // Tag 0 = Isolated function
                    let boxed_fn_ptr = ptr as *const IsolatedNativeFn;
                    let boxed_fn = unsafe { &*boxed_fn_ptr };

                    // Convert args to isolated values and call
                    let mut boundary = ContextualBoundary::new(self.vm.clone());
                    let isolated_args: Result<Vec<_>, _> = args
                        .iter()
                        .map(|arg| boundary.extract_isolated(*arg))
                        .collect();

                    match isolated_args {
                        Ok(isolated_args) => match boxed_fn(isolated_args) {
                            Ok(result) => boundary.alloc_from_isolated(result),
                            Err(e) => self.vm.eval_error(&e.to_string()),
                        },
                        Err(e) => self.vm.eval_error(&e.to_string()),
                    }
                } else {
                    // Tag 1 = Contextual function
                    let boxed_fn_ptr = ptr as *const ContextualNativeFn;
                    let boxed_fn = unsafe { &*boxed_fn_ptr };
                    let mut ctx = NativeContext::new(&self.vm);

                    // Call function and extract value (ignore suspension for now)
                    match boxed_fn(args, &mut ctx) {
                        EvalResult::Value(val) => val,
                        EvalResult::Suspended { .. } => {
                            // Convert suspension to error for now
                            self.vm
                                .eval_error("Native function suspension not supported")
                        }
                    }
                };

                // Native function completed - pop frame and handle return
                let completed_frame = self.call_stack.pop().unwrap();

                // Clean up registers
                self.register_stack.truncate(completed_frame.reg_start);

                if self.call_stack.is_empty() {
                    return Ok(return_value);
                }

                // Store return value in caller's register 0
                if let Some(caller_frame) = self.call_stack.last() {
                    self.register_stack[caller_frame.reg_start] = return_value;
                }
            } else if let FunctionRef::Closure(closure_obj, obj_ref) = &current_frame.func {
                // Extract the template function
                let template_fn = GcPtr::new(closure_obj.template).read_callable();

                // Check for end of function (same logic as CompiledFunction)
                if current_frame.pc >= template_fn.bytecode.len() {
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

                // Execute bytecode instruction (same as CompiledFunction but with closure context)
                let opcode = Opcode::from_u8(template_fn.bytecode[current_frame.pc])?;
                current_frame.pc += 1;

                let instruction_result = self.execute_instruction(
                    opcode,
                    &template_fn.bytecode,
                    &template_fn.constants,
                    current_frame.reg_start,
                    &mut current_frame.pc,
                )?;

                self.handle_instruction_result(instruction_result, current_frame)?;

                continue; // Continue to next iteration of the main execution loop
            }
        }

        Ok(ValueRef::nil())
    }

    fn execute_instruction(&mut self,
                           opcode: Opcode,
                           bytecode: &[u8],
                           constants: &[ValueRef],
                           reg_base: usize,
                           pc: &mut usize,
    ) -> Result<InstructionResult, String> {
        match opcode {
            Opcode::LoadImm8 => {
                let reg = Self::read_u8(bytecode, pc)?;
                let value = Self::read_u8(bytecode, pc)?;
                self.register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                Ok(InstructionResult::Continue)
            }
            Opcode::LoadImm16 => {
                let reg = Self::read_u8(bytecode, pc)?;
                let value = Self::read_u16(bytecode, pc)?;
                self.register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                Ok(InstructionResult::Continue)
            }
            Opcode::LoadImm32 => {
                let reg = Self::read_u8(bytecode, pc)?;
                let value = Self::read_u32(bytecode, pc)?;
                self.register_stack[reg_base + reg as usize] = ValueRef::number(value as f64);
                Ok(InstructionResult::Continue)
            }
            Opcode::LoadImmConst => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let const_index = Self::read_u8(bytecode, pc)?;

                if const_index as usize >= constants.len() {
                    return Err(format!(
                        "Constant index {} out of bounds (have {} constants)",
                        const_index,
                        constants.len()
                    ));
                }

                let constant = constants[const_index as usize];
                self.register_stack[reg_base + dest_reg as usize] = constant;
                Ok(InstructionResult::Continue)
            }
            Opcode::LoadLocal => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let src_reg = Self::read_u8(bytecode, pc)?;

                // Check bounds before accessing
                if reg_base + src_reg as usize >= self.register_stack.len() {
                    return Err(format!("Register {} out of bounds", src_reg));
                }

                let value = self.register_stack[reg_base + src_reg as usize];

                self.register_stack[reg_base + dest_reg as usize] = value;
                Ok(InstructionResult::Continue)
            }
            Opcode::LoadGlobal => {
                let dest_reg = Self::read_u8(bytecode, pc)?; // Register to store result
                let symbol_id = Self::read_u32(bytecode, pc)?; // Symbol ID to look up

                // Look up the global symbol (not use it as register index!)
                match self.vm.resolve_global_symbol(self.current_module, symbol_id) {
                    Some(value) => {
                        self.register_stack[reg_base + dest_reg as usize] = value; // Use dest_reg, not symbol_id
                    }
                    None => {
                        let symbol = self.vm.symbol_table.read().get_symbol(symbol_id);
                        return Err(format!(
                            "Global symbol {} not found",
                            symbol.unwrap_or("Unknown symbol.".to_string())
                        ));
                    }
                }
                Ok(InstructionResult::Continue)
            }
            Opcode::StoreGlobal => {
                let reg = Self::read_u8(bytecode, pc)?;
                let symbol_id = Self::read_u32(bytecode, pc)?;
                let value = self.register_stack[reg_base + reg as usize];
                let module_id = self.current_module;
                self.vm.update_module(module_id, symbol_id, value);
                Ok(InstructionResult::Continue)
            }
            Opcode::Add => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                let result = ValueRef::number(left_num + right_num);

                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Sub => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                let result = ValueRef::number(left_num - right_num);
                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Mul => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;
                let result = ValueRef::number(left_num * right_num);
                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Div => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];
                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;

                if right_num == 0.0 {
                    return Err("Division by zero".to_string());
                }

                let result = ValueRef::number(left_num / right_num);
                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Jump => {
                let offset = Self::read_i16(bytecode, pc)?;
                // Use current PC (after reading offset) as base for jump
                *pc = (*pc as i32 + offset as i32) as usize;
                Ok(InstructionResult::Continue)
            }
            Opcode::JumpIfTrue => {
                let test_reg = Self::read_u8(bytecode, pc)?;
                let offset = Self::read_i16(bytecode, pc)?;
                let test_value = self.register_stack[reg_base + test_reg as usize];
                if test_value.is_truthy() {
                    *pc = (*pc as i32 + offset as i32) as usize; // Fixed: (*pc) not (pc*)
                }
                Ok(InstructionResult::Continue)
            }
            Opcode::JumpIfFalse => {
                let test_reg = Self::read_u8(bytecode, pc)?;
                let offset = Self::read_i16(bytecode, pc)?;

                let test_value = self.register_stack[reg_base + test_reg as usize];

                if !test_value.is_truthy() {
                    let new_pc = (*pc as i32 + offset as i32) as usize;
                    *pc = new_pc;
                }

                Ok(InstructionResult::Continue)
            }
            Opcode::Call => {
                let func_reg = Self::read_u8(bytecode, pc)?;
                let arg_count = Self::read_u8(bytecode, pc)?;
                let _result_reg = Self::read_u8(bytecode, pc)?; // Ignored - always use reg 0

                let func_value = self.register_stack[reg_base + func_reg as usize];

                let frame = Self::setup_function_call(
                    &mut self.register_stack,
                    self.current_module,
                    func_value,
                    func_reg,
                    arg_count,
                    reg_base,
                )?;
                Ok(InstructionResult::Call(frame))
            }
            Opcode::Return => {
                let reg = Self::read_u8(bytecode, pc)?;
                let return_value = self.register_stack[reg_base + reg as usize];
                self.register_stack[reg_base] = return_value;
                Ok(InstructionResult::Return)
            }
            Opcode::ReturnNil => {
                self.register_stack[reg_base] = ValueRef::nil();
                Ok(InstructionResult::Return)
            }
            Opcode::Lt => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;

                let result = if left_num < right_num {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };

                self.register_stack[reg_base + result_reg as usize] = result;

                Ok(InstructionResult::Continue)
            }
            Opcode::Gt => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;

                let result = if left_num > right_num {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };

                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Eq => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let result = if left == right {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };

                self.register_stack[reg_base + result_reg as usize] = result;
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
            Opcode::GtEq => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;

                let result = ValueRef::boolean(left_num >= right_num);
                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::LtEq => {
                let result_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let left_num = Self::extract_number(left)?;
                let right_num = Self::extract_number(right)?;

                let result = ValueRef::boolean(left_num <= right_num);
                self.register_stack[reg_base + result_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::StoreLocal => todo!(),
            Opcode::AddImm8 => todo!(),
            Opcode::SubImm8 => todo!(),
            Opcode::MulImm8 => todo!(),
            Opcode::DivImm8 => todo!(),
            Opcode::TailCall => {
                let func_reg = Self::read_u8(bytecode, pc)?;
                let arg_count = Self::read_u8(bytecode, pc)?;

                let func = self.register_stack[reg_base + func_reg as usize];

                // For tail calls, we reuse the current stack frame
                // Move arguments to the beginning of the current register window
                for i in 0..arg_count {
                    let arg_reg = func_reg + 1 + i; // Args follow function register
                    let arg_value = self.register_stack[reg_base + arg_reg as usize];
                    self.register_stack[reg_base + i as usize] = arg_value;
                }

                // Set up for function call - this replaces the current frame
                match func {
                    ValueRef::Heap(gc_ptr) => {
                        // Handle compiled functions, closures, etc.
                        // This would integrate with existing function call logic
                        // but without creating a new stack frame

                        // For now, let's handle the basic case
                        todo!("Handle compiled function tail calls")
                    }
                    _ => {
                        return Err("TailCall: not a function".to_string());
                    }
                }
            }
            Opcode::CallDynamic => todo!(),
            Opcode::TailCallDynamic => todo!(),
            Opcode::PrepareArgs => todo!(),
            Opcode::BeginScope => todo!(),
            Opcode::EndScope => todo!(),
            Opcode::Bind => todo!(),
            Opcode::GetLength => todo!(),
            Opcode::GetElement => todo!(),
            Opcode::InitLoop => todo!(),
            Opcode::LoopTest => todo!(),
            Opcode::LoopIncr => todo!(),
            Opcode::And => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let result = if left.is_truthy() && right.is_truthy() {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };

                self.register_stack[reg_base + dest_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Or => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let left_reg = Self::read_u8(bytecode, pc)?;
                let right_reg = Self::read_u8(bytecode, pc)?;

                let left = self.register_stack[reg_base + left_reg as usize];
                let right = self.register_stack[reg_base + right_reg as usize];

                let result = if left.is_truthy() || right.is_truthy() {
                    ValueRef::boolean(true)
                } else {
                    ValueRef::boolean(false)
                };

                self.register_stack[reg_base + dest_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Not => {
                let dest_reg = Self::read_u8(bytecode, pc)?;
                let value_reg = Self::read_u8(bytecode, pc)?;

                let value = self.register_stack[reg_base + value_reg as usize];
                let result = if value.is_truthy() {
                    ValueRef::boolean(false)
                } else {
                    ValueRef::boolean(true)
                };

                self.register_stack[reg_base + dest_reg as usize] = result;
                Ok(InstructionResult::Continue)
            }
            Opcode::Spawn => {
                let result_reg = bytecode[*pc] as usize;
                let func_reg = bytecode[*pc + 1] as usize;
                *pc += 2;

                let func_value = self.register_stack[reg_base + func_reg];

                // Access the global runtime to spawn goroutine
                if let Some(runtime) = GLOBAL_RUNTIME.get() {
                    match runtime.spawn_goroutine(func_value) {
                        Ok(goroutine_id) => {
                            self.register_stack[reg_base + result_reg] =
                                ValueRef::number(goroutine_id as f64);
                        }
                        Err(error) => {
                            return Err(format!("Failed to spawn goroutine: {}", error));
                        }
                    }
                } else {
                    return Err("Runtime not initialized - cannot spawn goroutine".to_string());
                }

                Ok(InstructionResult::Continue)
            }
            Opcode::Await => {
                let result_reg = bytecode[*pc] as usize;
                let value_reg = bytecode[*pc + 1] as usize;
                *pc += 2;

                let value_ref = self.register_stack[reg_base + value_reg];

                // Handle different types gracefully
                if let Some(handle) = value_ref.get_future_handle() {
                    // Fast path: check if already completed
                    if let Some(result) = handle.try_poll() {
                        self.register_stack[reg_base + result_reg] = result;
                        return Ok(InstructionResult::Continue);
                    }

                    // Slow path: create continuation and do atomic register
                    let continuation = SuspendedContinuation {
                        goroutine_id: self.current_goroutine_id.unwrap_or(0),
                        dest_register: result_reg as u8,
                        call_stack: self.call_stack.clone(),
                        register_stack: self.register_stack.clone(),
                        current_module: self.current_module,
                        resume_pc: *pc,
                    };

                    match handle.register_continuation(continuation)? {
                        Some(completed_value) => {
                            self.register_stack[reg_base + result_reg] = completed_value;
                            Ok(InstructionResult::Continue)
                        }
                        None => {
                            Ok(InstructionResult::Suspend)
                        }
                    }
                } else {
                    // Not a future - check if it's an atom/ref or just return the value
                    // TODO: When atoms are implemented, add atom dereferencing here
                    // For now, just return the value as-is (non-futures pass through unchanged)
                    self.register_stack[reg_base + result_reg] = value_ref;
                    Ok(InstructionResult::Continue)
                }
            }
            Opcode::CreateFuture => {
                let result_reg = bytecode[*pc] as usize;
                *pc += 1;

                let future = self.vm.create_future();
                self.register_stack[reg_base + result_reg] = future;
                Ok(InstructionResult::Continue) 
            }
            Opcode::CompleteFuture => {
                let result_reg = bytecode[*pc] as usize;
                let future_reg = bytecode[*pc + 1] as usize;
                let value_reg = bytecode[*pc + 2] as usize;
                *pc += 3;

                let future_ref = self.register_stack[reg_base + future_reg];
                let value = self.register_stack[reg_base + value_reg];

                if future_ref.is_future() {

                    self.vm.complete_future_value(future_ref, value)?;
                    self.register_stack[reg_base + result_reg] = ValueRef::nil();
                    Ok(InstructionResult::Continue)
                } else {
                    Err("Expected future".to_string())
                }
            }
            Opcode::Suspend => {
                // Suspend current execution - this will need coordination with scheduler
                // For now, just continue execution
                // TODO: Integrate with BlinkRuntime scheduler
                Ok(InstructionResult::Continue)
            }
            Opcode::Resume => {
                // Resume execution - this will need coordination with scheduler
                // For now, just continue execution
                // TODO: Integrate with BlinkRuntime scheduler
                Ok(InstructionResult::Continue)
            }
        }
    }

    fn setup_function_call(
        register_stack: &mut Vec<ValueRef>,
        current_module: u32,
        func_value: ValueRef,
        func_reg: u8,
        arg_count: u8,
        caller_reg_base: usize,
    ) -> Result<CallFrame, String> {
        let (func_ref, module) = match func_value {
            ValueRef::Heap(heap) => {
                let type_tag = heap.type_tag();
                let obj_ref = heap.0;
                match type_tag {
                    TypeTag::UserDefinedFunction | TypeTag::Macro => {
                        let compiled_func = heap.read_callable();
                        let module = compiled_func.module;
                        (
                            FunctionRef::CompiledFunction(compiled_func, Some(obj_ref)),
                            module,
                        )
                    }
                    TypeTag::Closure => {
                        let closure_obj = heap.read_closure();
                        let template_fn = GcPtr::new(closure_obj.template).read_callable();
                        let module = template_fn.module;
                        (FunctionRef::Closure(closure_obj, Some(obj_ref)), module)
                    }
                    _ => return Err(format!("Invalid function value: {:?}", func_value)),
                }
            }
            ValueRef::Handle(native) => (FunctionRef::Native(native), current_module),
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
                    let arg_value =
                        register_stack[caller_reg_base + func_reg as usize + 1 + i as usize];

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
                // For native functions, we need minimal register setup but still create a frame
                // for call stack tracing and suspension support

                // Allocate minimal registers: 1 for return value + arg_count for arguments
                let reg_start = register_stack.len();
                let reg_count = 1 + arg_count as usize; // Return slot + arguments

                // Resize register stack
                for _ in 0..reg_count {
                    register_stack.push(ValueRef::nil());
                }

                // Copy arguments to the new frame's registers (starting at register 1)
                // Register 0 is reserved for the return value
                for i in 0..arg_count {
                    let arg_value =
                        register_stack[caller_reg_base + func_reg as usize + 1 + i as usize];
                    register_stack[reg_start + 1 + i as usize] = arg_value;
                }

                let frame = CallFrame {
                    func: FunctionRef::Native(native_fn),
                    pc: 0, // Native functions don't use PC, but set to 0 for consistency
                    reg_start,
                    reg_count: reg_count as u8,
                    current_module: module,
                };

                Ok(frame)
            }

            FunctionRef::Closure(closure_object, object_reference) => {
                // Extract the template function from the closure object
                let template_obj_ref = closure_object.template;
                let template_fn = GcPtr::new(template_obj_ref).read_callable();

                // Allocate registers for new frame - same as CompiledFunction but with closure context
                let reg_start = register_stack.len();
                let reg_count = template_fn.register_count;

                // Resize register stack - include register 0 for return value
                for _ in 0..reg_count {
                    register_stack.push(ValueRef::nil());
                }

                // Copy function arguments to parameter registers (same as CompiledFunction)
                let param_start = template_fn.register_start;
                for i in 0..arg_count.min(template_fn.parameter_count) {
                    let arg_value =
                        register_stack[caller_reg_base + func_reg as usize + 1 + i as usize];
                    register_stack[reg_start + (param_start as usize) + i as usize] = arg_value;
                }

                // Create call frame with closure function reference that includes the closure object reference
                // This ensures that LoadUpvalue and StoreUpvalue instructions can access the upvalues
                let frame = CallFrame {
                    func: FunctionRef::Closure(closure_object, object_reference),
                    pc: 0,
                    reg_start,
                    reg_count,
                    current_module: module,
                };

                Ok(frame)
            }
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
        let bytes = [
            bytecode[*pc],
            bytecode[*pc + 1],
            bytecode[*pc + 2],
            bytecode[*pc + 3],
        ];
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
                            result.push_str(&format!(
                                " r{}, const[{}] = {:?}",
                                reg, const_idx, constants[const_idx as usize]
                            ));
                        }
                        pc += 2;
                    }
                }

                Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div => {
                    if pc + 2 < bytecode.len() {
                        let result_reg = bytecode[pc];
                        let left_reg = bytecode[pc + 1];
                        let right_reg = bytecode[pc + 2];
                        result
                            .push_str(&format!(" r{}, r{}, r{}", result_reg, left_reg, right_reg));
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
