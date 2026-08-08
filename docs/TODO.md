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

## Features Não Implementadas

### `@timer`

**Estado:** PRD completo em `docs/PRD-timer.md` (498 linhas). Não implementado.

**Resumo:** Diretiva que mede tempo de execução e publica via `@log`. Runtime precisa `kata_rt_timer_now()` (monotonic clock), codegen com `iconst` no prólogo e subtração no epílogo. Interação com `@cache` (cache hit → timer não dispara no epílogo) e `@log` (publica delta). Estratégia de TCO via canal buffer-1 drop para não bloquear fiber.

### `invoke!()` — dispatch dinâmico por string

**Estado:** Mencionado no `PRD-introspection.md` como PRD separado. Não escrito.

---

## Fora do Escopo 1.0

Mantidos no ROADMAP. Não mover para cá sem decisão explícita.

- Tensor/SIMD
- `@heapstack` (otimização heurística de arena em loops)
- `@restart` (retry policy para Actions)
- Doc comments (`///`, `"""doc"""`)
- Tuplas variádicas (`T...`)
- GC/reclamation granular para fibers long-lived