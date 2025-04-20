use std::cell::RefCell;
use std::rc::Rc;
use rustyline::{Editor, Config, CompletionType, EditMode};
use rustyline::history::FileHistory;
use crate::parser::{tokenize, parse};
use crate::eval::{eval, EvalContext};
use crate::env::Env;
use crate::error::LispError;
use crate::value::BlinkValue;

const DEBUG_POS: bool = true;

pub fn start_repl() {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .bracketed_paste(true)
        .build();

    let mut rl = Editor::<(), FileHistory>::with_config(config).expect("failed to start editor");
    rl.load_history("history.txt").ok();

    let global_env = Rc::new(RefCell::new(Env::new()));
    crate::native_functions::register_builtins(&global_env);
    let mut ctx = EvalContext::new(&mut global_env.borrow_mut());

    println!("🔮 Welcome to your blink REPL. Type 'exit' to quit.");

    loop {
        match read_multiline(&mut rl) {
            Ok(line) if line.trim() == "exit" => break,
            Ok(code) => match run_line(&code, &mut ctx) {
                Ok(val) => println!("=> {}", val.borrow().value),
                Err(e) => {
                    println!("⚠️  Error: {e}");
                    if DEBUG_POS {
                        match &e {
                            LispError::TokenizerError { pos, .. }
                            | LispError::ParseError { pos, .. }
                            | LispError::UnexpectedToken { pos, .. } => {
                                println!("   [at {}]", pos);
                            }
                            LispError::EvalError { pos, .. }
                            | LispError::ArityMismatch { pos, .. }
                            | LispError::UndefinedSymbol { pos, .. } => {
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

fn read_multiline(rl: &mut Editor<(), FileHistory>) -> Result<String, rustyline::error::ReadlineError> {
    let mut lines = Vec::new();

    loop {
        let prompt = if lines.is_empty() { "λ> " } else { "... " };
        let line = rl.readline(prompt)?;

        lines.push(line);
        let code = lines.join("\n");

        match tokenize(&code).and_then(|mut toks| parse(&mut toks)) {
            Ok(_) => return Ok(code),
            Err(LispError::ParseError { message, .. }) if message.contains("Unclosed") => continue,
            Err(_) => return Ok(code), // Let the main handler display the error
        }
    }
}

fn run_line(code: &str, ctx: &mut EvalContext) -> Result<BlinkValue, LispError> {
    let mut tokens = tokenize(code)?;
    let ast = parse(&mut tokens)?;
    eval(ast, ctx)
}
