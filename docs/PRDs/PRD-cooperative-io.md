# PRD: I/O Cooperativo — File e Socket com Yield Entre Chunks

## Status

**Status:** 🔴 Pendente
**Data:** 2026-09-16
**Depende de:** PRD-file-io (File I/O — `FileInner`, FFI de read/write/readline), PRD-socket-io (Socket I/O — `SocketInner`, suspensão via `WaitingOnSelect`)
**Resolve:** TODO.md itens 🔴#1 (File I/O bloqueia scheduler), 🟡#5 (`read!(Socket)` slurp sem limite), `read!(File)` slurp sem limite

## 1. Objetivo

Tornar todo I/O de File cooperativo com o scheduler: leitura em chunks com
yield cooperativo entre sycalls, suspensão em EAGAIN para pipes/FIFOs/FUSE,
e yield no slurp de `read!` sem `n` para File e Socket. Unificar File e Socket
no mesmo modelo de I/O non-blocking com `raw_read`/`raw_write` direto no FD.

## 2. Motivação

### 2.1. File I/O bloqueia o scheduler inteiro

`kata_rt_file_read` usa `BufReader::read_to_end` — uma syscall blocking.
`kata_rt_file_readline` usa `BufReader::read_line` — blocking. O FD não é
non-blocking. Nenhuma chamada `with_suspend` existe no caminho de file. Um
`read!(File)` num arquivo grande congela todos os fibers até a syscall
completar.

Socket I/O já é cooperativo: FD non-blocking, `raw_read` direto, suspensão
em EAGAIN via `WaitingOnSelect`. File I/O precisa seguir o mesmo modelo.

### 2.2. Slurp sem limite em `read!` sem `n`

`kata_rt_socket_read` faz loop de 8KB em 8KB até EAGAIN sem ceder CPU.
`kata_rt_file_read` faz `read_to_end` — uma syscall blocking. Ambos
jogam o conteúdo integral na arena sem yield. Para uma linguagem com
arena allocation, um slurp de megabytes sem controle é perigoso.
`read!(handle, n)` existe para controle explícito, mas `read!(handle)`
sem `n` deveria pelo menos ceder CPU entre chunks para não monopolizar
o scheduler.

### 2.3. `BufReader` é incompatível com non-blocking

`BufReader::read` traduz `EAGAIN` para `WouldBlock` e retorna como erro —
sem retry/suspend. O socket bypass `BufReader` e usa `raw_read` direto
justamente por isso. Enquanto File usar `BufReader`, não pode ser
non-blocking.

## 3. Design

### 3.1. `FileInner` — FD bruto em vez de `BufReader`

```rust
pub(crate) struct FileInner {
    pub closed: bool,
    pub fd: i32,            // FD bruto (igual SocketInner)
    pub io: IoHandle,
    pub is_stdio: bool,
    pub path: String,
    pub line_buf: Vec<u8>,  // Buffer parcial para readline (igual SocketInner)
}
```

O `BufReader<File>` é removido. O FD bruto permite `raw_read`/`raw_write`
direto e poll uniforme. O `line_buf` acumula bytes parciais entre chamadas
de `readline` — mesmo pattern do `SocketInner`.

### 3.2. Non-blocking no `file_open`

`kata_rt_file_open` extrai o FD bruto do `File` via `into_raw_fd` (Unix) /
`into_raw_handle` (Windows), chama `set_nonblocking(fd)`, e armazena o FD
em `FileInner.fd`. Para arquivos regulares locais, `O_NONBLOCK` é no-op
(kernel ignora). Para pipes/FIFOs/FUSE, habilita EAGAIN — permite suspensão
cooperativa como o socket.

### 3.3. Chunked read com yield cooperativo

`kata_rt_file_read` e `kata_rt_file_read_chunk` leem em chunks de 64KB
via `raw_read`. Entre chunks, fazem yield cooperativo: `with_suspend(|s|
s.suspend(YieldReason::Cooperative))`. O scheduler faz round-robin —
outros fibers rodam. O fiber resumido continua a leitura.

Se `raw_read` retorna EAGAIN (pipe/FIFO non-blocking), suspende via
`WaitingOnSelect { file_handles: [handle], .. }` — o scheduler faz poll
no FD e resume quando há dados.

```
const CHUNK: usize = 64 * 1024;
loop {
    n = raw_read(fd, buf, CHUNK)
    if n > 0 { data.extend(...); yield_cooperative(); continue }
    if n == 0 { break }  // EOF
    if EAGAIN {
        if data.is_empty() { suspend(WaitingOnSelect{file_handles:[handle]}); continue }
        break  // partial read — retorna o que leu
    }
    return Err
}
```

### 3.4. Sem cap no slurp de `read!` sem `n`

`read!` sem `n` lê o conteúdo integral com yield cooperativo entre
chunks. O scheduler não congela — outros fibers rodam entre as syscalls
de 64KB. A arena recebe o arquivo completo. A responsabilidade de não
jogar 500MB na arena é do caller: se precisa de controle, usa
`read!(handle, n)`. Se pediu `read!(handle)`, está pedindo o arquivo
inteiro.

### 3.5. `readline` com `raw_read` + `line_buf`

`kata_rt_file_readline` reescrito com o mesmo pattern do
`kata_rt_socket_readline`: `raw_read` em chunks de 8KB, acumula em
`line_buf`, procura `\n`. Yield cooperativo entre reads. Suspensão em
EAGAIN. EOF com dados no buffer retorna linha parcial; EOF com buffer
vazio retorna `Err("EOF")`.

**Limitação:** não misturar `readline` com `read`/`read_chunk` no mesmo
handle — `read`/`read_chunk` lêem do FD diretamente, ignorando `line_buf`,
e consomem bytes que `readline` esperava. Mesma limitação do socket.

### 3.6. Write com `raw_write`

`kata_rt_file_write_text` e `kata_rt_file_write_bytes` usam `raw_write`
em vez de `file.write_all`. Para pipes com buffer cheio (EAGAIN),
suspendem via `WaitingOnSelect`. Para arquivos regulares, `write` é
blocking mas retorna em microssegundos (page cache).

### 3.7. Socket: cap e yield no slurp

`kata_rt_socket_read` ganha o mesmo cap de 1MB. O loop de 8KB já existe —
adicionar yield cooperativo entre iterações (atualmente lê tudo sem ceder
CPU se há muitos dados disponíveis).

### 3.8. stdio

`kata_rt_input` e `alloc_stdio_inner` reescritos para o novo `FileInner`
com `fd` e `line_buf`. `kata_rt_input` usa `raw_read` + `line_buf` em vez
de `buf_reader.read_line`. stdin (FD 0) é setado non-blocking — permite
suspensão cooperativa se stdin é um pipe.

### 3.9. Close

`kata_rt_file_close` muda de `drop_in_place` (que rodava Drop de
`BufReader→File`) para `close_fd(inner.fd)` (syscall direta). O campo
`closed` garante idempotência. A memória do `FileInner` permanece na
arena até o teardown.

## 4. Decisões de design

### D1: `BufReader` removido — `raw_read` direto no FD

**Escolha:** FD bruto + `raw_read`/`raw_write`. **Alternativa rejeitada:**
manter `BufReader` e adicionar thread pool para file I/O async. O thread
pool quebra o runtime single-threaded ("Decisão A" do scheduler) com
complexidade de sincronização. O yield entre chunks resolve o caso
comum (arquivos locais) sem threading.

### D2: Yield cooperativo entre chunks, não suspensão blocking

**Escolha:** `YieldReason::Cooperative` entre chunks — o fiber volta para
a run_queue e outros fibers rodam. **Alternativa rejeitada:** suspender
via `WaitingOnSelect` entre chunks. `Cooperative` é mais leve — não
exige poll, apenas round-robin. `WaitingOnSelect` é reservado para EAGAIN
(quando não há dados e o FD precisa de poll).

### D3: Sem cap no slurp — yield resolve o bloqueio

**Escolha:** `read!` sem `n` lê tudo, com yield entre chunks. **Alternativa
rejeitada:** cap de 1MB com partial read. O cap introduz ambiguidade: o
caller não distingue "li tudo" de "li exatamente o cap e acabou" —
precisaria de EOF tipado (TODO #3, fora de escopo) para resolver. Sem cap,
`read!` sem `n` tem semântica clara: retorna o conteúdo integral. A
responsabilidade de não jogar megabytes na arena é do caller, que dispõe
de `read!(handle, n)` para controle explícito.

### D4: Chunk de 64KB para yield

**Escolha:** 64KB por syscall. **Justificativa:** balanceia granularidade
do yield com overhead de syscall. Arquivos locais servem 64KB do page
cache em microssegundos — a janela de bloqueio é negligenciável. NFS/FUSE
lento pode bloquear uma syscall de 64KB por segundos, mas isso está fora
do escopo desta iteração.

### D5: `line_buf` no `FileInner` — mesma limitação do socket

**Escolha:** buffer manual de readline no `FileInner`, igual
`SocketInner`. **Consequência:** não misturar `readline` com
`read`/`read_chunk` no mesmo handle. **Alternativa rejeitada:** manter
`BufReader` só para readline. Dois caminhos de I/O (BufReader para
readline, raw_read para read) no mesmo handle causa state corruption —
bytes lidos por um não são visíveis ao outro.

### D6: EOF permanece como `Err("EOF")`

**Escolha:** manter `Err("EOF")` consistente com o comportamento atual.
**Alternativa rejeitada:** introduzir `IoError` tipado (TODO #3). Isso é
um PRD separado com blast radius maior (stdlib, codegen, todos os testes
E2E). Este PRD não muda a semântica de EOF.

### D7: Rodízio entre braços do `select`

**Escolha:** `kata_rt_select_combined` mantém um offset de rodízio em
TLS. A cada chamada, a checagem de braços começa a partir do próximo
braço da rodada anterior. Isto previne starvation quando um braço
always-ready (arquivo regular — `poll` sempre retorna "pronto") vence
todos os outros. **Alternativa rejeitada:** remover file handles do
select. O rodízio preserva a multiplexação sem restringir a API.

## 5. Fases

### Fase 1: `FileInner` — refactor estrutural

- Remover `BufReader<File>` de `FileInner`, adicionar `fd: i32` e
  `line_buf: Vec<u8>`
- `kata_rt_file_open`: extrair FD via `into_raw_fd`, `set_nonblocking`,
  construir `FileInner` com `fd`
- `kata_rt_file_close`: `close_fd(inner.fd)` em vez de `drop_in_place`
- `stdio.rs`: `alloc_stdio_inner` constrói `FileInner` com `fd`
- `file/select.rs`: `try_select_files` e `collect_file_fds` leem
  `inner.fd` em vez de `file_raw_fd(inner.buf_reader.get_ref())`
- `platform.rs`: remover `file_raw_fd` (FD agora é direto)
- **DoD:** `cargo build -p kata-rt` compila. Testes existentes passam.

### Fase 2: Read cooperativo

- Reescrever `kata_rt_file_read` com `raw_read` em chunks de 64KB, yield
  cooperativo entre chunks, suspensão em EAGAIN
- Reescrever `kata_rt_file_read_chunk` com yield cooperativo e suspensão
  em EAGAIN
- Reescrever `kata_rt_file_readline` com `raw_read` + `line_buf`, yield
  cooperativo, suspensão em EAGAIN
- **DoD:** `cargo test -p kata-rt` passa. E2E de file read/readline
  passam.

### Fase 3: Write cooperativo

- Reescrever `kata_rt_file_write_text` e `kata_rt_file_write_bytes` com
  `raw_write`, suspensão em EAGAIN
- **DoD:** E2E de file write passam.

### Fase 4: stdio

- Reescrever `kata_rt_input` com `raw_read` + `line_buf`
- **DoD:** E2E de stdio/input passam.

### Fase 5: Socket — yield no slurp

- Adicionar yield cooperativo entre chunks no loop de 8KB de
  `kata_rt_socket_read`
- **DoD:** E2E de socket read passam.

### Fase 6: Testes E2E novos

- `file_read_large_yields` — arquivo > 64KB, `read!` lê tudo mas scheduler
  não congela (outro fiber roda durante a leitura)
- `file_read_chunk_yield` — `read!(handle, n)` com n grande, scheduler
  não congela (outro fiber roda durante a leitura)
- `file_readline_large` — arquivo > 8KB com múltiplas linhas, readline
  preserva bytes parciais
- `socket_read_yield` — socket com muito dado disponível, `read!` lê
  tudo mas cede CPU entre chunks
- `file_read_pipe_suspends` — pipe non-blocking sem dados, `read!`
  suspende e resume quando pipe recebe dados

## 6. Estruturas afetadas

| Arquivo | Mudança |
|---|---|
| `kata-rt/src/file.rs` | `FileInner` perde `BufReader`, ganha `fd` + `line_buf`. Read/write/readline reescritos com `raw_read`/`raw_write`. Open seta non-blocking. Close usa `close_fd`. |
| `kata-rt/src/file/stdio.rs` | `alloc_stdio_inner` e `kata_rt_input` reescritos. |
| `kata-rt/src/file/select.rs` | `try_select_files` e `collect_file_fds` leem `inner.fd`. |
| `kata-rt/src/channel/select.rs` | `kata_rt_select_combined` com rodízio entre braços. TLS `SELECT_ROTATION`. |
| `kata-rt/src/socket/io.rs` | `kata_rt_socket_read` ganha yield cooperativo entre chunks. |
| `kata-rt/src/scheduler/ffi.rs` | `reset_tls_between_runs` reseta `SELECT_ROTATION`. |
| `kata-rt/src/platform.rs` | `file_raw_fd` removido. Adicionar `file_into_raw_fd` se não existir. |

## 7. Fora do escopo

- **EOF tipado (`IoError` enum)** — TODO #3. Blast radius maior (stdlib,
  codegen, testes). PRD separado.
- **Thread pool para file I/O truly async** — NFS/FUSE lento onde uma
  syscall de 64KB bloqueia por segundos. A janela de 64KB é aceitável
  para arquivos locais. Thread pool exige rework do runtime
  single-threaded.
- **`connect` TCP non-blocking** — TODO #2. Ortogonal a este PRD.
- **Trampoline engole erros** — TODO #6. Ortogonal.
- **`listen!` → `accept!`** — TODO #7. Ortogonal.

## 8. Riscos

### R1: Regressão em readline intercalado

A remoção do `BufReader` e introdução de `line_buf` muda a semântica de
readline quando intercalado com `read`/`read_chunk`. O `BufReader`
persistente era a solução para state corruption. **Mitigação:** o
`line_buf` cumpre o mesmo papel para readline. A limitação documentada
(não misturar readline com read/read_chunk) é a mesma do socket. Teste
E2E `file_readline_large` valida.

### R2: `kata_rt_input` regressão

`kata_rt_input` muda de `BufReader::read_line` para `raw_read` +
`line_buf`. stdin em terminal interativo pode ter comportamento
diferente com non-blocking. **Mitigação:** stdin em terminal é
blocking-read (não retorna EAGAIN). Non-blocking só ativa EAGAIN em
pipes. Teste E2E de input valida.

### R3: Windows — `raw_read` não funciona para files

No Windows, `raw_read` usa `recv` (Winsock), que não funciona para file
handles. **Mitigagem:** `raw_read` no Windows já é `recv` — usado para
sockets. Para files no Windows, precisamos de `ReadFile` ou manter
`std::fs::File::read`. Adicionar `#[cfg(windows)]` com path separado
usando `std::io::Read` para file FDs. O yield cooperativo entre chunks
ainda se aplica (a chamada `std::io::Read::read` é blocking mas retorna
em microssegundos para arquivos locais).