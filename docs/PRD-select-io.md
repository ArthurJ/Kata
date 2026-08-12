# PRD: Select I/O — Multiplexação de Canais, Files e Sockets

## Status

**Status:** ✅ Concluído (Fases 1-6)
**Data:** 2026-08-03
**Commit:** `6601e51` — `feat(rt,codegen,inference,parser): select combinado (channels + files) com FFI única + Expr::Block`
**Depende de:** PRD-file-io (File I/O — `read(handle, n)` streaming), Fio 11 (CSP — canais, `select` atual)
**Pré-requisito:** `read(handle, n)` / `kata_rt_file_read_chunk` implementado (Fase 5 do PRD-file-io) ✅
**Habilita:** Sockets/Pipes como handles de I/O no `select` ✅ Implementado (PRD-socket-io)

## 1. Objetivo

Generalizar o `select` para multiplexar não apenas canais CSP, mas também
leitura de File handles (`read(handle, n)`) e, futuramente, Sockets/Pipes.
Hoje `select` só aceita braços de canal (`rx !> nome: body`). Após esta
mudança, aceitará também braços de leitura de I/O (`read(handle, n) !> data: body`),
permitindo que um fiber espere simultaneamente por: mensagem de canal, chunk
de arquivo, ou dados de rede — com um único timeout compartilhado.

## 2. Motivação

### 2.1. Servidores precisam de multiplexação de I/O

Um servidor Kata que processa requests de múltiplas fontes (canal de outro
fiber, pipe de child process, socket de cliente) precisa esperar em todos
simultaneamente. Sem `select` genérico, o desenvolvedor teria que fazer
polling manual — loop com `recv` non-blocking + `read` non-blocking + sleep,
que é ineficiente e propenso a bugs.

### 2.2. `select` é a primitiva natural de multiplexação

Kata já tem `select` para canais. Generalizar para I/O é a extensão natural
— em vez de inventar uma nova primitiva (`poll`, `epoll`), reutiliza a
sintaxe e o mecanismo de scheduler que já existe. O desenvolvedor usa a
mesma estrutura para canais e I/O.

### 2.3. Prepara o terreno para Sockets

`IoHandle` (camada comum File/Socket) já existe no runtime. Quando Sockets
forem adicionados, `select` já saberá lidar com eles — um socket é apenas
mais um FD para fazer `poll(POLLIN)`. A generalização de `select` para files
estabelece o padrão que sockets seguirão.

## 3. Design

### 3.0. Decisão de Design: Opção A (FFI Única Combinada)

**Decisão fechada:** Uma FFI única `kata_rt_select_combined(chan_ptr, n_c, file_ptr, n_f, timeout_ms) -> i64`
que tenta channels + files num loop, suspende uma vez com ambos os conjuntos.

Escolhida sobre duas alternativas (B: duas FFIs separadas com codegen combinando; C: módulo
runtime separado para files) porque:
- O loop try/suspend/retry é preocupação de runtime, não de codegen
- Mantém codegen simples (1 chamada + branch chain)
- Reusa padrão existente de `kata_rt_select`
- Menos FFIs e menos registro
- A "modularidade" de C é falsa — channels e files precisam ser tentados/suspensos/resumidos juntos

**Assinatura:** `kata_rt_select_combined(chan_ptr, n_c, file_ptr, n_f, timeout_ms) -> i64`
- Retorna índice global: `0..n_c-1` = channel arm, `n_c..n_c+n_f-1` = file arm
- `-1` = WOULD_BLOCK, `-2` = SELECT_TIMEOUT

### 3.1. Sintaxe — braços mistos de canal e I/O

```kata
select
  rx !> msg: echo!("canal: " + msg)
  read(file, 4096) !> data: processa_chunk(data)
  timeout 5000: echo!("timeout")
```

- **Braço de canal:** `receiver !> binding: body` — sintaxe existente, sem mudança.
- **Braço de I/O:** `read(handle, n) !> binding: body` — nova sintaxe.
  - `read(handle, n)` é a expressão de leitura (action `@ffi`).
  - `!>` sobrecarregado: "o dado lido flui para `binding`".
  - O `n` controla quantos bytes ler por chunk — decisão explícita do desenvolvedor.
- **Timeout:** `timeout N: body` — sintaxe existente, sem mudança.

### 3.2. Semântica de `read(handle, n) !> binding` dentro de select

Fora de `select`, `read!(handle, 4096)` executa imediatamente e retorna
`Result::(Bytes, Text)`. Dentro de `select`, a mesma expressão significa
"registrar interesse em readiness deste handle, e quando pronto, executar
`read` e conectar o resultado ao binding".

Isto é análogo ao que já acontece com `!>` em canais: fora de `select`,
`rx !> nome` é recebimento bloqueante; dentro de `select`, é "registrar
interesse neste receiver". O `select` reinterpreta os operadores dos braços
como "registro de interesse" em vez de "executar agora".

O binding recebe `Result::(Bytes, Text)` — o desenvolvedor faz `match` no
body para distinguir Ok (dados) de Err (EOF ou erro):

```kata
select
  read(file, 4096) !> result:
    match result
      Result::Ok chunk: echo!("li " + show(length(chunk)) + " bytes")
      Result::Err msg: echo!("fim: " + msg)
```

### 3.3. Sem polimorfismo de tipo — codegen separa por tipo

Handles são pseudo-tipos (`Ty::File`, `Ty::Channel(T)`) sem subtyping ou
trait. O runtime não faz dispatch dinâmico por tipo de handle. Em vez disso,
o **codegen** conhece os tipos dos braços em compile-time e gera código
separado para cada categoria:

1. **Channel handles:** array de handles de canal → `try_select_channels`
   (in-memory, `can_recv`)
2. **File handles:** array de handles de file → `try_select_files`
   (`poll(POLLIN, timeout=0)` non-blocking nos FDs)
3. Se algum pronto: dispatch para o braço correspondente
4. Se nenhum pronto: suspender fiber

O runtime recebe arrays separados e não precisa saber distinguir handle de
canal de handle de file — o codegen já separou.

### 3.4. Estrutura de dados do codegen

```rust
/// Braço de select — generalizado para canal ou I/O.
pub enum SelectArm {
    /// `rx !> nome: body` — braço de canal (existente).
    Channel {
        channel: Spanned<Expr>,
        bind_name: String,
        body: Spanned<Expr>,
    },
    /// `read(handle, n) !> nome: body` — braço de leitura de I/O.
    /// handle_expr avalia para File (futuramente Socket).
    /// chunk_size_expr avalia para Int (n bytes por chunk).
    IoRead {
        handle_expr: Spanned<Expr>,
        chunk_size_expr: Spanned<Expr>,
        bind_name: String,
        body: Spanned<Expr>,
    },
}
```

O parser diferencia os dois casos: se o braço começa com `read(...)` seguido
de `!>`, é `IoRead`. Se começa com uma expressão de canal seguida de `!>`,
é `Channel`. A distinção é sintática — `read` é keyword/action conhecida.

### 3.5. Runtime — duas funções de try_select

```rust
/// try_select_channels — in-memory, sem syscall.
/// Já existe hoje (can_recv em cada handle).
fn try_select_channels(handles: &[i64]) -> i64;

/// try_select_files — poll non-blocking nos FDs.
/// Nova. Para arquivos regulares, poll retorna "pronto" instantaneamente.
/// Para pipes/sockets, poll(POLLIN, timeout=0) é non-blocking.
fn try_select_files(handles: &[i64]) -> i64;
```

`try_select_files` extrai o FD de cada `FileInner` (via
`file_from_handle` → `io.file.as_raw_fd()`) e faz `poll(POLLIN, timeout=0)`.

### 3.6. Generalização do scheduler

Hoje `YieldReason::WaitingOnSelect(Vec<i64>, Option<Instant>)` carrega apenas
handles de canal. O scheduler faz `can_recv` no wake_pass e `poll_ipc` no
sleep path.

**Mudança:** `WaitingOnSelect` passa a carregar dois arrays:

```rust
WaitingOnSelect {
    channel_handles: Vec<i64>,
    file_handles: Vec<i64>,
    deadline: Option<Instant>,
}
```

O scheduler no wake_pass:
1. `can_recv` em cada channel_handle (in-memory)
2. `poll(POLLIN, timeout=0)` em cada file_handle (non-blocking)
3. Se algum pronto: acordar o fiber

O scheduler no sleep path (run_queue vazia, blocked non-empty):
1. Coletar IPC handles (canais cross-process) — já faz hoje
2. Coletar FDs de file handles — novo
3. `poll` em todos os FDs (IPC + files) com timeout = remaining
4. Após poll: wake_pass

Isto unifica o poll de IPC e de files num único `poll` syscall — o scheduler
já faz `poll_ipc_with_timeout`, generaliza para incluir FDs de files.

### 3.7. Arquivos regulares nunca bloqueiam

Para arquivos regulares (disco), `poll(POLLIN)` retorna "pronto" imediatamente
— sempre. Então `try_select_files` retorna o índice no primeiro check, e o
`read(handle, n)` subsequente é uma syscall que retorna os dados ou EOF. Sem
bloqueio da thread.

Para pipes/sockets (futuro), `poll(POLLIN, timeout=0)` é non-blocking. Se
não há dados, retorna WOULD_BLOCK e o fiber suspende. O scheduler faz
`poll` blocking (com timeout) no sleep path — igual ao padrão IPC atual.

### 3.8. FFI — nova função para try_select_files

```rust
/// try_select_files — verifica quais file handles têm dados para leitura.
/// Retorna o índice (0..N-1) do primeiro handle pronto, ou WOULD_BLOCK (-1).
///
/// Usa poll(POLLIN, timeout=0) — non-blocking. Para arquivos regulares,
/// sempre retorna "pronto". Para pipes/sockets, verifica readiness real.
///
/// # Safety
/// `handles` deve apontar para um array de `n_handles` handles de File válidos.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_select_files(handles: *const i64, n_handles: i64) -> i64;
```

O codegen chama `kata_rt_select` (channels) e `kata_rt_select_files` (files)
separadamente. Se ambos retornam WOULD_BLOCK, suspende o fiber.

### 3.9. Codegen — lower_select generalizado

O `lower_select` atual:
1. Aloca array de N handles na arena
2. Avalia `arm.channel` para cada braço, store no array
3. Chama `kata_rt_select(handles, N, timeout_ms)` → idx
4. Branch chain por idx

O `lower_select` generalizado:
1. **Separa braços por tipo** em compile-time:
   - `channel_arms`: braços Channel (índices originais preservados)
   - `io_arms`: braços IoRead (índices originais preservados)
2. **Aloca dois arrays** na arena: `channel_handles[N_c]` e `file_handles[N_f]`
3. **Avalia e store** handles em cada array
4. **Chama `kata_rt_select`** (channels) e **`kata_rt_select_files`** (files)
5. **Combina resultados:**
   - Se channels retornou idx ≥ 0: braço channel[idx]
   - Se files retornou idx ≥ 0: braço io[idx]
   - Se ambos WOULD_BLOCK: suspender fiber
   - Se timeout expirou: timeout_body
6. **Branch chain** para o braço vencedor:
   - Channel: `channel_recv(handle)` → binding → body
   - IoRead: `kata_rt_file_read_chunk(handle, n)` → binding (Result) → body

O branch chain precisa mapear índices dos arrays separados de volta aos
índices originais dos braços (para saber qual body executar).

## 4. Exemplos de uso

### 4.1. Servidor lendo de canal e arquivo simultaneamente

```kata
action servidor (requests::Channel(Request), logfile::File) => Unit
  loop
    select
      requests !> req: handle!(req)
      read(logfile, 4096) !> result:
        match result
          Result::Ok chunk: parse_log!(chunk)
          Result::Err msg: return
      timeout 1000: heartbeat!()
```

### 4.2. Processador de stream com canal de controle

```kata
action processa_stream (input::File, ctrl::Channel(Cmd)) => Unit
  loop
    select
      ctrl !> cmd:
        match cmd
          Cmd::Stop: return
          Cmd::Reset: seek!(input, 0)
      read(input, 8192) !> result:
        match result
          Result::Ok chunk: transform!(chunk)
          Result::Err msg: return
```

### 4.3. Select só de files (sem canais)

```kata
select
  read(file_a, 1024) !> a: echo!("A: " + show(length(a)))
  read(file_b, 1024) !> b: echo!("B: " + show(length(b)))
  timeout 500: echo!("timeout")
```

Funciona — `channel_arms` é vazio, `kata_rt_select` não é chamado, só
`kata_rt_select_files`.

## 5. Fases de implementação

### Fase 0: Pré-requisitos — ✅ Concluído

- `read(handle, n)` / `kata_rt_file_read_chunk` implementado (PRD-file-io Fase 5) ✅
- `BufReader` persistente em `FileInner` (PRD-file-io Fase 6) ✅
- `FileInner` expõe o FD bruto para `poll` via `collect_file_fds` ✅

### Fase 1: AST e parser — ✅ Concluído

- `SelectArm` vira enum com variants `Channel` e `IoRead` ✅
- Parser: distinguir `read(...) !>` de `expr !>` no braço do select ✅
- `IoRead` arm: `handle_expr`, `chunk_size_expr`, `bind_name`, `body` ✅
- Compatibilidade: braços só de canal continuam funcionando ✅

### Fase 2: Runtime — ✅ Concluído

- `kata_rt_select_combined(chan_ptr, n_c, file_ptr, n_f, timeout_ms)` — FFI única combinada ✅
- `collect_file_fds` em `file.rs` — coleta FDs brutos de file handles ✅
- `ipc_read_fd` em `channel/ipc.rs` — retorna read_fd bruto de canal IPC ✅
- Scheduler: sleep path com poll unificado (IPC + file FDs) com timeout ✅
- Scheduler: poll blocking para file FDs sem deadline ✅

### Fase 3: Codegen — ✅ Concluído

- `lower_select` reescrito: FFI única `kata_rt_select_combined` ✅
- Branch chain compara `global_idx` (channels: `idx < n_c`; files: `idx >= n_c`) ✅
- `FfiSymbol::SelectCombined` em `ffi.rs`, assinatura em `ffi_sigs.rs`, registro em `ffi_registry.rs` ✅
- Cache key para `Block` em `cache_key.rs` ✅

### Fase 4: Inferência — ✅ Concluído

- `infer_select` unifica tipos dos **bodies** dos braços (não tipos dos bindings) ✅
- Permite selects mistos (channel binding `Int` + IoRead binding `Result::(Bytes, Text)`, ambos bodies retornam `Int`) ✅
- Typecheck de `IoRead` arm: `handle_expr` deve ser `Ty::File`, `chunk_size_expr` deve ser `Ty::Int` ✅
- Binding recebe `Result::(Bytes, Text)` ✅

### Fase 5: Testes E2E — ✅ Concluído

- `select_io_e2e.rs` — 4 testes: select só-files ✅, select misto ✅, select EOF ✅, select timeout ✅
- `let_in_match_e2e.rs` — 2 testes: let em match arm indentado ✅, let em match arm same-line + indentado ✅
- Total: 1361 passed, 0 failed, 5 ignored

### Fase 6: Documentação — ✅ Concluído

- `sintaxe-mapa.md` — seção `select` atualizada com braços de I/O ✅
- `PRD-select-io.md` — status e fases atualizados ✅
- `ROADMAP.md` — feature select-io adicionada ✅

## 6. Decisões fechadas

| Decisão | Valor | Justificativa |
|---|---|---|
| FFI única combinada | `kata_rt_select_combined` (Opção A) | Loop try/suspend/retry é de runtime; codegen simples com 1 chamada |
| Sintaxe do braço de I/O | `read(handle, n) !> binding: body` | Explícito, controle de n bytes |
| `!>` sobrecarregado | Canal e I/O usam `!>` | Mesmo conceito: "dado flui para binding" |
| Binding de I/O recebe `Result` | `Result::(Bytes, Text)` | Consistente com `read!(handle, n)` fora de select |
| Sem polimorfismo de tipo | Codegen separa handles por tipo em compile-time | Handles são pseudo-tipos sem subtyping |
| Runtime não distingue handle | Codegen passa arrays separados | Sem tagging, sem dispatch dinâmico |
| `try_select_files` usa `poll` | `poll(POLLIN, timeout=0)` non-blocking | Arquivos regulares sempre prontos; pipes/sockets cooperativos |
| Arquivos regulares não bloqueiam | `poll` retorna "pronto" imediatamente | Sem bloqueio de thread |
| Scheduler generalizado | `WaitingOnSelect` carrega channels + files | Poll unificado no sleep path |
| `SelectArm` vira enum | Variants `Channel` e `IoRead` | Tipos diferentes de braço |
| Pré-requisito | `read(handle, n)` implementado | Select I/O precisa de read_chunk |
| `infer_select` unifica bodies | Tipos dos bodies, não dos bindings | Permite select misto (channel `Int` + IoRead `Result`, bodies `Int`) |

## 7. Decisões em aberto

### 7.1. Outras operações de I/O no select (futuro)

Hoje só `read(handle, n)` é suportado em braços de I/O. Futuramente:
- `readline(handle) !> line: body` — leitura linha-a-linha em select
- `write(handle, data) !> _: body` — readiness para escrita (POLLOUT)
- `accept(socket) !> conn: body` — aceitar conexão em select

Por ora, apenas `read(handle, n)`. As outras seguem o mesmo padrão quando
existirem.

### 7.2. Sockets no select — ✅ Implementado

Sockets foram adicionados como `Ty::Socket` (PRD-socket-io). O `select` com
`read(socket, n) !> data: body` funciona — o codegen separa `file_arms` e
`socket_arms` por tipo em compile-time e passa arrays separados para
`kata_rt_select_combined` (estendida com `socket_ptr` + `n_s` params, 7 args
no total). O runtime chama `try_select_sockets` para socket handles (não
pode usar `try_select_files` — cast para `FileInner` sobre `SocketInner`
produziria FD lixo por layout de memória diferente).

`poll(POLLIN)` em socket bloqueia cooperativamente (não retorna "pronto"
instantaneamente como arquivos regulares). 2 testes E2E de select com
socket passam: `socket_select_with_socket` e `socket_select_misto_channel_socket`.

### 7.3. Ordem de prioridade entre channels e files

Quando um channel e um file estão prontos simultaneamente, qual braço
executa? Hoje `try_select` retorna o primeiro índice encontrado. Com arrays
separados, o codegen precisa definir uma ordem: channels primeiro, ou files
primeiro, ou intercalado?

**Sugestão:** channels primeiro (in-memory check é O(1),.syscall tem mais
overhead). Se um channel está pronto, não precisa nem chamar
`try_select_files`. Mas isto pode starvation files se channels sempre têm
dados. Avaliar na implementação.

## 8. Dependências

| Dependência | Status | Nota |
|---|---|---|
| `read(handle, n)` / `kata_rt_file_read_chunk` | PENDENTE (PRD-file-io Fase 5) | Pré-requisito absoluto |
| `BufReader` persistente em `FileInner` | PENDENTE (PRD-file-io Fase 6) | Consistência de cursor |
| `IoHandle` comum File/Socket | ✅ Existe | `file.rs` já tem `IoHandle` |
| `select` de canais | ✅ Implementado | Base a generalizar |
| Scheduler cooperativo com `YieldReason` | ✅ Implementado | Base a estender |
| `poll(POLLIN)` para IPC | ✅ Implementado | Padrão a reutilizar para files |

## 9. Riscos

| Risco | Mitigação |
|---|---|
| `poll` em arquivo regular sempre retorna pronto — select spin | Para arquivos regulares, `read` retorna EOF rapidamente; o fiber sai do loop naturalmente |
| `FileInner` não expõe FD bruto | Adicionar método `as_raw_fd()` ou acessar `io.file.as_raw_fd()` via `std::os::unix::io::AsRawFd` |
| Parser ambíguo: `read(...) !>` vs `expr !>` | `read` é keyword/action conhecida — parser verifica se o braço começa com `read(` antes de `!>` |
| `WaitingOnSelect` breaking change | Enum já é `#[derive(Debug, Clone)]` — mudar struct interna não quebra ABI externa |
| Scheduler poll unificado pode ser complexo | Reutilizar `poll_ipc_with_timeout` como base, generalizar para poll de múltiplos FDs |