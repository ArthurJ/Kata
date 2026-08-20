# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-20.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### Recursão no REPL

**Estado:** Funções recursivas definidas no REPL falham no codegen com
"Closure sem ffi_symbol e callee não-Ident". O codegen não resolve o
símbolo da função dentro do próprio corpo quando a função é compilada
via `jit_eval_repl` (JIT incremental do REPL). Isto impede doctests
que definem funções recursivas (ex: `fatorial` recursiva). Funções
não-recursivas, constants, types, structs funcionam normalmente.

**Impacto:** Doctests no kata-book não podem usar recursão. O cap 16
foi escrito sem exemplos recursivos como workaround.

---

## Migração de Exemplos

### `parallel.kata` (Cluster 4)

**Estado:** `spawn!` totalmente implementado (parser, inference, codegen, runtime). 10 testes E2E passando. Falta apenas criar `examples/parallel.kata` migrando `examples/legacy/test_parallel.kata` (ajustar sintaxe `n :: Int` → `n::Int`).

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD
- `@restart` (retry policy para Actions)