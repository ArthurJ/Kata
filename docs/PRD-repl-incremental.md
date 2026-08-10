# PRD — REPL: Compilação Incremental via Snapshots

**Status:** Rascunho
**Data:** 2026-08-10
**Depende de:** A2 ✅ (Runtime reentrante — `rt_ptr` persistente), Fio 12 ✅ (comptime — `HeapSnapshot`, `serialize_snapshot`, `kata_rt_load_snapshot`, `kata_rt_get_snapshot`)
**Não depende de:** A3 (LowerCtx decomposition), LSP

## 1. Objetivo

Eliminar a recompilação total de bindings acumulados no REPL. Hoje, a linha
N+1 recompila todos os N bindings anteriores + a nova expressão. Com este PRD,
bindings anteriores são persistidos como snapshots na root_arena — a linha N+1
compila apenas a nova expressão, referenciando bindings via `kata_rt_get_snapshot`.

### Princípio: persistir valores, não código

Modelos alternativos (bindings como funções JIT separadas, linking entre
modules) tentam persistir **código compilado**. O Cranelift JIT não suporta
adicionar funções a um module finalizado, e linking entre modules exige
`Linkage::Import` + symbol registry com function pointers absolutos —
complexidade alta, mudança no modelo mental do codegen (`let` deixa de ser
inline).

Este PRD persiste **valores**. O sistema de snapshots já serializa valores
comptime em bytes contíguos + rebase_offsets, carrega na root_arena, e os
indexa por ID na tabela TLS `SNAPSHOT_PTRS`. Valores são dados — não exigem
ABI, calling convention, ou relocations. `memcpy` + rebasing basta.

### Princípio: reusar infraestrutura existente

`HeapSnapshotData`, `serialize_snapshot`, `kata_rt_load_snapshot`,
`kata_rt_get_snapshot`, `TypedExprKind::HeapSnapshot` — tudo já existe e está
testado (Fio 12). Este PRD estende o uso desses mecanismos do contexto comptime
para o contexto REPL. A única adição ao runtime é um skip de load para
snapshots já carregados (evitar re-alocação desnecessária).

## 2. Modelo

### 2.1. Estado da ReplSession

```rust
pub(crate) struct ReplSession {
    /// Items não-Let acumulados (Sig, DataDecl, EnumDecl, AliasDecl, etc.).
    /// Estes continuam sendo reprocessados pelo pipeline a cada linha
    /// (custo de resolution + inference, sem JIT codegen).
    pub(crate) items: Vec<Spanned<Item>>,

    /// Bindings `let` acumulados — mapa nome → (snapshot_id, Ty).
    /// Shadowing: reatribuir o mesmo nome substitui o snapshot_id.
    /// O snapshot antigo permanece na arena como lixo (aceitável para REPL).
    pub(crate) bindings: HashMap<String, (u32, Ty)>,

    /// Dados de snapshots acumulados — indexados por snapshot_id.
    /// Cada binding escalar ou complexo tem uma entrada aqui.
    pub(crate) snapshots: Vec<HeapSnapshotData>,

    /// Prelude resolvido (recarregado em `:reset`).
    pub(crate) prelude: ResolvedModule,

    /// Runtime persistente — vive entre avaliações.
    /// A root_arena dentro do Runtime mantém os valores carregados.
    pub(crate) rt_ptr: i64,

    /// Histórico rustyline.
    pub(crate) history_path: PathBuf,
}
```

### 2.2. Fluxo por linha

#### Linha com `let x := <expr>` (binding novo ou shadowing)

```
1. Lex + parse input → items
2. Se input tem EntryExpr que é `Let { name, value }`:
   a. Constrói TypedModule minimal:
      - pre_entry: bindings anteriores como HeapSnapshots
      - entry: o `let x := <expr>` (para inferir o tipo e JIT-executar)
      - functions/actions: vazias (não há funções nomeadas nesta linha)
   b. resolve → infer_module → monomorphize → optimize → tree_shake
   c. jit_eval(mono, type_id_map, type_shapes, rt_ptr) → JitResult { raw, ty }
   d. Serializa o resultado:
      - Escalar (Int SMI, Float, Boolean, Unit): converte para TypedExprKind::IntLit/FloatLit/VariantQual/Unit
      - Complexo (List, Struct, Tuple, Text, Sum): serialize_snapshot → HeapSnapshotData
   e. Atribui snapshot_id:
      - Se escalar: bindings[name] = (id_literal, ty) — id é sintético, não entra em `snapshots`
      - Se complexo: snapshots.push(data); bindings[name] = (snapshots.len() - 1, ty)
   f. JITModule descartado (leaked — páginas de código permanecem mapeadas)
```

#### Linha com expressão (não-let)

```
1. Lex + parse input → items
2. Constrói TypedModule minimal:
   - pre_entry: bindings anteriores como HeapSnapshots (ou literais se escalares)
   - entry: a expressão
3. resolve → infer_module → monomorphize → optimize → tree_shake
4. jit_eval(mono, type_id_map, type_shapes, rt_ptr) → JitResult
5. display(result.raw, result.ty)
6. JITModule descartado
```

#### Linha com declaração (Sig, Data, Enum, etc.)

```
1. Lex + parse input → items
2. Adiciona a `self.items` (não é binding, não gera snapshot)
3. Valida com pipeline_typed (typeck apenas, sem execução)
```

### 2.3. Injeção de bindings como pre_entry

Para cada binding ativo em `self.bindings`, injeta um `Spanned<TypedExpr>` em
`pre_entry` do TypedModule minimal:

```rust
fn build_pre_entry(&self) -> Vec<Spanned<TypedExpr>> {
    let mut pre_entry = Vec::new();
    for (name, (snapshot_id, ty)) in &self.bindings {
        let kind = match ty {
            // Escalares: literal direto (zero custo de snapshot)
            Ty::Prim(PrimTy::Int) => {
                // O valor foi decodificado no momento do binding.
                // Guardar o texto do literal junto com o snapshot_id.
                // Ou: usar HeapSnapshot para tudo (simplifica, mas perde
                // a otimização de literal direto).
                // Decisão: usar HeapSnapshot para tudo no primeiro corte.
                // Otimização de escalares fica para depois se perf mostrar ganho.
                TypedExprKind::HeapSnapshot { snapshot_id: *snapshot_id, ty: ty.clone() }
            }
            _ => TypedExprKind::HeapSnapshot { snapshot_id: *snapshot_id, ty: ty.clone() },
        };

        let expr = TypedExpr {
            span: Span::synthetic(),
            ty: ty.clone(),
            tail_pos: false,
            escape: kata_core::EscapeTarget::default(),
            kind: TypedExprKind::Let {
                name: name.clone(),
                value: Box::new(Spanned::new(
                    TypedExpr { span: Span::synthetic(), ty: ty.clone(), tail_pos: false, escape: Default::default(), kind },
                    Span::synthetic(),
                )),
            },
        };
        pre_entry.push(Spanned::new(expr, Span::synthetic()));
    }
    pre_entry
}
```

O codegen já lowera `Let` em `pre_entry` fazendo `def_var(name, value)` no
`var_map`. O `HeapSnapshot` lowera para `kata_rt_get_snapshot(id)` que retorna
o ponteiro da arena. O `Ident` que referencia `name` faz `use_var(name)` que
retorna o valor. **Nenhuma mudança no codegen.**

### 2.4. Carregamento de snapshots no prólogo

O `lower_module` já emite `kata_rt_load_snapshot` para cada snapshot no
`TypedModule.snapshots` no prólogo de `__kata_entry`. Para o REPL, os
snapshots de bindings anteriores já estão na tabela TLS — re-executar
`load_snapshot` re-aloca e re-memcpy's o mesmo valor, desperdiçando arena.

**Extensão necessária:** `kata_rt_load_snapshot` deve pular o load se
`SNAPSHOT_PTRS[id] != 0` (já carregado). Adicionar verificação no início da
função:

```rust
pub extern "C" fn kata_rt_load_snapshot(
    root_arena: i64,
    bytes_ptr: i64,
    bytes_len: i64,
    rebase_offsets_ptr: i64,
    rebase_count: i64,
    snapshot_id: i64,
) {
    // Skip se já carregado — REPL reusa snapshots entre linhas.
    SNAPSHOT_PTRS.with(|table| {
        let table = table.borrow();
        let id = snapshot_id as usize;
        if id < table.len() && table[id] != 0 {
            return; // já carregado, não re-alocar
        }
    });

    // ... resto do load existente ...
}
```

Isto é uma mudança de ~5 linhas no runtime. Não afeta o fluxo comptime
normal (cada compilação começa com tabela limpa via `reset_snapshot_table`).

### 2.5. Shadowing

Shadowing é trivial: `bindings["x"] = (novo_id, ty)` substitui a entrada.
O snapshot antigo permanece na root_arena como lixo. A próxima linha que
referencia `x` usa o novo `snapshot_id`. O antigo não é referenciado por
nome — ocupa espaço mas não causa incorreção.

Closures capturam valores concretos no momento de criação (serializados
dentro do snapshot do closure), não referências a bindings. Shadowing de
um binding capturado não retroage — semântica correta de shadowing funcional:

```
linha 1: let x := 42          → snapshot_id=0
linha 2: let f := lambda n: n + x   → closure captura 42 no snapshot_id=1
linha 3: let x := 99          → snapshot_id=2 (shadowing)
linha 4: echo!(f 10)          → f usa snapshot_id=1 → 10 + 42 = 52 ✓
```

### 2.6. Snapshots de closures (function pointers)

Closures são structs na arena com layout `{ function_ptr, captures[] }`.
O `function_ptr` é um ponteiro absoluto para código JIT no module da linha
onde o closure foi criado. O `JITModule` é leaked (sem handle) — as páginas
de código permanecem mapeadas, o ponteiro absoluto continua válido.

`serialize_snapshot` converte ponteiros para dados na arena em offsets
relativos (rebasing). Para `function_ptr`, o ponteiro é absoluto e não
precisa rebasing — aponta para código, não para dados na arena.

**Verificar:** o serializador atual trata todos os i64s em `rebase_offsets`
como offsets relativos. Se `function_ptr` estiver em `rebase_offsets`, o
rebasing soma `base_ptr` ao function_ptr, corrompendo-o. Solução: o
serializador precisa distinguir ponteiros para dados (rebase) de ponteiros
para código (não rebase). Ver `serialize_snapshot` em
`crates/kata-comptime/src/snapshot.rs`.

**Implementado (Fase 3):** o braço `Ty::Function` serializa o CaptureBox
como raw i64s (fn_ptr, refcount, n_captures, captures) **sem adicionar
nenhum offset a `rebase_offsets`**. O `fn_ptr` é ponteiro absoluto para
código JIT (páginas leaked permanecem mapeadas). Os captures são valores
imediatos ou ponteiros absolutos para a arena persistente — ambos válidos
sem rebasing porque o Runtime do REPL persiste entre linhas.

**Isto é a única extensão do serializador.** Pode ser postergada: o primeiro
corte do REPL incremental suporta bindings escalares e estruturas de dados
sem closures. Closures como bindings `let` podem usar o fluxo de fallback
(recompilação total) até que o serializador seja estendido.

### 2.7. Type table

`register_type_table(rt, shapes)` é chamado dentro de `jit_eval` se
`!type_shapes.is_empty()`. No REPL, `type_shapes` é `&[]` (vazio) — o REPL
não usa marshalling hoje. Se uma linha define um struct, a type table
precisaria acumular.

**Verificar:** `register_type_table` é idempotent (append ou replace?).
Se replace, type tables de linhas anteriores são perdidas. Se append,
acumula corretamente. Para o primeiro corte, o REPL sem marshalling
funciona — structs são alocados na arena e acessados por ponteiro, sem
serialização cross-process.

### 2.8. Arena growth

Cada binding (e cada shadowing) aloca na root_arena. A root_arena é uma
TrackedArena — suporta dealloc individual. Mas snapshots não têm ARC
tracking (são carregados via `load_snapshot`, não via `incref/decref`).

Para o primeiro corte: aceitar growth. REPL é sessão curta. `:reset`
recria o Runtime (arena nova, tabela TLS limpa). Se growth for problema
em sessões longas, marcar snapshots não referenciados após shadowing e
dar `arena_dealloc` — a TrackedArena suporta.

### 2.9. Scheduler state

Se a linha 1 faz `fork!(action)` e o scheduler tem fibers pendentes, a
linha 2 (expressão pura) chama `kata_rt_run` — que drena fibers prontos.
Isto pode causar efeitos colaterais inesperados (output de fibers da
linha 1 aparecendo na linha 2).

Hoje o scheduler está dentro do Runtime persistente — fibers sobrevivem
entre linhas. Isto é correto para persistência mas pode confundir o
usuário. Para o primeiro corte: manter comportamento atual (fibers
persistem). Documentar no REPL que `fork!` deixa fibers pendentes que
executam na próxima avaliação.

## 3. Fases

### Fase 1 — Bindings escalares (Int, Float, Boolean, Unit)

**Escopo:** `let x := 42` depois `echo!(x)` funciona sem recompilação do 42.

**Mudanças:**
1. `ReplSession`: adicionar `bindings: HashMap<String, (u32, Ty)>` e
   `snapshots: Vec<HeapSnapshotData>`
2. `eval_expr`: detectar `Let` em EntryExpr, JIT-executar o valor,
   serializar, armazenar em `bindings`
3. `build_pre_entry`: injetar bindings como `pre_entry` (HeapSnapshot)
4. `run_pipeline_eval`: usar `build_pre_entry` em vez de reprocessar
   todos os items
5. `kata_rt_load_snapshot`: adicionar skip se já carregado (~5 linhas)

**Verificação:**
- `let x := 42` depois `echo!(x)` → 42
- `let x := 42` depois `let x := 99` depois `echo!(x)` → 99 (shadowing)
- `let x := 3.14` depois `echo!(x)` → 3.14
- `let b := True` depois `echo!(b)` → True
- `cargo test --workspace --no-fail-fast -- --test-threads=8` → 1493+ passed

### Fase 2 — Estruturas de dados (List, Tuple, Struct, Text)

**Escopo:** `let xs := [1 2 3]` depois `echo!(xs)` funciona sem
recompilação.

**Mudanças:**
1. `serialize_snapshot` já suporta List, Tuple, Struct, Text, Sum
2. `build_pre_entry` já injeta HeapSnapshot para qualquer tipo
3. Validar que o snapshot carregado na linha 1 sobrevive na root_arena
   para a linha 2

**Verificação:**
- `let xs := [1 2 3]` depois `echo!(xs)` → [1 2 3]
- `let s := "hello"` depois `echo!(s)` → "hello"
- `let p := Point{ x: 1, y: 2 }` depois `echo!(p.x)` → 1
- `let t := (1, 2.0, True)` depois `echo!(t)` → (1, 2.0, True)

### Fase 3 — Closures como bindings ✅

**Escopo:** `let f := lambda n: n + 1` depois `echo!(f 10)` funciona
sem recompilação.

**Mudanças:**
1. `serialize_snapshot`: adicionado braço `Ty::Function` — serializa
   o CaptureBox como raw i64s (fn_ptr, refcount, n_captures,
   captures[0..n]) sem rebase. O `fn_ptr` é ponteiro absoluto para
   código JIT (páginas leaked permanecem mapeadas). Os captures são
   valores imediatos ou ponteiros absolutos para a arena persistente
   — ambos válidos sem rebasing porque o Runtime do REPL persiste.
2. `result_to_literal`: adicionado `Ty::Function(_, _)` ao match de
   tipos complexos que viram `HeapSnapshot`.

**Verificação:**
- `let f := lambda n: n + 1` depois `echo!(f 10)` → 11 ✅
- `let x := 42` depois `let f := lambda n: n + x` depois `echo!(f 10)` → 52 ✅
- `let x := 42` depois `let f := lambda n: n + x` depois `let x := 99` depois `echo!(f 10)` → 52 (shadowing não retroage) ✅
- `cargo test --workspace --no-fail-fast -- --test-threads=8` → 1496 passed ✅

### Fase 4 — Funções nomeadas ✅

**Escopo:** `double :: Int => Int` / `double x := * x 2` persiste entre
linhas sem recompilação do body.

**Mudanças:**
1. Funções nomeadas são `typed.functions` (não `pre_entry`) — compiladas
   como `__kata_fn_N` no JITModule. Hoje, o JITModule é descartado.
2. Para persistir funções nomeadas sem recompilação, extrair function
   pointers antes de descartar o JITModule (leak sem handle, páginas de
   código permanecem mapeadas) e armazenar numa tabela de símbolos
   na ReplSession.
3. A próxima linha registra os function pointers no
   `JITBuilder::symbol_registry` do novo JITModule, e declara as funções
   anteriores como `Linkage::Import`.

**Isto é a parte que sai do modelo de snapshots.** Funções nomeadas têm
corpo compilado — não são valores serializáveis. Precisam de linking
entre modules JIT. Mas o custo é isolado a esta fase — bindings `let`
(Fases 1-3) não precisam de linking.

**Implementação:**
- `canonical_fn_id` (FNV-1a de nome + param_types + clauses) identifica
  funções univocamente. Redefinição (corpo diferente) produz hash
  diferente → recompila como Export.
- `declare_kata_function` aceita `Linkage` (Import vs Export).
- `lower_module` recebe `prev_funcs: &HashMap<i64, (String, *const u8)>`
  (fn_hash → (cranelift_name, fn_ptr)). Para cada `typed.functions`,
  decide Import vs Export baseado no hash. Retorna `Vec<CompiledFunc>`
  com info das funções Export recém-compiladas.
- `jit_eval_repl` (wrapper de `jit_eval` para o REPL): registra
  function pointers persistidos no `JITBuilder::symbol()`, executa,
  extrai fn_ptrs das funções novas, retorna `ReplJitResult`.
- `ReplSession.function_table: PrevFuncMap` acumula fn_hash →
  (cranelift_name, fn_ptr) entre linhas.
- Callers existentes (`jit_eval`, `jit_compile_tests`, `aot_emit`)
  passam `&HashMap::new()` — sem mudanças.

**Verificação:**
- `double :: Int => Int` / `lambda x: * x 2` → `echo!(double 5)` → 10 ✅
- `double :: Int => Int` / `lambda x: * x 2` → `echo!(double (double 5))` → 20 ✅
- `fat :: Int Int => Int` / `lambda 0 acc: acc` / `lambda n acc: fat (- n 1) (* n acc)`
  → `echo!(fat 5 1)` → 120, `echo!(fat 10 1)` → 3628800 ✅ (recursão)
- Redefinição: `double` com `* x 2` → 10, depois `double` com `+ x 100` → 105 ✅
- `cargo test --workspace --no-fail-fast -- --test-threads=8` → 1501 passed ✅

## 4. Estrutura esperada ao concluir

```
kata-driver:
  repl/mod.rs:
    - ReplSession com compilação incremental completa
    - frozen_bindings: escalares → literais AST
    - snapshot_bindings: complexos + closures → HeapSnapshot na TAST
    - function_table: funções nomeadas → (cranelift_name, fn_ptr)
    - run_pipeline_eval usa jit_eval_repl (não jit_eval)

kata-codegen:
  lowering/jit.rs:
    - jit_eval_repl: registra prev_funcs no JITBuilder, extrai new_funcs
    - PrevFuncMap: HashMap<i64, (String, *const u8)>
  lowering/module.rs:
    - lower_module recebe prev_funcs, decide Import vs Export
    - CompiledFunc: fn_hash + cranelift_name + func_id (funções Export)
  lowering/function_def.rs:
    - declare_kata_function aceita Linkage (Import vs Export)
  lowering/cache_key.rs:
    - canonical_fn_id: pub(crate) — hash FNV-1a de nome + tipos + clauses

kata-comptime:
  snapshot.rs:
    - serialize_snapshot: braço Ty::Function serializa CaptureBox
      como raw i64s sem rebase — Fase 3 ✅

kata-rt:
  snapshot.rs:
    - kata_rt_load_snapshot: skip se SNAPSHOT_PTRS[id] != 0
```

## 5. O que não muda

- `lower_module` — não muda. Continua lowerando `pre_entry` e `__kata_entry`
  como hoje.
- `lower_expr` para `HeapSnapshot` — não muda. Já emite
  `kata_rt_get_snapshot(id)`.
- `jit_eval` — não muda. Continua recebendo `rt_ptr` e criando JITModule.
- `TypedExprKind::HeapSnapshot` — não muda. Já existe e é lowered corretamente.
- `serialize_snapshot` — não muda (Fases 1-2). Extensão apenas na Fase 3
  para function pointers.

## 6. Riscos e mitigações

### 6.1. SNAPSHOT_PTRS é TLS — não sobrevive entre threads

Se o REPL usar threads diferentes para avaliação (improvável mas possível
em LSP), a tabela TLS de uma thread não tem os snapshots da outra. Para o
REPL single-threaded, não é problema. Para LSP, cada request pode precisar
do seu próprio Runtime + tabela — já é o modelo do A2.

### 6.2. Snapshot IDs colidem entre compilações

O `TypedModule.snapshots` é indexado por posição (snapshot_id =
índice no Vec). Se a linha 1 tem snapshots [A, B] (IDs 0, 1) e a linha 2
tem snapshots [C] (ID 0), o ID 0 da linha 2 colide com o ID 0 da linha 1.

**Mitigação:** a ReplSession mantém seu próprio `snapshots: Vec<HeapSnapshotData>`
com IDs globais (índice no Vec da sessão). O `TypedModule.snapshots` enviado
ao `jit_eval` contém apenas os snapshots novos da linha atual — mas os IDs
precisam ser globais, não locais.

**Solução:** o `TypedModule.snapshots` da linha atual referencia IDs globais.
O `build_pre_entry` injeta `HeapSnapshot { snapshot_id: id_global, ty }` onde
`id_global` é o ID no Vec da ReplSession. O prólogo do `__kata_entry` chama
`kata_rt_load_snapshot` para cada snapshot no `TypedModule.snapshots` — mas
apenas os novos (skip se já carregado). Os bindings anteriores já estão na
tabela TLS.

**Isto exige que `TypedModule.snapshots` use IDs globais**, não locais. Verificar
se o `lower_module` indexa snapshots por posição local ou por ID explícito.

### 6.3. Type checking sem reprocessar items não-Let

Declarações (Sig, Data, Enum, etc.) em `self.items` ainda precisam ser
reprocessadas pelo pipeline (resolve + infer) para que o type_env esteja
correto. Isto é custo de resolution + inference, não de codegen. Para o
primeiro corte, aceitar este custo — a otimização de cache de type_env
fica para depois.

### 6.4. Reset não limpa snapshots leaked

`:reset` recria o Runtime (arena nova) mas os JITModules leaked das linhas
anteriores permanecem mapeados. O `reset_snapshot_table()` limpa a tabela
TLS, mas os bytes na arena antiga são liberados quando o Runtime antigo é
droppado. Os JITModules leaked são irreversíveis — código JIT permanece
mapeado até o processo sair. Isto é aceitável para REPL (mesmo comportamento
do `jit_eval` hoje).

## 7. Relação com A2 (Runtime reentrante)

A2 adicionou `rt_ptr: i64` persistente na ReplSession. Hoje, o Runtime
persistente é "inofensivo mas desnecessário" — a persistência funciona por
recompilação. Este PRD torna o Runtime persistente **necessário**: os
snapshots são carregados na root_arena do Runtime persistente, e os
ponteiros sobrevivem entre linhas porque a arena não é resetada.

## 8. Relação com Fio 12 (Comptime)

Fio 12 criou a infraestrutura de snapshots para `@comptime`. Este PRD
reusa essa infraestrutura em um contexto diferente (REPL). A única
sobreposição é o `kata_rt_load_snapshot` — a extensão de skip (se já
carregado) é compatível com o fluxo comptime (que começa com tabela
limpa).