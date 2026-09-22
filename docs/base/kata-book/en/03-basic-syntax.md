# Chapter 3 — Basic Syntax

Kata uses prefix notation: the function comes before the arguments, without parentheses. This eliminates precedence ambiguity and treats operators and functions the same way.

## Prefix notation

```kata
+ 1 2       # addition
* 3 4       # multiplication
- 10 3      # subtraction
```

```
3
12
7
```

Operators (`+`, `-`, `*`) are function names like any other. `+ 1 2` and `soma 1 2` have the same structure. There's no precedence table to memorize.

## Division and remainder

Kata has four division operators with distinct semantics:

```kata
echo!(/ 10 (3::NonZero))       # exact division — requires NonZero
echo!(// 10 (3::NonZero))      # integer division — requires NonZero, returns Int
echo!(mod 10 (3::NonZero))     # remainder — requires NonZero
match (div 10 3)               # dynamic division — returns Result
    Result::Ok v: echo!(v)
    Result::Err e: echo!("erro")
```

```
3
3
1
3
```

`/` is exact mathematical division: it returns the pure value directly, without
`Result`. To guarantee the divisor isn't zero, `/` requires `NonZero` —
a refined type over the `NUM` interface (see chapter 12). The literal
`3::NonZero` is validated at compile-time: the compiler proves that 3 ≠ 0.

`//` is integer division: it returns the truncated quotient as `Int`,
regardless of the operands' type. It also requires `NonZero`.

`mod` is the remainder of division. It also requires `NonZero` for the same reason.

`div` is dynamic division: it accepts any value, checks for zero at
runtime, and returns `Result::(Self, Text)` — `Ok` with the quotient or
`Err` with an error message. Use `div` when the divisor comes from a source
that the type system can't prove non-zero at compile-time.

## Comments

`#` starts a comment that goes to the end of the line:

```kata
# this is a comment
42 # comment next to code
```

Multiline comments use `#{ }#`:

```kata
#{
  Multiline comment.
  Can span multiple lines.
}#
```

## Literals

| Syntax | Type | Example |
|---------|------|---------|
| `42` | Int | `42` |
| `0xFF` | Int (hex) | `255` |
| `1_000` | Int | `1000` |
| `3.14` | Float | `3.14` |
| `"hello"` | Text | `"hello"` |
| `True` | Boolean | `True` |
| `False` | Boolean | `False` |
| `()` | Unit | `()` |

Integers have arbitrary precision (BigInt). There is no overflow.

## `echo!` and `show`

`echo!` prints text to the screen. Every type in Kata implements the `SHOW` interface, so `echo!` accepts any value directly:

```kata
echo!(42)
echo!(3.14)
echo!("hello")
echo!(True)
echo!(())
```

```
42
3.14
hello
True
()
```

`show` is the function that converts any value to `Text`. `echo!` already calls `show` internally, so you rarely need to call `show` explicitly — only when you want the text without printing. `show` works with all types — collections, tuples, enums, structs. We'll see more in chapter 8.

## Converting Text to a number

The `int` and `float` functions convert `Text` to `Int` and `Float` respectively. Since they can fail (the text might not be a valid number), they return `Result` — failure is a value, not a side effect. The `|` operator unwraps the `Ok` and provides a fallback if it fails (seen in chapter 2):

```kata
echo!(int("42") | 0)
echo!(int("0xFF") | 0)
echo!(float("3.14") | 0.0)
echo!(+ (float("1.5") | 0.0) (float("2.5") | 0.0))
```

```
42
255
3.14
4.0
```

`int` and `float` are pure functions — they don't use `!` because they have no side effects. Failure is represented as `Result::(Int, Text)`, not as an action. `int` supports decimal, hexadecimal (`0x`), octal (`0o`), binary (`0b`), and underscores (`1_000`). `float` supports decimal and exponential notation (`1e10`).

## Concatenating text

The `+` operator also concatenates text:

```kata
echo!(+ "hello " "world")
```

```
hello world
```

## Next chapter

Now that you know how to write expressions, the next chapter shows how to give names to values with bindings and explore the language's primitive types. → [Chapter 4](04-bindings-types.md)