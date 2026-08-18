# Capítulo 5 — Funções

Funções em Kata são puras: recebem argumentos, computam um resultado, e não têm efeitos colaterais. A definição tem duas partes — assinatura e corpo.

## Assinatura e corpo

A assinatura declara o nome e os tipos. O corpo usa `lambda` com os parâmetros:

```kata
dobrar :: Int => Int
lambda x: * x 2

echo!(dobrar 21)
```

```
42
```

A assinatura `dobrar :: Int => Int` lê-se: `dobrar` recebe um `Int` e retorna um `Int`. O `::` etiqueta o nome com seu tipo. O `=>` separa argumentos do retorno.

## Múltiplas cláusulas

Uma função pode ter várias cláusulas `lambda`. A primeira que encaixa nos argumentos vence:

```kata
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat (- n 1) (* n acc)

echo!(fat 5 1)
```

```
120
```

A primeira cláusula casa quando o primeiro argumento é `0` — retorna o acumulador. A segunda cláusula casa com qualquer outro valor `n` — chama `fat` recursivamente com `n-1` e `n * acc`.

## Recursão

A recursão é o mecanismo de iteração no domínio puro. Não há `while` ou `for` em funções puras.

```kata
fib :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (fib (- n 1)) (fib (- n 2))

echo!(fib 10)
```

```
55
```

## Recursão de cauda

Quando a chamada recursiva é a última operação da função, o compilador otimiza para não crescer a pilha. O fatorial acima usa um acumulador (`acc`) — a chamada `fat (- n 1) (* n acc)` é a última operação, então é recursão de cauda:

```kata
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat (- n 1) (* n acc)

echo!(fat 100000 1)
```

```
282422940796034787429342157802...
```

Mesmo com 100000 chamadas recursivas, a pilha não estoura. O compilador reescreve a recursão de cauda como um loop interno.

## Próximo capítulo

Funções ficam mais poderosas com pattern matching. O próximo capítulo mostra como ramificar a lógica com `match`, guards, e `otherwise`.