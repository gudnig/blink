use crate::collections::{ContextualValueRef, ValueContext};
use crate::env::Env;
use crate::error::{BlinkError, BlinkErrorType, ParseErrorType};

use crate::parser::{parse, tokenize, ReaderContext};
use crate::runtime::SymbolTable;
use crate::value::{ParsedValue, ParsedValueWithPos, ValueRef};
use crate::value::SharedValue;

use parking_lot::RwLock;
use rustyline::history::FileHistory;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::sync::Arc;

use crate::eval::{eval, EvalContext, EvalResult};

const DEBUG_POS: bool = true;

pub async fn start_repl() {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .bracketed_paste(true)
        .build();

    let mut rl = Editor::<(), FileHistory>::with_config(config).expect("failed to start editor");
    rl.load_history("history.txt").ok();

    let global_env = Arc::new(RwLock::new(Env::new()));

    let symbol_table = Arc::new(RwLock::new(SymbolTable::new()));

    let mut ctx = EvalContext::new(global_env.clone(), symbol_table.clone());
    {
        crate::native_functions::register_builtins(&mut ctx);
        crate::native_functions::register_builtin_macros(&mut ctx);
        crate::native_functions::register_complex_macros(&mut ctx);
    }
    crate::parser::preload_builtin_reader_macros(&mut ctx);

    println!("🔮 Welcome to your blink REPL. Type 'exit' to quit.");

    loop {
        
        let reader_macros = ctx.reader_macros.read().reader_macros.clone();
        let temp_reader_ctx = crate::parser::ReaderContext { reader_macros };
        let mut temp_reader_ctx = Arc::new(RwLock::new(temp_reader_ctx));

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
                    },
                
                    _ => {
                        match run_line(parsed, &mut ctx) {
                            EvalResult::Value(val) => 
                            {
                                match &val {
                                    ValueRef::Shared(idx) =>{
                                        let shared_arena = ctx.shared_arena.clone();
                                        let shared_arena_guard = shared_arena.read();
                                        let value = shared_arena_guard.get(*idx).unwrap();
                                        match value.as_ref() {
                                            SharedValue::Error(e) =>{
                                        
                                                println!("Error: {e}");
                                                if DEBUG_POS {
                                                    if let Some(pos) = e.pos {
                                                        println!("   [at {}]", pos);
                                                    }
                                                }        
                                            }
                                            _ => {
                                                let value_context = ValueContext::new(ctx.shared_arena.clone());
                                                let contextual_value = ContextualValueRef::new(val, value_context);
                                                println!("=> {}", contextual_value);
                                            }
                                        
                                    }}
                                    _ => {
                                        let value_context = ValueContext::new(ctx.shared_arena.clone());
                                        let contextual_value = ContextualValueRef::new(val, value_context);
                                        println!("=> {}", contextual_value);
                                    }
                                }
                            },
                
                
                            EvalResult::Suspended { mut future, mut resume } => {
                                loop {
                                    let val = future.await ;
                                    match resume(val, &mut ctx) {
                                        EvalResult::Value(v) => {
                                            let value_context = ValueContext::new(ctx.shared_arena.clone());
                                            let contextual_value = ContextualValueRef::new(v, value_context);
                                            println!("=> {}", contextual_value);
                                            break;
                                        }
                                        EvalResult::Suspended { future: next_future, resume: next_resume } => {
                                            future = next_future;
                                            resume = next_resume;
                                        }
                                    }
                                }
                            }
                        }
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

pub fn read_multiline(
    rl: &mut Editor<(), FileHistory>,
    ctx: &mut EvalContext,
) -> Result<ParsedValueWithPos, ReadError> {
    let mut lines = Vec::new();

    loop {
        let prompt = if lines.is_empty() { "λ> " } else { "... " };
        let line = rl.readline(prompt).map_err(|e| ReadError::Readline(e))?;

        lines.push(line);
        let code = lines.join("\n");

        let symbol_table =ctx.symbol_table.clone();
        let mut symbol_table_guard = symbol_table.write();
        let reader_macros = ctx.reader_macros.clone();
        let reader_macros_guard = reader_macros.write();

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
