# Maquinaria Interna — HashMaps/HashSets do Compilador

Este documento cataloga as estruturas de dados internas (HashMaps, HashSets) que o
compilador usa para seu funcionamento — não as estruturas da linguagem Kata (Dict/Set
em Fio 13), mas a maquinaria interna do compilador Rust que implementa o pipeline.

Cada entrada descreve **o que é**, **para que serve**, **qual fio a cria**, e os
**porquês** e **invariantes** que o design deve respeitar desde o início.

---

## 1. TypeEnv — Resolução de Escopos

```rust
struct TypeEnv {
    bindings: HashMap<String, Ty>,
    parent: Option<Box<TypeEnv>>,
}
```

- **Fio:** 1 (kata-resolution + kata-inference)
- **Função:** Árvore de escopos para name resolution. `lookup(name)` percorre a
  cadeia de pais; `define(name, ty)` insere no escopo atual.
- **Porquê:** Toda inferência precisa resolver nomes. O TypeEnv é populado no
  resolution (Pass 0+1) e consumido no inference (Pass 2). Não sobrevive além do
  typeck — o TAST já carrega os tipos resolvidos em cada nó.
- **Invariantes:**
  - O prelude é injetado pelo `kata-module-loader`, não pelo typeck. O typeck
    recebe o TypeEnv já populado com Boolean, Int, Float, Text, Rational.
  - Cache de TypeEnv por módulo para prevenir ciclos (§3.5 do manual). Quando um
    módulo é solicitado, a infraestrutura verifica o cache primeiro.

### Cache de módulos (kata-module-loader)

```rust
struct ModuleLoader {
    cache: HashMap<PathBuf, Arc<ModuleArtifact>>,
    type_env_cache: HashMap<PathBuf, Arc<TypeEnv>>,
}
```

- **Fio:** 10 (sistema de módulos completo), mas cache básico desde Fio 1
- **Função:** Cache de módulos carregados e seus TypeEnv. Previne recompilação
  e quebra ciclos de import.
- **Porquê:** Sem cache, `import A` → `import B` → `import A` causaria loop
  infinito. Com cache, a segunda solicitação de A devolve a referência já
  compilada, interrompendo o ciclo.

---

## 2. DispatchTable — Despacho Múltiplo por Dominância

```rust
struct DispatchTable {
    // index por nome → lista de overloads (params estão dentro de OverloadInfo)
    entries: HashMap<String, Vec<OverloadInfo>>,
    // funções marcadas com @commutative (dispatch tenta args invertidos)
    commutative: HashSet<String>,
}

struct OverloadInfo {
    name: String,
    params: Vec<Ty>,
    ret: Ty,
    ffi_symbol: Option<String>,
    is_action: bool,
    is_generic: bool,        // Fio 7
    is_constructor: bool,    // smart constructors
    associative_neutral: Option<i64>,  // @associative(0) para TRMA
    // Fio 7 adiciona:
    // substitutions: Option<HashMap<String, Ty>>,
    // generic_ast_key: Option<String>,
    // generic_param_names: Vec<String>,
}
```

- **Fio:** 1 (nasce com scoring), 7 (múltiplas overloads + generics)
- **Função:** Tabela de overloads indexada por nome. O dispatcher coleta
  candidatos por nome, pontua por compatibilidade de tipos, e seleciona o de
  maior score. Empate → `AmbiguousDispatch`.
- **Porquê:** O scoring por dominância nasce em Fio 1 mesmo com 1 overload.
  O algoritmo não fica mais complexo com mais entradas — fica mais complexo
  quando existem ambiguidades, e isso só acontece com interfaces (Fio 7).
  Nascer com scoring evita retrofit.

### Score 4D — Categorias de Match

O scoring classifica cada par (arg, param) em uma de quatro categorias,
ordenadas lexicograficamente por prioridade:

```rust
struct Score {
    exact: usize,   // arg == param (tipo idêntico)
    alias: usize,   // arg é alias de param via alias_registry (Fio 5)
    refined: usize, // arg é subtipo refinado de param (Fio 6: PositiveInt <: Int)
    iface: usize,   // arg implementa param (Fio 7: Int implementa NUM)
    is_generic_origin: bool,  // tiebreak: concreto (false) vence genérico (true)
}
```

**Ordenação:** lexicográfica decrescente. Mais `exact` vence; empate em
`exact` → mais `alias` vence; empate em `alias` → mais `refined`; empate em
`refined` → mais `iface`; empate total → concreto vence genérico.

**Evolução por fio:**

| Fio | exact | alias | refined | iface | is_generic |
|-----|-------|-------|---------|-------|------------|
| 1   | ✅    | —     | —       | —     | —          |
| 5   | ✅    | ✅    | —       | —     | —          |
| 6   | ✅    | ✅    | ✅      | —     | —          |
| 7   | ✅    | ✅    | ✅      | ✅    | ✅         |

Em Fio 1, só `exact` é não-zero. As outras dimensões são sempre 0, mas a
estrutura do Score e a ordenação lexicográfica já estão prontas — adicionar
uma dimensão nova é preencher um campo que já existe, não mudar o algoritmo.

### Algoritmo de Resolução

```
resolve(name, args):
    1. FILTRAR: para cada overload com mesma arity:
       score = match_score(args, params)
       se score.is_compatible(args.len()):
           adiciona candidato (overload, score)
    2. COMMUTATIVE: se 0 candidatos e @commutative e arity == 2:
       tenta args invertidos (uma única vez)
    3. ORDENAR: lexicográfico decrescente por Score
    4. TOPO ÚNICO → Ok(info)
    5. EMPATE → AmbiguousDispatch
```

O `match_score` itera posição-a-posição. Para cada par `(arg, param)`:
- `arg == param` → `exact++`
- alias match → `alias++` (Fio 5)
- refined subtype → `refined++` (Fio 6)
- interface compatível → `iface++` (Fio 7)
- nenhum → `Score::incompatible()` (todos zero, descarta candidato)

Se `exact + alias + refined + iface != args.len()`, o candidato é
incompatível e descartado.

### Commutative

Funções marcadas com `@commutative` (ex: `=`, `+`) têm um short-circuit: se
nenhum candidato compatível é encontrado com os args originais e a função é
comutativa com arity 2, o dispatcher tenta args invertidos. Isto resolve
casos como `Float == Int` quando só existe overload `Int == Float`.

### Invariantes

- Actions são registradas com `vec![]` (zero params) no Pass 1. O typeck
  faz bypass do dispatch table para actions com args (ex: `fork!(consumer! ch)`
  — o action `consumer` tem 0 params declarados mas recebe 1 arg via
  thread-local `fork_arg!()`).
- O `substitutions` (Fio 7) é `None` para funções não-genéricas e `Some(map)`
  para instâncias monomorfizadas.
- FFI e smart constructors **sempre mantidos** no tree shaking — não são
  alcançáveis via refs normais mas são necessários.

### Overload counters

```rust
struct TypeChecker {
    // ...
    overload_counters: HashMap<String, usize>,
}
```

- **Fio:** 7 (generics)
- **Função:** Contador de overloads geradas por monomorphização para nomes
  únicos. Evita colisão quando múltiplas instâncias do mesmo genérico são
  especializadas.
- **Porquê:** Sem contadores, `soma_T_Int` e `soma_T_Float` poderiam colidir
  se o naming scheme não for determinístico.

---

## 3. InterfaceRegistry — Interfaces e Implementações

```rust
struct InterfaceRegistry {
    interfaces: HashMap<String, InterfaceInfo>,
    impls: Vec<ImplEntry>,
}

struct InterfaceInfo {
    name: String,
    supertraits: Vec<String>,
    signatures: Vec<Signature>,
}

struct ImplEntry {
    type_name: String,
    interface_name: String,
    methods: Vec<MethodImpl>,
}
```

- **Fio:** 7 (interfaces, generics, dispatch com múltiplas overloads)
- **Função:** Catálogo de interfaces declaradas e suas implementações.
  `register` retorna `Result` — detecta ciclos de supertraits via DFS com
  `HashSet<String>` visiting → `TypeError::CircularInterface`.
- **Porquê:** Interfaces com herança (`SHOW : EQ`) podem formar ciclos se
  mal declaradas. O DFS com HashSet de visiting detecta isso no registro,
  não em runtime.
- **Invariantes:**
  - `type_name_str` DEVE ter braço para `Ty::Interface(name) => name`.
    Sem isso, validação de assinatura compara com string vazia → sempre falha.
    Uma linha faltando quebra toda validação de interfaces.
  - `register_impl` DEVE aceitar `ImplEntry` mesmo quando a interface não existe
    no registry local do módulo do usuário. Interfaces do prelude (ex: `NUM`,
    `SHOW`) são mergidas *depois* do resolve do módulo do usuário. Se
    `register_impl` descarta o `ImplEntry` porque a interface não existe
    localmente, a implementação nunca é registrada → dispatch via interface
    não encontra a implementação. Sintoma: `[resolution] warning: interface
    'NUM' não declarada` + dispatch falha. (Fase 9: `Complex implements NUM`
    em `stdlib/complex.kata`.)

---

## 4. EscapeCtx — Escape Analysis

```rust
struct EscapeCtx {
    closure_vars: HashSet<String>,
    escaping: HashSet<usize>,           // endereços TAST que escapam
    heap_vars: HashMap<String, usize>,  // aliases de heap-values
    shared_exprs: HashSet<usize>,        // endereços que escapam para heap
}
```

- **Fio:** 9 (closures, escape analysis, ARC, TRMA)
- **Função:** 4 passes sobre a TAST:
  1. **Return:** closures em retorno de funções puras
  2. **Sintático:** análise de captura (free vars)
  3. **Alias:** rastreamento de aliases de heap-values
  4. **Promoção:** marca `CaptureStorage::Stack` vs `Heap`
- **Porquê:** Decisão arena vs ARC. Valores que não escapam → arena (bump alloc,
  zero overhead, liberada no epílogo). Valores que escapam (via canal, retorno
  de closure, store em heap) → ARC na shared arena (refcount, sobrevive à fiber).
- **Invariantes:**
  - **Shared é recursivo:** se `expr` é marcado shared (ARC), todos os seus
    sub-heap-values também são shared. Um ARC pointer nunca tem fields que
    são arena pointers. Isso é garantido pelo escape analysis: `mark_escape`
    recursa em sub-expressões heap aninhadas.
  - Este invariant protege o `walk_and_decref` — ele pode caminhar fields de
    um ARC pointer sem risco de alcançar arena pointers.

### shared_exprs no TypedModule

```rust
struct TypedModule {
    // ...
    shared_exprs: HashSet<usize>,
    escaping: HashSet<usize>,
    type_table: HashMap<u32, TypeShape>,
    type_id_map: HashMap<Ty, u32>,
}
```

- **Fio:** 9 (escape), 5 (type_id)
- **Porquê:** `shared_exprs` é anexado ao `TypedModule` (não ao `TypedExpr`) para
  zero mudança na estrutura do TAST. O codegen consulta
  `module.shared_exprs.contains(&addr)` antes de decidir `arena_alloc` vs
  `alloc_arc`.
- **⚠️ Lição:** ao adicionar campo a `TypedModule`, TODOS os sites de
  construção precisam ser atualizados. Usar `grep -rn "shared_exprs.*HashSet::new()"`
  para encontrar todos.

---

## 5. TypeIdAssignment — Reflexão Estrutural Runtime

```rust
struct TypeIdAssignment {
    table: HashMap<u32, TypeShape>,   // type_id → shape (runtime)
    ids: HashMap<Ty, u32>,            // Ty → type_id (compile-time)
}

struct TypeCollector {
    seen: HashSet<Ty>,
    next_id: u32,
}
```

- **Fio:** 1 (TypeShape), mas atribuição completa é pós-escape
- **Função:** Atribui `type_id: u32` incremental para cada `Ty` distinto (por
  Hash+Eq) no módulo. Constrói duas tabelas:
  - `TypeIdMap` (Ty→u32): o codegen usa para descobrir o type_id de um Ty
  - `TypeTable` (u32→TypeShape): o runtime usa para reflexão estrutural
- **Porquê:** O decref precisa saber o `TypeShape` do valor que está liberando
  para fazer walk type-directed (liberar filhos ARC recursivamente). O type_id
  é a chave que conecta o ponteiro runtime ao seu formato.
- **Invariantes:**
  - `assign_type_ids` roda **após** escape analysis (para refletir o módulo
    final) e **antes** de tree shaking (tree shaking pode remover funções,
    mas a type_table já capturou todos os tipos).
  - `to_shape()` DEVE ser tolerante a tipos não-resolvidos (Generic → User,
    Interface → User, InferVar → Unit). `assign_type_ids` roda durante typeck,
    antes da monomorfização — Generic/Interface podem aparecer. Panic é
    para invariantes de codegen, não de typeck.
  - `type_id` é estrutural: mesmo `Ty` (por Hash+Eq) = mesmo `type_id`,
    independente de arena/ARC. `Tuple(Int, Float)` na arena e como ARC
    compartilham ID. Correto: `TypeShape` é o mesmo.

---

## 6. Type Table Runtime (Global)

```rust
static TYPE_TABLE: LazyLock<Mutex<HashMap<u32, TypeShape>>>;
```

- **Fio:** 1 (registro), 9 (consumo pelo decref)
- **Função:** Tabela global `type_id → TypeShape` no runtime. Registrada pelo
  driver (Rust-to-Rust, não C-ABI) antes da execução.
- **Porquê:** `TypeShape` tem `Box`, `String`, `Vec` — sem layout C-ABI estável.
  Serializar através da fronteira C-ABI exigiria formato binário + struct
  espelho C-compatible. Mais trabalho, mais frágil. Solução: o driver (Rust)
  chama `kata_rt::register_type_table(table)` diretamente antes de executar.
  O código JIT não precisa registrar tipos — a table já está no runtime.
- **Invariantes:**
  - O codegen **não emite** nenhuma instrução para a type table na Fase 1.
    A Fase 5 (value table) é que fará o codegen emitir `register_type(ptr,
    type_id)` após cada alocação ARC.

---

## 7. Value Table Runtime (3 tabelas)

```rust
// Opção C — três tabelas separadas
static SHARED_ARC: LazyLock<Mutex<HashMap<i64, u32>>>;     // ARC pointers
static SHARED_ARENA: LazyLock<Mutex<HashMap<i64, u32>>>;  // arena shared (entry)
// per-fiber: owned pelo FiberEntry
PER_FIBER_ARENA: HashMap<i64, u32>;  // arena per-fiber (TLS)
```

- **Fio:** 9 (value table), mas design maduro é pós-Fase 5
- **Função:** Side table `ptr → type_id` para reflexão runtime. O codegen emite
  `kata_rt_register_type(ptr, type_id)` após cada alocação. O decref consulta
  para fazer walk type-directed.
- **Porquê:** O header ARC (24 bytes: fn_ptr@0, refcount@8, data_size@16,
  data@24) não tem espaço para type_id. Embedir type_id no header exigiria
  expandir para 32 bytes (mudar todos os offsets de field access) ou trocar
  data_size (decref perde o size para dealloc). A side table é não-invasiva.
- **Decisão: 3 tabelas (Opção C):**

  O design evoluiu através de três iterações até chegar na solução final. O motivo:

  | Problema | Causa |
  |---|---|
  | 1 tabela global | Arena pointers per-fiber morrem com a fiber, entries ficam órfãs |
  | 2 tabelas (ARC + arena) | ARC pointer alocado dentro de fiber com `CURRENT_ARENA` ativa → registra na per-fiber → fiber termina → entry perdida → decref não encontra → leak |

  3 tabelas resolvem: ARC sempre vai para `SHARED_ARC` (global, independente de
  `CURRENT_ARENA`). Arena shared vai para `SHARED_ARENA`. Arena per-fiber vai
  para `PER_FIBER_ARENA` (TLS, morre com a fiber).

  | Solução | Prós | Contras |
  |---|---|---|
  | 2 FFI (A) | Correto, sem stale | `typeof` não distingue ARC de arena |
  | Struct Entry (B) | Explícito, extensível | 8 bytes/entry vs 4 |
  | **3 tabelas (C) ✅** | 4 bytes/entry; `decref` consulta só ARC; sem ambiguidade | 3 tabelas para gerenciar |

- **Invariantes:**
  - ARC pointers são sempre registrados em `SHARED_ARC`, **independente** de
    `CURRENT_ARENA`. O codegen sabe que é ARC (escape analysis marcou
    `is_shared=true`) e emite `RegisterType` (não `RegisterTypeArena`).
  - `decref` consulta apenas `SHARED_ARC` — arena pointers não precisam de
    decref (bump reset cuida).
  - `typeof` consulta `SHARED_ARC` → `SHARED_ARENA` → `PER_FIBER_ARENA`
    (fallback chain).
  - `clear_value_table()` entre execuções cobre o entry point (poucas alocações).

---

## 8. LowerCtx — Contexto de Lowering (a struct mais complexa)

```rust
pub struct LowerCtx {
    // --- bindings ---
    bindings: HashMap<String, Value>,

    // --- closures ---
    closure_fn_names: HashMap<Value, String>,   // env_ptr → closure_name
    closure_sigs: HashMap<String, IrSignature>, // sig_key → signature
    extra_signatures: Vec<IrSignature>,
    type_sig_ids: HashMap<String, String>,      // dedup de assinaturas

    // --- escape/ARC ---
    arc_values: HashSet<Value>,      // ponteiros ARC
    arena_values: HashSet<Value>,   // ponteiros arena
    shared_exprs: Option<Rc<HashSet<usize>>>, // endereços TAST que escapam

    // --- type id ---
    type_id_map: Option<Rc<HashMap<Ty, u32>>>,

    // --- function-level ---
    current_params: Vec<Value>,
    current_insts: Vec<Inst>,
    blocks: Vec<Block>,
    // ...
}
```

- **Fio:** 1 (básico), 9 (closures, escape, ARC), 5 (type_id)
- **Função:** Contexto de lowering function-level. Acumula instruções, blocos,
  bindings, e metadados enquanto baixa uma função/action/entry.
- **Princípio de design (Arthur):** LowerCtx é um builder **function-level**,
  não module-level. Não acumula responsabilidades de module (como
  `pending_functions`). `lower_module` faz pré-passos separados para coletar
  closures. Razão: separação de concerns — LowerCtx constrói uma função,
  `lower_module` orquestra o módulo.

### Cada campo e seu porquê:

#### `closure_fn_names: HashMap<Value, String>`
- Mapeia `env_ptr → closure_name`. Permite que `lower_apply` emita `Inst::Call`
  direto pelo nome em vez de `CallIndirect` via fn_ptr.
- **Invariantes:**
  - **Offset 0 do env record SEMPRE tem fn_ptr.** Mesmo quando usando `Call`
    direto (que não lê fn_ptr), o store DEVE ser emitido. O `Call` direto é
    uma otimização que pula o load, não substituto do store. Closures recebidas
    via canal (`<! ch`) não estão em `closure_fn_names` — fazem `CallIndirect`
    lendo fn_ptr do offset 0. Se o store foi omitido, `CallIndirect` carrega 0
    e crasha.
  - Inicializado em `new()`, `new_with_params()`, e `from_parent()`.

#### `closure_sigs` e `extra_signatures`
- `closure_sigs` guarda assinaturas para CallIndirect de closures conhecidas.
- `extra_signatures` acumula assinaturas criadas on-the-fly (closures sem nome
  em `closure_fn_names`).
- **⚠️ Pitfall:** `ctx.finish()` consome `ctx` por move. Não se pode fazer
  `ctx.finish(...)` e depois acessar `ctx.extra_signatures`. Extrair com
  `std::mem::take(&mut ctx.extra_signatures)` ANTES de `ctx.finish()`.

#### `type_sig_ids: HashMap<String, String>`
- Dedup de assinaturas para CallIndirect. Closures com a mesma assinatura
  compartilham a mesma `IrSignature` (necessário porque o Cranelift emit
  importa SigRefs pelo id).

#### `arc_values: HashSet<Value>` e `arena_values: HashSet<Value>`
- Rastreamento paralelo: quais Values são ponteiros ARC vs arena.
- `arc_values`: ponteiros alocados via `alloc_arc` (refcount, shared arena).
  FieldAccess soma 24 ao offset (pula header ARC de 24 bytes).
- `arena_values`: ponteiros alocados via `arena_alloc` (bump alloc, per-fiber).
  FieldAccess soma 0 (sem header).
- **Porquê:** O emit precisa saber se um ponteiro tem header ARC (offset +24
  para field access) ou não (offset +0). Sem rastreamento, o codegen não sabe
  qual offset aplicar.

#### `shared_exprs: Option<Rc<HashSet<usize>>>`
- Endereços TAST que escapam (marcados pelo escape analysis). Consultado via
  `is_shared(addr)` antes de decidir `arena_alloc` vs `alloc_arc`.
- `Option<Rc<...>>` porque pode ser `None` em contextos sem escape analysis
  (ex: testes isolados).

#### `type_id_map: Option<Rc<HashMap<Ty, u32>>>`
- Mapa `Ty → type_id` para emitir `register_type(ptr, type_id)` após alocações
  ARC. `emit_register_type` é o helper que emite a CallFfi.
- **Porquê:** Sem este mapa, o codegen não tem como descobrir o `type_id` de
  um `Ty` — o `assign_type_ids` retorna `TypeIdAssignment { table, ids }`, mas
  se `ids` for descartado em pass2, o codegen não tem como descobrir o `type_id`.
  Solução: armazenar `ids` no `TypedModule` e propagar via `Rc` para o `LowerCtx`.

### `kata_refs`/`kata_ids: HashMap<FuncKey, ...>`
- **Mudança (Fase 9):** Agora usam `FuncKey = (String, Vec<Ty>, Ty)` como chave
  (mesma mudança do `user_functions` no EmitCtx). O lookup por nome `String`
  foi substituído pelo lookup por chave composta extraída de `callee.ty`
  (params dos args, ret de `expr.ty`). Helper: `func_key_from_callee`.

### FieldAccess/IndexAccess — carregamento com tipo CLIF correto
- **Bug latente (Fase 5→9):** `FieldAccess` e `IndexAccess` carregavam sempre
  como `I64` (`load(I64, ...)`) hard-coded. Para structs com campos `Int`
  (todas as fases anteriores), isso funcionava porque `Int` mapeia para `I64`.
  Mas quando `Complex` (campos `Float`) foi exercitado em funções Kata puras,
  o `load(I64)` produzia `i64` em vez de `f64`, causando mismatch na chamada
  FFI `kata_rt_fadd(f64, f64) -> f64` → `Cranelift("Verifier errors")`.
- **Fix:** `load(ty_to_clif(&expr.ty), ...)` — usa o tipo da expressão.
- **Diagnóstico:** IR dump via `eprintln!("{}", ctx.func.display())` no
  `map_err` de `define_function` revelou o mismatch `load.i64` vs `f64`.

### repr_synthesis — `kata_rt_float_to_text` para Float
- **Bug latente (Fase 5→9):** `repr_synthesis.rs` usava `kata_rt_int_to_text`
  (i64 → text) para converter campos `Float`, com um TODO stale dizendo
  "adicionar kata_rt_float_to_text no runtime". Mas `kata_rt_float_to_text`
  já existia no runtime e já estava registrada no `ffi_registry`.
- **Fix:** trocar para `kata_rt_float_to_text` (f64 → text).
- **Lição:** TODOs em código de síntese podem ficar stale quando a feature
  mencionada já foi implementada. Sempre verificar se a FFI mencionada já
  existe no runtime com `grep -rn "float_to_text\|kata_rt_float" crates/kata-rt/src/`.

---

## 9. EmitCtx — Emissão Cranelift

```rust
struct EmitCtx {
    values: HashMap<Value, cranelift::Value>,  // IR Value → Cranelift SSA
    sig_refs: HashMap<String, cranelift::SigRef>,
    user_functions: HashMap<String, cranelift::FuncId>,
    // ...
}
```

- **Fio:** 1 (básico), 9 (CallIndirect)
- **Função:** Contexto de emissão. Mapeia IR Values para Cranelift SSA values,
  mantém SigRefs para CallIndirect, e FuncIds para chamadas diretas.

### `values: HashMap<Value, cranelift::Value>` — O HashMap mais perigoso

- **⚠️ Invariante crítico:** Para `block_idx > 0`, **SEMPRE** fazer `stack_load`
  dos merge slots para todos os block params. NÃO reusar o valor Cranelift do
  HashMap global mesmo se o IR `Value(N)` já está mapeado.

  **Porquê:** O `emit_ctx.values` é um HashMap global. Se um block param (phi)
  tem o mesmo IR `Value(N)` que uma instrução dst em outro bloco, o emit pode
  pular o `stack_load` e reusar o valor Cranelift original. Se o bloco original
  não dominava o bloco atual → Cranelift verifier error "uses value from
  non-dominating inst". O inliner exacerba: ao inlinear função com múltiplos
  Returns (ex: try_propagate), cada Return vira Jump para merge_block com args
  diferentes.

  **Correto:** Para `block_idx > 0`, sempre `stack_load`. O `stack_load` produz
  valor SSA fresco local ao bloco — correto para phi semantics.

- **Porquê o HashMap global é frágil:** Cranelift SSA values têm dominância
  implícita — um valor definido no block A só é válido em blocos dominados
  por A. O HashMap global ignora dominância. Block params (phis) precisam
  de valores frescos por bloco.

### `sig_refs: HashMap<String, SigRef>`
- **⚠️ Cranelift 0.125/0.133:** `call_indirect` espera `SigRef` (owned), não
  `&Signature` ou `&SigRef`. Obter via `bcx.import_signature(clif_sig)` dentro
  do `FunctionBuilder` context. Armazenar no HashMap e dereference com `*sig`.

### `user_functions: HashMap<FuncKey, FuncId>`

```rust
type FuncKey = (String, Vec<Ty>, Ty);  // (nome, tipos de entrada, tipo de saída)
```

- Pré-declaração de funções do usuário no Cranelift. `Inst::Call` e
  `Inst::FnPtr` consultam este mapa. Sem pré-declaração, o callee fica com
  assinatura vazia → `inst_results` retorna 0 elementos → panic.
- **Mudança (Fase 9):** A chave mudou de `String` para `(String, Vec<Ty>, Ty)`.
  Múltiplas overloads do mesmo método em `implements` com corpo Kata (ex:
  `+ :: Complex Complex => Complex` e `+ :: Complex Int => Complex`) produzem
  `FunctionDef` com o mesmo `name`. Com chave `String`, a segunda sobrescreve
  a primeira no `symbol_table` → `module.get_name(name)` retorna o FuncId
  errado → `Cranelift("Verifier errors")`. A chave composta `(nome, params, ret)`
  distingue as overloads. `Ty` já implementa `Hash + Eq`.
- **Nome no Cranelift é plumbing interno:** `__kata_fn_N` com contador
  incremental. O `FuncId` é passado diretamente para `define_function_body`
  e `define_kata_action`, eliminando o lookup por nome (`module.get_name`).
- **Princípio (Arthur):** "funções do usuário e da linguagem devem ter o mesmo
  nível de reconhecimento." A chave composta trata FFI e Kata igualmente —
  mangling de nomes (`__kata_impl__...`) foi rejeitado por ser lossy e frágil.

---

## 10. Scheduler — Runtime CSP

```rust
pub struct Scheduler {
    run_queue: VecDeque<FiberId>,
    blocked: HashMap<FiberId, BlockReason>,
    pending_wakes: HashSet<FiberId>,
    timers: TimerQueue,
    current_fiber: Option<FiberId>,
}
```

- **Fio:** 11 (CSP, scheduler multithread)
- **Função:** Scheduler explícito (não TLS global). `run_queue` tem fibers
  prontas, `blocked` rastreia fibers esperando em canais/timers, `pending_wakes`
  implementa semântica unpark.
- **Porquê struct explícita:** TLS global impede work-stealing entre threads e
  dificulta testes isolados. Struct explícita permite passar `&Scheduler` como
  parâmetro, tornando testes triviais e desbloqueando multi-thread sem
  refatoração. TLS é usado apenas para `yield` (acesso implícito de dentro de
  FFI — o código JIT não tem como receber `&Scheduler`).

### `pending_wakes: HashSet<FiberId>` — semântica unpark

- **Porquê HashSet, não VecDeque:** `make_ready(id)` é idempotente — múltiplas
  chamadas não criam entradas duplicadas. Replica `std::thread::park`/`unpark`:
  se `unpark` é chamado antes de `park`, o `park` não bloqueia.
- **Como funciona:** `yield_to_scheduler()` verifica
  `pending_wakes.remove(&my_id)` antes de suspender. Se existe, não suspende
  (o wake já foi recebido).

### `blocked: HashMap<FiberId, BlockReason>`

- Rastreia por que cada fiber está bloqueada (esperando canal, esperando
  timer, etc.). Necessário para deadlock detection: se `run_queue` está vazia
  e `blocked` não está vazia, ou há deadlock ou há timers pendentes.
- **⚠️ Deadlock detection precisa de timer awareness:** antes de declarar
  deadlock, verificar `has_pending_timers()`. Se há timers, `sleep(1ms)` +
  `continue` — a timer thread vai chamar `make_ready` quando expirar.

---

## 11. Tree Shaking — Reachability

```rust
// worklist algorithm
reachable: HashSet<String>,
worklist: Vec<String>,
```

- **Fio:** 1 (kata-tree-shaking, após escape analysis)
- **Função:** Dead code elimination. Worklist algorithm: raízes = actions +
  entry, coleta refs via `collect_refs` (Apply, ActionCall, Ident), remove
  funções mortas antes do lowering.
- **Invariantes:**
  - FFI e smart constructors **sempre mantidos** — não são alcançáveis via
    refs normais mas são necessários.
  - Tree shaking roda **após** escape analysis e type_id assignment (a
    type_table já capturou todos os tipos, tree shaking pode remover funções
    sem perder type info).

---

## 12. Monomorphização

```rust
struct TypeChecker {
    // ...
    mono_requests: Vec<MonoRequest>,
    mono_cache: HashMap<(String, String), String>,  // (func, type_sig) → mono_name
}
```

- **Fio:** 7 (generics)
- **Função:** Cache de instâncias monomorfizadas. `(func_name, type_signature)`
  → nome da versão especializada. Evita duplicar especialização do mesmo
  genérico com os mesmos tipos.
- **Porquê:** Sem cache, `map_T_Int` poderia ser gerada múltiplas vezes para
  diferentes call sites com os mesmos tipos. O cache deduplica.

---

## 13. Collect Lambdas — Pointer Identity

```rust
fn collect_lambdas(module: &TypedModule) -> HashMap<usize, String>
// key = std::ptr::addr_of!(expr.kind) as usize
```

- **Fio:** 9 (closure lowering)
- **Função:** Pré-passo que coleta todas as lambdas do módulo e atribui nomes
  únicos (`__closure_N`). Usa pointer identity (endereço de `expr.kind` no
  TAST) como key.
- **Porquê é sound:** O TAST é imutável durante o lowering — endereços não
  mudam. Permite identificar cada lambda unicamente sem adicionar campos ao
  `TypedExpr`.
- **⚠️ Pitfall (addr mismatch):** `lower_lambda` pode receber `body: &TypedExpr`
  e calcular `addr_of!(body)` — endereço do parâmetro local no stack frame,
  não do TAST original. `collect_lambdas` usa `addr_of!(expr.kind)`.
  **Solução:** passar `lambda_addr: usize` explicitamente de `lower_typed_expr`
  (`addr_of!(expr.kind) as usize`) para `lower_lambda`. Sem isso, closures
  let-bound nunca são encontradas no pré-passo.

---

## 14. Lazy Specialization

```rust
pure_refs: HashMap<PureKey, FuncId>,
ffi_ids: HashMap<String, FuncId>,
```

- **Fio:** 7+ (otimização, pós-monomorphização)
- **Função:** `pure_refs` cache de referências para funções puras especializadas
  (lazy specialization). `ffi_ids` mapeia nomes de FFI para FuncIds no Cranelift.
- **Porquê:** Permite especialização sob demanda — só especializa quando o
  call site é atingido, não ahead-of-time.

---

## 15. Error Wiring — Walkers

```rust
fn check_unique_names(
    param_names: &HashSet<&str>,
    seen: &mut HashSet<String>,
    // ...
)
```

- **Fio:** 1 (resolution)
- **Função:** Detecção de duplicatas em nomes de parâmetros e bindings.
  `param_names` é o conjunto de nomes já vistos; `seen` acumula novos.
- **Porquê HashSet:** Inserção e lookup são O(1). Nomes duplicados causam
  erro de tipo — o HashSet detecta em tempo constante.

---

## 16. Comptime

```rust
struct ComptimePass {
    config: ComptimeConfig,
    // cache de resultados de avaliação comptime
    evaluated: HashMap<String, ComptimeValue>,
}
```

- **Fio:** 12 (comptime, @cache_strategy)
- **Função:** Cache de expressões avaliadas em compile-time via JIT-and-execute.
  `@cache_strategy` (memoização LRU) usa este cache para evitar reavaliação.
- **Porquê:** Comptime via JIT-and-execute compila via pipeline normal e executa
  no runtime real. Zero duplicação de semântica — o código comptime usa as
  mesmas FFIs, a mesma arena, o mesmo typeck. O cache evita recompilar/reexecutar
  a mesma expressão.

---

## 17. Ascription Hint Pipeline — Ret-directed Dispatch e Grouped Barrier

A ascription `expr::Type` não é apenas uma anotação pós-inferência. Em Kata4
ela participa ativamente do dispatch através de três mecanismos que Kata5
precisa implementar quando chegar aos fios relevantes (Fio 6+).

### 17.1. Ret-directed dispatch

```rust
fn infer_with_hint(&mut self, expr: &Spanned<Expr>, hint_ret: Option<&Type>)
```

- **Fio:** 6 (ascription refined), mas útil desde Fio 1 para operações
  polimórficas como `/`
- **Função:** Quando a ascription anota uma aplicação de função (ex:
  `(/ 1 3)::Int`), o tipo anotado propaga como `hint_ret` para
  `DispatchTable::resolve`. O dispatch filtra sobrecargas cujo tipo de
  retorno é compatível com o hint.
- **Porquê:** Sem ret-directed dispatch, `(/ 1 3)::Int` despacha pela
  primeira sobrecarga de `/` com args `[Int, Int]` — que é `idiv` (ret Int).
  Mas `(/ 1.0 3.0)::Float` precisa selecionar `fdiv` (ret Float), não `idiv`.
  O hint de retorno desambigua sobrecargas que têm os mesmos tipos de
  argumento mas retornos diferentes.
- **Exemplo:**

```kata
(/ 1 3)::Int         # 0 — dispatch seleciona idiv (ret Int) ✓
(/ 1.0 3.0)::Float   # 0.333... — dispatch seleciona fdiv (ret Float) ✓
(/ 1 3)::Rational    # erro — nenhuma sobrecarga de / com args [Int Int] retorna Rational
```

- **Implementação Kata4:** `infer_ascription` chama `infer_with_hint(expr,
  Some(&ann_ty))`. O hint atravessa `Apply` e é passado para
  `dispatch_table.resolve(name, args, Some(hint_ret))`. O dispatcher
  pontua não só por compatibilidade de args, mas também por compatibilidade
  de retorno com o hint.
- **Invariantes:**
  - O hint é `Option<&Type>` — `None` quando não há ascription. O dispatch
    com `None` ignora o retorno (comportamento actual do Kata5).
  - O hint não força o retorno — apenas filtra. Se nenhuma sobrecarga
    retorna o tipo esperado, é erro claro, não coerção implícita.

### 17.2. Grouped como barreira de hint

```rust
// Grouped(inner) onde inner não é Grouped = strip (hint atravessa)
// Grouped(Grouped(...)) = barrier (avalia sem hint, depois converte)
```

- **Fio:** 6 (ascription refined), mas o mecanismo é do parser
- **Função:** Parênteses extras forçam avaliação independente antes da
  ascription. Um nível de `Grouped` é transparente ao hint (strip). Dois ou
  mais níveis de `Grouped` são barreira: o mais interno avalia sem hint,
  dissolve, e o externo converte o resultado.
- **Porquê:** `(/ 1 3)::Rational` falha porque o hint diz "quero Rational"
  mas nenhuma sobrecarga de `/` com args `[Int, Int]` retorna Rational.
  `((/ 1 3))::Rational` funciona: o `Grouped` interno avalia `/ 1 3`
  **sem hint** → resulta em `Int 0` (idiv default), depois o externo
  converte `Int 0 → Rational` via `from_int`.

```kata
(/ 1 3)::Rational     # erro — hint Rational, nenhuma sobrecarga de / retorna Rational
((/ 1 3))::Rational  # 1/3 — Grouped interno avalia sem hint → Int 0, depois from_int converte
```

- **Implementação Kata4:** `infer_ascription` detecta `Grouped(Grouped(...))`
  e desempacota até o `Grouped` mais interno. Inferir o expr interno sem
  hint, depois chamar `convert_typed_expr(inner, ann_ty)`.
- **Invariantes:**
  - `Grouped(inner)` onde inner **não é Grouped** = strip — cai no fluxo
    padrão (hint atravessa).
  - `Grouped(Grouped(...))` = barrier — cada nível extra é uma nova barreira.
  - O strip é o comportamento default; o barrier é a exceção.

### 17.3. convert_typed_expr — conversão runtime via dispatch

```rust
fn convert_typed_expr(
    inner: TypedExpr,
    target_ty: &Type,
    span: Span,
) -> TypeResult<TypedExpr>
```

- **Fio:** 6 (grouped barrier precisa de conversão)
- **Função:** Converte um `TypedExpr` de `from_ty` para `target_ty` via
  dispatch normal. Procura uma função de conversão no `DispatchTable`
  (ex: `from_int`, `from_float`, `to_float`) que aceite `from_ty` e retorne
  `target_ty`.
- **Tabela de conversões conhecidas (Kata4):**

```rust
match (&inner.ty, target_ty) {
    (Int, Rational) => "from_int",
    (Float, Rational) => "from_float",
    (Rational, Float) => "to_float",
    _ => TypeMismatch,
}
```

- **Porquê:** O barrier de `Grouped` produz um valor de tipo `from_ty`
  que precisa ser convertido para `target_ty` da ascription. Sem
  `convert_typed_expr`, o barrier não tem como entregar o tipo esperado.
- **Invariantes:**
  - Literais têm caminho compile-time (não passam por dispatch) —
    `Int(n) → Rational(n.to_string())` direto na TAST.
  - Não-literais passam por dispatch — `from_int`, `from_float`, `to_float`
    são funções normais no `DispatchTable`.
  - Se não há função de conversão conhecida, é `TypeMismatch` — não há
    coerção implícita.

### Resumo dos três mecanismos

| Mecanismo | Onde atua | Fio |
|---|---|---|
| Ret-directed dispatch | `DispatchTable::resolve` recebe `hint_ret` | 6 |
| Grouped barrier | `infer_ascription` detecta `Grouped(Grouped(...))` | 6 |
| convert_typed_expr | Barrier chama conversão via dispatch | 6 |

Os três são complementares: ret-directed dispatch resolve ambiguidade de
retorno, grouped barrier força avaliação independente, convert_typed_expr
converte o resultado do barrier para o tipo esperado. Sem os três, a
ascription não consegue desambiguar sobrecargas polimórficas.

---

## 9. Codegen de Coleções (Fio 8, Fases 6-7)

### FFI symbols do runtime

**List (`crates/kata-rt/src/list.rs`):**
- `kata_rt_list_nil()` → 0 (null pointer)
- `kata_rt_list_cons(head, tail, arena_handle)` → ponteiro para Cons cell (16 bytes: head@0, tail@8)
- `kata_rt_list_is_empty(ptr)` → 0/1
- `kata_rt_list_head(ptr)` → load do offset 0
- `kata_rt_list_tail(ptr)` → load do offset 8
- `kata_rt_list_len(ptr)` → SMI-tagged (`(count << 1) | 1`)
- `kata_rt_list_get_checked(ptr, idx)` → Result (SMI-tagged Int ou box Err)
- `kata_rt_list_contains(ptr, item)` → 0/1

**Array (`crates/kata-rt/src/array.rs`):**
- `kata_rt_array_alloc(len, arena_handle)` → ponteiro (header 8 bytes + data len*8)
- `kata_rt_array_len(ptr)` → SMI-tagged (load do offset 0, tagged)
- `kata_rt_array_get(ptr, idx)` → load direto (sem bounds check)
- `kata_rt_array_set(ptr, idx, val)` → store
- `kata_rt_array_get_checked(ptr, idx)` → Result
- `kata_rt_array_contains(ptr, item)` → 0/1

**Range (`crates/kata-rt/src/range.rs`):**
- `kata_rt_range_alloc(arena_handle)` → ponteiro (24 bytes: start@0, step@8, end@16)
- Operações de next/done são **inlined pelo codegen**, não há FFI para isso.

**Regra SMI tagging:** toda FFI de runtime que retorna `Int` (não ponteiro) deve
retornar SMI-tagged: `(val << 1) | 1`. FFIs que retornam ponteiros (List, Array,
Range, Sum box) não taggeiam (ponteiro cru, LSB=0). FFIs que retornam Boolean
retornam 0/1 cru (Boolean não é SMI-tagged).

### Lowering de literais

**ListLit `[1 2 3]`** — constrói Cons chain de trás para frente:
1. `nil = kata_rt_list_nil()` → 0
2. Para cada elemento (em ordem reversa): `acc = kata_rt_list_cons(head, acc, arena)`
3. Bitcast F64→I64 se o elemento for Float (Cons cell armazena i64 cru)
4. Arena é selecionada por `expr.escape` (Local → fiber_arena, Caller/Ancestor → caller_arena)

**ArrayLit `{1 2 3}`** — aloca header+data contíguo:
1. `ptr = kata_rt_array_alloc(len, arena)`
2. Para cada elemento: `kata_rt_array_set(ptr, idx, val)` (bitcast F64→I64 se Float)

**RangeLit `[0..2..10]`** — aloca 3 words e store direto:
1. `ptr = kata_rt_range_alloc(arena)`
2. `store(start, ptr, 0)`, `store(step, ptr, 8)`, `store(end, ptr, 16)`
3. Bitcast F64→I64 se os valores forem Float

### Lowering de ForIn

**Inlined por tipo concreto da coleção** — sem dispatch em runtime, o tipo é
conhecido em compile-time pela TAST:

- **List:** percorre Cons cells. `current = coll_ptr`; condição `current != 0`
  (Nil); head = `load(current, 0)`, tail = `load(current, 8)`; define var =
  head, current = tail; executa body; jump para loop.
- **Array:** percorre índices 0..len. `len = load(coll_ptr, 0)`; `idx = 0`;
  condição `idx >= len`; elem = `load(coll_ptr + 8 + idx*8)`; `idx += 1`;
  executa body.
- **Range:** percorre `current = start`, `current += step`; condição
  `current >= end` (exclusive — inclusive ainda não implementado no codegen,
  ver comentário no match); define var = current; `current += step`; executa
  body.

Reusa maquinaria de `loop`/`break`/`continue`: salva/restaura `loop_break_block`
e `loop_continue_block` do LowerCtx. Cria três blocos: `loop_block`,
`continue_block`, `break_block`.

### Lowering de `in` (membership)

**Dispatch por tipo concreto** (também inlined, sem FFI dispatch):

- **List/Array:** chama FFI `contains(ptr, item)` → 0/1 cru. Bitcast F64→I64
  se o item for Float.
- **Range:** O(1) aritmético — `start <= item AND item < end`. Dois `icmp`
  (I8 cada), combinados com `band` (I8), depois `uextend(I64, result)` para
  cruzar a fronteira do entry point. Não verifica step.

### Lowering de Pattern Cons `[h : t]`

- `val` é ponteiro para Cons cell (head@0, tail@8) ou 0 (Nil).
- Condição: `val != 0` (não-Nil) — `icmp_imm(Ne, val, 0)` → I8.
- Extrai head = `load(val, 0)`, tail = `load(val, 8)`.
- Testa sub-patterns (`head_cond`, `tail_cond`) via `test_single_pattern`.
- Combina com `band`: `not_nil AND head_cond AND tail_cond` → I8.
- **Regra:** `band` de dois I8 precisa `uextend(I64, result)` antes de
  retornar do entry point (que espera I64). `icmp` retorna I8, não I64.

### Lowering de map/filter/fold (Fase 8)

**Interceptação no `infer_apply`** (não via DispatchTable): `map`/`filter`/`fold`
são interceptados por nome no `infer_apply` antes do dispatch normal, como
`format` e `len` já fazem. O typeck descobre o tipo concreto do container no
call site e produz `TypedExprKind::Map`/`Filter`/`Fold`.

**Codegen** (`collections_hof.rs`): cada função itera pela coleção concreta
(List/Array/Range), chama o callback via `call_indirect`, e constrói o resultado.

- **Map:** percorre coleção, chama callback(elem), constrói Cons chain com
  `prepend` (inverte ordem), chama `kata_rt_list_reverse` no final. Se input
  era Array, converte List→Array via `list_to_array`.
- **Filter:** mesmo padrão, mas só faz `cons` se o predicado retornar true
  (branch `cons_block`/`skip_block`). Reverte no final.
- **Fold:** percorre coleção, `acc = call_indirect(callback, acc, elem)`,
  retorna `acc`. Não constrói coleção.

**Operador como callback standalone:** `fold + 0 [1 2 3]` — o `+` é
`Expr::Ident` que só é resolvido via DispatchTable quando aparece como callee
em `Apply`. Como callback standalone, `infer_expr_hinted` não o encontra.
**Fix:** `resolve_operator_callback` em `collections_hof.rs` detecta
`Expr::Ident` no DispatchTable e constrói um lambda sintético
`lambda __hof_0 __hof_1 ...: op __hof_0 __hof_1 ...` que o pipeline normal
sabe inferir e gerar.

**SMI arithmetic no iterador de Range:** `iadd` de dois SMI-tagged values
produz SMI inválido (`(a<<1|1) + (b<<1|1) = (a+b)<<1 | 2`, LSB=0 em vez de 1).
**Fix:** `iadd_imm(result, -1)` após cada `iadd` no iterador de Range. Aplicado
em 4 sites: `lower_map`, `lower_filter`, `lower_fold` (Range arm), e `ForIn`
em `expr.rs` (bug pré-existente).

### Lowering de stream fusion (Fase 9)

**Optimizer pass** (`kata-optimizer/src/stream_fusion.rs`): detecta composições
de `Map`/`Filter` na TAST e reescreve em `TypedExprKind::FusedStream`, eliminando
coleções intermediárias.

- `try_fuse(expr)` — recursão: funde o `collection` primeiro, depois adiciona
  o stage atual. Se o collection não é fundível mas é Map/Filter simples, cria
  FusedStream com 1 stage + o stage atual.
- `expr_to_stage(expr)` — converte Map/Filter em `FusedStage`.
- `stream_fusion_pass(typed)` — aplica `fuse_expr` no entry point + body de
  cada função.

**Padrões detectados:**
- `Map(f, Filter(g, src))` → FusedStream [Filter(g), Map(f)]
- `Map(f, Map(g, src))` → FusedStream [Map(g), Map(f)]
- `Filter(g, Map(f, src))` → FusedStream [Map(f), Filter(g)]
- `Filter(g, Filter(h, src))` → FusedStream [Filter(h), Filter(g)]
- 3+ níveis: `Map(f, Map(g, Filter(h, src)))` → FusedStream [Filter(h), Map(g), Map(f)]

**TAST:** `FusedStream { stages, source, coll_ty, source_elem_ty, result_elem_ty, ret_ty }`
onde `stages: Vec<FusedStage>` (enum com `Filter { callback, input_elem_ty }` e
`Map { callback, input_elem_ty, output_elem_ty }`). O novo variant exigiu cascata
E0004 em 5 arquivos (captures, recursion, instantiate, monomorph rewrite).

**Codegen** (`lower_fused_stream` em `collections_hof.rs`): mesmo loop de
`lower_map` (List/Array/Range), mas chama `apply_stages` que aplica a cadeia
completa. `apply_stages` retorna `(val, keep_flag)` — keep_flag é I64 (0=descartar,
1=keep). Cada `FusedStage::Filter` faz `band(keep, pred_result)` — AND lógico em
I64 cru. Cada `FusedStage::Map` chama o callback via `call_indirect`. Após
`apply_stages`, `brif(keep != 0, cons_block, skip_block)` — cons só se keep=1.
No final: `kata_rt_list_reverse(acc, arena)` + conversão List→Array se coll_ty
era Array.

**Sem short-circuit em SSA:** `apply_stages` sempre chama todos os callbacks,
mesmo se keep já é 0. Filter chama o predicado (resultado ignorado pelo `band`),
Map aplica a transformação (resultado ignorado pelo `brif`). O overhead é mínimo
e a alternativa exigiria blocks condicionais para cada stage.

**Limitação:** Fold não é fundido (sempre consome, não produz lista).

---

## Resumo: Invariantes de design

| Estrutura | Invariante a respeitar desde o início |
|---|---|
| TypeEnv + cache | Cache de módulos previne ciclos de import |
| DispatchTable | Scoring por dominância nasce em Fio 1, mesmo com 1 overload. Score 4D: (exact, alias, refined, iface) + tiebreak genérico |
| InterfaceRegistry | DFS com HashSet para cycle detection no registro |
| EscapeCtx + shared_exprs | Shared é recursivo: ARC pointer nunca tem fields arena |
| TypeIdAssignment | `ids` DEVE ser armazenado no TypedModule (não descartado) |
| Type Table runtime | Rust-to-Rust, não C-ABI (TypeShape tem Box/String/Vec) |
| Value Table | 3 tabelas: SHARED_ARC + SHARED_ARENA + PER_FIBER_ARENA |
| LowerCtx | Builder function-level, não module-level |
| EmitCtx.values | stack_load sempre para block params em blocos não-entry |
| Scheduler | Struct explícita, TLS só para yield dentro de FFI |
| Tree shaking | FFI e smart constructors sempre mantidos |
| Mono cache | Deduplica especialização do mesmo genérico |
| Collect lambdas | lambda_addr passado explicitamente (não addr do parâmetro) |
| Lazy specialization | Especializa sob demanda, não ahead-of-time |
| Comptime cache | Evita recompilar/reexecutar a mesma expressão |
| Ascription hint | Ret-directed dispatch + grouped barrier + convert_typed_expr (Fio 6) |

Cada invariante acima existe porque violá-lo produz bugs concretos — não são
preferências estéticas. O documento os captura como prescrição, não como história.