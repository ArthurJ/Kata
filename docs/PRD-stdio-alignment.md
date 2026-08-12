# PRD — Alinhamento de stdio: `stdout`/`stderr` como `File`, `log!` com File, sobrecarga de `@log`

**Status:** 📋 Proposto
**Data:** 2026-08-04
**Depende de:** Fio 14 ✅ (`@log`, `log!`, `log_recv!`), File I/O ✅ (`Ty::File`, `open!`, `write!`, `close!`), Socket I/O ✅ (`Ty::Socket`)
**Não depende de:** `spawn!` (não implementado)
**Habilita:** PRD-trace-directive.md (diretiva `trace`/`log` em Kata — exige `log!()` com `File` e stdin/stdout/stderr como `File` em módulo `stdio`)

## 1. Objetivo

Alinhar stdin/stdout/stderr com o sistema de File I/O existente e com o sistema de
log, eliminando os "tópicos mágicos" (`"stdout"`, `"stderr"`) do runtime de log.
Três mudanças coordenadas:

1. **`stdin`, `stdout` e `stderr` como `File`** — FFIs novas no runtime que
   retornam handles `File` apontando para FDs 0, 1 e 2. Disponíveis em um
   **módulo `stdio` separado** (`stdlib/stdio.kata`), não no prelude. O usuário
   importa explicitamente: `import stdio.(stdin, stdout, stderr)`. Se o core
   precisar dos handles, pode importar o módulo `stdio`. Componíveis com
   `echo!(msg, stdout)`, `write!(stdout, msg)`, `read!(stdin)`, etc.

2. **`log!()` aceita `File` como 3º argumento** — bifurcação por tipo no
   `infer_log_builtin`: `Text` → canal CSP (como hoje), `File` → write direto
   no arquivo via `kata_rt_file_write_text`. Sem canal, sem policy, sem
   `log_recv!`.

3. **Sobrecarga de `@log`** — múltiplas diretivas `@log` na mesma action/função,
   cada uma com seu destino. `@log{msg: ..., topic: "metrics"}` publica em CSP.
   `@log{msg: ..., file: stdout}` escreve direto no arquivo. Alinha `@log` com
   `@test` (que já suporta múltiplas ocorrências) e prepara o terreno para
   diretivas Kata (decorators) com sobrecarga.

4. **`log_recv!` retorna `Result::(Text, Text)`** — `Ok(msg)` se recebeu
   mensagem, `Err(reason)` se tópico inexistente ou canal fechou. Mais
   ergonômico que retornar `0` silenciosamente.

## 2. Sintaxe

### 2.1. `stdin`, `stdout` e `stderr` no módulo `stdio`

```
# stdlib/stdio.kata
import core

@ffi("kata_rt_stdin")
action stdin () => File

@ffi("kata_rt_stdout")
action stdout () => File

@ffi("kata_rt_stderr")
action stderr () => File
```

`stdin!()`, `stdout!()` e `stderr!()` são actions FFI que retornam um handle
`File` apontando para FD 0, 1 e 2 respectivamente. O handle é um `FileInner`
alocado na root_arena com `is_stdio: true` — o `close!` é no-op para estes
handles.

O módulo `stdio` é um módulo Kata normal em `stdlib/stdio.kata`, **não** parte
do prelude. O usuário importa explicitamente:

```
import stdio.(stdout, stderr)       # seletivo
import stdio                        # acesso via stdio.stdout!()
```

Se o `core` precisar de `stdout`/`stderr`/`stdin` (ex: `echo` com File,
sinergias futuras), pode importar o módulo `stdio`:
`import stdio.(stdout)`.

Uso:

```
echo!("mensagem", stdout!())      # escreve em stdout via File
echo!("erro", stderr!())          # escreve em stderr via File
write!(stdout!(), "bytes\n")     # write direto
```

### 2.2. `log!()` com `File`

```
log!(LogLevel::Info, "mensagem", stdout!())     # write direto em stdout
log!(LogLevel::Info, "mensagem", stderr!())     # write direto em stderr
log!(LogLevel::Info, "mensagem", f)              # write direto em arquivo aberto
log!(LogLevel::Info, "mensagem", "metrics")      # CSP (como hoje)
log!(LogLevel::Info, "mensagem", "metrics", "block")  # CSP com policy
```

Bifurcação por tipo do 3º argumento:

| 3º arg | Tipo | Comportamento |
|---|---|---|
| `Text` | `Ty::Text` | Canal CSP (Broadcast/Queue). Aceita 4º arg `policy`. `log_recv!` funciona. |
| `File` | `Ty::File` | Write direto via `kata_rt_file_write_text`. Rejeita 4º arg `policy` (erro de tipo). `log_recv!` não aplicável. |

`topic` (Text) e `file` (File) podem coexistir na mesma chamada — o log
publica no canal CSP E escreve no arquivo. Nesse caso, `log!()` recebe
5 args: `(level, msg, topic, file, policy?)`. A bifurcação por tipo se
aplica ao argumento que seria `topic` ou `file`: se é `Text`, é topic;
se é `File`, é file. Quando ambos estão presentes, a ordem é
`(level, msg, topic, file)` ou `(level, msg, file, topic)` — decidir na
implementação.

A mensagem é tratada como template em ambos os caminhos (CSP e File).
`{expr}` interpola expressões do escopo, `{{` escapa `{` literal. O parsing
reusa `parse_template`/`parse_placeholder` de `log_synthesis.rs` e desugara
para `format!()` via `infer_format`.

A variável `{log_level}` está disponível como placeholder em `@log` e `log!()`,
resolvendo para a string do level (`"Info"`, `"Warn"`, etc.):

```
log!(LogLevel::Warn, "[{log_level}] aviso: {codigo}")
@log{msg: "[{log_level}] entrada x={x}", when: "enter", file: stdout}
```

**Mudança de semântica:** `log!()` agora trata msg como template, não como
`Text` puro. Isso quebra retrocompatibilidade — strings com `{` literal
precisam escapar com `{{`. Justificativa: unifica `log!()` e `@log` no mesmo
mecanismo de template, e permite `{log_level}` sem formatação manual.

### 2.3. Sobrecarga de `@log`

Múltiplas diretivas `@log` na mesma action/função:

```
@log{msg: "entrada x={x}", when: "enter", topic: "metrics"}
@log{msg: "entrada x={x}", when: "enter", file: stdout}
action processar (x::Int) => Int
    + x 1
```

Cada diretiva é processada independentemente. O codegen injeta uma chamada
por diretiva no prólogo/epílogo.

Campos da diretiva `@log` (atualizado):

| Campo | Tipo | Default | Descrição |
|---|---|---|---|
| `msg` | `Text` | **obrigatório** | Template compile-time. `{expr}` interpola. `{{` escapa `{`. |
| `level` | `LogLevel` | `LogLevel::Info` | Variante do enum `LogLevel`. |
| `when` | `Text` | **obrigatório** | `"enter"` = prólogo. `"exit"` = epílogo. |
| `topic` | `Text` | herdado ou `"log"` | Nome do canal CSP. Pode coexistir com `file`. |
| `policy` | `Text` | herdado ou `"drop"` | `"drop"` ou `"block"`. Só com `topic`. |
| `file` | `Expr` (Ident) | — | Expressão que avalia para `File`. Pode coexistir com `topic`. |

Validação: `policy` sem `topic` (com `file` mas sem `topic`) é erro. `topic`
e `file` juntos são válidos — o log publica no canal CSP E escreve no arquivo.

### 2.4. `log_recv!` retorna `Result`

```
match log_recv!("audit")
    Result::Ok msg: echo!(msg)
    Result::Err reason: echo!("log error: " + reason)
```

`log_recv!(topic)` retorna `Result::(Text, Text)`:
- `Ok(msg)` — mensagem recebida do tópico.
- `Err("topic not found")` — tópico não existe no registry.
- `Err("channel closed")` — canal fechou.

## 3. Semântica

### 3.1. `stdin`/`stdout`/`stderr` como `File`

`FileInner` ganha campo `is_stdio: bool`. Se `true`:
- `kata_rt_file_close` é no-op (não fecha FD 0/1/2).
- `kata_rt_file_write_text` funciona normalmente para stdout/stderr — escreve
  no FD subjacente.
- `kata_rt_file_read`/`readline`/`read_chunk` funcionam normalmente para stdin
  — lê do FD 0. Para stdout/stderr, retornam `Err("not readable")`
  (stdout/stderr são write-only por convenção).
- `kata_rt_file_write_text` em stdin retorna `Err("not writable")`
  (stdin é read-only por convenção).

O handle é criado uma única vez (lazy static no runtime) e cached. Múltiplas
chamadas a `stdout!()` retornam o mesmo handle. O mesmo para `stdin!()` e
`stderr!()`.

Plataforma: `File::from_raw_fd(0)` / `File::from_raw_fd(1)` /
`File::from_raw_fd(2)` é Unix-only. O runtime já assume Unix
(`std::os::unix::io::AsRawFd` em `file.rs`). Se multiplataforma for necessária
no futuro, adicionar `#[cfg(unix)]` + fallback.

### 3.2. Bug fix: leak de FDs — escape analysis + registry de FileInner

**Problema:** `FileInner` é alocado na root_arena (compartilhada entre fibers)
via `kata_rt_get_root_arena_handle()` hardcoded em `file.rs:88-93`. Quando uma
fiber é destruída (`try_destroy`), apenas a arena do fiber é destruída — a
root_arena não é tocada. `Bump::reset()` (arena_destroy) não chama Drop em
objetos alocados. Portanto, `FileInner` abertos sem `close!()` explícito
**nunca têm o FD fechado** — leak de FD até o fim do processo.

Entre testes, `reset_scheduler` chama `reset_all_arenas` que faz `Bump::reset`
em todas as arenas (incluindo root_arena), descartando a memória sem Drop.
FDs vazados ficam abertos até o processo terminar.

**Causa raiz:** `file_open` ignora o sistema de escape analysis que já existe
no compilador. Canais (channel, queue, broadcast) são alocados na `fiber_arena`
do criador; FileInner deveria seguir o mesmo padrão.

#### Nível 1 (neste PRD): escape analysis + registry global

**Mudança na FFI:** `kata_rt_file_open` recebe `arena_handle` como parâmetro
adicional (como `kata_rt_list_cons`, `kata_rt_array_alloc` já fazem). O codegen
injeta a arena via `arena_handle_for_escape(expr.escape, ctx)`:

- `escape == Local` → `fiber_arena` (arquivo local à action/fiber)
- `escape == Caller` → `caller_arena` (arquivo retornado pela action)
- `escape == Heap` → `root_arena` (arquivo enviado via canal entre fibers)

`kata_rt_file_open(path_ptr, mode_box, arena_handle) -> result_box`

O `arena_alloc` em `file.rs` passa a usar o `arena_handle` recebido em vez de
`kata_rt_get_root_arena_handle()`.

**Registry global:** `OPEN_FILES: RefCell<Vec<i64>>` TLS em `file.rs`:

- `kata_rt_file_open`: após `alloc_file_inner`, registra handle em `OPEN_FILES`.
- `kata_rt_file_close`: remove handle de `OPEN_FILES`, faz `drop_in_place`
  (exceto `is_stdio` — no-op).
- `reset_file_registry` (nova): chamada por `reset_scheduler` entre testes.
  Itera `OPEN_FILES`, chama `kata_rt_file_close` em cada handle, limpa Vec.
- `stdin!()`/`stdout!()`/`stderr!()`: handles `is_stdio` **não** são registrados (não
  precisam de cleanup).

O registry global é o safety net: independente de qual arena o `FileInner`
está, FDs não fechados explicitamente são fechados no fim do programa/teste.

#### Nível 2 (última fase deste PRD): registry por-fiber com close na destruição

**Motivação:** No Nível 1, FDs de arquivos locais (`escape == Local`) só são
fechados no fim do programa. Em servidores longos com fibers efêmeras abrindo
muitos arquivos, isso pode exaurir FDs antes do cleanup.

**Mudança:** `FiberEntry` ganha `open_files: Vec<i64>`. TLS
`CURRENT_FIBER_ID` rastreia a fiber em execução.

- `kata_rt_file_open`: se `arena_handle` é `fiber_arena` (local), registra
  handle em `FiberEntry.open_files` da fiber atual via `CURRENT_FIBER_ID`.
  Se `caller_arena` ou `root_arena`, registra no registry global.
- `try_destroy`: antes de `kata_rt_arena_destroy(arena_handle)`, itera
  `entry.open_files` e chama `kata_rt_file_close` em cada handle. FDs locais
  são fechados antes da memória ser descartada — sem leak.
- Arquivos transferidos (`escape == Caller`/`Heap`) permanecem no registry
  global — o destino (caller/receiver) é responsável.
- `CURRENT_FIBER_ID` TLS é setado pelo scheduler em `resume_fiber` (já que
  `self.current_fiber = Some(fiber_id)` na linha 423) e limpo em `try_destroy`.

**Acesso ao Scheduler:** `file_open` lê `CURRENT_FIBER_ID` (TLS i64) e
`SCHEDULER` (TLS `RefCell<Option<Scheduler>>`). Como `file_open` é chamado
dentro de `resume_fiber` (o scheduler tem `borrow_mut` ativo), usar
`borrow_mut()` causaria panic. Solução: `CURRENT_FIBER_ID` aponta para o
`FiberId`, e `file_open` usa `SCHEDULER.with(|s| unsafe { (*s.as_ptr()).as_mut() })`
para acessar o `Scheduler` sem borrow check (mesmo padrão usado por
`reset_scheduler_tls` no `fork()` child — ver `scheduler/ffi.rs:139`).

**Limitação conhecida (ambos os níveis):** se a fiber A abre um arquivo, envia
o handle para B via canal, e A chama `close!()` antes de B terminar — B
recebe um handle dangling (use-after-close). Exigiria ownership tracking ou
reference counting de FileInner. Não resolvido neste PRD.

### 3.3. `log!()` bifurcação e template

O `infer_log_builtin` hoje recebe `msg` como `Text` puro. A mudança:

1. Parsear `msg` como template (reusa `parse_template`/`parse_placeholder`
   de `log_synthesis.rs`, extraídos para módulo compartilhado).
2. Extrair placeholders `{expr}` e `{log_level}`.
3. `{log_level}` resolve para `TextLit` com a string do level
   (`"Info"`, `"Warn"`, etc.), derivada da tag i64 do 1º argumento.
4. Construir tupla de args (placeholders + `log_level`) e chamar
   `infer_format` para produzir a cadeia de `text_replace_first`.
5. O resultado é um `Text` formatado em runtime.

Bifurcação por tipo do 3º argumento:

1. Inferir o tipo do 3º argumento.
2. Se `Ty::text()` → caminho CSP. Valida 4º arg como `Text` se presente.
   Lowera para `kata_rt_log_publish(topic, level, formatted_msg, policy)`.
3. Se `Ty::File` → caminho File. Rejeita 4º arg (erro de tipo se presente).
   Lowera para `kata_rt_file_write_text(file_handle, formatted_msg)`.
4. Se ausente (2 args) → caminho CSP com config herdada.
5. Outro tipo → erro de tipo.

O codegen não precisa de mudanças estruturais — o `infer_log_builtin` produz
diferentes `TypedExprKind::Closure` com diferentes `ffi_symbol` dependendo do
caminho. Para File, o `ffi_symbol` é `"kata_rt_file_write_text"` e os args
são `[file_handle, msg_ptr]`.

**Quebra de compatibilidade:** `log!()` agora trata msg como template. Strings
com `{` literal precisam escapar com `{{`. Antes, `log!(LogLevel::Info,
"msg")` tratava `"msg"` como Text puro. Agora, `"msg"` é template (sem
placeholders, resultado idêntico). Apenas strings com `{` não escapado
quebram.

### 3.4. Sobrecarga de `@log`

Alinhado com `@test` (que já itera sobre múltiplas diretivas):

- `extract_log_specs` (plural) itera sobre todas as diretivas `@log` com
  `for d in directives { if d.name == "log" { ... } }`.
- Retorna `Vec<LogSpec>` em vez de `Option<LogSpec>`.
- `ActionDef.log` / `FunctionDef.log` muda de `Option<LogSpec>` para
  `Vec<LogSpec>`.
- `TypedAction.log` / `TypedFunction.log` muda de `Option<TypedLogSpec>` para
  `Vec<TypedLogSpec>`.
- `inject_log` no codegen itera sobre a lista e injeta uma chamada por spec.

### 3.5. `log_recv!` → `Result`

O runtime `kata_rt_log_recv` hoje retorna `i64` (handle Text ou 0). Muda para
retornar um Result box (16 bytes na arena via `kata_rt_store_sum_result`):
- Tag 0 (Ok) + payload (handle Text) — mensagem recebida.
- Tag 1 (Err) + payload (handle Text) — reason string.

O typeck muda o tipo de retorno de `Ty::text()` para
`Ty::Result(Box::new(Ty::text()), Box::new(Ty::text()))`.

### 3.6. Remoção dos tópicos mágicos

O bloco em `kata_rt_log_publish` (log.rs:185-193) que intercepta
`topic == "stdout" || topic == "stderr"` é removido. stdin/stdout/stderr passam a
ser acessados via `File` (módulo `stdio`), não via strings mágicas.

## 4. Fases de implementação

### Fase 1: Runtime — `stdin`/`stdout`/`stderr` como `File`

- Adicionar `is_stdio: bool` a `FileInner` em `file.rs`.
- Implementar `kata_rt_stdin() -> i64`, `kata_rt_stdout() -> i64` e
  `kata_rt_stderr() -> i64`:
  - `stdin`: `FileInner` com `BufReader::new(File::from_raw_fd(0))`,
    `is_stdio: true`, `mode: Read`. Cacheia em TLS lazy static.
  - `stdout`: `FileInner` com `BufWriter::new(File::from_raw_fd(1))`,
    `is_stdio: true`, `mode: Write`. Cacheia em TLS lazy static.
  - `stderr`: `FileInner` com `BufWriter::new(File::from_raw_fd(2))`,
    `is_stdio: true`, `mode: Write`. Cacheia em TLS lazy static.
- `kata_rt_file_close`: se `is_stdio`, no-op (não fecha FD).
- `kata_rt_file_write_text`: se `is_stdio` com `mode: Read` (stdin), retorna
  `Err("not writable")`.
- `kata_rt_file_read`/`readline`/`read_chunk`: se `is_stdio` com `mode: Write`
  (stdout/stderr), retornam `Err("not readable")`.
- Registrar FFIs em todos os sites (pitfall #31):
  - `FfiSymbol::Stdin`, `FfiSymbol::Stdout`, `FfiSymbol::Stderr` em
    `kata-core/ffi.rs`
  - `symbol_name()`, `return_type()`, `from_name()`, `ffi_signature()` em
    `ffi_sigs.rs`
  - `all_ffi_symbols()`, `declare_ffi_symbols`, `register_ffi_symbols` em
    `ffi_registry.rs`
  - `builder.symbol()` em `lib.rs`

**Arquivos:**
- `crates/kata-rt/src/file.rs` — `is_stdio`, FFIs novas, close guard, guards de read/write por mode
- `crates/kata-core/src/ffi.rs` — `FfiSymbol::Stdin`, `Stdout`, `Stderr`
- `crates/kata-codegen/src/ffi_sigs.rs` — assinatura `(caller_arena) -> File`
- `crates/kata-codegen/src/ffi_registry.rs` — registro
- `crates/kata-rt/src/lib.rs` — re-export, `builder.symbol`

**Verificação:** `cargo check --workspace --all-targets`

### Fase 1b: Bug fix Nível 1 — escape analysis em file_open + registry global

- **FFI signature:** `kata_rt_file_open` ganha parâmetro `arena_handle`:
  `kata_rt_file_open(path_ptr, mode_box, arena_handle) -> result_box`.
  Atualizar `FfiSymbol::FileOpen` em `ffi_sigs.rs` (adicionar `AbiParam::new(I64)`).
- **Runtime:** `arena_alloc` em `file.rs` usa o `arena_handle` recebido em
  vez de `kata_rt_get_root_arena_handle()`. `alloc_file_inner` recebe
  `arena_handle` como parâmetro.
- **Codegen:** adicionar `FfiSymbol::FileOpen` à lista de FFIs que recebem
  arena injetada. O `closure.rs` já injeta `fiber_arena` via
  `ffi_needs_arena` — adicionar `FileOpen` à lista em `ffi_sigs.rs`.
  Alternativa: interceptar `kata_rt_file_open` no `closure.rs` (como
  Dict/Set) e injetar `arena_handle_for_escape(expr.escape, ctx)`.
- Adicionar `OPEN_FILES: RefCell<Vec<i64>>` TLS em `file.rs`.
- `kata_rt_file_open`: após `alloc_file_inner`, registrar handle em `OPEN_FILES`.
- `kata_rt_file_close`: remover handle de `OPEN_FILES` antes de `drop_in_place`.
- `kata_rt_stdin`/`kata_rt_stdout`/`kata_rt_stderr`: **não** registrar em `OPEN_FILES` (is_stdio).
- `reset_file_registry` (nova função pub(crate)): iterar `OPEN_FILES`, chamar
  `kata_rt_file_close` em cada handle, limpar Vec.
- `reset_scheduler` em `scheduler/ffi.rs`: adicionar chamada
  `crate::file::reset_file_registry()` antes de `reset_all_arenas`.

**Arquivos:**
- `crates/kata-rt/src/file.rs` — `arena_alloc` recebe handle, `OPEN_FILES` TLS, registro em open/close, `reset_file_registry`
- `crates/kata-rt/src/scheduler/ffi.rs` — chamada `reset_file_registry` em `reset_scheduler`
- `crates/kata-codegen/src/ffi_sigs.rs` — `FfiSymbol::FileOpen` ganha param `arena_handle`, adicionar a `ffi_needs_arena`
- `crates/kata-codegen/src/lowering/closure.rs` — injetar arena via `arena_handle_for_escape` para `kata_rt_file_open`

**Verificação:** `cargo test --workspace --no-fail-fast` — verificar que testes
de file I/O não vazam FDs entre execuções.

### Fase 2: Módulo `stdio` — `stdin`/`stdout`/`stderr` como actions FFI

- Criar `stdlib/stdio.kata` como módulo Kata normal (não prelude):
  ```
  import core

  @ffi("kata_rt_stdin")
  action stdin () => File

  @ffi("kata_rt_stdout")
  action stdout () => File

  @ffi("kata_rt_stderr")
  action stderr () => File
  ```
- O módulo `stdio` importa `core` para ter acesso a `File`, `Result`, etc.
- O usuário importa `stdio` explicitamente: `import stdio.(stdout, stderr)`.
- Se o `core` precisar de `stdout`/`stderr`/`stdin` (ex: `echo` com File),
  pode importar `stdio`: `import stdio.(stdout)`.
- Verificar que o DispatchTable registra os overloads e que `stdout!()`
  retorna `File`.

**Arquivos:**
- `stdlib/stdio.kata` — novo módulo

**Verificação:** `cargo test -p kata-resolution -- prelude` (o prelude
não deve conter stdout/stderr/stdin) e compilar um programa que faça
`import stdio.(stdout)` seguido de `echo!(msg, stdout!())`.

### Fase 3: Remoção dos tópicos mágicos

- Remover o bloco `if topic == "stdout" || topic == "stderr"` de
  `kata_rt_log_publish` em `log.rs`.
- Atualizar testes E2E que usam `topic: "stdout"` para usar `file: stdout`
  (após Fase 5).

**Arquivos:**
- `crates/kata-rt/src/log.rs` — remover linhas 185-193

**Verificação:** `cargo check --workspace --all-targets`

### Fase 4: `log!()` bifurcação por tipo + template

- Extrair `parse_template`/`parse_placeholder` de `log_synthesis.rs` para
  módulo compartilhado (ex: `log_template.rs`).
- Em `infer_log_builtin` (`log_builtins.rs`):
  - Parsear `msg` (2º arg) como template: extrair `{expr}` e `{log_level}`.
  - `{log_level}` resolve para `TextLit` com a string do level, derivada da
    tag i64 do 1º arg (ex: tag 1 → `"Info"`, tag 2 → `"Warn"`).
  - Outros placeholders `{expr}` são inferidos contra o escopo (como `@log`
    faz hoje).
  - Construir tupla de args e chamar `infer_format` → cadeia de
    `text_replace_first`. O resultado é `Text` formatado.
  - Inferir tipo do 3º arg.
  - Se `Ty::text()` → caminho atual (CSP). Valida 4º arg como `Text` se
    presente. Lowera para `kata_rt_log_publish`.
  - Se `Ty::File` → caminho File. Rejeita 4º arg (erro de tipo se presente).
    Lowera para `kata_rt_file_write_text(file_handle, formatted_msg)`.
  - Se ausente (2 args) → caminho CSP com config herdada (como hoje).
- Adicionar `{log_level}` ao `@log` em `log_synthesis.rs`: injetar como
  variável sintética no escopo antes do parsing do template.

**Arquivos:**
- `crates/kata-inference/src/infer/log_template.rs` — `parse_template`/`parse_placeholder` extraídos (novo)
- `crates/kata-inference/src/infer/log_builtins.rs` — bifurcação + template
- `crates/kata-inference/src/infer/log_synthesis.rs` — `{log_level}` sintético, reusa `log_template`

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 5: Sobrecarga de `@log` — resolution + types

- `LogSpec` ganha campo `file: Option<Spanned<Expr>>`.
- `extract_log_spec` → `extract_log_specs` (plural):
  - `find` → `filter` (itera sobre todas as diretivas `@log`).
  - Retorna `Vec<LogSpec>`.
  - Nova chave `file:` aceita `Expr::Ident` (ex: `stdout`).
  - Validação: `topic` e `file` mutuamente exclusivos. `policy` sem `topic`
    (com `file`) é erro.
- `ActionDef.log` / `FunctionDef.log` → `Vec<LogSpec>`.
- `lib.rs` (resolution): `extract_log_spec` → `extract_log_specs`.

**Arquivos:**
- `crates/kata-resolution/src/types.rs` — `LogSpec.file`, `ActionDef.log` → `Vec`
- `crates/kata-resolution/src/directives.rs` — `extract_log_specs`
- `crates/kata-resolution/src/lib.rs` — chamada

**Verificação:** `cargo check --workspace --all-targets`

### Fase 6: Sobrecarga de `@log` — inference + codegen

- `TypedLogSpec` ganha campo `file: Option<Spanned<TypedExpr>>`.
- `synthesize_log_spec` → `synthesize_log_specs` (plural): retorna
  `Vec<TypedLogSpec>`.
  - Se `file` presente, tipa a expressão como `Ty::File`. Não sintetiza
    `format` para `topic`/`policy`.
- `TypedAction.log` / `TypedFunction.log` → `Vec<TypedLogSpec>`.
- `inject_log` no codegen: itera sobre a lista. Para cada spec:
  - Se `topic` presente → `kata_rt_log_publish` (CSP, como hoje).
  - Se `file` presente → `kata_rt_file_write_text(file_handle, msg_ptr)`.
    Lowera a expressão `file` (produz `i64` handle) e a expressão `msg`.
  - Injeta no prólogo (`when: "enter"`) ou epílogo (`when: "exit"`).
- `action_def.rs` e `function_def.rs`: iterar sobre `log: Vec<TypedLogSpec>`
  em vez de `log: Option<TypedLogSpec>`.

**Arquivos:**
- `crates/kata-inference/src/typed_module.rs` — `TypedLogSpec.file`, `TypedAction.log` → `Vec`
- `crates/kata-inference/src/infer/log_synthesis.rs` — `synthesize_log_specs`
- `crates/kata-inference/src/infer/action_infer.rs` — chamada
- `crates/kata-inference/src/infer/mod.rs` — chamada
- `crates/kata-codegen/src/lowering/log.rs` — iterar + bifurcar
- `crates/kata-codegen/src/lowering/action_def.rs` — iterar
- `crates/kata-codegen/src/lowering/function_def.rs` — iterar

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 7: `log_recv!` → `Result`

- Runtime `kata_rt_log_recv`: retorna Result box em vez de `i64` cru.
  - `alloc_result_box(0, msg_handle)` para Ok.
  - `alloc_result_box(1, error_text("topic not found"))` para Err.
  - `alloc_result_box(1, error_text("channel closed"))` para canal fechado.
- Typeck `infer_log_recv_builtin`: tipo de retorno muda de `Ty::text()` para
  `Ty::Result(Box::new(Ty::text()), Box::new(Ty::text()))`.
- FFI signature em `ffi_sigs.rs`: retorno continua `i64` (Result box é i64 na
  ABI). Nenhuma mudança no codegen — o caller já lida com Result boxes.
- Atualizar testes E2E: `let msg := log_recv!("topic")` →
  `match log_recv!("topic") Result::Ok msg: ... Result::Err _: ...`.

**Arquivos:**
- `crates/kata-rt/src/log.rs` — `kata_rt_log_recv` retorna Result box
- `crates/kata-inference/src/infer/log_builtins.rs` — tipo de retorno
- `crates/kata-driver/tests/log_e2e.rs` — atualizar testes

**Verificação:** `cargo test --workspace --no-fail-fast`

### Fase 8: Testes E2E ✅

Testes novos em `crates/kata-driver/tests/stdio_log_e2e.rs` (17 testes):

- `stdio_file_stdin.kata` — `read!(stdin!())` lê de stdin via File (import stdio).
- `stdio_file_stdout.kata` — `echo!(msg, stdout!())` escreve em stdout (import stdio).
- `stdio_file_stderr.kata` — `echo!(msg, stderr!())` escreve em stderr (import stdio).
- `stdio_file_close_noop.kata` — `close!(stdout!())` é no-op, não crash.
- `stdio_stdin_write_erro.kata` — `write!(stdin!(), "msg")` retorna `Err("not writable")`.
- `stdio_stdout_read_erro.kata` — `read!(stdout!())` retorna `Err("not readable")`.
- `log_to_file.kata` — `log!(LogLevel::Info, "msg {x}", stdout!())` escreve em stdout.
- `log_to_file_arquivo.kata` — `log!(LogLevel::Info, "msg {x}", f)` escreve em arquivo.
- `log_template_level.kata` — `log!(LogLevel::Warn, "[{log_level}] {x}")` interpola level.
- `log_directive_file.kata` — `@log{msg: "...", file: stdout}` escreve em stdout.
- `log_directive_multiplas.kata` — duas `@log` (uma topic, uma file) ambas disparam.
- `log_directive_log_level.kata` — `@log{msg: "[{log_level}] {x}", ...}` interpola level.
- `log_recv_result_ok.kata` — `log_recv!` retorna `Ok(msg)`.
- `log_recv_result_err.kata` — `log_recv!` em tópico inexistente retorna `Err`.
- `log_file_rejeita_policy.kata` — `log!(level, msg, stdout!(), "drop")` é erro de tipo.
- `log_directive_topic_file_exclusivos.kata` — `@log{topic: ..., file: ...}` é erro.

**Verificação:** `cargo test --workspace --no-fail-fast`, 0 failed. ✅ 17 testes passando.

### Fase 9: Nível 2 — registry por-fiber com close na destruição ✅

- Adicionar `CURRENT_FIBER_ID: Cell<Option<FiberId>>` TLS em `scheduler.rs`.
- `resume_fiber`: setar `CURRENT_FIBER_ID` com o fiber_id atual (linha 423,
  onde `self.current_fiber = Some(fiber_id)` já é feito).
- `FiberEntry` ganha campo `open_files: Vec<i64>`.
- `kata_rt_file_open`: após `alloc_file_inner`, determinar se o `arena_handle`
  é `fiber_arena` ou não. Se fiber_arena, registrar em `FiberEntry.open_files`
  via `CURRENT_FIBER_ID` + `SCHEDULER` TLS (acesso sem borrow check — ver
  padrão em `scheduler/ffi.rs:139`). Se não, registrar no registry global
  `OPEN_FILES`.
- `kata_rt_file_close`: remover handle de `open_files` (se presente) e/ou
  `OPEN_FILES` (se presente) antes de `drop_in_place`.
- `try_destroy`: antes de `kata_rt_arena_destroy(arena_handle)`, iterar
  `entry.open_files` e chamar `kata_rt_file_close` em cada handle. Limpar Vec.
- `reset_file_registry`: continua drenando `OPEN_FILES` para handles não-fiber.
- Limpar `CURRENT_FIBER_ID` em `try_destroy` após processar `open_files`.

**Arquivos:**
- `crates/kata-rt/src/scheduler.rs` — `CURRENT_FIBER_ID` TLS, `open_files` em `FiberEntry`, close em `try_destroy`
- `crates/kata-rt/src/file.rs` — `file_open` bifurca registro (fiber vs global), `file_close` remove de ambos

**Verificação:** `cargo test --workspace --no-fail-fast` — teste E2E onde
fiber abre arquivo sem `close!()`, fiber termina, FD é fechado em `try_destroy`. ✅ Implementado com `CURRENT_FIBER_ARENA` + `FIBER_OPEN_FILES` TLS (swap in/out em `resume_fiber`), `open_files` em `FiberEntry`, close em `try_destroy`. Teste `fiber_fecha_arquivo_sem_close` passando.

## 5. Fora do escopo

- Multiplataforma Windows (`from_raw_fd` é Unix-only) — o runtime já assume
  Unix. Fallback futuro se necessário.
- Diretivas Kata genéricas (decorators) — este PRD alinha `@log` com `@test`
  no padrão de múltiplas ocorrências, mas não implementa o sistema de
  decorators. Isso é trabalho futuro.
- `log!()` com sintaxe nomeada (`log!{level: ..., msg: ..., file: ...}`) —
  Fase 8 do PRD-fio14-log, não implementada.
- Use-after-close entre fibers: se a fiber A fecha arquivo que fiber B ainda
  usa. Exigiria ownership tracking ou refcount de FileInner. Não resolvido.

## 6. DoDs (Definitions of Done)

1. `stdin!()` retorna `File` apontando para FD 0. `stdout!()` retorna `File`
   apontando para FD 1. `stderr!()` retorna `File` apontando para FD 2.
   Todos disponíveis via `import stdio.(...)`, não no prelude.
2. `echo!(msg, stdout!())` escreve em stdout via File. `echo!(msg, stderr!())`
   escreve em stderr via File.
3. `close!(stdout!())` é no-op — não fecha FD 0/1/2.
4. `read!(stdin!())` lê de stdin via File (retorna `Result::(Bytes, Text)`).
5. `write!(stdin!(), "msg")` retorna `Err("not writable")` (stdin é read-only).
6. `read!(stdout!())` retorna `Err("not readable")` (stdout é write-only).
7. `log!(LogLevel::Info, "msg {x}", stdout!())` escreve `"msg 42"` em stdout
   via `kata_rt_file_write_text`. `{log_level}` interpola para `"Info"`.
8. `log!(LogLevel::Info, msg, "topic")` publica em canal CSP (como hoje).
9. `log!(LogLevel::Info, msg, stdout!(), "drop")` é erro de tipo (policy com
   File).
10. Múltiplas `@log` na mesma action/função: cada uma injeta independentemente.
11. `@log{msg: ..., topic: "metrics"}` publica em CSP. `@log{msg: ..., file:
    stdout}` escreve direto. Ambos na mesma action.
12. `@log{topic: "x", file: stdout}` é erro (mutuamente exclusivos).
13. `log_recv!("topic")` retorna `Result::(Text, Text)`. `Ok(msg)` se sucesso,
    `Err(reason)` se tópico inexistente ou canal fechou.
14. Tópicos mágicos `"stdout"`/`"stderr"` removidos do `kata_rt_log_publish`.
15. `kata_rt_file_open` recebe `arena_handle` do codegen via escape analysis.
    `FileInner` alocado na arena correta (`fiber_arena` se local,
    `caller_arena` se retornado, `root_arena` se enviado via canal).
16. `FileInner` abertos sem `close!()` explícito têm FDs fechados por
    `reset_file_registry` no `reset_scheduler` entre testes.
17. `stdin!()`/`stdout!()`/`stderr!()` não são registrados em `OPEN_FILES`
    (is_stdio).
18. `CURRENT_FIBER_ID` TLS rastreia a fiber em execução. `try_destroy`
    fecha FDs locais (`open_files` da fiber) antes de destruir a arena.
19. `cargo test --workspace --no-fail-fast` passa sem regressão.
20. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

## 7. Arquitetura — componentes afetados

```
stdlib/stdio.kata                             # stdin/stdout/stderr actions (novo módulo)
crates/kata-rt/src/file.rs                    # is_stdio, kata_rt_stdin/stdout/stderr, close guard, OPEN_FILES registry, reset_file_registry, file_open bifurca registro (fiber vs global), guards read/write por mode
crates/kata-rt/src/log.rs                     # remover tópicos mágicos, log_recv → Result box
crates/kata-rt/src/lib.rs                    # re-exports, builder.symbol
crates/kata-rt/src/scheduler.rs              # CURRENT_FIBER_ID TLS, open_files em FiberEntry, close em try_destroy (Fase 9)
crates/kata-rt/src/scheduler/ffi.rs          # reset_scheduler chama reset_file_registry
crates/kata-core/src/ffi.rs                   # FfiSymbol::Stdin, Stdout, Stderr
crates/kata-codegen/src/ffi_sigs.rs           # assinatura stdin/stdout/stderr
crates/kata-codegen/src/ffi_registry.rs       # registro stdin/stdout/stderr
crates/kata-resolution/src/types.rs           # LogSpec.file, ActionDef.log → Vec
crates/kata-resolution/src/directives.rs     # extract_log_specs (plural), chave file:
crates/kata-resolution/src/lib.rs            # chamada extract_log_specs
crates/kata-inference/src/typed_module.rs    # TypedLogSpec.file, TypedAction.log → Vec
crates/kata-inference/src/infer/log_synthesis.rs  # synthesize_log_specs (plural)
crates/kata-inference/src/infer/action_infer.rs   # chamada
crates/kata-inference/src/infer/mod.rs            # chamada
crates/kata-inference/src/infer/log_builtins.rs   # log! bifurcação, log_recv → Result
crates/kata-codegen/src/lowering/log.rs       # iterar + bifurcar (topic → CSP, file → write)
crates/kata-codegen/src/lowering/action_def.rs    # iterar sobre Vec<TypedLogSpec>
crates/kata-codegen/src/lowering/function_def.rs   # iterar sobre Vec<TypedLogSpec>
crates/kata-driver/tests/                    # testes E2E novos
crates/kata-codegen/tests/                   # testes E2E novos
```

## 8. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | `stdin`/`stdout`/`stderr` como `File` via FFI em módulo `stdio` | Reusa toda a maquinaria de File I/O (`echo`, `write`, `read`, `close`). Sem novo tipo. Módulo separado mantém o prelude enxuto — stdio é opt-in, não importado por padrão. |
| D2 | `is_stdio` flag em `FileInner` | Previne `close!` de fechar FD 0/1/2. Simples, não invasivo. |
| D3 | Handle cached em TLS lazy static | Múltiplas chamadas a `stdout!()` retornam o mesmo handle. Sem leak. |
| D4 | `log!()` bifurca por tipo do 3º arg | Mesmo nome, comportamento distinto por tipo. Consistente com overload de `echo`. |
| D5 | `@log` com chave `file:` distinta de `topic:` | Não mistura conceitos (canal CSP vs arquivo). Mutuamente exclusivo. |
| D6 | Sobrecarga de `@log` via `Vec<LogSpec>` | Alinha com `@test` (já suporta múltiplas). Prepara para diretivas Kata. |
| D7 | `log_recv!` retorna `Result` | Mais ergonômico que `0` silencioso. Reusa padrão de `file_read`, `dict_get_checked`. |
| D8 | Remoção dos tópicos mágicos | stdout/stderr são `File`, não strings. Elimina comportamento especial no runtime. |
| D9 | `log!()` msg é template (não Text puro) | Unifica `log!()` e `@log` no mesmo mecanismo. Permite `{log_level}` sem formatação manual. Quebra retrocompatibilidade para strings com `{` não escapado. |
| D10 | `{log_level}` é variável sintética no escopo do template | Resolve para a string do level (`"Info"`, etc.). Disponível em `@log` e `log!()`. Derivada da tag i64 do 1º arg. |
| D11 | `stdin`/`stdout`/`stderr` em módulo `stdio` separado | stdio é I/O de processo — conceitualmente distinto do prelude (tipos/interfaces base). Módulo opt-in: programa que não usa terminal não paga custo. Core pode importar `stdio` se precisar. |

## 9. Riscos

| Risco | Mitigação |
|---|---|
| `close!(stdout!())` antes do fim do programa | `is_stdio` flag — close é no-op. |
| `stdout!()` chamado múltiplas vezes cria múltiplos FileInner | TLS lazy static — cria uma vez, cacheia. |
| `log!()` com `File` quebrado (arquivo fechado) | `kata_rt_file_write_text` retorna `Err` — o `log!` ignora o retorno (fire-and-forget). Documentar. |
| Mudança de `Option<LogSpec>` para `Vec<LogSpec>` quebra código que assume `Some/None` | Migrar todos os sites: `action_def.rs`, `function_def.rs`, `action_infer.rs`, `mod.rs`. Busca grep por `.log` em `typed_module.rs`. |
| `log_recv!` → `Result` quebra testes E2E existentes | Atualizar todos os testes em `log_e2e.rs` para usar `match`. |
| `log!()` msg como template quebra strings com `{` não escapado | Documentar migração: `{{` para `{` literal. Buscar usos de `log!()` em examples/ e testes. |
| `from_raw_fd` é Unix-only | Runtime já assume Unix. Se necessário, `#[cfg(unix)]` + fallback. |
| Use-after-close: fiber A fecha arquivo que fiber B ainda usa | Não resolvido neste PRD. Exigiria ownership tracking ou refcount de FileInner. Registry global só fecha no fim do programa. |
| `reset_file_registry` fecha FDs em ordem arbitrária | Ordem não importa — cada close é independente e idempotente. |

## 10. Atualização da documentação

Ao concluir:
- `docs/PRD-stdio-alignment.md` — este arquivo (status → concluído)
- `docs/PRD-fio14-log.md` — atualizar: tópicos mágicos removidos, `log_recv!`
  retorna `Result`, `@log` suporta múltiplas ocorrências
- `docs/ROADMAP.md` — adicionar "Alinhamento stdio ✅" no pós-Fio 15
- `docs/Kata-lang-manual.md` — documentar `stdin!()`, `stdout!()`, `stderr!()`,
  `log!(level, msg, file)`, `@log{file: stdout}`