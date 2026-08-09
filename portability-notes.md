# Portabilidade do binário `kata` — Windows e Apple (macOS)

Análise baseada na inspeção do código-fonte em 2026-08-09.

## 1. Codegen (Cranelift) — ✅ portável (com ressalva)

O Cranelift suporta x86_64 e aarch64 nativamente, e gera código para macOS
(Mach-O) e Windows (PE) sem problemas. O `cranelift_native::builder()` detecta
a plataforma automaticamente. O AOT test em `aot_emit_e2e.rs` (linha 92) já
reconhece `("macos", "x86_64")` e `("macos", "aarch64")`.

### Ressalva: CallConv hardcoded

Todo o codegen usa `CallConv::SystemV` hardcoded para FFI e `CallConv::Tail`
para funções Kata. No Windows, a calling convention nativa é
`CallConv::WindowsFastcall`, não `SystemV`. O `CallConv::Tail` é suportado pelo
Cranelift em ambas as arquiteturas, mas misturar `SystemV` com código Windows
nativo vai corromper a ABI.

Isso é um refactor não-trivial — precisa de um `#[cfg]` para selecionar a
calling convention correta, ou usar `CallConv::SystemV` (que o Cranelift
mapeia para a ABI da plataforma host) em vez de hardcoded.

Arquivos afetados (todas as ocorrências de `CallConv::SystemV`):
- `crates/kata-codegen/src/ffi_sigs/file_io.rs`
- `crates/kata-codegen/src/ffi_sigs/arithmetic.rs`
- `crates/kata-codegen/src/ffi_sigs/bytes.rs`
- `crates/kata-codegen/src/ffi_registry.rs`
- `crates/kata-codegen/src/ffi_sigs/channels.rs`
- `crates/kata-codegen/src/ffi_sigs/comptime.rs`
- `crates/kata-codegen/src/ffi_sigs/collections.rs`
- `crates/kata-codegen/src/ffi_sigs/io.rs`
- `crates/kata-codegen/src/ffi_sigs/scheduler.rs`
- `crates/kata-codegen/src/ffi_sigs/arena.rs`
- `crates/kata-codegen/src/lowering/module.rs`
- `crates/kata-codegen/src/lowering/test_runner.rs`
- `crates/kata-driver/src/main.rs`

## 2. Runtime (kata-rt) — ⚠️ parcialmente portável

### Fibers (wasmtime-fiber)

`wasmtime-fiber` suporta macOS (x86_64 e aarch64) e Windows (x86_64). Isso
deveria funcionar sem mudanças.

### libc direto

O runtime usa chamadas libc diretas:

- `libc::fork()` — `crates/kata-rt/src/ipc.rs:80`
- `libc::sigaction` / `SIGCHLD` / `SIGPIPE` — `crates/kata-rt/src/ipc.rs:45-48`
- `libc::poll()` / `libc::pollfd` — `crates/kata-rt/src/file/select.rs`,
  `crates/kata-rt/src/scheduler.rs:300-380`, `crates/kata-rt/src/channel/ipc.rs`
- `libc::fcntl` / `libc::setsockopt` — `crates/kata-rt/src/socket/mod.rs:167-183`
- `std::os::unix::io::AsRawFd` — `crates/kata-rt/src/file/select.rs:13`
- `std::os::unix::net::UnixListener` / `UnixStream` —
  `crates/kata-rt/src/socket/create.rs:14,163-204`

### macOS

Tudo funciona — macOS é UNIX (POSIX), `fork`/`poll`/`sigaction`/Unix domain
sockets são nativos.

### Windows

- `fork` não existe
- `poll` é diferente (WinSock `WSAPoll`)
- `sigaction` não existe
- Unix domain sockets existem desde Windows 10 1803 mas a API é diferente
- `AsRawFd` retorna um handle Windows, não um fd POSIX

## 3. AOT linker — ❌ Linux-only

O `aot.rs` invoca `cc`/`gcc`/`clang` com flags Unix (`-lkata_rt`, `-lpthread`,
`-Wl,-rpath`). No Windows, o linker é `link.exe` (MSVC) ou `lld-link`, as flags
são diferentes, e `lpthread` não existe. No macOS, o `cc` é `clang` e as flags
são quase compatíveis, mas `-Wl,-rpath` usa sintaxe diferente.

Arquivo afetado: `crates/kata-driver/src/aot.rs` (função `link`, linha 126).

## Tabela resumo

| Componente                       | macOS (x86_64/aarch64) | Windows (x86_64)      |
|----------------------------------|------------------------|----------------------|
| Lexer/Parser/AST/Resolution      | ✅                     | ✅                   |
| Inference/Typechecker            | ✅                     | ✅                   |
| Cranelift JIT                    | ✅ (provável)          | ⚠️ (CallConv hardcoded) |
| Cranelift AOT emit               | ✅                     | ✅                   |
| AOT linker                       | ⚠️ (flags quase)       | ❌ (linker/flags Unix-only) |
| Fibers (wasmtime-fiber)          | ✅                     | ✅                   |
| I/O (file/socket TCP)            | ✅                     | ✅ (via std)         |
| Unix domain sockets              | ✅                     | ⚠️ (API diferente)   |
| `fork()`/`spawn!`                | ✅                     | ❌                   |
| `poll()`/`sigaction`/`fcntl`    | ✅                     | ❌                   |
| `select_files` (poll-based)      | ✅                     | ❌                   |

## Veredito

### macOS

O binário deve compilar e rodar com ajustes menores — principalmente
corrigir as flags do linker AOT e possivelmente ajustar `CallConv`. O runtime
POSIX é compatível.

### Windows

Não vai compilar sem um refactor significativo do `kata-rt`. O bloco inteiro
de IPC/spawn (`fork`, `sigaction`, `poll`, Unix sockets) precisa de uma
abstração de plataforma com backend Windows (`CreateProcess`, `WSAPoll`,
named pipes, etc). Isso é um esforço considerável, não um porting casual.