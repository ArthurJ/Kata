# PRD — stdio como valores: `__stdin__`, `__stdout__`, `__stderr__`

**Status:** ✅ Concluído
**Data:** 2026-08-23
**Depende de:** File I/O ✅ (`Ty::File`, `open!`, `write!`, `close!`), `is_stdio` ✅ (flag em `FileInner`)
**Substitui:** `PRD-stdio-alignment.md` §2.1 (stdout/stderr como `action`)

## 1. Objetivo

Transformar `stdin`, `stdout`, `stderr` de actions (`stdin!()`, `stdout!()`, `stderr!()`)
em valores (`__stdin__`, `__stdout__`, `__stderr__`) disponíveis no escopo global.

**Motivação:** FD 0/1/2 existem quando o processo nasce. O programa não "faz" stdout
— ele *tem* stdout. Modelar acesso a um recurso preexistente como behavior (`action
stdout () => File`) é semanticamente dishonesto: `stdout!()` não modifica estado, não
suspende o fiber, é idempotente (mesmo handle cached). A distinção entre "ter" e
"usar" fica mais clara: `__stdout__` é o valor; `echo!(msg, __stdout__)` é o comportamento.

**Princípio:** `__` é reservado pelo parser (`validate_name` rejeita `__` no início
de qualquer identificador do usuário). Identificadores dunder (`__name__`) são
necessariamente built-in — nunca confundíveis com nomes definidos pelo usuário.
Mesmo padrão de `__self`, `__result`, `__kata_show__*`.

## 2. Sintaxe

### 2.1. Declaração no prelude

Os três valores são injetados no `TypeEnv` do prelude como bindings globais com tipo
`Ty::File`. Não há declaração `action` nem `@ffi` no `stdlib/stdio.kata` — a injeção
é estrutural, feita no resolution.

```kata
# stdio.kata — não declara mais nada.
# __stdin__, __stdout__, __stderr__ são injetados pelo resolution,
# não por declarações neste arquivo.
import core
```

### 2.2. Uso

```kata
import stdio

echo!("mensagem", __stdout__)       # escreve em stdout
echo!("erro", __stderr__)           # escreve em stderr
write!(__stdout__, "bytes\n")       # write direto

match read!(__stdin__)
    Result::Ok bytes: echo!(show bytes)
    Result::Err reason: echo!("erro: " + reason)
```

Sem `!`, sem `()`. O identificador é um valor de tipo `File`, como `True` é um valor
de tipo `Boolean`.

### 2.3. Diretiva `@log`

```kata
@log{msg: "{_return}\n", when: "exit", file: __stdout__}
```

O campo `file` da diretiva `@log` recebe uma expressão `Expr::Ident` que resolve para
`Ty::File`. Hoje `file: stdout!()` requer action call; `file: __stdout__` é um
identificador puro — mais simples no typeck e no codegen.

## 3. Semântica

### 3.1. Valores, não computações

`__stdin__`, `__stdout__`, `__stderr__` são bindings imutáveis no `TypeEnv` global. O
typeck os resolve no caminho 1 de `Expr::Ident` (`env.lookup(name)`) — o mesmo
caminho de variáveis locais e parâmetros. Não passam pelo DispatchTable, não são
actions, não são funções.

### 3.2. Materialização no codegen

O codegen lowera `TypedExprKind::Ident { name: "__stdout__" }` (quando `expr.ty ==
Ty::File`) para uma chamada FFI `kata_rt_stdout()` que retorna o handle cached.
O mesmo para `__stdin__` e `__stderr__`.

Isso é lazy materialization: o handle só é criado na primeira referência ao valor.
Se o programa nunca referencia `__stderr__`, o handle nunca é criado.

### 3.3. Propriedades herdadas do `is_stdio` flag

As FFIs `kata_rt_stdin`, `kata_rt_stdout`, `kata_rt_stderr` já existem e já
retornam handles com `is_stdio: true`:
- `close!()` é no-op (não fecha FD 0/1/2)
- `read!(stdout)` retorna `Err("not readable")` (stdout/stderr são write-only)
- `write!(stdin, ...)` retorna `Err("not writable")` (stdin é read-only)
- Múltiplas referências retornam o mesmo handle (TLS cache)

Nenhuma mudança no runtime — as FFIs e o `FileInner` já estão corretos.

### 3.4. Import

`import stdio` disponibiliza `__stdin__`, `__stdout__`, `__stderr__` no escopo. Sem o
import, os identificadores não resolvem (`UnboundName`). O módulo `stdio` existe
apenas como namespace de importação — não contém declarações.

## 4. O que muda onde

### 4.1. Resolution (Pass 0+1)

Hoje `stdlib/stdio.kata` declara três `action` com `@ffi`. Após a mudança:

- `stdlib/stdio.kata` não declara mais os três actions. O arquivo fica vazio (só
  `import core`), ou é removido inteiramente se não houver outras declarações.
- O resolution injeta três bindings no `TypeEnv` do módulo `stdio`:
  - `__stdin__` → `Ty::File`, origin `"stdio"`
  - `__stdout__` → `Ty::File`, origin `"stdio"`
  - `__stderr__` → `Ty::File`, origin `"stdio"`
- A injeção é feita no carregamento do módulo `stdio`, não no prelude global —
  só fica disponível se o usuário fizer `import stdio`.
- As três FFIs (`kata_rt_stdin`, `kata_rt_stdout`, `kata_rt_stderr`) são
  **mantidas** no runtime e no codegen — o codegen as chama ao lowerar os
  identificadores. O que sai é a declaração `action` no prelude; a FFI continua
  existindo como mecanismo de materialização.

### 4.2. Inference (Pass 2)

Hoje `Expr::Ident { name: "stdout" }` sem `!` cai no caminho 3 (DispatchTable) e
retorna `Ty::Action` (first-class reference). Após a mudança:

- `Expr::Ident { name: "__stdout__" }` cai no caminho 1 (`env.lookup`) e retorna
  `Ty::File` diretamente. Não passa pelo DispatchTable.
- `TypedExprKind::Ident { name: "__stdout__" }` com `ty: Ty::File` é o que chega
  ao codegen.

### 4.3. Codegen

Hoje `stdout!()` é uma action call: o codegen lowera `ActionCall { callee: "stdout",
args: [] }` chamando a FFI `kata_rt_stdout`. Após a mudança:

- `TypedExprKind::Ident { name: "__stdout__" }` com `ty: Ty::File` precisa de um
  novo caso no lowering de `Ident` em `expr.rs`:
  - Se `name` é um dos três identificadores especiais e `expr.ty == Ty::File`,
    emitir `call kata_rt_stdout()` (ou stdin/stderr) e retornar o `i64` handle.
  - O resto do lowering de `Ident` (var_map, function pointer, action ref) não
    muda.
- O `__stdout__` em posição de argumento (`echo!(msg, __stdout__)`) lowera o
  identificador primeiro (produz `i64` handle) e passa como arg para `echo!`.
  Mesmo fluxo de `echo!(msg, f)` onde `f` é uma variável local `File`.

### 4.4. Runtime

**Nenhuma mudança.** As FFIs `kata_rt_stdin`, `kata_rt_stdout`, `kata_rt_stderr`
já existem, já retornam handles cached com `is_stdio: true`, já são registradas
em todos os 6+ pontos de toque. O `is_stdio` flag já previne `close!` de fechar
FD 0/1/2.

### 4.5. `PRD-stdio-alignment.md`

O PRD-stdio-alignment §2.1 descreve `stdout!()`/`stderr!()` como actions. Esta
mudança substitui essa seção: os três viram valores `__stdin__`/`__stdout__`/`__stderr__`.
As demais seções do PRD-stdio-alignment (log! bifurcação, sobrecarga de @log,
log_recv → Result, remoção de tópicos mágicos) continuam válidas — `log!` com
`File` recebe `__stdout__` em vez de `stdout!()`, e `@log{file: __stdout__}` em vez
de `@log{file: stdout}`.

## 5. Fases de implementação

### Fase 1: Resolution — injetar bindings

- Em `kata-resolution`, ao carregar o módulo `stdio`, injetar três bindings no
  `TypeEnv` exportado pelo módulo:
  - `__stdin__` → `Ty::File`, origin `"stdio"`
  - `__stdout__` → `Ty::File`, origin `"stdio"`
  - `__stderr__` → `Ty::File`, origin `"stdio"`
- Remover as três declarações `action stdin/stdout/stderr` de `stdlib/stdio.kata`.
- Manter `import core` no arquivo (ou remover o arquivo se vazio — decidir na
  implementação se `import stdio` ainda faz sentido como namespace vazio).
- As FFIs `kata_rt_stdin/stdout/stderr` permanecem registradas no `FfiSymbol`
  enum, `ffi_registry.rs`, `ffi_sigs/file_io.rs` — o codegen as usa ao lowerar
  os identificadores.

**Arquivos:**
- `stdlib/stdio.kata` — remover declarações `action`
- `crates/kata-resolution/src/` — injeção de bindings ao carregar módulo `stdio`

**Verificação:** `cargo check --workspace --all-targets`

### Fase 2: Codegen — lowerar identificadores especiais

- Em `lower_expr` (`kata-codegen/src/lowering/expr.rs`), no braço de
  `TypedExprKind::Ident { name }`:
  - Antes do caminho 1 (var_map), adicionar verificação:
    se `name == "__stdin__" && expr.ty == Ty::File` → emitir
    `call kata_rt_stdin()`.
    Se `name == "__stdout__"` → `call kata_rt_stdout()`.
    Se `name == "__stderr__"` → `call kata_rt_stderr()`.
  - Retornar o `i64` handle como valor.
- O runtime FFI já está registrado (`builder.symbol` em `ffi_registry.rs`),
  a signature já está em `ffi_sigs/file_io.rs`.

**Arquivos:**
- `crates/kata-codegen/src/lowering/expr.rs` — novo caso em `Ident`

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 3: Migrar usos existentes

- Buscar `stdout!()`, `stdin!()`, `stderr!()` em exemplos, testes E2E, e
  snapshots.
- Substituir por `__stdout__`, `__stdin__`, `__stderr__`.
- Atualizar `@log{file: stdout!()}` para `@log{file: __stdout__}`.
- Atualizar snapshots (`cargo insta accept`).

**Arquivos:**
- `examples/quicksort.kata` — `file:stdout!()` → `file:__stdout__`
- `crates/kata-driver/tests/` — testes E2E que usam `stdout!()`
- `crates/kata-codegen/tests/` — testes E2E que usam `stdout!()`
- Snapshots afetados

**Verificação:** `cargo test --workspace --no-fail-fast`, 0 failed

### Fase 4: Limpeza

- Remover `FfiSymbol::Stdin`, `Stdout`, `Stderr` do `OverloadInfo` no
  DispatchTable (se ainda registrados como overloads de action). As FFIs
  continuam no runtime e no codegen — só saem do DispatchTable.
- Verificar se `kata_rt_text_to_int`/`kata_rt_text_to_float` (versões antigas
  que mascaram erro) podem ser removidas do runtime (não relacionadas aos
  stdio values, mas é cleanup adjacente — commit separado).

**Verificação:** `cargo test --workspace --no-fail-fast` + `cargo clippy -- -D warnings`

## 6. Fora do escopo

- **Effect system / capability system** — `__stdout__` como valor não introduz
  um sistema de capabilities. É um binding global, não um tipo novo. A
  discussão sobre "resource" vs "action" como categoria semântica fica para
  o futuro — esta mudança é puramente sobre modelagem de stdio.
- **stdin como read-only / stdout-stderr como write-only no type system** —
  a restrição é runtime (`is_stdio` flag + `IoMode`), não no tipo. `__stdin__`
  e `__stdout__` têm o mesmo tipo (`Ty::File`). A distinção read/write é
  enforcement do runtime, não do type system.
- **Outros "false actions"** — `int(Text)` e `float(Text)` já foram migrados
  para funções puras. Não há outros casos identificados no prelude.

## 7. DoDs (Definitions of Done)

1. `__stdin__`, `__stdout__`, `__stderr__` são valores de tipo `Ty::File` disponíveis
   ao fazer `import stdio`.
2. `echo!(msg, __stdout__)` escreve em stdout via File. `echo!(msg, __stderr__)`
   escreve em stderr via File.
3. `write!(__stdout__, "text")` funciona (write direto no FD 1).
4. `read!(__stdin__)` funciona (read do FD 0).
5. `close!(__stdout__)` é no-op — não fecha FD 1.
6. Múltiplas referências a `__stdout__` no mesmo programa retornam o mesmo handle
   (TLS cache — não cria múltiplos `FileInner`).
7. `stdout!()`, `stdin!()`, `stderr!()` como action calls não existem mais
   (não há overload no DispatchTable).
8. `@log{file: __stdout__}` funciona — o campo `file` recebe `Expr::Ident` que
   resolve para `Ty::File`.
9. `import stdio` sem usar nenhum dos três valores não materializa nenhum handle
   (lazy — só cria na primeira referência).
10. `cargo test --workspace --no-fail-fast` passa sem regressão.
11. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

## 8. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | Dunder `__name__` em vez de keyword nova (`resource`, `capability`) | `__` no início é reservado estruturalmente pelo parser. Não precisa de keyword nova, annotation, ou categoria de tipo. Zero mudança no type system. |
| D2 | Valores no `TypeEnv`, não no DispatchTable | FD 0/1/2 são fatos do mundo, não computações. `env.lookup` é o caminho natural — mesmo que `True`, `False`, variantes unitárias. |
| D3 | Materialização via FFI no codegen (lazy) | O handle é criado na primeira referência ao valor. Se o programa nunca referencia `__stderr__`, o `FileInner` nunca é alocado. Reusa as FFIs existentes. |
| D4 | Módulo `stdio` como namespace de importação | `import stdio` disponibiliza os três valores. Sem o import, não estão no escopo. Mesmo padrão de outros módulos. |
| D5 | `Ty::File` para os três — sem tipo novo | stdin/stdout/stderr são `File`. A restrição read/write é runtime (`IoMode`), não tipo. Introduzir `Ty::ReadOnlyFile` / `Ty::WriteOnlyFile` seria complexidade sem benefício de verificação. |
| D6 | Remover `action` do prelude, manter FFI no runtime | A declaração `action stdout () => File` sai. A FFI `kata_rt_stdout()` fica — é o mecanismo de materialização que o codegen chama. |

## 9. Riscos

| Risco | Mitigação |
|---|---|
| `__stdout__` em posição não-argumento (ex: `let f := __stdout__`) precisa funcionar | O lowering de `Ident` produz o `i64` handle em qualquer posição. `let f := __stdout__` lowera o identificador e armazena o handle na variável. |
| Codegen precisa distinguir `__stdout__` (valor especial) de `__stdout__` como nome de função | Verificar `expr.ty == Ty::File` no lowering de `Ident`. Se `ty` é `Ty::File` e `name` é um dos três, emitir FFI call. Senão, cair nos caminhos existentes. |
| `import stdio` vazio (sem declarações) pode confundir o module loader | O module loader injeta os bindings estruturalmente ao carregar `stdio`. O arquivo `stdio.kata` pode ficar vazio ou ser removido — a injeção é no resolution, não no arquivo. |
| Remover `action stdout` do DispatchTable quebra código que faz `let f := stdout` (first-class action ref) | Buscar usos de `stdout` sem `!` no codebase. Se existir, migrar para `__stdout__`. Provável que não exista — `stdout` sem `!` não fazia sentido antes (retornava `Ty::Action`, não `Ty::File`). |
| Snapshots mudam porque `stdout!()` → `__stdout__` altera o output | Migrar snapshots com `cargo insta accept`. |

### Fase 5: Atualização da documentação

Migrar toda a documentação que referencia `stdin!()/stdout!()/stderr!()` como
actions para o novo modelo de valores `__stdin__`/`__stdout__`/`__stderr__`.

**5a. `docs/Kata-lang-manual.md`**
- §22.5 "Descritores Padrão (stdio como File)" — reescrever: actions → valores,
  `stdin!()` → `__stdin__`, `stdout!()` → `__stdout__`, `stderr!()` → `__stderr__`.
  Atualizar tabela de operações e exemplos de código.
- Linha ~2172: `@log{file: stdout}` → `@log{file: __stdout__}`; remover "identificador
  de action 0-ary" da descrição do campo `file`.
- Linha ~2924: tabela de `@log` — atualizar descrição do campo `file`.
- Linhas ~2994-3014: exemplos `log!(..., stdout!())` → `log!(..., __stdout__)`,
  `echo!(msg, stdout!())` → `echo!(msg, __stdout__)`.
- Linha ~3205: título e corpo da seção — "stdin/stdout/stderr como valores".

**5b. `docs/kata-book/`**
- `07-actions.md` — seção sobre stdio: `stdout!()` → `__stdout__` (ou remover se
  stdio não for mais apresentado como action).
- `11-actions-avancadas.md` — seção sobre `@log` com `file: stdout` → `file: __stdout__`.
- `16-doctests.md` — referência a stderr (linha ~140).
- `17-plataformas-limitacoes.md` — linha 30: "I/O de arquivo, stdin/stdout/stderr"
  (atualizar se descreve como action).
- `02-guessing-game.md` —linha 49: "O `!` em `input!`" — esclarecer que `input!`
  continua sendo action (não confundir com `__stdin__` que é valor).
- `00-compilando.md` e `18-rationale.md` — verificar se mencionam stdio como action.

**5c. `docs/sintaxe-mapa.md`**
- Linha ~741: tabela de `@log` — campo `file`: atualizar descrição de "Identificador
  de action 0-ary" para "expressão que resolve para `Ty::File` (ex: `__stdout__`)".
- Linhas ~825, 835: exemplos `echo!(msg, stdout!())` → `echo!(msg, __stdout__)`.
- Linhas ~858-893: seção "Módulo `stdio`" — reescrever: `stdin!()/stdout!()/stderr()`
  → `__stdin__`/`__stdout__`/`__stderr__`. Tabela de actions → tabela de valores. Exemplos
  de `import stdio.(stdout)` → `import stdio` (valores são globais do módulo).

**5d. `docs/mapa-funcionalidades.md`**
- Não há entrada para stdio hoje — criar nova entrada documentando `__stdin__`,
  `__stdout__`, `__stderr__` como valores `Ty::File` disponíveis via `import stdio`.
- Incluir propriedades: lazy materialization, `is_stdio` flag, `close!` no-op,
  read/write enforcement por `IoMode` (runtime, não tipo).

**5e. PRDs relacionados**
- `docs/PRD-stdio-values.md` — este arquivo (status → concluído).
- `docs/PRD-stdio-alignment.md` — atualizar §2.1: `stdout!()`/`stderr!()` →
  `__stdout__`/`__stderr__`. Demais seções (log! bifurcação, @log sobrecarga,
  log_recv → Result) continuam válidas — substituir apenas a modelagem de stdio.

**Verificação:** revisão manual de cada arquivo; buscar `stdout!()`/`stdin!()`/
`stderr!()` residual: `grep -rn 'std\(out\|in\|err\)!(' docs/` deve retornar vazio.