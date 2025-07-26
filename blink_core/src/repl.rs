
use crate::env::Env;
use crate::error::{BlinkError, BlinkErrorType, ParseErrorType};

use crate::module::{Module, SerializedModuleSource};
use crate::parser::{parse, tokenize};
use crate::runtime::{BlinkVM, SymbolTable};
use crate::value::{GcPtr, ParsedValue, ParsedValueWithPos, ValueRef};

use parking_lot::RwLock;
use rustyline::history::FileHistory;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::Thread;
use std::time::Duration;

use crate::eval::{eval, EvalContext, EvalResult};

const DEBUG_POS: bool = true;

fn get_final_value(mut result: EvalResult, ctx: &mut EvalContext) -> ValueRef {
    let final_value = loop {
        match result {
            EvalResult::Value(value) => break value,
            EvalResult::Suspended { future, resume } => {
                // Poll until ready
                let val = loop {
                    if let Some(val) = future.try_poll() {
                        break val;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                };
                result = resume(val,  ctx);
            }
        }
    };

    final_value
}

pub async fn start_repl() {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .bracketed_paste(true)
        .build();

    let mut rl = Editor::<(), FileHistory>::with_config(config).expect("failed to start editor");
    rl.load_history("history.txt").ok();

    let vm_arc = BlinkVM::new_arc();
    vm_arc.symbol_table.read().print_all();
    

    let mut ctx = EvalContext::new(vm_arc.global_env(), vm_arc.clone());
    let user_module_name = ctx.vm.symbol_table.write().intern("user");

    let user_module = Module {
        name: user_module_name,
        exports: HashMap::new(),
        source: SerializedModuleSource::Repl,
        ready: true,
        imports: HashMap::new(),
    };
    let _user_module_ref = ctx.register_module(user_module);
    ctx.current_module = user_module_name;
    

    println!("Global env: {}", vm_arc.global_env());
    


    println!("🔮 Welcome to your blink REPL. Type 'exit' to quit.");

    loop {
        
        match read_multiline(&mut rl, &mut ctx) {
            
            Ok(parsed) => {
                match parsed.value {
                    ParsedValue::Symbol(s) => {
                        let name = ctx.get_symbol_name_from_id(s);
                        if let Some(name) = name {
                            if name == "exit" {
                                break;
                            }
                        }
                        let current_result = run_line(parsed, &mut ctx);
                        let final_value = get_final_value(current_result, &mut ctx);
                        println!("=> {}", final_value);
                    },
                
                    _ => {
                        let current_result = run_line(parsed, &mut ctx);
                        let final_value = get_final_value(current_result, &mut ctx);
                        

                        println!("=> {}", final_value);
                    }
                }
            },
            
            Err(e) => {
                match e {
                    ReadError::Readline(e) => println!("Error: {e}"),
                    ReadError::Blink(e) => {
                        println!("Error: {}", e);

                        if DEBUG_POS {
                            if let Some(pos) = e.pos {
                                println!("   [at {}]", pos);
                            }
                        }        
                    },
                }
            }
        }
    }

    rl.save_history("history.txt").ok();
}

enum ReadError {
    Readline(rustyline::error::ReadlineError),
    Blink(BlinkError),
}

fn read_multiline(
    rl: &mut Editor<(), FileHistory>,
    ctx: &mut EvalContext,
) -> Result<ParsedValueWithPos, ReadError> {
    let mut lines = Vec::new();

    loop {
        let prompt = if lines.is_empty() { "λ> " } else { "... " };
        let line = rl.readline(prompt).map_err(|e| ReadError::Readline(e))?;

        lines.push(line);
        let code = lines.join("\n");

        
        let mut symbol_table_guard = ctx.vm.symbol_table.write();
        
        let reader_macros_guard = ctx.vm.reader_macros.write();

        match tokenize(&code).and_then(|mut toks| parse(&mut toks, &reader_macros_guard, &mut *symbol_table_guard)) {
            Ok(parsed) => return Ok(parsed),
            Err(BlinkError { error_type: BlinkErrorType::Parse(ParseErrorType::UnclosedDelimiter(_message )), .. }) => continue,
            Err(a) => return Err(ReadError::Blink(a)), // Let the main handler display the error
        }
    }
}

fn run_line(
    parsed: ParsedValueWithPos,
    ctx: &mut EvalContext,
) -> EvalResult {

    
    let ast =  ctx.alloc_parsed_value(parsed);
    eval(ast, ctx)
}
