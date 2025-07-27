use crate::value::ValueRef;

// Opcodes - each fits in a single byte
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Opcode {
    // Load/Store operations
    LoadImm8 = 0x01,        // Load 8-bit immediate
    LoadImm16 = 0x02,       // Load 16-bit immediate  
    LoadImm32 = 0x03,       // Load 32-bit immediate
    LoadImmConst = 0x04,    // Load from constant table
    LoadLocal = 0x05,       // Load from local register
    LoadGlobal = 0x06,      // Load from global symbol
    LoadUpvalue = 0x07,     // Load from upvalue
    
    StoreLocal = 0x10,      // Store to local register
    StoreGlobal = 0x11,     // Store to global symbol
    StoreUpvalue = 0x12,    // Store to upvalue
    
    // Arithmetic operations
    Add = 0x20,             // Add two registers
    Sub = 0x21,             // Subtract two registers
    Mul = 0x22,             // Multiply two registers
    Div = 0x23,             // Divide two registers
    AddImm8 = 0x24,         // Add 8-bit immediate to register
    SubImm8 = 0x25,         // Subtract 8-bit immediate from register
    MulImm8 = 0x26,         // Multiply 8-bit immediate by register
    DivImm8 = 0x27,         // Divide register by 8-bit immediate
    
    // Comparison operations
    Eq = 0x30,              // Equal comparison
    Lt = 0x31,              // Less than comparison
    Gt = 0x32,              // Greater than comparison
    
    // Control flow
    Jump = 0x40,            // Unconditional jump
    JumpIfTrue = 0x41,      // Jump if register is truthy
    JumpIfFalse = 0x42,     // Jump if register is falsy
    
    // Function operations
    Call = 0x50,            // Call function
    TailCall = 0x51,        // Tail call function
    Return = 0x52,          // Return from function
    ReturnNil = 0x53,       // Return nil
    CallDynamic = 0x54,     // Call function with prepared arguments
    TailCallDynamic = 0x55, // Tail call with prepared arguments
    PrepareArgs = 0x56,     // Prepare argument registers for function call
    
    // Scope operations
    BeginScope = 0x60,      // Begin new scope
    EndScope = 0x61,        // End current scope
    Bind = 0x62,            // Bind value to symbol

    // Collection operations
    GetLength = 0x70,       // Get length of list/vector
    GetElement = 0x71,      // Get element at index
    
    // Loop operations  
    InitLoop = 0x80,        // Initialize loop counter
    LoopTest = 0x81,        // Test loop condition and jump
    LoopIncr = 0x82,        // Increment loop counter
}

impl Opcode {
    pub fn from_u8(byte: u8) -> Result<Self, String> {
        match byte {
            0x01 => Ok(Opcode::LoadImm8),
            0x02 => Ok(Opcode::LoadImm16),
            0x03 => Ok(Opcode::LoadImm32),
            0x04 => Ok(Opcode::LoadImmConst),
            0x05 => Ok(Opcode::LoadLocal),
            0x06 => Ok(Opcode::LoadGlobal),
            0x07 => Ok(Opcode::LoadUpvalue),
            0x10 => Ok(Opcode::StoreLocal),
            0x11 => Ok(Opcode::StoreGlobal),
            0x12 => Ok(Opcode::StoreUpvalue),
            0x20 => Ok(Opcode::Add),
            0x21 => Ok(Opcode::Sub),
            0x22 => Ok(Opcode::Mul),
            0x23 => Ok(Opcode::Div),
            0x30 => Ok(Opcode::Eq),
            0x31 => Ok(Opcode::Lt),
            0x32 => Ok(Opcode::Gt),
            0x40 => Ok(Opcode::Jump),
            0x41 => Ok(Opcode::JumpIfTrue),
            0x42 => Ok(Opcode::JumpIfFalse),
            0x50 => Ok(Opcode::Call),
            0x51 => Ok(Opcode::TailCall),
            0x52 => Ok(Opcode::Return),
            0x53 => Ok(Opcode::ReturnNil),
            0x60 => Ok(Opcode::BeginScope),
            0x61 => Ok(Opcode::EndScope),
            0x62 => Ok(Opcode::Bind),
            _ => Err(format!("Invalid opcode: 0x{:02x}", byte)),
        }
    }
}

// Bytecode is just a vector of bytes
pub type Bytecode = Vec<u8>;

// Compiled function stores raw bytecode + metadata
#[derive(Clone, Debug)]
pub struct CompiledFunction {
    pub bytecode: Bytecode,
    pub constants: Vec<ValueRef>,  // Constant pool for complex values
    pub parameter_count: u8,
    pub register_count: u8,
    pub module: u32,
}

// Label patch for jump instructions
#[derive(Debug)]
pub struct LabelPatch {
    pub bytecode_offset: usize,
    pub label_id: u16,
}