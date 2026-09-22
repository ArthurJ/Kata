# Addendum — Design Rationale

Kata is iteration 5 of a development process — but it is the first public version, so there are no comparisons with previous versions. The design decisions below are not the language spec; they are a record of *why* certain choices were made. The spec describes *what* the language does; this addendum explains *the why*.

## Why is there no `if`?

`match`, `lambda` clauses, and guards cover the cases where `if-else` would be used. We do not have the words `if`/`else`, but the corresponding semantics are present in the language.

The exhaustiveness of `match` is a second reason. If `if` existed, `else` would have to be mandatory to preserve exhaustiveness — an unnecessary inconvenience given that `match` already handles this in a more structured way.

Finally, one of the language's goals is less nested code. `if-else` encourages nesting. We do not yet know whether we have achieved a satisfactory result on this front — only usage will tell.

## Why can't functions execute actions?

A strong initial inspiration was functional languages, but in general I did not find satisfactory examples of separation between the pure and impure worlds. I really like Haskell, for example, but I could not understand monads well enough to explain them to another person.

The explicit separation between pure functions and actions has concrete advantages: maintenance and testing of program logic become easier, and the optimization possibilities for pure functions are a real advantage.

## Why is `Result` a normal enum and `|` is defined over any enum?

One of my goals is to separate operators from functions in the language. I was not 100% successful on this point, but I sought to implement the language in itself whenever possible (*eat your own dog food*). The enum system seemed good enough to support `Result`, `Optional`, and `Boolean` without special treatment.

Offering `|` (fallback) to all enums seemed like an excellent opportunity to make enums more useful and the language less verbose and more practical. Special-casing `Result` would give it a privilege that has no reason to exist.

## Why does `var` only exist in actions?

The language's immutability is relevant for several reasons, but I understand that sometimes it is necessary or useful to allow mutation. Actions are the safe space where this makes sense — they serve to model behavior, and that demands mutation.

At the top level, `var` would be an invitation to bugs. In pure functions, it would be a risk to purity — a given input should always produce the same output, without side effects.

Furthermore, although the language does not enforce it, I hope that `lambda` clauses are generally one-liners, or use guards, or matches. In all these cases, `var` does not fit.

## Why channels instead of async/await?

The initial inspiration was Python's `async` system — a model I consider simple and elegant. Channels in Kata are not all synchronous: rendezvous blocks until the receiver synchronizes, but buffered queues with backpressure and fire-and-forget broadcasts are asynchronous. What is synchronous is the *scheduler*: single-threaded, cooperative, with fibers (wasmtime-fiber, 1MB stack each). `send`/`recv` that cannot complete suspend the fiber; the scheduler does a wake pass checking availability without consuming, and wakes the fiber when it can proceed. Deadlock is detected when all fibers are blocked with no possible progress.

`select` (multiplexing) combines channels, file handles, and sockets in a single atomic suspension, with optional timeout. Interprocess concurrency is supported via fork + Unix pipes (IPC channels), and `select` is already prepared for threads in the future. The choice of channels over async/await avoids the complexity of coloring functions as async or sync, and keeps reasoning about concurrency at the level of channel operations, not at the level of each call.

## Why are refined types not new runtime types?

Refined types are aliases with predicates validated at compile-time. At runtime, a `PositiveInt` is literally an `Int` — same bits, same Cranelift type, no wrapping, no tag, no overhead. Predicate validation happens in the smart constructor (which returns `Result`) or in literal ascriptions (validated at compile-time, with no runtime cost). The decision not to create new runtime types avoids the cost of boxing/unboxing and keeps codegen simple — the compiler only needs to resolve the chain of aliases down to the base primitive.

The trade-off is that there is no type safety at runtime: if you escape the type system (e.g., via FFI), nothing prevents a negative `Int` from being treated as a `PositiveInt`. This is acceptable because validation is in the constructors, which are the only way to build refined values within the language.

## Why is shadowing prohibited?

Smaller error surface in Kata code, and greater ease in compiler implementation.

## Why are NUM and ORD separate typeclasses?

Not every number is orderable — `Complex` is the example. There is no semantic sense in forcing both together.

## Why prefix notation?

Prefix notation eliminates the need for precedence rules between operators. In infix notation, the compiler needs to know that `*` comes before `+`, that parentheses group, and so on — a precedence table that grows with each new operator. In prefix notation, function application is always "callee followed by arguments," whether the callee is `+` or `soma_valores`. The parser does not treat arithmetic operators specially: `+`, `-`, `*`, `<`, `>`, `=` are all ordinary identifiers, lexed and dispatched like any other function name.

This also simplifies the compiler implementation. Without precedence, the parser is more straightforward — there is no ambiguity to resolve. And visually, when code is broken into small enough pieces, prefix notation is less dense because it dispenses with the organizers (parentheses, commas) that infix notation requires for disambiguation.

## Why arenas instead of a garbage collector?

Kata has no tracing GC and no borrow checker. Memory management is via arenas: each fiber has its own arena (bump allocator), where local data is allocated in O(1) and freed in O(1) when the fiber ends — there is no individual deallocation. Data that needs to outlive the fiber (values returned to the caller, values sent over channels) is allocated in the caller's arena or in the root arena, the latter with individual deallocation via reference counting for closures with capture.

The compiler performs escape analysis to determine where to allocate each value: local to the fiber, escaping to the caller, or escaping to another fiber via a channel. This selection is static, decided at compile-time. The result is that purely local data — the overwhelming majority of cases in pure functions — has maximum cache locality and zero atomic overhead, without the programmer needing to think about ownership.

The model works because the scheduler is structured concurrency: a fiber is only destroyed when it completes *and* all its children have completed. This guarantees that the parent's caller_arena is alive when a child returns a value or sends over a channel, and that siblings sharing the parent's arena exchange valid values. Without this invariant, the model would break — values sent over a channel could be use-after-free if the sender fiber died before the receiver consumed them.

## Why is there no try/catch?

Invisible exceptions break functional purity. A function that can throw an exception at any moment, without declaring it, is not truly pure — the caller does not know it is subject to a flow deviation not expressed in the signature. The absence of try/catch forces explicit error handling in the type system: fallible operations return `Result`, and the compiler requires the programmer to handle both branches. The `?` operator in actions and the `|` operator in pure functions are the propagation mechanisms — but failure is always visible in the signature.

## Why no dynamic reflection?

The prohibition of dynamic reflection — invocation by string, `eval`, dispatch based on function name at runtime — allows the compiler to build a complete and deterministic call graph from the entry point. Any function, type, interface, or implementation not in the dependency graph is dead code, extirpated by the tree-shaker before codegen. The final binary does not load unused stdlib.

Kata has two introspection mechanisms, both compile-time. `type!()` queries the type of an expression, returning the nominal name as `Text` — resolved in the monomorphizer from the static type, with no edge in the call graph. Reflection variables (`_name`, `_arity`, `_types`, `_return_type`, `_is_action`) are made available in the body of directives and expose metadata of the decorated function; static ones are replaced by literals during desugaring, and dynamic ones (`_args`, `_return`) are synthesized from the function's parameters and return. Neither mechanism queries runtime, invokes by string, or interferes with tree-shaking.

## Why are naming conventions mandatory?

The lexer and parser use identifier capitalization for disambiguation. The convention is not stylistic — it is structural. The parser needs to distinguish a type name from a function name in ambiguous positions, and capitalization resolves this without redundant annotations. Violation constitutes a fatal compilation error, not a warning.

## Why does the compiler have no builtins?

The "no builtins" principle means that arithmetic, comparison, strings, collections, and I/O are all defined in the stdlib in Kata code via `@ffi` — there is no special treatment for `+`, `-`, `<`, `=` in the parser, typeck, or codegen. They are ordinary identifiers pointing to functions in `kata-rt`. The compiler only knows the FFI symbol catalog and the representation mapping strings (`"i64"`, `"f64"`, `"kata_rt_string"`).

We were not 100% successful on this point. `map`, `filter`, and `fold` are intercepted by typeck before normal dispatch — the compiler recognizes them by name, extracts the collection's element type, infers the callback with a hint, and produces dedicated TAST nodes. This is necessary for stream fusion and for desugaring standalone operators as callbacks (`map + [1 2 3]` needs to transform `+` into a synthetic lambda). This is the point where the compiler still knows specific language names.