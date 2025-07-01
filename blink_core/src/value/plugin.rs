use crate::value::NativeFn;

pub struct Plugin {
    pub name: String,
    pub functions: Vec<(String, NativeFn)>,
}