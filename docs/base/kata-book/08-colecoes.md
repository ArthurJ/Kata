# Capítulo 8 — Coleções

Kata tem quatro coleções principais, cada uma com delimitadores próprios. Todas são imutáveis e persistentes — modificar cria uma nova versão que compartilha estrutura com a original.

## List

Listas usam `[ ]`. São encadeadas (Cons) — imutabilidade de custo zero via partilha estrutural:

```kata
action main
    let lista := [1 2 3]
    echo!(lista)
main!()
```

```
[1, 2, 3]
```

`+` concatena listas:

```kata
echo!(+ [1 2] [3 4])
```

```
[1, 2, 3, 4]
```

## Tupla

Tuplas agrupam valores heterogêneos. Vírgula separa elementos. Parênteses obrigatórios:

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

Dicionários usam `{k: v}`. Chaves devem implementar `HASHABLE`:

```kata
action main
    let d := {"nome": "Ana"}
    echo!(d)
main!()
```

```
{"nome": "Ana"}
```

O `:` após a primeira entrada desambigua de Array. Para múltiplas entradas:

```kata
action main
    let d := {"nome": "Ana" "cidade": "São Paulo"}
    echo!(d)
main!()
```

```
{"cidade": "São Paulo", "nome": "Ana"}
```

Todos os valores em um Dict devem ter o mesmo tipo — `{"nome": "Ana" "idade": 30}` é um erro de tipo (mistura `Text` e `Int`).

## Set

Conjuntos usam `{| |}`. Não há ordem garantida:

```kata
action main
    let s := {|1 2 3|}
    echo!(s)
main!()
```

```
{|3, 2, 1|}
```

`+` une sets:

```kata
echo!(+ {|1 2|} {|3 4|})
```

```
{|3, 4, 2, 1|}
```

## Ranges

Ranges são preguiçosos — geram valores sob demanda. A sintaxe base é `[start..end]` (exclusivo) ou `[start..=end]` (inclusivo), com step default de 1:

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

Para um step diferente de 1, use a forma de três componentes `[start..step..end]` (exclusivo) ou `[start..step..=end]` (inclusivo):

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

Range decrescente com step negativo:

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

## `for` — iteração em actions

`for x in coleção` itera sobre qualquer coleção que implemente `ITERABLE`:

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

## `in` — pertinência

O operador `in` testa se um elemento pertence à coleção:

```kata
echo!((3 in [1 2 3]))
echo!((5 in [1 2 3]))
```

```
True
False
```

## Próximo capítulo

Coleções ficam mais poderosas com funções de ordem superior. O próximo capítulo mostra `map`, `filter`, `fold`, e o operador pipeline `|>`. → [Capítulo 9](09-hofs-closures.md)