# Capítulo 14 — Otimizações

Kata aplica otimizações automaticamente. O programador escreve código declarativo; o compilador transforma para rodar sem estourar a pilha e sem coleções intermediárias.

## TCO — Tail Call Optimization

Quando a chamada recursiva é a última operação da função, o compilador a reescreve como um loop. A pilha não cresce:

```kata
fat_tail :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat_tail (- n 1) (* n acc)

fatorial :: Int => Int
lambda n: fat_tail n 1

action main
    echo!(fatorial 5)
main!()
```

```
120
```

A chamada `fat_tail (- n 1) (* n acc)` está em posição de cauda — não há nada depois dela. O compilador detecta isso e elimina o crescimento da pilha.

## TRMA — Tail Recursion Modulo Associativity

E se a recursão não for de cauda? Quando a chamada está dentro de uma operação associativa (`+`, `*`), o otimizador reescreve com acumulador:

```kata
soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))
```

A chamada `soma (- n 1)` está dentro de `+ n (...)` — não é cauda. Mas `+` é associativo, então o compilador reescreve para:

```kata
soma_acc :: Int Int => Int
lambda 0 acc: acc
lambda n acc: soma_acc (- n 1) (+ acc n)
```

Agora é cauda — TCO aplica. O programador não precisa fazer nada:

```kata
echo!(soma 1000000)
```

```
500000500000
```

Sem TRMA, 1 milhão de chamadas recursivas estourariam a pilha. Com TRMA, roda sem problema.

## Stream fusion

`map`, `filter`, e `fold` são interceptados pelo compilador e geram nós especiais na TAST. Quando você compõe `filter(f, map(g, arr))`, o otimizador funde os dois em um único loop — sem coleções intermediárias:

```kata
processar :: [Int] => Int
lambda arr: fold (+) 0 (filter (lambda x: > x 0) (map (* _ 2) arr))

echo!(processar [1 -2 3])
```

```
8
```

Sem fusion: `map` cria uma lista intermediária, `filter` cria outra. Com fusion: um único percurso da lista, aplicando `* 2` e filtrando simultaneamente.

## `constant` — avaliação em compile-time

`constant` avalia a expressão durante a compilação e embute o resultado no binário:

```kata
fatorial :: Int => Int
lambda 0: 1
lambda n: * n (fatorial (- n 1))

constant fatorial_10 := fatorial 10

echo!(fatorial_10)
```

```
3628800
```

O cálculo `fatorial 10` roda em compile-time. No binário, `fatorial_10` é o literal `3628800` — não há chamadas de função em runtime.

## `@cache` — memoização

`@cache{strategy: "LRU"}` intercepta chamadas repetidas e armazena resultados em uma tabela hash:

```kata
@cache{strategy: "LRU"}
dobro :: Int => Int
lambda n: * n 2

echo!(dobro 5)
echo!(dobro 5)
```

```
10
10
```

A segunda chamada com o mesmo argumento retorna o valor cacheado sem reexecutar o body.

> **Limitação atual:** `@cache` suporta apenas funções `Int => Int`. O diagnostic rejeita outros tipos de parâmetro ou retorno com uma mensagem clara.

## Próximo capítulo

Otimizações são automáticas. O próximo capítulo mostra o REPL interativo — explorar a linguagem sem criar arquivos. → [Capítulo 15](15-repl.md)