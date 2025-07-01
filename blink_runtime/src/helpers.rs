impl IsolatedValue {
    // Better than your current expect_arity - this should be a standalone function
}

// Standalone helper functions (more ergonomic)
pub fn expect_arity(args: &[IsolatedValue], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!("Expected {} arguments, got {}", expected, args.len()))
    } else {
        Ok(())
    }
}

pub fn expect_min_arity(args: &[IsolatedValue], min: usize) -> Result<(), String> {
    if args.len() < min {
        Err(format!("Expected at least {} arguments, got {}", min, args.len()))
    } else {
        Ok(())
    }
}

pub fn expect_arity_range(args: &[IsolatedValue], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min || args.len() > max {
        Err(format!("Expected {}-{} arguments, got {}", min, max, args.len()))
    } else {
        Ok(())
    }
}

// Argument getters with better error messages
pub fn get_number(args: &[IsolatedValue], index: usize) -> Result<f64, String> {
    args.get(index)
        .ok_or_else(|| format!("Missing argument {} (expected number)", index))?
        .as_number()
}

pub fn get_string(args: &[IsolatedValue], index: usize) -> Result<&str, String> {
    args.get(index)
        .ok_or_else(|| format!("Missing argument {} (expected string)", index))?
        .as_string()
}

pub fn get_bool(args: &[IsolatedValue], index: usize) -> Result<bool, String> {
    args.get(index)
        .ok_or_else(|| format!("Missing argument {} (expected bool)", index))?
        .as_bool()
}

pub fn get_list(args: &[IsolatedValue], index: usize) -> Result<&Vec<IsolatedValue>, String> {
    args.get(index)
        .ok_or_else(|| format!("Missing argument {} (expected list)", index))?
        .as_list()
}

// Optional arguments
pub fn get_optional_number(args: &[IsolatedValue], index: usize) -> Result<Option<f64>, String> {
    match args.get(index) {
        Some(val) => Ok(Some(val.as_number()?)),
        None => Ok(None),
    }
}

pub fn get_optional_string(args: &[IsolatedValue], index: usize) -> Result<Option<&str>, String> {
    match args.get(index) {
        Some(val) => Ok(Some(val.as_string()?)),
        None => Ok(None),
    }
}

// Collection access helpers
pub fn get_sequence(args: &[IsolatedValue], index: usize) -> Result<&[IsolatedValue], String> {
    args.get(index)
        .ok_or_else(|| format!("Missing argument {} (expected list or vector)", index))?
        .as_sequence()
}