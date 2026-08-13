# Capítulo 8 — HOFs e Closures

Funções de ordem superior (HOFs) recebem ou retornam funções. Kata tem três builtins — `map`, `filter`, `fold` — e o operador pipeline `|>`.

## `map`

`map` aplica uma função a cada elemento de uma coleção:

```kata
echo!(show map (* _ 2) [1 2 3])
```

```
[2, 4, 6]
```

O `_` é um hole — um espaço a preencher. `* _ 2` cria uma closure que aguarda o argumento faltante. É equivalente a `lambda x: * x 2`, mas mais conciso.

## `filter`

`filter` seleciona elementos que satisfazem um predicado:

```kata
echo!(show filter (lambda x: > x 0) [1 -2 3])
```

```
[1, 3]
```

Aqui usamos `lambda` em vez de hole porque a condição `> x 0` precisa nomear o parâmetro.

## `fold`

`fold` reduz uma coleção a um único valor, acumulando:

```kata
echo!(show fold (+) 0 [1 2 3])
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
    echo!(show (soma_dez 5))
main!()
```

```
15
```

`+ 10 _` cria uma closure de aridade 1 que espera o segundo argumento. `soma_dez 5` fornece o argumento faltante.

## Pipeline `|>`

O pipeline passa o resultado da esquerda como argumento da função à direita. Associatividade à esquerda:

```kata
echo!(show (5 |> + 1 _ |> * 2 _))
```

```
12
```

Equivalente a `* 2 (+ 1 5)` = `* 2 6` = `12`. O `_` marca onde o resultado da esquerda entra. Sem `_`, o resultado é injetado como primeiro argumento.

## Próximo capítulo

HOFs transformam coleções. O próximo capítulo mostra como definir seus próprios tipos com `enum` (tipos soma) e `data` (tipos produto).