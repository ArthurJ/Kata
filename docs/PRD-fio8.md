# PRD — Fio 8: Coleções, ITERABLE, Stream Fusion

## Visão

Fio 8 introduz coleções (List, Array, Range), interfaces de coleção
(ITERABLE, COUNTABLE, INDEXABLE, CONTAINS), iteração (`for x in`), operações
high-order (`map`, `filter`, `fold` via `@builtin`), stream fusion, e
operador de membership (`in`).

Constrói sobre Fio 7 (interfaces, generics, dispatch) e Fio 5 (Tuple
para special case de `len` e `.N`).

## Sintaxe

### Delimitadores de coleção

| Sintaxe | Tipo | Layout | Exemplo |
|---|---|---|---|
| `[1 2 3]` | `List(T)` | Encadeada (Cons) | `[1 2 3]` → `List(Int)` |
| `{1 2 3}` | `Array(T)` | Contíguo (imutável) | `{1 2 3}` → `Array(Int)` |
| `[0..1..10]` | `Range(A)` | Lazy (start, step, end exclusive) | `[0..1..10]` → `Range(Int)` (0 1 2 3 4 5 6 7 8 9) |
| `[0..1..=10]` | `Range(A)` | Lazy (start, step, end inclusive) | `[0..1..=10]` → `Range(Int)` (0 1 2 3 4 5 6 7 8 9 10) |
| `[0..2..10]` | `Range(A)` | Lazy (com step 2, exclusive) | `[0..2..10]` → `Range(Int)` (0, 2, 4, 6, 8) |
| `[0..2..=10]` | `Range(A)` | Lazy (com step 2, inclusive) | `[0..2..=10]` → `Range(Int)` (0, 2, 4, 6, 8, 10) |
| `[0.0..0.1..1.0]` | `Range(A)` | Lazy (Float, exclusive) | `[0.0..0.1..1.0]` → `Range(Float)` |
| `[]` | `List(T)` | Nil (vazia) | `[]` → `List(InferVar)` |
| `{}` | `Array(T)` | Vazio | `{}` → `Array(InferVar)` |
| `[h : t]` | Pattern Cons | Match em List | `match lst [h : t]: ...` |

**Range sempre exige passo explícito:** `[start..step..end]`. O fim pode ser
exclusive (`..end`, condição `current >= end`) ou inclusive (`..=end`,
condição `current > end`).

Range é genérico: `Range(A)` onde A implementa `Add` e `Ord` (ou as
interfaces que Kata usar para `+` e `>=`). O typeck valida isso estaticamente.

### Keywords novas

| Keyword | Uso |
|---|---|
| `for` | `for x in colecao` — iteração via ITERABLE |

### Operadores novos

| Token | Sintaxe | Uso |
|---|---|---|
| `DotDot` | `..` | Separador de componentes em Range: `[a..s..b]` |
| `DotDotEq` | `..=` | Separador de end inclusivo em Range: `[a..s..=b]` |
| `In` | `in` | Separador em `for x in coll` e operador binário de membership: `x in coll` |

### `for x in`

```kata
# Em Actions (statement — como loop):
action soma_array (arr) -> Int
    var total := 0
    for x in arr
        total := + total x
    return total

# Em funções puras: PROIBIDO. `for` em lambda = compile error.
# Funções puras iteram via map/filter/fold ou recursão.
```

`for x in colecao` desugara para iteração via `ITERABLE::next`:
- Chama `next colecao` → `Optional(A)`
- `Some(v)` → bind `x := v`, executa body, repete
- `None` → termina

`break` e `continue` funcionam em `for` (mesmo que em `loop`).

**`for` em lambda = compile error.** `for` só existe em Action body (como
`loop`). Tentar usar `for` dentro de uma lambda ou função pura produz erro
de compilação. Não há `for` como expressão em função pura — iteração é via
`map`/`filter`/`fold` ou recursão.

### Operador `in` (membership)

```kata
# `x in coll` — operador binário, retorna Boolean
# Desugara para `contains coll x` → dispatch via CONTAINS
3 in {1 2 3}              # true
5 in [1 2 3]              # false
5 in [0..1..1000000]      # true — Range é O(1): arithmetic check
"ell" in "hello"          # true — substring check
```

`in` tem precedência de comparação (mesmo nível de `==`, `>`).

O parser distingue os dois usos de `in` pela posição:
- Depois de `for <ident>`: `in` é separador (não produz expressão)
- Em contexto de expressão: `in` é operador binário

### `.N` em coleções

```kata
let arr := {1 2 3}
arr.0          # desugar → at arr 0      → Result::(Int, Err)
arr.(-1)       # desugar → at arr (-1)   → Result::(Int, Err), runtime resolve
arr.0 ?        # desugar → (at arr 0) ?   → Int (panic se Err)
arr.0 | 0     # desugar → (at arr 0) | 0 → Int (fallback)

let lst := [1 2 3]
lst.1 ?        # desugar → (at lst 1) ?   → Int
```

`.N` em coleções é syntactic sugar para `at` (interface INDEXABLE).
O typeck faz o desugar baseado no tipo do receptor:
- `Tuple` → IndexAccess compile-time (direto, sem Result)
- Implementa `INDEXABLE` → desugar para `at obj N` (retorna Result)
- Outro → `NotIndexable` (type error)

**Decisão: `.N` em coleções é checked por padrão** (retorna `Result`).
O caminho feliz exige `?` (unwrap, panic em Err) ou `| default` (fallback).
Isso faz o tipo avisar que o access pode falhar — coerente com "estados
inválidos irrepresentáveis". Para Tuple, `.0` é access compile-time direto
(sem Result) porque o tamanho é conhecido e o access é infalível.

### `len`

```kata
len {1 2 3}              # 3 — Array (COUNTABLE dispatch, kata_rt_array_len)
len [1 2 3]              # 3 — List (COUNTABLE dispatch, kata_rt_list_len)
len "hello"              # 5 — Text (COUNTABLE dispatch, kata_rt_string_len)
len (10, 20)             # 2 — Tuple (síntese compile-time, special case)
```

`len` em Tuple é special case do typeck (síntese: `IntLit(elements.len())`).
`len` em coleções é dispatch via interface `COUNTABLE`.

## Tipos

### `Ty` — novos variants

```rust
pub enum Ty {
    // ... existentes ...
    /// Lista persistente: `[T]` — Cons cell.
    List(Box<Ty>),
    /// Array contíguo: `{T}` — bloco imutável.
    Array(Box<Ty>),
    /// Range lazy: `[a..s..b]` — start, step, end. Genérico sobre A.
    Range(Box<Ty>),
}
```

**Decisão: variants intrínsecos de `Ty`, não `data` opacos.**
- List/Array/Range são tipos estrututais do compilador, não declarados pelo
  usuário. O codegen precisa saber o layout (Cons cell vs contíguo vs lazy),
  e isso é informação de `Ty`, não de `@ffi`.
- O resolution reconhece os nomes `List`, `Array`, `Range` como tipos
  intrínsecos (não precisam de `data` no prelude).
- `List(A)` é `Ty::List(Box::new(Ty::Prim(Int)))` — o type param é o tipo
  do elemento. `Generic("List", [Int])` não é usado — `List` é intrínseco,
  não um enum genérico como `Result`.
- `Range(A)` é `Ty::Range(Box::new(A))` — A é o tipo do elemento, genérico.
  A deve implementar as interfaces necessárias para `+` e `>=`.

### InterfaceRegistry — interfaces de coleção

Novas interfaces no prelude (`stdlib/core.kata`):

```kata
interface ITERABLE(A)
    next :: Self => Optional::(A)

interface COUNTABLE
    len :: Self => Int

interface INDEXABLE(A)
    at :: Self Int => Result::(A, Err)

interface CONTAINS(A)
    contains :: Self A => Boolean
```

### Implementações

| Tipo | ITERABLE | COUNTABLE | INDEXABLE | CONTAINS |
|---|---|---|---|---|
| `Array(A)` | ✅ (kata_rt_array_next) | ✅ (kata_rt_array_len) | ✅ (kata_rt_array_get_checked) | ✅ (linear scan) |
| `List(A)` | ✅ (kata_rt_list_next) | ✅ (kata_rt_list_len) | ✅ (kata_rt_list_get_checked) | ✅ (linear scan) |
| `Text` | ✅ (kata_rt_string_next) | ✅ (kata_rt_string_len) | ✅ (kata_rt_string_get_checked) | ✅ (substring search) |
| `Range(A)` | ✅ (codegen inline) | ✅ (compile-time) | — | ✅ (O(1) arithmetic) |
| `Tuple` | — | special case (síntese) | special case (compile-time) | — |

Tuple não implementa interfaces — é tipo estrutural, não nominal.

**Extensibilidade via interface:** O typeck consulta o InterfaceRegistry
para determinar se um tipo implementa ITERABLE, COUNTABLE, INDEXABLE, ou
CONTAINS. Tipos definidos pelo usuário que implementam essas interfaces
recebem `for x in`, `len`, `.N`, e `in` automaticamente — sem hardcoded
pattern-match em variants de `Ty`.

## AST — novos nós

### `Expr` (kata-ast/src/expr.rs)

```rust
pub enum Expr {
    // ... existentes ...

    /// `[1 2 3]` — lista literal (Cons cells).
    ListLit { elements: Vec<Spanned<Expr>> },

    /// `{1 2 3}` — array literal (contíguo).
    ArrayLit { elements: Vec<Spanned<Expr>> },

    /// `[a..s..b]` ou `[a..s..=b]` — range lazy. Step é sempre explícito.
    RangeLit {
        start: Box<Spanned<Expr>>,
        step: Box<Spanned<Expr>>,
        end: Box<Spanned<Expr>>,
        /// true = `..=` (inclusive), false = `..` (exclusive)
        inclusive: bool,
    },

    /// `for x in colecao` — iteração via ITERABLE.
    /// `var_name` é o binding do elemento.
    /// `body` é o corpo do loop.
    ForIn {
        var_name: String,
        iterable: Box<Spanned<Expr>>,
        body: Vec<Spanned<Expr>>,
    },

    /// `x in coll` — operador de membership (dispatch via CONTAINS).
    In {
        item: Box<Spanned<Expr>>,
        collection: Box<Spanned<Expr>>,
    },
}
```

### `TypedExprKind` (kata-inference/src/typed.rs)

```rust
pub enum TypedExprKind {
    // ... existentes ...

    /// `[1 2 3]` — lista literal. `elem_ty` é o tipo unificado dos elementos.
    ListLit { elements: Vec<Spanned<TypedExpr>> },

    /// `{1 2 3}` — array literal.
    ArrayLit { elements: Vec<Spanned<TypedExpr>> },

    /// `[a..s..b]` ou `[a..s..=b]` — range lazy. `elem_ty` é o tipo do elemento (genérico).
    RangeLit {
        start: Box<Spanned<TypedExpr>>,
        step: Box<Spanned<TypedExpr>>,
        end: Box<Spanned<TypedExpr>>,
        /// true = `..=` (inclusive), false = `..` (exclusive)
        inclusive: bool,
        elem_ty: Ty,
    },

    /// `for x in colecao` — iteração via ITERABLE.
    ForIn {
        var_name: String,
        var_ty: Ty,
        iterable: Box<Spanned<TypedExpr>>,
        body: Vec<Spanned<TypedExpr>>,
    },

    /// `x in coll` — membership via CONTAINS.
    In {
        item: Box<Spanned<TypedExpr>>,
        collection: Box<Spanned<TypedExpr>>,
    },

    /// `map f colecao` — @builtin("map") → nó TAST especializado.
    /// Stream fusion: Map(Filter(arr)) → único loop.
    Map {
        func: Box<Spanned<TypedExpr>>,
        iterable: Box<Spanned<TypedExpr>>,
        /// Tipo do elemento de saída.
        out_ty: Ty,
    },

    /// `filter f colecao` — @builtin("filter") → nó TAST especializado.
    Filter {
        func: Box<Spanned<TypedExpr>>,
        iterable: Box<Spanned<TypedExpr>>,
    },

    /// `fold f init colecao` — @builtin("fold") → nó TAST especializado.
    Fold {
        func: Box<Spanned<TypedExpr>>,
        init: Box<Spanned<TypedExpr>>,
        iterable: Box<Spanned<TypedExpr>>,
        /// Tipo do acumulador.
        acc_ty: Ty,
    },
}
```

## Runtime

### `kata_rt_list_*` (crates/kata-rt/src/list.rs — NOVO)

```c
// Cons cell: 2 words (head, tail). Nil = null pointer (0).
// List é sempre heap type (ponteiro).

// Construtor: [1 2 3] = cons(1, cons(2, cons(3, nil)))
kata_rt_list_nil() -> ptr     // retorna 0 (null)
kata_rt_list_cons(head: i64, tail: ptr) -> ptr
kata_rt_list_is_empty(lst: ptr) -> i64
kata_rt_list_head(lst: ptr) -> i64
kata_rt_list_tail(lst: ptr) -> ptr
```

Layout: Cons cell = `{ head: i64, tail: ptr }` (16 bytes na arena).

### `kata_rt_array_*` (crates/kata-rt/src/array.rs — NOVO)

```c
// Array contíguo: header (len: i64) + data (len * 8 bytes).
// Array é sempre heap type (ponteiro).

kata_rt_array_alloc(len: i64) -> ptr      // aloca header + data
kata_rt_array_len(arr: ptr) -> i64
kata_rt_array_get(arr: ptr, idx: i64) -> i64
kata_rt_array_set(arr: ptr, idx: i64, val: i64)
kata_rt_array_get_checked(arr: ptr, idx: i64) -> i64   // retorna tag do Result
```

Layout: `{ len: i64, data: [i64; len] }` (header 8 bytes + data).

### `kata_rt_range_*` (crates/kata-rt/src/range.rs — NOVO)

```c
// Range lazy: 3 words (start, step, end). Genérico sobre A.
// O runtime só aloca o struct. As operações de next (+) e done (>= ou >)
// são inlined pelo codegen com base no tipo concreto de A e no flag inclusive.
// Range é heap type (ponteiro).

kata_rt_range_alloc() -> ptr   // aloca 3 words na arena
```

Layout: `{ start: i64, step: i64, end: i64 }` (24 bytes).

**O codegen inlined as operações de `+` (next) e `>=` (done) com base no
`elem_ty` do `RangeLit`.** Para `Range(Int)`, gera `kata_rt_add_int` /
`kata_rt_compare_ge_int`. Para `Range(Float)`, gera `kata_rt_add_float` /
`kata_rt_compare_ge_float`. Para tipos do usuário, gera as chamadas de
dispatch apropriadas. O runtime não interpreta bits — o tipo é conhecido
estaticamente no TAST.

## Codegen

### Lowering de ListLit

```
[1 2 3]
→
arena_alloc(16)  // cons(3, nil)
arena_alloc(16)  // cons(2, prev)
arena_alloc(16)  // cons(1, prev)
```

Constrói de trás para frente: `nil → cons(3, nil) → cons(2, ...) → cons(1, ...)`.

### Lowering de ArrayLit

```
{1 2 3}
→
ptr = kata_rt_array_alloc(3)
kata_rt_array_set(ptr, 0, 1)
kata_rt_array_set(ptr, 1, 2)
kata_rt_array_set(ptr, 2, 3)
```

### Lowering de RangeLit

```
[0..2..10]
→
ptr = kata_rt_range_alloc()
store(ptr, 0, 0)    // start
store(ptr, 8, 2)    // step
store(ptr, 16, 10)  // end
```

O codegen armazena start, step, end no struct alocado. As operações de
iteração (`next` = `current + step`, `done` = `current >= end`) são inlined
com chamadas específicas ao tipo concreto de A.

### Lowering de ForIn

```
for x in arr
    body
→
loop:
    opt = next(arr)
    match opt
        Some(v) → x = v; body; goto loop
        None    → done
```

Desugara para loop + match em Optional. Reusa a maquinaria de `loop`/`break`/
`continue` já existente (Fio 3 Fase 4).

### Lowering de `in` (membership)

```
x in coll
→
contains(coll, x)    // dispatch via CONTAINS
```

Desugar no typeck (não no codegen): `x in coll` → `contains coll x` →
dispatch CONTAINS.

### Lowering de `.N` em coleções

Desugar no typeck (não no codegen): `arr.0` → `at arr 0` → dispatch INDEXABLE.

### Lowering de `len`

Desugar no typeck: `len tuple` → síntese compile-time; `len coll` →
dispatch COUNTABLE.

### Lowering de map/filter/fold

Nós TAST especializados — não passam pelo dispatch normal.

```
map f arr
→
loop com ITERABLE::next:
    opt = next(arr)
    match opt
        Some(v) → f(v); cons na lista de saída
        None    → retorna lista
```

Stream fusion: `map f (filter g arr)` → único loop com ambos predicados.

## Inference

### Tipagem de ListLit

```
[1 2 3]
→
infer cada elemento: Int, Int, Int
unificar: todos do mesmo tipo T
resultado: Ty::List(Box::new(Int))
```

Elementos de tipos diferentes → type error (List é homogênea).

### Tipagem de ArrayLit

Mesma lógica de ListLit: `Ty::Array(Box::new(T))`.

### Tipagem de RangeLit

```
[0..1..10]      → Ty::Range(Box::new(Int))
[0.0..0.1..1.0] → Ty::Range(Box::new(Float))
```

Range é genérico: `Range(A)`. Start, step, end devem ser do mesmo tipo A.
A deve implementar as interfaces necessárias para `+` (Add) e `>=` (Ord) —
type error caso contrário. Step é sempre explícito na sintaxe.

### Tipagem de ForIn

```
for x in colecao
→
infer colecao: T
consultar InterfaceRegistry: T implementa ITERABLE(A)? Qual é A?
definir x: A no escopo do body
infer body
```

**O typeck extrai A consultando o InterfaceRegistry**, não fazendo
pattern-match em variants de `Ty`. Isso permite que tipos definidos pelo
usuário que implementam ITERABLE funcionem com `for x in`.

Exemplos (ilustrativos, não exaustivos):
- `List(A)` implementa `ITERABLE(A)` → A = elemento
- `Array(A)` implementa `ITERABLE(A)` → A = elemento
- `Range(A)` implementa `ITERABLE(A)` → A = tipo do range
- `MinhaColecao(A) implements ITERABLE(A)` → A = elemento (tipo do usuário)

### Tipagem de `in` (membership)

```
x in coll
→
infer coll: T
infer x: B
consultar InterfaceRegistry: T implementa CONTAINS(B)?
resultado: Boolean
```

### Pattern Cons ganha semântica

```
match lst
    [h : t]: ...
→
h = head(lst)
t = tail(lst)
```

`TypedPattern::Cons` agora faz match em `List(A)`:
- `head` recebe tipo `A`
- `tail` recebe tipo `List(A)`

### `len` — dispatch vs síntese

- `len tuple` → síntese compile-time: `IntLit(elements.len())` (special case)
- `len colecao` → dispatch via COUNTABLE: procura `len :: Self => Int`

### `.N` — dispatch vs compile-time

- `len tuple` → síntese compile-time: `IntLit(elements.len())` (special case)
- `len colecao` → dispatch via COUNTABLE: procura `len :: Self => Int`

## Fases de implementação

### Fase 1: Tipos + Lexer + Tokens

**Arquivos:**
- `crates/kata-core/src/ty.rs` — adicionar `Ty::List(Box<Ty>)`, `Ty::Array(Box<Ty>)`, `Ty::Range(Box<Ty>)`
- `crates/kata-ast/src/token.rs` — adicionar `Token::For`, `Token::DotDot`, `Token::DotDotEq`, `Token::In`
- `crates/kata-lexer/src/ident.rs` — adicionar `"for" => Token::For`, `"in" => Token::In`
- `crates/kata-lexer/src/dispatch.rs` — lexar `..` → `DotDot`, `..=` → `DotDotEq` (no branch do `.`)
- `crates/kata-ast/src/token.rs` — `is_keyword()` + `Display`

**Nota:** `Token::DotDotEq` (`..=`) marca o fim inclusive do Range. A sintaxe
completa é `[start..step..end]` (exclusive) ou `[start..step..=end]` (inclusive).

**Verificação:** `cargo check --workspace` passa. `kata lex "for x in [0..1..10]"` produz tokens corretos.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 1 | `Ty::List`, `Ty::Array`, `Ty::Range(Box<Ty>)` existem e implementam `Hash + Eq + Clone` | 1 |
| 2 | `Token::For` é produzido pelo lexer para `for` | 1 |
| 3 | `Token::DotDot` é produzido para `..` e `Token::DotDotEq` para `..=` | 1 |
| 4 | `Token::In` é produzido para `in` | 1 |

### Fase 2: Parser — List/Array/Range literals

**Arquivos:**
- `crates/kata-ast/src/expr.rs` — adicionar `Expr::ListLit`, `Expr::ArrayLit`, `Expr::RangeLit`
- `crates/kata-parser/src/expressions.rs` — `parse_expr_atom` trata `LBracket` e `LBrace`
- `crates/kata-parser/src/lib.rs` ou novo arquivo — `parse_list_lit`, `parse_array_lit`, `parse_range_lit`

**Parsing de `[`:**
- `[` → pode ser ListLit (`[1 2 3]`) ou RangeLit (`[0..1..10]`)
- Após parsear primeiro elemento, se próximo token é `..`, é Range
- Range tem exatamente 3 componentes: `start .. step .. end` (ou `..= end`)
- Se o segundo `..` for `..=`, `inclusive = true`; senão `inclusive = false`
- Caso contrário, continua coletando elementos para ListLit
- `[]` → lista vazia (ListLit com 0 elementos)

**Parsing de `{`:**
- `{` → ArrayLit (`{1 2 3}`)
- `{}` → array vazio
- Cuidado: `{` em diretivas (`@nome{...}`) é tratado no parser de diretivas,
  não em `parse_expr_atom`. Se `parse_expr_atom` só é chamado quando o parser
  espera uma expressão, não há conflito. **Documentar esse invariant no parser.**

**Verificação:** `kata parse "[1 2 3]"` produz AST com `ListLit`. `kata parse "{1 2 3}"` produz `ArrayLit`. `kata parse "[0..1..10]"` produz `RangeLit`.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 5 | `kata parse "[1 2 3]"` produz `Expr::ListLit` com 3 elementos | 2 |
| 6 | `kata parse "{1 2 3}"` produz `Expr::ArrayLit` com 3 elementos | 2 |
| 7 | `kata parse "[0..1..10]"` produz `Expr::RangeLit` com start=0, step=1, end=10, inclusive=false | 2 |
| 8 | `kata parse "[0..1..=10]"` produz `Expr::RangeLit` com start=0, step=1, end=10, inclusive=true | 2 |
| 9 | `kata parse "[0..2..10]"` produz `Expr::RangeLit` com start=0, step=2, end=10, inclusive=false | 2 |
| 10 | `kata parse "[0..2..=10]"` produz `Expr::RangeLit` com start=0, step=2, end=10, inclusive=true | 2 |
| 11 | `kata parse "[0.0..0.1..1.0]"` produz `Expr::RangeLit` com elementos Float | 2 |
| 12 | `kata parse "[]"` produz `Expr::ListLit` com 0 elementos | 2 |

### Fase 3: Parser — `for x in` + operador `in`

**Arquivos:**
- `crates/kata-ast/src/expr.rs` — adicionar `Expr::ForIn`, `Expr::In`
- `crates/kata-parser/src/expressions.rs` ou novo arquivo — `parse_for_in`, `parse_in_op`
- `crates/kata-parser/src/lib.rs` — `For` no `parse_expr_atom`

**Sintaxe:**
```
for x in colecao
    body1
    body2
```

`for` é seguido de: identificador (`x`), `in` (token `Token::In`), expressão
iterável, e body indentado.

**`in` como operador binário:** Em contexto de expressão (não após `for`),
`in` é operador binário com precedência de comparação. `x in coll` produz
`Expr::In`.

**`for` em Actions vs funções puras:** `for` só é aceito em Action body
(como `loop`). `for` em lambda ou função pura = **compile error**.

**Verificação:** `kata parse "for x in arr\n    echo!(x)"` produz `Expr::ForIn`. `kata parse "3 in {1 2 3}"` produz `Expr::In`.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 13 | `for x in arr` parseia e produz `Expr::ForIn` | 3 |
| 14 | `for` só é aceito em Action body (erro fora de Action, como `loop`) | 3 |
| 15 | `for` em lambda produz compile error | 3 |
| 16 | `3 in {1 2 3}` parseia e produz `Expr::In` | 3 |

### Fase 4: Resolution — interfaces no prelude

**Arquivos:**
- `stdlib/core.kata` — adicionar interfaces ITERABLE, COUNTABLE, INDEXABLE, CONTAINS
- `crates/kata-core/src/ty.rs` — reconhecer `List`, `Array`, `Range` como tipos intrínsecos
- `crates/kata-resolution/src/` — resolver `List(A)`, `Array(A)`, `Range(A)` como `Ty::List(A)`, etc.

**Prelude adiciona:**
```kata
interface ITERABLE(A)
    next :: Self => Optional::(A)

interface COUNTABLE
    len :: Self => Int

interface INDEXABLE(A)
    at :: Self Int => Result::(A, Err)

interface CONTAINS(A)
    contains :: Self A => Boolean
```

**Tipos intrínsecos no resolution:** Quando o resolver encontra `List(Int)`,
produz `Ty::List(Box::new(Ty::Prim(Int)))`. Não procura `List` como `data`
ou `enum` — é tipo intrínseco do compilador (como `Tuple`). O mesmo para
`Array(A)` e `Range(A)`.

**Implementações de coleção no prelude:** Array, List, Range, e Text
implementam as interfaces apropriadas. Como List/Array/Range são intrínsecos
(não `data`), as implementações usam FFI direto:

```kata
# Array implementa ITERABLE, COUNTABLE, INDEXABLE, CONTAINS
Array(A) implements ITERABLE(A)
    next :: Array(A) => Optional::(A) @ffi("kata_rt_array_next")

Array(A) implements COUNTABLE
    len :: Array(A) => Int @ffi("kata_rt_array_len")

Array(A) implements INDEXABLE(A)
    at :: Array(A) Int => Result::(A, Err) @ffi("kata_rt_array_get_checked")

Array(A) implements CONTAINS(A)
    contains :: Array(A) A => Boolean @ffi("kata_rt_array_contains")

# List implementa ITERABLE, COUNTABLE, INDEXABLE, CONTAINS
List(A) implements ITERABLE(A)
    next :: List(A) => Optional::(A) @ffi("kata_rt_list_next")

List(A) implements COUNTABLE
    len :: List(A) => Int @ffi("kata_rt_list_len")

List(A) implements INDEXABLE(A)
    at :: List(A) Int => Result::(A, Err) @ffi("kata_rt_list_get_checked")

List(A) implements CONTAINS(A)
    contains :: List(A) A => Boolean @ffi("kata_rt_list_contains")

# Range implementa ITERABLE, COUNTABLE, CONTAINS
# Range não implementa INDEXABLE (não faz sentido para Range lazy)
# ITERABLE e COUNTABLE são inlined pelo codegen, não FFI
Range(A) implements ITERABLE(A)
    next :: Range(A) => Optional::(A) @builtin("range_next")

Range(A) implements COUNTABLE
    len :: Range(A) => Int @builtin("range_len")

Range(A) implements CONTAINS(A)
    contains :: Range(A) A => Boolean @builtin("range_contains")

# Text implementa ITERABLE, COUNTABLE, INDEXABLE, CONTAINS
Text implements ITERABLE(Text)
    next :: Text => Optional::(Text) @ffi("kata_rt_string_next")

Text implements COUNTABLE
    len :: Text => Int @ffi("kata_rt_string_len")

Text implements INDEXABLE(Text)
    at :: Text Int => Result::(Text, Err) @ffi("kata_rt_string_get_checked")

Text implements CONTAINS(Text)
    contains :: Text Text => Boolean @ffi("kata_rt_string_contains")
```

**Range usa `@builtin` em vez de `@ffi`** para ITERABLE/COUNTABLE/CONTAINS
porque as operações são inlined pelo codegen com base no tipo concreto de A.
Não há uma única função FFI — o codegen gera código específico por tipo.

**Verificação:** `cargo test -p kata-resolution` passa. Interfaces registradas.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 17 | `ITERABLE(A)`, `COUNTABLE`, `INDEXABLE(A)`, `CONTAINS(A)` registradas no InterfaceRegistry | 4 | ✅
| 18 | `Array(A) implements ITERABLE(A)` registra no DispatchTable | 4 | ✅
| 19 | `List(A) implements ITERABLE(A)` registra no DispatchTable | 4 | ✅
| 20 | `Range(A) implements ITERABLE(A)` registra no DispatchTable | 4 | ✅
| 21 | `Text implements CONTAINS(Text)` registra no DispatchTable | 4 | ✅

### Fase 5: Inference — tipagem de coleções

**Arquivos:**
- `crates/kata-inference/src/typed.rs` — `TypedExprKind::ListLit`, `ArrayLit`, `RangeLit`, `ForIn`, `In`
- `crates/kata-inference/src/infer/` — inferência de ListLit, ArrayLit, RangeLit, ForIn, In
- `crates/kata-inference/src/patterns.rs` — Cons pattern ganha semântica (match em List)

**Inferência de ListLit:**
1. Inferir tipo de cada elemento
2. Unificar todos (erro se tipos diferentes)
3. Produzir `Ty::List(Box::new(elem_ty))`

**Inferência de ArrayLit:** Mesma lógica, `Ty::Array(Box::new(elem_ty))`.

**Inferência de RangeLit:**
- Start, step, end devem ser do mesmo tipo A
- A deve implementar as interfaces necessárias para `+` (Add) e `>=` (Ord)
- Type error se A não suporta essas operações
- `inclusive` é preservado do AST (não afeta o tipo, só a condição de parada)
- Produz `Ty::Range(Box::new(A))`

**Inferência de ForIn:**
1. Inferir tipo de `iterable`
2. Consultar InterfaceRegistry: o tipo implementa `ITERABLE(A)`? Qual é `A`?
3. Se não implementa → type error (não é ITERABLE)
4. Definir `var_name: A` no escopo
5. Inferir body

**O typeck extrai A via InterfaceRegistry lookup**, não pattern-match em
variants de `Ty`. Tipos do usuário que implementam ITERABLE funcionam.

**Inferência de `in` (membership):**
1. Inferir tipo de `collection`
2. Inferir tipo de `item`
3. Consultar InterfaceRegistry: o tipo da coleção implementa `CONTAINS(B)`?
4. Se não → type error
5. Resultado: `Boolean`

**Pattern Cons:**
- `TypedPattern::Cons { head, tail }` faz match em `Ty::List(A)`
- `head` recebe tipo `A`, `tail` recebe tipo `List(A)`

**`.N` em coleções (desugar no typeck):**
- Receptor é `List(A)` ou `Array(A)` → desugar para `at receptor N`
- `at` despacha via INDEXABLE → retorna `Result::(A, Err)`
- Receptor é `Tuple` → IndexAccess compile-time (direto, sem Result)
- Receptor é `Range` → type error (Range não implementa INDEXABLE)

**`len` (desugar no typeck):**
- Receptor é `Tuple` → síntese: `IntLit(elements.len())`
- Receptor implementa COUNTABLE → dispatch normal
- Outro → type error

**Verificação:** `cargo test -p kata-inference` passa. Testes unitários de inferência.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 22 | `[1 2 3]` infere `List(Int)` | 5 | ✅ |
| 23 | `{1 2 3}` infere `Array(Int)` | 5 | ✅ |
| 24 | `[0..1..10]` infere `Range(Int)` | 5 | ✅ |
| 25 | `[0..1..=10]` infere `Range(Int)` com `inclusive=true` | 5 | ✅ |
| 26 | `[0.0..0.1..1.0]` infere `Range(Float)` | 5 | ✅ |
| 27 | `[]` infere `List(InferVar)` — tipo resolvido pelo uso | 5 | ✅ |
| 28 | `for x in [1 2 3]` define `x: Int` no escopo do body | 5 | ✅ |
| 29 | `[h : t]` pattern match em List: `h: Int, t: List(Int)` | 5 | ✅ |
| 30 | `arr.0` em Array desugara para `at arr 0` → `Result::(Int, Err)` | 5 | ✅ |
| 31 | `len (10, 20)` → `2` (síntese compile-time) | 5 | ✅ |
| 32 | `3 in {1 2 3}` infere `Boolean` | 5 | ✅ |

### Fase 6: Runtime + Codegen

**Arquivos:**
- `crates/kata-rt/src/list.rs` — NOVO: kata_rt_list_nil/cons/is_empty/head/tail
- `crates/kata-rt/src/array.rs` — NOVO: kata_rt_array_alloc/len/get/set/get_checked
- `crates/kata-rt/src/range.rs` — NOVO: kata_rt_range_alloc (apenas alocação)
- `crates/kata-rt/src/lib.rs` — registrar novos FFI symbols
- `crates/kata-codegen/src/lowering/` — lowering de ListLit, ArrayLit, RangeLit, ForIn, In
- `crates/kata-codegen/src/lowering/expr.rs` — IndexAccess em coleções (desugar → at via INDEXABLE dispatch)
- `crates/kata-core/src/ffi.rs` — novos FfiSymbol variants

**Layout de memória:**
- Cons cell: `{ head: i64, tail: ptr }` — 16 bytes, arena alloc
- Array: `{ len: i64, data: [i64; len] }` — 8 + len*8 bytes, arena alloc
- Range: `{ start: i64, step: i64, end: i64 }` — 24 bytes, arena alloc
- Nil = null pointer (0)

**Lowering de ListLit `[1 2 3]`:**
1. Constrói de trás para frente: nil → cons(3) → cons(2) → cons(1)
2. Cada Cons cell: arena_alloc(16), store head, store tail
3. Resultado: ponteiro para o primeiro Cons

**Lowering de ArrayLit `{1 2 3}`:**
1. `kata_rt_array_alloc(3)` — aloca header + data
2. `kata_rt_array_set(ptr, 0, 1)` etc
3. Resultado: ponteiro para o array

**Lowering de RangeLit `[0..2..10]`:**
1. `kata_rt_range_alloc()` — aloca 3 words
2. Store start, step, end no struct
3. Se `inclusive=true`, o codegen gera condição `current > end` (em vez de `current >= end`)
4. Operações de next (+) e done (>= ou >) são inlined pelo codegen com
   chamadas específicas ao tipo concreto de A

**Lowering de ForIn:**
- Desugara para loop + match em Optional::next
- Reusa maquinaria de `loop`/`break`/`continue` existente

**Lowering de `in` (membership):**
- Desugara para `contains coll x` → dispatch via CONTAINS

**Verificação:** `cargo test --workspace --no-fail-fast` passa.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 33 | `kata_rt_list_nil` retorna 0 (null) | 6 | ✅ |
| 34 | `kata_rt_list_cons(head, tail)` aloca Cons cell na arena | 6 | ✅ |
| 35 | `kata_rt_array_alloc(len)` aloca array contíguo | 6 | ✅ |
| 36 | `kata_rt_array_get_checked(arr, idx)` retorna Result | 6 | ✅ |
| 37 | `kata_rt_range_alloc()` aloca struct Range (3 words) | 6 | ✅ |
| 38 | ListLit lowering produz Cons chain | 6 | ✅ |
| 39 | ArrayLit lowering produz array contíguo | 6 | ✅ |
| 40 | RangeLit lowering produz Range com operações inlined por tipo | 6 | ✅ |
| 41 | ForIn lowering desugara para loop + match | 6 | ✅ |
| 42 | `in` lowering desugara para dispatch CONTAINS | 6 | ✅ |

### Fase 7: Testes E2E

**Arquivo:** `crates/kata-codegen/tests/fio8_collections_e2e.rs` — NOVO

**Testes (DoDs do ROADMAP):**

```kata
# DoD: map (+ 10 _) [1 2 3] → [11 12 13]
# DoD: filter (> _ 5) {1 8 3 9} → {8 9}
# DoD: arr.0 ? desempacota
# DoD: len (10, 20) → 2 (compile-time)
# DoD: for x in {1 2 3 4 5} itera via ITERABLE
# DoD: 3 in {1 2 3} → true
# DoD: 5 in [0..2..10] → true (O(1) arithmetic)
# DoD: [0..1..=5] itera 0 1 2 3 4 5 (inclusive)
```

Testes E2E seguem o padrão dos testes existentes (concatenar prelude inline,
source + expressão, compilar + executar via JIT, verificar output).

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 43 | `[1 2 3]` executa e produz List(Int) | 7 | ✅ |
| 44 | `{1 2 3}` executa e produz Array(Int) | 7 | ✅ |
| 45 | `[0..1..10]` executa e produz Range(Int) | 7 | ✅ |
| 46 | `[0..1..=10]` executa e produz Range(Int) inclusive | 7 | ✅ |
| 47 | `[0.0..0.1..1.0]` executa e produz Range(Float) | 7 | ✅ |
| 48 | `+ (head [1 2 3]) 10` → `11` (head de List) | 7 | ✅ |
| 49 | `arr.0 ?` em `{1 2 3}` → `1` (index + unwrap) | 7 | ✅ |
| 50 | `len [1 2 3]` → `3` (COUNTABLE dispatch) | 7 | ✅ |
| 51 | `len {1 2 3}` → `3` (COUNTABLE dispatch) | 7 | ✅ |
| 52 | `len (10, 20)` → `2` (síntese compile-time) | 7 | ✅ |
| 53 | `match [1 2 3] [h : t]: + h (head t)` → `3` (pattern Cons) | 7 | ✅ |
| 54 | `for x in {1 2 3 4 5}: echo!(show x)` imprime 1 2 3 4 5 | 7 | ✅ |
| 55 | `3 in {1 2 3}` → `true` (CONTAINS dispatch) | 7 | ✅ |
| 56 | `5 in [0..2..10]` → `true` (Range CONTAINS O(1)) | 7 | ✅ |

### Fase 8: map/filter/fold + stream fusion ✅ (DoDs 57-59)

**Arquivos:**
- `crates/kata-inference/src/infer/` — reconhecer `@builtin("map"/"filter"/"fold")` e gerar nós TAST
- `crates/kata-optimizer/src/` — stream fusion pass (Map(Filter(arr)) → único loop)
- `stdlib/core.kata` — declarar map/filter/fold com @builtin

**Prelude adiciona:**
```kata
@builtin("map")
map :: (A -> B) List::A => List::B

@builtin("filter")
filter :: (A -> Boolean) List::A => List::A

@builtin("fold")
fold :: (A B -> A) A List::B => A
```

**Decisão de design (Arthur, 2026-07-17):** NÃO usar interfaces em assinaturas
genéricas — `unify` não casa `Generic("ITERABLE", [A])` com `List(Int)`.
Interceptar map/filter/fold no `infer_apply` por nome (como `format` e `len`).
map/filter retornam sempre List; para Array, o codegen converte List→Array no
final. Stream fusion postergada para fase separada (DoD 60).

**@builtin no typeck:** Quando vê `@builtin("map")` em uma assinatura,
o typeck intercepta a chamada e produz `TypedExprKind::Map` em vez de
`TypedExprKind::Closure` (dispatch normal). O body em Kata é ignorado —
o lowering é especializado por tipo concreto.

**Universalidade:** map/filter/fold despacham via ITERABLE — funcionam com
qualquer tipo que implemente ITERABLE(A), incluindo tipos do usuário.
O trade-off é que o usuário não pode redefinir `map` (só o builtin existe),
mas pode criar funções com outros nomes (`map_with_index`, `flat_map`, etc.)
que iteram via ITERABLE normalmente.

**Stream fusion:** O optimizer detecta `Map(Filter(arr))` na TAST e fusiona
em um único loop, evitando coleções intermediárias.

**DoDs:**
| # | Descrição | Fase |
|---|---|---|
| 57 | `map (+ 10 _) [1 2 3]` → `[11 12 13]` | 8 | ✅ |
| 58 | `filter (> _ 5) [1 8 3 9]` → `[8 9]` | 8 | ✅ |
| 59 | `fold + 0 [1 2 3]` → `6` | 8 | ✅ |
| 60 | `map (+ 10 _) (filter (> _ 5) [1 8 3 9])` → `[18 19]` (stream fusion) | — | Pendente (fase separada) |

## Atualização da documentação

Ao concluir:
- `docs/ROADMAP.md` — marcar Fio 8 ✅ Concluído
- `docs/PRD-fio8.md` — marcar fases e DoDs como ✅
- `docs/Kata-lang-manual.md` — **NÃO atualizar** (aspiracional)
- `docs/maquinaria-interna.md` — atualizar seções relevantes (LowerCtx, EmitCtx,
  novos HashMaps para coleções, stream fusion, CONTAINS dispatch)
- `docs/sintaxe-mapa.md` — adicionar `for`, `in` se faltante (delimitadores já existem)

## Regras críticas

- **Testes SEMPRE em `tests/` separado**, não inline no `src/`.
- **Edições cegas proibidas.** `write_file` exige leitura COMPLETA prévia.
- **`patch` tool: NÃO usar `new_string` >15 linhas Rust com aspas.**
- **Manual Kata5 é aspiracional** — não propor atualizações.
- **Convenção de status do PRD:** sufixo ✅ no fim da linha.
- **PRD é fonte de verdade para status** — se `cargo test` passa mas PRD diz "pendente", PRD está desatualizado.
- **Lambda dentro de `implements` deve estar em UMA LINHA.**
- **`import` não é processado pelo resolution.** Concatenar código inline nos testes E2E.
- **Kata NUNCA teve `if`.** Condicional = pattern matching + guards.
- **Validar sintaxe contra o parser** antes de escrever exemplos no PRD.
- **Chave composta no codegen:** `FuncKey = (String, Vec<Ty>, Ty)`.
- **List/Array/Range são intrínsecos de `Ty`, não `data` opacos.**
- **ForIn extrai A via InterfaceRegistry lookup**, não pattern-match em `Ty`.
- **`for` em lambda = compile error.** `for` só existe em Action body.
- **`.N` em coleções é checked** (retorna Result). Tuple é compile-time direto.
- **Range sempre exige passo explícito:** `[a..s..b]` (exclusive) ou `[a..s..=b]` (inclusive).
- **Range é genérico:** `Range(A)` onde A implementa Add e Ord.
- **Range codegen inlined operações** com base no tipo concreto. Runtime só aloca.

## Dependências

- **Fio 7** (interfaces, generics, dispatch) — ✅ Concluído
- **Fio 5** (Tuple para special case de `len` e `.N`) — ✅ Concluído
- **Fio 3** (loop, break, continue — reusado por `for`) — ✅ Concluído

## Desbloqueia

- **Fio 13** (Dict, Set — HAMT) — depende de Fio 8