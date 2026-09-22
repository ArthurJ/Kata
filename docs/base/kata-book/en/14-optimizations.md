# Chapter 14 — Optimizations

Kata applies optimizations automatically. The programmer writes declarative code; the compiler transforms it to run without overflowing the stack and without intermediate collections.

## TCO — Tail Call Optimization

When the recursive call is the last operation of the function, the compiler rewrites it as a loop. The stack does not grow:

```kata
fat_tail :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat_tail (- n 1) (* n acc)

fatorial :: Int => Int
lambda n: fat_tail n 1

action main
    echo!(fatorial 5)
main!()
```

```
120
```

The call `fat_tail (- n 1) (* n acc)` is in tail position — there is nothing after it. The compiler detects this and eliminates stack growth.

## TRMA — Tail Recursion Modulo Associativity

What if the recursion is not tail-recursive? When the call is inside an associative operation (`+`, `*`), the optimizer rewrites it with an accumulator:

```kata
soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))
```

The call `soma (- n 1)` is inside `+ n (...)` — it is not in tail position. But `+` is associative, so the compiler rewrites it to:

```kata
soma_acc :: Int Int => Int
lambda 0 acc: acc
lambda n acc: soma_acc (- n 1) (+ acc n)
```

Now it is in tail position — TCO applies. The programmer does not need to do anything:

```kata
echo!(soma 1000000)
```

```
500000500000
```

Without TRMA, 1 million recursive calls would overflow the stack. With TRMA, it runs without issue.

## Stream fusion

`map`, `filter`, and `fold` are intercepted by the compiler and generate special nodes in the TAST. When you compose `filter(f, map(g, arr))`, the optimizer fuses the two into a single loop — no intermediate collections:

```kata
processar :: [Int] => Int
lambda arr: fold (+) 0 (filter (lambda x: > x 0) (map (* _ 2) arr))

echo!(processar [1 -2 3])
```

```
8
```

Without fusion: `map` creates an intermediate list, `filter` creates another. With fusion: a single pass over the list, applying `* 2` and filtering simultaneously.

## `constant` — compile-time evaluation

`constant` evaluates the expression during compilation and embeds the result in the binary:

```kata
fatorial :: Int => Int
lambda 0: 1
lambda n: * n (fatorial (- n 1))

constant fatorial_10 := fatorial 10

echo!(fatorial_10)
```

```
3628800
```

The computation `fatorial 10` runs at compile-time. In the binary, `fatorial_10` is the literal `3628800` — there are no function calls at runtime.

## Recursion depth limit

TCO and TRMA eliminate stack growth in tail recursion. But non-tail recursion still grows the stack — and without protection, deep recursion overflows the process stack (SIGSEGV with no message).

Kata has a **software depth counter** that detects excessive recursion before SIGSEGV and produces a graceful failure with a clear message:

```
recursion depth exceeded: 1200 (limit: 1000)
```

The default limit is **1000 frames**. Tail calls do not increment the counter — TCO continues to work. The limit is configurable in Kata code via `constant`:

```kata
import stdlib.config

constant _ := config.set_recursion_limit(5000)

soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))

echo!(soma 3000)  # works — limit is 5000
```

`set_recursion_limit` takes a `PositiveInt` — the type system rejects `set_recursion_limit(-5)` at compile-time. The configuration is per-module and immutable at runtime.

## Next chapter

Optimizations are automatic. The next chapter shows the interactive REPL — exploring the language without creating files. → [Chapter 15](15-repl.md)