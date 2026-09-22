# Appendix — Platforms and Limitations

Kata is developed on Linux (Arch, x86_64). This section documents the state of the port to other platforms — what works, what is experimental, and what does not work yet.

## Linux (x86_64)

Primary development platform. All features are implemented and tested.

## macOS (x86_64 and Apple Silicon)

Complete and verified port. The binary compiles natively for both architectures after a POSIX compatibility fix. Mach-O binaries were generated and tested on a real Mac (Apple Silicon).

Building from source:

```bash
git clone <repo-url> && cd kata
cargo build --release
```

The `kata` binary works in both JIT and AOT mode. All features — including `fork!`, channels, `select`, and `spawn!` — are available.

## Windows (x86_64)

Experimental stage port. The compiler generates a native PE32+ binary (24MB) via cross-compilation (`x86_64-pc-windows-gnu`), but it has **never been tested on real Windows**. Most features are implemented, but there are important limitations.

### What works (in theory — not verified on hardware)

- JIT compilation and code generation
- Interactive REPL
- File I/O, and the stdio values (`__stdin__`, `__stdout__`, `__stderr__` via `import stdio`)
- TCP sockets
- Channels (`channel!`, `<!`, `!>`, `select`, `timeout`)
- `fork!` (cooperative fibers)
- `sleep!`

### What does not work

**`spawn!`** — is a stub on Windows. The `spawn!` action exists and compiles, but at runtime it is a no-op that returns 0. There is no child process, no communication. The design decision (whether to use `CreateProcessW`, threads, or another mechanism) is pending.

### What works differently

**Unix domain sockets** — on Linux and macOS, Unix sockets use the filesystem (`/tmp/socket`). On Windows, they are replaced by TCP localhost with a port coordination file. Functionally equivalent for IPC, but the implementation is different.

**Signals** — `SIGPIPE` and `SIGCHLD` do not exist on Windows. Handling of broken pipes and process reaping uses different Win32 mechanisms internally. The user does not notice the difference — except that `spawn!` (which would depend on signals) does not work.

### What has not been verified

- **Testing on real Windows** — the complete test suite has never been run on Windows. There may be undiscovered bugs in runtime, codegen, or AOT linking.
- **Tail calls** — tail call optimization (`CallConv::Tail`) is supported by the code generator in theory, but has not been tested empirically on Windows.
- **AOT build** — the Windows linker (`lld-link`) is configured, but has never been tested with a real program on Windows.
- **MSVC toolchain** — only the MinGW toolchain (`x86_64-pc-windows-gnu`) has been tested via cross-compilation. The MSVC toolchain (`x86_64-pc-windows-msvc`) should work but has not been verified.

### Windows ARM64

Out of scope. The code generator supports the architecture, but the additional effort is not justified at this time.

## Summary

| Platform | Status | `spawn!` | Tests on real hardware |
|-----------|--------|----------|------------------------|
| Linux x86_64 | ✅ Complete | ✅ | ✅ |
| macOS x86_64 | ✅ Complete | ✅ | ✅ (binary tested) |
| macOS ARM64 | ✅ Complete | ✅ | ✅ (binary tested) |
| Windows x86_64 | ⚠️ Experimental | ❌ Stub | ❌ Not tested |
| Windows ARM64 | ❌ Out of scope | — | — |