# Capítulo 15 — REPL Interativo

O REPL (Read-Eval-Print Loop) é a forma mais rápida de experimentar Kata. Sem criar arquivos, você avalia expressões, inspeciona tipos, e carrega módulos.

## Iniciando

```bash
kata repl
```

```
Kata REPL — digite :help para comandos, :quit para sair
```

## Avaliando expressões

Digite qualquer expressão Kata. O resultado aparece imediatamente:

```
>>> + 1 2
3
>>> * 3 4
12
>>> echo!("olá")
olá
```

## Bindings persistentes

Bindings feitos com `let` persistem entre linhas:

```
>>> let x := 42
>>> + x 8
50
```

O binding `x` fica disponível nas linhas seguintes até você sair ou resetar.

## `:type` — inspecionar tipos

Sem executar, veja o tipo de uma expressão:

```
>>> :type + 1 2
Int
```

Útil para entender o que o typechecker infere sem rodar o código.

## `:env` — ver o ambiente

Lista todos os bindings e seus tipos:

```
>>> let x := 42
>>> :env
  x: Int
```

## `:load` — carregar um arquivo

Carrega um arquivo `.kata` no ambiente do REPL:

```
>>> :load fatorial.kata
carregado: fatorial.kata
>>> fat 5 1
120
```

As funções e constantes do arquivo ficam disponíveis para uso interativo.

## `:reset` — limpar o ambiente

Remove todos os bindings e recarrega o prelude:

```
>>> let x := 42
>>> :reset
sessão resetada — prelude recarregado
>>> :env
(nenhum binding)
```

## `:help` — comandos disponíveis

```
>>> :help
comandos:
  :help          mostra esta mensagem
  :type <expr>   mostra o tipo de <expr> sem executar
  :env           mostra bindings e tipos no TypeEnv atual
  :load <file>   carrega arquivo .kata (items entram no env)
  :reset         limpa bindings, recarrega prelude
  :quit          sai do REPL
```

## Próximo capítulo

O REPL é ótimo para experimentar, mas testes reproduzíveis são essenciais. O próximo capítulo mostra como escrever doctests — exemplos executáveis embutidos em comentários que rodam com `kata test`:

→ [Capítulo 16 — Doctests](16-doctests.md)

Para aprofundar:
- `examples/` — exemplos completos de cada feature
- `docs/Kata-lang-manual.md` — manual técnico de referência
- `docs/sintaxe-mapa.md` — mapa completo de sintaxe