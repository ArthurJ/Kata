# Chapter 9 — HOFs and Closures

Higher-order functions (HOFs) take or return functions. Kata has three builtins — `map`, `filter`, `fold` — and two pipeline operators: `|>` (forward) and `|N>` (limited).

## `map`

`map` applies a function to each element of a collection:

```kata
echo!(map (* _ 2) [1 2 3])
```

```
[2, 4, 6]
```

The `_` is a hole — a space to fill. `* _ 2` creates a closure that awaits the missing argument. It is equivalent to `lambda x: * x 2`, but more concise.

## `filter`

`filter` selects elements that satisfy a predicate:

```kata
echo!(filter (lambda x: > x 0) [1 -2 3])
```

```
[1, 3]
```

Here we use `lambda` instead of a hole because the condition `> x 0` needs to name the parameter.

## `fold`

`fold` reduces a collection to a single value by accumulating:

```kata
echo!(fold (+) 0 [1 2 3])
```

```
6
```

`fold` takes three arguments: the function `(+)`, the initial value `0`, and the collection. The `+` function is passed as a value — grouped by parentheses to avoid being confused with application.

## Holes — explicit currying

The `_` in place of an argument freezes the application, producing a closure:

```kata
action main
    let soma_dez := + 10 _
    echo!(soma_dez 5)
main!()
```

```
15
```

`+ 10 _` creates a closure of arity 1 that expects the second argument. `soma_dez 5` provides the missing argument.

## Pipeline `|>`

The pipeline passes the result of the left side as an argument to the function on the right. Left-associative:

```kata
echo!(5 |> + 1 _ |> * 2 _)
```

```
12
```

Equivalent to `* 2 (+ 1 5)` = `* 2 6` = `12`. The `_` marks where the result from the left enters.

### Pipe without a Hole

If the function on the right has no `_`, the result from the left is injected as the first argument:

```kata
echo!(5 |> show)
```

```
5
```

Equivalent to `show 5`. Useful when the function already has arity 1 and needs no hole.

## Limited pipe `|N>`

The limited pipe combines pipelining with lazy evaluation — it processes only the first N elements of the collection:

```kata
action main
    var r := [0 1 2 3 4 5 6 7 8 9] |3> map (+ _ 1) _
    echo!(show r)
main!()
```

```
[1, 2, 3]
```

`|3>` takes the first 3 elements before applying `map`. With an infinite or very large range, only the first N elements are consumed — the rest is never evaluated:

```kata
action main
    var r := [0..1..1000000] |5> map (+ _ 1) _
    echo!(show r)
main!()
```

```
[1, 2, 3, 4, 5]
```

`|N>` works with `map`, `filter`, and `fold`. With `filter`, the limit applies before the predicate — the N elements are taken from the source and filtered afterward:

```kata
echo!([0 1 2 3 4 5] |3> filter (> _ 2) _)
```

```
[]
```

The first 3 elements (0, 1, 2) are taken and none passes the filter `> _ 2`.

## Next chapter

HOFs transform collections. The next chapter shows how to define your own types with `enum` (sum types) and `data` (product types). → [Chapter 10](10-enums-structs.md)