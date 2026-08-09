# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-08 após auditoria completa do código.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### 1. `@test{expects: "CompileError"}` — execução não implementada

**Estado:** Parser aceita, codegen cria placeholder, driver imprime `[PENDENTE]` e pula.

**O que falta (design C1 — sub-módulos isolados):**
1. Extrair o sub-módulo referenciado por `expects`
2. Compilá-lo isoladamente (lex → parse → resolve → infer)
3. Verificar que a compilação falha com o erro esperado (substring match)
4. Reportar PASS se falhou com o erro esperado, FAIL se compilou ou falhou com erro diferente

**Arquivos:** `crates/kata-driver/src/main.rs:234-242`, `crates/kata-codegen/src/lowering/test_runner.rs:68-72`, `crates/kata-driver/tests/test_runner_e2e.rs` (teste `#[ignore]`)

**Impacto:** Médio. Testes negativos são silenciosamente pulados — o programador escreve o teste, o runner diz `[PENDENTE]`, mas a validação nunca acontece.

### 2. SIGCHLD handler — reap automático de processos filhos de `spawn!` ✅

**Resolvido.** Instalado `sigaction(SIGCHLD, SIG_IGN, SA_RESTART)` antes do
`fork()` via `std::sync::Once` em `kata_rt_spawn_process`. O kernel descarta
o status do child automaticamente — nenhum zombie é criado. `SIG_IGN` para
SIGPIPE consolidado no mesmo `Once` (antes era instalado após o fork em cada
`spawn!`); o child herda ambos via `fork()`.

`spawn!` é fire-and-forget por design — toda comunicação entre actions é por
canais, nunca por join/waitpid. `SIG_IGN` é a solução definitiva.

**Arquivos:** `crates/kata-rt/src/ipc.rs:18-47` (handler), `ipc.rs:73-77` (call site)

### 3. First-class Action reference ambígua — OverloadSet ✅ Resolvido

**Resolvido.** Implementado em duas camadas:

1. **Hint-based (segunda versão do PRD §12):** `select_action_overload` em
   `expr.rs` usa o hint de tipo esperado (`Ty::Action(params, ret)`) para
   selecionar o overload compatível via `match_score` quando há múltiplos
   overloads. Se o hint resolve para um único overload, produz `Ty::Action`
   concreto.

2. **OverloadSet (Fase 1 + Fase 2):** Quando não há hint ou há múltiplos
   compatíveis, o typeck produz `Ty::OverloadSet { name, overloads }` —
   tipo interno que carrega os overloads adiante. No call site (`f!(args)`),
   o dispatch por args usa `match_score` para selecionar o overload
   compatível e resolve para `ActionCall` direto com `callee = action_name`.

   **Fase 2 (monomorfização):** `let f := worker` onde `worker` é uma action
   genérica (`msg :: SHOW`) produz `Ty::Action([Interface("SHOW")], ())`.
   O monomorfizador instancia `worker_SHOW_Text` e `echo_SHOW_Text`,
   remove as templates genéricas de `typed.actions`, e remove
   `indirect_callee` do `ActionCall` para que a chamada seja direta.

**Arquivos:**
- `crates/kata-core/src/ty.rs` — `Ty::OverloadSet`
- `crates/kata-inference/src/infer/expr.rs` — `select_action_overload`, caminho 3,
  `fn_alias` estendido para Actions
- `crates/kata-inference/src/infer/action_call.rs` — dispatch por args para OverloadSet,
  `match_score` no caminho indirect, `callee = fn_alias_of(callee)`
- `crates/kata-resolution/src/lib.rs` — `resolve_with_prelude`
- `crates/kata-monomorph/src/lib.rs` — remoção de templates genéricas,
  remoção de `indirect_callee` ao instanciar
- `crates/kata-monomorph/src/overload_resolution.rs` — guard `Ty::Interface`
  em `instantiate_generic_action_call`, `resolve_erased_ffi_symbol` reescreve callee
- `crates/kata-codegen/src/lowering/expr.rs` — placeholder para Ident com OverloadSet
  e `Ty::Action` com `Interface(_)` nos params
- `crates/kata-codegen/tests/overloadset_actions.rs` — 10 testes E2E

   **Fase 3 (passar OverloadSet como argumento):** `dispatcher!(echo)` passa
   uma action como argumento para outra action. O `match_score` (braço
   OverloadSet vs Action) aceita na inference. O monomorphizer instancia
   `echo_SHOW_Text` na posição de argumento (`instantiate_overloadset_arg`)
   e rewrites o arg de `Ident("echo")` + `OverloadSet` para
   `Ident("echo_SHOW_Text")` + `Action([Text], Unit)`. O codegen encontra a
   instância em `kata_ids` e produz fn_ptr válido — sem segfault.

   A concretização acontece na monomorphization (estágio correto do pipeline),
   não na inference. Sem `convert_overloadset_args` ou `dispatch_params`.

**Arquivos:**
- `crates/kata-core/src/ty.rs` — `Ty::OverloadSet`
- `crates/kata-core/src/dispatch.rs` — braço OverloadSet vs Action no `match_score`
- `crates/kata-inference/src/infer/expr.rs` — `select_action_overload`, caminho 3,
  `fn_alias` estendido para Actions
- `crates/kata-inference/src/infer/action_call.rs` — dispatch por args para OverloadSet,
  `match_score` no caminho indirect, `callee = fn_alias_of(callee)`
- `crates/kata-resolution/src/lib.rs` — `resolve_with_prelude`
- `crates/kata-monomorph/src/lib.rs` — remoção de templates genéricas,
  remoção de `indirect_callee` ao instanciar, call site de `instantiate_overloadset_arg`
  no braço ActionCall de `rewrite_typed_expr`
- `crates/kata-monomorph/src/overload_resolution.rs` — guard `Ty::Interface`
  em `instantiate_generic_action_call`, `resolve_erased_ffi_symbol` reescreve callee,
  `instantiate_overloadset_arg` instancia action genérica na posição de argumento
- `crates/kata-codegen/src/lowering/expr.rs` — placeholder para Ident com OverloadSet
  e `Ty::Action` com `Interface(_)` nos params
- `crates/kata-codegen/tests/overloadset_actions.rs` — 10 testes E2E

**Cobertura:** `let f := echo` sem uso, `f!("hello")` (dispatch por args),
`f!(42)` (Int implementa SHOW), dispatch por arity, action genérica com SHOW
via `let f := worker` (Text e Int), chamada direta `worker!("hello")`,
`dispatcher!(echo)` (OverloadSet como arg), `let f := echo; dispatcher!(f)` (via variável).

---

## Migração de Exemplos

### `parallel.kata` (Cluster 4)

**Estado:** `spawn!` totalmente implementado (parser, inference, codegen, runtime). 10 testes E2E passando. Falta apenas criar `examples/parallel.kata` migrando `examples/legacy/test_parallel.kata` (ajustar sintaxe `n :: Int` → `n::Int`).

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## LSP — Fase 4 (parcial)

**Estado:** Fases 1-3 ✅. Fase 4 parcial: error recovery implementado (`parse_with_recovery` no parser + LSP).

**Faltam:**
- Benchmark de latência (DoD: < 100ms para arquivos < 500 linhas)
- Subcomando `kata lsp` no CLI (opcional — binário `kata-lsp` já existe como processo separado)

**PRD:** `docs/PRD-lsp.md`

---

## Arquitetura — Análise de Refatoração (do zero)

Itens identificados na análise arquitetural de 2026-08-09. Cada item descreve
o problema, a proposta, e o status (analisar / decidir / executar).

### A1. Orquestração do pipeline como composição de passos ✅ Resolvido

**Resolvido.** Implementado `pipeline.rs` no `kata-driver` com um `Pipeline`
struct composicional. Cada passo (lex, parse, resolve, desugar, infer,
monomorph, optimize, tree_shake, comptime, build_type_table) existe uma
única vez como método que consome `self` e produz o próximo estado. Os três
backends (JIT, test, AOT) escolhem os modos (`ParseMode::TwoPass` vs `Single`,
`ShakeMode::Default` vs `PreserveTests`) e terminam com `.jit_eval()`,
`.jit_tests()` ou `.aot_emit()` sobre o `CompiledModule`.

**Bônus:** A unificação corrigiu 3 divergências do AOT que eram bugs por
omissão — AOT agora faz 2-pass parse, `desugar_directives`, e
`resolve_with_imports` (diretivas importadas), como JIT e test.

**Arquivos:**
- `crates/kata-driver/src/pipeline.rs` — novo, `Pipeline` + `CompiledModule`
- `crates/kata-driver/src/main.rs` — `run_pipeline_with_file` e `cmd_test`
  reescritos como chamadas ao Pipeline (~90 linhas removidas)
- `crates/kata-driver/src/aot.rs` — `cmd_build` reescrito (~40 linhas
  removidas), imports limpos

### A2. Runtime reentrante — eliminar globals e TLS

**Problema:** `TYPE_TABLE: LazyLock<Mutex<...>>`, `SHARED_ARC`, `SHARED_ARENA`,
`PER_FIBER_ARENA` (TLS), scheduler em `thread_local!`. `kata_rt_run` não é
concorrente — testes precisam ser serializados. Estado global impede
múltiplas execuções isoladas no mesmo processo.

**Proposta:** O scheduler e as arenas viveriam num `Runtime` struct explícito,
passado por referência ao código JIT (generalizando o padrão já usado para
`fiber_arena`). Em vez de `static TYPE_TABLE`, o `Runtime` carrega a type
table. Torna o runtime reentrante, permite testes paralelos, remove a
fragilidade de estado global.

**Status:** Analisar. O custo é mais um parâmetro nas FFIs — mas o
`fiber_arena` já é passado dessa forma; é generalizar o padrão. Avaliar
impacto no codegen e na ABI das FFIs.

### A3. Decompor LowerCtx em sub-contextos estratificados

**Problema:** `LowerCtx` tem 30+ campos misturando 5 concerns: bindings,
closures, escape/ARC, type IDs, function-level state, arena management,
I/O handles, loop/continue blocks, epílogo de Action. O doc
`maquinaria-interna.md` chama de "a struct mais complexa".

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

### A5. Eliminar dead code — ModuleLoader não usado pelo driver

**Problema:** O `ModuleLoader` é testado mas é dead code — não é chamado pelo
driver. O driver tem `imports::load_module_imports` que reimplementa a lógica.
Dois sistemas de import, um não usado.

**Proposta:** Deletar o `ModuleLoader` (e seu cache de TypeEnv) e documentar
que o driver é o sistema. Se funcionalidade do ModuleLoader for necessária
no futuro (ex: caching real de módulos), re-implementar no driver.

**Status:** Executar. Deletar dead code — sem perda de funcionalidade.

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
- Doc comments (`///`, `"""doc"""`)
- Tuplas variádicas (`T...`)
- GC/reclamation granular para fibers long-lived