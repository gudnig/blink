// use proc_macro::TokenStream;
// use quote::quote;
// use syn::{parse_macro_input, ItemFn, FnArg, ReturnType, Type};


// // TODO move to own crate
// #[proc_macro_attribute]
// pub fn blink_export(_attr: TokenStream, item: TokenStream) -> TokenStream {
//     let input_fn = parse_macro_input!(item as ItemFn);
//     let fn_name = &input_fn.sig.ident;
//     let fn_name_str = fn_name.to_string();
    
//     // Extract parameters and generate boundary calls
//     let mut param_extractions = Vec::new();
//     let mut param_names = Vec::new();
    
//     for (i, input) in input_fn.sig.inputs.iter().enumerate() {
//         if let FnArg::Typed(pat_type) = input {
//             if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
//                 let param_name = &pat_ident.ident;
//                 param_names.push(param_name);
                
//                 let extraction = match &*pat_type.ty {
//                     Type::Path(type_path) if type_path.path.is_ident("String") => {
//                         quote! {
//                             let #param_name = boundary.extract_string(&args[#i])
//                                 .map_err(|e| BlinkError::eval(format!("Arg {}: {}", #i, e)))?;
//                         }
//                     },
//                     Type::Path(type_path) if type_path.path.is_ident("f64") => {
//                         quote! {
//                             let #param_name = boundary.extract_number(&args[#i])
//                                 .map_err(|e| BlinkError::eval(format!("Arg {}: {}", #i, e)))?;
//                         }
//                     },
//                     Type::Path(type_path) if type_path.path.is_ident("bool") => {
//                         quote! {
//                             let #param_name = boundary.extract_bool(&args[#i])
//                                 .map_err(|e| BlinkError::eval(format!("Arg {}: {}", #i, e)))?;
//                         }
//                     },
//                     _ => {
//                         return syn::Error::new_spanned(
//                             pat_type,
//                             "Unsupported parameter type. Use String, f64, or bool"
//                         ).to_compile_error().into();
//                     }
//                 };
//                 param_extractions.push(extraction);
//             }
//         }
//     }
    
//     // Generate return allocation
//     let return_allocation = match &input_fn.sig.output {
//         ReturnType::Type(_, ty) => {
//             match &**ty {
//                 Type::Path(type_path) if type_path.path.is_ident("String") => {
//                     quote! { boundary.alloc_string(result) }
//                 },
//                 Type::Path(type_path) if type_path.path.is_ident("f64") => {
//                     quote! { boundary.alloc_number(result) }
//                 },
//                 Type::Path(type_path) if type_path.path.is_ident("bool") => {
//                     quote! { boundary.alloc_bool(result) }
//                 },
//                 Type::Tuple(tuple) if tuple.elems.is_empty() => {
//                     // Unit type () -> nil
//                     quote! { blink_runtime::values::nil() }
//                 },
//                 _ => {
//                     return syn::Error::new_spanned(
//                         ty,
//                         "Unsupported return type. Use String, f64, bool, or ()"
//                     ).to_compile_error().into();
//                 }
//             }
//         },
//         ReturnType::Default => quote! { blink_runtime::values::nil() },
//     };
    
//     let param_count = param_names.len();
//     let original_fn = &input_fn;
    
//     let expanded = quote! {
//         #original_fn
        
//         paste::paste! {
//             pub fn [<blink_wrapper_ #fn_name>](args: Vec<BlinkValue>) -> Result<BlinkValue, BlinkError> {
//                 use blink_runtime::boundary::{ValueBoundary, CurrentBoundary};
                
//                 // Arity check
//                 if args.len() != #param_count {
//                     return Err(BlinkError::arity(#param_count, args.len(), #fn_name_str));
//                 }
                
//                 let mut boundary = CurrentBoundary;
                
//                 // Extract arguments (copy out of GC)
//                 #(#param_extractions)*
                
//                 // Call the pure Rust function
//                 let result = #fn_name(#(#param_names),*);
                
//                 // Allocate result (copy into GC)
//                 Ok(#return_allocation)
//             }
            
//             pub fn [<register_ #fn_name>](env: &mut blink_runtime::Env) -> String {
//                 blink_runtime::register_fn(env, #fn_name_str, [<blink_wrapper_ #fn_name>])
//             }
//         }
//     };
    
//     TokenStream::from(expanded)
// }