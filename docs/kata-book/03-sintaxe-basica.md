# Capítulo 3 — Sintaxe Básica

Kata usa notação prefixa: a função vem antes dos argumentos, sem parênteses. Isto elimina ambiguidade de precedência e trata operadores e funções da mesma forma.

## Notação prefixa

```kata
+ 1 2       # soma
* 3 4       # multiplicação
- 10 3      # subtração
```

```
3
12
7
```

Operadores (`+`, `-`, `*`) são nomes de função como qualquer outro. `+ 1 2` e `soma 1 2` têm a mesma estrutura. Não há tabela de precedência para memorizar.

## Comentários

`#` inicia um comentário que vai até o final da linha:

```kata
# isto é um comentário
42 # comentário ao lado do código
```

Comentários multilinha usam `#{ }#`:

```kata
#{
  Comentário multilinha.
  Pode span várias linhas.
}#
```

## Literais

| Sintaxe | Tipo | Exemplo |
|---------|------|---------|
| `42` | Int | `42` |
| `0xFF` | Int (hex) | `255` |
| `1_000` | Int | `1000` |
| `3.14` | Float | `3.14` |
| `"hello"` | Text | `"hello"` |
| `True` | Boolean | `True` |
| `False` | Boolean | `False` |
| `()` | Unit | `()` |

Inteiros têm precisão arbitrária (BigInt). Não há overflow.

## `echo!` e `show`

`echo!` imprime texto na tela. Todo tipo em Kata implementa a interface `SHOW`, então `echo!` aceita qualquer valor diretamente:

```kata
echo!(42)
echo!(3.14)
echo!("hello")
echo!(True)
echo!(())
```

```
42
3.14
hello
True
()
```

`show` é a função que converte qualquer valor em `Text`. `echo!` já chama `show` internamente, então você raramente precisa chamar `show` explicitamente — apenas quando quiser o texto sem imprimir. `show` funciona com todos os tipos — coleções, tuplas, enums, structs. Veremos mais no capítulo 8.

## Converter Text em número

As actions `int!` e `float!` convertem `Text` para `Int` e `Float` respectivamente. Como são actions (podem falhar se o texto não é um número válido), retornam `Result`. O operador `|` desempacota o `Ok` e fornece um fallback se falhar (visto no capítulo 2):

```kata
echo!(int!("42") | 0)
echo!(int!("0xFF") | 0)
echo!(float!("3.14") | 0.0)
echo!(+ (float!("1.5") | 0.0) (float!("2.5") | 0.0))
```

```
42
255
3.14
4.0
```

`int!` suporta decimal, hexadecimal (`0x`), octal (`0o`), binário (`0b`) e underscores (`1_000`). `float!` suporta notação decimal e exponencial (`1e10`).

## Concatenar texto

O operador `+` também concatena texto:

```kata
echo!(+ "hello " "world")
```

```
hello world
```

## Próximo capítulo

Agora que você sabe escrever expressões, o próximo capítulo mostra como dar nomes a valores com bindings e explorar os tipos primitivos da linguagem.