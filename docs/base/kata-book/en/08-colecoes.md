# Chapter 8 — Collections

Kata has four main collections, each with its own delimiters. All are immutable and persistent — modifying one creates a new version that shares structure with the original.

## List

Lists use `[ ]`. They are linked (Cons) — zero-cost immutability via structural sharing:

```kata
action main
    let lista := [1 2 3]
    echo!(lista)
main!()
```

```
[1, 2, 3]
```

`+` concatenates lists:

```kata
echo!(+ [1 2] [3 4])
```

```
[1, 2, 3, 4]
```

## Tuple

Tuples group heterogeneous values. Commas separate elements. Parentheses are required:

```kata
action main
    let t := (1, "a", True)
    echo!(t)
main!()
```

```
(1, "a", True)
```

## Dict

Dictionaries use `{k: v}`. Keys must implement `HASHABLE`:

```kata
action main
    let d := {"nome": "Ana"}
    echo!(d)
main!()
```

```
{"nome": "Ana"}
```

The `:` after the first entry disambiguates it from an Array. For multiple entries:

```kata
action main
    let d := {"nome": "Ana" "cidade": "São Paulo"}
    echo!(d)
main!()
```

```
{"cidade": "São Paulo", "nome": "Ana"}
```

All values in a Dict must have the same type — `{"nome": "Ana" "idade": 30}` is a type error (mixing `Text` and `Int`).

## Set

Sets use `{| |}`. There is no guaranteed order:

```kata
action main
    let s := {|1 2 3|}
    echo!(s)
main!()
```

```
{|3, 2, 1|}
```

`+` unions sets:

```kata
echo!(+ {|1 2|} {|3 4|})
```

```
{|3, 4, 2, 1|}
```

## Ranges

Ranges are lazy — they generate values on demand. The base syntax is `[start..end]` (exclusive) or `[start..=end]` (inclusive), with a default step of 1:

```kata
action main
    for x in [1..5]
        echo!(x)
main!()
```

```
1
2
3
4
```

```kata
action main
    for x in [0..=3]
        echo!(x)
main!()
```

```
0
1
2
3
```

For a step other than 1, use the three-component form `[start..step..end]` (exclusive) or `[start..step..=end]` (inclusive):

```kata
action main
    for x in [0..2..10]
        echo!(x)
main!()
```

```
0
2
4
6
8
```

Descending range with a negative step:

```kata
action main
    for x in [10..-1..=0]
        echo!(x)
main!()
```

```
10
9
8
7
6
5
4
3
2
1
0
```

## `for` — iteration in actions

`for x in coleção` iterates over any builtin collection (List, Array, Range, Text, Dict, Set):

```kata
action main
    for x in [1 2 3]
        echo!(x)
main!()
```

```
1
2
3
```

## `in` — membership

The `in` operator tests whether an element belongs to a collection:

```kata
echo!((3 in [1 2 3]))
echo!((5 in [1 2 3]))
```

```
True
False
```

## Next chapter

Collections become more powerful with higher-order functions. The next chapter shows `map`, `filter`, `fold`, and the pipeline operator `|>`. → [Chapter 9](09-hofs-closures.md)