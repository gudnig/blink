use crate::{
    error::BlinkError,
    eval::{EvalContext, EvalResult},
    runtime::{ContextualBoundary, ValueBoundary},
    value::{IsolatedValue, ValueRef},
};

pub type IsolatedNativeFn =
    Box<dyn Fn(Vec<IsolatedValue>) -> Result<IsolatedValue, String> + Send + Sync>;
pub type ContextualNativeFn =
    Box<dyn Fn(Vec<ValueRef>, &mut EvalContext) -> EvalResult + Send + Sync>;

pub enum NativeFn {
    Isolated(IsolatedNativeFn),
    Contextual(ContextualNativeFn),
}

impl std::fmt::Debug for NativeFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeFn::Isolated(_) => write!(f, "NativeFn::Isolated(<function>)"),
            NativeFn::Contextual(_) => write!(f, "NativeFn::Contextual(<function>)"),
        }
    }
}

impl NativeFn {

    pub fn call(&self, args: Vec<ValueRef>, ctx: &mut EvalContext) -> EvalResult {
        match self {
            NativeFn::Isolated(f) => {
                let mut boundary = ContextualBoundary::new(ctx);

                // Extract to isolated values
                let isolated_args: Result<Vec<_>, _> = args
                    .iter()
                    .map(|arg| boundary.extract_isolated(*arg))
                    .collect();
                let isolated_args = isolated_args.map_err(|e| BlinkError::eval(e.to_string()));

                if let Err(e) = isolated_args {
                    return EvalResult::Value(ctx.eval_error(&e.to_string()));
                }
                let isolated_args = isolated_args.unwrap();
                // Call function
                let result = f(isolated_args);

                match result {
                    Ok(result) => {
                        // Convert back
                        EvalResult::Value(boundary.alloc_from_isolated(result))
                    }
                    Err(e) => EvalResult::Value(ctx.eval_error(&e.to_string())),
                }
            }

            NativeFn::Contextual(f) => f(args, ctx),
        }
    }
}
