use crate::error::{BlinkError, BlinkErrorType, ParseErrorType};
use crate::module::{Module, SerializedModuleSource};
use crate::parser::{parse, tokenize};
use crate::runtime::{BlinkVM, BlinkRuntime, EvalResult, ExecutionContext, SymbolTable};
use crate::value::{GcPtr, ParsedValue, ParsedValueWithPos, ValueRef};

use parking_lot::RwLock;
use rustyline::history::FileHistory;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::Thread;
use std::time::Duration;

const DEBUG_POS: bool = true;

// Global state for the current REPL session
use std::sync::OnceLock;
use crate::output_manager::{OutputManager, OutputSender};

static GLOBAL_OUTPUT_SENDER: OnceLock<OutputSender> = OnceLock::new();

pub fn set_global_output_sender(sender: OutputSender) {
    let _ = GLOBAL_OUTPUT_SENDER.set(sender);
}

pub fn get_global_output_sender() -> Option<&'static OutputSender> {
    GLOBAL_OUTPUT_SENDER.get()
}

pub async fn start_repl() {
    // Create output manager - this is REPL's responsibility
    let output_manager = OutputManager::new();
    let output_sender = output_manager.get_sender();

    // Set up global output context for this REPL session
    set_global_output_sender(output_sender.clone());

    let config = Config::builder()
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .bracketed_paste(true)
        .build();

    let mut rl = Editor::<(), FileHistory>::with_config(config).expect("failed to start editor");
    rl.load_history("history.txt").ok();

    let vm_arc = BlinkVM::new_arc();
    vm_arc.symbol_table.read().print_all();

    let user_module_name = vm_arc.symbol_table.write().intern("user");
    println!("user_module_name: {}", user_module_name);

    // Initialize global runtime for goroutines
    let _runtime = BlinkRuntime::init_global(vm_arc.clone(), user_module_name)
        .expect("Failed to initialize global runtime");

    let mut ctx = ExecutionContext::new(vm_arc.clone(), user_module_name);

    let user_module = Module {
        name: user_module_name,
        exports: HashMap::new(),
        source: SerializedModuleSource::Repl,
        ready: true,
        imports: HashMap::new(),
    };

    vm_arc.module_registry.write().register_module(user_module);

    println!("🔮 Welcome to your blink REPL. Type 'exit' to quit.");
    println!("💡 Tip: End a line with \\ to continue on the next line");

    loop {
        // First, flush any pending output from goroutines
        output_manager.flush_pending_output();

        match read_multiline(&mut rl, &mut ctx) {
            Ok(parsed) => {
                match parsed.value {
                    ParsedValue::Symbol(s) => {
                        let name = vm_arc.get_symbol_name(s);
                        if let Some(name) = name {
                            if name == "exit" {
                                break;
                            }
                        }
                        let current_result = run_line(parsed, vm_arc.clone(), &mut ctx);
                        match current_result {
                            Ok(val) => println!("=> {}", val),
                            Err(err) => println!("=> {}", err),
                        }
                    },
                    _ => {
                        let current_result = run_line(parsed, vm_arc.clone(), &mut ctx);
                        match current_result {
                            Ok(val) => println!("=> {}", val),
                            Err(err) => println!("=> {}", err),
                        }
                    }
                }

                // After processing the command, wait a bit for any goroutine output
                // This handles cases like (complete future "value") triggering goroutines
                let _goroutine_output_count = output_manager.process_messages_until_quiet(100).await;
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

        // Always flush output before showing the next prompt
        output_manager.flush_pending_output();
    }

    rl.save_history("history.txt").ok();
}

enum ReadError {
    Readline(rustyline::error::ReadlineError),
    Blink(BlinkError),
}

fn read_multiline(
    rl: &mut Editor<(), FileHistory>,
    ctx: &mut ExecutionContext,
) -> Result<ParsedValueWithPos, ReadError> {
    let mut lines = Vec::new();
    let mut current_input = String::new();

    loop {
        let prompt = if lines.is_empty() { "λ> " } else { "... " };
        let line = rl.readline(prompt).map_err(|e| ReadError::Readline(e))?;

        // Check if the line ends with a backslash (continuation character)
        if line.ends_with('\\') {
            // Remove the backslash and add the line content
            let line_content = line[..line.len()-1].to_string();
            current_input.push_str(&line_content);
            current_input.push('\n');
            lines.push(line_content);
            continue;
        }

        // Add the current line
        current_input.push_str(&line);
        lines.push(line);

        // Try to parse the complete input
        let code = current_input.clone();

        let mut symbol_table_guard = ctx.vm.symbol_table.write();
        let reader_macros_guard = ctx.vm.reader_macros.write();

        match tokenize(&code).and_then(|mut toks| parse(&mut toks, &reader_macros_guard, &mut *symbol_table_guard)) {
            Ok(parsed) => return Ok(parsed),
            Err(BlinkError { error_type: BlinkErrorType::Parse(ParseErrorType::UnclosedDelimiter(_message )), .. }) => {
                // Continue reading for unclosed delimiters
                current_input.push('\n');
                continue;
            },
            Err(a) => return Err(ReadError::Blink(a)),
        }
    }
}

fn run_line(
    parsed: ParsedValueWithPos,
    vm: Arc<BlinkVM>,
    ctx: &mut ExecutionContext,
) -> Result<ValueRef, BlinkError> {
    let ast = vm.alloc_parsed_value(parsed);
    ctx.compile_and_execute(ast)
}