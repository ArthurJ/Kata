# Chapter 10 — Enums and Structs

Kata models data with algebraic types. `enum` defines sum types (OR) — a value is one of several variants. `data` defines product types (AND) — a value combines several fields.

## `enum` — sum types

Each variant on an indented line. No `|` separator:

```kata
enum Cor
    Verde
    Amarelo
    Vermelho

echo!(Verde)
echo!(Cor::Amarelo)
```

```
Verde
Amarelo
```

Unit variants (no payload) are available without qualification. `Verde` and `Cor::Verde` are the same value.

## Variants with payload

Variants can carry data. `Optional` from the prelude has `Some(T)` and `None`:

```kata
enum Optional
    Some(Int)
    None

echo!((Some 42))
echo!(None)
```

```
Some(42)
None
```

`Some 42` constructs the variant with payload. The parentheses in `echo!(Some 42)` are necessary — `echo!` has arity 1 and `Some 42` needs to be grouped.

## `match` on enums

Pattern matching unpacks the payload:

```kata
enum Optional
    Some(Int)
    None

match (Some 42)
    Some v: echo!(v)
    None: echo!("nada")
```

```
42
```

The pattern `Some v` extracts the payload into `v`. The compiler checks exhaustiveness — you must cover all variants.

## `data` — product types

`data` defines a struct with named fields. Fields are typed via `::`:

```kata
data Pessoa (nome::Text idade::Int)

action main
    let p := Pessoa "João" 30
    echo!(p.nome)
    echo!(p.idade)
main!()
```

```
João
30
```

Field access with `.` — `p.nome` reads the `nome` field. Construction is positional: `Pessoa "João" 30` passes arguments in the declared order.

## Combining everything

```kata
data Ponto (x::Int y::Int)

action main
    let p := Ponto 3 4
    echo!(+ (* p.x p.x) (* p.y p.y))
main!()
```

```
25
```

The squared distance from the origin: `3² + 4² = 25`.

## `data` with type params — parametric generics

`data` can be parameterized by type params. Type params are detected
implicitly: PascalCase in type position in the fields (like `Ok(T)` in
enums). There is no explicit `::(...)` list in the declaration — the
`::(...)` instantiation is used at call sites.

### Shared form — type param with bound

```kata
data Complex (re::T im::T) where T implements SCALAR
```

- `(re::T im::T)` — fields with type param `T`.
- `where T implements SCALAR` — bound: `T` must implement `SCALAR`.
- `T` is the same type in both fields — `Complex 3 4` types as
  `Complex::(Int, Int)`, `Complex 1.0 2.0` as `Complex::(Float, Float)`.
- `Complex "a" "b"` fails — `Text` does not implement `SCALAR`.

The instantiation `Complex::(Int, Int)` appears automatically when the
constructor dispatches: the monomorphizer creates concrete methods on-demand
for each combination of type args used.

### Independent form — anonymous vars with bound

```kata
data Par (first::SCALAR scd::SCALAR)
```

- `SCALAR` in the field's type position is interpreted as "fresh anonymous
  var with bound `SCALAR`". Each occurrence is a distinct var.
- Allows different types in each field: `Par 3 4.0` accepted (`Int` and
  `Float` both implement `SCALAR`).
- It is sugar for independent bounds, not for shared params.

### Free form — type param without bound

```kata
data Par (first::A second::B)
```

- `A` and `B` are free type params (no bound). PascalCase in type
  position, detected in pass0.
- Constructor accepts any pair of types: `Par 3 "hello"` is valid.

### Instantiation

Instantiation uses `::(...)` — **1 type arg per occurrence of type param
in the fields, not per distinct variable.**

```kata
data Complex (re::T im::T) where T implements SCALAR
# 2 ocorrências de T → 2 type args
# Complex::(Int, Int)         — re::Int, im::Int
# Complex::(Float, Float)     — re::Float, im::Float

data Pair (fst::A scd::B)
# 2 params independentes → 2 type args
# Pair::(Int, Text)
```

`::(...)` is instantiation, **not** declaration. Writing
`data Complex::(T) (re::T im::T)` is a syntax error.

### Implementing interfaces for generic types

Methods are defined for a specific instantiation:

```kata
data Complex (re::T im::T) where T implements SCALAR

Complex::(Float, Float) implements RING
    + :: Complex::(Float, Float) Complex::(Float, Float) => Complex::(Float, Float)
    lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
```

The monomorphizer instantiates the body by substituting `T` with the concrete
type (`Float`), and `+ a.re b.re` dispatches to `+ :: Float Float => Float`.

### Complete example

```kata
data Pair (first::T second::T) where T implements NUM

Pair::(Int, Int) implements EQ
    = :: Pair::(Int, Int) Pair::(Int, Int) => Boolean
    lambda a b: and (= a.first b.first) (= a.second b.second)

action main
    let p := Pair 3 4
    let q := Pair 3 4
    echo!(= p q)
main!()
```

```
True
```

## `?` — short-circuit in Actions

The `?` operator unpacks `Result` and `Optional` inside Actions. If the value is `Ok(v)` or `Some(v)`, it yields `v` and continues. If it is `Err(e)` or `None`, it aborts the action with `return Err(e)` or `return None`:

```kata
action parse_num (s::Text) => Result::(Int, Text)
    let n := int(s) ?
    Ok n

action main => Unit
    echo!(show (parse_num!("42")))
    echo!(show (parse_num!("abc")))
main!()
```

```
Ok(42)
Err("número inválido")
```

`int(s)` returns `Result::(Int, Text)`. The `?` unpacks the `Ok` and binds `n` to the inner value. If `int(s)` fails, `?` aborts the action — the `Ok n` line never executes, and the `Err` propagates as the return.

Without `?`, the equivalent would be:

```kata
match int(s)
    Ok v: Ok v
    Err e: Err e
```

`?` only works inside Actions — it needs a `return` to abort. In pure functions, use `|` (fallback) or explicit `match`.

## `|` — fallback (coalescing)

The `|` operator is a synthetic `match` over enums. The rule is general, not specific to `Result` or `Optional`:

- **Non-tail** variants (all except the last) must have payload — `|` unpacks and returns the value
- The **tail** (last variant) triggers the fallback — evaluates the right-hand expression. If it has payload, it is discarded

Unlike `?`, it does not abort — it is a pure expression, works in functions and Actions.

With `Optional`, the tail `None` has no payload:

```kata
echo!(show (Some 42 | 99))
echo!(show (None | 99))
```

```
42
99
```

`Some 42 | 99` unpacks `42`. `None | 99` falls into the tail and evaluates the fallback `99`.

With `Result`, the tail `Err` has payload — but it is discarded. You chose `|` instead of `match`, indicating you don't need the error:

```kata
echo!(show (Ok 42 | 0))
echo!(show (Err "err" | 99))
```

```
42
99
```

`|` works with any user enum that respects the rule. All non-tail variants must have payload; the tail may or may not:

```kata
enum Light
    Red(Int)
    Green(Int)
    Off

echo!(show (Light::Red 42 | 0))
echo!(show (Light::Green 7 | 0))
echo!(show (Light::Off | 0))
```

```
42
7
0
```

`Red` and `Green` are non-tail with payload — they unpack. `Off` is the tail — evaluates the fallback. If a non-tail variant were unitary (no payload), the compiler would reject it — there is nothing to unpack.

`|` is useful for providing a default value when an operation may fail:

```kata
echo!(show (int("42") | 0))
echo!(show (int("abc") | 0))
```

```
42
0
```

### `?` vs `|`

| | `?` | `|` |
|---|---|---|
| Aborts? | Yes — `return Err(e)` | No — evaluates rhs |
| Context | Actions only | Actions and pure functions |
| Access to error | Yes — propagates | No — discards |
| Syntax | `expr ?` | `lhs \| rhs` |

Use `?` when you want to propagate the error to the caller. Use `|` when you want a default value and don't care about the error.

## Next chapter

Data types are the pure side. The next chapter enters the world of concurrency — `fork!`, channels, `select`, and communication between fibers. → [Chapter 11](11-actions-avancadas.md)