# Blink

Blink is a lightweight Lisp dialect written in Rust, focused on power and extensibility, with a powerful REPL* and great developer experience. It supports closures, macros*, dynamic plugin loading, and native Rust interop—all in a simple, expressive syntax.  
*coming soon

## Features

- Closures with lexical scoping
- Special forms: `def`, `fn`, `if`, `quote`, `do`, `let`, `and`, `or`, `try`
- Native function registration (e.g. `+`, `map`, `reduce`)
- Plugin loading via `.so` libraries using `native-import`
- Line and column tracking for rich error reporting
- Readline-based REPL with multiline input and history
- Macros and module system (in progress)

## Getting Started

### 1. Clone & Build

```bash
git clone https://github.com/yourname/blink.git
cd blink
cargo build --release
```

### 2. Run the REPL

```bash
cargo run
```

You’ll see:

```
Welcome to your blink REPL. Type 'exit' to quit.
λ>
```

Example session:

```lisp
λ> (def x 42)
λ> (+ x 8)
=> 50
```

## Plugin System

### Build a Plugin

Each plugin lives in `plugins/<name>` and is a standalone Rust crate:

```bash
cd plugins/greeter
cargo build --release
```

This produces `target/release/libgreeter.so`.

### Install the Plugin

Move the `.so` into the `native/` directory:

```bash
mv target/release/libgreeter.so ../../native/
```

Then in Blink:

```lisp
λ> (native-import "greeter")
=> "native-imported: greeter"
```

## Project Structure

```
src/
├── main.rs              # Entry point (starts REPL)
├── repl.rs              # REPL loop
├── parser.rs            # Tokenizer & parser
├── eval.rs              # Evaluation engine
├── native_functions.rs  # Built-in Rust functions
├── env.rs               # Lexical environment
├── value.rs             # Lisp value types
├── error.rs             # Error types and reporting
```

## License

Licensed under the [MIT License](LICENSE).

## Roadmap

See `features.md` for the full roadmap. Planned features include:

- Macros and code quoting
- Module system with namespaces
- Plugin scaffolding and install tools
- Async/actor-aware evaluation model
- Web DSL and server bindings
