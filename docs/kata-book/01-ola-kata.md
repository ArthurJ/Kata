# Capítulo 1 — Olá, Kata

Kata é uma linguagem funcional com notação prefixa, tipos algébricos, e concorrência cooperativa via canais. Este capítulo mostra como instalar, executar seu primeiro programa, e explorar o REPL.

## Instalação

O binário `kata` é distribuído como executável nativo. Após instalar, verifique a versão:

```bash
kata --version
```

```
kata 0.1.0
```

## Seu primeiro programa

Kata usa notação prefixa: a função vem antes dos argumentos. Para somar dois números:

```kata
+ 1 2
```

Salve em um arquivo `hello.kata` e execute:

```bash
kata run hello.kata
```

```
3
```

Sem `action main`, sem boilerplate. A última expressão do arquivo é o entry point do programa.

## Imprimindo na tela

Para mostrar valores na tela, use `echo!`:

```kata
echo!("olá, mundo")
```

```
olá, mundo
```

O `!` no final indica que `echo!` é uma *action* — uma função impura que interage com o mundo exterior. Funções puras não usam `!`.

## REPL interativo

Para experimentar expressões sem criar arquivos, use o REPL:

```bash
kata repl
```

```
kata> + 1 2
3
kata> echo!("olá")
olá
kata> :quit
```

O REPL mantém bindings entre linhas e suporta `:type` para inspecionar tipos, `:env` para ver o ambiente, e `:load arquivo.kata` para carregar um módulo.

## Próximos passos

Você escreveu seu primeiro programa Kata. O próximo capítulo constrói um jogo de adivinhação completo — um mini-projeto que introduz actions, pattern matching, e leitura de entrada antes da teoria.