# Blink Bytecode Compiler Status

## CORE LANGUAGE

- [ ] Basic compilation
  - [x] Expressions to bytecode with register allocation
- [ ] Arithmetic
  - [x] +, -, *, / opcodes compile and execute
- [ ] Comparison operators
  - [x] <, >, <=, >=, !=, =
- [ ] Special forms
  - [x] def - Global variable definition
  - [x] fn - Function definition with closures
    - [x] Variadic functions - [a b & rest] parameter syntax
  - [x] if - Conditional with else branch
  - [x] let - Local bindings with scope
  - [x] do - Sequential expression evaluation
  - [x] quote - Prevent evaluation
  - [x] macro - Macro definition
    - [x] Variadic macros - [a b & rest] parameter syntax
  - [ ] macroexpand - Macro expand at runtime
  - [x] and / or - Logical operators
  - [ ] try - Error handling (exists but may need bytecode work)
  - [x] quasiquote / unquote - Template expansion (compilation stubbed)
  - [ ] mod - Module declaration
  - [ ] imp - Module import
  - [ ] load - Load file with multiple source types
  - [ ] rmac - Remove macro
  - [x] loop / recur - Tail-recursive loops
    - [ ] loop / recur - use TCO
  - [ ] set - Update local binding or global value
  - [ ] eval - Runtime code evaluation opcode
  - [ ] apply - Function application with argument lists

- [x] Reader macros
  - [x] ' (quote), ` (quasiquote), ~ (unquote), ~@ (unquote-splicing), @ (deref)

- [x] Symbols - Symbol type distinct from keywords
- [x] Keywords - :keyword syntax implemented

- [ ] Comments
  - [x] Line comments - `;` to end of line
  - [ ] Block comments - Multiline comments, e.g. `#| ... |#`

- [ ] Unicode identifiers and strings
  - [ ] Full Unicode identifiers - Allow variable and function names to use full Unicode, including emoji and non-Latin scripts (for modern devx)
  - [x] Full Unicode strings

- [ ] Source mapping
  - [ ] Per-form source locations - Every expression retains file/line/col for precise error reporting, debugging, LSP, and mapping bytecode back to source.

- [ ] Std lib/core fns
  - [x] Collections
    - [x] vec - create vec
    - [x] list - create list
    - [x] set - create set
    - [ ] tuples - fixed-size, immutable grouping (under consideration)
    - [x] cons - prepend to list
    - [x] first - get first element
    - [x] rest - get all but first
    - [x] concat - concatenate lists/vectors
    - [x] empty? - check if collection is empty
    - [x] count - get collection length
    - [ ] map - map function over collection
    - [ ] reduce - reduce collection to single value
    - [ ] filter - filter collection by predicate
    - [ ] nth - get element at index
    - [ ] last - get last element
    - [ ] reverse - reverse collection
    - [ ] take - take first n elements
    - [ ] drop - skip first n elements
    - [ ] sort - sort collection
    - [ ] partition - split collection by predicate
  - [x] Hash-maps
    - [x] hash-map - create map
    - [ ] get - access by key
    - [ ] assoc - add/update key-value pair
    - [ ] dissoc - remove key
    - [ ] keys - get all keys
    - [ ] vals - get all values
    - [ ] contains? - check if key exists
    - [ ] merge - merge maps
  - [x] Logic & Predicates
    - [x] not - logical negation
    - [ ] nil? - check for nil
    - [ ] true? - check for true
    - [ ] false? - check for false
    - [ ] number? - check if number
    - [ ] int?/float? - type predicates if/when introduced
    - [ ] string? - check if string
    - [ ] list? - check if list
    - [ ] vector? - check if vector
    - [ ] map? - check if hash-map
    - [ ] set? - check if set
    - [ ] tuple? - check if tuple
    - [ ] fn? - check if function
    - [ ] keyword? - check if keyword
    - [ ] symbol? - check if symbol
  - [ ] Numeric operations
    - [ ] inc - increment by 1
    - [ ] dec - decrement by 1
    - [ ] mod - modulo operation
    - [ ] abs - absolute value
    - [ ] floor - round down
    - [ ] ceil - round up
    - [ ] round - round to nearest
    - [ ] min - minimum of values
    - [ ] max - maximum of values
  - [ ] String operations
    - [ ] str - string concatenation
    - [ ] subs - substring
    - [ ] str-split - split string
    - [ ] str-join - join strings
    - [ ] str-upper - uppercase
    - [ ] str-lower - lowercase
    - [ ] str-replace - replace substring
    - [ ] str-trim - trim whitespace
    - [ ] unicode-normalize - normalize unicode string
  - [x] I/O & Debugging
    - [x] print - output values
    - [x] type-of - get value type
    - [x] gc-stress - stress test GC
    - [x] report-gc-stats - GC statistics
  - [ ] File & System
    - [ ] env - Access and modify environment variables
    - [ ] args - Access program arguments
    - [ ] fs-path - Path manipulation functions
    - [ ] fs-dir - Directory walking, creation, removal

- [ ] Built in macros
  - [ ] defn - Function definition macro (stubbed in builtins.rs)
  - [ ] when - Single-branch conditional
  - [ ] cond - Multi-branch conditional (stubbed in builtins.rs)
  - [ ] -> / ->> - Threading macros

- [x] Closures - CreateClosure opcode with upvalue capture

- [ ] Function calls
  - [x] Call - Support fn, closure, macro and native fn calls
  - [ ] TailCall - Tail call optimization (opcode exists, optimization incomplete)

---

## Language features

- [ ] Advanced function features
  - [ ] Multiple arity - (fn ([x] ...) ([x y] ...))
  - [ ] Docstrings - (defn foo "doc" [x] ...)
  - [ ] Metadata - ^{:doc "..."} (defn foo ...)
  - [ ] REPL documentation integration - Show docs, arglists, signatures
  - [ ] Reflection/introspection - Query types, function signatures, env, etc.

- [ ] Mutable state
  - [ ] Atoms - (atom x), (swap! a f), (reset! a v)
  - [ ] References - Thread-safe mutable references

- [x] Custom error types - User-defined exceptions with data attached

- [ ] Error handling
  - [ ] Result map implementation - {:ok value} and {:error reason} conventions
  - [ ] Result specialised implementation - {:ok value} and {:error reason} conventions
  - [ ] with macro - (with [{:ok x} (fetch)] ...) for error pipelines
  - [ ] Error reporting improvements
    - [ ] Stack traces - Call stack on errors (currently only source position)
    - [ ] Error context - Attach context as errors propagate
    - [ ] Call frame tracking - Track function call chain
    - [ ] Source mapping - Map runtime errors to source, especially for macros
    - [ ] Better REPL errors - Cleaner error display in REPL
    - [ ] Friendly error messages - Errors provide context, hints, and "did you mean" suggestions for better developer experience
    - [ ] Error recovery - Suggest fixes for common errors
    - [ ] Assert/preconditions - (assert condition message)
    - [ ] Warning system - Compile-time/runtime warnings (unused bindings, deprecations)

- [ ] I/O & Practical features
  - [ ] File I/O - slurp, spit for reading/writing files
  - [ ] JSON handling - Parse/generate JSON
  - [ ] EDN handling - Parse/generate EDN
  - [ ] Regular expressions - Pattern matching on strings
  - [ ] Date/time - Basic temporal operations

- [ ] Serialization
  - [ ] Bytecode serialization - Save/load compiled code
  - [ ] Binary compilation - Package scripts as standalone binaries (VM + bytecode)

- [ ] Advanced features
  - [ ] Lazy sequences - Infinite/deferred computation
  - [ ] Transducers - Composable data transformation
  - [ ] Protocols/Interfaces - Define behavior contracts
  - [ ] Multimethods - Dispatch on value/type
  - [ ] Namespaced keywords - ::local and :namespace/qualified
   - [ ] Contract system (runtime contracts for values, functions, and structs)
    - [ ] Contracts are first-class values: can be passed, composed, and attached to data/functions.
    - [ ] Contracts are composable: support for primitives (predicates), and/or/or, maps, vectors, optionals, etc.
    - [ ] Contracts can be aggressively inlined and compiled to efficient checks, not just fn calls (flatten/inline contract logic at compile time for performance).
    - [ ] Contracts can carry rich, composable error messages: each contract can define a custom error message or error message function, which can incorporate both the value and the context.
    - [ ] Contract checks track error *paths*: when checking nested data (e.g., a vector of positive numbers in a struct field), the error will include the path to the failure (e.g., `"Expected positive number in withdrawals[2] in User struct"`).
    - [ ] Multiple contract failures can be collected and reported, not just the first.
    - [ ] Contracts can be attached to functions (pre/post), struct fields, or values, and checked at runtime, optionally only in dev/debug builds.
    - [ ] Libraries can leverage contracts to perform automatic validation at boundaries (e.g., API validation, FFI, form handling).
  

- [ ] Structs
  - [ ] Struct constructors - (Name. value1 value2) or (Name :field1 val1 :field2 val2)
  - [ ] Field access - (:field struct) or (.-field struct)
  - [ ] Type - (type? ) and dot notation .type optimized to bypass field accessors
  - [ ] Postfix dot notation - (struct.method args) (after structs are done)
  - [ ] Type validation - Runtime checking of struct field types

- [ ] Pattern matching
  - [ ] Match form
  - [ ] Destructuring - (let [[a b] list] ...) and (fn [{:keys [x]}] ...)

- [ ] Async model
  - [ ] Futures - (future ...) for concurrent computation
  - [ ] Goroutines - (go ...) for lightweight concurrency
  - [ ] Channels (CSP style) - Communication primitives for goroutines/futures
  - [ ] Async I/O - File/network I/O, timers, etc.
  - [ ] Timers/sleep
  - [ ] Future compilation and storage
  - [ ] Continuation capture
  - [ ] complete - complete future
  - [ ] fail - fails future
  - [ ] Delimited continuations
    - [ ] shift
    - [ ] reset
  - [ ] Async/await syntax - Syntactic sugar for working with futures/promises (possibly in future, built on current async features)

- [ ] First class env
  - [ ] Capture env

- [ ] Unicode support
  - [x] Full Unicode strings
  - [ ] String normalization

- [ ] FFI and Interop
  - [x] Rust FFI - Native function calls from bytecode
  - [ ] C FFI - Foreign function interface for C libraries
  - [ ] OS/Shell interop - Run OS commands, access shell features
  - [ ] Networking - Sockets, HTTP client/server (via plugins/libraries)
  - [ ] Plugin system - Load Rust/C plugins into runtime

- [ ] Sandboxing
  - [ ] Sandboxed eval - Run untrusted code safely (research/optional)

---

## Runtime features

- [ ] Goroutine scheduler - Task scheduling and execution loop
- [ ] Future and goroutine API - API for VM to access futures/act on them
- [ ] GC integration
  - [x] Root scanning
  - [x] Mark and sweep
  - [ ] Gencopy
- [ ] JIT compilation - Native code generation from bytecode (future)
- [ ] Native AOT compilation
- [ ] Performance profiling
  - [ ] Profiler hooks and API - Built-in hooks for performance measurement and profiling in the bytecode VM and REPL

---

## Devx

- [ ] Socket REPL - Remote bytecode compilation and execution
- [ ] LSP integration - Bytecode debugging, code intelligence, inline docs, autocomplete, warnings
  - [ ] Warning system - Unused bindings, deprecations
  - [ ] Source mapping - Error and stack trace mapping to source
- [ ] Debugger - Debugger integrated into plugin
  - [ ] Stepping
  - [ ] TimeTravel
  - [ ] Hot change values
  - [ ] Visualize program execution
- [ ] Testing framework - Unit/integration tests, assertions, runner
  - [ ] Property-based testing - QuickCheck-style random testing for properties (future)
- [ ] Formatter / linter - Code formatting, style checks
  - [ ] Idiomatic code detection - Suggest idiomatic patterns and anti-patterns in formatter/linter
- [ ] Package manager / module registry - Distribute/install libraries
- [ ] Script runner/shebang - Direct execution via shebang line
- [ ] Hot reload / code swapping - Live code changes in REPL/dev
- [ ] REPL documentation - Show docstrings, arglists, metadata
- [ ] CLI ergonomics
  - [ ] blink run / blink repl / blink fmt / blink test - Ergonomic, batteries-included CLI for dev workflows
- [ ] Editor/structural editing
  - [ ] Paredit/smart editing support - Structural editing for parens and forms, particularly in VSCode and other plugin editors
- [ ] Config api woriking with .edn files

---

## Ideas to Explore

- Macro hygiene & binding hygiene  
  Research gensyms, hygienic macros, and safe variable binding in macros.

- Advanced macro system  
  Explore syntax-quote (auto-gensym), macro stepper/visualizer for advanced devx.

- Deterministic execution and sandboxing  
  Study mechanisms for limiting resource usage, timeouts, and providing deterministic evaluation for REPL or running untrusted code.

- Streams and Lazy IO  
  Investigate lazy stream abstractions for IO (file, socket, etc.), both as a core feature and for advanced users.

- Interactive tutorials  
  Potentially add a "learn mode" or inline tutorial support in the REPL for newcomers.

---

**Implementation Priority:**  
1. Futures and goroutines and scheduling
