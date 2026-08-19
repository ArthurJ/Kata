# Capítulo 4 — Bindings e Tipos

Bindings dão nomes a valores. Kata tem três tipos de binding: `constant` no nível de módulo, `let` para bindings locais imutáveis, e `var` para bindings locais mutáveis.

## `constant` — constantes de módulo

No top-level de um arquivo, use `constant` para declarar valores avaliados em compile-time:

```kata
constant pi := 3.14
constant nome := "Kata"

echo!(pi)
echo!(nome)
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
    echo!(+ x y)
main!()
```

```
141
```

`let` não existe no top-level de arquivos — apenas dentro de actions e funções. No top-level, use `constant`.

## `let` é único por escopo

Cada `let` declara um binding imutável e **único** no escopo atual. Re-declarar o mesmo nome é erro:

```kata
action main
    let x := 42
    let x := 99
    echo!(x)
main!()
```

```
Error: type.duplicate_decl

  × tipo `x` já declarado
```

Para reusar um nome, use `var` — descrito na próxima seção.

## `var` — bindings mutáveis

`var` cria um binding mutável e pode ser redeclarado no mesmo escopo, substituindo o binding anterior:

```kata
action main
    var x := 42
    var x := + x 1
    echo!(x)
main!()
```

```
43
```

`var` é o mecanismo correto quando você precisa reusar um nome ou atualizar um valor dentro de um escopo. `let` é imutável e único; `var` é mutável e substituível.

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
echo!(* 99999999999999999999 99999999999999999999)
```

```
9999999999999999999800000000000000000001
```

## O operador `::`

`::` é um operador multifunção em Kata. Seu papel principal é **ascription de tipo** — anexar um tipo a uma expressão:

```kata
action main
    let x := 42 :: Int
    echo!(x)
main!()
```

```
42
```

A ascription é útil quando você quer ser explícito sobre o tipo de uma expressão. O compilador verifica que o tipo é compatível com o valor.

### Conversão de literal com `::`

`::` também converte literais entre tipos relacionados. O caso mais comum é `Rational`:

```kata
echo!(3.14::Rational)
```

```
3.14
```

O texto bruto do literal é preservado — não há passagem por `f64`, então não há perda de precisão na conversão. (Para precisão exata com `Rational`, veja a próxima seção.)

### Os outros papéis de `::`

`::` aparece em quatro outros contextos em Kata. Cada um será explorado em seu capítulo:

- **Assinatura de função** (cap 5): `dobrar :: Int => Int` etiqueta o nome com seu tipo
- **Tipagem de campos e parâmetros** (caps 7, 10): `data Pessoa (nome::Text)`, `action jogar (alvo::Int)`
- **Qualificação de variante** (cap 10): `Cor::Amarelo`, `Result::Ok` — acessa uma variante de enum pelo nome do tipo
- **Tipos refinados** (cap 12): `5 :: PositiveInt` valida predicados em compile-time; `a :: Int` faz downcast de refinado para base

Embora o token seja o mesmo, ascription (`valor :: Tipo`) e qualificação (`Tipo::Variante`) operam em direções opostas: a primeira vai do valor ao tipo, a segunda do tipo à variante.

## Rational — precisão exata

Floats têm imprecisão inerente. `Rational` é exato — `1/3 * 3 = 1`, não `0.999...`:

```kata
echo!(* 3.14::Rational 100)
```

```
314
```

Com `Float`, o mesmo cálculo poderia dar `314.00000000000006`. `Rational` preserva o valor exato.

## Próximo capítulo

Agora que você sabe dar nomes a valores, o próximo capítulo mostra como definir funções com assinaturas, múltiplas cláusulas, e recursão.