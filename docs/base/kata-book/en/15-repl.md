# Chapter 15 — Interactive REPL

The REPL (Read-Eval-Print Loop) is the fastest way to experiment with Kata. Without creating files, you evaluate expressions, inspect types, and load modules.

## Starting

```bash
kata repl
```

```
Kata REPL — type :help for commands, :quit to exit
```

## Evaluating expressions

Type any Kata expression. The result appears immediately:

```
>>> + 1 2
3
>>> * 3 4
12
>>> echo!("olá")
olá
```

## Persistent bindings

Bindings made with `let` persist across lines:

```
>>> let x := 42
>>> + x 8
50
```

The binding `x` remains available in subsequent lines until you exit or reset.

## `:type` — inspect types

Without executing, see the type of an expression:

```
>>> :type + 1 2
Int
```

Useful for understanding what the typechecker infers without running the code.

## `:env` — view the environment

Lists all bindings and their types:

```
>>> let x := 42
>>> :env
  x: Int
```

## `:load` — load a file

Loads a `.kata` file into the REPL environment:

```
>>> :load fatorial.kata
carregado: fatorial.kata
>>> fat 5 1
120
```

The functions and constants from the file become available for interactive use.

## `:reset` — clear the environment

Removes all bindings and reloads the prelude:

```
>>> let x := 42
>>> :reset
sessão resetada — prelude recarregado
>>> :env
(no bindings)
```

## `:help` — available commands

```
>>> :help
comandos:
  :help          shows this message
  :type <expr>   shows the type of <expr> without executing
  :env           shows bindings and types in the current TypeEnv
  :load <file>   loads a .kata file (items enter the env)
  :reset         clears bindings, reloads prelude
  :quit          exits the REPL
```

## Next chapter

The REPL is great for experimenting, but reproducible tests are essential. The next chapter shows how to write doctests — executable examples embedded in comments that run with `kata test`:

→ [Chapter 16 — Doctests](16-doctests.md)

To go deeper:
- `examples/` — complete examples of each feature
- `docs/Kata-lang-manual.md` — technical reference manual
- `docs/sintaxe-mapa.md` — complete syntax map