# Chapter 13 — Modules

Kata organizes code into modules. Each `.kata` file is a module. Items exported with `export` are visible to importers; non-exported items are private.

## Exporting

Declare functions, actions, and types normally. At the end, `export` lists what is public:

```kata
# mod_math.kata
dobrar :: Int => Int
lambda x: + x x

triplicar :: Int => Int
lambda x: * x 3

quadrupar :: Int => Int
lambda x: * x 4

export dobrar triplicar
```

`quadrupar` is not in the list — it is private to the module.

## Importing an entire module

`import mod` brings the module into scope. Access via the `mod.fn` prefix:

```kata
import mod_math

action main
    let dobro := mod_math.dobrar 21
    let triplo := mod_math.triplicar 21
    echo!(dobro)
    echo!(triplo)
main!()
```

```
42
63
```

## Selective import

`import mod.(item)` brings specific items into direct scope, without a prefix:

```kata
import mod_math.(triplicar)

action main
    let triplo := triplicar 21
    echo!(triplo)
main!()
```

```
63
```

## Import with alias

Rename items on import to avoid collisions:

```kata
import mod_math.(dobrar as d, triplicar as t)

action main
    let dobro := d 21
    let triplo := t 21
    echo!(dobro)
    echo!(triplo)
main!()
```

```
42
63
```

## Path resolution

The loader looks for modules in two places:

1. **Importer file's directory** (`entry_dir`) — the directory of the file doing the `import`.
2. **Stdlib** — the standard library, as fallback.

For `import mod_math`, the loader looks for `mod_math.kata` in the importer's directory. Nested paths (`import subdir.mod`) follow the directory structure.

## `mod.kata` — directory as module

A directory can be imported as a unit if it contains `mod.kata`:

```
projeto/
  main.kata        → import math.(dobrar)
  math/
    mod.kata       → dobrar :: Int => Int ...
    algebra.kata   → ...
```

```kata
# math/mod.kata
dobrar :: Int => Int
lambda x: * x 2
export dobrar
```

```kata
# main.kata
import math.(dobrar)

dobrar 21
```

```
42
```

Without `mod.kata`, `import math` is an error. But `import math.algebra` works without `mod.kata` — direct submodules do not need it.

## `super.` — importing from parent directories

`super.` goes up one level in the directory tree, relative to the file doing the import:

```
projeto/
  utils.kata       → helper :: Int => Int ...
  math/
    algebra.kata   → import super.utils.(helper)
```

```kata
# utils.kata
helper :: Int => Int
lambda x: + x 1
export helper
```

```kata
# math/algebra.kata
import super.utils.(helper)

helper 41
```

```
42
```

`super.super.X` goes up two levels. `super` only resolves in the resolved directory — no fallback to stdlib.

## `stdlib.` — forcing the standard library

When there is a local module with the same name as the stdlib, `stdlib.` forces the stdlib:

```kata
import stdlib.math.(pi)

pi
```

```
3.141592653589793
```

Without the `stdlib.` prefix, `import math` would load the local module (if it exists). With `stdlib.`, it ignores the local one and goes straight to the stdlib.

## Next chapter

Modules organize code. The next chapter shows the optimizations the compiler applies automatically — TCO, TRMA, stream fusion, and `@cache`. → [Chapter 14](14-otimizacoes.md)