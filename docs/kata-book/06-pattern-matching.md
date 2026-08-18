# Capítulo 6 — Pattern Matching

Kata não tem `if`. Condicionais são expressas via pattern matching e guards. Isto garante exaustividade — o compilador verifica que todos os casos estão cobertos.

## `match`

`match` examina um valor e executa o braço correspondente à sua forma:

```kata
match True
    Boolean::True: echo!("sim")
    Boolean::False: echo!("não")
```

```
sim
```

Cada braço tem um padrão à esquerda do `:` e uma expressão à direita. O primeiro padrão que encaixa vence.

## Desempacotando valores

Padrões podem extrair o payload de variantes. `Result` tem `Ok(T)` e `Err(E)`:

```kata
match (div 10 3)
    Result::Ok v: echo!(v)
    Result::Err e: echo!("erro")
```

```
3
```

O padrão `Result::Ok v` casa quando o valor é `Ok` e liga o payload à variável `v`.

## Guards em lambda

Dentro de funções, guards substituem `if/else`. Um guard é uma condição booleana após o parâmetro:

```kata
abs :: Int => Int
lambda x:
    > x 0: x
    otherwise: - 0 x

echo!(abs 5)
echo!(abs (- 0 5))
```

```
5
5
```

O primeiro guard `> x 0` testa se `x` é positivo. `otherwise` é o fallback obrigatório quando há guards — garante cobertura total.

## `with` — computações prévias

Às vezes um guard precisa de um valor intermediário. O bloco `with` declara bindings visíveis em todos os guards da cláusula:

```kata
classify :: Int => Text
lambda x:
    > doubled 10: "grande"
    otherwise: "pequeno"
    with
        doubled := * x 2

echo!(classify 3)
echo!(classify 6)
```

```
pequeno
grande
```

`doubled := * x 2` é avaliado antes dos guards, mesmo sendo escrito depois deles. A ordem é visual — `with` é pós-escrito mas pré-avaliado.

## Próximo capítulo

Tudo até aqui é código puro — sem efeitos colaterais. O próximo capítulo introduz actions, a barreira entre código puro e impuro, e os mecanismos de controle imperativo: `var`, `loop`, `break`.