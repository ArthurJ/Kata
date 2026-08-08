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

2. **OverloadSet (Fase 1):** Quando não há hint ou há múltiplos compatíveis,
   o typeck produz `Ty::OverloadSet { name, overloads }` — tipo interno que
   carrega os overloads adiante. No call site (`f!(args)`), o dispatch por
   args usa `match_score` para selecionar o overload compatível e resolve
   para `ActionCall` direto com `callee = action_name`.

**Arquivos:**
- `crates/kata-core/src/ty.rs` — `Ty::OverloadSet`
- `crates/kata-inference/src/infer/expr.rs` — `select_action_overload`, caminho 3
- `crates/kata-inference/src/infer/action_call.rs` — dispatch por args para OverloadSet
- `crates/kata-codegen/src/lowering/expr.rs` — placeholder para Ident com OverloadSet
- `crates/kata-codegen/tests/overloadset_actions.rs` — 5 testes E2E

**Cobertura:** `let f := echo` sem uso, `f!("hello")` (dispatch por args),
`f!(42)` (Int implementa SHOW), dispatch por arity. Actions em módulos
diferentes (monomorfização) fica para Fase 2.

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

## Fora do Escopo 1.0

Mantidos no ROADMAP. Não mover para cá sem decisão explícita.

- Tensor/SIMD
- `@heapstack` (otimização heurística de arena em loops)
- `@restart` (retry policy para Actions)
- Doc comments (`///`, `"""doc"""`)
- Tuplas variádicas (`T...`)
- GC/reclamation granular para fibers long-lived