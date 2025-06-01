# Blink Project Features

This document tracks all current and planned features for the Blink Lisp interpreter in a unified checklist format. Use this as a living roadmap.

---

## ✅ Core Interpreter

- ✅ Tokenizer and Parser (with line/column tracking)
- ✅ Eval engine with closures and environment chaining
- ✅ Special forms:
  - ✅ `def`
  - ✅ `fn`
  - ✅ `if`
  - ✅ `quote`
  - ✅ `do`
  - ✅ `let`
  - ✅ `and` / `or`
  - ✅ `try`
- ✅ User-defined functions with lexical scoping
- ✅ Native function registration system
- ✅ Fully working REPL (readline, errors, history)
- ✅ `import` for loading `.bl` source modules
- ✅ `native-import` for loading native Rust plugins (`.so`/`.dll`)
- ✅ Support for macros (reader and compiler-level)
- ⬜ Macro cache to avoid frequent expansion
- ⬜ Built-in numeric, string, and collection functions (core lib)
- ✅ Module system with namespace isolation
- ⬜ Rich pattern matching or destructuring forms
- ⬜ Optional type tags or value introspection utilities
- ⬜ Inline documentation / docstrings for functions
- ⬜ Evaluation metering or cost accounting (for sandboxing)

## 🔌 Plugin System

- ✅ Plugin loading via `libloading`
- ✅ Plugins managed with `HashMap<String, Rc<Library>>`
- ✅ Hot-reload support for plugins
- ⬜ CLI tools to scaffold, build, and install plugins
  - ⬜ `blink new-plugin <name>` to generate template
  - ⬜ `blink build-plugin` to compile to `.so`/`.dll`
  - ⬜ `blink install-plugin` to move built plugin to standard location
- ⬜ Plugin versioning & metadata support
  - ⬜ Support `plugin.blink` or `blink.toml` for metadata
  - ⬜ Load version and description for listing
  - ⬜ Resolve compatible versions when loading
- ⬜ Optional plugin sandboxing or isolation
  - ⬜ Prevent plugins from modifying core bindings unless whitelisted
  - ⬜ Limit access to eval/environment manipulation
  - ⬜ Tag plugins as trusted/untrusted
- ⬜ (compile-plugin <name>) form to build plugin from source at runtime
  - ✅ Compiles plugins/<name> via cargo build
  - ✅ Moves resulting .so to native/
  - ✅ Calls native-import automatically
- ⬜ (plugin-installed? <name>) to check availability
- ⬜ (list-plugins) to show active or cached plugins
- ⬜ (plugin-info <name>) to inspect plugin metadata
- ⬜ Add plugin error handling for failed builds / bad symbols
- ⬜ Support `:url` values that point to:
  - `.bl` files (single-file modules)
  - `.zip` / `.tar.gz` packages (multi-file)
  - GitHub or Git repos (cloned into `lib/<name>/`)
- ⬜ Optional `:entry` key for specifying entry file inside a package

## 🛠 CLI / Developer Workflow

- ⬜ Socket-based REPL (for editor tooling and remote eval)
- ⬜ VS Code plugin
  - ⬜ Syntax highlighting
  - ⬜ Inline eval / hover docs
  - ⬜ Integration with socket REPL
- ⬜ Language server protocol (LSP) integration (optional fallback if not using socket)
- ⬜ `blink daemon` for editor/linter/tooling integration
- ⬜ Code formatter and/or linter for `.blink` files

## 📦 Native Binaries

- ✅ Embeddable interpreter for apps
- ⬜ Compile to single binary with embedded code
- ⬜ Static native plugin registration support
  - ⬜ Define a `blink_register_<plugin>()` function per plugin crate
  - ⬜ Register native functions directly from Rust at startup
  - ⬜ Use `#[cfg(...)]` or Cargo features to control plugin inclusion
- ⬜ Replace `native-import` with static calls when compiling
  - ⬜ At compile time, skip `libloading` and directly invoke `blink_register_<plugin>()`
  - ⬜ Allow fallback to dynamic load in dev mode

## 🔥 Performance & Optimization

- ⬜ Tail call optimization (TCO)
- ⬜ Constant folding at parse time
- ⬜ Function call caching / inline expansion

## 🌐 Web App Support

- ⬜ HTTP server bindings (Axum/Hyper)
- ⬜ Blink DSL for routing
- ⬜ Request/response helpers (`:json`, `:status`, etc.)
- ⬜ Templating macros / HTML DSL
- ⬜ Hot-reloading dev server (`blink serve`)
- ⬜ State management / component rendering support

## 🚀 Transpilation & Compilation

- ⬜ Blink-to-Rust transpiler (`blinkc`)
- ⬜ Blink-to-JavaScript transpiler (for browser support)
- ⬜ Blink runtime in WASM

## ⚠️ Error Reporting

- ✅ Source position tracking (line/column)
- ⬜ Backtrace on errors
- ⬜ REPL error display improvements
- ⬜ Value formatter improvements for debug printing

## 🧪 Experimental Ideas

- ⬜ Macro profiler / debugger
- ⬜ Async / actor model integration
- ⬜ Type hints / inference for optimization
- ⬜ WASM interop or JS FFI layer
