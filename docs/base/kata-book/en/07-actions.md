# Chapter 7 — Actions

Everything so far has been pure code — functions without side effects. Actions are the impure side: they interact with the world, have mutable state, and control imperative flow.

## The pure/impure barrier

Pure functions have no `!`, no loops, no mutable state. Actions have `!` in the call, can have `var` and `loop`. The compiler enforces this barrier at compile-time — there is no way to call an action from inside a pure function.

## Declaring and calling actions

The declaration does not use `!` — only the call does:

```kata
action greet
    echo!("olá")
    echo!("mundo")

greet!()
```

```
olá
mundo
```

The `!` in the call (`greet!()`) signals impurity. The parentheses pass arguments as a tuple — `greet!()` is an empty tuple.

## Actions with parameters

Actions receive named parameters with `nome::Tipo`:

```kata
action somar_acumulado (n::Int) => Int
    var acc := 0
    var i := 0
    loop
        acc := + acc i
        i := + i 1
        match (>= i n)
            Boolean::True: break
            Boolean::False: continue
    acc

echo!(somar_acumulado!(5))
```

```
10
```

The signature `(n::Int) => Int` declares a parameter `n` of type `Int` and return type `Int`. The call uses `!` and a tuple: `somar_acumulado!(5)`.

## `var` — mutable binding

`var` creates a mutable binding (exclusive to actions). To update the value, use `nome := expr` — **without** `var`:

```kata
action contar
    var i := 0
    loop
        i := + i 1
        echo!(i)
        match (> i 3)
            Boolean::True: break
            Boolean::False: continue
    echo!("fim")

contar!()
```

```
1
2
3
4
fim
```

`var i := 0` creates the binding. `i := + i 1` reassigns it. Writing `var i := + i 1` would be **wrong** — `var` always creates a new binding that shadows the previous one, it does not update.

## `loop`, `break`, `continue`

`loop` is an infinite loop. `break` exits the loop. `continue` goes to the next iteration. The exit condition uses `match` — there is no `if`:

```kata
match (> i 3)
    Boolean::True: break
    Boolean::False: continue
```

## Implicit return and `;`

The last expression of an action without `;` is the return value:

```kata
action calcular => Int
    let x := 5
    let y := + x 1
    y

echo!(calcular!())
```

```
6
```

The `;` terminates a statement and suppresses the return — the action returns `Unit`. Useful for multiple statements on the same line:

```kata
action test_semi
    let x := 5; echo!(x)
    echo!("depois")

test_semi!()
```

```
5
depois
```

## Next chapter

Actions have state and control. The next chapter introduces collections — lists, tuples, dictionaries, sets — and how to iterate over them with `for`. → [Chapter 8](08-colecoes.md)