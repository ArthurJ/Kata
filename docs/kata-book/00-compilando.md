# Capítulo 0 — Compilando o Kata

Antes de escrever qualquer programa, você precisa do binário `kata`. Este capítulo mostra como compilar o compilador a partir do código-fonte e quais comandos estão disponíveis.

## Pré-requisitos

- **Rust 1.85 ou superior** — o compilador Kata é escrito em Rust e usa a edition 2024. Instale via [rustup](https://rustup.rs) ou o gerenciador de pacotes do seu sistema.
- **Linker C (`cc`)** — necessário para gerar executáveis nativos. No Linux, instale `gcc` ou `clang`. No macOS, o Xcode Command Line Tools fornece `clang`.

Verifique que o Rust está disponível:

```bash
rustc --version
```

```
rustc 1.85.0
```

## Compilando

Clone o repositório e compile em modo release:

```bash
git clone https://github.com/arthurjulia/kata.git
cd kata
cargo build --release
```

O binário fica em `target/release/kata`. Para evitar digitar o path completo, adicione ao seu `PATH`:

```bash
export PATH="$PWD/target/release:$PATH"
```

Verifique:

```bash
kata --version
```

```
kata 0.1.0
```

## Comandos disponíveis

O binário `kata` tem vários subcomandos. Os mais usados:

### `kata run` — executar um arquivo

Compila e executa um arquivo `.kata` imediatamente:

```bash
kata run examples/fatorial.kata
```

```
120
```

### `kata eval` — avaliar uma expressão

Avalia uma expressão direto da linha de comando, sem criar arquivo:

```bash
kata eval '+ 1 2'
```

```
3
```

Útil para testar rapidamente uma expressão.

### `kata repl` — REPL interativo

Inicia o REPL para experimentar expressões interativamente. Detalhado no [Capítulo 15](15-repl.md).

### `kata build` — compilar para executável nativo

Gera um executável standalone a partir de um arquivo `.kata`:

```bash
kata build examples/fatorial.kata
./fatorial
```

Por padrão, o runtime é linkado estaticamente. Use `--dynamic` para linkar dinamicamente (binário menor, mas depende da lib em runtime):

```bash
kata build examples/fatorial.kata --dynamic
```

### `kata test` — executar testes

Descobre e executa testes em um arquivo ou diretório. Há dois tipos:

- **`@test`** — testes anotados com a diretiva `@test` em actions
- **Doctests** — exemplos executáveis em comentários multilinha `#{ }#` com marcador `>>> `

```bash
kata test examples/assertions.kata
```

Use `--filter` para rodar apenas testes `@test` que contenham uma substring:

```bash
kata test examples/ --filter "fib"
```

Doctests sempre rodam (não são afetados por `--filter`). Veja o [Capítulo 16](16-doctests.md) para detalhes sobre doctests.

### `kata lex` e `kata parse` — inspeção do compilador

Mostram os tokens e a AST de um arquivo, respectivamente. Úteis para entender como o compilador vê seu código:

```bash
kata lex examples/hello.kata
kata parse examples/hello.kata
```

### `kata lsp` — servidor de linguagem

Inicia o servidor LSP (Language Server Protocol) em stdio. Editores como VS Code e Neovim podem conectar para obter autocomplete, diagnósticos, e hover de tipos. O apêndice sobre plataformas ([Apêndice](17-plataformas-limitacoes.md)) tem mais sobre onde o LSP funciona.

## Onde encontrar exemplos

O diretório `examples/` no repositório tem dezenas de programas `.kata` que exercitam cada feature da linguagem:

```bash
ls examples/
```

Alguns notáveis:

| Arquivo | O que mostra |
|---------|-------------|
| `fatorial.kata` | Recursão com tail-call optimization |
| `fizzbuzz.kata` | Pattern matching + guards |
| `quicksort.kata` | HOFs + List |
| `select_queue.kata` | CSP: channels + select + timeout |
| `refined_types.kata` | Tipos refinados + smart constructors |

Explore à vontade — todo exemplo é executável com `kata run`.

## Syntax highlighting

O repositório inclui um bundle TextMate na raiz para realce de sintaxe em editores:

```
Kata.tmbundle/
├── info.plist                  — metadados do bundle
└── Syntaxes/
    └── Kata.tmLanguage.json    — gramática TextMate
```

### VS Code

O VS Code usa o arquivo `.tmLanguage.json` diretamente. Para instalar manualmente, copie `Kata.tmLanguage.json` para a pasta de extensões do VS Code, ou use uma extensão que carregue gramáticas TextMate customizadas.

### JetBrains (IntelliJ, CLion, RustRover, etc.)

IDEs da JetBrains leem o bundle `.tmbundle` inteiro. Importe via *Settings → Editor → TextMate Bundles → +* e selecione a pasta `Kata.tmbundle/` na raiz do projeto. O editor passa a reconhecer arquivos `.kata` com a gramática do bundle.

### LSP

Além do highlighter, o binário `kata` inclui um servidor LSP (`kata lsp`) que fornece diagnósticos, hover de tipos, e autocomplete para editores que suportam Language Server Protocol. Veja o [Capítulo 15](15-repl.md) para mais detalhes sobre a integração com editores.

## Próximo capítulo

Com o binário compilado e os comandos em mãos, o [Capítulo 1](01-ola-kata.md) mostra seu primeiro programa em Kata.