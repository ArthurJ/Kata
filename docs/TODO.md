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

### 3. First-class Action reference ambígua — resolution por tipo esperado

**Estado:** Actions são first-class (PRD-first-class-actions ✅), mas
quando uma Action com múltiplos overloads é referenciada como valor (sem
`!`), o typeck pega silenciosamente o primeiro overload em vez de usar o
hint de tipo esperado para desambiguar. O PRD §12 (Riscos) prevê este
caso: "Primeira versão: erro se ambíguo. Depois: resolution por tipo
esperado do param." A implementação atual não faz nem uma coisa nem a
outra — pega `action_overloads[0]` sem verificar ambiguidade.

**O que falta:** Em `expr.rs` (caminho 3 — Ident como first-class Action
ref), quando `action_overloads.len() > 1`:
1. Tentar desambiguar pelo `hint: Option<&Ty>` — se o hint é
   `Ty::Action(params, ret)`, selecionar o overload compatível
2. Se não há hint ou há múltiplos compatíveis, emitir erro de ambiguidade
   (não pegar o primeiro silenciosamente)

**Arquivos:** `crates/kata-inference/src/infer/expr.rs:170-185`

**Impacto:** Baixo. O caso só surge com Actions overloadadas (como `read`
e `write` do prelude) referenciadas como valor first-class — algo raro na
prática. Actions de assinatura única (o caso comum em `fork!`) funcionam
corretamente. Overloading de Actions na chamada (com `!`) já funciona
via DispatchTable.

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