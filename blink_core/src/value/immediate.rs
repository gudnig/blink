use std::fmt::Display;


// NaN-tagging constants
const NAN_MASK: u64 = 0x7FF0_0000_0000_0000;
const TAG_MASK: u64 = 0x7;

const BOOL_TAG: u64 = 1;
const SYMBOL_TAG: u64 = 2;
const NIL_TAG: u64 = 3;
const KEYWORD_TAG: u64 = 4;
const MODULE_TAG: u64 = 5;

// Packing functions
pub fn pack_number(n: f64) -> u64 {
    let bits = n.to_bits();
    if (bits & NAN_MASK) == NAN_MASK {
        panic!("Cannot pack NaN");
    }
    bits
}

pub fn pack_bool(b: bool) -> u64 {
    NAN_MASK | ((b as u64) << 3) | BOOL_TAG
}

pub fn pack_symbol(symbol_id: u32) -> u64 {
    NAN_MASK | ((symbol_id as u64) << 3) | SYMBOL_TAG
}

pub fn pack_nil() -> u64 {
    NAN_MASK | NIL_TAG
}

pub fn pack_keyword(keyword_id: u32) -> u64 {
    NAN_MASK | ((keyword_id as u64) << 3) | KEYWORD_TAG
}

pub fn pack_module(module_id: u32, symbol_id: u32) -> u64 {
    NAN_MASK | ((module_id as u64) << 3) | MODULE_TAG
}

// Unpacking
pub enum ImmediateValue {
    Number(f64),
    Bool(bool),
    Symbol(u32),
    Keyword(u32),
    Module(u32, u32),
    Nil,
}

impl Display for ImmediateValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImmediateValue::Number(n) => write!(f, "{}", n),
            ImmediateValue::Bool(b) => write!(f, "{}", b),
            ImmediateValue::Symbol(s) => write!(f, "{}", s),
            ImmediateValue::Keyword(k) => write!(f, "{}", k),
            ImmediateValue::Module(m, s) => write!(f, "{}:{}", m, s),
            ImmediateValue::Nil => write!(f, "nil"),
        }
    }
}

impl ImmediateValue {
    pub fn type_tag(&self) -> &'static str  {
        match self {
            ImmediateValue::Number(_) => "number",
            ImmediateValue::Bool(_) => "bool",
            ImmediateValue::Symbol(_) => "symbol",
            ImmediateValue::Keyword(_) => "keyword",
            ImmediateValue::Module(_, _) => "module",
            ImmediateValue::Nil => "nil",
        }
    }
}

pub fn unpack_immediate(packed: u64) -> ImmediateValue {
    if (packed & NAN_MASK) != NAN_MASK {
        // Regular number
        ImmediateValue::Number(f64::from_bits(packed))
    } else {
        // Tagged value
        match packed & TAG_MASK {
            BOOL_TAG => ImmediateValue::Bool(((packed >> 3) & 1) != 0),
            SYMBOL_TAG => ImmediateValue::Symbol((packed >> 3) as u32),
            NIL_TAG => ImmediateValue::Nil,
            _ => panic!("Invalid immediate tag: {}", packed & TAG_MASK),
        }
    }
}

// Convenient type checking
pub fn is_number(packed: u64) -> bool {
    (packed & NAN_MASK) != NAN_MASK
}

pub fn is_bool(packed: u64) -> bool {
    (packed & NAN_MASK) == NAN_MASK && (packed & TAG_MASK) == BOOL_TAG
}

pub fn is_nil(packed: u64) -> bool {
    packed == (NAN_MASK | NIL_TAG)
}

pub fn is_symbol(packed: u64) -> bool {
    (packed & NAN_MASK) == NAN_MASK && (packed & TAG_MASK) == SYMBOL_TAG
}

pub fn is_module(packed: u64) -> bool {
    (packed & NAN_MASK) == NAN_MASK && (packed & TAG_MASK) == MODULE_TAG
}