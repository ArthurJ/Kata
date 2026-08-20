# Apêndice — Plataformas e Limitações

Kata é desenvolvido em Linux (Arch, x86_64). Esta seção documenta o estado do port para outras plataformas — o que funciona, o que é experimental, e o que ainda não funciona.

## Linux (x86_64)

Plataforma primária de desenvolvimento. Todas as features estão implementadas e testadas.

## macOS (x86_64 e Apple Silicon)

Port completo e verificado. O binário compila nativamente para ambas as arquiteturas após uma correção de compatibilidade POSIX. Binários Mach-O foram gerados e testados em Mac real (Apple Silicon).

Build a partir do código-fonte:

```bash
git clone <repo-url> && cd Kata5
cargo build --release
```

O binário `kata` funciona em modo JIT e AOT. Todas as features — incluindo `fork!`, canais, `select`, e `spawn!` — estão disponíveis.

## Windows (x86_64)

Port em estágio experimental. O compilador gera um binário PE32+ nativo (24MB) via cross-compilation (`x86_64-pc-windows-gnu`), mas **nunca foi testado em Windows real**. A maioria das features está implementada, mas há limitações importantes.

### O que funciona (em teoria — não verificado em hardware)

- Compilação JIT e geração de código
- REPL interativo
- I/O de arquivo, stdin/stdout/stderr
- Sockets TCP
- Canais (`channel!`, `<!`, `!>`, `select`, `timeout`)
- `fork!` (fibers cooperativos)
- `sleep!`

### O que não funciona

**`spawn!`** — é um stub no Windows. A action `spawn!` existe e compila, mas em runtime é um no-op que retorna 0. Não há processo filho, não há comunicação. A decisão de design (usar `CreateProcessW`, threads, ou outro mecanismo) está pendente.

### O que funciona de forma diferente

**Unix domain sockets** — no Linux e macOS, sockets Unix usam o sistema de arquivos (`/tmp/socket`). No Windows, são substituídos por TCP localhost com um arquivo de coordenação de porta. Funcionalmente equivalente para IPC, mas a implementação é diferente.

**Signals** — `SIGPIPE` e `SIGCHLD` não existem no Windows. O tratamento de pipes quebrados e reaping de processos usa mecanismos Win32 diferentes internamente. O usuário não percebe a diferença — exceto que `spawn!` (que dependeria de signals) não funciona.

### O que não foi verificado

- **Testes em Windows real** — a suíte de testes completa nunca foi corrida em Windows. Pode haver bugs não-descobertos em runtime, codegen, ou linking AOT.
- **Tail calls** — a otimização de chamadas em cauda (`CallConv::Tail`) é suportada pelo gerador de código em teoria, mas não foi testada empiricamente no Windows.
- **Build AOT** — o linker Windows (`lld-link`) está configurado, mas nunca foi testado com um programa real no Windows.
- **Toolchain MSVC** — apenas o toolchain MinGW (`x86_64-pc-windows-gnu`) foi testado via cross-compilation. O toolchain MSVC (`x86_64-pc-windows-msvc`) deve funcionar mas não foi verificado.

### Windows ARM64

Fora do escopo. O gerador de código suporta a arquitetura, mas o esforço adicional não se justifica no momento.

## Resumo

| Plataforma | Estado | `spawn!` | Testes em hardware real |
|------------|--------|----------|------------------------|
| Linux x86_64 | ✅ Completo | ✅ | ✅ |
| macOS x86_64 | ✅ Completo | ✅ | ✅ (binário testado) |
| macOS ARM64 | ✅ Completo | ✅ | ✅ (binário testado) |
| Windows x86_64 | ⚠️ Experimental | ❌ Stub | ❌ Não testado |
| Windows ARM64 | ❌ Fora do escopo | — | — |