use crate::env::Env;
use crate::error::{BlinkError, BlinkErrorType, LispError, ParseErrorType};

use crate::parser::{parse, preload_builtin_reader_macros, tokenize, ReaderContext};
use crate::value_ref::ValueRef;

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

    {
        crate::native_functions::register_builtins(&global_env);
    }

    let mut ctx = EvalContext::new(global_env.clone());
    preload_builtin_reader_macros(&mut ctx);

    println!("🔮 Welcome to your blink REPL. Type 'exit' to quit.");

    loop {
        
        let reader_macros = ctx.reader_macros.read().reader_macros.clone();
        let mut temp_reader_ctx = crate::parser::ReaderContext { reader_macros };
        let mut temp_reader_ctx = Arc::new(RwLock::new(temp_reader_ctx));

        match read_multiline(&mut rl, &mut temp_reader_ctx) {
            Ok(line) if line.trim() == "exit" => break,
            Ok(code) => match run_line(&code, &mut ctx, &mut temp_reader_ctx) {
                        EvalResult::Value(val) => 
                        {
                            match &val {
                                ValueRef::Shared(idx) =>{
                                    let value = ctx.shared_arena.read().get(*idx).unwrap();
                                    match value {
                                        Value::Error(e) =>{
                                    
                                            println!("Error: {e}");
                                            if DEBUG_POS {
                                                if let Some(pos) = e.pos {
                                                    println!("   [at {}]", pos);
                                                }
                                            }        
                                        }
                                    
                                }}
                                _ => {
                                    println!("=> {}", val);
                                }
                            }
                        },
            
            
                        EvalResult::Suspended { mut future, mut resume } => {
                            loop {
                                let val = future.await ;
                                match resume(val, &mut ctx) {
                                    EvalResult::Value(v) => {
                                        println!("=> {}", v.read().value);
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
            Err(e) => println!("Error: {e}"),
        }
    }

    rl.save_history("history.txt").ok();
}

pub fn read_multiline(
    rl: &mut Editor<(), FileHistory>,
    rcx: &mut Arc<RwLock<ReaderContext>>,
) -> Result<String, rustyline::error::ReadlineError> {
    let mut lines = Vec::new();

    loop {
        let prompt = if lines.is_empty() { "λ> " } else { "... " };
        let line = rl.readline(prompt)?;

        lines.push(line);
        let code = lines.join("\n");

        match tokenize(&code).and_then(|mut toks| parse(&mut toks, rcx)) {
            Ok(_) => return Ok(code),
            Err(BlinkError { error_type: BlinkErrorType::Parse(ParseErrorType::UnclosedDelimiter(_message )), .. }) => continue,
            Err(_) => return Ok(code), // Let the main handler display the error
        }
    }
}

fn run_line(
    code: &str,
    ctx: &mut EvalContext,
    reader_macros: &mut Arc<RwLock<ReaderContext>>,
) -> EvalResult {
    let mut tokens = match tokenize(code) {
        Ok(tokens) => tokens,
        Err(e) => return EvalResult::Value(e.into_blink_value()),
    };
    
    let ast = match parse(&mut tokens, reader_macros) {
        Ok(ast) => ast,
        Err(e) => return EvalResult::Value(e.into_blink_value()),
    };
    eval(ast, ctx)
}
