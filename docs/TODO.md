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

## OverloadSet — Partial Dispatch com Lambda Diferido ✅

**Estado:** Concluído (2026-08-10). 6 fases implementadas e testadas.

PRD: `docs/PRD-overloadset-partial.md`

**Problema:** Overloads cross-type (ex: `+ :: Int Float => Float @commutative`)
causam ambiguidade no partial dispatch. `+ 10 _` casa com `Int Int`, `Int Float`,
`Int Rational` (segunda posição é hole = não restringe). Antes, isso produzia
`LambdaInferenceFail`.

**Solução:** `Ty::OverloadSet { name, overloads }` — projeta as overloads
compatíveis para o lambda deferido. O call site (ou HOF com hint) desambigua.

**Fases:**
1. `PartialResolveOutcome::Ambiguous` em `resolve_partial` (commit `c6386ab`)
2. `infer_lambda` produz `OverloadSet` quando ambíguo sem hint (commit `c6386ab`)
3. `infer_apply` caminho 2c: re-infere lambda com args concretos (commit `c6386ab`)
4. `infer_map`/`infer_fold`/`infer_filter` tratam `OverloadSet` no callback (commit `5065e61`)
5. Re-inferência na inference layer: HOF com `OverloadSet` re-infere lambda com hint concreto (commit `a6eb330`)
6. 20 testes atualizados em 8 arquivos (commit `9511662`)

**Resultado:** `fold f 0 [1 2 3]` com `f := + _ _` (OverloadSet) retorna 6.
`map f [1 2 3]` com `f := + _ 2` (partial dispatch resolve) retorna [3 4 5].

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
`jit_eval` recebe `rt_ptr` como parâmetro — o caller controla o lifecycle do Runtime.
Testes E2E usam `leak_rt_ptr()` (leak aceitável para processos efêmeros). O REPL cria
um Runtime persistente que vive entre avaliações — valores na arena Bump e type table
persistem entre linhas. 1515 testes passando, 0 falhas.

**Paralelismo de testes:** `cargo test --workspace --no-fail-fast -- --test-threads=8`
funciona verificado empiricamente. Cada thread tem seu próprio `RT_PTR` TLS e seu
próprio `Runtime` — sem data races no runtime. Limitação: `TIMEOUT_EXPIRED`/
`PENDING_TIMER` são `static` (shared entre threads); testes que usam o timer de teste
podem racear se executados em paralelo. Os testes E2E de codegen não usam timer, então
rodam em paralelo sem problemas.

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

## Análise `constant` — Refatoração (2026-08-11)

Itens identificados na análise da funcionalidade `constant` após a remoção de
`@comptime`. Cada item descreve o problema, a proposta e o status.

### C1. Remover `TypedExprKind::Comptime` (código morto) ✅

**Concluído (2026-08-11).** Removidos `Expr::Comptime` da AST,
`TypedExprKind::Comptime` da TAST, o braço de inferência que os conectava, o
arquivo `replace.rs` inteiro (substituição de nós Comptime), `contains_comptime`
e `walk_ref` de `walk.rs`, e 18 match arms em 15 arquivos (tree-shaker,
monomorph, codegen, walk, pureza, constness, tail_call, cache_key, desugar,
desugar_holes, hover, test helpers). O fixpoint loop do comptime pass foi
simplificado: removidas as calls a `replace_comptime_in_place` e
`contains_comptime`, ficou só o processamento de `ConstantBinding` +
`fold_literal_calls` + predicates. `fold_literal_calls` mantido — folda
`Closure` com args literais, independe de `Comptime`. 1541 passed, 0 failed.

### C2. Pre-pass dedicado para avaliação de constants (sem fixpoint) ✅

**Concluído (2026-08-11).** Separada a avaliação de constants do fixpoint loop.

Mudança arquitetural:
1. `evaluate_constants()` — pre-pass linear que percorre constants em ordem
   de declaração, avalia cada uma uma vez (JIT-executa, substitui por literal
   ou HeapSnapshot). Sem fixpoint. A ordem de declaração é a ordem de
   dependência — a inferência já garante que forward references falham com
   `UnboundName` (comportamento correto).
2. Fixpoint loop simplificado — só `fold_literal_calls` (cascata de folds:
   o resultado de um fold pode ser arg literal de outro). Após foldar uma
   constant, registra o valor em `comptime_bindings` imediatamente.
3. `is_already_evaluated` deixou de ser workaround anti-loop — é só um skip
   de "já pronta" no pre-pass linear (constants importadas, já avaliadas).

Constants que são Closures (chamadas de função) são puladas no pre-pass e
deixadas para o `fold_literal_calls` no fixpoint. Após o fold transformar
uma Closure em literal, o valor é registrado em `comptime_bindings`.

1541 passed, 0 failed.

### C3. Inferência dedicada para constants (sem wrapping em `Expr::Let`) ✅

**Concluído (2026-08-11).** Inferência de constants sem envolver value em
`Expr::Let`. `check_constant_lambda` na inferência rejeita lambdas/sections
em `constant` (Arthur: use sintaxe de função nomeada no top-level).
`type_env.define(name, ty, "__module__")` direto (sem `set_origin` hack).

`constness.rs` (novo módulo em `kata-inference/src/infer/`):
- `peel_to_lambda_ty(expr)` — detecta Lambda após desugar de holes
- `check_constant_lambda(name, value, span)` — retorna `ConstantLambda` se value é lambda
- `is_consttime_available_at_infer(expr, type_env, dispatch_table)` — versão simplificada

Pureza (`check_purity`) e comptime-availability (`is_comptime_available`)
continuam no comptime pass — são context-dependent (testes que não rodam
comptime pass, como `spawn_ipc_e2e`, dependem de `constant ch := channel!()`
impuro funcionando). Mover para inferência quebraria esses testes.

8 testes migrados de `constant f := + _ 2` (section → lambda) para named functions.
1541 passed, 0 failed.

### C4. Folding de chamadas literais em corpos de functions ✅

**Concluído (2026-08-11).** Após `fold_constant_refs_in_functions` substituir
`Ident("const")` → literal nos corpos, `fold_literal_calls` roda nos corpos
de functions (Fase 3b do comptime pass). Fixpoint local por function — o
resultado de um fold pode ser arg literal de outro fold na mesma function.

Guard adicional em `fold_literal_calls`: chamadas que retornam `Ty::Function`
(closures) não são foldadas — ponteiros de função JIT não são serializáveis.
Sem este guard, `make_adder 5` era foldado para ponteiro bruto → SIGSEGV.

A recursão é tratada pelo `catch_unwind` existente — se o JIT executa
`fatorial 10` recursivamente e termina, folda; se panica, não folda.

1546 testes passando, 0 falhas, 9 ignorados.

---

## Fora do Escopo 1.0

Mantidos no ROADMAP. Não mover para cá sem decisão explícita.

- Tensor/SIMD
- `@heapstack` (otimização heurística de arena em loops)
- `@restart` (retry policy para Actions)
- Doc comments (`///`, `\"\"\"doc\"\"\"`)
- Tuplas variádicas (`T...`)
- GC/reclamation granular para fibers long-lived