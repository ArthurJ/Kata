# Capítulo 6 — Actions

Tudo até aqui foi código puro — funções sem efeitos colaterais. Actions são o lado impuro: interagem com o mundo, têm estado mutável, e controlam fluxo imperativo.

## A barreira pure/impure

Funções puras não têm `!`, não têm loops, não têm estado mutável. Actions têm `!` na chamada, podem ter `var` e `loop`. O compilador enforce essa barreira em compile-time — não há como chamar uma action de dentro de uma função pura.

## Declarando e chamando actions

A declaração não usa `!` — só a chamada:

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

O `!` na chamada (`greet!()`) sinaliza impureza. Os parênteses passam argumentos como tupla — `greet!()` é uma tupla vazia.

## Actions com parâmetros

Actions recebem parâmetros nomeados com `nome::Tipo`:

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

echo!(show somar_acumulado!(5))
```

```
10
```

A assinatura `(n::Int) => Int` declara um parâmetro `n` do tipo `Int` e retorno `Int`. A chamada usa `!` e tupla: `somar_acumulado!(5)`.

## `var` — binding mutável

`var` cria um binding mutável (exclusivo de actions). Para atualizar o valor, use `nome := expr` — **sem** `var`:

```kata
action contar
    var i := 0
    loop
        i := + i 1
        echo!(show i)
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

`var i := 0` cria o binding. `i := + i 1` reatribui. Escrever `var i := + i 1` estaria **errado** — `var` sempre cria um novo binding que sombreia o anterior, não atualiza.

## `loop`, `break`, `continue`

`loop` é um laço infinito. `break` sai do laço. `continue` vai para a próxima iteração. A condição de saída usa `match` — não existe `if`:

```kata
match (> i 3)
    Boolean::True: break
    Boolean::False: continue
```

## Retorno implícito e `;`

A última expressão de uma action sem `;` é o retorno:

```kata
action calcular => Int
    let x := 5
    let y := + x 1
    y

echo!(show calcular!())
```

```
6
```

O `;` termina um statement e suprime o retorno — a action retorna `Unit`. Útil para múltiplas statements na mesma linha:

```kata
action test_semi
    let x := 5; echo!(show x)
    echo!("depois")

test_semi!()
```

```
5
depois
```

## Próximo capítulo

Actions têm estado e controle. O próximo capítulo introduz coleções — listas, tuplas, dicionários, sets — e como iterar sobre elas com `for`.