use crate::{
    env::Env, native_functions::{
        native_add, native_concat, native_cons, native_div, native_eq, native_error, native_first, native_future, native_gc_stress, native_get, native_list, native_map_construct, native_mul, native_not, native_print, native_report_gc_stats, native_rest, native_run_scheduler, native_sub, native_type_of, native_vector
    }, runtime::{BlinkVM, EvalResult, Macro}, value::{pack_number, Callable, GcPtr, NativeContext, NativeFn, ValueRef}
};

impl BlinkVM {
    pub fn register_builtins(&mut self, module: u32) {
        

        let mut reg =
            |s: &str, f: fn(Vec<ValueRef>, ctx: &mut NativeContext) -> EvalResult, module: u32| -> ValueRef {
                let sym = self.symbol_table.write().intern(s);
                let boxed = Box::new(f);
                let native_fn = NativeFn::Contextual(boxed);
                let val = ValueRef::native_function(native_fn);
                
                self.update_module(module, sym, val);

                val
            };

        reg("+", native_add, module);
        reg("-", native_sub, module);
        reg("*", native_mul, module);
        reg("/", native_div, module);
        reg("=", native_eq, module);
        reg("not", native_not, module);

        reg("list", native_list, module);
        reg("vector", native_vector, module);
        reg("hash-map", native_map_construct, module);
        //reg("map", native_map); // TODO need to do a map fn
        reg("print", native_print, module);
        reg("type-of", native_type_of, module);
        reg("cons", native_cons, module);
        reg("concat", native_concat, module);
        reg("first", native_first, module);
        reg("rest", native_rest, module);
        reg("get", native_get, module);
        reg("report-gc-stats", native_report_gc_stats, module);
        reg("gc-stress", native_gc_stress, module);
        // TODO: Error module
        reg("err", native_error, module);

        // TODO: async module
        reg("future", native_future, module);

        // Goroutine/scheduler module
        reg("run-scheduler", native_run_scheduler, module);

        
    }

    pub fn register_builtin_macros(&mut self, module: u32) {
        
    //     let mut symbol_table = self.symbol_table.write();

    //     let if_sym_val = ValueRef::symbol(symbol_table.intern("if"));
    //     let do_sym_val = ValueRef::symbol(symbol_table.intern("do"));
    //     let let_sym_val = ValueRef::symbol(symbol_table.intern("let"));
    //     let fn_sym_val = ValueRef::symbol(symbol_table.intern("fn"));
    //     let def_sym_val = ValueRef::symbol(symbol_table.intern("def"));

    //     let not_sym_val = ValueRef::symbol(symbol_table.intern("not"));
    //     let count_sym_val = ValueRef::symbol(symbol_table.intern("count"));
    //     let cons_sym_val = ValueRef::symbol(symbol_table.intern("cons"));
    //     let list_sym_val = ValueRef::symbol(symbol_table.intern("list"));
    //     let first_sym_val = ValueRef::symbol(symbol_table.intern("first"));
    //     let rest_sym_val = ValueRef::symbol(symbol_table.intern("rest"));
    //     let empty_sym_val = ValueRef::symbol(symbol_table.intern("empty?"));
    //     let nil_sym_val = ValueRef::symbol(symbol_table.intern("nil"));
    //     let true_sym_val = ValueRef::symbol(symbol_table.intern("true"));
    //     let eq_sym_val = ValueRef::symbol(symbol_table.intern("="));

    //     let condition_sym_val = ValueRef::symbol(symbol_table.intern("condition"));
    //     let when_sym_val = ValueRef::symbol(symbol_table.intern("when"));
    //     let unless_sym_val = ValueRef::symbol(symbol_table.intern("unless"));
    //     let forms_sym_val = ValueRef::symbol(symbol_table.intern("forms"));
    //     let and_sym_val = ValueRef::symbol(symbol_table.intern("and"));
    //     let or_sym_val = ValueRef::symbol(symbol_table.intern("or"));

    //     let condition_sym = symbol_table.intern("condition");
    //     let when_sym = symbol_table.intern("when");
    //     let unless_sym = symbol_table.intern("unless");
    //     let forms_sym = symbol_table.intern("forms");
    //     let and_sym = symbol_table.intern("and");
    //     let or_sym = symbol_table.intern("or");

    //     let body_sym_val = ValueRef::symbol(symbol_table.intern("body"));

    //     // when - expands to (if condition (do ...))
    //     let cons_expr = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![cons_sym_val, do_sym_val, body_sym_val], true),
    //     ));
    //     let when_body = vec![if_sym_val, condition_sym_val, cons_expr];
    //     let empty_env = self.alloc_env(Env::new());

    //     let when_macro = Callable {
    //         params: vec![condition_sym],
    //         is_variadic: true,
    //         body: when_body,
    //         env: empty_env,
    //         module: module,
    //     };

    //     let macro_value = self.alloc_macro(when_macro);
    //     let value = ValueRef::Heap(GcPtr::new(macro_value));
    //     self.update_module(module, when_sym, value);

    //     // unless - expands to (if (not condition) (do ...))
    //     let not_expr = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![not_sym_val, condition_sym_val], true),
    //     ));
    //     let unless_body = vec![if_sym_val, not_expr, cons_expr];
    //     let empty_env = self.alloc_env(Env::new());
    //     let unless_macro = Callable {
    //         params: vec![condition_sym],
    //         is_variadic: true,
    //         body: unless_body,
    //         env: empty_env,
    //         module: module,
    //     };
    //     let macro_value = self.alloc_macro(unless_macro);
    //     let value = ValueRef::Heap(GcPtr::new(macro_value));
    //     self.update_module(module, unless_sym, value);

    //     // and - expands to nested ifs
    //     let empty_check = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![empty_sym_val, forms_sym_val], true),
    //     ));
    //     let count_check = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![count_sym_val, forms_sym_val], true),
    //     ));
    //     let one = ValueRef::Immediate(pack_number(1.0));
    //     let single_check = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![eq_sym_val, count_check, one], true),
    //     ));
    //     let first_form = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![first_sym_val, forms_sym_val], true),
    //     ));
    //     let rest_forms = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![rest_sym_val, forms_sym_val], true),
    //     ));
    //     let recursive_and = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![and_sym_val, rest_forms], true),
    //     ));

    //     // Build the innermost if first
    //     let inner_if = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
    //         vec![if_sym_val, first_form, recursive_and, first_form],
    //         true,
    //     )));

    //     // Build the middle if
    //     let middle_if = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![if_sym_val, single_check, first_form, inner_if], true),
    //     ));

    //     // Build the outermost if (the complete expansion)
    //     let and_body = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![if_sym_val, empty_check, true_sym_val, middle_if], true),
    //     ));

    //     let empty_env = self.alloc_env(Env::new());

    //     let and_macro = Callable {
    //         params: vec![forms_sym],
    //         is_variadic: true,
    //         body: vec![and_body], // Single expansion expression
    //         env: empty_env,
    //         module: module,
    //     };

    //     let macro_value = self.alloc_macro(and_macro);
    //     let value = ValueRef::Heap(GcPtr::new(macro_value));
    //     self.update_module(module, and_sym, value);

    //     // or - expands to nested ifs: (if (empty? forms) nil (if (first forms) (first forms) (or (rest forms))))
    //     let first_form = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![first_sym_val, forms_sym_val], true),
    //     ));
    //     let rest_forms = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![rest_sym_val, forms_sym_val], true),
    //     ));
    //     let recursive_or = ValueRef::Heap(GcPtr::new(
    //         self.alloc_list_from_items_or_list(vec![or_sym_val, rest_forms], true),
    //     ));

    //     let inner_or_if = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
    //         vec![
    //             if_sym_val,
    //             first_form.clone(), // condition
    //             first_form,         // then (return the truthy value)
    //             recursive_or,       // else (recurse on rest)
    //         ],
    //         true,
    //     )));

    //     let or_body = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
    //         vec![if_sym_val, empty_check, nil_sym_val, inner_or_if],
    //         true,
    //     )));

    //     let empty_env = self.alloc_env(Env::new());

    //     let or_macro = Callable {
    //         params: vec![forms_sym],
    //         is_variadic: true,
    //         body: vec![or_body],
    //         env: empty_env,
    //         module: module,
    //     };

    //     let macro_value = self.alloc_macro(or_macro);
    //     let value = ValueRef::Heap(GcPtr::new(macro_value));
    //     self.update_module(module, or_sym, value);

        
        // cond - expands to nested ifs
        // defn - expands to (def name (fn ...))
        // -> and ->> - threading macros
    }

    pub fn register_complex_macros(&mut self, module: u32) {
        
        let mut symbol_table = self.symbol_table.write();

        // // Pre-allocate all the symbols and values we'll need
        // let if_sym_val = ValueRef::symbol(symbol_table.intern("if"));
        // let cons_sym_val = ValueRef::symbol(symbol_table.intern("cons"));
        // let list_sym_val = ValueRef::symbol(symbol_table.intern("list"));
        // let first_sym_val = ValueRef::symbol(symbol_table.intern("first"));
        // let rest_sym_val = ValueRef::symbol(symbol_table.intern("rest"));
        // let empty_sym_val = ValueRef::symbol(symbol_table.intern("empty?"));
        // let nil_sym_val = ValueRef::symbol(symbol_table.intern("nil"));
        let def_sym_val = ValueRef::symbol(symbol_table.intern("def"));
        let fn_sym_val = ValueRef::symbol(symbol_table.intern("fn"));
        // let let_sym_val = ValueRef::symbol(symbol_table.intern("let"));
        // let list_check_sym_val = ValueRef::symbol(symbol_table.intern("list?"));

        // let cond_sym_val = ValueRef::symbol(symbol_table.intern("cond"));
        // let clauses_sym_val = ValueRef::symbol(symbol_table.intern("clauses"));
        // // cond macro - recursive expansion
        // let cond_sym = symbol_table.intern("cond");
        // let clauses_sym = symbol_table.intern("clauses");

        // // Build the macro body step by step to avoid nested ctx borrows
        // let empty_check = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![empty_sym_val.clone(), clauses_sym_val.clone()], true),
        // ));
        // let first_clause = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![first_sym_val.clone(), clauses_sym_val.clone()], true),
        // ));
        // let rest_clauses = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![rest_sym_val.clone(), clauses_sym_val.clone()], true),
        // ));
        // let second_clause = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![first_sym_val.clone(), rest_clauses.clone()], true),
        // ));
        // let remaining_clauses = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![rest_sym_val.clone(), rest_clauses], true),
        // ));
        // let recursive_cond = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![cond_sym_val, remaining_clauses], true),
        // ));

        // let inner_if = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         if_sym_val.clone(),
        //         first_clause,
        //         second_clause,
        //         recursive_cond,
        //     ],
        //     true,
        // )));

        // let cond_body = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         if_sym_val.clone(),
        //         empty_check,
        //         nil_sym_val.clone(),
        //         inner_if,
        //     ],
        //     true,
        // )));

        // let empty_env = self.alloc_env(Env::new());

        // let cond_macro = Callable {
        //     params: vec![clauses_sym],
        //     is_variadic: true,
        //     body: vec![cond_body],
        //     env: empty_env,
        //     module: module,
        // };

        // let cond_macro_val = self.alloc_macro(cond_macro);
        // let value = ValueRef::Heap(GcPtr::new(cond_macro_val));
        // self.update_module(module, cond_sym, value);

        // defn macro - complete implementation with quasiquote/unquote
        let defn_sym_val = ValueRef::symbol(symbol_table.intern("defn"));
        let name_sym_val = ValueRef::symbol(symbol_table.intern("name"));
        let args_sym_val = ValueRef::symbol(symbol_table.intern("args"));
        let body_sym_val = ValueRef::symbol(symbol_table.intern("body"));

        let defn_sym = symbol_table.intern("defn");
        let name_sym = symbol_table.intern("name");
        let args_sym = symbol_table.intern("args");
        let body_sym = symbol_table.intern("body");

        // Get the required symbols for quasiquote/unquote
        let quasiquote_sym_val = ValueRef::symbol(symbol_table.intern("quasiquote"));
        let unquote_sym_val = ValueRef::symbol(symbol_table.intern("unquote"));
        let unquote_splicing_sym_val = ValueRef::symbol(symbol_table.intern("unquote-splicing"));

        // Build: `(def ~name (fn ~name ~args ~@body))
        //
        // This creates a quasiquote expression that will be evaluated to produce:
        // (def actual-name (fn actual-name actual-args body-form1 body-form2 ...))

        // Build ~name (unquote name)
        let unquote_name = ValueRef::Heap(GcPtr::new(
            self.alloc_list_from_items(vec![unquote_sym_val.clone(), name_sym_val.clone()]),
        ));

        // Build ~args (unquote args)
        let unquote_args = ValueRef::Heap(GcPtr::new(
            self.alloc_list_from_items(vec![unquote_sym_val.clone(), args_sym_val.clone()]),
        ));

        // Build ~@body (unquote-splicing body)
        let unquote_splicing_body = ValueRef::Heap(GcPtr::new(
            self.alloc_list_from_items(vec![unquote_splicing_sym_val, body_sym_val.clone()]),
        ));

        // Build the fn expression: (fn ~name ~args ~@body)
        let fn_expr = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items(
            vec![
                fn_sym_val.clone(),
                unquote_name.clone(),
                unquote_args,
                unquote_splicing_body,
            ]
        )));

        // Build the complete def expression: (def ~name (fn ~name ~args ~@body))
        let def_expr = ValueRef::Heap(GcPtr::new(
            self.alloc_list_from_items(vec![def_sym_val.clone(), unquote_name, fn_expr]),
        ));

        // Wrap the entire thing in quasiquote: `(def ~name (fn ~name ~args ~@body))
        let defn_body = ValueRef::Heap(GcPtr::new(
            self.alloc_list_from_items(vec![quasiquote_sym_val, def_expr]),
        ));

        

        // Create the macro
        let defn_macro = Macro {
            params: vec![name_sym, args_sym, body_sym],
            is_variadic: true, // body can have multiple forms
            body: vec![defn_body],
            
            module: module,
        };

        let defn_macro_val = self.alloc_macro(defn_macro);
        let value: ValueRef = ValueRef::Heap(GcPtr::new(defn_macro_val));
        self.update_module(module, defn_sym, value);

        // // -> (thread-first) macro

        // let x_sym_val = ValueRef::symbol(symbol_table.intern("x"));
        // let forms_sym_val = ValueRef::symbol(symbol_table.intern("forms"));
        // let form_sym_val = ValueRef::symbol(symbol_table.intern("form"));
        // let rest_forms_sym_val = ValueRef::symbol(symbol_table.intern("rest-forms"));
        // let threaded_sym_val = ValueRef::symbol(symbol_table.intern("threaded"));

        // let thread_first_sym_val = ValueRef::symbol(symbol_table.intern("->"));

        // let thread_first_sym = symbol_table.intern("->");
        // let x_sym = symbol_table.intern("x");
        // let forms_sym = symbol_table.intern("forms");
        // let form_sym = symbol_table.intern("form");
        // let rest_forms_sym = symbol_table.intern("rest-forms");
        // let threaded_sym = symbol_table.intern("threaded");

        // // Build all the sub-expressions step by step
        // let empty_forms_check = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![empty_sym_val.clone(), forms_sym_val.clone()], true),
        // ));
        // let first_forms = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![first_sym_val.clone(), forms_sym_val.clone()], true),
        // ));
        // let rest_forms_expr = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![rest_sym_val.clone(), forms_sym_val.clone()], true),
        // ));
        // let first_form = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![first_sym_val.clone(), form_sym_val.clone()], true),
        // ));
        // let rest_form = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![rest_sym_val.clone(), form_sym_val.clone()], true),
        // ));
        // let list_check = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![list_check_sym_val, form_sym_val.clone()], true),
        // ));

        // let cons_x_rest = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![cons_sym_val.clone(), x_sym_val.clone(), rest_form],
        //     true,
        // )));
        // let threaded_list = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![cons_sym_val.clone(), first_form, cons_x_rest], true),
        // ));
        // let simple_thread = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         list_sym_val.clone(),
        //         form_sym_val.clone(),
        //         x_sym_val.clone(),
        //     ],
        //     true,
        // )));

        // let threading_if = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![if_sym_val.clone(), list_check, threaded_list, simple_thread],
        //     true,
        // )));

        // let recursive_thread = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         thread_first_sym_val.clone(),
        //         threaded_sym_val.clone(),
        //         rest_forms_sym_val.clone(),
        //     ],
        //     true,
        // )));

        // let let_bindings = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         form_sym_val,
        //         first_forms,
        //         rest_forms_sym_val,
        //         rest_forms_expr,
        //         threaded_sym_val,
        //         threading_if,
        //     ],
        //     true,
        // )));

        // let let_body = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![let_sym_val.clone(), let_bindings, recursive_thread],
        //     true,
        // )));

        // let thread_first_body = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         if_sym_val.clone(),
        //         empty_forms_check,
        //         x_sym_val.clone(),
        //         let_body,
        //     ],
        //     true,
        // )));

        // let empty_env = self.alloc_env(Env::new());

        // let thread_first_macro = Callable {
        //     params: vec![x_sym, forms_sym],
        //     is_variadic: true,
        //     body: vec![thread_first_body],
        //     env: empty_env,
        //     module: module,
        // };

        // let thread_first_macro_val = self.alloc_macro(thread_first_macro);
        // let value = ValueRef::Heap(GcPtr::new(thread_first_macro_val));
        // self.update_module(module, thread_first_sym, value);

        // // ->> (thread-last) macro - similar but threads as last argument
        // let thread_last_sym_val = ValueRef::symbol(symbol_table.intern("->>"));
        // let thread_last_sym = symbol_table.intern("->>");

        // // ->> (thread-last) macro - similar but threads as last argument
        // let thread_last_sym_val = ValueRef::symbol(symbol_table.intern("->>"));
        // let concat_sym_val = ValueRef::symbol(symbol_table.intern("concat")); // You'll need this function

        // // Pre-build all the pieces
        // let x_list = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![list_sym_val.clone(), x_sym_val.clone()], true),
        // ));
        // let form_plus_x = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![concat_sym_val, form_sym_val.clone(), x_list], true),
        // ));
        // let list_check_form = ValueRef::Heap(GcPtr::new(
        //     self.alloc_list_from_items_or_list(vec![list_check_sym_val.clone(), form_sym_val.clone()], true),
        // ));
        // let simple_thread_last = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         list_sym_val.clone(),
        //         form_sym_val.clone(),
        //         x_sym_val.clone(),
        //     ],
        //     true,
        // )));

        // // Build the conditional for thread-last
        // let thread_last_if = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         if_sym_val.clone(),
        //         list_check_form,
        //         form_plus_x,
        //         simple_thread_last,
        //     ],
        //     true,
        // )));

        // let thread_last_recursive = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         thread_last_sym_val.clone(),
        //         threaded_sym_val.clone(),
        //         rest_forms_sym_val.clone(),
        //     ],
        //     true,
        // )));

        // let thread_last_let_bindings = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         form_sym_val.clone(),
        //         first_forms.clone(),
        //         rest_forms_sym_val.clone(),
        //         rest_forms_expr.clone(),
        //         threaded_sym_val.clone(),
        //         thread_last_if,
        //     ],
        //     true,
        // )));

        // let thread_last_let_body = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         let_sym_val.clone(),
        //         thread_last_let_bindings,
        //         thread_last_recursive,
        //     ],
        //     true,
        // )));

        // let thread_last_body = ValueRef::Heap(GcPtr::new(self.alloc_list_from_items_or_list(
        //     vec![
        //         if_sym_val,
        //         empty_forms_check.clone(),
        //         x_sym_val.clone(),
        //         thread_last_let_body,
        //     ],
        //     true,
        // )));

        // let empty_env = self.alloc_env(Env::new());

        // let thread_last_macro = Callable {
        //     params: vec![x_sym, forms_sym],
        //     is_variadic: true,
        //     body: vec![thread_last_body],
        //     env: empty_env,
        //     module: module,
        // };

        // let thread_last_macro_val = self.alloc_macro(thread_last_macro);
        // let value = ValueRef::Heap(GcPtr::new(thread_last_macro_val));
        // self.update_module(module, thread_last_sym, value);
    }
}
