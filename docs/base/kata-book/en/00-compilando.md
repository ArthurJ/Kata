# Chapter 0 — Compiling Kata

Before writing any program, you need the `kata` binary. This chapter shows how to compile the compiler from source and which commands are available.

## Prerequisites

- **Rust 1.85 or higher** — the Kata compiler is written in Rust and uses the 2024 edition. Install via [rustup](https://rustup.rs) or your system's package manager.
- **C linker (`cc`)** — needed to generate native executables. On Linux, install `gcc` or `clang`. On macOS, Xcode Command Line Tools provides `clang`.

Verify that Rust is available:

```bash
rustc --version
```

```
rustc 1.85.0
```

## Compiling

Clone the repository and build in release mode:

```bash
git clone https://github.com/arthurjulia/kata.git
cd kata
cargo build --release
```

The binary is at `target/release/kata`. To avoid typing the full path, add it to your `PATH`:

```bash
export PATH="$PWD/target/release:$PATH"
```

Verify:

```bash
kata --version
```

```
kata 0.1.0
```

## Available commands

The `kata` binary has several subcommands. The most commonly used:

### `kata run` — run a file

Compiles and executes a `.kata` file immediately:

```bash
kata run examples/functions/fatorial.kata
```

```
120
```

### `kata eval` — evaluate an expression

Evaluates an expression directly from the command line, without creating a file:

```bash
kata eval '+ 1 2'
```

```
3
```

Useful for quickly testing an expression.

Both `kata run` and `kata eval` accept the `--emit-ir` flag to print the Cranelift IR before executing. Useful for inspecting the generated code — optimizations, TCO, stream fusion:

```bash
kata eval '+ 1 2' --emit-ir
```

### `kata repl` — interactive REPL

Starts the REPL to experiment with expressions interactively. Detailed in [Chapter 15](15-repl.md).

### `kata build` — compile to native executable

Generates a standalone executable from a `.kata` file:

```bash
kata build examples/functions/fatorial.kata
./fatorial
```

By default, the runtime is statically linked. Use `--dynamic` to link dynamically (smaller binary, but depends on the lib at runtime):

```bash
kata build examples/functions/fatorial.kata --dynamic
```

### `kata test` — run tests

Discovers and runs tests in a file or directory. There are two types:

- **`@test`** — tests annotated with the `@test` directive in actions
- **Doctests** — executable examples in `#{ }#` multiline comments with the `>>> ` marker

```bash
kata test examples/directives/assertions.kata
```

Use `--filter` to run only `@test` tests that contain a substring:

```bash
kata test examples/ --filter "fib"
```

Doctests always run (not affected by `--filter`). See [Chapter 16](16-doctests.md) for details on doctests.

### `kata lex` and `kata parse` — compiler inspection

Show the tokens and AST of a file, respectively. Useful for understanding how the compiler sees your code:

```bash
kata lex examples/basics/hello_action.kata
kata parse examples/basics/hello_action.kata
```

### `kata lsp` — language server

Starts the LSP (Language Server Protocol) server on stdio. Editors like VS Code and Neovim can connect to get autocomplete, diagnostics, and type hover. The appendix on platforms ([Appendix](17-plataformas-limitacoes.md)) has more on where the LSP works.

## Where to find examples

The `examples/` directory in the repository has dozens of `.kata` programs that exercise each feature of the language:

```bash
ls examples/
```

Some notable ones:

| File | What it demonstrates |
|---------|-------------|
| `fatorial.kata` | Recursion with tail-call optimization |
| `fizzbuzz.kata` | Pattern matching + guards |
| `quicksort.kata` | HOFs + List |
| `select_queue.kata` | CSP: channels + select + timeout |
| `refined_types.kata` | Refined types + smart constructors |

Explore freely — every example is runnable with `kata run`.

## Syntax highlighting

The repository includes a TextMate bundle at the root for syntax highlighting in editors:

```
Kata.tmbundle/
├── info.plist                  — bundle metadata
└── Syntaxes/
    └── Kata.tmLanguage.json    — TextMate grammar
```

### VS Code

VS Code uses the `.tmLanguage.json` file directly. To install manually, copy `Kata.tmLanguage.json` to your VS Code extensions folder, or use an extension that loads custom TextMate grammars.

### JetBrains (IntelliJ, CLion, RustRover, etc.)

JetBrains IDEs read the entire `.tmbundle` bundle. Import via *Settings → Editor → TextMate Bundles → +* and select the `Kata.tmbundle/` folder at the project root. The editor will then recognize `.kata` files with the bundle's grammar.

### LSP

In addition to the highlighter, the `kata` binary includes an LSP server (`kata lsp`) that provides diagnostics, type hover, and autocomplete for editors that support the Language Server Protocol. See [Chapter 15](15-repl.md) for more details on editor integration.

## Next chapter

With the binary compiled and the commands at hand, [Chapter 1](01-ola-kata.md) shows your first program in Kata.