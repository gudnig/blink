use blink_core::{eval::{EvalContext, EvalResult}, value::{ ContextualNativeFn, IsolatedNativeFn, IsolatedValue, NativeFn, Plugin}, ValueRef};

pub struct PluginBuilder {
    name: String,
    functions: Vec<(String, NativeFn)>,
}

impl PluginBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            functions: Vec::new(),
        }
    }

    // Now this works with closures that capture variables
    pub fn function<F>(mut self, name: &str, f: F) -> Self 
    where
        F: Fn(Vec<IsolatedValue>) -> Result<IsolatedValue, String> + Send + Sync + 'static,
    {
        let boxed_fn: IsolatedNativeFn = Box::new(f);
        self.functions.push((name.to_string(), NativeFn::Isolated(boxed_fn)));
        self
    }

    pub fn contextual_function<F>(mut self, name: &str, f: F) -> Self
    where
        F: Fn(Vec<ValueRef>, &mut EvalContext) -> EvalResult + Send + Sync + 'static,
    {
        let boxed_fn: ContextualNativeFn = Box::new(f);
        self.functions.push((name.to_string(), NativeFn::Contextual(boxed_fn)));
        self
    }

    pub fn build(self) -> Plugin {
        Plugin {
            name: self.name,
            functions: self.functions,
        }
    }
}


// Registration function signature that plugins export
//pub type PluginRegisterFn = extern "C" fn() -> Plugin; 

#[macro_export]
macro_rules! blink_plugin {
    (
        name: $name:literal,
        functions: {
            $($fn_name:literal => $fn_body:expr),+ $(,)?
        }
    ) => {
        #[no_mangle]
        pub extern "C" fn blink_register() -> $crate::Plugin {
            $crate::PluginBuilder::new($name)
                $(
                    .function($fn_name, $fn_body)
                )+
                .build()
        }
    };
}