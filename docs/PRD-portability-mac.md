# PRD — Portabilidade para macOS (x86_64 e aarch64)

**Data:** 2026-08-12
**Base:** `docs/portability-notes.md` (inspeção de 2026-08-09)
**Estado do código:** 1555 testes passando, C-series completo

---

## Contexto

O binário `kata` é desenvolvido em Linux (Arch, x86_64). Este PRD cobre o
port para macOS — ambos x86_64 (Intel) e aarch64 (Apple Silicon). O port
para Windows é um esforço significativamente maior (ver seção "Futuro")
e está fora do escopo deste documento.

### Premissa

A análise estática (`docs/portability-notes.md` + inspeção atual do código)
indica que **o código provavelmente já compila em macOS sem mudanças**.
Este PRD trata de **verificar e corrigir** — não de reescrever.

---

## Fase 1 — Cross-compilation check (Linux → macOS)

**Objetivo:** Confirmar que o código compila para macOS sem sair do Linux.

### 1.1 Setup do toolchain

```bash
# Instalar rustup (Arch usa rust via pacman, sem rustup)
sudo pacman -S rustup
rustup default stable

# Adicionar targets macOS
rustup target add x86_64-apple-darwin aarch64-apple-darwin
```

### 1.2 Cross-check por crate

```bash
cargo check --target x86_64-apple-darwin -p kata-lexer
cargo check --target x86_64-apple-darwin -p kata-parser
cargo check --target x86_64-apple-darwin -p kata-resolution
cargo check --target x86_64-apple-darwin -p kata-inference
cargo check --target x86_64-apple-darwin -p kata-codegen
cargo check --target x86_64-apple-darwin -p kata-optimizer
cargo check --target x86_64-apple-darwin -p kata-monomorph
cargo check --target x86_64-apple-darwin -p kata-tree-shaking
cargo check --target x86_64-apple-darwin -p kata-comptime
cargo check --target x86_64-apple-darwin -p kata-rt
cargo check --target x86_64-apple-darwin -p kata-driver
cargo check --target x86_64-apple-darwin -p kata-lsp
```

Repetir para `aarch64-apple-darwin`.

### 1.3 O que esperar

| Crate | Esperado | Motivo |
|-------|----------|--------|
| kata-lexer | ✅ compila | Sem dependências de plataforma |
| kata-parser | ✅ compila | Puro Rust |
| kata-resolution | ✅ compila | Puro Rust |
| kata-inference | ✅ compila | Puro Rust |
| kata-codegen | ⚠️ verificar | `CallConv::SystemV` — ver Fase 2 |
| kata-optimizer | ✅ compila | Puro Rust |
| kata-monomorph | ✅ compila | Puro Rust |
| kata-tree-shaking | ✅ compila | Puro Rust |
| kata-comptime | ✅ compila | Puro Rust |
| kata-rt | ⚠️ verificar | libc POSIX — ver Fase 3 |
| kata-driver | ⚠️ verificar | AOT linker — ver Fase 4 |
| kata-lsp | ✅ compila | Puro Rust (tower-lsp) |

### 1.4 Critério de sucesso

Zero erros de compilação em todos os crates para ambos os targets.
Warnings são aceitáveis; erros devem ser catalogados para a fase apropriada.

---

## Fase 2 — CallConv no codegen

**Objetivo:** Garantir que `CallConv::SystemV` funciona em macOS.

### Análise

`CallConv::SystemV` é a calling convention nativa em macOS x86_64.
Em aarch64 macOS, o Cranelift mapeia `SystemV` → AAPCS automaticamente
via `cranelift_native::builder()` (já usado em `jit.rs:61` e `aot.rs:44`).

`CallConv::Tail` é suportado em ambas as arquiteturas macOS.

**Conclusão:** Os 17 sites com `CallConv::SystemV` hardcoded **não
precisam de mudança** para macOS. São todos FFI signatures, e FFI no
macOS usa SystemV.

### Ação

Nenhuma mudança de código esperada. Se o cross-check da Fase 1 passar
sem erros no `kata-codegen`, esta fase está completa.

### Verificação empírica (no Mac)

```bash
# No Mac, rodar um teste JIT simples:
cargo test -p kata-codegen --test jit_e2e -- --test-threads=1
```

Se algum teste JIT falhar com erro de segfault ou calling convention,
documentar qual teste e qual arquitetura.

---

## Fase 3 — Runtime POSIX (kata-rt)

**Objetivo:** Confirmar que o runtime compila e funciona em macOS.

### Análise

macOS é UNIX certificado (POSIX). As 35 chamadas libc diretas são todas
compatíveis:

| Chamada | macOS | Notas |
|---------|-------|-------|
| `libc::fork()` | ✅ | Nativo |
| `libc::sigaction` / `SIGCHLD` / `SIGPIPE` | ✅ | Nativo |
| `libc::poll()` / `libc::pollfd` | ✅ | Nativo |
| `libc::fcntl` / `libc::setsockopt` | ✅ | Nativo |
| `std::os::unix::io::AsRawFd` | ✅ | Nativo |
| `std::os::unix::net::UnixListener` / `UnixStream` | ✅ | Nativo |
| `wasmtime-fiber` | ✅ | Suporta macOS x86_64 e aarch64 |

### Possíveis problemas

1. **`fork()` no macOS** — funcional mas com diferenças subtis:
   - `fork()` no macOS não é `pthread_atfork`-safe por default.
   - Se o runtime usa threads + fork (não seemed fork-exec), pode haver
     deadlock. Verificar se `spawn!` faz fork-exec (provável) ou
     fork sem exec.

2. **`poll()` timeout units** — idêntico (ms), sem problema.

3. **`AsRawFd`** — no macOS, `RawFd` é `i32` (mesmo tipo que Linux).
   Sem problema.

### Ação

Nenhuma mudança de código esperada. Se o cross-check da Fase 1 passar
sem erros no `kata-rt`, esta fase está completa.

### Verificação empírica (no Mac)

```bash
cargo test -p kata-rt --tests --no-fail-fast
```

Falhas aqui indicam problemas de runtime (não de compilação).
Documentar cada falha.

---

## Fase 4 — AOT linker (kata-driver)

**Objetivo:** Garantir que o linker AOT funciona em macOS.

### Análise

O arquivo `crates/kata-driver/src/aot.rs` (função `link`, linha 86)
invoca `cc` com flags Unix. No macOS:

| Flag | Linux | macOS | Compatível? |
|------|-------|-------|-------------|
| `-o <output>` | ✅ | ✅ | Sim |
| `-c -o <shim.o>` | ✅ | ✅ | Sim |
| `-L<lib_dir>` | ✅ | ✅ | Sim |
| `-lkata_rt` | ✅ | ✅ | Sim (resolve para `.dylib` ou `.a`) |
| `-lm` | ✅ | ✅ | Sim (libSystem inclui libm) |
| `-lpthread` | ✅ | ⚠️ | macOS: `-lpthread` é aceito mas desnecessário (pthreads em libSystem) |
| `-Wl,-rpath,<dir>` | ✅ | ✅ | Sim (sintaxe idêntica) |
| `libkata_rt.a` (estático) | ✅ | ✅ | Sim |

### Possíveis problemas

1. **`-lpthread` no macOS** — aceito pelo clang como no-op, mas pode
   gerar warning. Se houver erro (não warning), remover condicionalmente:

   ```rust
   #[cfg(target_os = "macos")]
   { /* não adiciona -lpthread */ }
   #[cfg(not(target_os = "macos"))]
   { cmd.args(["-lm", "-lpthread"]); }
   ```

2. **Extensão da lib dinâmica** — o código usa `-lkata_rt` (não path
   direto), então o linker resolve automaticamente para `.dylib` no
   macOS. **Sem problema.**

3. **`__builtin_memcpy` no shim C** — compatível com clang (macOS).

4. **`find_linker()`** — procura `cc`, `gcc`, `clang`. No macOS, `cc`
   é `clang`. **Sem problema.**

### Ação

Se o cross-check passar, nenhuma mudança necessária. Se `-lpthread`
causar erro, aplicar o `#[cfg]` acima.

### Verificação empírica (no Mac)

```bash
# Compilar um programa simples via AOT
cargo run -p kata-driver -- build examples/soma.kata -o /tmp/soma_mac
/tmp/soma_mac
```

---

## Fase 5 — Suíte de testes no Mac

**Objetivo:** Rodar a suíte completa no Mac e catalogar falhas.

### 5.1 Setup no Mac

```bash
# Clonar o repo
git clone <repo-url> && cd Kata5

# Instalar dependências (se necessário)
# macOS não precisa de nada extra além de Rust + Xcode CLT

# Compilar e rodar testes
cargo test --workspace --no-fail-fast -- --test-threads=8
```

### 5.2 Critério de sucesso

- **Verde:** todos os 1555 testes passam → port completo, zero mudanças.
- **Amarelo:** < 20 falhas, todas em testes de runtime (não codegen) →
  port é viável com correções pontuais.
- **Vermelho:** > 20 falhas ou falhas em codegen/JIT → port precisa de
  investigação dedicada.

### 5.3 Categorização de falhas

Para cada falha, catalogar:

```
| Teste | Crate | Arquitetura | Tipo | Causa provável |
|-------|-------|-------------|------|----------------|
| ... | ... | x86_64 / aarch64 | compile / runtime / segfault | ... |
```

Tipos:
- **compile** — erro de compilação (deveria ter sido pego na Fase 1)
- **runtime** — panic, resultado incorreto, timeout
- **segfault** — crash nativo (calling convention, fiber, etc)
- **link** — erro de linking AOT

---

## Fase 6 — Correções (se necessário)

Esta fase só existe se a Fase 5 encontrar falhas. As correções dependem
do que for encontrado. As correções mais prováveis são:

### 6.1 `#[cfg(target_os)]` no AOT linker

Se `-lpthread` causar erro:

```rust
// aot.rs, função link()
if dynamic {
    cmd.arg(format!("-L{}", lib_dir.display()));
    cmd.arg("-lkata_rt");
    cmd.arg("-lm");
    #[cfg(not(target_os = "macos"))]
    cmd.arg("-lpthread");
    cmd.arg(format!("-Wl,-rpath,{}", lib_dir.display()));
} else {
    let static_lib = lib_dir.join("libkata_rt.a");
    cmd.arg(&static_lib);
    cmd.arg("-lm");
    #[cfg(not(target_os = "macos"))]
    cmd.arg("-lpthread");
}
```

### 6.2 Ajustes de runtime

Se `fork()` tiver problemas com threads no macOS, garantir que `spawn!`
sempre faz fork-exec (nunca fork sem exec).

### 6.3 Ajustes de Cranelift

Se `CallConv::Tail` falhar em aarch64 macOS (improvável mas possível),
investigar se o Cranelift precisa de flags específicas para Apple Silicon.

---

## Fase 7 — Documentação final

### 7.1 Atualizar `portability-notes.md`

Após o port ser validado, atualizar a tabela resumo:

```
| Componente | macOS x86_64 | macOS aarch64 |
|------------|-------------|---------------|
| (preencher com resultados reais) |
```

### 7.2 Atualizar README

Adicionar macOS à lista de plataformas suportadas, com instruções de
build.

---

## Futuro — Port para Windows

Coberto em `docs/PRD-portability-windows.md`. O port para Windows requer:

1. **Abstração de plataforma no `kata-rt`** — trait com backend POSIX
   (Linux + macOS) e backend Windows (`CreateProcess`, `WSAPoll`,
   named pipes).
2. **`CallConv::WindowsFastcall`** no codegen — `#[cfg]` para selecionar
   a calling convention correta (os 17 sites de `CallConv::SystemV`).
3. **AOT linker para Windows** — `link.exe` ou `lld-link` com flags
   COFF/PE (sem `-lpthread`, sem `-Wl,-rpath`).
4. **`select_files` no Windows** — `WSAPoll` ou IOCP em vez de `poll()`.

Estimativa: 2-3 dias de trabalho focado. Ver PRD dedicado.

---

## Dependências entre fases

```
Fase 1 (cross-check)
├── Fase 2 (CallConv) — só se 1 falhar no codegen
├── Fase 3 (runtime) — só se 1 falhar no kata-rt
├── Fase 4 (AOT linker) — só se 1 falhar no kata-driver
└── Fase 5 (testes no Mac) — independente de 1 (mas 1 dá confiança)
    └── Fase 6 (correções) — só se 5 encontrar falhas
        └── Fase 7 (docs) — após 6 (ou após 5 se verde)
```

## Nota sobre o SSH

O SSH do PC de desenvolvimento está com bug `hostkeys confused` no
OpenSSH 10.4 (Arch). Antes de testar no Mac do amigo, corrigir:

```bash
# Descomentar as 3 linhas HostKey em /etc/ssh/sshd_config
sudo sed -i 's/^#HostKey \/etc\/ssh\/ssh_host_/HostKey \/etc\/ssh\/ssh_host_/' /etc/ssh/sshd_config

# Corrigir AllowUsers (múltiplas linhas não acumulam — usar uma só)
sudo sed -i 's/^AllowUsers arthur@192.168.68.\*/AllowUsers arthur@192.168.68.* arthur@100.*/' /etc/ssh/sshd_config
sudo sed -i '/^AllowUsers arthur@100.\*/d' /etc/ssh/sshd_config

# Reiniciar sshd
sudo systemctl restart sshd
```

Alternativamente, atualizar o sistema (`sudo pacman -Syu`) pode trazer
uma versão do OpenSSH que corrige o bug `hostkeys confused`.