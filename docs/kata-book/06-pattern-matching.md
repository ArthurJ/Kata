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

`div` é a divisão dinâmica: retorna `Result` porque o divisor pode ser zero
— o type system não pode provar que 3 ≠ 0 em compile-time (3 é um `Int`
comum, não `NonZero`). O braço `Ok` recebe o quociente; o braço `Err`
recebe a mensagem de erro. A divisão exata `/` exige `NonZero` e não
retorna `Result` — ver capítulo 12.

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

`otherwise` nem sempre é necessário. Quando a disjunção dos guards cobre
todo o espaço de entrada de forma **provável**, o compilador aceita a
função sem fallback:

```kata
sinal :: Int => Int
lambda x:
    > x 0: 1
    < x 0: - 0 1
    = x 0: 0

action main
    echo!(sinal 42)
    echo!(sinal (- 0 7))
    echo!(sinal 0)
main!()
```

```
1
-1
0
```

Todo Int é maior, menor ou igual a zero — a disjunção dos três guards é
sempre verdadeira e o compilador prova isso estaticamente. Mas se faltar
um caso:

```kata
sinal :: Int => Int
lambda x:
    > x 0: 1
    < x 0: - 0 1
```

o compilador rejeita com `match não-exaustivo` — `0` não é coberto por
nenhum guard. Adicione o caso que falta ou `otherwise:` como fallback.

O compilador faz um esforço honesto para distinguir quando `otherwise` é
necessário: prova o que consegue provar e só solicita o fallback ao
desenvolvedor quando não consegue decidir — nunca aceita um `match`
potencialmente incompleto, nunca exige um fallback sabidamente
desnecessário.

Cláusulas redundantes também são erro: se um braço anterior já cobre todos
os valores de um braço posterior, o posterior é inalcançável e o
compilador emite um erro, indicando qual cláusula o tornou redundante.

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

### `with` cross-clause — bindings compartilhados

Quando uma função tem múltiplas cláusulas `lambda`, o `with` pode ser declarado no nível outer — após a última cláusula. Os bindings são injetados seletivamente em cada cláusula que os referencia:

```kata
classify :: Int => Text
lambda 0: tag_zero
lambda x:
    > doubled 10: "grande"
    otherwise: "pequeno"
with
    doubled := * x 2
    tag_zero := "zero"
```

```kata
echo!(classify 0)
echo!(classify 3)
echo!(classify 6)
```

```
zero
pequeno
grande
```

A cláusula `lambda 0` recebe `tag_zero` (referencia no body) mas não `doubled` (não referencia). A cláusula `lambda x` recebe `doubled` mas não `tag_zero`. A injeção é seletiva — cada binding só vai para as cláusulas que o usam, evitando erros de variável não vinculada quando os patterns diferem.

## Próximo capítulo

Tudo até aqui é código puro — sem efeitos colaterais. O próximo capítulo introduz actions, a barreira entre código puro e impuro, e os mecanismos de controle imperativo: `var`, `loop`, `break`. → [Capítulo 7](07-actions.md)