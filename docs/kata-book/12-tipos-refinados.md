# Capítulo 12 — Tipos Refinados

Tipos refinados adicionam predicados a tipos existentes. Em vez de definir um novo tipo do zero, você refina um tipo base com uma condição que o valor deve satisfazer.

## Declarando um tipo refinado

A sintaxe `data (Base, predicado) as Nome` cria um tipo refinado. O `_` no predicado representa o valor sendo testado:

```kata
data (Int, > _ 0) as PositiveInt
```

Isto declara `PositiveInt` — um `Int` que deve ser maior que zero. O predicado é verificado em compile-time para literais e em runtime para valores dinâmicos.

## Ascription literal

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

## Smart constructor

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

## `refines` — delegação de interface

Um tipo refinado não herda automaticamente as operações do tipo base. `refines` delega uma interface ao tipo base:

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

Sem `refines NUM`, `+ a b` falharia — `+` não está definido para `PositiveInt`. Com `refines`, o dispatch tenta o tipo base `Int` e envolve o resultado no construtor falível, produzindo `Result::(PositiveInt, Err)`.

## Downcast com `::`

Para converter um tipo refinado de volta ao tipo base, use `::`:

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

O downcast é um no-op em runtime — mesmos bits, sem custo. O typeck verifica que o alvo é o tipo base.

## Próximo capítulo

Tipos refinados garantem invariantes em compile-time. O próximo capítulo mostra como organizar código em módulos com `import` e `export`. → [Capítulo 13](13-modulos.md)