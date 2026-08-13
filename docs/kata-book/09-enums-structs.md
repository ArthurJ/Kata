# Capítulo 9 — Enums e Structs

Kata modela dados com tipos algébricos. `enum` define tipos soma (OR) — um valor é uma de várias variantes. `data` define tipos produto (AND) — um valor combina vários campos.

## `enum` — tipos soma

Cada variante em uma linha indentada. Sem `|` separador:

```kata
enum Cor
    Verde
    Amarelo
    Vermelho

echo!(show Verde)
echo!(show Cor::Amarelo)
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

echo!(show (Some 42))
echo!(show None)
```

```
Some(42)
None
```

`Some 42` constrói a variante com payload. Os parênteses em `show (Some 42)` são necessários — `show` tem aridade 1 e `Some 42` precisa ser agrupado.

## `match` em enums

Pattern matching desempacota o payload:

```kata
enum Optional
    Some(Int)
    None

match (Some 42)
    Some v: echo!(show v)
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
    echo!(show p.nome)
    echo!(show p.idade)
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
    echo!(show + (* p.x p.x) (* p.y p.y))
main!()
```

```
25
```

A distância ao quadrado da origem: `3² + 4² = 25`.

## Próximo capítulo

Tipos de dados são o lado puro. O último capítulo entra no mundo da concorrência — `fork!`, canais, `select`, e comunicação entre fibers.