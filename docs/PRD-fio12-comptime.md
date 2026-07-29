# PRD — Fio 12: Comptime, `@cache`

**Status:** Fase 1 ✅, Fase 2 ✅ (Fases 3-6 pendentes)
**Data:** 2026-07-28
**Depende de:** Fio 1-10 ✅ (pipeline completo, módulos, arenas hierárquicas), Fio 11 ✅ (TypeShape para serialização de args em `@cache`)
**Não depende de:** `@parallel` (congelado), Fio 13 (Dict/Set), Fio 15 (REPL)

## 1. Objetivo

Permitir que expressões sejam avaliadas durante a compilação via JIT-and-execute,
substituindo o resultado por um literal ou HeapSnapshot na TAST. E permitir
memoização opt-in de funções puras via cache em runtime.

### Princípio: JIT-and-execute, não interpretador de TAST

O comptime pass compila a expressão usando o pipeline normal (infer → codegen →
JIT) e executa no `kata-rt` real. Justificativa (documentada no manual §2.2):

- **Sem reimplementação:** o runtime já avalia tudo — match, guards, lambda,
  recursão, FFI, listas, structs. Zero código duplicado.
- **Sem teto:** cobre tudo que o codegen compila. Um interpretador teria
  cobertura limitada aos tipos que `ConstValue` representa.
- **Consistência semântica:** comptime e runtime usam exactamente o mesmo código.
  Não há risco de divergência.

### Princípio: explícito e opt-in (estilo Zig, não D)

`@comptime` é sempre explícito no call-site. O compilador nunca infere, não
propaga, não adivinha. O caller aceita o custo de compilação e o custo de binário
maior. O Cranelift continua fazendo constant folding trivial (`+ 1 2` → `3`) ao
nível de IR — isso é ortogonal e automático, sem directiva.

## 2. Sintaxe

### 2.1. `@comptime` em `let` top-level

```kata
@comptime let config := parse_config "default.json"
@comptime let tabela := map (* _ 2) [1..100]
```

O valor é computado em compile-time, vira HeapSnapshot (ou literal se escalar), e o
binding aponta para esse snapshot no runtime. Zero código de construção no binário.

### 2.2. `@comptime` em expressão top-level (entry point)

```kata
@comptime fatorial 10
```

Avalia, embeda o resultado, substitui na TAST por literal ou snapshot.

### 2.3. `@comptime` em call-site dentro de body

```kata
action main =>
  let x := @comptime fibonacci 20 in
  ...
```

O `@comptime` diz "esta call, com estes args, avalia agora." O compilador tenta
JIT-and-execute. Se consegue, substitui por snapshot. Se não consegue, erro de
compilação — call-site é *guarantee*, não hint.

### 2.4. `@cache{strategy: "LRU"}` em definição de função

```kata
dobro :: Int => Int @cache{strategy: "LRU"}
dobro x := * x 2
```

Anota a definição. O codegen emite cache lookup no prólogo da função, antes da
primeira cláusula e antes do primeiro lambda. O body fica intacto — sem
reescrita de TAST.

### 2.5. Sem definition-site `@comptime` (removido do ROADMAP)

O ROADMAP original listava `@comptime` definition-site (hint) — "avalia se
consegue, senão runtime." Isto foi removido. A decisão de avaliar em compile-time
pertence ao call-site, onde os argumentos são visíveis. Não há "talvez avalia."

## 3. Semântica

### 3.1. Critério de constness

A análise é uma dataflow simples. "Comptime-available" é binário — existe em
compile-time ou não existe. Não há lattice, nem níveis de "quão constante."

**Comptime-available:**
- Literais (Int, Float, Text, Boolean)
- Resultados de `@comptime` anterior (já são snapshots/literals na TAST)
- `let` bindings cujo initializer é comptime-available (propagação dataflow trivial)
- Definições de função do módulo (a definição existe; o comptime pass compila e executa)

**Não comptime-available:**
- Parâmetros de função
- `var` de Action
- `let` bindings cujo initializer não é comptime-available
- Qualquer valor que depende de runtime I/O

Se todos os valores referenciados por uma expressão `@comptime` são
comptime-available → JIT-and-execute → HeapSnapshot. Se algum não é → erro de
compilação: "`n` não é disponível em compile-time."

### 3.2. Tipos preservados exactamente

O comptime pass é uma optimização de *quando*, não de *o quê*. O nó `HeapSnapshot`
na TAST tem o mesmo `ty: Ty` que a expressão original. O typeck não re-valida o
snapshot — confia que o pipeline produziu um valor do tipo correcto.

- `@comptime fatorial 10` → snapshot tem tipo `Int`, valor `3628800`
- `@comptime PositiveInt 5` → snapshot tem tipo `Result::(PositiveInt, Err)`, valor `Ok(...)`
- `@comptime PositiveInt (-5)` → snapshot tem tipo `Result::(PositiveInt, Err)`, valor `Err(...)`
- `@comptime [1..100]` → snapshot tem tipo `List(Int)`, valor são 100 cons cells

Se uma avaliação em runtime devolve `Result`, em comptime devolve `Result` também.
Mudar isso geraria inconsistência.

### 3.3. Ascription refined + comptime

O Fio 6 faz avaliação constante *local ao typeck* para predicados de ascription
refined. Isto funciona para predicados triviais (`> _ 0`) mas não para complexos
(`is_prime _`) — o typeck não tem evaluator.

Com a infraestrutura de comptime disponível, a hierarquia fica natural:

1. **Valor comptime-available + predicado trivial** (`> _ 0`, `= _ 0`) — typeck
   reduz localmente. Sem JIT, sem overhead. Fast path. Comportamento existente
   do Fio 6 preservado.
2. **Valor comptime-available + predicado complexo** (`is_prime _`, `> _ (fatorial 5)`)
   — typeck delega ao comptime pass, que faz JIT-and-execute do predicado e
   retorna `Boolean`. O typeck consome o resultado, decide passar (`PositiveInt`
   directo) ou falhar (type error). O typeck não embeda snapshot — apenas consulta.
3. **Valor não comptime-available** — predicado validado em runtime pelo smart
   constructor falível, como já acontece. Comportamento existente preservado.

**Invariante:** o comportamento do Fio 6 não muda. O que era validado localmente
continua sendo. O comptime pass só é consultado para predicados que o typeck não
consegue avaliar — e apenas quando o valor é comptime-available.

### 3.4. `@cache{strategy: "LRU"}`

#### Mecanismo

`@cache` anota a definição da função. O codegen emite cache logic no prólogo da
função compilada, antes da primeira cláusula e antes do primeiro lambda:

1. Hash dos args → `kata_rt_cache_lookup(handle, key) -> (hit, value)`
2. Se hit: retorna value direto, não executa o body
3. No epílogo, após computar o resultado: `kata_rt_cache_insert(handle, key, value)`

O body fica intacto. Sem reescrita de TAST. O cache é um gate na entrada da função.

#### Arena do cache

O cache hashmap é lazy-allocated na `caller_arena` na primeira chamada. Os valores
cacheados ficam na mesma arena. Quando a arena morre (fiber termina, ou root no
fim do run), cache e valores morrem juntos.

Isto resolve use-after-free sem deep-copy, sem forçar escape target, sem restringir
a escalares. O cache e o seu conteúdo têm o mesmo lifetime da arena que os aloca.

- De uma action: `caller_arena` = `fiber_arena` → cache vive no fiber
- Do entry point puro: `caller_arena` = `root_arena` → cache vive na arena raiz

Fibers diferentes não partilham cache. Para memoização, o caso de uso típico é
"dentro desta computação, não recalcular" — fiber-local é suficiente.

#### Key via TypeShape

A key é o hash dos bytes dos argumentos serializados segundo o TypeShape. O Fio 11
introduziu TypeShape para `@parallel` (IPC entre processos). O cache reusa a mesma
infraestrutura de serialização. Zero trabalho novo de serialização.

#### Transparência

`@cache` é opt-in. O compilador não detecta oportunidades de memoização
automaticamente. Quem escreve `@cache{strategy: "LRU"}` sabe que está fazendo
memoização.

## 4. HeapSnapshot

### 4.1. O problema

`@comptime fatorial 10` produz `3628800` — fácil, vira literal. Mas
`@comptime range 1 100` produz uma lista de 100 cons cells — uma estrutura de
dados com ponteiros. Não vira literal. Precisa capturar os bytes do heap após a
execução e reproduzi-los no runtime.

### 4.2. Design com arenas

O runtime tem arenas contíguas, bump-allocated (Pré-11). Ponteiros dentro de uma
arena são offsets relativos. Uma snapshot de uma bump-allocated arena é trivial:
os bytes já estão contíguos, os ponteiros são offsets relativos dentro do bloco.

Em load-time: `memcpy` para root_arena + rebasing (somar `base_ptr`). Sem walking,
sem fix-up table complexa.

Isto é **mais simples** com arenas do que sem. O Pré-11 tornou HeapSnapshot mais
viável, não menos.

### 4.3. HeapSnapshot node na TAST

```rust
// kata-inference/src/typed.rs
TypedExprKind::HeapSnapshot {
    snapshot_id: u32,      // índice na tabela de snapshots do módulo
    ty: Ty,                // mesmo tipo que a expressão original
}
```

O comptime pass substitui a expressão `@comptime` por `HeapSnapshot { snapshot_id,
ty }`. O codegen lowera `HeapSnapshot` para um load do snapshot + rebasing.

### 4.4. Tipos escalares vs complexos

- **Escalares** (Int, Float, Boolean): o resultado é um valor imediato (i64/f64).
  Vira `TypedExprKind::Literal` directo. Sem snapshot, sem heap.
- **Complexos** (List, Struct, Tuple, Enum com payload, Text): o resultado é um
  ponteiro para dados no heap. Vira `HeapSnapshot` com bytes + rebasing.

### 4.5. Tabela de snapshots por módulo

Cada módulo compila com uma tabela de snapshots (`Vec<HeapSnapshotData>`). O
codegen emite esta tabela como dados estáticos no binário. Em load-time, o
runtime faz `memcpy` de cada snapshot para root_arena e rebase os ponteiros.

`HeapSnapshotData`:
- `bytes: Vec<u8>` — conteúdo bruto da arena temporária
- `rebase_offsets: Vec<usize>` — offsets dentro de `bytes` onde há ponteiros que
  precisam de rebasing
- `ty: Ty` — tipo do valor (para verificação de consistência)

### 4.6. Sharing

Se dois snapshots referenciam o mesmo objeto (ex: `@comptime let x := [1 2 3]`
usado em dois `@comptime` posteriores), o comptime pass detecta a partilha via
dataflow e emite um único snapshot para `x`, referenciado pelos dois usos. Isto
exige que o comptime pass execute numa arena partilhada para todo o módulo, não
uma arena por expressão.

## 5. Pipeline

### 5.1. Posição

```
lex → parse → module load → resolution → inference → monomorph → escape
  → tree shaking → comptime → lowering → optimize → emit → JIT/AOT → execução
```

O comptime pass vê a TAST (TypedModule), não o CLIF. Ele:

1. Identifica expressões `@comptime` na TAST
2. Verifica constness (dataflow — todos os valores são comptime-available?)
3. Compila a expressão usando o pipeline completo (infer → codegen → JIT) numa
   arena temporária dedicada
4. Executa via JIT no `kata-rt` real
5. Captura o resultado (bytes da arena temporária + offsets para rebasing)
6. Substitui o nó TAST por `HeapSnapshot { snapshot_id, ty }` ou `Literal`
7. Repete até fixpoint (sem novas expressões `@comptime`)

O Cranelift depois nunca vê a expressão original — vê só o snapshot/literal.

### 5.2. Arena temporária de comptime

O comptime pass cria uma arena temporária dedicada para executar as expressões
`@comptime` do módulo. Todos os snapshots do módulo são capturados desta arena
(para suportar sharing). Após capturar todos os snapshots, a arena temporária é
destruída — os bytes já foram copiados para a tabela de snapshots.

### 5.3. Pureza verification

Antes de avaliar, o comptime pass verifica que a expressão é pura: walk na TAST
da função a executar — se contém `ActionCall`, é impura → erro de compilação.
Funções puras podem chamar outras funções puras; a verificação é transitiva.

`@cache` também exige pureza — a função anotada deve ser pura. Mesma verificação.

## 6. Runtime

### 6.1. Comptime — load-time

```c
// Em load-time (antes da primeira função Kata executar):
kata_rt_load_snapshots(root_arena, snapshot_table, n_snapshots) -> snapshot_ptrs
```

Para cada snapshot:
1. `kata_rt_arena_alloc(root_arena, bytes.len)` → base_ptr
2. `memcpy(base_ptr, bytes, bytes.len)`
3. Para cada offset em `rebase_offsets`: `*(base_ptr + offset) += base_ptr`
4. Guardar `base_ptr` na tabela de snapshot_ptrs indexada por `snapshot_id`

O codegen lowera `HeapSnapshot { snapshot_id, ty }` para um load de
`snapshot_ptrs[snapshot_id]` — um ponteiro já válido na root_arena.

### 6.2. `@cache` — runtime

```c
kata_rt_cache_get_or_create(arena_handle, fn_id, capacity) -> cache_handle
kata_rt_cache_lookup(cache_handle, key_bytes, key_len) -> i64   // 0=miss, ptr=hit
kata_rt_cache_insert(cache_handle, key_bytes, key_len, value_ptr) -> ()
```

Hash table open-addressing, LRU via counter ou clock algorithm. `fn_id` identifica
qual função o cache pertence (para lazy alloc na primeira chamada). A key é o
hash dos bytes dos argumentos serializados via TypeShape.

## 7. AOT

O `kata build` (AOT) precisa embedar os snapshots no binário. Os snapshots viram
uma secção de dados estáticos no `.o` emitido por `cranelift-object`. Em
load-time, o shim C chama `kata_rt_load_snapshots` antes de `__kata_entry`.

A tabela de snapshots é emitida como um array de structs `{bytes_ptr, bytes_len,
rebase_offsets_ptr, rebase_offsets_len}` apontando para dados contíguos na mesma
secção. O linker resolve os ponteiros absolutos.

## 8. Fases

### Fase 1: `@comptime` em `let` top-level com resultado escalar ✅

- Parser reconhece `@comptime` antes de `let` top-level
- Comptime pass avalia expressão via JIT
- Resultado escalar → `Literal` na TAST
- Verificação de constness (dataflow)
- Pureza verification
- **DoD:** `@comptime let x := + 1 2` gera `x = 3` literal no binário. ✅
- **Commit:** `8c3d299`

### Fase 2: HeapSnapshot para tipos complexos ✅

- `HeapSnapshot` node na TAST
- Serialização type-aware (List, Tuple, Struct, Text, Sum com payload)
- `kata_rt_load_snapshot` em load-time com rebasing de ponteiros
- Codegen lowera `HeapSnapshot` para `kata_rt_get_snapshot(id)`
- **DoD:** `@comptime let t := [1 2 3]` gera lista no binário sem código de
  construção. ✅ `len`, `head`, `tail` funcionam sobre o snapshot carregado.
- **Commit:** `b7f7485`

### Fase 3: `@comptime` em call-site dentro de body

- Parser reconhece `@comptime` antes de expressão em posição de call
- Comptime pass avalia no contexto do body (verifica constness dos bindings locais)
- Substitui por snapshot/literal na TAST
- **DoD:** `let x := @comptime fibonacci 20 in ...` substitui por literal no binário.

### Fase 4: Ascription refined delega predicados complexos ao comptime

- Typeck detecta predicado complexo (`is_prime _`) em ascription refined
- Valor é comptime-available → delega ao comptime pass
- Comptime pass avalia predicado, retorna `Boolean`
- Typeck consome resultado: `True` → tipo refined directo, `False` → type error
- Comportamento do Fio 6 preservado para predicados triviais
- **DoD:** `5::Prime` com `is_prime` definido no módulo valida em compile-time.
  `4::Prime` produz type error. `n::PositiveInt` onde `n` é runtime → validação
  em runtime (sem mudança).

### Fase 5: `@cache{strategy: "LRU"}`

- Parser reconhece `@cache{strategy: "LRU"}` em definição de função
- Codegen emite cache lookup no prólogo, insert no epílogo
- Runtime: `kata_rt_cache_get_or_create/lookup/insert`
- Hash table open-addressing com LRU
- Key via TypeShape (reuso do Fio 11)
- Cache lazy-allocated em `caller_arena`
- Pureza verification (função `@cache` deve ser pura)
- **DoD:** `dobro :: Int => Int @cache{strategy: "LRU"}` com 100 chamadas a
  `dobro 5` executa o body 1 vez. Cache hit retorna resultado sem re-executar.

### Fase 6: AOT embedding

- Tabela de snapshots emitida como secção de dados no `.o`
- Shim C chama `kata_rt_load_snapshots` antes de `__kata_entry`
- `kata build` produz executável com snapshots embedados
- **DoD:** `kata build examples/comptime.kata` produz executável que executa
  sem o compilador e usa snapshots em load-time.

## 9. Não depende de

- `@parallel` (congelado — não afeta comptime ou cache)
- Fio 13 (Dict/Set — não necessário para cache; hash table é runtime nativo)
- Fio 15 (REPL — comptime funciona em JIT e AOT; REPL herda JIT)

## 10. Fora do escopo

- `@comptime` definition-site (hint) — removido. Só call-site explícito.
- Inferência automática de constness — o compilador não propaga sem `@comptime`.
- `@cache` com estratégias diferentes de LRU — apenas LRU no 1.0.
- Cross-fiber cache sharing — cache é fiber-local (`caller_arena`).
- Interpretação de TAST — sempre JIT-and-execute.

## 11. DoD do Fio

- `@comptime fatorial 10` substitui por literal `3628800` no binário.
- `@comptime range 1 100` substitui por HeapSnapshot (lista de 100 cons cells).
- `@comptime` com arg não-constante → erro de compilação.
- `5::Prime` com predicado complexo valida em compile-time via comptime pass.
- `dobro :: Int => Int @cache{strategy: "LRU"}` memoiza função pura repetida.
- `kata build` produz executável com snapshots embedados.