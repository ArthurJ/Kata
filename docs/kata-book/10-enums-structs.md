# Capítulo 10 — Enums e Structs

Kata modela dados com tipos algébricos. `enum` define tipos soma (OR) — um valor é uma de várias variantes. `data` define tipos produto (AND) — um valor combina vários campos.

## `enum` — tipos soma

Cada variante em uma linha indentada. Sem `|` separador:

```kata
enum Cor
    Verde
    Amarelo
    Vermelho

echo!(Verde)
echo!(Cor::Amarelo)
```

```
Verde
Amarelo
```

Variantes unitárias (sem payload) ficam disponíveis sem qualificação. `Verde` e `Cor::Verde` são o mesmo valor.

## Variantes com payload

Variantes podem carregar dados. `Optional` do prelude tem `Some(T)` e `None`:

```kata
enum Optional
    Some(Int)
    None

echo!((Some 42))
echo!(None)
```

```
Some(42)
None
```

`Some 42` constrói a variante com payload. Os parênteses em `echo!(Some 42)` são necessários — `echo!` tem aridade 1 e `Some 42` precisa ser agrupado.

## `match` em enums

Pattern matching desempacota o payload:

```kata
enum Optional
    Some(Int)
    None

match (Some 42)
    Some v: echo!(v)
    None: echo!("nada")
```

```
42
```

O padrão `Some v` extrai o payload para `v`. O compilador verifica exaustividade — você precisa cobrir todas as variantes.

## `data` — tipos produto

`data` define um struct com campos nomeados. Tipagem dos campos via `::`:

```kata
data Pessoa (nome::Text idade::Int)

action main
    let p := Pessoa "João" 30
    echo!(p.nome)
    echo!(p.idade)
main!()
```

```
João
30
```

Acesso a campos com `.` — `p.nome` lê o campo `nome`. A construção é posicional: `Pessoa "João" 30` passa os argumentos na ordem declarada.

## Combinando tudo

```kata
data Ponto (x::Int y::Int)

action main
    let p := Ponto 3 4
    echo!(+ (* p.x p.x) (* p.y p.y))
main!()
```

```
25
```

A distância ao quadrado da origem: `3² + 4² = 25`.

## `?` — short-circuit em Actions

O operador `?` desempacota `Result` e `Optional` dentro de Actions. Se o valor for `Ok(v)` ou `Some(v)`, devolve `v` e continua. Se for `Err(e)` ou `None`, aborta a action com `return Err(e)` ou `return None`:

```kata
action parse_num (s::Text) => Result::(Int, Text)
    let n := int s ?
    Ok n

action main => Unit
    echo!(show (parse_num!("42")))
    echo!(show (parse_num!("abc")))
main!()
```

```
Ok(42)
Err("número inválido")
```

`int s` retorna `Result::(Int, Text)`. O `?` desempacota o `Ok` e liga `n` ao valor interno. Se `int s` falha, `?` aborta a action — a linha `Ok n` nunca executa, e o `Err` propaga como retorno.

Sem `?`, o equivalente seria:

```kata
match (int s)
    Ok v: Ok v
    Err e: Err e
```

O `?` só funciona dentro de Actions — ele precisa de um `return` para abortar. Em funções puras, use `|` (fallback) ou `match` explícito.

## `|` — fallback (coalescência)

O operador `|` é um `match` sintético sobre enums. A regra é geral, não específica de `Result` ou `Optional`:

- Variantes **não-cauda** (todas exceto a última) devem ter payload — o `|` desempacota e devolve o valor
- A **cauda** (última variante) ativa o fallback — avalia a expressão da direita. Se tiver payload, descarta

Diferente de `?`, não aborta — é uma expressão pura, funciona em funções e Actions.

Com `Optional`, a cauda `None` não tem payload:

```kata
echo!(show (Some 42 | 99))
echo!(show (None | 99))
```

```
42
99
```

`Some 42 | 99` desempacota `42`. `None | 99` cai na cauda e avalia o fallback `99`.

Com `Result`, a cauda `Err` tem payload — mas é descartada. Você escolheu `|` em vez de `match`, indicando que não precisa do erro:

```kata
echo!(show (Ok 42 | 0))
echo!(show (Err "err" | 99))
```

```
42
99
```

O `|` funciona com qualquer enum do usuário que respeite a regra. Todas as variantes não-cauda precisam ter payload; a cauda pode ou não ter:

```kata
enum Light
    Red(Int)
    Green(Int)
    Off

echo!(show (Light::Red 42 | 0))
echo!(show (Light::Green 7 | 0))
echo!(show (Light::Off | 0))
```

```
42
7
0
```

`Red` e `Green` são não-cauda com payload — desempacotam. `Off` é a cauda — avalia o fallback. Se uma variante não-cauda fosse unitária (sem payload), o compilador rejeitaria — não há nada para desempacotar.

`|` é útil para dar um valor default quando uma operação pode falhar:

```kata
echo!(show ((int "42") | 0))
echo!(show ((int "abc") | 0))
```

```
42
0
```

### `?` vs `|`

| | `?` | `|` |
|---|---|---|
| Aborta? | Sim — `return Err(e)` | Não — avalia rhs |
| Contexto | Só Actions | Actions e funções puras |
| Acesso ao erro | Sim — propaga | Não — descarta |
| Sintaxe | `expr ?` | `lhs | rhs` |

Use `?` quando quer propagar o erro para quem chamou. Use `|` quando quer um valor default e não se importa com o erro.

## Próximo capítulo

Tipos de dados são o lado puro. O último capítulo entra no mundo da concorrência — `fork!`, canais, `select`, e comunicação entre fibers.