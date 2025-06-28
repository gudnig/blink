pub enum ParsedValue {
    // Immediate values that can be packed
    Number(f64),
    Bool(bool),
    Symbol(u32),
    Keyword(u32),
    Nil,
    
    // Complex values that need allocation
    String(String),
    List(Vec<ParsedValue>),
    Vector(Vec<ParsedValue>),
    Map(Vec<(ParsedValue, ParsedValue)>), // Keep as pairs for now
}