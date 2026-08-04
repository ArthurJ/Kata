# PRD: Socket I/O — Handles Opacos para Sockets TCP e Unix

## Status

**Status:** ✅ Concluído (Fases 1-5)
**Data:** 2026-08-04 (atualizado)
**Commit:** `14a1d46` — `fix(codegen,rt): separar file_arms e socket_arms no select`
**Depende de:** PRD-file-io (File I/O — `IoHandle`, close no epílogo, `Result::(T, Text)`), PRD-select-io (`select` com files, `poll(POLLIN)`, scheduler cooperativo)
**Habilita:** Servidores e clientes de rede em Kata; `select` com sockets

## 1. Objetivo

Introduzir sockets na linguagem Kata: criar listeners, aceitar conexões,
conectar a endereços remotos, ler e escrever dados via um handle opaco. O
handle é um valor `Socket` que o usuário passa para as actions de I/O — sem
enxergar a representação interna (FD, estado, endereço).

`Socket` é o segundo tipo de I/O depois de `File`. O runtime já tem
`IoHandle` como camada comum, e `select` já sabe fazer `poll(POLLIN)` em
FDs — a infraestrutura pesada está pronta.

## 2. Motivação

### 2.1. Rede é o próximo passo de I/O

File I/O cobre disco. Sem sockets, Kata não pode implementar servidores,
clientes de rede, ou comunicação inter-processo via Unix domain. Sockets
são o caso de uso natural que motiva `select` com multiplexação real —
diferente de arquivos regulares (sempre prontos), sockets bloqueiam de
verdade, e o scheduler cooperativo brilha.

### 2.2. `IoHandle` e `select` já preparam o terreno

O PRD-file-io introduziu `IoHandle` (camada comum File/Socket) e o
PRD-select-io generalizou `select` para multiplexar canais + file handles
via `poll(POLLIN)`. O PRD-select-io diz explicitamente (seção 7.2):

> Sockets serão adicionados como `Ty::Socket` no futuro. Quando existirem,
> `select` com `read(socket, n) <! data: body` funcionará naturalmente —
> o codegen trata Socket igual a File (ambos têm FD para poll).

Este PRD cumpre essa promessa.

### 2.3. Non-blocking encaixa no scheduler cooperativo

Sockets non-blocking são o caso perfeito para o scheduler cooperativo:
`poll(POLLIN, timeout=0)` no wake pass, `poll(POLLIN, timeout=remaining)`
no sleep path. O fiber suspende sem bloquear a thread. Isto já funciona
para canais IPC (que são pipes internamente) e files — sockets são o
mesmo padrão.

## 3. Design do tipo

### 3.1. `Socket` — tipo opaco intrínseco

`Socket` é uma variante dedicada de `Ty`, paralela a `File`:

```rust
pub enum Ty {
    // ... variantes existentes ...
    File,
    /// Socket — handle opaco para socket TCP/Unix aberto. Sem parametrização
    /// de tipo (encoding determinado pela operação: read → Bytes, write aceita
    /// Text ou Bytes). Ponteiro na ABI (i64). Pode ser Listener (passivo) ou
    /// Connected (ativo, full-duplex).
    Socket,
}
```

O usuário não enxerga fields, não faz pattern matching na estrutura, não
constrói `Socket` diretamente. O único modo de obter um `Socket` é via
`open!` (que retorna `Result::(Socket, Text)`), `listen!` (que retorna
`Result::(Socket, Text)` a partir de um listener), ou `connect!` (que
retorna `Result::(Socket, Text)`).

### 3.2. `SocketKind` — enum com 2 variantes (payload Text)

```kata
enum SocketKind
    TCP(Text)
    Unix(Text)
```

| Variante | Payload | Semântica |
|---|---|---|
| `TCP("host:porta")` | Endereço no formato `host:port` | Socket TCP IPv4/IPv6 |
| `Unix("/path")` | Path do socket file no filesystem | Unix domain socket |

O payload carrega o endereço. `TCP("127.0.0.1:8080")`,
`TCP("[::1]:8080")`, `Unix("/tmp/kata.sock")`.

### 3.3. `SocketMode` — enum com 2 variantes

```kata
enum SocketMode
    Listener    # open! → bind + listen → socket passivo
    Connected   # open! → connect → socket ativo (lê e escreve)
```

| Variante | Semântica | Syscalls | Operações válidas |
|---|---|---|---|
| `Listener` | Cria listener: binda no endereço e espera conexões | `socket() + bind() + listen()` | `listen!` apenas |
| `Connected` | Conecta a um endereço remoto | `socket() + connect()` | `read!`, `write!`, `close!` |

Não há `Read`/`Write`/`ReadWrite` como em `FileMode` porque:

- Todo socket conectado (TCP/Unix stream) é **full-duplex** — lê e
  escreve simultaneamente, por definição. Não há "socket só de leitura".
- O listener não transporta dados — só aceita conexões via `listen!`.
- `shutdown(2)` existe (fecha uma direção) mas é operação destrutiva,
  não modo de abertura.

### 3.4. Posição no sistema de tipos

`Socket` é tratado como ponteiro opaco pelo codegen (`ty_to_clif` →
`I64`), igual a `File`. No `TypeShape`, mapeia para `Prim` (valor escalar,
não coleção). O codegen rastreia sockets abertos em `io_handle_vars`
(generalização de `file_handle_vars`) para close automático no epílogo.

## 4. API — Actions no prelude

```kata
# ── Socket I/O ────────────────────────────────────────────────────
# Socket é um tipo intrínseco (Ty::Socket) — handle opaco para socket
# TCP ou Unix aberto. Pode ser Listener (passivo) ou Connected (ativo).

enum SocketKind
    TCP(Text)
    Unix(Text)

enum SocketMode
    Listener
    Connected

@ffi("kata_rt_socket_open")
action open (kind::SocketKind, mode::SocketMode) => Result::(Socket, Text)

@ffi("kata_rt_socket_listen")
action listen (listener::Socket) => Result::(Socket, Text)

@ffi("kata_rt_socket_read")
action read (s::Socket) => Result::(Bytes, Text)

@ffi("kata_rt_socket_read_chunk")
action read (s::Socket, n::Int) => Result::(Bytes, Text)

@ffi("kata_rt_socket_write_text")
action write (s::Socket, content::Text) => Result::(Unit, Text)

@ffi("kata_rt_socket_write_bytes")
action write (s::Socket, content::Bytes) => Result::(Unit, Text)

@ffi("kata_rt_socket_close")
action close (s::Socket) => Unit

# echo para socket — escreve show(msg) + newline
action echo (msg::SHOW, s::Socket) => Unit
    let s_val := show msg
    let _ := write!(s, s_val)
    let _ := write!(s, "\n")
```

### 4.1. Convenções

- **Tudo que pode falhar retorna `Result::(T, Text)`.** O `Text` do `Err` é
  a mensagem de erro. `close` e `echo` não falham (retornam `Unit`).
- **`write` tem 2 overloads** — `Text` e `Bytes`. Cada overload mapeia
  para sua própria FFI: `kata_rt_socket_write_text` e
  `kata_rt_socket_write_bytes`. Mesma convenção de File.
- **`echo` é action Kata composta**, não `@ffi`. Chama `write!` duas vezes.
- **`open` recebe `SocketKind` e `SocketMode` como enums.** O codegen
  extrai o tag da variante do Sum box via `sum_tag_int` e o payload (Text)
  do box.
- **`read` tem 2 overloads por aridade** — `read(s)` (slurp) e
  `read(s, n)` (chunk). Monomorphizador resolve por nome+aridade, igual
  a File.
- **`listen!` opera sobre listener.** Retorna um socket `Connected` (o
  do cliente que chegou). O listener continua `Listener` — pode aceitar
  múltiplas conexões.
- **Non-blocking:** todo socket conectado é non-blocking
  (`O_NONBLOCK`). O scheduler cooperativo faz `poll` no sleep path.

### 4.2. Distinção Listener vs Connected no runtime

O runtime valida o modo antes de cada operação:

| Operação | `Listener` | `Connected` |
|---|---|---|
| `listen!` (aceitar) | ✅ | ❌ `Err("socket conectado não aceita conexões")` |
| `read!` | ❌ `Err("socket listener não suporta read")` | ✅ |
| `write!` | ❌ `Err("socket listener não suporta write")` | ✅ |

Isto é análogo a `FileInner` checar `IoMode` antes de `read`/`write`.

### 4.3. Exemplo de uso — servidor TCP

```kata
action servidor (addr::Text) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
    match result
      Result::Ok listener:
        loop
          let client := listen!(listener)
          match client
            Result::Ok conn:
              fork!(handle_client, (conn,))
            Result::Err msg: echo!(msg)
      Result::Err msg: echo!(msg)

action handle_client (conn::Socket) => Unit
    let data := read!(conn)
    match data
      Result::Ok bytes:
        let _ := write!(conn, bytes)
        close!(conn)
      Result::Err msg:
        echo!(msg)
        close!(conn)
```

### 4.4. Exemplo de uso — cliente TCP

```kata
action cliente (addr::Text) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "hello\n")
        let data := read!(sock)
        match data
          Result::Ok bytes: echo!(show(bytes))
          Result::Err msg: echo!(msg)
        close!(sock)
      Result::Err msg: echo!(msg)
```

### 4.5. Exemplo de uso — Unix domain socket

```kata
action servidor_unix (path::Text) => Unit
    let result := open!(SocketKind::Unix(path), SocketMode::Listener)
    match result
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn: echo!("conectado!", conn)
          Result::Err msg: echo!(msg)
      Result::Err msg: echo!(msg)
```

## 5. Runtime

### 5.1. `SocketInner` — socket aberto

```rust
use std::net::TcpListener;
use std::os::unix::net::UnixListener;

/// Estado do socket — determina quais operações são válidas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocketState {
    Listener,
    Connected,
}

/// SocketInner — socket aberto com estado, FD e endereço.
/// Alocado via `arena_alloc` na root_arena.
///
/// O descritor OS é armazenado como FD bruto (i32), não como
/// `TcpStream`/`UnixStream`. Isto permite:
/// - Extrair FD para `poll` uniformemente (igual FileInner)
/// - Non-blocking controlado via `fcntl(F_SETFL, O_NONBLOCK)`
/// - `read`/`write` via syscall direta (nãoBufReader — sockets não
///   têm a semântica de buffering de arquivo)
///
/// Para `Listener`, o FD é do listener socket (TcpListener/UnixListener).
/// Para `Connected`, o FD é do stream (TcpStream/UnixStream ou FD
/// retornado por `accept()`).
pub(crate) struct SocketInner {
    pub closed: bool,
    pub fd: i32,               // FD bruto — uniforme para poll
    pub state: SocketState,    // Listener ou Connected
    pub kind: SocketKindRust,   // TCP ou Unix (para close correto)
    pub addr: String,           // endereço bindado/conectado
}

pub(crate) enum SocketKindRust {
    Tcp,
    Unix,
}
```

### 5.2. Por que FD bruto em vez de `TcpStream`/`UnixListener`?

`FileInner` usa `BufReader<File>` — específico para arquivos. Sockets
não usam `BufReader` (buffering de socket tem semântica diferente de
arquivo — `read` em socket pode retornar menos bytes sem ser EOF).
Armazenar FD bruto permite:

1. **`poll` uniforme** — `try_select_sockets` extrai FD diretamente, sem
   match em `SocketKindRust`. Mesmo padrão de `try_select_files`.
2. **Non-blocking simples** — `fcntl(fd, F_SETFL, O_NONBLOCK)` uma vez
   na criação.
3. **`read`/`write` diretos** — `libc::read(fd, ...)` /
   `libc::write(fd, ...)`, sem wrapper Rust.
4. **Close uniforme** — `libc::close(fd)`, sem match em tipo de stream.

O trade-off é não ter as abstrações de `TcpListener::accept()` e
`TcpStream::connect()` do Rust std — usamos syscalls diretas via `libc`.
Mas isto é consistente com o resto do runtime (FFI usa `libc` para
`poll`, `fork`, etc).

### 5.3. FFI functions

```rust
kata_rt_socket_open(kind_box: i64, mode_box: i64) -> i64
// kind_box: Sum box SocketKind (tag + payload Text)
// mode_box: Sum box SocketMode (tag only)
// Retorna: Result box Ok(handle) ou Err(text)

kata_rt_socket_listen(listener_handle: i64) -> i64
// Retorna: Result box Ok(connected_handle) ou Err(text)
// accept(2) no FD do listener, cria novo SocketInner Connected

kata_rt_socket_read(handle: i64) -> i64
// Retorna: Result box Ok(bytes_ptr) ou Err(text)

kata_rt_socket_read_chunk(handle: i64, n: i64) -> i64
// Retorna: Result box Ok(bytes_ptr) ou Err(text)
// n é SMI-tagged (payload = n >> 1)

kata_rt_socket_write_text(handle: i64, data_ptr: i64) -> i64
// data_ptr é C string (Text)
// Retorna: Result box Ok(0) ou Err(text)

kata_rt_socket_write_bytes(handle: i64, data_ptr: i64) -> i64
// data_ptr é Bytes blob (len@0, data@8)
// Retorna: Result box Ok(0) ou Err(text)

kata_rt_socket_close(handle: i64) -> ()
// Idempotente via campo `closed`
```

Todas as alocações (Result boxes, Bytes, Text, SocketInner) usam
`kata_rt_arena_alloc` na root_arena. Sem header ARC, sem incref/decref.
Mesmo padrão de File.

### 5.4. `kata_rt_socket_open` — detalhes

```rust
pub unsafe extern "C" fn kata_rt_socket_open(kind_box: i64, mode_box: i64) -> i64 {
    // 1. Extrair kind: tag (0=TCP, 1=Unix) + payload (Text = C string)
    let kind_tag = sum_tag_int(kind_box);
    let addr_ptr = /* payload do Sum box (offset 8) */;
    let addr = CStr::from_ptr(addr_ptr).to_string_lossy().to_string();

    // 2. Extrair mode: tag (0=Listener, 1=Connected)
    let mode_tag = sum_tag_int(mode_box);

    match (kind_tag, mode_tag) {
        (0, 0) => create_tcp_listener(&addr),   // socket + bind + listen
        (0, 1) => create_tcp_connected(&addr),   // socket + connect
        (1, 0) => create_unix_listener(&addr),   // socket + bind + listen
        (1, 1) => create_unix_connected(&addr),  // socket + connect
        _ => alloc_result_box(1, error_text("kind/mode inválido")),
    }
}
```

Cada path cria o socket, configura non-blocking (fcntl O_NONBLOCK), e
aloca `SocketInner` na root_arena.

### 5.5. `kata_rt_socket_listen` — detalhes

```rust
pub unsafe extern "C" fn kata_rt_socket_listen(listener_handle: i64) -> i64 {
    let inner = file_from_handle_socket(listener_handle);

    // Validar que é Listener
    if inner.state != SocketState::Listener {
        return alloc_result_box(1, error_text("socket conectado não aceita conexões"));
    }

    // accept(2) — non-blocking
    let client_fd = libc::accept(inner.fd, ...);

    if client_fd < 0 {
        if errno == EAGAIN || errno == EWOULDBLOCK {
            // Sem conexão pendente — suspender fiber, scheduler poll
            suspend_with(YieldReason::WaitingOnSelect {
                channel_handles: vec![],
                file_handles: vec![listener_handle],
                deadline: None,
            });
            // Após resume: tentar novamente
            client_fd = libc::accept(inner.fd, ...);
        }
        if client_fd < 0 {
            return alloc_result_box(1, error_text("accept falhou"));
        }
    }

    // Configurar client_fd como non-blocking
    set_nonblocking(client_fd);

    // Alocar SocketInner Connected
    let client_inner = SocketInner {
        closed: false,
        fd: client_fd,
        state: SocketState::Connected,
        kind: inner.kind,
        addr: /* peer addr */,
    };
    let handle = alloc_socket_inner(client_inner);
    alloc_result_box(0, handle)
}
```

### 5.6. `read`/`read_chunk` — non-blocking com suspensão

```rust
pub unsafe extern "C" fn kata_rt_socket_read_chunk(handle: i64, n: i64) -> i64 {
    let inner = file_from_handle_socket(handle);

    // Validar que é Connected
    if inner.state != SocketState::Connected {
        return alloc_result_box(1, error_text("socket listener não suporta read"));
    }

    let max_bytes = (n >> 1) as usize;
    let mut buf = vec![0u8; max_bytes];

    loop {
        let n_read = libc::read(inner.fd, buf.as_mut_ptr(), max_bytes);
        if n_read >= 0 {
            // Sucesso — retorna bytes lidos
            buf.truncate(n_read as usize);
            return alloc_result_box(0, alloc_bytes(&buf));
        }
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
            // Sem dados — suspender fiber
            suspend_with(YieldReason::WaitingOnSelect {
                channel_handles: vec![],
                file_handles: vec![handle],
                deadline: None,
            });
            // Após resume: tentar novamente
            continue;
        }
        // Erro real
        return alloc_result_box(1, error_text(&format!("erro de leitura: {err}")));
    }
}
```

### 5.7. Close — idempotente

`kata_rt_socket_close` faz `libc::close(fd)` e marca `closed = true`.
Idempotente via campo `closed`, igual a `FileInner`. A memória do
`SocketInner` permanece na root_arena até o teardown.

### 5.8. Select para Socket handles

```rust
/// try_select_sockets — verifica quais socket handles têm dados para leitura.
/// Usa poll(POLLIN, timeout=0) — non-blocking.
/// Retorna o índice (0..N-1) do primeiro handle pronto, ou WOULD_BLOCK (-1).
pub(crate) fn try_select_sockets(handles: &[i64]) -> i64;

/// Coleta FDs brutos de socket handles — para scheduler sleep path.
pub(crate) fn collect_socket_fds(handles: &[i64]) -> Vec<libc::pollfd>;
```

Mesma estrutura de `try_select_files`/`collect_file_fds`. Extrai `fd`
de cada `SocketInner` (não `buf_reader.get_ref().as_raw_fd()` — FD direto).

### 5.9. Scheduler — extensão do poll unificado

O scheduler já faz poll unificado (IPC + file FDs) no sleep path. Adicionar
socket FDs é estender o array:

```rust
// Em scheduler sleep path:
let mut all_fds: Vec<libc::pollfd> = vec![];
all_fds.extend(collect_ipc_fds(&ipc_handles));
all_fds.extend(collect_file_fds(&file_handles));
all_fds.extend(collect_socket_fds(&socket_handles));  // NOVO
poll(all_fds, timeout);
```

`YieldReason::WaitingOnSelect` passa a carregar três arrays:

```rust
WaitingOnSelect {
    channel_handles: Vec<i64>,
    file_handles: Vec<i64>,
    socket_handles: Vec<i64>,   // NOVO
    deadline: Option<Instant>,
}
```

## 6. Decisões de memória

### 6.1. Onde os Result boxes, Bytes, Text, SocketInner vivem?

**Decisão: root_arena via `arena_alloc`.** Mesmo padrão de File. Sem
header ARC, sem incref/decref. A memória é liberada no fim do processo.

### 6.2. Handle — ponteiro puro

**Decisão: handle = ponteiro puro para SocketInner.** Sem tag scheme,
igual a File. O type system já garante que só `Ty::Socket` chega às FFIs.

### 6.3. Non-blocking obrigatório

**Decisão: todo socket é non-blocking.** `fcntl(fd, F_SETFL, O_NONBLOCK)`
aplicado na criação (open, accept, connect). O scheduler cooperativo gerencia
o bloqueio via `poll` + suspensão de fiber.

### 6.4. Não generalizar FileInner → IoInner

**Decisão: structs separadas.** `FileInner` e `SocketInner` são diferentes
demais (BufReader vs FD bruto, FileMode vs SocketState, path vs addr).
A generalização criaria um enum com fields redundantes ou uma struct com
campos opcionais — nenhum dos dois é mais limpo.

O que é compartilhado (FD para poll, close idempotente, modo) é gerenciado
por convenção, não por herança. O codegen separa handles por tipo em
compile-time — o runtime nunca precisa distinguir tipos dinamicamente.

### 6.5. `write` com Bytes (null bytes) — FFI separada

**Decisão: FFI separada.** `kata_rt_socket_write_text` trata o dado como
C string (para no null byte). `kata_rt_socket_write_bytes` lê o header
de len (i64 no offset 0) e os dados no offset 8 — suporta null bytes.
Mesma convenção de File.

### 6.6. `read` slurp + streaming

**Decisão: ambas as APIs.** `read(s)` lê tudo disponível (slurp).
`read(s, n)` lê até n bytes (chunk). Ambas suspendem o fiber se não há
dados (EAGAIN) e retomam via scheduler poll.

### 6.7. Close automático no epílogo

**Decisão: epílogo fecha handles não fechados.** O codegen rastreia
sockets abertos em `io_handle_vars` (generalização de `file_handle_vars`
para incluir `Ty::Socket`). O epílogo da action chama
`kata_rt_socket_close` em cada um. O close é idempotente — se o
programador chamou `close!` explicitamente, o epílogo é no-op.

## 7. Codegen

### 7.1. `Ty::Socket` no sistema de tipos

Pontos de mudança (paralelos a `Ty::File`):

- `ty.rs`: adicionar `Socket` no enum `Ty`
- `ty_to_clif`: `Ty::Socket => I64` (ponteiro)
- `TypeShape`: `Ty::Socket => TypeShape::Prim` (escalar)
- `Display`/`type_name_str`: `Ty::Socket => "Socket"`
- `type_resolve.rs`: `"Socket" => Ty::Socket` (dois locais, igual a `"File"`)
- `contains_channel_type`: `Ty::Socket => false` (não é canal)
- `check_exhaustiveness`: `Ty::Socket` não é enum pattern (igual a File)
- `escape_arena.rs`: `Ty::Socket` não escapa arena (igual a File)

### 7.2. `FfiSymbol` — novas variantes

```rust
// Em ffi.rs:
pub enum FfiSymbol {
    // ... existentes ...
    // Socket I/O
    SocketOpen,
    SocketListen,
    SocketRead,
    SocketReadChunk,
    SocketWriteText,
    SocketWriteBytes,
    SocketClose,
}
```

Assinaturas Cranelift em `ffi_sigs.rs`, registros em `ffi_registry.rs`.

### 7.3. `io_handle_vars` — generalização de `file_handle_vars`

```rust
// Em LowerCtx:
pub io_handle_vars: Vec<cranelift_frontend::Variable>,
```

Rastreia variáveis de `Ty::File` **e** `Ty::Socket`. O epílogo fecha
cada uma com a FFI correspondente (`kata_rt_file_close` ou
`kata_rt_socket_close`). O codegen precisa saber qual FFI chamar para
cada handle — armazena o tipo junto com a variável:

```rust
pub io_handle_vars: Vec<(cranelift_frontend::Variable, IoHandleKind)>,

pub enum IoHandleKind {
    File,
    Socket,
}
```

Onde `let` ou pattern binding recebe `Ty::File` → registra `(var, File)`.
Recebe `Ty::Socket` → registra `(var, Socket)`. O epílogo despacha:

```rust
for (var, kind) in &lower.io_handle_vars {
    match kind {
        IoHandleKind::File => emit kata_rt_file_close(var),
        IoHandleKind::Socket => emit kata_rt_socket_close(var),
    }
}
```

### 7.4. `select` com sockets — arrays separados (implementado)

O `lower_select` separa braços em `channel_arms`, `file_arms` e
`socket_arms` por tipo em compile-time. O codegen chama
`kata_rt_select_combined` com três conjuntos separados: channels, files,
sockets.

**Decisão final:** arrays separados, não array unificado. O PRD original
sugeria array unificado (FD é FD para `poll(POLLIN)`), mas a implementação
revelou que `try_select_files` faz cast para `FileInner` — usar o mesmo
array para sockets produziria cast sobre `SocketInner` (layout de memória
diferente → FD lixo). A solução foi separar `file_arms`/`socket_arms` no
codegen e estender `kata_rt_select_combined` com `socket_ptr` + `n_s`
params (7 args em vez de 5). O runtime chama `try_select_sockets` para
socket handles.

### 7.5. `infer_select` — extensão

O typecheck do `IoRead` arm hoje valida que `handle_expr` é `Ty::File`.
Extensão:

```rust
// Em infer_select, validação do IoRead arm:
if handle_ty != Ty::File && handle_ty != Ty::Socket {
    return Err("expected File or Socket in select I/O arm");
}
```

O binding continua recebendo `Result::(Bytes, Text)` — mesmo tipo para
files e sockets.

## 8. Interação com `spawn!` e canais IPC

### 8.1. Sockets vs canais IPC

| Característica | Canal IPC (`spawn!` + canal) | Socket TCP/Unix |
|---|---|---|
| Escopo | Parent-child (fork) | Qualquer processo na máquina (Unix) ou rede (TCP) |
| Serialização | Automática (marshal via type table) | Manual (Bytes/Text) |
| Tipagem | Tipada (`Channel(T)`) | Não-tipada (`Socket` + Bytes/Text) |
| Bidirecional | Dois canais (Sender + Receiver) | Full-duplex (um socket) |

Sockets e canais IPC são complementares, não concorrentes. Canais IPC
são tipados e automáticos para parent-child. Sockets são para comunicação
com processos arbitrários e rede.

### 8.2. Pipes — adiado

Pipes anônimos (`pipe()`) e pipes de processo (`spawn_process!` com
stdin/stdout capturados) **não estão no escopo deste PRD**. A linguagem
já tem `spawn!` + canais IPC para comunicação parent-child tipada.
Pipes seriam uma extensão de baixo nível que não compensa o custo no
momento. Serão revisitados quando houver caso de uso real.

## 9. Fases de implementação

### Fase 1: Tipo e runtime base — ✅ Concluído

- `Ty::Socket` no enum, `type_name_str`, `ty_to_clif` (→ I64),
  `to_shape` (→ Prim), `Display` ✅
- `socket.rs`: `SocketInner`, `SocketState`, `SocketKindRust`, 7 FFI functions ✅
- Result boxes construídos via `arena_alloc` na root_arena ✅
- `kata_rt_socket_open`: TCP/Unix × Listener/Connected (4 paths) ✅
- `kata_rt_socket_listen`: accept com suspensão cooperativa ✅
- `kata_rt_socket_read`/`read_chunk`: non-blocking com suspensão ✅
- `kata_rt_socket_write_text`/`write_bytes`: non-blocking com suspensão ✅
- `kata_rt_socket_close`: idempotente via campo `closed` ✅
- `try_select_sockets`/`collect_socket_fds` ✅
- `SO_REUSEADDR` hardcoded em listeners TCP ✅

### Fase 2: Codegen e prelude — ✅ Concluído

- `FfiSymbol::SocketOpen/SocketListen/SocketRead/SocketReadChunk/SocketWriteText/SocketWriteBytes/SocketClose` em `ffi.rs` ✅
- Assinaturas Cranelift em `ffi_sigs.rs` ✅
- Registros em `ffi_registry.rs` ✅
- `Ty::Socket` em `type_table.rs`, `escape_arena.rs` ✅
- `"Socket" => Ty::Socket` em `type_resolve.rs` ✅
- `Ty::Socket` em `contains_channel_type` (false), `check_exhaustiveness` ✅
- `enum SocketKind` + `enum SocketMode` + 8 actions em `stdlib/core.kata` ✅
- `io_handle_vars` generalizado para `(Variable, IoHandleKind)` em `LowerCtx` ✅
- Epílogo despacha close por `IoHandleKind` ✅

### Fase 3: Scheduler cooperativo — ✅ Concluído

- `YieldReason::WaitingOnSelect` com `socket_handles: Vec<i64>` ✅
- `kata_rt_socket_listen` suspende fiber com `WaitingOnSelect` ✅
- `kata_rt_socket_read_chunk` suspende fiber com `WaitingOnSelect` ✅
- Scheduler wake_pass: `poll(POLLIN, timeout=0)` em socket FDs ✅
- Scheduler sleep path: poll unificado (IPC + files + sockets) ✅

### Fase 4: `select` com sockets — ✅ Concluído

- `infer_select`: `IoRead` arm aceita `Ty::File` ou `Ty::Socket` ✅
- `lower_select`: separa `file_arms` e `socket_arms` por tipo em compile-time ✅
- `kata_rt_select_combined`: estendida com `socket_ptr` + `n_s` params (7 args) ✅
- Runtime chama `try_select_sockets` para socket handles ✅

### Fase 5: Testes E2E — ✅ Concluído

10 testes E2E em `crates/kata-codegen/tests/socket_io_e2e.rs`:

- `socket_tcp_listen_connect_roundtrip` ✅
- `socket_tcp_echo_server` ✅
- `socket_read_chunk_streaming` ✅
- `socket_close_epilogo` ✅
- `socket_unix_listen_connect_roundtrip` ✅
- `socket_listener_read_fails` ✅
- `socket_open_invalid_addr_fails` ✅
- `socket_connect_refused_fails` ✅
- `socket_connected_listen_fails` ✅ (corrigido — main é servidor, fork! cliente, sem canal)
- `socket_select_with_socket` ✅ (NOVO)
- `socket_select_misto_channel_socket` ✅ (NOVO)

Total: 1372 passed, 0 failed, 5 ignored.

### Fase 6: Documentação — ✅ Concluído

- `docs/PRD-socket-io.md` — status e fases atualizados ✅
- `docs/ROADMAP.md` — feature Socket I/O adicionada ✅
- `docs/PRD-select-io.md` — seção 7.2 atualizada ✅
- `docs/Kata-lang-manual.md` — seção Socket I/O (solicitar permissão)
- `docs/sintaxe-mapa.md` — entradas de socket (solicitar permissão)

## 10. Decisões fechadas

| Decisão | Valor | Justificativa |
|---|---|---|
| `Ty::Socket` opaca | Sem `Box<Ty>`, sem fields expostos | Handle é implementação interna |
| `SocketKind` enum | `TCP(Text)` + `Unix(Text)` | Payload carrega o endereço |
| `SocketMode` enum | `Listener` + `Connected` | Descreve o estado resultante |
| `open!(kind, mode)` | Única action de criação | Despacha por kind × mode (4 paths) |
| `listen!(listener)` | Aceita conexão | Retorna socket Connected do cliente |
| Listener não transporta dados | Só `listen!` é válido | Listener é passivo |
| Connected é full-duplex | `read!` + `write!` | TCP/Unix stream são bidirecionais |
| Non-blocking obrigatório | `fcntl O_NONBLOCK` na criação | Scheduler cooperativo via poll |
| `SocketInner` separada | Não generaliza `FileInner` | Structs diferentes demais |
| FD bruto em `SocketInner` | `fd: i32` direto | poll uniforme, sem BufReader |
| API | open, listen, read×2, write×2, close, echo | Paralela a File |
| `Result::(T, Text)` | Tudo que falha retorna Result | Consistente com File, INDEXABLE |
| `@ffi` para builtins | open, listen, read, write, close | FFIs diretas no runtime |
| `echo` como action Kata | Não é `@ffi` | Compõe `write!` — sem FFI nova |
| `close` retorna `Unit` | Não falha | `close` de socket inválido é no-op |
| Result box: arena bump | root_arena via `arena_alloc` | Sem overhead de ARC |
| Handle: ponteiro puro | Sem tag scheme | Type system valida |
| write: FFI separada | `write_text` + `write_bytes` | Text para no null; Bytes tem len |
| `read` + `read_chunk` | Ambas APIs | Conveniência + streaming |
| Close no epílogo | `io_handle_vars` generalizado | FD leak se programador esquece close |
| Close idempotente | Campo `closed` em `SocketInner` | Double-close = no-op |
| `select` com sockets | Arrays separados (file_arms/socket_arms) | `try_select_files` cast para `FileInner` — array unificado produziria FD lixo |
| Pipes | **Adiado** | `spawn!` + canais IPC cobrem o caso |

## 11. Pendências

| Item | Status | Nota |
|---|---|---|
| `Ty::Socket` + `SocketInner` + 7 FFIs | ✅ Implementado | Commit `fa9b33e`..`d10b338` |
| `SocketKind`/`SocketMode` enums + 8 actions no prelude | ✅ Implementado | Commit `72d496b` |
| `io_handle_vars` generalizado | ✅ Implementado | Commit `6784b24` |
| Scheduler com socket FDs no poll unificado | ✅ Implementado | Commit `d10b338` |
| `select` com sockets (arrays separados) | ✅ Implementado | Commit `14a1d46` |
| `socket_connected_listen_fails` | ✅ Corrigido | Teste reescrito sem `channel!()` — main é o servidor, `fork!` do cliente, `listen!(conn)` retorna Err diretamente. Commit `5fbe32f` |
| `write` em `select` (POLLOUT) | ✅ Verificado — não é bug | Scheduler já usa `POLLIN \| POLLOUT` em `collect_socket_fds`, `try_select_sockets` e `wake_pass`. Comentário stale corrigido. Commit `5fbe32f` |
| `SO_REUSEADDR` | ✅ Hardcoded em listeners TCP | Decisão 12.2 fechada |
| `accept` em `select` | Adiado | Médio — novo tipo de braço no `select` |
| Pipes anônimos | Adiado | `spawn!` + canais IPC cobrem o caso |
| `spawn_process!` com pipes | Adiado | — |
| `shutdown!` (fechar uma direção do socket) | Adiado | — |
| `setsockopt` como API explícita | Adiado | Hardcoded `on` por padrão |

## 12. Decisões em aberto

### 12.1. `accept` em `select` (futuro)

Hoje `listen!(listener)` é bloqueante cooperativo (suspende fiber,
scheduler poll). No futuro, `accept(listener) <! conn: body` dentro de
`select` permitiria um servidor esperar conexões e dados de sockets
existentes simultaneamente. Segue o padrão previsto no PRD-select-io
seção 7.1. Adiado — `fork!` por conexão é suficiente para começar.

### 12.2. `setsockopt` — SO_REUSEADDR — ✅ Fechado

**Decisão:** hardcoded `on` na criação do listener TCP. Implementado em
`set_reuseaddr(fd)` chamado em `create_tcp_listener`. Avaliar API
explícita no futuro se houver caso de uso.

### 12.3. EOF em sockets — ✅ Fechado

`read` em socket retorna 0 bytes quando o peer fecha a conexão
(EOF). **Decisão:** tratar como `Err("EOF")` (consistente com File).
Implementado no runtime.

### 12.4. IPv6

`SocketKind::TCP(Text)` aceita `"host:port"`. Para IPv6, o formato é
`"[::1]:8080"`. O runtime faz parse do endereço — `std::net::SocketAddr`
aceita ambos. Sem decisão de design extra — IPv6 é automático se o parse
funcionar. Testar na implementação.