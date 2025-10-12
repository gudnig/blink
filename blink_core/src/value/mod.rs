mod gc_ptr;
mod heap_value;
mod immediate;
mod isolated_value;
mod native_fn;
mod parsed_value;
mod plugin;
mod value_ref;
mod future_handle;
mod channel_handle;

pub use gc_ptr::*;
pub use heap_value::*;
pub use immediate::*;
pub use isolated_value::*;
pub use native_fn::*;
pub use parsed_value::*;
pub use plugin::*;
pub use value_ref::*;
pub use future_handle::*;
pub use channel_handle::*;
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct SourcePos {
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for SourcePos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.col)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub struct SourceRange {
    pub start: SourcePos,
    pub end: SourcePos,
}

impl SourceRange {
    pub fn new(start: SourcePos, end: SourcePos) -> Self {
        Self { start, end }
    }
}
impl Default for SourceRange {
    fn default() -> Self {
        Self {
            start: SourcePos { line: 0, col: 0 },
            end: SourcePos { line: 0, col: 0 },
        }
    }
}

impl std::fmt::Display for SourceRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}
