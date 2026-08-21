# PRD — Portabilidade para Windows (x86_64)

**Data:** 2026-08-12
**Base:** `docs/portability-notes.md` (inspeção de 2026-08-09)
**Estado do código:** 1583 testes passando, zero regressões
**Dependência:** PRD-portability-mac.md (Mac é portado primeiro) ✅
**Atualizado:** 2026-08-12 — Fases 1, 2, 3, 4, 5 e 7 completas. Binário PE32+ gerado. Apenas Fase 6 pendente.

---

## Contexto

O binário `kata` é desenvolvido em Linux (Arch, x86_64). Este PRD cobre
o port para Windows x86_64. Diferente do macOS (que é POSIX e provavelmente
compila sem mudanças), o Windows exige um refactor significativo do
runtime e ajustes no codegen.

### Premissa

O port para macOS (PRD-portability-mac.md) deve ser concluído primeiro.
Isso valida que o codegen e o pipeline estão corretos fora de Linux,
deixando o Windows focado nos problemas específicos de plataforma.

### Escopo

Windows x86_64 apenas. Windows aarch64 (ARM64) está fora do escopo —
o Cranelift suporta mas o esforço adicional não se justifica agora.

### Resultado real (2026-08-12)

Fases 1, 2, 3, 4, 5 e 7 completas. Os 12 crates compilam para
`x86_64-pc-windows-gnu` com zero erros. Binário PE32+ nativo (24MB)
gerado via `cargo build --target x86_64-pc-windows-gnu --release`.
CI multi-plataforma configurado (GitHub Actions).

**Fase 6 (testes em Windows real) pendente.** O único stub restante
é `spawn!` (decisão de design pendente — threads vs CreateProcessW).

---

## Os 4 problemas

| # | Componente | Problema | Esforço | Status |
|---|-----------|----------|---------|--------|
| 1 | Codegen | `CallConv::SystemV` hardcoded — Windows usa `WindowsFastcall` | Médio | ✅ Fase 1 |
| 2 | Runtime | 35 chamadas POSIX diretas (fork, poll, sigaction, Unix sockets) | Alto | ✅ Fase 2 (funções `#[cfg]`, sem trait) |
| 3 | AOT linker | `cc` com flags Unix (`-lpthread`, `-Wl,-rpath`) | Médio | ✅ Fase 3 |
| 4 | Scheduler | `select_files` baseado em `poll()` — Windows usa `WSAPoll` ou IOCP | Médio | ✅ Fase 4 |

---

## Fase 1 — CallConv no codegen

**Objetivo:** Selecionar a calling convention correta por plataforma.
**Status:** ✅ Completo (commit `78b2788`).

### 1.1 Problema

17 sites usam `CallConv::SystemV` hardcoded para FFI:

```
crates/kata-codegen/src/ffi_sigs/file_io.rs
crates/kata-codegen/src/ffi_sigs/arithmetic.rs
crates/kata-codegen/src/ffi_sigs/bytes.rs
crates/kata-codegen/src/ffi_registry.rs
crates/kata-codegen/src/ffi_sigs/channels.rs
crates/kata-codegen/src/ffi_sigs/comptime.rs
crates/kata-codegen/src/ffi_sigs/collections.rs
crates/kata-codegen/src/ffi_sigs/io.rs
crates/kata-codegen/src/ffi_sigs/scheduler.rs
crates/kata-codegen/src/ffi_sigs/arena.rs
crates/kata-codegen/src/lowering/module.rs
crates/kata-codegen/src/lowering/test_runner.rs
crates/kata-driver/src/main.rs
```

No Windows x86_64, a ABI nativa é `CallConv::WindowsFastcall`. Usar
`SystemV` corrompe a ABI — chamadas FFI vão crashar ou retornar lixo.

`CallConv::Tail` (usado em Actions e funções Kata) é suportado pelo
Cranelift em Windows x86_64, mas precisa de verificação empírica —
`return_call` em Windows pode ter restrições.

### 1.2 Solução

Criar uma função helper que seleciona a calling convention de FFI:

```rust
/// crates/kata-codegen/src/call_conv.rs
use cranelift_codegen::ir::CallConv;

/// Returns the native FFI calling convention for the target platform.
pub fn ffi_call_conv() -> CallConv {
    #[cfg(target_os = "windows")]
    { CallConv::WindowsFastcall }
    #[cfg(not(target_os = "windows"))]
    { CallConv::SystemV }
}
```

Substituir os 17 sites:

```rust
// Antes:
let mut sig = Signature::new(CallConv::SystemV);

// Depois:
let mut sig = Signature::new(ffi_call_conv());
```

### 1.3 CallConv::Tail

`CallConv::Tail` é usado em 8 sites (Actions, funções Kata, broker CSP).
O Cranelift suporta `Tail` em Windows x86_64, mas o `return_call` pode
ter limitações. Verificar empiricamente na Fase 6.

Se `Tail` falhar em Windows, fallback: usar `WindowsFastcall` para
funções Kata também (perde tail-call optimization mas funciona).

### 1.4 Verificação

```bash
# Cross-check (no Linux, se rustup instalado)
rustup target add x86_64-pc-windows-gnu
cargo check --target x86_64-pc-windows-gnu -p kata-codegen
```

Alternativa: `x86_64-pc-windows-msvc` (toolchain MSVC). O target GNU
é mais fácil de cross-checkar no Linux.

---

## Fase 2 — Abstração de plataforma no runtime

**Objetivo:** Eliminar chamadas POSIX diretas em `kata-rt`.
**Status:** ✅ Completo (commit `b23e7c6`).

### 2.1 Problema

35 chamadas POSIX diretas em `kata-rt`:

| API POSIX | Equivalente Windows | Arquivos |
|-----------|---------------------|----------|
| `libc::fork()` | `CreateProcessW()` | `ipc.rs:80` |
| `libc::sigaction` / `SIGCHLD` / `SIGPIPE` | Sem equivalente direto | `ipc.rs:45-48` |
| `libc::poll()` / `libc::pollfd` | `WSAPoll()` ou IOCP | `file/select.rs`, `scheduler.rs:300-380`, `channel/ipc.rs` |
| `libc::fcntl` / `libc::setsockopt` | `setsockopt` (Winsock) | `socket/mod.rs:167-183` |
| `std::os::unix::io::AsRawFd` | `std::os::windows::io::AsRawSocket` | `file/select.rs:13` |
| `std::os::unix::net::UnixListener` / `UnixStream` | TCP localhost (sem named pipes) | `socket/create.rs:14,163-204` |

### 2.2 Solução adotada — funções `#[cfg]`, sem trait

**Decisão do Arthur:** não usar trait `Platform`. O overhead de vtable
e a tentação de adicionar backends futuros não se justificam. Em vez
disso, `platform.rs` tem funções com `#[cfg(unix)]`/`#[cfg(windows)]`:

```
crates/kata-rt/src/
├── platform.rs          — funções #[cfg] + bindings Win32 (winsock, win32)
├── ipc.rs               — #[cfg(unix)] fork, #[cfg(windows)] stub
├── file.rs              — stdio: #[cfg(unix)] from_raw_fd, #[cfg(windows)] GetStdHandle
├── file/select.rs       — poll: platform::poll_fds (unificado)
├── channel/ipc.rs       — pipe: #[cfg(unix)] libc::pipe, #[cfg(windows)] socketpair TCP
├── socket/create.rs     — TCP: Winsock real. Unix: #[cfg(unix)] real, #[cfg(windows)] TCP localhost
├── socket/io.rs         — read/write: platform::raw_read/raw_write
├── socket/select.rs     — poll: platform::poll_fds
├── scheduler.rs         — poll: platform::poll_fds
└── fiber.rs             — sem mudança (wasmtime-fiber é portável)
```

Funções em `platform.rs`:
- `set_nonblocking(fd)` — `fcntl(O_NONBLOCK)` / `ioctlsocket(FIONBIO)`
- `close_fd(fd)` — `libc::close` / `closesocket`
- `raw_read(fd, buf, len)` — `libc::read` / `winsock::recv`
- `raw_write(fd, buf, len)` — `libc::write` / `winsock::send`
- `poll_fds(fds, timeout)` — `libc::poll` / `winsock::WSAPoll`
- `set_reuseaddr(fd)` — `setsockopt` / `winsock::setsockopt`
- `is_would_block(errno)` — `EAGAIN/EWOULDBLOCK` / `WSAEWOULDBLOCK`
- `tcp_listener_fd/into_fd`, `tcp_stream_fd/into_fd` — `as_raw_fd` / `as_raw_socket`
- `file_raw_fd(file)` — `as_raw_fd` / `as_raw_handle`
- `ensure_winsock_init()` — `WSAStartup` (Once)

Bindings diretos em `platform::winsock` (link `ws2_32`) e
`platform::win32` (link `kernel32`). Sem dependência `windows-sys`.

### 2.3 Signals

`SIGCHLD` e `SIGPIPE` não existem no Windows:

- `SIGPIPE` — ignorado no Unix para evitar crash. No Windows, writes em
  sockets quebrados retornam `WSAECONNRESET`. A lógica de tratamento muda
  mas o efeito é o mesmo (erro tratado, sem crash).
- `SIGCHLD` — usado para reaproveitar processos filhos (`waitpid`).
  No Windows, `WaitForSingleObject` no handle do processo substitui.
  Sem signal handler necessário. `spawn!` ainda é stub (ver Fase 4).

### 2.4 Verificação

```bash
cargo check --target x86_64-pc-windows-gnu -p kata-rt
cargo check -p kata-rt  # Linux — zero regressões
```

---

## Fase 3 — AOT linker para Windows

**Objetivo:** Linkar object files Cranelift em executáveis Windows (PE).
**Status:** ✅ Completo (commit `ef699d4`).

### 3.1 Problema

`crates/kata-driver/src/aot.rs` (função `link`, linha 86) usa:

```
cc -o <output> <shim.o> <cranelift.o> -L<lib_dir> -lkata_rt -lm -lpthread -Wl,-rpath,<dir>
```

No Windows:
- O linker é `link.exe` (MSVC) ou `lld-link` (LLVM)
- Flags são diferentes: `/OUT:` em vez de `-o`, `/LIBPATH:` em vez de `-L`
- `-lpthread` não existe (pthreads via Windows threads)
- `-Wl,-rpath` não existe (DLLs são buscadas no PATH ou no diretório do exe)
- `-lm` não existe (math é parte da CRT)

### 3.2 Solução

```rust
fn link(object_bytes: &[u8], output: &Path, dynamic: bool, type_tag: i32)
    -> Result<(), String>
{
    #[cfg(unix)]
    { link_unix(object_bytes, output, dynamic, type_tag) }
    #[cfg(windows)]
    { link_windows(object_bytes, output, dynamic, type_tag) }
}

#[cfg(unix)]
fn link_unix(/* ... */) -> Result<(), String> {
    // código atual, sem mudanças
}

#[cfg(windows)]
fn link_windows(/* ... */) -> Result<(), String> {
    let linker = find_windows_linker()?;

    // Gerar shim C (mesma lógica, mas com #include <windows.h> se necessário)
    // Compilar shim: cl /c /Fo<shim.o> <shim.c>  (MSVC)
    //               ou  clang -c -o <shim.o> <shim.c>  (LLVM)
    // Linkar: lld-link /OUT:<output> <shim.o> <cranelift.o> <libkata_rt.lib>
    //        kernel32.lib ws2_32.lib userenv.lib
}
```

### 3.3 Considerações

- **MSVC vs GNU** — dois toolchains possíveis no Windows:
  - `x86_64-pc-windows-msvc` (MSVC, padrão)
  - `x86_64-pc-windows-gnu` (MinGW, mais fácil de cross-compilar)
  Suportar os dois é possível mas o PRD foca em MSVC (mais comum).

- **Shim C** — o shim atual usa `__builtin_memcpy` (GCC/clang). MSVC
  não suporta `__builtin_*`. Usar `memcpy` direto com `#include <string.h>`.

- **Lib estática** — no Windows, a extensão é `.lib` (não `.a`).
  `libkata_rt.lib` em vez de `libkata_rt.a`.

- **Lib dinâmica** — `.dll` em vez de `.so`/`.dylib`.

### 3.4 Verificação

Compilar e linkar um programa trivial no Windows:
```bash
cargo run -p kata-driver -- build examples/soma.kata -o soma.exe
soma.exe
```

---

## Fase 4 — Scheduler no Windows

**Objetivo:** Garantir que o scheduler não-bloqueante funciona no Windows.

**Status:** ✅ Completo (exceto `spawn!` — stub intencional).

### 4.1 Problema

O scheduler usa `libc::poll()` em três lugares:

```
crates/kata-rt/src/file/select.rs
crates/kata-rt/src/scheduler.rs:300-380
crates/kata-rt/src/channel/ipc.rs
```

`poll()` no Windows é `WSAPoll()` — semelhante mas só funciona com
sockets (não com fds arbitrários como no POSIX). Files, pipes e
dispositivos não são sockets no Windows.

### 4.2 Solução adotada

**`WSAPoll` para sockets + `GetStdHandle` para stdio + TCP loopback
para pipe/Unix sockets.** Sem IOCP, sem threads bloqueantes.

| Componente | Solução Windows | Arquivo |
|------------|----------------|---------|
| poll | `WSAPoll` (via `platform::poll_fds`) | `platform.rs` |
| TCP sockets | Winsock2 real (`socket`/`bind`/`listen`/`connect`/`accept`) | `socket/create.rs` |
| stdin/stdout/stderr | `GetStdHandle` + `FromRawHandle` | `file.rs` |
| pipe IPC | socketpair TCP localhost (substitui `libc::pipe`) | `channel/ipc.rs` |
| Unix domain sockets | TCP localhost + arquivo de coordenação de porta | `socket/create.rs` |
| `spawn!` | **Stub** (no-op retorna 0) — decisão de design pendente | `ipc.rs` |

### 4.3 Implementação

A abstração `platform::*` (Fase 2) encapsula as diferenças. O scheduler
chama `platform::poll_fds()` que, no Windows, usa `WSAPoll`. Sockets
TCP e socketpair TCP são naturalmente compatíveis com `WSAPoll`.

**stdin/stdout/stderr** usam `GetStdHandle` (binding direto em
`platform::win32`) + `File::from_raw_handle`. O I/O de `File` no
Windows usa `ReadFile`/`WriteFile` (não `recv`/`send`), então é
compatível com `BufReader` sem mudanças no `FileInner`.

**pipe IPC** cria um par TCP conectado em loopback: server em
`127.0.0.1:0`, `getsockname` para descobrir a porta, `connect` +
`accept`, fecha listener. Ambos os sockets são non-blocking. Como os
FDs são sockets, `raw_read`/`raw_write` (que usam `recv`/`send` no
Windows) e `WSAPoll` funcionam nativamente.

**Unix domain sockets** usam TCP localhost com arquivo de coordenação:
o listener faz bind em porta efêmera, escreve a porta num arquivo no
mesmo path que o Unix usaria. O connected lê o arquivo, conecta na
porta. `SocketKindRust::Unix` é preservado no `SocketInner` para que o
`close` saiba que não deve fazer `unlink` do path (apenas `closesocket`).
O arquivo de coordenação é removido no início de `create_unix_listener`
(equivalente ao `remove_file` no Unix).

---

## Fase 5 — Cross-check no Linux

**Objetivo:** Confirmar que o código compila para Windows sem sair do Linux.
**Status:** ✅ Completo. 12/12 crates + binário PE32+ 24MB linkam.

### 5.1 Setup

```bash
rustup target add x86_64-pc-windows-gnu
# ou, se tiver MSVC toolchain disponível:
# rustup target add x86_64-pc-windows-msvc
```

### 5.2 Cross-check por crate

```bash
cargo check --target x86_64-pc-windows-gnu -p kata-lexer
cargo check --target x86_64-pc-windows-gnu -p kata-parser
cargo check --target x86_64-pc-windows-gnu -p kata-resolution
cargo check --target x86_64-pc-windows-gnu -p kata-inference
cargo check --target x86_64-pc-windows-gnu -p kata-codegen
cargo check --target x86_64-pc-windows-gnu -p kata-rt
cargo check --target x86_64-pc-windows-gnu -p kata-driver
cargo check --target x86_64-pc-windows-gnu -p kata-lsp
```

### 5.3 Critério de sucesso

Zero erros de compilação. Warnings são aceitáveis.

---

## Fase 6 — Testes no Windows

**Objetivo:** Rodar a suíte de testes no Windows.
**Status:** Pendente — requer máquina Windows real.

### 6.1 Setup no Windows

```bash
# Instalar Rust
# Instalar Visual Studio Build Tools (MSVC) ou MinGW
git clone <repo-url> && cd Kata5
cargo test --workspace --no-fail-fast -- --test-threads=8
```

### 6.2 Critério de sucesso

- **Verde:** todos os testes passam → port completo.
- **Amarelo:** < 20 falhas, todas em runtime (IPC, sockets, spawn) →
  esperado, corrigir com a abstração de plataforma.
- **Vermelho:** falhas em codegen/JIT/inference → problema arquitetural,
  investigar antes de continuar.

### 6.3 Testes que provavelmente falham

- **`spawn!` / IPC** — `spawn!` ainda é stub no Windows (no-op retorna 0).
  Testes que exercitam `spawn!` vão falhar. Todos os outros componentes
  (stdio, pipe IPC, sockets TCP, Unix sockets via TCP) estão implementados.
- **AOT build** — linker flags podem ter diferenças não cobertas pelo
  cross-check (só validável em Windows real).

Estes testes devem ser `#[cfg(unix)]` ou marcados como `#[ignore]`
no Windows até `spawn!` ser implementado.

---

## Fase 7 — CI multi-plataforma

**Objetivo:** Garantir que futuras mudanças não quebram portabilidade.
**Status:** ✅ Completo (commit `8702857`). 4 jobs: Linux, macOS x86+arm, cross-check Windows, Windows nativo (`continue-on-error`).

### 7.1 GitHub Actions matrix

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
    # macOS: x86_64 e aarch64 (macos-14 é arm64)
    include:
      - os: macos-13   # x86_64
      - os: macos-14   # aarch64
```

### 7.2 Cross-check no CI

Mesmo sem rodar testes no Windows, o cross-check (`cargo check --target`)
pode rodar no CI Linux:

```yaml
- name: Cross-check Windows
  run: |
    rustup target add x86_64-pc-windows-gnu
    cargo check --target x86_64-pc-windows-gnu -p kata-codegen
    cargo check --target x86_64-pc-windows-gnu -p kata-rt
    cargo check --target x86_64-pc-windows-gnu -p kata-driver
```

---

## Dependências entre fases

```
[PRD-portability-mac.md completo]
         │
         ▼
Fase 1 (CallConv) ────────── Fase 5 (cross-check) ✅
         │                          │
         ▼                          ▼
Fase 2 (platform #[cfg]) ─── Fase 6 (testes Windows) ⏳
         │                          │
         ▼                          ▼
Fase 3 (AOT linker) ──────── Fase 7 (CI) ✅
         │
         ▼
Fase 4 (scheduler/Winsock) ✅
```

Fases 1, 2 e 3 podem ser feitas em paralelo (são independentes).
Fase 4 depende de Fase 2 (usa a trait Platform).
Fase 5 depende de 1, 2, 3 (precisa compilar).
Fase 6 depende de 5 (precisa compilar para testar).
Fase 7 depende de 6 (CI só vale a pena quando os testes passam).

---

## Esforço real

| Fase | Esforço estimado | Esforço real | Status |
|------|-----------------|--------------|--------|
| 1 — CallConv | 2-3h | ~2h | ✅ |
| 2 — Platform `#[cfg]` | 1-2 dias | ~4h | ✅ |
| 3 — AOT linker | 3-4h | ~3h | ✅ |
| 4 — Scheduler/Winsock | 4-6h | ~5h (3 sessões) | ✅ |
| 5 — Cross-check | 1h | ~1h | ✅ |
| 6 — Testes Windows | 2-4h | — | ⏳ Pendente |
| 7 — CI | 2h | ~2h | ✅ |

**Total real (Fases 1-5,7):** ~17h em 4 sessões (2026-08-09 a 2026-08-12).

---

## Decisões adiadas

1. **Windows aarch64** — fora do escopo. Reavaliar após Windows x86_64.
2. **IOCP** — mais performático que WSAPoll mas refactor grande. Adiar
   até que o port WSAPoll esteja estável e a performance seja um problema.
3. **MSVC vs MinGW** — **Resolvido:** MinGW (`x86_64-pc-windows-gnu`)
   adotado. Mais fácil de cross-compilar no Linux (Arch). MinGW-w64
   16.1.0 instalado via pacman. MSVC não testado mas possível via
   `x86_64-pc-windows-msvc` se necessário.
4. **Named pipes vs TCP** — **Resolvido:** TCP loopback adotado para
   pipe IPC e Unix domain sockets. Named pipes não usadas — TCP
   localhost é mais simples e funcionalmente equivalente para IPC.
5. **`spawn!` no Windows** — **Pendente.** Stub atual (no-op retorna 0).
   Opções: (a) `CreateProcessW` com serialização de args; (b) threads
   com memória compartilhada; (c) processo filho via Win32 API com
   pipes/socketpair TCP para IPC. Decisão de design — não implementar
   até ter clareza sobre o modelo de concorrência desejado.