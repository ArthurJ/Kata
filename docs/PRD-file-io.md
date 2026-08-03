# PRD: File I/O — Handle Opaco para Arquivos Abertos

## Status

**Status:** ✅ Implementado (exceto `read_chunk` — streaming pendente)
**Data:** 2026-08-03 (atualizado após sessão 8 — modelo de memória finalizado)
**Depende de:** Fio 11 (CSP — `IoHandle` segue o pattern de `ChannelInner`), Fio 8 (coleções — `Bytes` é o tipo de retorno de `read`)
**Não depende de:** `spawn!` (File I/O é local ao fiber), canais cross-process

## 1. Objetivo

Introduzir I/O de arquivos na linguagem Kata: abrir, ler, escrever, fechar arquivos
locais via um handle opaco. O handle é um valor `File` que o usuário passa para
as actions de I/O — sem enxergar a representação interna (FD, path, modo).

`File` é o primeiro tipo de I/O. `IoHandle` é a camada comum de runtime entre
`File` e futuro `Socket` — ambos envolvem um descritor OS e um modo de operação.

## 2. Motivação

### 2.1. I/O de arquivo é fundamental

Toda linguagem de propósito geral precisa ler e escrever arquivos. Sem File I/O,
Kata não pode processar configuração, logs, dados de entrada, ou produzir
saída persistente. `echo!` para stdout existe, mas é insuficiente.

### 2.2. `Bytes` precisa de um consumidor real

O tipo `Bytes` (PRD-bytes) foi introduzido para marshalling de `spawn!` e
manipulação de dados binários. `read` é o consumidor natural — lê o conteúdo
de um arquivo como `Bytes` cru, sem assumptions de encoding. `write` aceita
`Bytes` para escrever dados binários.

### 2.3. `IoHandle` prepara o terreno para Sockets

File e Socket compartilham a mesma estrutura: um descritor OS, um modo de
operação (read/write/both), e operações de I/O. `IoHandle` é a camada comum
que evita duplicação quando Socket for implementado.

## 3. Design do tipo

### 3.1. `File` — tipo opaco intrínseco

`File` é uma variante dedicada de `Ty`, não `Prim(PrimTy)`:

```rust
pub enum Ty {
    // ... variantes existentes ...
    /// File — handle opaco para arquivo aberto. Encoding determinado
    /// pela operação: read → Bytes, readline → Text.
    File,
}
```

O usuário não enxerga fields, não faz pattern matching na estrutura, não
constrói `File` diretamente. O único modo de obter um `File` é via `open!`,
que retorna `Result::(File, Text)`.

### 3.2. `FileMode` — enum com 5 variantes

```kata
enum FileMode
    Read
    Write
    Append
    ReadWrite
    Create
```

| Variante | Semântica | OpenOptions do Rust |
|---|---|---|
| `Read` | Abre para leitura; falha se não existe | `File::open` |
| `Write` | Cria/trunca para escrita | `File::create` |
| `Append` | Cria se não existe, append no fim | `append(true).create(true)` |
| `ReadWrite` | Abre sem truncar, read+write | `read(true).write(true)` |
| `Create` | Cria exclusivo — falha se já existe | `create_new(true).write(true)` |

### 3.3. Posição no sistema de tipos

`File` é tratado como ponteiro opaco pelo codegen (`ty_to_clif` → `I64`).
No `TypeShape`, mapeia para `Prim` (valor escalar, não coleção). O codegen
rastreia file handles em `file_handle_vars` para close automático no
epílogo da action.

## 4. API — Actions no prelude

```kata
@ffi("kata_rt_file_open")
action open (path::Text, mode::FileMode) => Result::(File, Text)

@ffi("kata_rt_file_read")
action read (f::File) => Result::(Bytes, Text)

# FUTURO — não implementado ainda:
# @ffi("kata_rt_file_read_chunk")
# action read (f::File, n::Int) => Result::(Bytes, Text)

@ffi("kata_rt_file_readline")
action readline (f::File) => Result::(Text, Text)

@ffi("kata_rt_file_write_text")
action write (f::File, content::Text) => Result::(Unit, Text)

@ffi("kata_rt_file_write_bytes")
action write (f::File, content::Bytes) => Result::(Unit, Text)

@ffi("kata_rt_file_close")
action close (f::File) => Unit

# echo para file — escreve show(msg) + newline
action echo (msg::SHOW, f::File) => Unit
    let s := show msg
    let _ := write!(f, s)
    let _ := write!(f, "\n")
```

### 4.1. Convenções

- **Tudo que pode falhar retorna `Result::(T, Text)`.** O `Text` do `Err` é a
  mensagem de erro. `close` e `echo` não falham (retornam `Unit`).
- **`write` tem 2 overloads** — `Text` e `Bytes`. Cada overload mapeia para
  sua própria FFI: `kata_rt_file_write_text` e `kata_rt_file_write_bytes`.
  A FFI de Text trata o dado como C string (para no null byte). A FFI de
  Bytes lê o header de len (i64 no offset 0) e os dados no offset 8.
- **`echo` é action Kata composta**, não `@ffi`. Chama `write!` duas vezes.
- **`content` não se chama `data`** porque `data` é keyword no prelude.
- **`open` recebe `FileMode` como enum**, não como flag/int. O codegen
  extrai o tag da variante do Sum box via `sum_tag_int`.

### 4.2. Exemplo de uso

```
let f := open!("arquivo.txt", FileMode::Read)
match f
  Result::Ok handle:
    let content := read!(handle)
    match content
      Result::Ok bytes: echo!(show(bytes))
      Result::Err msg: echo!(msg)
  Result::Err msg: echo!(msg)
close!(handle)
```

## 5. Runtime

### 5.1. `IoHandle` — camada comum File/Socket

```rust
pub(crate) struct IoHandle {
    pub file: File,       // std::fs::File — descritor OS
    pub mode: IoMode,
}

pub(crate) enum IoMode {
    Read,
    Write,
    Append,
    ReadWrite,
    Create,
}
```

### 5.2. `FileInner` — arquivo aberto com path

```rust
pub(crate) struct FileInner {
    pub closed: bool,     // idempotência de close
    pub io: IoHandle,
    pub path: String,
}
```

O campo `closed` garante que `kata_rt_file_close` é idempotente —
múltiplas chamadas (close explícito + epílogo) não causam double-free.

### 5.3. FFI functions

```rust
kata_rt_file_open(path_ptr: *const c_char, mode_box: i64) -> i64
kata_rt_file_read(handle: i64) -> i64
kata_rt_file_readline(handle: i64) -> i64
kata_rt_file_write_text(handle: i64, data_ptr: i64) -> i64
kata_rt_file_write_bytes(handle: i64, data_ptr: i64) -> i64
kata_rt_file_close(handle: i64) -> ()
```

Todas as alocações (Result boxes, Bytes, Text, FileInner) usam
`kata_rt_arena_alloc` na root_arena. Sem header ARC, sem incref/decref.

### 5.4. Result boxes

As actions `@ffi` no prelude declaram retorno `Result::(T, E)`, mas o codegen
trata o retorno de FFIs `@ffi` como valor cru. Para que o `match` funcione, a
FFI precisa retornar um **Result box** (ponteiro para struct 16 bytes: tag +
payload), não um escalar.

Precedente: `kata_rt_list_get_checked` e `kata_rt_dict_get_checked` já fazem
isto. As FFIs de file constroem o Result box internamente via `arena_alloc`
na root_arena e escrevem tag (i64) no offset 0 e payload (i64) no offset 8.

### 5.5. Close — idempotente

`kata_rt_file_close` faz `drop_in_place` do `FileInner` (fecha o FD via Drop
de `File`, libera a `String` do path). O campo `closed` impede double-close.
A memória do `FileInner` permanece na root_arena até o teardown do processo
— `drop_in_place` não chama dealloc.

## 6. Decisões de memória

### 6.1. Onde os Result boxes vivem?

**Decisão: root_arena via `arena_alloc`.** Result boxes são alocados na
root_arena via `kata_rt_arena_alloc`. Sem header ARC, sem destructor
encadeado. A memória é liberada quando a root_arena é destruída no fim
do processo.

### 6.2. Onde os Bytes/Text de `read`/`readline` vivem?

**Decisão: root_arena via `arena_alloc`.** Bytes e Text são alocados na
root_arena. Sem header ARC, sem incref/decref. A memória é liberada no
fim do processo.

### 6.3. Handle — sem tag scheme

**Decisão: handle = ponteiro puro.** O tag scheme (3 bits baixos) foi
removido. Handle = `data_ptr` puro, validação por `ptr != 0`. O type
system já garante que só `Ty::File` chega às FFIs — o tag era redundante.

### 6.4. `write` com Bytes (null bytes) — DECIDIDO: FFI separada

**Decisão: FFI separada.** `kata_rt_file_write_text` trata o dado como
C string (para no null byte). `kata_rt_file_write_bytes` lê o header de
len (i64 no offset 0) e os dados no offset 8 — suporta null bytes. O
codegen despacha por overload: o overload com `Text` mapeia para
`write_text`, o overload com `Bytes` mapeia para `write_bytes`.

### 6.5. `read` slurp + streaming

**Decisão: ambas as APIs.** `read(handle)` permanece como conveniência
(slurp integral). `read(handle, n)` adicionado ao PRD como API de
streaming. Nova FFI `kata_rt_file_read_chunk(handle, n)`.

**Status: `read(handle)` implementado. `read(handle, n)` NÃO implementado.**

### 6.6. Close automático no epílogo

**Decisão: epílogo fecha handles não fechados.** O codegen rastreia
file handles abertos em `file_handle_vars` (registrados quando `let`
ou pattern binding recebe `Ty::File`). O epílogo da action chama
`kata_rt_file_close` em cada um. O close é idempotente — se o
programador chamou `close!` explicitamente, o epílogo é no-op.

## 7. Fases de implementação

### Fase 0: Modelo de memória — ✅ CONCLUÍDA

Modelo de memória finalizado: arenas bump para tudo, close explícito
para file handles. Ver `Kata-lang-manual.md` §5.2.2 para detalhes.

### Fase 1: Tipo e runtime — ✅ CONCLUÍDA

- `Ty::File` no enum, `type_name_str`, `ty_to_clif` (→ I64), `to_shape` (→ Prim), `Display`
- `file.rs`: `IoHandle`, `FileInner` (com campo `closed`), `IoMode`, 6 FFI functions
- Result boxes construídos via `arena_alloc` na root_arena

### Fase 2: Codegen e prelude — ✅ CONCLUÍDA

- `FfiSymbol::FileOpen/FileRead/FileReadline/FileWriteText/FileWriteBytes/FileClose` em `ffi.rs`
- Assinaturas Cranelift em `ffi_sigs.rs`
- Registros em `ffi_registry.rs`
- `Ty::File` em `type_table.rs`, `escape_arena.rs`
- `"File" => Ty::File` em `type_resolve.rs`
- `Ty::File` em `contains_channel_type` (false), `check_exhaustiveness`
- `enum FileMode` + 7 actions em `stdlib/core.kata`
- `file_handle_vars` em `LowerCtx` para close no epílogo
- Monomorphizador: `find` por nome + aridade (bug corrigido)

### Fase 3: Testes E2E — ✅ CONCLUÍDA

7 testes E2E passando:
- `file_open_read_bytes` — open + read → Bytes, verifica len
- `file_open_readline_text` — open + readline → Text, verifica conteúdo
- `file_write_read_roundtrip` — write + close + open + read, verifica round-trip
- `file_create_falha_se_existe` — Create em arquivo existente → Err
- `file_create_sucesso_se_nao_existe` — Create em arquivo novo → Ok
- `file_echo_writes_show_plus_newline` — echo!(42, file) + read, verifica "42\n"
- `file_open_inexistente_retorna_err` — open arquivo inexistente → Err

### Fase 4: Documentação — ✅ CONCLUÍDA

- `docs/Kata-lang-manual.md` §5.2.2 — modelo de memória documentado
- `docs/sintaxe-mapa.md` — pendente (adicionar seção File I/O)
- `docs/ROADMAP.md` — pendente (adicionar feature)

### Fase 5: Streaming — PENDENTE

Decisões de design (sessão 2026-08-03):

1. **Overload por aridade:** `read!(f)` (whole file) e `read!(f, n)` (chunk) coexistem,
   mesmo nome, aridade diferente. Monomorphizador resolve por nome+aridade.
2. **EOF como `Err("EOF")`:** consistente com `readline` atual. Err para EOF, Ok para
   dados (mesmo bytes vazios). Padrão uniforme com readline.
3. **BufReader persistente em `FileInner`:** todos os reads (read, read_chunk,
   readline) passam pelo mesmo BufReader. Resolve bug latente de readline (BufReader
   recreado perde bytes bufferizados em arquivos > 8KB) e previne state corruption
   entre read_chunk e readline intercalados.
4. **Ambos read (whole file) e read (chunk n) coexistem:** read_to_end para
   arquivos pequenos, chunks para streaming.

Implementação:

- `read(handle, n)` — nova FFI `kata_rt_file_read_chunk(handle, n) -> i64`
- Nova action no prelude: `@ffi("kata_rt_file_read_chunk") action read (f::File, n::Int) => Result::(Bytes, Text)`
- Overload de `read` com 2 params (dispatch por aridade — monomorphizador já suporta)
- FFI lê `n` bytes do arquivo e retorna Result box Ok(bytes) ou Err(text)
- Teste E2E: escrever conteúdo conhecido, ler em chunks, verificar

### Fase 6: Otimizações — PENDENTE

- `BufReader` dentro de `FileInner` — readline recria BufReader a cada chamada
- **Decisão:** BufReader persistente em FileInner, todos os reads passam por ele
- Resolve bug latente (bytes bufferizados perdidos em arquivos > 8KB)
- Previne state corruption entre read_chunk e readline intercalados

## 8. Decisões fechadas

| Decisão | Valor | Justificativa |
|---|---|---|
| `Ty::File` opaco | Sem `Box<Ty>`, sem fields expostos | Handle é implementação interna |
| `FileMode` enum | 5 variantes (Read/Write/Append/ReadWrite/Create) | Mapeia para `OpenOptions` do Rust |
| API | open, read, readline, write×2, close, echo | Cobertura mínima para I/O de arquivo |
| `Result::(T, Text)` | Tudo que falha retorna Result | Consistente com `INDEXABLE`, `dict_get_checked` |
| `IoHandle` | Camada comum File/Socket | Evita duplicação quando Socket existir |
| `@ffi` para builtins | open, read, readline, write_text, write_bytes, close | FFIs diretas no runtime |
| `echo` como action Kata | Não é `@ffi` | Compõe `write!` — sem FFI nova |
| `close` retorna `Unit` | Não falha | `close` de arquivo inválido é no-op |
| `content` não é `data` | `data` é keyword no prelude | Convenção do parser |
| Result box: arena bump | 6.1 — root_arena via `arena_alloc` | Sem overhead de ARC |
| Bytes/Text: arena bump | 6.2 — root_arena via `arena_alloc` | Sem overhead de ARC |
| Handle: ponteiro puro | 6.3 — tag scheme removido | Type system valida, não precisa de tag |
| write: FFI separada | 6.4 — `write_text` + `write_bytes` | Text para no null; Bytes tem len |
| `read` + `read_chunk` | 6.5 — ambas APIs | Conveniência + streaming |
| Close no epílogo | 6.6 — `file_handle_vars` + `kata_rt_file_close` | FD leak se programador esquece close |
| Close idempotente | Campo `closed` em `FileInner` | Double-close = no-op, não double-free |

## 9. Pendências

| Item | Status | Esforço |
|---|---|---|
| `read(handle, n)` / `kata_rt_file_read_chunk` | Não implementado | Médio — nova FFI + overload + teste |
| `BufReader` em `FileInner` | Não implementado | Baixo — mover struct |
| `sintaxe-mapa.md` seção File I/O | Não atualizado | Baixo — documentação |
| `ROADMAP.md` feature File I/O | Não atualizado | Baixo — documentação |
| Commit das mudanças | Pendente | `feat(rt,codegen,inference): remove ARC, arenas bump for all values` |
| Warning `path` field never read | Pendente | Remover campo ou usar |
| Snapshots TAST | Atualizados | Já passam — verificar se precisam re-approve |