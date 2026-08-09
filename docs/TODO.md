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

### A5. Eliminar dead code — ModuleLoader não usado pelo driver ✅ Resolvido

**Resolvido (premissa incorreta).** O TODO original afirmava que
`ModuleLoader` era dead code não chamado pelo driver. Inspeção do código
mostra o oposto: `ModuleLoader` é usado ativamente em dois call sites:

1. `crates/kata-driver/src/imports.rs:222` — `load_module_imports` cria um
   `ModuleLoader` e chama `load_imports`. É o único caminho de import do
   driver (JIT, test, AOT).
2. `crates/kata-lsp/src/analysis.rs:90` — o LSP usa `ModuleLoader` para
   resolver imports multi-arquivo.

Não há dois sistemas de import concorrentes. `ModuleLoader` é o único.
Nada a deletar.

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