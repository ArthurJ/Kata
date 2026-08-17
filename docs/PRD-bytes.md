# PRD: Bytes — Tipo Blob Contíguo para Marshalling e I/O

## Status

**Status:** 📄 Rascunho
**Data:** 2026-07-30
**Depende de:** Fio 11 (CSP — `spawn!` usa `Bytes` como formato serializado), Fio 8 (coleções — `Bytes` segue modelo de `Array`/`Text`)
**Não depende de:** `spawn!` (Bytes tem utilidade independente — I/O, buffers, manipulação de dados crus)

## 1. Objetivo

Introduzir o tipo `Bytes` na linguagem: uma sequência contígua de bytes (u8)
com acesso indexado, concatenação, operadores bitwise, e conversão explícita
de/para `Int`. `Bytes` é o tipo de retorno de `to_bytes()` e o formato aceito
por `spawn!{serialized: ...}`, mas também é um tipo útil por si só — I/O cru,
buffers binários, manipulação de dados de baixo nível.

## 2. Motivação

### 2.1. Marshalling de `spawn!`

`to_bytes()` precisa retornar um tipo. O blob convertido (bytes contíguos +
rebase_offsets) precisa ser armazenável, passável, e reusável. Hoje a Kata5
não tem um tipo para dados binários crus — `Text` é string (bytes + encoding),
`Array<Int>` é 8x o tamanho (cada byte vira i64).

### 2.2. I/O binário

Quando a Kata5 tiver I/O de arquivo/socket, `Bytes` é o tipo natural para ler/
escrever dados crus. Não é um tipo criado só para `spawn!` — é um tipo que
falta na linguagem.

### 2.3. Operações de baixo nível

Aritmética de bytes (AND, OR, XOR, shifts) é necessária para protocolos
binários, criptografia, e manipulação de buffers. Hoje a linguagem não tem
operadores bitwise — este PRD introduz `and`, `or`, `xor`, `>>`, `<<`.

### 2.4. Text como coleção indexável

Hoje `Text` não implementa `INDEXABLE` nem `COUNTABLE`. Não é possível acessar
codepoints individuais de uma string, nem saber seu tamanho em codepoints,
nem fazer slice. Isso é uma lacuna — strings são a estrutura de dados mais
fundamental da linguagem e não são tratadas como coleções. Este PRD corrige
isso, operando em codepoints Unicode (não bytes crus).

### 2.5. I/O de arquivos

`Bytes` é o tipo natural para leitura/escrita de arquivos binários. Quando
a Kata5 tiver I/O de arquivo (futuro), `read!(file) => Bytes` e
`write!(file, bytes)` usam `Bytes` como tipo de dados crus. Este PRD cria
o tipo; o I/O é um fio separado.

## 3. Design do tipo

### 3.1. Posição no sistema de tipos

`Bytes` NÃO é `Prim(PrimTy)`. PrimTy é o mapeamento FFI (Int→i64, Float→f64,
Text→ptr, Rational→ptr). `Bytes` segue o modelo de `List`, `Array`, `Dict` —
uma variante dedicada de `Ty`:

```rust
pub enum Ty {
    // ... variantes existentes ...
    /// Bytes — sequência contígua de u8. Blob opaco para I/O e marshalling.
    Bytes,
}
```

O codegen trata `Bytes` como ponteiro (como `Array`, `Text`, `Struct`). O
runtime gerencia o layout interno.

### 3.2. Layout no runtime

```rust
/// Blob contíguo de bytes.
/// Layout: [len: i64][data: u8 * len]
/// O ponteiro retornado aponta para o início do header.
pub(crate) struct Bytes {
    len: i64,
    data: *mut u8,  // len bytes alocados na arena
}
```

Alocação na arena (bump para dados locais, tracked para dados que escapam),
igual a `Array` e `Text`.

### 3.3. Sintaxe

```
# Literal — byte string: b"..."
let b := b"Hello"              # 5 bytes: 48 65 6c 6c 6f
let raw := b"\x00\xFF"         # 2 bytes: 00 FF

# Acesso indexado — .N (retorna Result, como INDEXABLE)
let primeiro := b.0              # Result::(Byte, Text) — Ok byte ou Err out of bounds
let p := b.0 | byte(0)           # fallback com |
let ultimo := b.(-1)             # índice negativo (final)

# Concatenação — + (como List, Set, Dict)
let combinado := b + b2

# Tamanho — |b| (como List, Array, Text)
let n := |b|

# Slice — b.[start..end] (nova sintaxe DotAccess com Range)
let sub := b.[1..3]              # bytes 1 até 3 (exclusive)
let sub2 := b.[2..=5]           # bytes 2 até 5 (inclusive)

# Conversão explícita Byte → Int
let valor := int(b.0)             # 72 (0x48 = 'H')

# Conversão explícita Int → Bytes (4 bytes little-endian)
let bytes4 := bytes(42)          # [2A 00 00 00]
```

### 3.4. Byte não é Int

`Byte` é um tipo distinto de `Int`. `b.0` retorna `Byte`, não `Int`. Isso
impede aritmética acidental — `b.0 + 1` é erro de tipo (Byte + Int não
compila). Para fazer aritmética, converter explicitamente:

```
let v := int(b.0)              # Byte → Int
let v2 := + v 1                # Int + Int = 73
```

Conversão explícita via `int()` e `bytes()` — mesma convenção de
`int("42")` / `text(42)` que já existe.

### 3.5. Indexação

Segue o padrão existente de `DotAccess`:

- `b.0` — índice 0 (primeiro byte)
- `b.3` — índice 3
- `b.(-1)` — índice -1 (último byte)

O typechecker infere `b.N :: Result::(Byte, Text)` quando `b :: Bytes` —
consistente com `INDEXABLE::A` existente (`at :: Self Int => Result::A`).
Retorna `Ok(Byte)` se o índice é válido, `Err(Text)` com mensagem se out of
bounds. Para o caso comum onde o índice é sabidamente válido, o programador
usa `|` (fallback): `b.0 | byte(0)`.

O lowering emite `kata_rt_bytes_get(ptr, idx)` que faz bounds check e retorna
um `Result` (SMI tagged).

### 3.6. Concatenação

`+ :: Bytes Bytes => Bytes` — sobrecarga do operador `+` já usado para
concatenação de List, união de Set, e merge de Dict. Semântica idêntica a
`+ :: List List => List`: cria novo blob com os bytes dos dois operandos em
sequência.

FFI: `kata_rt_bytes_concat(a_ptr, b_ptr) -> ptr`.

## 4. Operadores Bitwise

### 4.1. Novos operadores

A Kata5 usa notação prefixa — operadores são identificadores. Os seguintes
passam a ter sobrecargas para `Byte` e `Bytes`:

| Operador | Sintaxe | Semântica |
|---|---|---|
| `and` | `and a b` | AND bit-a-bit (Byte, Byte) => Byte |
| `or` | `or a b` | OR bit-a-bit (Byte, Byte) => Byte |
| `xor` | `xor a b` | XOR bit-a-bit (Byte, Byte) => Byte |
| `not` | `not a` | NOT bit-a-bit (Byte) => Byte (inverte todos os bits) |
| `>>` | `>> a n` | Shift right lógico (Byte, Int) => Byte (desloca n bits) |
| `<<` | `<< a n` | Shift left (Byte, Int) => Byte (desloca n bits) |

### 4.2. Sobrecarga para Bytes

`and`, `or`, `xor` também operam em `Bytes` (elemento-a-elemento). Quando os
operandos têm tamanhos diferentes, o menor é zero-padded até o tamanho do
maior (broadcast):

```
let mask := b"\xFF\x00\xFF\x00"
let masked := and bytes mask     # AND elemento-a-elemento
let mixed := and b"\xFF\xF0" b"\xAA"  # broadcast → 2 bytes: [AA 00]
```

Broadcast com zero-pad preserva a semântica algebraica: `x OR 0 = x`,
`x XOR 0 = x`, `x AND 0 = 0`. O resultado tem sempre o tamanho do maior
operand.

### 4.3. Tokens no lexer

`>>` e `<<` são novos tokens multi-char:

| Token | Sintaxe | Como lexar |
|---|---|---|
| `Shr` | `>>` | `>` seguido de `>` — verificar peek após consumir `>` |
| `Shl` | `<<` | `<` seguido de `<` — verificar peek após consumir `<` |

**Atenção:** `<` já tem lookahead para `!>` (RecvArrow). A ordem de verificação
deve ser: `<` seguido de `!` → RecvArrow; `<` seguido de `<` → Shl; senão →
lex_ident (operador de comparação `<`).

`and`, `or`, `xor`, `not` são identificadores comuns (notação prefixa) —
nenhum token novo necessário. O typeck resolve a sobrecarga pelo tipo dos
operandos, igual a `+`.

### 4.4. Parser

`>>` e `<<` seguem o mesmo padrão de `|` e `|>` — operadores infixos baixos
no loop de pós-aplicação de `parse_expr`:

```rust
Token::Shr => {
    parser.advance();
    let rhs = parse_apply(parser)?;
    lhs = Expr::BinOp { op: ">>".into(), lhs: Box::new(lhs), rhs: Box::new(rhs) };
}
Token::Shl => {
    parser.advance();
    let rhs = parse_apply(parser)?;
    lhs = Expr::BinOp { op: "<<".into(), lhs: Box::new(lhs), rhs: Box::new(rhs) };
}
```

`and`, `or`, `xor`, `not` são identificadores — já são parseados como apply
de função. O typeck resolve a sobrecarga.

### 4.5. Typeck

Novas sobrecargas no DispatchTable:

```
and :: Byte Byte => Byte          @ffi("kata_rt_byte_and")
or  :: Byte Byte => Byte          @ffi("kata_rt_byte_or")
xor :: Byte Byte => Byte          @ffi("kata_rt_byte_xor")
not :: Byte => Byte               @ffi("kata_rt_byte_not")
>>  :: Byte Int => Byte           @ffi("kata_rt_byte_shr")
<<  :: Byte Int => Byte           @ffi("kata_rt_byte_shl")

and :: Bytes Bytes => Bytes       @ffi("kata_rt_bytes_and")
or  :: Bytes Bytes => Bytes       @ffi("kata_rt_bytes_or")
xor :: Bytes Bytes => Bytes       @ffi("kata_rt_bytes_xor")

# Sobrecarga de + (já existe para List, Set, Dict)
+ :: Bytes Bytes => Bytes         @ffi("kata_rt_bytes_concat")
```

## 5. Interfaces

### 5.1. Interfaces novas

```kata
interface SLICEABLE::A
    slice :: Self Int Int => A
```

### 5.2. Interfaces existentes — implementações para Bytes

| Interface | Assinatura | FFI |
|---|---|---|
| `INDEXABLE::Byte` | `.N :: Bytes => Result::(Byte, Text)` | `kata_rt_bytes_get` | Byte em índice N, ou `Err` se out of bounds |
| `COUNTABLE` | `len :: Bytes => Int` | `kata_rt_bytes_len` | Número de bytes |
| `EQ` | `= :: Bytes Bytes => Boolean` | `kata_rt_bytes_eq` | Comparação byte-a-byte |
| `SHOW` | `show :: Bytes => Text` | `kata_rt_bytes_show` | Representação hex |
| `SLICEABLE::Bytes` | `slice :: Bytes Int Int => Bytes` | `kata_rt_bytes_slice` | Sub-blob por range de bytes |

`+ :: Bytes Bytes => Bytes` é overload do operador `+` (não interface — segue
o padrão de List/Set/Dict).

### 5.3. Text como INDEXABLE, COUNTABLE e SLICEABLE

Hoje `Text` implementa apenas `SHOW` e `HASHABLE`. Não implementa `INDEXABLE`
nem `COUNTABLE` — não há como acessar caracteres individuais ou contar o
tamanho de uma string na linguagem. Isso é uma lacuna.

Este PRD adiciona:

| Interface | Assinatura | FFI | Descrição |
|---|---|---|---|
| `INDEXABLE::Text` | `.N :: Text => Result::(Text, Text)` | `kata_rt_text_at` | Codepoint em índice N — retorna `Text` de comprimento 1, ou `Err` se out of bounds |
| `COUNTABLE` | `len :: Text => Int` | `kata_rt_text_len` | Número de codepoints Unicode (não bytes) |
| `SLICEABLE::Text` | `slice :: Text Int Int => Text` | `kata_rt_text_slice` | Sub-string por range de codepoints |

**Decisão: codepoint, não byte.** `Text` opera em codepoints Unicode. `t.0`
retorna um `Text` contendo o primeiro codepoint. `len(t)` retorna o número de
codepoints. `t.[0..4]` retorna os primeiros 4 codepoints como `Text`. Internamente
`Text` é UTF-8 (1-4 bytes por codepoint), mas a indexação é por codepoint — o
runtime faz a decodificação.

**`Text` indexa para `Text`,** não para um tipo `Char` novo. `t.0` é um `Text`
de comprimento 1. Isso evita adicionar um tipo novo à linguagem — `Text` já
existe, já tem `SHOW`, `EQ`, `HASHABLE`, concatenação via `string_concat`. Um
codepoint isolado é semanticamente um `Text` curto.

**Indexação retorna `Result`.** `t.0` retorna `Result::(Text, Text)` — `Ok`
com o codepoint, ou `Err` com mensagem se o índice está fora dos limites. Isso
é consistente com `INDEXABLE::A` existente (`at :: Self Int => Result::A`).
Para o caso comum onde o índice é sabidamente válido, o programador usa `|`:

```
let c := t.0 | "?"          # fallback para codepoint inválido
```

**Conversão Text ↔ Bytes é explícita:** `bytes(text)` codifica para UTF-8
(produz `Bytes`), `text(bytes)` decodifica UTF-8 (pode falhar em bytes
inválidos — retorna `Result::(Text, Text)`). Para operar em bytes crus de
uma string, converter para `Bytes` explicitamente.

**`len` já existe como FFI** (`kata_rt_string_len` no manual, linha 559), mas
não está exposta como implementação de `COUNTABLE` no prelude. Este PRD
conecta a FFI existente à interface. A FFI atual conta bytes — precisa ser
ajustada ou uma nova FFI `kata_rt_text_len` conta codepoints.

### 5.4. Array e List como SLICEABLE

`Array::A` já implementa `INDEXABLE::A` e `COUNTABLE`. Adiciona `SLICEABLE::A`:

| Interface | Assinatura | FFI |
|---|---|---|
| `SLICEABLE::(Array::A)` | `slice :: (Array::A) Int Int => (Array::A)` | `kata_rt_array_slice` |

`List::A` adiciona `SLICEABLE::(List::A)`:

| Interface | Assinatura | FFI |
|---|---|---|
| `SLICEABLE::(List::A)` | `slice :: (List::A) Int Int => (List::A)` | `kata_rt_list_slice` |

## 6. Conversões

### 6.1. Conversões básicas

| Função | Assinatura | Descrição |
|---|---|---|
| `int` | `int :: Byte => Int` | Byte → Int (0-255) |
| `byte` | `byte :: Int => Byte` | Int → Byte (trunca para 0-255, mod 256) |
| `bytes` | `bytes :: Int => Bytes` | Int → 4 bytes (little-endian) |
| `to_bytes` | `to_bytes :: A => Bytes` | Valor qualquer → blob convertido (spawn!) |
| `from_bytes` | `from_bytes :: Bytes => A` | Blob convertido → valor (usado no spawn!) |

### 6.2. Conversões Text ↔ Bytes com encoding

| Função | Assinatura | Descrição |
|---|---|---|
| `bytes` | `bytes :: Text => Bytes` | Text → Bytes (codifica UTF-8 — padrão) |
| `bytes` | `bytes :: Text Encoding => Bytes` | Text → Bytes (codifica com encoding especificado) |
| `text` | `text :: Bytes => Result::(Text, Text)` | Bytes → Text (decodifica UTF-8, pode falhar) |
| `text` | `text :: Bytes Encoding => Result::(Text, Text)` | Bytes → Text (decodifica com encoding, pode falhar) |

**Sobrecarga de aridade:** `DispatchTable` filtra por aridade
(`info.params.len() != args.len()` → skip). `bytes` com 1 param e `bytes` com
2 params são overloads distintas que coexistem sem ambiguidade. Confirmado
pela leitura do código em `dispatch.rs` linha 207 e `overload_resolution.rs`
linha 34.

**Enum `Encoding`:**

```kata
enum Encoding { Utf8, Utf16, Latin1, Ascii }
```

Encodings suportados são determinados pelo que o Rust std oferece nativamente:

- **UTF-8** — `String::from_utf8` / `String::into_bytes` (std, sem deps)
- **UTF-16** — `String::from_utf16` (std, sem deps)
- **Latin-1** — implementação trivial (1 byte = 1 codepoint, tabela direta)
- **ASCII** — range check (byte ≤ 0x7F); rejeita bytes > 0x7F com erro. Simples
  de implementar e útil como validação — garante que o conteúdo é ASCII puro.

Outros encodings (Shift-JIS, Big5, etc.) exigiriam a crate `encoding_rs`
(dependência externa). Adiar até demanda real.

## 7. Questões em aberto

### 7.1. Slice — `b.[1..3]`

Sintaxe decidida: `b.[1..3]` — DotAccess com Range literal entre `[]`.

Hoje DotAccess aceita `DotIndex::Field(name)` e `DotIndex::Int(n)`. A extensão
adiciona `DotIndex::Range(start, end)` — o parser produz este índice quando
encontra `.[` após um expression (sinal de que vem um range/expr entre colchetes).

```
b.[1..3]        # bytes do índice 1 ao 3 (exclusive)
b.[2..=5]       # bytes do índice 2 ao 5 (inclusive)
t.[0..4]        # primeiros 4 caracteres de um Text
```

Isso requer uma nova interface:

```kata
interface SLICEABLE::A
    slice :: Self Int Int => A
```

`Bytes`, `Text`, `Array::A`, e `List::A` implementam `SLICEABLE`.

### 7.2. Mutabilidade

`Bytes` é imutável (como `Array` e `Text`). `kata_rt_bytes_set` existe no
runtime para uso interno (construção de blobs pelo runtime), mas não é
exposta na linguagem. Para modificar, criar novo `Bytes` via concatenação ou
conversão.

### 7.3. Bytes literal — sintaxe

`b"Hello"` ou `b'Hello'` — byte string literal. Aspas duplas e simples são
equivalentes (mesma semântica que Text). Conteúdo entre aspas é interpretado
como bytes crus (não UTF-8 processado, não escape sequences de texto). Aceita
qualquer byte 0x00-0xFF.

```
let b := b"Hello"              # 5 bytes: 48 65 6c 6c 6f
let raw := b"\x00\xFF"         # 2 bytes: 00 FF (escape hex)
let mixed := b"ABC\x00"        # 4 bytes: 41 42 43 00
let eq := b'Hello'             # equivalente a b"Hello"
```

Escape sequences: `\xNN` (hex byte), `\\` (backslash literal), `\"`, `\'` (aspas).
Demais escapes de Text (`\n`, `\t`) também aceitos — representam o byte
correspondente.

## 8. Runtime

### 8.1. FFI functions

```rust
// Alocação
kata_rt_bytes_alloc(len: i64) -> i64          // ptr para blob de len bytes (não inicializado)
kata_rt_bytes_from_ptr(src: i64, len: i64) -> i64  // cria blob copiando de src
kata_rt_bytes_from_ints(ptrs: i64, count: i64) -> i64  // cria blob de array de i64s

// Acesso
kata_rt_bytes_get(ptr: i64, idx: i64) -> i64  // byte em idx (0-255 como SMI)
kata_rt_bytes_set(ptr: i64, idx: i64, val: i64)  // seta byte (uso interno)
kata_rt_bytes_len(ptr: i64) -> i64

// Operações
kata_rt_bytes_concat(a: i64, b: i64) -> i64
kata_rt_bytes_eq(a: i64, b: i64) -> i64        // 0 ou 1
kata_rt_bytes_show(ptr: i64) -> i64           // Text (hex)

// Slice
kata_rt_bytes_slice(ptr: i64, start: i64, end: i64) -> i64   // sub-blob
kata_rt_text_slice(ptr: i64, start: i64, end: i64) -> i64    // sub-string
kata_rt_array_slice(ptr: i64, start: i64, end: i64) -> i64   // sub-array
kata_rt_list_slice(ptr: i64, start: i64, end: i64) -> i64   // sub-list

// Bitwise (elemento-a-elemento)
kata_rt_bytes_and(a: i64, b: i64) -> i64
kata_rt_bytes_or(a: i64, b: i64) -> i64
kata_rt_bytes_xor(a: i64, b: i64) -> i64
kata_rt_bytes_not(a: i64) -> i64

// Bitwise (escalar)
kata_rt_byte_and(a: i64, b: i64) -> i64
kata_rt_byte_or(a: i64, b: i64) -> i64
kata_rt_byte_xor(a: i64, b: i64) -> i64
kata_rt_byte_not(a: i64) -> i64
kata_rt_byte_shr(a: i64, n: i64) -> i64
kata_rt_byte_shl(a: i64, n: i64) -> i64

// Conversões
kata_rt_byte_to_int(b: i64) -> i64            // já é SMI, só untag/tag
kata_rt_int_to_byte(n: i64) -> i64            // mod 256, tag como SMI
kata_rt_int_to_bytes(n: i64) -> i64           // 4 bytes little-endian
kata_rt_text_to_bytes(ptr: i64) -> i64        // codifica UTF-8
kata_rt_text_to_bytes_enc(ptr: i64, enc: i64) -> i64  // codifica com encoding
kata_rt_bytes_to_text(ptr: i64) -> i64        // decodifica UTF-8 (Result)
kata_rt_bytes_to_text_enc(ptr: i64, enc: i64) -> i64 // decodifica com encoding (Result)
kata_rt_text_at(ptr: i64, idx: i64) -> i64    // codepoint em idx (Result)
kata_rt_text_len(ptr: i64) -> i64             // número de codepoints
```

### 8.2. Arena

`Bytes` é alocado na mesma arena do contexto (fiber_arena, caller_arena, ou
root_arena para dados que escapam). O header (len) e os dados (u8[]) são
alocados num único bloco contíguo na arena.

## 9. Fases de implementação

### Fase 1: Tipo e runtime

**kata-core:**
- `Ty::Bytes` no enum
- `type_name_str`, `ty_to_clif` (→ I64 ptr), `to_shape` para `Bytes`

**kata-rt:**
- `bytes.rs`: struct `Bytes`, FFI functions de alocação e acesso
- `kata_rt_bytes_alloc/get/set/len/concat/eq/show`

**DoD Fase 1:** Runtime cria blob, acessa bytes, concatena. Testes unitários.

### Fase 2: Lexer e parser

**kata-lexer:**
- Tokens `Shr` (`>>`) e `Shl` (`<<`)
- Literal `b"..."` (byte string) — lexer produz `Expr::BytesLit`

**kata-parser:**
- `>>` e `<<` no loop de pós-aplicação (mesma camada que `|`, `|>`)
- `BytesLit` na tabela de atoms

**DoD Fase 2:** `kata parse` de programa com `b"Hello"`, `>>`, `<<`
produz AST correta. Snapshots.

### Fase 3: Typeck e interfaces

**kata-inference:**
- Typeck de `DotAccess` em `Bytes` → `Result::(Byte, Text)`
- Typeck de `+` em `Bytes Bytes` → `Bytes`
- Typeck de `and/or/xor/not` em `Byte` e `Bytes`
- Typeck de `>>/<<` em `Byte Int` → `Byte`
- Conversões `int()`, `byte()`, `bytes()`, `text()` para `Bytes`/`Byte`
- Conversões `bytes()` / `text()` com `Encoding` (sobrecarga de aridade)
- Síntese de `show` para `Bytes` (hex)
- `Byte` como tipo distinto — `Ty::Byte` (variante dedicada, não PrimTy)
- `Text` implementa `INDEXABLE::Text`, `COUNTABLE`, `SLICEABLE::Text`
- `Bytes` implementa `INDEXABLE::Byte`, `COUNTABLE`, `SLICEABLE::Bytes`
- `Array::A` e `List::A` implementam `SLICEABLE`

**kata-core:**
- `Byte` no enum Ty — variante dedicada (não PrimTy)
- `Bytes` no enum Ty — variante dedicada (não PrimTy)
- `Encoding` no enum registry (enum do prelude)

**DoD Fase 3:** Typeck rejeita `b.0 + 1` (Result + Int). Aceita
`int(b.0 | byte(0)) + 1` (Int + Int). Aceita `+ b1 b2` (Bytes + Bytes).
Sobrecargas de `and/or/xor/not` resolvem por tipo. `t.0` retorna
`Result::(Text, Text)`. `len(t)` retorna `Int` (codepoints).

### Fase 4: Codegen

**kata-codegen:**
- Lowering de `BytesLit` → `kata_rt_bytes_from_ptr` com dados embutidos
- Lowering de `DotAccess` em `Bytes` → `kata_rt_bytes_get` (retorna Result)
- Lowering de `DotAccess` em `Text` → `kata_rt_text_at` (retorna Result)
- Lowering de `+` em `Bytes` → `kata_rt_bytes_concat`
- Lowering de `len` em `Text` → `kata_rt_text_len` (codepoints)
- Lowering de `len` em `Bytes` → `kata_rt_bytes_len`
- Lowering de slice `.[start..end]` → `kata_rt_*_slice`
- Lowering de bitwise → FFIs correspondentes
- Lowering de conversões → FFIs correspondentes
- Lowering de conversões com `Encoding` → FFIs correspondentes

**DoD Fase 4:** Programa com `Bytes` compila e executa. `let b := b"Hello"
echo!(show(b.0 | byte(0)))` imprime `0x48`. `+ b b` produz 10 bytes.
`t.0 | "?"` retorna primeiro codepoint. `len("Hello")` retorna `5`.

### Fase 5: to_bytes() / from_bytes() ✅ Concluído

**kata-rt:**
- `kata_rt_to_bytes(value_ptr, type_id, arena_handle) -> bytes_ptr` — serializa
  valor em blob `Bytes` com header estendido (data_len + type_id + rebase_count +
  rebase_offsets + data). Reaproveita mecânica de `HeapSnapshotData` (main +
  appended, rebasing de ponteiros relativos).
- `kata_rt_from_bytes(bytes_ptr, arena_handle) -> value_ptr` — reconstrói o
  valor na arena destino lendo type_id e rebase_offsets do header do blob.
- `TypeShape` (em `kata-rt`, não `kata-core`) — projeção runtime de `Ty` com
  fields/variants completos. Type table TLS indexada por `type_id`, registrada
  Rust-to-Rust pelo driver via `kata_rt::register_type_table()`.
- 2 FFIs registradas nos 7 pontos de toque do codegen (FfiSymbol::ToBytes /
  FromBytes).
- 4 testes unitários: roundtrip Int, Text, Tuple(Int,Text), List<Int>.

**Design decisions:**
- `TypeShape` vive em `kata-rt` (não `kata-core`) para manter o isolamento do
  runtime. O driver converte `kata_core::Ty` → `kata_rt::TypeShape` (preenchendo
  dos registries) antes do JIT.
- O blob carrega `type_id` e `rebase_offsets` no header — `from_bytes` não
  precisa de informação externa para reconstruir.
- `to_bytes` recebe `type_id` (não `type_shape_ptr`) porque `TypeShape` tem
  `String`/`Vec`/`Box` — sem layout C-ABI estável.

**Commit:** `bfcf1a4`

**DoD Fase 5:** ✅ `to_bytes` serializa Int, Text, Tuple, List. `from_bytes`
reconstrói na arena destino. Testes unitários passam.

### Fase 6: Testes E2E ✅ Concluído

- Acesso indexado: `b.0`, `b.(-1)` (retorna Result, usa `|` fallback)
- Text indexado: `t.0`, `t.(-1)` (retorna Result, codepoint como Text)
- Concatenação: `+ b1 b2`
- Bitwise: `and`, `or`, `xor`, `not`, `>>`, `<<`
- Conversões: `int(b.0 | byte(0))`, `byte(72)`, `bytes(42)`
- Text ↔ Bytes: `bytes("oi")`, `text(b"Hello")`, `text(b"Hello", Encoding::Ascii)`
- Encodings: UTF-8 (padrão), UTF-16, Latin-1, ASCII (rejeita > 0x7F)
- to_bytes/from_bytes roundtrip
- Show: `show(b)` produz hex
- EQ: `== b1 b2`
- len: `len(b)` (bytes), `len(t)` (codepoints)
- Slice: `b.[1..3]`, `t.[0..4]`

**DoD Fase 6:** ✅ 45 testes E2E passando. Cobertura: BytesLit, indexação
(positiva/negativa/out-of-bounds), concatenação, len, slice, show (hex),
eq (`=`), bitwise Byte (and/or/xor/not), bitwise Bytes (elemento-a-elemento),
conversões (int/byte/bytes), Text ↔ Bytes, Text indexável (at/len/slice),
roundtrips (text→bytes→index, int→bytes→index→int, byte→int→byte→int).

Bug crítico descoberto e corrigido: SMI tagging em funções de indexação.
O codegen passa índices como SMI-tagged `(val << 1) | 1`, mas as funções de
runtime tratavam `idx` como valor bruto. Corrigido com `untag_smi()` em
todas as funções de indexação/slice de Bytes, Text, Array, e List.

## 10. Não faz parte deste PRD

- `spawn!` (Fio 11 — Fase 9 do PRD-fio11) — consome `Bytes` via `serialized:`
- I/O de arquivo/socket (futuro — `Bytes` é o tipo, mas leitura/escrita é fio separado)
- Grapheme clusters (indexação é por codepoint; clusters de combining marks são fio separado)
- Mutabilidade de Bytes (decidido: imutável)
- Encodings além de UTF-8/UTF-16/Latin-1/ASCII (exigiria `encoding_rs`)

## 11. Dependências

| Fio | Status | Relação |
|---|---|---|
| Fio 11 (CSP) | ✅ exceto spawn! | `spawn!` consome `Bytes` via `serialized:`. `to_bytes()` retorna `Bytes` |
| Fio 8 (coleções) | ✅ | `Bytes` segue modelo de `Array`/`Text`. `SLICEABLE` estende coleções existentes |
| Fio 9 (closures/escape) | ✅ | `TypeShape` para `Bytes` — blob é leaf, sem ponteiros internos |
| Fio 12 (comptime) | ✅ | `HeapSnapshotData` mecânica reusada por to_bytes/from_bytes |

## 12. Relação com spawn!

Este PRD é **pré-requisito** para `spawn!` (Fase 9 do PRD-fio11):

- `to_bytes(value) -> Bytes` — produz o blob convertido
- `spawn!{callee: tarefa, serialized: payload}` — `payload :: Bytes`
- `from_bytes(bytes) -> value` — reconstrói na arena destino

Sem `Bytes`, `to_bytes()` não tem tipo de retorno. Sem `to_bytes()`, `spawn!`
não tem forma explícita de pré-serializar. O PRD-fio11 define a semântica de
`spawn!`; este PRD define o tipo que o transporte usa.

## 13. Relação com I/O de arquivos

Este PRD cria o tipo `Bytes`. I/O de arquivos (futuro) consome `Bytes`:

- `read_file!(path) -> Bytes` — lê arquivo binário
- `write_file!(path, bytes) -> Unit` — escreve bytes em arquivo
- `read_file_text!(path) -> Text` — lê arquivo como texto (delega para `bytes_to_text`)

O I/O é um fio separado que depende deste PRD.