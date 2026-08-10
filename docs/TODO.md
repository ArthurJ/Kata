# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-09.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### 1. `@test{expects: \"CompileError\"}` — execução não implementada

**Estado:** Parser aceita, codegen cria placeholder, driver imprime `[PENDENTE]` e pula.

**O que falta (design C1 — sub-módulos isolados):**
1. Extrair o sub-módulo referenciado por `expects`
2. Compilá-lo isoladamente (lex → parse → resolve → infer)
3. Verificar que a compilação falha com o erro esperado (substring match)
4. Reportar PASS se falhou com o erro esperado, FAIL se compilou ou falhou com erro diferente

**Arquivos:** `crates/kata-driver/src/main.rs:234-242`, `crates/kata-codegen/src/lowering/test_runner.rs:68-72`, `crates/kata-driver/tests/test_runner_e2e.rs` (teste `#[ignore]`)

**Impacto:** Médio. Testes negativos são silenciosamente pulados — o programador escreve o teste, o runner diz `[PENDENTE]`, mas a validação nunca acontece.

---

## Migração de Exemplos

### `parallel.kata` (Cluster 4)

**Estado:** `spawn!` totalmente implementado (parser, inference, codegen, runtime). 10 testes E2E passando. Falta apenas criar `examples/parallel.kata` migrando `examples/legacy/test_parallel.kata` (ajustar sintaxe `n :: Int` → `n::Int`).

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

---

## Arquitetura — Análise de Refatoração (do zero)

Itens identificados na análise arquitetural de 2026-08-09. Cada item descreve
o problema, a proposta, e o status (analisar / decidir / executar).

### A2. Runtime reentrante — eliminar TLS do scheduler e runtime

**Motivação:** Bloqueador para REPL e LSP. Ambos exigem múltiplas execuções
isoladas no mesmo processo.

- **REPL:** cada linha é JIT-compilada e executada via `kata_rt_run()`. O
  scheduler, as arenas e a type table estão em TLS — `reset_scheduler()`
  entre linhas destrói arenas (valores de uma linha não sobrevivem para a
  próxima), a type table é substituída (não incrementada), e o
  `ROOT_ARENA_HANDLE` é zerado. Não há como manter estado entre avaliações.
- **LSP:** requests sobrepostos (didChange + hover) compartilham o scheduler
  em TLS — execuções se pisam. Comptime eval, testes inline e debug eval
  precisam de runtime isolado por request.

**Problema (estado atual, pós-Fio 16):** Os nomes `SHARED_ARC`,
`SHARED_ARENA`, `PER_FIBER_ARENA` foram removidos pelo Fio 16 (pool de
arenas com handles i64 + TrackedArena). O que persiste:

- `SCHEDULER: RefCell<Option<Scheduler>>` (TLS, `scheduler/ffi.rs:42`) —
  scheduler de fibers. O `RefCell` causa double-borrow quando `kata_rt_spawn`
  é chamado de dentro de `resume()`.
- `PENDING_SPAWNS` (TLS, `scheduler/ffi.rs:56`) — workaround para o
  double-borrow acima. Existe **apenas** porque o scheduler está em `RefCell`
  em vez de ponteiro explícito. Desaparece com A2.
- `ROOT_ARENA_HANDLE: Cell<i64>` (TLS, `arena.rs:33`) — lido por
  `kata_rt_decref` em hot path de ARC. Frágil entre execuções.
- `TYPE_TABLE: RefCell<Vec<TypeShape>>` (TLS, `marshal/mod.rs:71`) —
  substituída por `register_type_table`, não incrementada.
- `YIELD_COUNTER`, `HAS_READY_FIBER` (TLS, `scheduler/ffi.rs:74`) — yield
  cooperativo, lidos em hot path de loops.
- `CURRENT_SUSPEND` (TLS, `fiber.rs:88`) — ponteiro de Suspend para
  yield/fork.
- Log, snapshot, dict, cache — TLS periféricas, não no caminho crítico.
- `TIMEOUT_EXPIRED: AtomicBool` + `PENDING_TIMER: Mutex<...>` (globals
  process-wide, `scheduler/ffi.rs:32-38`) — timer de teste; a thread OS
  timer não tem acesso ao `Runtime*`. Permanecem global ou exigem
  arquitetura separada.

**Proposta:** `Runtime` struct explícita carregando scheduler, arenas e type
table, passada por ponteiro (`*mut Runtime` como i64) ao código JIT —
generalizando o padrão já usado para `fiber_arena` e `caller_arena`. FFIs
afetadas: `kata_rt_scheduler_init`, `kata_rt_spawn`, `kata_rt_run`,
`kata_rt_yield`, `kata_rt_yield_check`, `kata_rt_decref`,
`kata_rt_get_root_arena_handle`, `kata_rt_to_bytes`, `kata_rt_from_bytes`.
Cada call site no codegen ganha um parâmetro `rt: i64`. `PENDING_SPAWNS` é
eliminado (acesso direto via ponteiro, sem RefCell).

**Status:** ✅ Concluído. `Runtime` struct implementada em `crates/kata-rt/src/runtime.rs`,
passada como `rt: i64` às FFIs centrais. TLS restante: `RT_PTR` (cache para FFIs
periféricas), `CURRENT_SUSPEND` (continuação de fiber), `TIMEOUT_EXPIRED`/`PENDING_TIMER`
(timer de teste). ABI de funções puras migrada para `(rt, arena_handle, box_ptr, ...args)`.
`jit_eval` faz leak do Runtime para preservar arenas (valores retornados são ponteiros
para a arena Bump). 1493 testes passando, 0 falhas.

**Paralelismo de testes:** `cargo test --workspace -- --test-threads=N` funciona
verificado empiricamente com N=4. Cada thread tem seu próprio `RT_PTR` TLS e seu
próprio `Runtime` (variável local em `jit_eval`) — sem data races no runtime.
Limitação: `TIMEOUT_EXPIRED`/`PENDING_TIMER` são `static` (shared entre threads);
testes que usam o timer de teste podem racear se executados em paralelo. Os testes
E2E de codegen não usam timer, então rodam em paralelo sem problemas.

### A3. Decompor LowerCtx em sub-contextos estratificados

**Problema:** `LowerCtx` tem 30+ campos misturando 5 concerns: bindings,
closures, escape/ARC, type IDs, function-level state, arena management,
I/O handles, loop/continue blocks, epílogo de Action. O doc
`maquinaria-interna.md` chama de \"a struct mais complexa\".

**Proposta:** Decompor em contextos estratificados:
- `ScopeCtx` — bindings e var_map
- `ClosureCtx` — closure_fn_names, closure_sigs, extra_signatures
- `ArenaCtx` — arc_values, arena_values, shared_exprs, fiber_arena, caller_arena
- `ActionCtx` — epilogue_block, io_handle_vars, loop blocks, scheduler_mode

O `LowerCtx` seria composição desses, não um deus-struct. Cada subcontexto
pode ser testado isoladamente e passado apenas onde é necessário (funções
puras não precisam de `ActionCtx`).

**Status:** Analisar. Refatoração estrutural no codegen — avaliar impacto
nos 29 submódulos de lowering.

### A4. Separar type checker, synthesizer e desugarer em camadas distintas

**Problema:** `kata-inference` (16.6k LOC, 42 submódulos) faz type checking,
desugar (pipe, hole, question, fallback), dispatch resolution, síntese de
`show`, síntese de smart constructors, CSP builtins, collections HOF, const
eval, log/timer synthesis — tudo no mesmo crate. O `infer_module` intercala
síntese com inferência: a ordem de descoberta acopla a ordem de processamento.

**Proposta:** Separar em três camadas:
- **Desugarer** — pipe, hole, question, fallback. Transforma AST antes do
  type checker.
- **Synthesizer** — smart constructors, show, predicados. Roda *após*
  resolution, *antes* do type checker. O `ResolvedModule` já teria todas as
  funções sintetizadas registradas no `DispatchTable`.
- **Type checker** — inferência, unificação, pattern checking,
  exhaustiveness. Não sintetiza nada — apenas consome.

Torna a ordem determinística e testável: `resolve → synthesize → infer`,
cada um com input/output bem definido.

**Status:** Analisar (caso de estudo). A separação é conceitualmente limpa,
mas a síntese de `show` depende de descobrir structs/enums no módulo, que
por sua vez pode depender de inferência parcial de imports. Avaliar se a
separação é possível sem resolver imports durante a síntese.

### A6. Unificação estrutural AST ↔ TAST

**Problema:** `Expr` (40+ variants) e `TypedExprKind` (espelha quase todos
com `Spanned<TypedExpr>` e info de tipo) são mantidos em sincronia manualmente.
Cada novo nó exige mudança em 2 lugares. Há variants na AST que somem na TAST
(Hole, Pipe, PipeFallback, Question — desugared) e variants na TAST que não
existem na AST (StructConstruct, FieldAccess, IndexAccess, VariantConstruct,
Map/Filter/Fold/FusedStream, ChannelCreate, Fork, Spawn, HeapSnapshot).

**Proposta:** Uma única enum parametrizada por metadata: `Expr<Meta = ()>`
para AST, `Expr<Meta = TypeInfo>` para TAST. O desugar produz nós da mesma
enum. O type checker preenche `Meta`. Elimina duplicação estrutural e risco
de drift.

**Status:** Analisar. Avaliar o impacto no codegen (que faz match exaustivo
em `TypedExprKind`), nos 42 submódulos de inference, e nos snapshots. A
mudança é profunda mas elimina uma classe inteira de bugs de sincronização.

---

## Fora do Escopo 1.0

Mantidos no ROADMAP. Não mover para cá sem decisão explícita.

- Tensor/SIMD
- `@heapstack` (otimização heurística de arena em loops)
- `@restart` (retry policy para Actions)
- Doc comments (`///`, `\"\"\"doc\"\"\"`)
- Tuplas variádicas (`T...`)
- GC/reclamation granular para fibers long-lived