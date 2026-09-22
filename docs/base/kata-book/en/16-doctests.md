# Chapter 16 — Doctests

Doctests are executable examples embedded in multiline comments. They allow you to document code with examples that are automatically validated by `kata test` — if the documentation lies, the test fails.

## Syntax

Inside a multiline comment `#{ }#`, lines that start with `>>> ` are REPL inputs. The content after `>>> ` is evaluated as a Kata expression. The following lines (without `>>> `) are the expected output.

```
#{
Calculates double of a number.

>>> constant n := 5
>>> + n n
10
>>> constant m := 0
>>> + m m
0
}#
```

## How it works

Each doctest block creates a fresh REPL session. Consecutive `>>> ` lines share bindings — what you define on one line is available on the next.

```
#{
>>> constant x := 10
>>> + x 1
11
>>> * x 2
20
}#
```

A blank line separates blocks. Each block starts a new session — bindings do not persist across blocks.

```
#{
>>> constant x := 10
>>> + x 1
11

>>> constant y := 20
>>> + y 1
21
}#
```

In the second block, `x` does not exist.

## Free text before doctests

Lines before the first `>>> ` inside `#{ }#` are free documentation — ignored by the doctest runner. This allows you to write explanatory prose before the examples.

```
#{
The `soma` function takes two Int and returns the sum.

Example:
>>> + 3 4
7
}#
```

## Multiline input

If the input after `>>> ` is incomplete (e.g., `match` without clauses), the following indented lines are a continuation of the input, not expected output.

```
#{
>>> match True
  True: "sim"
  False: "nao"
sim
}#
```

## No expected output

`>>> ` lines without subsequent output lines mean "produces no output." This covers declarations (`constant`, `let`, `Sig`, `data`, `enum`) that have no entry expression.

```
#{
>>> constant x := 42
>>> x
42
}#
```

The first line (`constant x := 42`) has no expected output. The second (`x`) produces `42`.

## Running

Doctests run automatically with `kata test`, before the `@test` tests:

```bash
kata test examples/directives/assertions.kata
```

```
  [PASS] exemplo.kata: doctest line 2
  [PASS] exemplo.kata: doctest line 3
  2 passed, 0 failed, 0 skipped
```

If the output does not match:

```
  [FAIL] exemplo.kata: doctest line 3: output mismatch
    expected: 99
    got:      10
  1 passed, 1 failed, 0 skipped
```

`--filter` does not affect doctests — they always run.

## Reusing code from the file

Doctests create a fresh REPL session — they do not see declarations from the file automatically. To test functions defined in the file, use `:load` as the first case in the block:

```
fat :: Int => Int
lambda 0: 1
lambda n: * n (fat (- n 1))

#{
The `fat` function computes the factorial.

>>> :load exemplos/fatorial.kata
>>> fat 5
120
>>> fat 0
1
}#
```

The message `carregado: exemplos/fatorial.kata` goes to `__stderr__` (the `File` value for stderr, available via `import stdio`) — it does not interfere with the doctest output capture, which only collects `__stdout__`.

To test functions from another module, use `import` as a `>>>` line:

```
#{
>>> import exemplos/mock_math.(dobrar)
>>> dobrar 5
10
}#
```

The distinction between `:load` and `import` is intentional:

- **`:load`** loads everything from the file (declarations + entry point),
  executes top-level, side effects happen. Does not require `export`.
- **`import`** brings only exported symbols, without executing top-level.
  No side effects. Requires `export` in the module.

## Comments without doctests

`#{ }#` comments without any `>>> ` lines are ignored completely. There is no impact on `kata test` — behavior is identical to before.

## Next chapter

You have completed all chapters of the main guide. There is also an appendix about supported platforms and limitations — including the state of the Windows port:

→ [Appendix — Platforms and Limitations](17-plataformas-limitacoes.md)