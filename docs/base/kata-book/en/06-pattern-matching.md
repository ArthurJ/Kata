# Chapter 6 — Pattern Matching

Kata has no `if`. Conditionals are expressed through pattern matching and guards. This guarantees exhaustiveness — the compiler verifies that all cases are covered.

## `match`

`match` examines a value and executes the arm corresponding to its shape:

```kata
match True
    Boolean::True: echo!("sim")
    Boolean::False: echo!("não")
```

```
sim
```

Each arm has a pattern on the left of the `:` and an expression on the right. The first pattern that matches wins.

## Unpacking values

Patterns can extract the payload of variants. `Result` has `Ok(T)` and `Err(E)`:

```kata
match (div 10 3)
    Result::Ok v: echo!(v)
    Result::Err e: echo!("erro")
```

```
3
```

The pattern `Result::Ok v` matches when the value is `Ok` and binds the payload to the variable `v`.

`div` is dynamic division: it returns `Result` because the divisor can be zero
— the type system cannot prove that 3 ≠ 0 at compile-time (3 is a regular `Int`,
not `NonZero`). The `Ok` arm receives the quotient; the `Err` arm receives the
error message. The exact division `/` requires `NonZero` and does not
return `Result` — see chapter 12.

## Guards in lambda

Inside functions, guards replace `if/else`. A guard is a boolean condition after the parameter:

```kata
abs :: Int => Int
lambda x:
    > x 0: x
    otherwise: - 0 x

echo!(abs 5)
echo!(abs (- 0 5))
```

```
5
5
```

The first guard `> x 0` tests whether `x` is positive. `otherwise` is the mandatory fallback when guards are present — it ensures total coverage.

`otherwise` is not always necessary. When the disjunction of the guards covers
the entire input space in a **provable** way, the compiler accepts the
function without a fallback:

```kata
sinal :: Int => Int
lambda x:
    > x 0: 1
    < x 0: - 0 1
    = x 0: 0

action main
    echo!(sinal 42)
    echo!(sinal (- 0 7))
    echo!(sinal 0)
main!()
```

```
1
-1
0
```

Every Int is greater than, less than, or equal to zero — the disjunction of the
three guards is always true and the compiler proves this statically. But if a
case is missing:

```kata
sinal :: Int => Int
lambda x:
    > x 0: 1
    < x 0: - 0 1
```

the compiler rejects it with `match não-exaustivo` — `0` is not covered by any
guard. Add the missing case or `otherwise:` as a fallback.

The compiler makes an honest effort to distinguish when `otherwise` is
necessary: it proves what it can prove and only requests a fallback from the
developer when it cannot decide — it never accepts a potentially incomplete
`match`, and never demands a fallback that is known to be unnecessary.

Redundant clauses are also an error: if an earlier arm already covers all the
values of a later arm, the later one is unreachable and the compiler emits an
error, indicating which clause made it redundant.

## `with` — pre-computations

Sometimes a guard needs an intermediate value. The `with` block declares bindings visible to all guards in the clause:

```kata
classify :: Int => Text
lambda x:
    > doubled 10: "grande"
    otherwise: "pequeno"
    with
        doubled := * x 2

echo!(classify 3)
echo!(classify 6)
```

```
pequeno
grande
```

`doubled := * x 2` is evaluated before the guards, even though it is written after them. The order is visual — `with` is post-written but pre-evaluated.

### `with` cross-clause — shared bindings

When a function has multiple `lambda` clauses, `with` can be declared at the outer level — after the last clause. The bindings are selectively injected into each clause that references them:

```kata
classify :: Int => Text
lambda 0: tag_zero
lambda x:
    > doubled 10: "grande"
    otherwise: "pequeno"
with
    doubled := * x 2
    tag_zero := "zero"
```

```kata
echo!(classify 0)
echo!(classify 3)
echo!(classify 6)
```

```
zero
pequeno
grande
```

The `lambda 0` clause receives `tag_zero` (referenced in the body) but not `doubled` (not referenced). The `lambda x` clause receives `doubled` but not `tag_zero`. The injection is selective — each binding goes only to the clauses that use it, avoiding unbound variable errors when the patterns differ.

## Next chapter

Everything so far is pure code — no side effects. The next chapter introduces actions, the barrier between pure and impure code, and the imperative control mechanisms: `var`, `loop`, `break`. → [Chapter 7](07-actions.md)