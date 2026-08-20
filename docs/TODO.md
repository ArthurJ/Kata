# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-20.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### `import` de módulo inteiro (sem `.(items)`)

**Estado:** `import modulo` (sem lista seletiva `.(items)`) falha com
"módulo não encontrado" tanto no pipeline normal quanto no REPL. O
`ModuleLoader` não resolve paths com `/` (ex: `examples/modules/mock_math`).
Import seletivo (`import mod.(fn)`) e import com alias funcionam normalmente.

**Impacto:** Usuários precisam sempre usar import seletivo. Não bloqueia
doctests (workaround: usar `.(items)`).

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