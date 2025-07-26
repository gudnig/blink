use crate::ValueRef;

#[derive(Clone, Debug)]
pub enum Instruction {
    // Load operations
    LoadImmediate(u8, ValueRef),     // LoadImmediate(reg, value)
    LoadLocal(u8, u8),               // LoadLocal(dest_reg, src_reg)
    LoadUpvalue(u8, u8),             // LoadUpvalue(dest_reg, upvalue_idx)
    LoadGlobal(u8, u32),             // LoadGlobal(dest_reg, symbol_id)
    
    // Store operations  
    StoreLocal(u8, u8),              // StoreLocal(src_reg, dest_reg)
    StoreUpvalue(u8, u8),            // StoreUpvalue(src_reg, upvalue_idx)
    StoreGlobal(u8, u32),            // StoreGlobal(src_reg, symbol_id)
    
    // Function calls
    Call(u8, u8, u8),                // Call(func_reg, arg_count, result_reg)
    TailCall(u8, u8),                // TailCall(func_reg, arg_count)
    Return(Option<u8>),              // Return(opt_value_reg)
    
    // Control flow
    Jump(i16),                       // Jump(offset)
    JumpIfTrue(u8, i16),            // JumpIfTrue(test_reg, offset)
    JumpIfFalse(u8, i16),           // JumpIfFalse(test_reg, offset)
    
    // Let bindings (for bytecode compiled from let forms)
    BeginScope,                      // Mark beginning of new scope
    EndScope(u8),                    // End scope, unbind N variables
    Bind(u8, u32),                   // Bind(value_reg, symbol_id)
}
