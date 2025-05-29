use crate::env::Env;
use crate::error::LispError;

use crate::parser::{parse, preload_builtin_reader_macros, tokenize, ReaderContext};
use crate::value::BlinkValue;
use parking_lot::RwLock;
use rustyline::history::FileHistory;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::sync::Arc;

use crate::eval::{eval, EvalContext};

const DEBUG_POS: bool = true;

pub fn start_repl() {
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
        // 🌟 Clone the reader macros once for this REPL iteration
        let reader_macros = ctx.reader_macros.read().reader_macros.clone();
        let mut temp_reader_ctx = crate::parser::ReaderContext { reader_macros };
        let mut temp_reader_ctx = Arc::new(RwLock::new(temp_reader_ctx));

        match read_multiline(&mut rl, &mut temp_reader_ctx) {
            Ok(line) if line.trim() == "exit" => break,
            Ok(code) => match run_line(&code, &mut ctx, &mut temp_reader_ctx) {
                Ok(val) => println!("=> {}", val.read().value),
                Err(e) => {
                    println!("Error: {e}");
                    if DEBUG_POS {
                        match &e {
                            LispError::TokenizerError { pos, .. }
                            
                                                    | LispError::UnexpectedToken { pos, .. } => {
                                                        println!("   [at {}]", pos);
                                                    },
                            | LispError::ParseError { pos, .. } => {
                                                        println!("   [at {}]", pos);
                                                    }
                            LispError::EvalError { pos, .. }
                                                    | LispError::ArityMismatch { pos, .. }
                                                    | LispError::UndefinedSymbol { pos, .. } => {
                                                        if let Some(pos) = pos {
                                                            println!("   [at {}]", pos);
                                                        }
                                                    }
                            LispError::ModuleError {  pos, .. } => {
                                if let Some(pos) = pos {
                                    println!("   [at {}]", pos);
                                }
                            }
                        }
                    }
                }
            },
            Err(_) => break,
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
            Err(LispError::ParseError { message, .. }) if message.contains("Unclosed") => continue,
            Err(_) => return Ok(code), // Let the main handler display the error
        }
    }
}

fn run_line(
    code: &str,
    ctx: &mut EvalContext,
    reader_macros: &mut Arc<RwLock<ReaderContext>>,
) -> Result<BlinkValue, LispError> {
    let mut tokens = tokenize(code)?;
    let ast = parse(&mut tokens, reader_macros)?;
    eval(ast, ctx)
}
