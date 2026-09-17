# PRD: ReadResult — EOF Tipado em Leitura

## Status

**Status:** ✅ Completo
**Data:** 2026-09-17
**Resolve:** TODO.md item 🔴#2 (EOF representado como `Err("EOF")`) — removido do TODO após auditoria

## 1. Objetivo

Substituir `Result::(T, Text)` por `ReadResult(T)` nas funções de leitura
da stdlib (File e Socket). EOF deixa de ser `Err("EOF")` (string opaca) e
vira uma variante tipada do enum `ReadResult`, distinta de `Error(Text)`.
`write`, `open`, `close`, e `listen` continuam com `Result` — não têm EOF.

## 2. Motivação

### 2.1. EOF não é erro

EOF é terminação normal de I/O — o stream acabou, não houve falha. Hoje a
FFI retorna `alloc_result_box(1, error_text("EOF"))` — um `Err` com payload
`Text("EOF")`. O caller faz `match read!(h) { Ok data: ..., Err msg: ... }`
e precisa distinguir `msg == "EOF"` (parar) de `msg == "permission denied"`
(abortar) por string-matching.

### 2.2. String-matching é frágil

O caller não consegue distinguir terminação normal de falha real sem
inspecionar o conteúdo da string. Typos, case, i18n quebram a distinção.
O tipo `Text` no `Err` não comunica quais valores são possíveis — não é
contrato.

### 2.3. Leitura é inerentemente tri-valorada

`Result` é binário: sucesso ou falha. Leitura tem três estados: dado,
fim, erro. Enfiar Eof dentro de `Err` como variante de um enum de erro
(`Err(IoError)`) categoriza terminação normal como falha — o mesmo
problema semântico, só tipado. `ReadResult` coloca os três estados no
mesmo nível. Escrita não tem EOF — continua com `Result`.

## 3. Design

### 3.1. Enum `ReadResult`

```kata
enum ReadResult(T)
    Data(T)
    Error(Text)
    Eof
```

**Ordem das variantes:** `Data` primeiro (sucesso — dados lidos), `Error(Text)` no meio
(falha com informação), `Eof` último (terminação normal, cauda). Segue a
convenção do Kata: última variante = caso default/falha, como `Err` em
`Result` e `False` em `Boolean`.

`Eof` não tem payload — EOF não carrega informação além de si mesmo.

**Nome `Data` em vez de `Ok`:** `Ok` colide com `Result::Ok` no
EnumRegistry — variantes desqualificadas de mesmo nome em dois enums
causam ambiguidade. `Data` é semanticamente mais preciso: comunica
"dados lidos do stream", não apenas "sucesso genérico". `Error` não
colide com `Err` (nomes diferentes).

### 3.2. `|` (pipe fallback) não se aplica

O `|` desugara para match onde todas as variantes não-cauda desempacotam
o payload e a cauda avalia o fallback. A invariante exige que toda
variante não-cauda tenha payload. `Eof` é cauda (última, sem payload) —
correto. Mas `Ok(T)` e `Error(Text)` têm payloads de tipos diferentes
quando `T ≠ Text` (ex: `read` retorna `ReadResult(Bytes)`). O match
sintético não unifica `Bytes` com `Text`.

`|` é coalescência binária (unwrap um, fallback outro). Com 3 variantes
onde duas têm payloads de tipos diferentes, não há configuração que faça
o `|` tipar. Isso é correto — coalescer 3 estados em 1 fallback é
exatamente a indistinção que `ReadResult` existe para evitar.

**Decisão:** `ReadResult` não é compatível com `|`. Match explícito sempre.

### 3.3. `?` (fail-fast) não se aplica

`?` é binário: desugara para `match { Ok v => v, Err e => return Err(e) }`.
Hardcoded para `Result` e `Optional`. Com 3 variantes, não há semântica
clara para o que `?` faz com `Eof` — propagar como early-return
conflaciona terminação com erro.

**Decisão:** `?` não se estende para `ReadResult`. Match explícito sempre.

### 3.4. Layout runtime

`ReadResult` é um sum type genérico — mesmo layout de `Result` e `Optional`:
16 bytes na arena, tag (i64) no offset 0, payload (i64) no offset 8.

| Variante | Tag | Payload |
|---|---|---|
| `Data(T)` | 0 | ponteiro para T |
| `Error(Text)` | 1 | ponteiro para C string |
| `Eof` | 2 | 0 (não usado) |

A FFI constrói o sum box com `alloc_result_box(tag, payload)` — mesma
função que hoje usa para `Result`. A mudança é trocar
`alloc_result_box(1, error_text("EOF"))` por `alloc_result_box(2, 0)`.

### 3.5. Stdlib — assinaturas afetadas

```kata
# Antes:
action read     (f::File)           => Result::(Bytes, Text)
action read     (f::File, n::Int)   => Result::(Bytes, Text)
action readline (f::File)           => Result::(Text, Text)

# Depois:
action read     (f::File)           => ReadResult::(Bytes)
action read     (f::File, n::Int)   => ReadResult::(Bytes)
action readline (f::File)           => ReadResult::(Text)
```

Socket — mesmas 3 funções com `Socket` em vez de `File`:

```kata
# Depois:
action read     (s::Socket)           => ReadResult::(Bytes)
action read     (s::Socket, n::Int)   => ReadResult::(Bytes)
action readline (s::Socket)           => ReadResult::(Text)
```

**Não afetadas (continuam com `Result`):**
- `write` (File e Socket) — escrita não tem EOF. `Result::(Unit, Text)`.
- `open` (File e Socket) — falha ao abrir é erro genuíno. `Result::(File/Socket, Text)`.
- `listen` (Socket) — aceitar conexão não tem EOF. `Result::(Socket, Text)`.
- `close` (File e Socket) — fechar não tem EOF. `Result::(Unit, Text)`.
- `input` (stdin) — retorna `Text` (vazio em EOF). Não é `Result`, não muda.
- `div :: Self Self => Result::(Self, Text)` — divisão por zero é erro, não EOF.

### 3.6. EOF com dados no buffer (readline parcial)

Hoje: `readline!` em EOF com dados no `line_buf` retorna `Ok(linha_parcial)`
em vez de `Err("EOF")`. A linha parcial é dados válidos — o caller
processa e na próxima chamada recebe EOF.

Com `IoResult` o comportamento não muda: retorna `Ok(linha_parcial)`.
`Eof` só aparece quando o buffer está vazio e o FD retorna 0 bytes.

## 4. Mudanças

### 4.1. Stdlib (`core.kata`)

1. Declarar `enum ReadResult(T)` com `Ok(T)`, `Error(Text)`, `Eof`
2. Trocar tipo de retorno de 6 funções de leitura (3 File + 3 Socket) de
   `Result::(T, Text)` para `ReadResult(T)`

### 4.2. FFI (`kata-rt`)

Substituir `alloc_result_box(1, error_text("EOF"))` por
`alloc_result_box(2, 0)` em 5 pontos:

| Arquivo | Função | Linha atual |
|---|---|---|
| `file.rs` | `kata_rt_file_read` | 592 |
| `file.rs` | `kata_rt_file_readline` | 672 |
| `socket/io.rs` | `kata_rt_socket_read` | 51 |
| `socket/io.rs` | `kata_rt_socket_read_chunk` | 125 |
| `socket/io.rs` | `kata_rt_socket_readline` | 226 |

Os erros reais (não-EOF) continuam com `alloc_result_box(1, error_text("..."))`
— tag 1 = `Error`, payload = mensagem. O `Ok` continua tag 0, payload =
ponteiro para dados.

### 4.3. Codegen

O codegen já lowered sum types genericamente via `VariantQual` (variantes
sem payload) e `VariantConstruct` (variantes com payload). `ReadResult` é
um enum genérico como `Result` e `Optional` — o codegen não precisa de
modificação estrutural.

O `EnumRegistry` precisa conhecer `ReadResult` — mas isso é automático: a
declaração em `core.kata` é processada pelo resolution/inference como
qualquer outro enum genérico.

### 4.4. `input!` (stdin)

`kata_rt_input` retorna `Text` (vazio em EOF), não `Result`. Não muda.

### 4.5. Testes E2E

Os testes atuais fazem `Err _` para EOF — o pattern continua válido
porque `Eof` cai no braço `Err _` do match? **Não.** `ReadResult` não é
`Result`. O match em `ReadResult` tem variantes `Ok`, `Error`, `Eof` — não
`Ok`/`Err`. `Err _` não é um pattern válido para `ReadResult`.

Mudança nos testes: trocar `Err _` por `Eof` ou `Error _` conforme o caso.
Testes que verificam EOF explícito (arquivo vazio, peer fechou) trocam
`Err _` por `Eof`. Testes que verificam erro real trocam `Err _` por
`Error _`.

Testes afetados (estimativa):
- `file_io_e2e.rs` — `file_read_chunk_eof_imediato`, `file_read_chunk_streaming`
- `socket_tcp_streaming.rs` — `socket_read_chunk_streaming`
- `socket_tcp_readline.rs` — `socket_readline_eof_partial`
- `select_io_e2e.rs` — `select_file_eof`

## 5. Testes novos

- `io_result_eof_file_read` — `read!(file)` em arquivo vazio retorna `Eof`
- `io_result_eof_file_readline` — `readline!(file)` em arquivo vazio retorna `Eof`
- `io_result_eof_socket_read` — `read!(socket)` após peer fechar retorna `Eof`
- `io_result_eof_socket_readline` — `readline!(socket)` após peer fechar retorna `Eof`
- `io_result_error_file` — `read!(file_fechado)` retorna `Error(msg)` (não `Eof`)
- `io_result_match_exaustivo` — match em `ReadResult` sem cobrir `Eof` é erro de exaustividade
- `io_result_ok_data` — `read!(file_com_dados)` retorna `Ok(bytes)`

## 6. Estruturas afetadas

| Arquivo | Mudança |
|---|---|
| `stdlib/core.kata` | Declarar `enum ReadResult(T)`. Trocar retorno de 6 funções de leitura. |
| `kata-rt/src/file.rs` | `alloc_result_box(1, error_text("EOF"))` → `alloc_result_box(2, 0)` em 2 pontos. |
| `kata-rt/src/socket/io.rs` | `alloc_result_box(1, error_text("EOF"))` → `alloc_result_box(2, 0)` em 3 pontos. |
| `kata-codegen/tests/file_io_e2e.rs` | `Err _` → `Eof` em testes de EOF. |
| `kata-codegen/tests/socket_tcp_streaming.rs` | `Err _` → `Eof` em teste de EOF. |
| `kata-codegen/tests/socket_tcp_readline.rs` | `Err _` → `Eof` em teste de EOF parcial. |
| `kata-codegen/tests/select_io_e2e.rs` | `Err _` → `Eof` em teste de select com EOF. |

## 7. Fora do escopo

- **`IoError` enum dentro de `Result`** — alternativa considerada e
  rejeitada. Categoriza EOF como `Err` (falha), preservando o problema
  semântico. `ReadResult` tri-valorado é semanticamente correto.
- **Extensão do `?` para 3 variantes** — `?` é binário por design.
  Estender quebra a semântica do operador. Leitura pede decisão explícita.
- **Extensão do `|` para `ReadResult`** — `|` é coalescência binária.
  Com 3 variantes de payloads de tipos diferentes, não tipa. Correto.
- **`connect` TCP non-blocking** — TODO #1. Ortogonal.
- **Trampoline engole erros** — TODO #3. Ortogonal.
- **`listen!` → `accept!`** — TODO #4. Ortogonal.

## 8. Riscos

### R1: Breakage em código usuário que faz `Err _` em read/readline

Código que hoje faz `match read!(h) { Ok d: ..., Err _: ... }` para de
compilar — `ReadResult` não tem `Ok` nem `Err`. O compilador reporta
exaustividade: `Eof` não coberto. **Mitigação:** o erro de exaustividade
já existe no Kata (PRD-exaustividade-aninhada) e aponta exatamente qual
variante falta. O fix é trocar `Ok d` por `Data d`, `Err _` por `Eof` ou
`Error _`. Breakage intencional e salutar — o código estava tratando
EOF como erro.

### R2: Select com leitura — tipo do braço muda

O `select` hoje dispara o braço com o valor retornado pelo read. Se
read retorna `ReadResult(Bytes)` em vez de `Result::(Bytes, Text)`, o body
do braço precisa fazer match em `ReadResult`. O select em si é indiferente
ao tipo — só passa o valor. **Mitigação:** mudança mecânica nos testes
de select com I/O. O select não inspeciona o tipo do valor do braço.