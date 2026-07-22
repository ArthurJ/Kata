# PRD — Fio 13: Dict, Set (HAMT)

## Visão

Fio 13 introduz dicionários (`Dict::(K, V)`) e conjuntos (`Set::T`) persistentes
imutáveis baseados em Hash Array Mapped Trie (HAMT), com sharing estrutural,
alocados na arena per-fiber. Traz a interface `HASHABLE` (função de hash
semântico), literals de Dict e Set, e implementação de ITERABLE, COUNTABLE,
CONTAINS para ambos. Set também implementa operações de conjunto (union,
intersection, difference).

Constrói sobre Fio 7 (interfaces, generics, dispatch) e Fio 8 (ITERABLE,
COUNTABLE, INDEXABLE, CONTAINS, `for x in`, `in`).

## Depende de

- **Fio 7** ✅ (interfaces, generics, dispatch por dominância, monomorphização)
- **Fio 8** ✅ (ITERABLE, COUNTABLE, INDEXABLE, CONTAINS, `for x in`, `in`, `len`)
- **Pré-11** ✅ (árvore de arenas, EscapeTarget, `kata_rt_arena_alloc`)

## Sintaxe

### Delimitadores de coleção (atualizado)

| Sintaxe | Tipo | Layout | Exemplo |
|---|---|---|---|
| `[1 2 3]` | `List(T)` | Encadeada (Cons) | `[1 2 3]` → `List(Int)` |
| `{1 2 3}` | `Array(T)` | Contíguo (imutável) | `{1 2 3}` → `Array(Int)` |
| `[0..1..10]` | `Range(A)` | Lazy | `[0..1..10]` → `Range(Int)` |
| `{"a": 1 "b": 2}` | `Dict::(K, V)` | HAMT persistente | `{"a": 1 "b": 2}` → `Dict::(Text, Int)` |
| `{\|1 2 3\|}` | `Set::T` | HAMT persistente (sem values) | `{\|1 2 3\|}` → `Set::Int` |
| `{}` | `Array(T)` | Vazio | `{}` → `Array(InferVar)` |
| `{\|\|}` | `Set::T` | Vazio | `{\|\|}` → `Set::InferVar` |
| `{:}` | `Dict::(K, V)` | Vazio | `{:}` → `Dict::(InferVar, InferVar)` |

### Desambiguação no parser

O parser decide o tipo de literal após `LBrace` pelo próximo token:

| Sequência | Decisão | Motivo |
|---|---|---|
| `LBrace` `RBrace` | ArrayLit vazio | `{}` já é Array |
| `LBrace` `Pipe` | SetLit | `Pipe` após `{` é exclusivo de Set |
| `LBrace` `Colon` | DictLit vazio | `{:}` — Dict sem entries |
| `LBrace` `<expr>` ... | ArrayLit ou DictLit | decide pelo conteúdo (se `Colon` aparece após primeiro elemento → Dict) |

Para Dict não-vazio, o parser lê pares `chave: valor` separados por espaço:
`{"a": 1 "b": 2}`. O `Colon` após a chave distingue de Array.

Para Set, o parser lê elementos separados por espaço entre `{|` e `|}`:
`{|1 2 3|}`. O `Pipe` é exclusivo de Set dentro de `{}` — não há ambiguidade
com `PipeFallback` (que é infixo, não aparece após `LBrace`).

### Exemplos

```kata
# Dict literal
let idades := {"alice": 30 "bob": 25 "carol": 35}
# Dict::(Text, Int)

# Set literal
let primos := {|2 3 5 7 11 13|}
# Set::Int

# Dict vazio
let vazio := {:}
# Dict::(InferVar, InferVar)

# Set vazio
let vazio_set := {||}
# Set::InferVar

# Lookup
at idades "alice"        # Result::(Int, Err) — INDEXABLE dispatch
at idades "bob" ?        # Int (panic se Err)
at idades "bob" | 0      # Int (fallback 0)

# Membership
"alice" in idades        # true — CONTAINS dispatch
5 in primos              # true — CONTAINS dispatch

# Insert (retorna novo Dict, original inalterado)
let idades2 := insert idades "dave" 40

# Remover
let idades3 := remove idades "bob"

# Iteração (Dict produz tuplas (K, V))
for par in idades
    echo!(par)

# Iteração (Set produz elementos)
for p in primos
    echo!(p)

# Operações de Set
union primos {|17 19|}         # Set::Int — união
intersection a b               # Set::Int — interseção
difference a b                 # Set::Int — diferença
```

## Decisões de design

### D1: Persistente imutável com sharing estrutural

Dict e Set são **persistente imutáveis**, como List e Array. `insert` retorna
um novo Dict/Set; o original é inalterado. O HAMT fornece sharing estrutural
automaticamente: cada `insert` aloca apenas O(log₃₂ n) nós novos (caminho
modificado), os nós fora do caminho são compartilhados entre o Dict original e
o novo.

**Por que não mutável ou CoW:**

- Arena bump allocation torna a alocação de nós novos praticamente free
  (O(1) por nó, sem malloc/free individual). O "custo" da persistência que
  existe em outras linguagens é amortizado pela arena.
- `var` + reassign resolve o caso de Actions: `var d := {:}; for x in items:
  d := insert d k v` — os Dicts intermediários são coletados pela arena em
  O(1) no reset.
- Kata não tem CoW em nenhum lugar. Introduzir CoW para Dict seria uma exceção
  arquitetural — EscapeTarget é sobre lifetime da arena, não sobre copying.
- Dobro de implementação (Dict + DictMut) é custo desnecessário quando `var`
  já substitui a mutabilidade.

### D2: HAMT — Hash Array Mapped Trie

**Estrutura:** trie de altura fixa com branching factor 32. Cada nó interno
tem um bitmap de 32 bits indicando quais filhos estão presentes. A chave é
hashada; os bits do hash selecionam o caminho na trie (5 bits por nível =
32 filhos).

```
nível 0:  bits [0..5)   → índice no root (bitmap de 32 bits)
nível 1:  bits [5..10)  → índice no nó filho
nível 2:  bits [10..15) → ...
nível 3:  bits [15..20)
nível 4:  bits [20..25)
nível 5:  bits [25..30) → leaf (collision node se hashes colidiram)
```

**Altura:** 6 níveis cobrem 30 bits de hash (32⁶ ≈ 10⁹ entries). Para mais,
collision nodes resolvem colisões.

**Layout de um nó interno (HAMTNode):**

```text
offset 0:  bitmap (i64) — bit i = 1 significa que child i existe
offset 8:  count (i64) — número de filhos presentes (popcount do bitmap)
offset 8 + i*8:  child[i] (i64) — ponteiro para filho ou entry
```

Filhos são densos — apenas filhos presentes são armazenados, indexados por
popcount do bitmap até a posição do bit. `bitmap & ((1 << idx) - 1)` dá o
offset compactado.

**Layout de uma entry (KVPair):**

```text
offset 0:  key (i64) — valor da chave (SMI ou ponteiro)
offset 8:  value (i64) — valor associado (SMI ou ponteiro)
```

Para Set, entries têm apenas a chave (sem value): 8 bytes por entry em vez
de 16.

**Sharing em `insert`:**

```
Dict original          Dict após insert(k, v)
     root                    root'
    / | \                   / | \
   A  B  C                 A  B'  C
      / \                     / \
     D   E                    D'  E
                                \
                                 v (novo)
```

A, C, D, E são compartilhados. B', D', root', e a nova entry são alocados.
Custo: O(log₃₂ n) nós novos por insert. Para 1 milhão de entries,
log₃₂(10⁶) ≈ 4 nós novos.

### D3: Alocação na arena

Dict e Set seguem o mesmo padrão de List e Array — todos os nós são
alocados via `kata_rt_arena_alloc(arena_handle, node_size)`, onde
`arena_handle` é decidido por `EscapeTarget`:

```rust
let arena_handle = match expr.escape {
    EscapeTarget::Local => ctx.fiber_arena,      // morre com o fiber
    EscapeTarget::Caller => ctx.caller_arena,     // sobrevive ao fiber
};
```

O sharing acontece naturalmente: ponteiros para nós antigos (não copiados)
continham válidos porque a arena não libera nada até o reset.

**ABI uniforme:** Dict e Set são ponteiros opacos (`i64`) no codegen, como
qualquer outro tipo boxed. `ty_to_clif` mapeia para `I64`.

### D4: `interface HASHABLE` — hash semântico

```kata
interface HASHABLE
    hash :: Self => Int
```

`hash` produz um `Int` determinístico, consistente com `=`: se `a = b`,
então `hash(a) = hash(b)`. O HAMT usa `hash` para navegar a trie e `=` (via
`EQ`) para confirmar colisões.

**Por que interface e não hash genérico no runtime:**

Todo valor em runtime é `i64`, mas para tipos boxed (Text, Struct, etc.) o
`i64` é um ponteiro. Hashar o ponteiro quebra a consistência com `=` — dois
valores semanticamente iguais em endereços diferentes teriam hashes
diferentes. O hash precisa ser **semântico** (baseado no conteúdo), e para
isso precisa conhecer o tipo. `HASHABLE` despacha via DispatchTable como
`EQ`, `ORD`, `SHOW` — cada tipo implementa a sua.

**Implementações no prelude:**

```kata
Int implements HASHABLE
    hash :: Int => Int @ffi("kata_rt_hash_int")

Text implements HASHABLE
    hash :: Text => Int @ffi("kata_rt_hash_text")

Rational implements HASHABLE
    hash :: Rational => Int @ffi("kata_rt_hash_rational")
```

`kata_rt_hash_int`: SMI untag → hash direto (ex: FNV-1a sobre os bits).
`kata_rt_hash_text`: hash dos bytes da string (FNV-1a).
`kata_rt_hash_rational`: hash do numerador XOR denominador.

**Float NÃO implementa HASHABLE por padrão.** Float tem problemas de
igualdade (NaN ≠ NaN, -0.0 = 0.0 em bits diferentes). Se o usuário quiser
`Dict::(Float, V)`, implementa `HASHABLE` manualmente com semântica
definida. Isso é uma omissão deliberada, não um esquecimento.

**Tipos compostos (Struct, Tuple, Sum):** O usuário implementa `HASHABLE`
combinando hashes dos campos. O compilador pode sintetizar `HASHABLE` para
Structs onde todos os campos implementam `HASHABLE` (evolução futura —
não no escopo v1).

### D5: Set como tipo próprio, não `Dict::(K, Unit)`

Set tem tipo próprio (`Ty::Set(Box<Ty>)`) e implementação própria (HAMT de
chaves sem values). Razões:

1. **Eficiência de espaço:** Set não armazena values — 8 bytes por entry em
   vez de 16. Em conjuntos grandes, isso é metade da memória.
2. **Operações de conjunto:** union, intersection, difference são naturais em
   um HAMT de chaves. Implementá-las sobre `Dict::(K, Unit)` exigiria ignorar
   os valores — semanticamente confuso.
3. **Tipagem clara:** `Set::Int` diz "conjunto de Ints". `Dict::(Int, Unit)`
   diz "dicionário de Int para Unit" — intenção menos clara.
4. **CONTAINS é a operação primária:** Set é essencialmente um CONTAINS
   otimizado. Dict é lookup de valor. São abstrações diferentes.

### D6: `{|...|}` — Pipe exclusivo de Set dentro de `{}`

`Pipe` (`|`) após `LBrace` é interpretado exclusivamente como abertura de
SetLit. Não há ambiguidade com `PipeFallback` (que é infixo e nunca aparece
após `LBrace`).

O fechamento `|}` é `Pipe` + `RBrace`. O parser em modo SetLit coleta
elementos separados por espaço até encontrar `Pipe` + `RBrace`.

`Pipe` dentro de `{}` mas não na posição inicial (ex: `{"a": f x | g y}`)
continua sendo `PipeFallback` — só `LBrace` imediatamente seguido de `Pipe`
ativa modo SetLit.

### D7: Dict vazio `{:}` vs Array vazio `{}`

`{}` já é Array vazio (Fio 8). Para Dict vazio, usamos `{:}` — `LBrace`
seguido de `Colon` indica Dict sem entries. Set vazio é `{||}` — `LBrace`
seguido de `Pipe` seguido de `Pipe` seguido de `RBrace`.

| Literal | Tipo | Sintaxe |
|---|---|---|
| Array vazio | `Array(T)` | `{}` |
| Dict vazio | `Dict::(K, V)` | `{:}` |
| Set vazio | `Set::T` | `{\|\|}` |

### D8: Ordem de inserção via Cons-list overlay

Dict mantém iteração em **ordem de inserção** (como Python 3.7+, JavaScript).
O HAMT não preserva ordem — a estrutura da trie segue os bits do hash, não a
sequência de inserção. Para garantir ordem sem perder sharing estrutural,
Dict carrega duas estruturas:

```
Dict = (hamt_root: i64, insert_log: i64)
```

- `hamt_root` — HAMT para lookup O(log₃₂ n)
- `insert_log` — Cons list persistente de ponteiros para KVPairs, para
  iteração em ordem de inserção

**Insert:** `insert d k v` faz duas coisas:
1. HAMT insert (como já existe) — O(log₃₂ n) nós novos
2. Cons prepend do ponteiro do KVPair — O(1), um novo Cons cell (16 bytes)

A versão antiga do Dict aponta para a Cons list antiga; a nova aponta para
a nova. Sharing funciona em ambas as estruturas.

**Remove:** marca o KVPair como tombstone (flag no KVPair, não na Cons list).
Iteração pula tombstones. O HAMT remove a entry normalmente.

**Iteração:** percorre a Cons list. Ordem é reversa de inserção (Cons prepend).
Se ordem direta for necessária, itera e reverte (O(n) extra na arena).

**Replace:** quando `insert` substitui um valor para a mesma chave, o novo
KVPair é prepended na Cons list. O KVPair antigo continua na Cons list mas
é ignorado durante iteração (o HAMT só aponta para o novo). Alternativa:
marcar o KVPair antigo como tombstone na Cons list. A escolha depende de
se `replace` deve ou não "mover" a chave para o fim da ordem de inserção.
**Decisão: replace NÃO move** — o novo KVPair é prepended, mas a iteração
verifica o HAMT para saber qual é a versão atual. Mais simples: iteração
percorre a Cons list e para cada KVPair, verifica se ele é a versão atual
no HAMT (comparando ponteiros). Se não for, skip.

| Operação | Custo | Memória extra |
|---|---|---|
| insert | O(log₃₂ n) HAMT + O(1) Cons | 16 bytes/entry (Cons cell) |
| get | O(log₃₂ n) HAMT | — |
| remove | O(log₃₂ n) HAMT + tombstone | — |
| iterate | O(n) Cons list | — |

**Set NÃO tem ordem de inserção.** Set itera via HAMT (ordem não-determinística).
Se o usuário precisar de ordem, converte para List e ordena. Set é um
CONTAINS otimizado, não uma sequência.

## Tipos

### `Ty` — novos variants

```rust
pub enum Ty {
    // ... existentes ...
    /// Dict persistente: `Dict::(K, V)` — HAMT de pares chave-valor.
    Dict(Box<Ty>, Box<Ty>),
    /// Set persistente: `Set::T` — HAMT de chaves (sem values).
    Set(Box<Ty>),
}
```

**Variants intrínsecos de `Ty`**, como `List`, `Array`, `Range` — não são
`data` opacos. O codegen precisa saber o layout (HAMT), e isso é informação
de `Ty`, não de `@ffi`.

`Dict::(Text, Int)` é `Ty::Dict(Box::new(Ty::Text), Box::new(Ty::Int))`.
`Set::Int` é `Ty::Set(Box::new(Ty::Int))`.

### Display

```rust
Ty::Dict(k, v) => write!(f, "Dict::({k}, {v})"),
Ty::Set(t) => write!(f, "Set::{t}"),
```

### `type_name` (dispatch)

```rust
Ty::Dict(_, _) => Some("Dict".into()),
Ty::Set(_) => Some("Set".into()),
```

## Interfaces

### HASHABLE — nova interface

```kata
interface HASHABLE
    hash :: Self => Int
```

Registrada no prelude (`stdlib/core.kata`). Tipos que podem ser chave de
Dict ou elemento de Set devem implementar `HASHABLE`.

### Implementações para Dict

```kata
Dict::(K, V) implements ITERABLE::(K, V)
    next :: Dict::(K, V) => Optional::(K, V) @ffi("kata_rt_dict_next")

Dict::(K, V) implements COUNTABLE
    len :: Dict::(K, V) => Int @ffi("kata_rt_dict_len")

Dict::(K, V) implements INDEXABLE::V
    at :: Dict::(K, V) K => Result::(V, Err) @ffi("kata_rt_dict_get_checked")

Dict::(K, V) implements CONTAINS::K
    contains :: Dict::(K, V) K => Boolean @ffi("kata_rt_dict_contains")
```

**Constraint:** `K implements HASHABLE`. O typeck valida isso estaticamente.
Se `K` não implementa `HASHABLE`, erro de compilação:

```
Type error: Dict key type `Pessoa` does not implement HASHABLE
```

### Implementações para Set

```kata
Set::T implements ITERABLE::T
    next :: Set::T => Optional::T @ffi("kata_rt_set_next")

Set::T implements COUNTABLE
    len :: Set::T => Int @ffi("kata_rt_set_len")

Set::T implements CONTAINS::T
    contains :: Set::T T => Boolean @ffi("kata_rt_set_contains")
```

Set **não** implementa `INDEXABLE` — não há lookup por índice em um conjunto.

**Constraint:** `T implements HASHABLE`. Mesma validação que Dict.

### Operações de Set (standalone, não-interface)

```kata
# União: retorna novo Set com elementos de ambos
@ffi("kata_rt_set_union")
union :: Set::T Set::T => Set::T

# Interseção: retorna novo Set com elementos em ambos
@ffi("kata_rt_set_intersection")
intersection :: Set::T Set::T => Set::T

# Diferença: retorna novo Set com elementos no primeiro mas não no segundo
@ffi("kata_rt_set_difference")
difference :: Set::T Set::T => Set::T
```

### Operações de Dict (standalone, não-interface)

```kata
# Insert: retorna novo Dict com a chave-valor adicionada/substituída
@ffi("kata_rt_dict_insert")
insert :: Dict::(K, V) K V => Dict::(K, V)

# Remove: retorna novo Dict sem a chave
@ffi("kata_rt_dict_remove")
remove :: Dict::(K, V) K => Dict::(K, V)
```

`insert` e `remove` são funções puras (não Actions) — recebem o Dict, K, V
e retornam um novo Dict. A arena usada para alocar os novos nós é decidida
pelo `EscapeTarget` do `TypedExpr` no call site, como qualquer outra
coleção.

## Runtime

### `kata-rt/src/dict.rs` — novo módulo

Funções C-ABI expostas para o codegen:

```rust
// Construção
kata_rt_dict_empty(arena_handle: i64) -> i64
    // Retorna ponteiro para Dict vazio (root node com bitmap=0)

kata_rt_dict_insert(dict_ptr: i64, key: i64, value: i64, hash: i64, arena_handle: i64) -> i64
    // Percorre a trie seguindo os bits do hash. Aloca nós novos no caminho.
    // Usa EQ (kata_rt_*) para confirmar colisões. Retorna novo root.

kata_rt_dict_remove(dict_ptr: i64, key: i64, hash: i64, arena_handle: i64) -> i64
    // Remove a entry. Se o nó fica com 1 filho, promove o filho (compaction).
    // Retorna novo root.

// Consulta
kata_rt_dict_get_checked(dict_ptr: i64, key: i64, hash: i64) -> i64
    // Percorre a trie. Se encontra: Result::Ok(value). Se não: Result::Err.

kata_rt_dict_contains(dict_ptr: i64, key: i64, hash: i64) -> i64
    // 1 se a chave está presente, 0 caso contrário.

kata_rt_dict_len(dict_ptr: i64) -> i64
    // Conta entries (pode ser O(n) ou cacheado no root).

// Iteração
kata_rt_dict_next(dict_ptr: i64, iter_state: i64) -> i64
    // Retorna Optional::(K, V) — próximo par, ou Optional::None.
    // iter_state é um cursor (índice linearizado ou caminho na trie).
```

### `kata-rt/src/set.rs` — novo módulo

```rust
// Construção
kata_rt_set_empty(arena_handle: i64) -> i64
kata_rt_set_insert(set_ptr: i64, elem: i64, hash: i64, arena_handle: i64) -> i64
kata_rt_set_remove(set_ptr: i64, elem: i64, hash: i64, arena_handle: i64) -> i64

// Consulta
kata_rt_set_contains(set_ptr: i64, elem: i64, hash: i64) -> i64
kata_rt_set_len(set_ptr: i64) -> i64

// Iteração
kata_rt_set_next(set_ptr: i64, iter_state: i64) -> i64

// Operações de conjunto
kata_rt_set_union(a: i64, b: i64, arena_handle: i64) -> i64
kata_rt_set_intersection(a: i64, b: i64, arena_handle: i64) -> i64
kata_rt_set_difference(a: i64, b: i64, arena_handle: i64) -> i64
```

### Funções de hash

```rust
kata_rt_hash_int(val: i64) -> i64
    // SMI untag, aplica FNV-1a sobre os bits.

kata_rt_hash_text(str_ptr: i64) -> i64
    // FNV-1a sobre os bytes da string.

kata_rt_hash_rational(rat_ptr: i64) -> i64
    // Hash do numerador XOR denominador (lê layout do Rational).
```

### Registro FFI

Todas as funções são registradas em `kata-rt/src/lib.rs` e exportadas. O
`ffi_registry.rs` no codegen as reconhece pelos nomes.

**Parâmetro `hash`:** `kata_rt_dict_insert`, `kata_rt_dict_get_checked`, etc.
recebem o hash como parâmetro explícito. O codegen chama `hash(key)` (via
DispatchTable, despachando para `kata_rt_hash_int` ou `kata_rt_hash_text`
conforme o tipo de `K` monomorfizado) e passa o resultado como `hash` para
as funções do Dict. Isso separa hash (despachado por tipo) de navegação da
trie (genérica, independente do tipo).

## Parser

### `Expr` — novos variants

```rust
pub enum Expr {
    // ... existentes ...
    /// `{"k": v "k2": v2}` — literal de Dict.
    DictLit { entries: Vec<(Spanned<Expr>, Spanned<Expr>)> },
    /// `{|1 2 3|}` — literal de Set.
    SetLit { elements: Vec<Spanned<Expr>> },
}
```

### Modificação em `parse_array_lit`

```rust
pub(crate) fn parse_brace_lit(&mut self) -> Result<Spanned<Expr>, FrontendError> {
    let start = self.expect(&Token::LBrace, "`{`")?;

    // `}` → Array vazio (existente)
    if matches!(self.peek(), Token::RBrace) { ... }

    // `|` → SetLit
    if matches!(self.peek(), Token::Pipe) {
        self.advance(); // consume |
        // `||}` → Set vazio
        if matches!(self.peek(), Token::Pipe) {
            self.advance(); // consume |
            self.expect(&Token::RBrace, "`}`")?;
            return Ok(SetLit { elements: vec![] });
        }
        // Coleta elementos separados por espaço até `|}`
        let mut elements = vec![parse_expr(self)?];
        while !matches!(self.peek(), Token::Pipe) {
            elements.push(parse_expr(self)?);
        }
        self.advance(); // consume |
        self.expect(&Token::RBrace, "`}`")?;
        return Ok(SetLit { elements });
    }

    // `:` → DictLit vazio `{:}`
    if matches!(self.peek(), Token::Colon) {
        self.advance(); // consume :
        self.expect(&Token::RBrace, "`}`")?;
        return Ok(DictLit { entries: vec![] });
    }

    // Primeiro elemento — parse expr
    let first = parse_expr(self)?;

    // Se próximo é `:` → modo Dict: pares chave:valor
    if matches!(self.peek(), Token::Colon) {
        self.advance(); // consume :
        let first_val = parse_expr(self)?;
        let mut entries = vec![(first, first_val)];
        while !matches!(self.peek(), Token::RBrace) {
            let key = parse_expr(self)?;
            self.expect(&Token::Colon, "`:` (chave: valor)")?;
            let val = parse_expr(self)?;
            entries.push((key, val));
        }
        self.expect(&Token::RBrace, "`}`")?;
        return Ok(DictLit { entries });
    }

    // Caso contrário → ArrayLit (existente)
    let mut elements = vec![first];
    while !matches!(self.peek(), Token::RBrace) {
        elements.push(parse_expr(self)?);
    }
    self.expect(&Token::RBrace, "`}`")?;
    Ok(ArrayLit { elements })
}
```

## Typeck

### Inferência de `DictLit`

1. Infere o tipo de cada chave e valor.
2. Unifica todas as chaves para um tipo `K` e todos os valores para `V`.
3. Verifica que `K` implementa `HASHABLE`.
4. Produz `TypedExprKind::DictLit { entries, key_ty, value_ty }`.

### Inferência de `SetLit`

1. Infere o tipo de cada elemento.
2. Unifica todos para um tipo `T`.
3. Verifica que `T` implementa `HASHABLE`.
4. Produz `TypedExprKind::SetLit { elements, elem_ty }`.

### TAST

```rust
pub enum TypedExprKind {
    // ... existentes ...
    DictLit {
        entries: Vec<(Spanned<TypedExpr>, Spanned<TypedExpr>)>,
        key_ty: Ty,
        value_ty: Ty,
    },
    SetLit {
        elements: Vec<Spanned<TypedExpr>>,
        elem_ty: Ty,
    },
}
```

## Codegen

### Lowering de `DictLit`

1. Aloca Dict vazio: `kata_rt_dict_empty(arena_handle)`.
2. Para cada par `(k, v)`:
   - Lowera `k` e `v`.
   - Chama `hash(k)` (despachado via DispatchTable para `kata_rt_hash_*`).
   - Chama `kata_rt_dict_insert(dict, k, v, hash, arena_handle)`.
3. O resultado é o Dict final (cadeia de inserts).

**Otimização futura:** construir a trie diretamente em vez de cadeia de
inserts. Adiado — a cadeia funciona e é simples.

### Lowering de `SetLit`

1. Aloca Set vazio: `kata_rt_set_empty(arena_handle)`.
2. Para cada elemento `e`:
   - Lowera `e`.
   - Chama `hash(e)`.
   - Chama `kata_rt_set_insert(set, e, hash, arena_handle)`.
3. O resultado é o Set final.

### Lowering de `at` (Dict)

`at dict key` → desugar para:
1. Lowera `dict` e `key`.
2. Chama `hash(key)`.
3. Chama `kata_rt_dict_get_checked(dict, key, hash)` → `Result::(V, Err)`.

### Lowering de `contains` (Set/Dict)

`contains coll elem` → desugar para:
1. Lowera `coll` e `elem`.
2. Chama `hash(elem)`.
3. Chama `kata_rt_dict_contains` ou `kata_rt_set_contains`.

### Lowering de `for x in dict`

`for` sobre `Dict::(K, V)` desugara para iteração via ITERABLE:
- `kata_rt_dict_next(dict, iter_state)` → `Optional::(K, V)`.
- `Some((k, v))` → bind `x := (k, v)`, executa body, repete.
- `None` → termina.

`for` sobre `Set::T`:
- `kata_rt_set_next(set, iter_state)` → `Optional::T`.
- `Some(v)` → bind `x := v`, executa body, repete.
- `None` → termina.

### Arena

`arena_handle` decidido por `EscapeTarget`, idêntico a List/Array:

```rust
let arena_handle = match expr.escape {
    EscapeTarget::Local => ctx.fiber_arena,
    EscapeTarget::Caller => ctx.caller_arena,
};
```

## Fases

### Fase 1: Runtime — HAMT em Rust

Implementar `dict.rs` e `set.rs` em `kata-rt/src/` com:
- HAMTNode (bitmap, children, entries).
- `kata_rt_dict_empty/insert/remove/get_checked/contains/len/next`.
- `kata_rt_set_empty/insert/remove/contains/len/next`.
- `kata_rt_set_union/intersection/difference`.
- `kata_rt_hash_int/hash_text/hash_rational`.
- Testes unitários Rust (não Kata) — construir Dict, inserir, recuperar,
  verificar sharing (ponteiros de nós antigos não mudam).

**DoD:** `cargo test -p kata-rt` passa. HAMT com 1000 inserts, gets
corretos. Sharing verificado. Set union/intersection/difference corretos.

### Fase 2: Ty + parser

- Adicionar `Ty::Dict(Box<Ty>, Box<Ty>)` e `Ty::Set(Box<Ty>)` em `ty.rs`.
- Adicionar `Expr::DictLit` e `Expr::SetLit` em `expr.rs`.
- Modificar `parse_array_lit` → `parse_brace_lit` com desambiguação.
- Atualizar `type_name`, `Display`.
- Testes de parser: `{}`, `{:}`, `{||}`, `{"a": 1}`, `{|1 2 3|}`.

**DoD:** Parser produz os nós corretos. `cargo test -p kata-parser` passa.
Snapshots atualizados.

### Fase 3: Typeck — HASHABLE + DictLit/SetLit

- Adicionar `interface HASHABLE` em `stdlib/core.kata`.
- Adicionar implementações de `HASHABLE` para Int, Text, Rational.
- Inferência de `DictLit` e `SetLit` (unificação de chaves/elementos,
  verificação de `HASHABLE`).
- Testes de typeck: inferência de tipos, erro quando `K` não é `HASHABLE`.

**DoD:** `Dict::(Text, Int)` e `Set::Int` inferidos corretamente.
`Dict::(Pessoa, Int)` sem `HASHABLE` → erro de compilação.

### Fase 4: Codegen — lowering

- Lowering de `DictLit` (cadeia de inserts).
- Lowering de `SetLit` (cadeia de inserts).
- Lowering de `at` (Dict) com hash.
- Lowering de `contains` (Set/Dict) com hash.
- Lowering de `for x in` (Dict/Set) com `kata_rt_*_next`.
- Lowering de `insert`, `remove`, `union`, `intersection`, `difference`.
- Registro de todas as funções FFI em `ffi_registry.rs`.
- Implementações de ITERABLE/COUNTABLE/INDEXABLE/CONTAINS em `core.kata`.

**DoD:** Programa Kata com `let d := {"a": 1 "b": 2}; echo!(at d "a" ?)`
imprime `1`. `let s := {|1 2 3|}; echo!(5 in s)` imprime `false`.

### Fase 5: Operações de Set end-to-end

- `union`, `intersection`, `difference` funcionando em Kata.
- Testes E2E com conjuntos grandes (10000+ elementos).
- Verificar que `for x in set` itera corretamente.

**DoD:**
```kata
let a := {|1 2 3 4 5|}
let b := {|3 4 5 6 7|}
echo!(len (union a b))          # 7
echo!(len (intersection a b))  # 3
echo!(len (difference a b))     # 2
```

### Fase 6: Testes e snapshots

- Testes de parser nomeados por responsabilidade em `tests/`.
- Testes E2E de Dict: insert, get, remove, contains, iterate, len.
- Testes E2E de Set: insert, contains, union, intersection, difference, iterate.
- Testes de erro: `K` não-`HASHABLE`, tipos misturados em Set.
- Snapshots de TAST para DictLit e SetLit.
- Teste de stress: 100000 inserts, verificação de O(log) por insert
  (medir tempo, não estourar timeout cooperativo).

**DoD:** Cobertura completa. `cargo test` e `cargo test -p kata-codegen`
passam. Sem regressões em testes existentes.

## Dependências de Fios

```
Fio 13
├── Fio 7 (interfaces, generics, dispatch, monomorph)
├── Fio 8 (ITERABLE, COUNTABLE, INDEXABLE, CONTAINS, for x in, in, len)
└── Pré-11 (arena, EscapeTarget)
```

## Não no escopo (post-1.0)

- **Síntese de `HASHABLE` para Structs:** o compilador sintetiza `hash`
  automaticamente para Structs onde todos os campos implementam `HASHABLE`.
- **`@builtin` para Dict/Set:** operações como `map`/`filter`/`fold` sobre
  Dict/Set (stream fusion com HAMT).
- **Dict comprehensions:** `{k: f k for k in keys}` — açúcar sintático.
- **Set comprehensions:** `{|x for x in lst if primo x|}` — açúcar.
- **Tensors:** usam `;` como separador de dimensão dentro de `{}` —
  Fio separado. Não interfere com Dict (que usa `:`) ou Set (que usa `|`).