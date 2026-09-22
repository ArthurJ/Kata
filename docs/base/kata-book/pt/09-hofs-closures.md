# Capítulo 9 — HOFs e Closures

Funções de ordem superior (HOFs) recebem ou retornam funções. Kata tem três builtins — `map`, `filter`, `fold` — e dois operadores pipeline: `|>` (forward) e `|N>` (limitado).

## `map`

`map` aplica uma função a cada elemento de uma coleção:

```kata
echo!(map (* _ 2) [1 2 3])
```

```
[2, 4, 6]
```

O `_` é um hole — um espaço a preencher. `* _ 2` cria uma closure que aguarda o argumento faltante. É equivalente a `lambda x: * x 2`, mas mais conciso.

## `filter`

`filter` seleciona elementos que satisfazem um predicado:

```kata
echo!(filter (lambda x: > x 0) [1 -2 3])
```

```
[1, 3]
```

Aqui usamos `lambda` em vez de hole porque a condição `> x 0` precisa nomear o parâmetro.

## `fold`

`fold` reduz uma coleção a um único valor, acumulando:

```kata
echo!(fold (+) 0 [1 2 3])
```

```
6
```

`fold` recebe três argumentos: a função `(+)`, o valor inicial `0`, e a coleção. A função `+` é passada como valor — agrupada por parênteses para não ser confundida com aplicação.

## Holes — currying explícito

O `_` no lugar de um argumento congela a aplicação, gerando uma closure:

```kata
action main
    let soma_dez := + 10 _
    echo!(soma_dez 5)
main!()
```

```
15
```

`+ 10 _` cria uma closure de aridade 1 que espera o segundo argumento. `soma_dez 5` fornece o argumento faltante.

## Pipeline `|>`

O pipeline passa o resultado da esquerda como argumento da função à direita. Associatividade à esquerda:

```kata
echo!(5 |> + 1 _ |> * 2 _)
```

```
12
```

Equivalente a `* 2 (+ 1 5)` = `* 2 6` = `12`. O `_` marca onde o resultado da esquerda entra.

### Pipe sem Hole

Se a função à direita não tem `_`, o resultado da esquerda é injetado como primeiro argumento:

```kata
echo!(5 |> show)
```

```
5
```

Equivalente a `show 5`. Útil quando a função já tem aridade 1 e não precisa de hole.

## Pipe limitado `|N>`

O pipe limitado combina pipeline com lazy evaluation — processa apenas os primeiros N elementos da coleção:

```kata
action main
    var r := [0 1 2 3 4 5 6 7 8 9] |3> map (+ _ 1) _
    echo!(show r)
main!()
```

```
[1, 2, 3]
```

`|3>` pega os 3 primeiros elementos antes de aplicar `map`. Com um range infinito ou muito grande, só os primeiros N são consumidos — o restante nunca é avaliado:

```kata
action main
    var r := [0..1..1000000] |5> map (+ _ 1) _
    echo!(show r)
main!()
```

```
[1, 2, 3, 4, 5]
```

`|N>` funciona com `map`, `filter`, e `fold`. Com `filter`, o limite aplica-se antes do predicado — os N elementos são tomados da fonte e filtrados depois:

```kata
echo!([0 1 2 3 4 5] |3> filter (> _ 2) _)
```

```
[]
```

Os 3 primeiros (0, 1, 2) são tomados e nenhum passa no filtro `> _ 2`.

## Próximo capítulo

HOFs transformam coleções. O próximo capítulo mostra como definir seus próprios tipos com `enum` (tipos soma) e `data` (tipos produto). → [Capítulo 10](10-enums-structs.md)