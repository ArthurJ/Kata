# Chapter 4 — Bindings and Types

Bindings give names to values. Kata has three kinds of bindings: `constant` at the module level, `let` for immutable local bindings, and `var` for mutable local bindings.

## `constant` — module-level constants

At the top level of a file, use `constant` to declare values evaluated at compile-time:

```kata
constant pi := 3.14
constant nome := "Kata"

echo!(pi)
echo!(nome)
```

```
3.14
Kata
```

`constant` is evaluated when the program compiles, not when it runs. The value is embedded in the binary.

## `let` — local bindings

Inside functions and actions, use `let` for immutable bindings:

```kata
action main
    let x := 42
    let y := 99
    echo!(+ x y)
main!()
```

```
141
```

`let` doesn't exist at the top level of files — only inside actions and functions. At the top level, use `constant`.

## `let` is unique per scope

Each `let` declares an immutable and **unique** binding in the current scope. Re-declaring the same name is an error:

```kata
action main
    let x := 42
    let x := 99
    echo!(x)
main!()
```

```
Error: type.duplicate_decl

  × type `x` already declared
```

To reuse a name, use `var` — described in the next section.

## `var` — mutable bindings

`var` creates a mutable binding and can be redeclared in the same scope, replacing the previous binding:

```kata
action main
    var x := 42
    var x := + x 1
    echo!(x)
main!()
```

```
43
```

`var` is the correct mechanism when you need to reuse a name or update a value within a scope. `let` is immutable and unique; `var` is mutable and replaceable.

## Type ascription in bindings

Both `let` and `var` accept a type annotation between the name and the `:=`:

```kata
action main
    let x::Int := 42
    echo!(x)
    var y::Text := "hello"
    echo!(y)
main!()
```

```
42
hello
```

Binding ascription annotates the **slot** — it doesn't convert the value. The compiler verifies that the RHS is compatible with the annotated type.

### Interface widening in `var`

When the ascription is an interface (like `NUM`), `var` accepts any concrete type that implements the interface. The concrete type is preserved for dispatch:

```kata
action main
    var z::NUM := 0
    echo!(+ z 1)
    z := 3.14
    echo!(+ z 1.0)
main!()
```

```
1
4.140000000000001
```

`z::NUM` accepts `Int` (0), `Float` (3.14), `Rational` — any type that implements `NUM`. Dispatch uses the concrete type: `+ z 1` calls `+ :: Int Int => Int`, and after `z := 3.14`, `+ z 1.0` calls `+ :: Float Float => Float`.

### Re-binding preserves the interface

Re-binding without ascription validates against the declared interface, not against the current concrete type:

```kata
action main
    var z::NUM := 0
    var z := 3.14
    echo!(z)
main!()
```

```
3.14
```

`var z := 3.14` is accepted because `Float` implements `NUM`. But `Text` doesn't implement `NUM`:

```kata
action main
    var z::NUM := 0
    var z := "hello"
    echo!(z)
main!()
```

```
Error: type.mismatch

  × incompatible type: expected `NUM`, found `Text`
```

### No ascription, no widening

Without an interface ascription, `var` infers the type from the RHS and locks it. Changing to a different type is an error:

```kata
action main
    var x := 0
    var x := 3.14
    echo!(x)
main!()
```

```
Error: type.mismatch

  × incompatible type: expected `Int`, found `Float`
```

To accept multiple types in the same binding, declare the interface explicitly with ascription.

## Primitive types

| Type | Description | Example |
|------|-----------|---------|
| `Int` | Arbitrary-precision integer | `42` |
| `Float` | 64-bit floating point | `3.14` |
| `Text` | Text (string) | `"hello"` |
| `Boolean` | True or false | `True` |
| `Unit` | Absence of value | `()` |
| `Rational` | Exact rational number | `3.14::Rational` |
| `Bytes` | Sequence of binary bytes | `b"hello"` |
| `Byte` | Individual byte (0–255) | via `at` on `Bytes` |

Integers have arbitrary precision. There is no overflow:

```kata
echo!(* 99999999999999999999 99999999999999999999)
```

```
9999999999999999999800000000000000000001
```

## The `::` operator

`::` is a multi-purpose operator in Kata. Its primary role is **type ascription** — attaching a type to an expression:

```kata
action main
    let x := 42 :: Int
    echo!(x)
main!()
```

```
42
```

Ascription is useful when you want to be explicit about the type of an expression. The compiler verifies that the type is compatible with the value.

### Literal conversion with `::`

`::` also converts literals between related types. The most common case is `Rational`:

```kata
echo!(3.14::Rational)
```

```
3.14
```

The raw text of the literal is preserved — there's no pass through `f64`, so there's no precision loss in the conversion. (For exact precision with `Rational`, see the next section.)

### The other roles of `::`

`::` appears in five other contexts in Kata. Each will be explored in its chapter:

- **Function signature** (ch 5): `dobrar :: Int => Int` labels the name with its type
- **Field and parameter typing** (chs 7, 10): `data Pessoa (nome::Text)`, `action jogar (alvo::Int)`
- **Variant qualification** (ch 10): `Cor::Amarelo`, `Result::Ok` — accesses an enum variant by type name
- **Refined types** (ch 12): `5 :: PositiveInt` validates predicates at compile-time; `a :: Int` downcasts from refined to base
- **Binding ascription** (ch 4): `let x::Int := 42`, `var z::NUM := 0` — annotates the binding's type, doesn't operate on the value

Although the token is the same, ascription (`value :: Type`) and qualification (`Type::Variant`) operate in opposite directions: the former goes from value to type, the latter from type to variant.

## Rational — exact precision

Floats have inherent imprecision. `Rational` is exact — `1/3 * 3 = 1`, not `0.999...`:

```kata
echo!(* 3.14::Rational 100)
```

```
314
```

With `Float`, the same calculation could give `314.00000000000006`. `Rational` preserves the exact value.

## `Bytes` — binary data

`Bytes` is an immutable sequence of raw bytes. Literals use the `b` prefix:

```kata
echo!(b"hello")
echo!(len b"hello")
echo!(show b"\xFF\x00")
```

```
68656c6c6f
5
ff00
```

`echo!` displays `Bytes` in hexadecimal. `show` does too — the textual representation of `Bytes` is always hex. `len` returns the number of bytes.

`+` concatenates `Bytes`:

```kata
echo!(+ b"abc" b"def")
```

```
616263646566
```

`Bytes` implements `INDEXABLE` — access by index returns `Result::(Byte, Text)` (error if index is out of range). `Byte` is the type of a single byte, with bitwise operations (`and`, `or`, `xor`, `not`, `<<`, `>>`). `int` and `float` accept `Byte` as an argument, converting it to the corresponding numeric value.

## Next chapter

Now that you know how to give names to values, the next chapter shows how to define functions with signatures, multiple clauses, and recursion. → [Chapter 5](05-funcoes.md)