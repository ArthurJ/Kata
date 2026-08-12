# PRD — Portabilidade para Windows (x86_64)

**Data:** 2026-08-12
**Base:** `docs/portability-notes.md` (inspeção de 2026-08-09)
**Estado do código:** 1555 testes passando, C-series completo
**Dependência:** PRD-portability-mac.md (Mac é portado primeiro)

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

---

## Os 4 problemas

| # | Componente | Problema | Esforço |
|---|-----------|----------|---------|
| 1 | Codegen | `CallConv::SystemV` hardcoded — Windows usa `WindowsFastcall` | Médio |
| 2 | Runtime | 35 chamadas POSIX diretas (fork, poll, sigaction, Unix sockets) | Alto |
| 3 | AOT linker | `cc` com flags Unix (`-lpthread`, `-Wl,-rpath`) | Médio |
| 4 | Scheduler | `select_files` baseado em `poll()` — Windows usa `WSAPoll` ou IOCP | Médio |

---

## Fase 1 — CallConv no codegen

**Objetivo:** Selecionar a calling convention correta por plataforma.

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

**Objetivo:** Criar uma trait de plataforma no `kata-rt` com backends
POSIX e Windows.

### 2.1 Problema

35 chamadas POSIX diretas em `kata-rt`:

| API POSIX | Equivalente Windows | Arquivos |
|-----------|---------------------|----------|
| `libc::fork()` | `CreateProcessW()` | `ipc.rs:80` |
| `libc::sigaction` / `SIGCHLD` / `SIGPIPE` | Sem equivalente direto | `ipc.rs:45-48` |
| `libc::poll()` / `libc::pollfd` | `WSAPoll()` ou IOCP | `file/select.rs`, `scheduler.rs:300-380`, `channel/ipc.rs` |
| `libc::fcntl` / `libc::setsockopt` | `setsockopt` (Winsock) | `socket/mod.rs:167-183` |
| `std::os::unix::io::AsRawFd` | `std::os::windows::io::AsRawSocket` | `file/select.rs:13` |
| `std::os::unix::net::UnixListener` / `UnixStream` | Named pipes (`\\.\pipe\`) | `socket/create.rs:14,163-204` |

### 2.2 Arquitetura proposta

```
crates/kata-rt/src/
├── platform/
│   ├── mod.rs          — trait Platform + factory
│   ├── posix.rs        — backend Linux + macOS
│   └── windows.rs      — backend Windows
├── ipc.rs              — usa platform::spawn_child
├── file/select.rs      — usa platform::poll
├── scheduler.rs        — usa platform::poll
├── channel/ipc.rs      — usa platform::poll
├── socket/mod.rs       — usa platform::setsockopt
├── socket/create.rs    — usa platform::domain_socket
└── fiber.rs            — sem mudança (wasmtime-fiber é portável)
```

### 2.3 Trait Platform

```rust
// crates/kata-rt/src/platform/mod.rs

/// Abstração de plataforma para operações de I/O e IPC.
///
/// Linux e macOS usam o backend POSIX (fork, poll, sigaction, Unix sockets).
/// Windows usa o backend Win32 (CreateProcess, WSAPoll, named pipes).
pub trait Platform {
    /// Spawna um processo filho. Retorna um handle/fd para comunicação.
    fn spawn_child(
        &self,
        cmd: &str,
        args: &[&str],
    ) -> Result<ChildHandle, PlatformError>;

    /// Espera por I/O em múltiplos fds/sockets.
    /// Equivalente a poll() no POSIX, WSAPoll() no Windows.
    fn poll(
        &self,
        fds: &[PollFd],
        timeout_ms: i32,
    ) -> Result<Vec<PollEvent>, PlatformError>;

    /// Cria um socket de domínio Unix (POSIX) ou named pipe (Windows).
    fn create_domain_endpoint(
        &self,
        path: &str,
    ) -> Result<DomainEndpoint, PlatformError>;

    /// Configura opções de socket (nível SOL_SOCKET etc).
    fn set_socket_opt(
        &self,
        sock: RawSocket,
        level: i32,
        opt: i32,
        val: &[u8],
    ) -> Result<(), PlatformError>;
}

// Factory
pub fn current_platform() -> Box<dyn Platform> {
    #[cfg(unix)]
    { Box::new(posix::PosixPlatform) }
    #[cfg(windows)]
    { Box::new(windows::WinPlatform) }
}
```

### 2.4 Migração incremental

Não migrar tudo de uma vez. Ordem de prioridade:

1. **`spawn!` / IPC** (`ipc.rs`) — `fork` → `CreateProcess`
2. **`select_files`** (`file/select.rs`) — `poll` → `WSAPoll`
3. **Scheduler** (`scheduler.rs`) — `poll` → `WSAPoll`
4. **Channel IPC** (`channel/ipc.rs`) — `poll` → `WSAPoll`
5. **Socket** (`socket/`) — Unix sockets → named pipes
6. **Signals** (`ipc.rs`) — `sigaction` → ignorar ou structured exceptions

Cada item deve ser migrado independentemente e testado no Linux (regressão)
antes de prosseguir.

### 2.5 Signals

`SIGCHLD` e `SIGPIPE` não existem no Windows. O comportamento atual:

- `SIGPIPE` — ignorado para evitar crash ao escrever em pipe quebrado.
  No Windows, writes em pipes quebrados retornam erro (`ERROR_BROKEN_PIPE`).
  A lógica de tratamento muda mas o efeito é o mesmo.

- `SIGCHLD` — usado para reaproveitar processos filhos (`waitpid`).
  No Windows, `WaitForSingleObject` no handle do processo substitui.
  Sem signal handler necessário.

### 2.6 Verificação

```bash
# Cross-check
cargo check --target x86_64-pc-windows-gnu -p kata-rt
```

---

## Fase 3 — AOT linker para Windows

**Objetivo:** Linkar object files Cranelift em executáveis Windows (PE).

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

### 4.2 Opções

| Abordagem | Prós | Contras |
|-----------|------|---------|
| `WSAPoll` | API similar ao poll, migração fácil | Só sockets — files precisam de outro mecanismo |
| IOCP | Mais performático, escala melhor | API complexa, refactor grande |
| Threads bloqueantes | Simples de implementar | Perde não-bloqueância, não escala |

### 4.3 Recomendação

**`WSAPoll` para sockets + threads para files.** O scheduler atual
já distingue entre tipos de I/O (channels via sockets, files via fds).
No Windows:

- **Channels (sockets/named pipes)** → `WSAPoll`
- **Files** → thread bloqueante por operação (ou overlapped I/O)

Isso preserva o modelo de concorrência para o caso principal (channels)
e degraga graciosamente para I/O de arquivos.

### 4.4 Implementação

A abstração `Platform::poll` (Fase 2) encapsula isso. O scheduler chama
`platform.poll()` que, no Windows, usa `WSAPoll` para sockets e fallback
para threads em files.

---

## Fase 5 — Cross-check no Linux

**Objetivo:** Confirmar que o código compila para Windows sem sair do Linux.

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

- **`spawn!` / IPC** — fork não existe no Windows
- **`select_files`** — poll não funciona com files no Windows
- **Channel IPC** — Unix domain sockets não existem no Windows
- **Socket create** — UnixListener não existe no Windows
- **AOT build** — linker flags incompatíveis

Estes testes devem ser `#[cfg(unix)]` ou marcados como `#[ignore]`
no Windows até a Fase 2 ser concluída.

---

## Fase 7 — CI multi-plataforma

**Objetivo:** Garantir que futuras mudanças não quebram portabilidade.

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
Fase 1 (CallConv) ────────── Fase 5 (cross-check)
         │                          │
         ▼                          ▼
Fase 2 (platform trait) ──── Fase 6 (testes Windows)
         │                          │
         ▼                          ▼
Fase 3 (AOT linker) ──────── Fase 7 (CI)
         │
         ▼
Fase 4 (scheduler)
```

Fases 1, 2 e 3 podem ser feitas em paralelo (são independentes).
Fase 4 depende de Fase 2 (usa a trait Platform).
Fase 5 depende de 1, 2, 3 (precisa compilar).
Fase 6 depende de 5 (precisa compilar para testar).
Fase 7 depende de 6 (CI só vale a pena quando os testes passam).

---

## Estimativa de esforço

| Fase | Esforço | Risco |
|------|---------|-------|
| 1 — CallConv | 2-3h | Baixo — refactor mecânico |
| 2 — Platform trait | 1-2 dias | Alto — refactor arquitetural |
| 3 — AOT linker | 3-4h | Médio — flags e toolchain |
| 4 — Scheduler | 4-6h | Médio — WSAPoll vs IOCP |
| 5 — Cross-check | 1h | Baixo |
| 6 — Testes Windows | 2-4h | Médio — depende de 1-4 |
| 7 — CI | 2h | Baixo |

**Total estimado:** 2-3 dias de trabalho focado.

---

## Decisões adiadas

1. **Windows aarch64** — fora do escopo. Reavaliar após Windows x86_64.
2. **IOCP** — mais performático que WSAPoll mas refactor grande. Adiar
   até que o port WSAPoll esteja estável e a performance seja um problema.
3. **MSVC vs MinGW** — PRD foca em MSVC. Suporte a MinGW pode ser
   adicionado depois se necessário.
4. **Named pipes vs TCP** — se channels IPC forem muito diferentes em
   Windows, pode ser mais simples usar TCP loopback em vez de named pipes.
   Decidir na Fase 2.