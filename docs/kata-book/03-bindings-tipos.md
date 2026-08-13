# Capítulo 3 — Bindings e Tipos

Bindings dão nomes a valores. Kata tem dois tipos de binding no nível de módulo e dentro de funções.

## `constant` — constantes de módulo

No top-level de um arquivo, use `constant` para declarar valores avaliados em compile-time:

```kata
constant pi := 3.14
constant nome := "Kata"

echo!(show pi)
echo!(show nome)
```

```
3.14
Kata
```

`constant` é avaliado quando o programa compila, não quando executa. O valor é embutido no binário.

## `let` — bindings locais

Dentro de funções e actions, use `let` para bindings imutáveis:

```kata
action main
    let x := 42
    let y := 99
    echo!(show + x y)
main!()
```

```
141
```

`let` não existe no top-level de arquivos — apenas dentro de actions e funções. No top-level, use `constant`.

## Shadowing

Um novo `let` com o mesmo nome sobrepõe o anterior:

```kata
action main
    let x := 42
    let x := 99
    echo!(show x)
main!()
```

```
99
```

O `x` original deixou de existir — o novo `x` toma seu lugar.

## Tipos primitivos

| Tipo | Descrição | Exemplo |
|------|-----------|---------|
| `Int` | Inteiro de precisão arbitrária | `42` |
| `Float` | Ponto flutuante 64-bit | `3.14` |
| `Text` | Texto (string) | `"hello"` |
| `Boolean` | Verdadeiro ou falso | `True` |
| `Unit` | Ausência de valor | `()` |
| `Rational` | Número racional exato | `3.14::Rational` |

Inteiros têm precisão arbitrária. Não há overflow:

```kata
echo!(show * 99999999999999999999 99999999999999999999)
```

```
9999999999999999999800000000000000000001
```

## Anotação de tipo com `::`

O operador `::` anexa um tipo a uma expressão. Útil quando você quer ser explícito:

```kata
action main
    let x := 42 :: Int
    echo!(show x)
main!()
```

```
42
```

## Rational — precisão exata

Floats têm imprecisão inerente. `Rational` é exato — `1/3 * 3 = 1`, não `0.999...`:

```kata
echo!(show 3.14::Rational)
```

```
3.14
```

Use `::Rational` para converter um literal em racional. O texto bruto do literal é preservado — não há passagem por `f64`, então não há perda de precisão na conversão.

## Próximo capítulo

Agora que você sabe dar nomes a valores, o próximo capítulo mostra como definir funções com assinaturas, múltiplas cláusulas, e recursão.