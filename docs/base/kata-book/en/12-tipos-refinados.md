# Chapter 12 — Refined Types and Aliases

Kata offers two ways to create a new nominal type over an existing type: **refined types** (with validation) and **aliases** (without validation). Both are zero-cost at runtime — same bits, same Cranelift type, no wrapping.

## Refined Types

The syntax `data (Base, predicate) as Nome` creates a refined type. The `_` in the predicate represents the value being tested:

```kata
data (Int, > _ 0) as PositiveInt
```

This declares `PositiveInt` — an `Int` that must be greater than zero. The predicate is checked at compile-time for literals and at runtime for dynamic values.

### Literal ascription

For literals that satisfy the predicate, use `::` directly:

```kata
action main
    let x := 5::PositiveInt
    echo!(x)
main!()
```

```
5
```

The compiler validates the predicate at compile-time. A negative literal fails:

```kata
let x := (- 0 5)::PositiveInt   # erro de tipo: -5 não é > 0
```

### Smart constructor

For dynamic values, use the fallible constructor. It returns `Result`:

```kata
data (Int, > _ 0) as PositiveInt

action main
    let r := PositiveInt 42
    match r
        Ok v: echo!(v)
        Err _: echo!("erro")
main!()
```

```
42
```

When the value does not satisfy the predicate, the constructor returns `Err`:

```kata
action main
    let r := PositiveInt (- 0 5)
    match r
        Ok v: echo!(v)
        Err _: echo!("erro")
main!()
```

```
erro
```

## Polymorphic families

The refined types we have seen so far are **concrete**: `PositiveInt` refines
`Int`. Kata also supports **polymorphic families** — refined types that
refine an entire *interface* instead of a specific type:

```kata
data (NUM, != _ (zero _)) as NonZero
```

`NonZero` refines `NUM` — the interface that `Int`, `Float`, and `Rational`
implement. The predicate `!= _ (zero _)` checks that the value is different
from `zero` of its type (`0` for Int, `0.0` for Float, `rational 0` for
Rational). There are three instances of `NonZero`: `NonZero::Int`,
`NonZero::Float`, and `NonZero::Rational`.

### Literal ascription

`5::NonZero` works just like with concrete refined types — the compiler
validates the predicate at compile-time. The concrete instance is inferred
from the literal's type:

```kata
action main
    let x := 5::NonZero       # NonZero::Int
    let y := 3.0::NonZero     # NonZero::Float
    echo!(x::Int)
    echo!(y::Float)
main!()
```

```
5
3.0
```

The downcast `x::Int` extracts the base value — it is a no-op at runtime (same
bits).

### Safe division with NonZero

The main purpose of `NonZero` is to guarantee safe division at
compile-time. The `/` operator requires `NonZero` as the divisor:

```kata
echo!(/ 10 (3::NonZero))           # 3 — divisão exata, sem Result
echo!(/ 10.0 (3.0::NonZero))       # 3.3333333333333335
echo!(// 10 (3::NonZero))          # 3 — divisão inteira, retorna Int
echo!(// 10.0 (3.0::NonZero))      # 3 — Float truncado para Int
echo!(mod 10 (3::NonZero))         # 1 — resto
```

If the divisor were zero, the ascription `0::NonZero` would fail at
compile-time — the program does not even compile. This eliminates an entire
class of division-by-zero bugs with no runtime cost.

### Fallible constructor

For dynamic values, the `NonZero` constructor returns `Result`. The
concrete instance is determined by the argument's type:

```kata
action main
    let r := NonZero 0
    match r
        Ok v: echo!(v::Int)
        Err _: echo!("zero rejeitado")
main!()
```

```
zero rejeitado
```

`NonZero 0` returns `Err` because `!= 0 (zero 0)` is false. `NonZero 42`
would return `Ok` with `NonZero::Int`. `NonZero 3.0` would return `Ok` with
`NonZero::Float`.

### Cross-type with NonZero

Since `NonZero` is a family over `NUM`, operations between different types
work when the divisor is qualified:

```kata
echo!(/ 10.0 (3::NonZero))              # 3.333... (Float ÷ NonZero::Int)
echo!(mod (rational 10) (3::NonZero))   # 1 (Rational ÷ NonZero::Int)
```

The divisor `3::NonZero` is `NonZero::Int`. The division `/ 10.0` dispatches
to the overload `Float × NonZero::Int → Float`. The compiler selects the
correct instance based on the argument's type.

## Aliases — Newtype without predicates

`alias` creates a new nominal type distinct from the original, but without
validation. It is a pure *newtype*: same bits, infallible constructor, zero
cost at runtime.

```kata
alias Float as Altura
```

`Altura` is a different type from `Float` for the type checker. The constructor
is infallible — it does not return `Result`, because there is no predicate
to fail:

```kata
action main
    let a := Altura 1.75
    echo!(a)
main!()
```

### When to use alias vs refined type

Use a **refined type** when there is a condition the value must satisfy
(`> 0`, `/= 0`, `< 100`). Use an **alias** when you just want a distinct name
for an existing type — for example, to avoid confusion between values
with the same representation but different meanings (`Altura` vs `Float`).

### Alias of a refined type

An alias can target a refined type. In that case, the alias inherits
the predicates and becomes refined as well:

```kata
data (Float, > _ 0.0) as PositiveFloat
alias PositiveFloat as Peso
```

`Peso` is a refined type: it has the same predicates as `PositiveFloat`,
the constructor is fallible (returns `Result`), and it can declare `refines`.
Internally, the compiler follows the chain: `Peso` → `alias_of` →
`PositiveFloat` → `alias_of` → `Float`.

## `refines` — interface delegation

A refined type does not automatically inherit the operations of the base type.
`refines` delegates an interface to the base type:

```kata
data (Int, > _ 0) as PositiveInt

PositiveInt refines NUM

action main
    let a := 5::PositiveInt
    let b := 3::PositiveInt
    let soma := PositiveInt (+ a b)
    match soma
        Ok v: echo!(v)
        Err _: echo!("erro")
main!()
```

```
8
```

Without `refines NUM`, `+ a b` would fail — `+` is not defined for
`PositiveInt`. With `refines`, dispatch tries the base type `Int` and wraps
the result in the fallible constructor, producing `Result::(PositiveInt, Err)`.

The `refines` fallback follows the `alias_of` chain. Therefore, if `Peso`
is an alias of `PositiveFloat` which has `refines NUM`, then `+ a b` where
`a` and `b` are `Peso` also works — the fallback traverses
`Peso → PositiveFloat → refines NUM → Float`.

A pure alias (without `refines`) does **not** interoperate with the base type
in dispatch. `Altura + 3.0` fails because `Altura` is nominally distinct from
`Float`. To interoperate, use explicit downcast.

## Downcast with `::`

To convert a refined type (or alias) back to the base type, use `::`:

```kata
action main
    let a := 5::PositiveInt
    let n := a::Int
    echo!(n)
main!()
```

```
5
```

The downcast is a no-op at runtime — same bits, no cost. The typechecker
verifies that the target is the base type (or a type in the `alias_of` chain).

## Finite domain — `match` without `otherwise`

When a refined type's predicates define a finite range over a
discrete type (Int, Rational, Boolean), the compiler **enumerates the domain**
and checks coverage case by case. If all possible values have an
arm, `otherwise` is unnecessary:

```kata
data (Int, > _ 0, < _ 3) as UmOuDois

describe :: UmOuDois => Text
lambda n:
    match n
        1: "um"
        2: "dois"

action main
    echo!(describe (1::UmOuDois))
    echo!(describe (2::UmOuDois))
main!()
```

```
um
dois
```

The type `UmOuDois` has exactly two possible values: `1` and `2`. The two
arms cover the entire domain and the `match` is exhaustive without a fallback.

Literals outside the domain are rejected at compile-time: `3::UmOuDois` is a
type error, because 3 violates `< _ 3`. The domain exists whenever the
predicates are bounds with literals (`> _ N`, `< _ M`, `= _ K`) over Int,
Rational, or Boolean.

Refined types over Rational work the same way — arms match
rational literals constructed via `rational`:

```kata
data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

describe :: RatUmOuDois => Text
lambda n:
    match n
        rational 1: "um"
        rational 2: "dois"

action main
    let r := RatUmOuDois (rational 1)
    match r
        Ok v: echo!(describe v)
        Err _: echo!("erro")
main!()
```

```
um
```

## Next chapter

Refined types and aliases guarantee invariants at compile-time. The next
chapter shows how to organize code into modules with `import` and `export`.
→ [Chapter 13](13-modulos.md)