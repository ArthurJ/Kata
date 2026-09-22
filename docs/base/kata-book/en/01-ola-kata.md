# Chapter 1 — Hello, Kata

Kata is a functional language with prefix notation, algebraic types, and cooperative concurrency via channels. This chapter shows how to install, run your first program, and explore the REPL.

## Installation

If you haven't compiled the `kata` binary yet, see [Chapter 0 — Compiling Kata](00-compilando.md) for build instructions. After installing, verify:

```bash
kata --version
```

```
kata 0.1.0
```

Kata runs on Linux and macOS natively. There is an experimental port for Windows — see the [Appendix — Platforms and Limitations](17-plataformas-limitacoes.md) for details.

## Your first program

Kata uses prefix notation: the function comes before the arguments. To add two numbers:

```kata
+ 1 2
```

Save it in a file `hello.kata` and run:

```bash
kata run hello.kata
```

```
3
```

No `action main`, no boilerplate. The last expression in the file is the program's entry point.

## Printing to the screen

To display values on the screen, use `echo!`:

```kata
echo!("olá, mundo")
```

```
olá, mundo
```

The `!` at the end indicates that `echo!` is an *action* — an impure function that interacts with the outside world. Pure functions don't use `!`.

## Interactive REPL

To experiment with expressions without creating files, use the REPL:

```bash
kata repl
```

```
>>> + 1 2
3
>>> echo!("olá")
olá
>>> :quit
```

The REPL maintains bindings between lines and supports `:type` to inspect types, `:env` to view the environment, and `:load arquivo.kata` to load a module.

## Next steps

You've written your first Kata program. The next chapter builds a complete guessing game — a mini-project that introduces actions, pattern matching, and reading input before the theory. → [Chapter 2](02-guessing-game.md)