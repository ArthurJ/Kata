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

## `data` com type params — generics paramétricos

`data` pode ser parametrizado por type params. Type params são detectados
implicitamente: PascalCase em posição de tipo nos fields (como `Ok(T)` em
enums). Não há lista explícita `::(...)` na declaração — a instanciação
`::(...)` é usada nos call sites.

### Forma compartilhada — type param com bound

```kata
data Complex (re::T im::T) where T implements SCALAR
```

- `(re::T im::T)` — campos com type param `T`.
- `where T implements SCALAR` — bound: `T` precisa implementar `SCALAR`.
- `T` é o mesmo tipo em ambos os campos — `Complex 3 4` tipa como
  `Complex::(Int, Int)`, `Complex 1.0 2.0` como `Complex::(Float, Float)`.
- `Complex "a" "b"` falha — `Text` não implementa `SCALAR`.

A instanciação `Complex::(Int, Int)` aparece automaticamente quando o
construtor despacha: o monomorphizer cria métodos concretos on-demand
para cada combinação de type args usada.

### Forma independente — vars anônimas com bound

```kata
data Par (first::SCALAR scd::SCALAR)
```

- `SCALAR` na posição de tipo do campo é interpretado como "var fresca
  anônima com bound `SCALAR`". Cada ocorrência é uma var distinta.
- Permite tipos diferentes em cada campo: `Par 3 4.0` aceito (`Int` e
  `Float` ambos implementam `SCALAR`).
- É açúcar para bounds independentes, não para params compartilhados.

### Forma livre — type param sem bound

```kata
data Par (first::A second::B)
```

- `A` e `B` são type params livres (sem bound). PascalCase em posição de
  tipo, detectados no pass0.
- Construtor aceita qualquer par de tipos: `Par 3 "hello"` é válido.

### Instanciação

A instanciação usa `::(...)` — **1 type arg por ocorrência de type param
nos fields, não por variável distinta.**

```kata
data Complex (re::T im::T) where T implements SCALAR
# 2 ocorrências de T → 2 type args
# Complex::(Int, Int)         — re::Int, im::Int
# Complex::(Float, Float)     — re::Float, im::Float

data Pair (fst::A scd::B)
# 2 params independentes → 2 type args
# Pair::(Int, Text)
```

`::(...)` é instanciação, **não** declaração. Escrever
`data Complex::(T) (re::T im::T)` é erro de sintaxe.

### Implementando interfaces para tipos genéricos

Métodos são definidos para uma instanciação específica:

```kata
data Complex (re::T im::T) where T implements SCALAR

Complex::(Float, Float) implements RING
    + :: Complex::(Float, Float) Complex::(Float, Float) => Complex::(Float, Float)
    lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
```

O monomorphizer instancia o corpo substituindo `T` pelo tipo concreto
(`Float`), e `+ a.re b.re` despacha para `+ :: Float Float => Float`.

### Exemplo completo

```kata
data Pair (first::T second::T) where T implements NUM

Pair::(Int, Int) implements EQ
    = :: Pair::(Int, Int) Pair::(Int, Int) => Boolean
    lambda a b: and (= a.first b.first) (= a.second b.second)

action main
    let p := Pair 3 4
    let q := Pair 3 4
    echo!(= p q)
main!()
```

```
True
```

## `?` — short-circuit em Actions

O operador `?` desempacota `Result` e `Optional` dentro de Actions. Se o valor for `Ok(v)` ou `Some(v)`, devolve `v` e continua. Se for `Err(e)` ou `None`, aborta a action com `return Err(e)` ou `return None`:

```kata
action parse_num (s::Text) => Result::(Int, Text)
    let n := int(s) ?
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

`int(s)` retorna `Result::(Int, Text)`. O `?` desempacota o `Ok` e liga `n` ao valor interno. Se `int(s)` falha, `?` aborta a action — a linha `Ok n` nunca executa, e o `Err` propaga como retorno.

Sem `?`, o equivalente seria:

```kata
match int(s)
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
echo!(show (int("42") | 0))
echo!(show (int("abc") | 0))
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

Tipos de dados são o lado puro. O próximo capítulo entra no mundo da concorrência — `fork!`, canais, `select`, e comunicação entre fibers. → [Capítulo 11](11-actions-avancadas.md)