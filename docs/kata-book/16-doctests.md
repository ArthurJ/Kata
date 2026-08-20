# Capítulo 16 — Doctests

Doctests são exemplos executáveis embutidos em comentários multilinha. Eles permitem documentar código com exemplos que são validados automaticamente por `kata test` — se a documentação mentir, o teste falha.

## Sintaxe

Dentro de um comentário multilinha `#{ }#`, linhas que começam com `>>> ` são inputs do REPL. O conteúdo após `>>> ` é avaliado como uma expressão Kata. As linhas seguintes (sem `>>> `) são o output esperado.

```
#{
Calcula o dobro de um número.

>>> constant n := 5
>>> + n n
10
>>> constant m := 0
>>> + m m
0
}#
```

## Como funciona

Cada bloco de doctest cria uma sessão REPL fresca. Linhas `>>> ` consecutivas compartilham bindings — o que você define numa linha fica disponível na próxima.

```
#{
>>> constant x := 10
>>> + x 1
11
>>> * x 2
20
}#
```

Uma linha vazia separa blocos. Cada bloco começa sessão nova — bindings não persistem entre blocos.

```
#{
>>> constant x := 10
>>> + x 1
11

>>> constant y := 20
>>> + y 1
21
}#
```

No segundo bloco, `x` não existe.

## Texto livre antes dos doctests

Linhas antes da primeira `>>> ` dentro do `#{ }#` são documentação livre — ignoradas pelo runner de doctests. Isto permite escrever prosa explicativa antes dos exemplos.

```
#{
A função `soma` recebe dois Int e retorna a soma.

Exemplo:
>>> + 3 4
7
}#
```

## Input multiline

Se o input após `>>> ` é incompleto (ex: `match` sem cláusulas), as linhas indentadas seguintes são continuação do input, não output esperado.

```
#{
>>> match True
  True: "sim"
  False: "nao"
sim
}#
```

## Sem output esperado

Linhas `>>> ` sem linhas de output seguintes significam "não produz output". Isto cobre declarações (`constant`, `let`, `Sig`, `data`, `enum`) que não têm expressão de entrada.

```
#{
>>> constant x := 42
>>> x
42
}#
```

A primeira linha (`constant x := 42`) não tem output esperado. A segunda (`x`) produz `42`.

## Executando

Doctests rodam automaticamente com `kata test`, antes dos testes `@test`:

```bash
kata test examples/exemplo.kata
```

```
  [PASS] exemplo.kata: doctest linha 2
  [PASS] exemplo.kata: doctest linha 3
  2 passed, 0 failed, 0 skipped
```

Se o output não bater:

```
  [FAIL] exemplo.kata: doctest linha 3: output mismatch
    esperado: 99
    obtido:   10
  1 passed, 1 failed, 0 skipped
```

`--filter` não afeta doctests — eles sempre rodam.

## Comentários sem doctests

Comentários `#{ }#` sem nenhuma linha `>>> ` são ignorados completamente. Não há impacto em `kata test` — comportamento é idêntico a antes.

## Próximo capítulo

Você completou todos os capítulos do guia principal. Há ainda um apêndice sobre plataformas suportadas e limitações — incluindo o estado do port para Windows:

→ [Apêndice — Plataformas e Limitações](17-plataformas-limitacoes.md)