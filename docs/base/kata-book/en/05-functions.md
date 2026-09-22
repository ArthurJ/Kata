# Chapter 5 — Functions

Functions in Kata are pure: they take arguments, compute a result, and have no side effects. The definition has two parts — signature and body.

## Signature and body

The signature declares the name and types. The body uses `lambda` with the parameters:

```kata
dobrar :: Int => Int
lambda x: * x 2

echo!(dobrar 21)
```

```
42
```

The signature `dobrar :: Int => Int` reads as: `dobrar` takes an `Int` and returns an `Int`. The `::` tags the name with its type. The `=>` separates arguments from the return type.

## Multiple clauses

A function can have multiple `lambda` clauses. The first one that matches the arguments wins:

```kata
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat (- n 1) (* n acc)

echo!(fat 5 1)
```

```
120
```

The first clause matches when the first argument is `0` — it returns the accumulator. The second clause matches any other value `n` — it calls `fat` recursively with `n-1` and `n * acc`.

## Recursion

Recursion is the iteration mechanism in the pure domain. There is no `while` or `for` in pure functions.

```kata
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

echo!(fib 10)
```

```
55
```

## Tail recursion

When the recursive call is the last operation of the function, the compiler optimizes it to avoid growing the stack. The factorial above uses an accumulator (`acc`) — the call `fat (- n 1) (* n acc)` is the last operation, so it is tail recursion:

```kata
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat (- n 1) (* n acc)

echo!(fat 100000 1)
```

```
282422940796034787429342157802...
```

Even with 100,000 recursive calls, the stack does not overflow. The compiler rewrites tail recursion as an internal loop.

## Next chapter

Functions become more powerful with pattern matching. The next chapter shows how to branch logic with `match`, guards, and `otherwise`. → [Chapter 6](06-pattern-matching.md)