// Add to BlinkVM or ExecutionContext
use std::collections::HashSet;
use std::sync::Arc;
use parking_lot::RwLock;
use crate::runtime::{BlinkVM, Opcode};


pub struct ArithmeticOptimizer {
    /// Track which arithmetic operators have been globally redefined
    redefined_operators: Arc<RwLock<HashSet<u32>>>, // symbol_ids that are redefined
    /// The original symbol IDs for core arithmetic operators
    core_arithmetic_symbols: HashSet<u32>,
    vm: Arc<BlinkVM>,
}

impl ArithmeticOptimizer {
    pub fn new(vm: Arc<BlinkVM>) -> Self {
        let mut core_symbols = HashSet::new();
        
        // Register core arithmetic operators - these get special treatment
        {
            let mut symbol_table = vm.symbol_table.write();
            core_symbols.insert(symbol_table.intern("+"));
            core_symbols.insert(symbol_table.intern("-"));
            core_symbols.insert(symbol_table.intern("*"));
            core_symbols.insert(symbol_table.intern("/"));
            // Add more as needed: mod, =, <, >, etc.
        }
        
        Self {
            redefined_operators: Arc::new(RwLock::new(HashSet::new())),
            core_arithmetic_symbols: core_symbols,
            vm,
        }
    }
    
    /// Check if a symbol can be optimized to arithmetic bytecode
    pub fn can_optimize_arithmetic(&self, symbol_id: u32) -> bool {
        // Must be a core arithmetic operator
        if !self.core_arithmetic_symbols.contains(&symbol_id) {
            return false;
        }
        
        // Must not have been globally redefined
        !self.redefined_operators.read().contains(&symbol_id)
    }
    
    /// Notify that a global symbol has been redefined
    /// Call this when (def + str) or similar happens
    pub fn mark_symbol_redefined(&self, symbol_id: u32) {
        if self.core_arithmetic_symbols.contains(&symbol_id) {
            self.redefined_operators.write().insert(symbol_id);
        }
    }
    
    /// Get the optimization state for compilation decisions
    pub fn get_arithmetic_instruction(&self, symbol_id: u32) -> Option<ArithmeticOp> {
        if !self.can_optimize_arithmetic(symbol_id) {
            return None;
        }
        
        // Look up the symbol name to determine which operation
        let symbol_name = self.vm.symbol_table.read().get_symbol(symbol_id)?;
        
        match symbol_name.as_str() {
            "+" => Some(ArithmeticOp::Add),
            "-" => Some(ArithmeticOp::Sub),
            "*" => Some(ArithmeticOp::Mul),
            "/" => Some(ArithmeticOp::Div),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ArithmeticOp {
    Add,
    Sub, 
    Mul,
    Div,
}

impl ArithmeticOp {
    pub fn to_instruction(self, result_reg: u8, left_reg: u8, right_reg: u8) -> Opcode {
        // match self {
        //     ArithmeticOp::Add => Instruction::Add(result_reg, left_reg, right_reg),
        //     ArithmeticOp::Sub => Instruction::Sub(result_reg, left_reg, right_reg),
        //     ArithmeticOp::Mul => Instruction::Mul(result_reg, left_reg, right_reg),
        //     ArithmeticOp::Div => Instruction::Div(result_reg, left_reg, right_reg),
        // }
        todo!()
    }
}