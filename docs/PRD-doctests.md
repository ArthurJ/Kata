# PRD — Doctests em Comentários Multilinha

**Status:** Implementado
**Data:** 2026-08-20
**Depende de:** REPL (`ReplSession`), `kata test`
**Não depende de:** AOT, LSP, `kata-rt` (zero mudanças no runtime)

## 1. Objetivo

Permitir testes executáveis embutidos em comentários multilinha `#{ }#`,
validados por `kata test`. Linhas `>>> ` dentro do comentário são
interpretadas como inputs do REPL — compilados e executados via
`ReplSession` existente. O output de cada avaliação é comparado com as
linhas seguintes (não-`>>> `) do comentário.

### Princípio: reusar o REPL

O `ReplSession` já acumula bindings entre linhas, resolve multiline,
chama `display::print_result` por expressão, e faz rollback em erro.
O doctest runner não reimplementa nada disso — dirige a sessão
existente.

### Princípio: pré-passo textual

O lexer descarta comentários `#{ }#` completamente (nenhum token, nenhum
span). Doctests não precisam de AST — precisam de texto e execução.
O scanner é um pré-passo textual antes do pipeline normal de `cmd_test`.

## 2. Sintaxe

### 2.1. Marcador `>>>`

Linhas que começam com `>>> ` (literal, case-sensitive) dentro de um
comentário multilinha `#{ }#` são inputs do REPL. O conteúdo após
`>>> ` é o input exato passado para `ReplSession::handle`.

```
#{
>>> 1 + 1
2
>>> constant x := 42
>>> x * 2
84
}#
```

### 2.2. Output esperado

Após uma linha `>>> ` cujo input está completo, todas as linhas
seguintes que NÃO começam com `>>> ` são o output esperado da
avaliação. A próxima linha `>>> ` inicia um novo input.

Linhas `>>> ` sem output esperado significam "não produz output".
Isto cobre declarações (`constant`, `let`, `Sig`, `Data`, `Enum`) que
não têm expressão de entrada.

### 2.3. Input multiline

Se o input após `>>> ` é incompleto (parse falha com `<EOF>`), as
linhas seguintes sem `>>> ` são continuação do input, não output.
A detecção usa `ReplSession::is_input_incomplete` (mesma heurística
do REPL interativo).

```
#{
>>> match True
  True => "sim"
  False => "nao"
sim
}#
```

Neste exemplo, `match True` é incompleto, então as linhas indentadas
são continuação do input. A linha `sim` é o output esperado.

### 2.4. Blocos e sessões

A partir da primeira linha `>>> ` em um `#{ }#`, tudo até o final do
comentário é doctest. Linhas `>>> ` consecutivas (sem linha vazia
entre elas) compartilham a mesma sessão REPL — bindings persistem.

Uma linha vazia que NÃO é explicada pelo input (ou seja, não é uma
linha vazia que termina um bloco multiline) separa blocos. Cada bloco
começa uma `ReplSession` fresca.

```
#{
>>> constant x := 10
>>> x + 1
11

>>> constant y := 20
>>> y + 1
21
}#
```

Aqui há dois blocos. O segundo bloco não vê `x` — começa sessão nova.

### 2.5. Texto livre antes de doctests

Linhas antes da primeira `>>> ` em um `#{ }#` são texto livre
(documentação) e são ignoradas pelo scanner de doctests.

```
#{
Calcula fatorial recursivamente.
Veja: https://exemplo.com/fatorial

>>> fatorial 5
120
}#
```

### 2.6. Comentários sem doctests

Comentários `#{ }#` sem nenhuma linha `>>> ` são ignorados
completamente — comportamento atual, zero impacto.

## 3. Supressão de Unit no REPL

### 3.1. Mudança

Atualmente, o REPL imprime `()` para expressões que retornam `Unit`
(incluindo `let` bindings e declarações). Python doctests não exigem
output para atribuições. Para que doctests sejam naturais, `()` deve
ser suprimido no REPL.

A mudança é no `eval_expr` de `ReplSession`: suprimir
`display::print_result` quando `result.ty` é `Ty::Unit`. Isto afeta
tanto o REPL interativo quanto os doctests — o usuário não vê mais `()`
para `let x := 42` ou `echo!("hello")`.

`cmd_run` já faz isto:
```rust
if !matches!(result.ty, Ty::Unit) {
    display::print_result(result.raw, &result.ty);
}
```

O REPL passa a fazer o mesmo.

### 3.2. Impacto

- `let x := 42` → sem output (antes: `()`)
- `echo!("hello")` → imprime `hello` via `kata_rt_println`, sem `()` (antes: `hello\n()`)
- Expressão que retorna Unit explicitamente → sem output
- Tipos não-Unit continuam imprimindo normalmente

## 4. Captura de Output

### 4.1. Sem mudanças em kata-rt

A captura de output só acontece em `kata test` (doctests). O runtime
(`kata-rt`) não é modificado — `kata_rt_print_result`, `kata_rt_print`,
`kata_rt_println` continuam fazendo `println!`/`print!` direto em
stdout, como hoje.

### 4.2. Redirecionamento de stdout no test runner

O doctest runner redireciona o stdout do processo antes de cada
avaliação e restaura depois. Mecanismo Unix:

```rust
use std::os::fd::AsRawFd;

fn capture_stdout<F: FnOnce() -> R, R>(f: F) -> String {
    // Cria pipe, salva stdout original, redireciona para pipe_write.
    // Executa f. Lê do pipe_read. Restaura stdout original.
    // Retorna conteúdo lido.
}
```

Isto é código de teste no `kata-driver` — não toca `kata-rt`, não
adiciona branches em hot paths, não afeta `kata repl`, `kata run`, ou
`kata build`.

### 4.3. Limitação aceitável

Output de actions que rodam em threads do scheduler pode chegar ao
stdout original (não redirecionado) se a thread escrever antes do
eval retornar. Como doctests são casos simples (expressões puras,
declarações), isto não é problema na prática. Se surgir, captura
via pipe global pode ser revisitada — mas não no escopo deste PRD.

## 5. Arquitetura

### 5.1. Scanner de doctests

Novo módulo `kata-driver/src/doctest.rs`:

```rust
/// Um bloco de doctest — sessão REPL isolada.
struct DocBlock {
    /// Casos: (input, expected_output)
    cases: Vec<(String, Option<String>)>,
    /// Linha inicial no source (para diagnósticos)
    line: usize,
}

/// Escaneia source por comentários `#{ }#` contendo `>>> `.
/// Retorna lista de blocos de doctest.
fn scan_doctests(source: &str) -> Vec<DocBlock>
```

O scanner:
1. Itera sobre o source caractere a caractere
2. Detecta `#{` → início de comentário multilinha
3. Acumula conteúdo até `}#` → fim do comentário
4. No conteúdo, procura linhas `>>> `
5. Se nenhuma `>>> ` → ignora (comentário normal)
6. Se há `>>> ` → processa como doctest

### 5.2. Parser de blocos

Dentro de um comentário com `>>> `:

1. Linhas antes da primeira `>>> ` → texto livre, ignoradas
2. A partir da primeira `>>> ` → doctest
3. Acumula casos:
   - `>>> <input>` → inicia novo caso com input
   - Se `is_input_incomplete(input)`, linhas seguintes sem `>>> ` são continuação do input
   - Se input está completo, linhas seguintes sem `>>> ` são output esperado
   - Próxima `>>> ` → fecha output esperado, inicia novo caso
   - Linha vazia (não explicada pelo input) → fim do bloco atual, inicia novo bloco
4. `}#` → fim do comentário, fecha último bloco

### 5.3. Runner

Integrado em `cmd_test`, **antes** dos wrappers `@test`:

```rust
// Doctests primeiro
let doctests = doctest::scan_doctests(&source);
for block in &doctests {
    let mut session = ReplSession::new().map_err(...)?;
    for (input, expected) in &block.cases {
        let actual = capture_stdout(|| session.handle(input));
        match result {
            Ok(true) => {
                if let Some(expected) = expected {
                    if normalize(&actual) == normalize(expected) {
                        // PASS
                    } else {
                        // FAIL: output mismatch
                    }
                } else {
                    // Sem output esperado — pass se actual está vazia
                }
            }
            Ok(false) => { /* :quit — não acontece em doctest */ }
            Err(e) => { // FAIL: erro de execução }
        }
    }
}

// Depois wrappers @test (comportamento atual)
for w in &wrappers { ... }
```

### 5.4. Normalização

- Trim de whitespace à direita em cada linha
- Múltiplas linhas de output são concatenadas com `\n`
- Comparação: `normalize(actual) == normalize(expected)`

## 6. Integração com `kata test`

### 6.1. Ordem de execução

`cmd_test` executa, para cada arquivo `.kata`:
1. Doctests (novo)
2. Wrappers `@test` (comportamento atual)

Ambos compartilham os mesmos contadores `total_pass`, `total_fail`,
`total_skip`.

### 6.2. Labels

```
[PASS] arquivo.kata: doctest linha 3
[FAIL] arquivo.kata: doctest linha 7: output mismatch
  esperado: 2
  obtido:   3
```

### 6.3. Filtro

`--filter` já existe para `@test` (substring na descrição). Doctests
não têm descrição. Opções:
- `--filter` ignora doctests (doctests sempre rodam)
- `--filter doctest` roda apenas doctests, `--filter @test` apenas wrappers
- Doctests usam a linha como identificador: `--filter "linha 3"`

Por ora, doctests sempre rodam — não são afetados por `--filter`.

## 7. Escopo do PRD

### 7.1. Incluso

- Scanner textual de `#{ }#` com `>>> `
- Reutilização de `ReplSession` para execução
- Captura de output via redirecionamento de stdout (sem mudanças em kata-rt)
- Supressão de `Unit` no REPL
- Normalização (trim trailing)
- Integração com `kata test` (doctests antes de `@test`)
- Testes E2E em `kata-driver/tests/`

### 7.2. Excluído (futuro)

- Anexar doctests a declarações específicas (precisaria preservar
  spans de comentários no lexer)
- Doctests em comentários de linha `#` (apenas multilinha `#{ }#`)
- Captura de stderr (panic/erros de runtime)
- Modo "atualizar doctests" (reescrever expected com output real)
- LSP integration (hover com doctests, run doctest on save)

## 8. Testes

### 8.1. Testes unitários (scanner)

Em `kata-driver/tests/doctest_scan.rs`:
- Comentário sem `>>> ` → zero blocos
- Comentário com texto livre + `>>> ` → um bloco, texto livre ignorado
- Múltiplos `>>> ` consecutivos → um bloco, múltiplos casos
- Linha vazia entre `>>> ` → dois blocos
- Input multiline (`match True` + cláusulas) → um caso, input multiline
- `>>> ` com output esperado de múltiplas linhas
- `>>> ` sem output esperado (declaração)

### 8.2. Testes E2E (runner)

Em `kata-driver/tests/doctest_e2e.rs`:
- Doctest simples: `>>> 1 + 1` / `2` → PASS
- Doctest com binding: `>>> constant x := 42` / `>>> x` / `42` → PASS
- Doctest com output mismatch → FAIL com mensagem esperada
- Doctest multiline: `>>> match True` + cláusulas → PASS
- Doctest com `echo!`: `>>> echo!("hello")` / `hello` → PASS
- Múltiplos blocos separados por linha vazia → segundo não vê bindings do primeiro
- `@test` e doctest no mesmo arquivo → ambos rodam
- Supressão de `()`: `>>> let x := 42` sem output esperado → PASS
- Comentário `#{ }#` sem `>>> ` → ignorado, sem doctests

### 8.3. Snapshot

Output de `kata test` com doctests deve ser estável para snapshots.