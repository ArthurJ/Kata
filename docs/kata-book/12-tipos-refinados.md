# Capítulo 12 — Tipos Refinados e Alias

Kata oferece duas formas de criar um novo tipo nominal sobre um tipo existente: **tipos refinados** (com validação) e **alias** (sem validação). Ambos são zero-cost em runtime — mesmos bits, mesmo Cranelift type, sem wrapping.

## Tipos Refinados

A sintaxe `data (Base, predicado) as Nome` cria um tipo refinado. O `_` no predicado representa o valor sendo testado:

```kata
data (Int, > _ 0) as PositiveInt
```

Isto declara `PositiveInt` — um `Int` que deve ser maior que zero. O predicado é verificado em compile-time para literais e em runtime para valores dinâmicos.

### Ascription literal

Para literais que satisfazem o predicado, use `::` diretamente:

```kata
action main
    let x := 5::PositiveInt
    echo!(x)
main!()
```

```
5
```

O compilador valida o predicado em compile-time. Um literal negativo falha:

```kata
let x := (- 0 5)::PositiveInt   # erro de tipo: -5 não é > 0
```

### Smart constructor

Para valores dinâmicos, use o construtor falível. Ele retorna `Result`:

```kata
data (Int, > _ 0) as PositiveInt

action main
    let r := PositiveInt 42
    match r
        Ok v: echo!(v)
        Err _: echo!("erro")
main!()
```

```
42
```

Quando o valor não satisfaz o predicado, o construtor retorna `Err`:

```kata
action main
    let r := PositiveInt (- 0 5)
    match r
        Ok v: echo!(v)
        Err _: echo!("erro")
main!()
```

```
erro
```

## Famílias polimórficas

Os refineds que vimos até agora são **concretos**: `PositiveInt` refina
`Int`. Kata também suporta **famílias polimórficas** — refineds que
refinam uma *interface* inteira em vez de um tipo específico:

```kata
data (NUM, != _ (zero _)) as NonZero
```

`NonZero` refina `NUM` — a interface que `Int`, `Float` e `Rational`
implementam. O predicado `!= _ (zero _)` verifica que o valor é diferente
de `zero` do seu tipo (`0` para Int, `0.0` para Float, `rational 0` para
Rational). Existem três instâncias de `NonZero`: `NonZero::Int`,
`NonZero::Float` e `NonZero::Rational`.

### Ascription de literal

`5::NonZero` funciona como nos refineds concretos — o compilador valida
o predicado em compile-time. A instância concreta é inferida do tipo do
literal:

```kata
action main
    let x := 5::NonZero       # NonZero::Int
    let y := 3.0::NonZero     # NonZero::Float
    echo!(x::Int)
    echo!(y::Float)
main!()
```

```
5
3.0
```

O downcast `x::Int` extrai o valor base — é um no-op em runtime (mesmos
bits).

### Divisão segura com NonZero

O propósito principal de `NonZero` é garantir divisão segura em
compile-time. O operador `/` exige `NonZero` como divisor:

```kata
echo!(/ 10 (3::NonZero))           # 3 — divisão exata, sem Result
echo!(/ 10.0 (3.0::NonZero))       # 3.3333333333333335
echo!(// 10 (3::NonZero))          # 3 — divisão inteira, retorna Int
echo!(// 10.0 (3.0::NonZero))      # 3 — Float truncado para Int
echo!(mod 10 (3::NonZero))         # 1 — resto
```

Se o divisor fosse zero, a ascription `0::NonZero` falharia em
compile-time — o programa nem compila. Isto elimina uma classe inteira
de bugs de divisão por zero sem custo de runtime.

### Construtor falível

Para valores dinâmicos, o construtor `NonZero` retorna `Result`. A
instância concreta é determinada pelo tipo do argumento:

```kata
action main
    let r := NonZero 0
    match r
        Ok v: echo!(v::Int)
        Err _: echo!("zero rejeitado")
main!()
```

```
zero rejeitado
```

`NonZero 0` retorna `Err` porque `!= 0 (zero 0)` é falso. `NonZero 42`
retornaria `Ok` com `NonZero::Int`. `NonZero 3.0` retornaria `Ok` com
`NonZero::Float`.

### Cross-type com NonZero

Como `NonZero` é uma família sobre `NUM`, operações entre tipos diferentes
funcionam quando o divisor é qualificado:

```kata
echo!(/ 10.0 (3::NonZero))              # 3.333... (Float ÷ NonZero::Int)
echo!(mod (rational 10) (3::NonZero))   # 1 (Rational ÷ NonZero::Int)
```

O divisor `3::NonZero` é `NonZero::Int`. A divisão `/ 10.0` despacha para
a overload `Float × NonZero::Int → Float`. O compilador seleciona a
instância correta baseada no tipo do argumento.

## Alias — Newtype sem predicados

`alias` cria um novo tipo nominal distinto do original, mas sem validação.
É um *newtype* puro: mesmos bits, construtor infalível, custo zero em runtime.

```kata
alias Float as Altura
```

`Altura` é um tipo diferente de `Float` para o type checker. O construtor é
infalível — não retorna `Result`, porque não há predicado para falhar:

```kata
action main
    let a := Altura 1.75
    echo!(a)
main!()
```

### Quando usar alias vs tipo refinado

Use **tipo refinado** quando há uma condição que o valor deve satisfazer
(`> 0`, `/= 0`, `< 100`). Use **alias** quando quer apenas um nome distinto
para um tipo existente — por exemplo, para evitar confusão entre valores
com a mesma representação mas significados diferentes (`Altura` vs `Float`).

### Alias de tipo refinado

Um alias pode ter como alvo um tipo refinado. Nesse caso, o alias herda
os predicados e torna-se refinado também:

```kata
data (Float, > _ 0.0) as PositiveFloat
alias PositiveFloat as Peso
```

`Peso` é um tipo refinado: tem os mesmos predicados que `PositiveFloat`,
o construtor é falível (retorna `Result`), e pode declarar `refines`.
Internamente, o compilador segue a cadeia: `Peso` → `alias_of` →
`PositiveFloat` → `alias_of` → `Float`.

## `refines` — delegação de interface

Um tipo refinado não herda automaticamente as operações do tipo base.
`refines` delega uma interface ao tipo base:

```kata
data (Int, > _ 0) as PositiveInt

PositiveInt refines NUM

action main
    let a := 5::PositiveInt
    let b := 3::PositiveInt
    let soma := PositiveInt (+ a b)
    match soma
        Ok v: echo!(v)
        Err _: echo!("erro")
main!()
```

```
8
```

Sem `refines NUM`, `+ a b` falharia — `+` não está definido para
`PositiveInt`. Com `refines`, o dispatch tenta o tipo base `Int` e envolve
o resultado no construtor falível, produzindo `Result::(PositiveInt, Err)`.

O fallback de `refines` segue a cadeia de `alias_of`. Por isso, se `Peso`
é alias de `PositiveFloat` que tem `refines NUM`, então `+ a b` onde
`a` e `b` são `Peso` também funciona — o fallback percorre
`Peso → PositiveFloat → refines NUM → Float`.

Alias puro (sem `refines`) **não** interoperaciona com o tipo base no
dispatch. `Altura + 3.0` falha porque `Altura` é nominalmente distinta de
`Float`. Para interoperar, use downcast explícito.

## Downcast com `::`

Para converter um tipo refinado (ou alias) de volta ao tipo base, use `::`:

```kata
action main
    let a := 5::PositiveInt
    let n := a::Int
    echo!(n)
main!()
```

```
5
```

O downcast é um no-op em runtime — mesmos bits, sem custo. O typeck
verifica que o alvo é o tipo base (ou um tipo na cadeia de `alias_of`).

## Domínio finito — `match` sem `otherwise`

Quando os predicados de um refinado definem um intervalo finito sobre um
tipo discreto (Int, Rational, Boolean), o compilador **enumera o domínio**
e verifica cobertura caso a caso. Se todos os valores possíveis têm um
braço, o `otherwise` é dispensável:

```kata
data (Int, > _ 0, < _ 3) as UmOuDois

describe :: UmOuDois => Text
lambda n:
    match n
        1: "um"
        2: "dois"

action main
    echo!(describe (1::UmOuDois))
    echo!(describe (2::UmOuDois))
main!()
```

```
um
dois
```

O tipo `UmOuDois` tem exatamente dois valores possíveis: `1` e `2`. Os dois
braços cobrem o domínio inteiro e o `match` é exaustivo sem fallback.

Literais fora do domínio são rejeitados em compile-time: `3::UmOuDois` é um
erro de tipo, porque 3 viola `< _ 3`. O domínio existe sempre que os
predicados são bounds com literais (`> _ N`, `< _ M`, `= _ K`) sobre Int,
Rational ou Boolean.

Refineds sobre Rational funcionam da mesma forma — os braços casam com
literais racionais construídos via `rational`:

```kata
data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

describe :: RatUmOuDois => Text
lambda n:
    match n
        rational 1: "um"
        rational 2: "dois"

action main
    let r := RatUmOuDois (rational 1)
    match r
        Ok v: echo!(describe v)
        Err _: echo!("erro")
main!()
```

```
um
```

## Próximo capítulo

Tipos refinados e alias garantem invariantes em compile-time. O próximo
capítulo mostra como organizar código em módulos com `import` e `export`.
→ [Capítulo 13](13-modulos.md)