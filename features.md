✅ Core Interpreter

✅ Tokenizer and Parser (with line/column tracking)
✅ Eval engine with closures and environment chaining
✅ Special forms:

✅ def
✅ fn
✅ if
✅ quote
✅ do
✅ let
✅ and / or
✅ try
✅ macro (compiler macros)
✅ quasiquote / unquote / unquote-splicing
✅ go (goroutines)
✅ deref (future dereferencing)


✅ User-defined functions with lexical scoping
✅ Native function registration system
✅ Fully working REPL (readline, errors, history)
✅ imp for loading .blink source modules
✅ load with multiple source types (:file, :native)
✅ mod for module declaration and context switching
✅ Support for macros (reader and compiler-level)
✅ Reader macros (', `, ~, ~@, @)
✅ Built-in numeric, string, and collection functions

✅ Arithmetic: +, -, *, /, =
✅ Logic: not
✅ Collections: list, vector, hash-map, cons, first, rest, get, map
✅ I/O: print
✅ Introspection: type-of


✅ Module system with namespace isolation
✅ Async/Future support with future, complete, deref
✅ Goroutines with go
⬜ Macro cache to avoid frequent expansion
⬜ Rich pattern matching or destructuring forms
⬜ Optional type tags or value introspection utilities
⬜ Inline documentation / docstrings for functions
⬜ Evaluation metering or cost accounting (for sandboxing)

🔌 Plugin System

✅ Plugin loading via libloading
✅ Hot-reload support for plugins
✅ compile-plugin form to build plugin from source at runtime

✅ Compiles plugins/<name> via cargo build
✅ Moves resulting .so to native/
✅ Calls native-import automatically


✅ Plugin registration with blink_register function
✅ Plugin builder API in blink_runtime crate
⬜ CLI tools to scaffold, build, and install plugins

⬜ blink new-plugin <name> to generate template
⬜ blink build-plugin to compile to .so/.dll
⬜ blink install-plugin to move built plugin to standard location


⬜ Plugin versioning & metadata support

⬜ Support plugin.blink or blink.toml for metadata
⬜ Load version and description for listing
⬜ Resolve compatible versions when loading


⬜ Optional plugin sandboxing or isolation

⬜ Prevent plugins from modifying core bindings unless whitelisted
⬜ Limit access to eval/environment manipulation
⬜ Tag plugins as trusted/untrusted


⬜ (plugin-installed? <name>) to check availability
⬜ (list-plugins) to show active or cached plugins
⬜ (plugin-info <name>) to inspect plugin metadata
⬜ Add plugin error handling for failed builds / bad symbols
⬜ Support :url values that point to:

.bl files (single-file modules)
.zip / .tar.gz packages (multi-file)
GitHub or Git repos (cloned into lib/<name>/)


⬜ :entry key for specifying entry file inside a package

🛠 CLI / Developer Workflow

✅ Socket-based REPL server (blink_socket)
✅ Language Server Protocol (LSP) implementation

✅ Text synchronization (didOpen, didChange, didClose)
✅ Diagnostics (parse errors, etc.)
✅ Completion (built-ins, symbols, special forms)
✅ Hover documentation
✅ Document symbols
✅ Go to definition


✅ Session management for multi-client support
✅ Socket client (blink_sclient)
✅ VS Code plugin

⬜ Syntax highlighting
⬜ Inline eval / hover docs
⬜ Integration with socket REPL


⬜ blink daemon for editor/linter/tooling integration
⬜ Code formatter and/or linter for .blink files

📦 Native Binaries

✅ Embeddable interpreter for apps (blink_core as library)
✅ Multiple runtime crates (blink_repl, blink_socket, etc.)
⬜ Compile to single binary with embedded code
⬜ Static native plugin registration support

⬜ Define a blink_register_<plugin>() function per plugin crate
⬜ Register native functions directly from Rust at startup
⬜ Use #[cfg(...)] or Cargo features to control plugin inclusion


⬜ Replace native-import with static calls when compiling

⬜ At compile time, skip libloading and directly invoke blink_register_<plugin>()
⬜ Allow fallback to dynamic load in dev mode



🎯 High Priority Features (Target Focus)
🌟 First-Class Environments

⬜ Serializable environments (import/export)
⬜ Environment introspection and manipulation
⬜ Environment composition and merging
⬜ Persistent environment storage

🌟 Delimited Continuations

⬜ shift and reset operators
⬜ Continuation capture and restoration
⬜ Non-local control flow
⬜ Generator/iterator support via continuations

🌟 Compilation System

⬜ REPL-driven compilation
⬜ Interpreter and compiled code interop on shared memory
⬜ Incremental compilation
⬜ Hot code swapping

🌟 Garbage Collection

✅ Shared arena for temporary values (current implementation)
⬜ Mark-and-sweep GC for long-lived objects
⬜ Generational GC
⬜ Weak references
⬜ Finalizers

🌟 Advanced DevX (Post-Continuations & Envs)

⬜ Time travel debugging
⬜ Execution history capture/replay
⬜ Interactive debugging with continuation manipulation
⬜ Live environment inspection and modification

🔥 Performance & Optimization

⬜ Tail call optimization (TCO)
⬜ Constant folding at parse time
⬜ Function call caching / inline expansion
⬜ JIT compilation hints from interpretation
⬜ Profile-guided optimization


⚠️ Error Reporting

✅ Source position tracking (line/column)
✅ Rich error types with context
✅ Error propagation in async contexts
⬜ Backtrace on errors
⬜ REPL error display improvements
⬜ Value formatter improvements for debug printing
⬜ Error recovery and suggestions


Current Architecture Status
✅ Implemented Core Systems

Value System: NaN-tagged immediates + shared arena + future GC hooks
Evaluation: Async-aware eval with suspension/resumption
Environment: Lexical scoping with parent chains
Module System: File-based modules with exports/imports
Native Integration: Plugin system with Rust FFI boundary
Async Runtime: Tokio-based goroutines and futures
Developer Tools: LSP server with rich IDE integration