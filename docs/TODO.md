# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-20.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### Search paths configuráveis (-I / KATA_PATH)

**Estado:** Import relativo via `super.` IMPLEMENTADO (PRD-modulos-super).
`mod.kata` como ponto de entrada de diretório IMPLEMENTADO. `stdlib.` como
namespace explícito IMPLEMENTADO. O que falta: search paths configuráveis
via CLI (`-I`) ou env var (`KATA_PATH`) para libs externas — não suportado,
Kata5 não tem libs externas por enquanto.

**Impacto:** Baixo. `super.` + `mod.kata` cobrem organização intra-projeto.
Search paths configuráveis só são necessários para libs externas.

---

### `filter_exports` não preserva dependências transitivas de funções

**Estado:** Se módulo A exporta `fn1` cujo corpo referencia `fn2` (não
exportada), e módulo B importa `A.(fn1)`, o `infer_module` falha com
`unbound_name` para `fn2`. O `filter_exports` remove `fn2` do
`ResolvedModule` filtrado, mas o corpo de `fn1` referencia `fn2`. O
`resolved_unfiltered` tem `fn2`, mas `merge_imports` usa o filtrado.

**Impacto:** Médio. Sub-módulos que importam de outros sub-módulos e
re-exportam funções que dependem de imports internos falham. Workaround:
exportar todas as funções referenciadas por funções exportadas, ou usar
funções autocontidas (sem dependências internas).

** Quando surgir caso de uso real:** avaliar se `filter_exports` deve
preservar o fechamento transitivo das referências de cada função exportada
(percorrer `TypedFunction` body em busca de `Ident` que resolvem para
signatures/functions do mesmo módulo).

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