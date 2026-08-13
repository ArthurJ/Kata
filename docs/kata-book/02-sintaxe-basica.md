# Capítulo 2 — Sintaxe Básica

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

`echo!` imprime texto na tela. `show` converte qualquer valor em texto. Combine os dois para imprimir qualquer tipo:

```kata
echo!(show 42)
echo!(show 3.14)
echo!(show "hello")
echo!(show True)
echo!(show ())
```

```
42
3.14
hello
True
()
```

`show` funciona com todos os tipos — coleções, tuplas, enums, structs. Veremos mais no capítulo 7.

## Concatenar texto

O operador `+` também concatena texto:

```kata
echo!(show + "hello " "world")
```

```
hello world
```

## Próximo capítulo

Agora que você sabe escrever expressões, o próximo capítulo mostra como dar nomes a valores com bindings e explorar os tipos primitivos da linguagem.