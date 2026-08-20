# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-20.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### Search paths configuráveis + import relativo

**Estado:** Imports são relativos apenas ao diretório do arquivo importador
(`entry_dir`) + stdlib. Não há como importar de diretórios pai (`import ../math`
— parser rejeita `..`), nem configurar search paths adicionais via CLI ou env var.
Import de módulo inteiro (`import mod` sem `.(items)`) **funciona** quando o
módulo está no search path correto — o diagnóstico anterior de "bug" era erro
de teste (path composto a partir de diretório errado).

**Impacto:** Projetos com estrutura de diretórios não-trivial não podem
organizar módulos em subdiretórios sem que cada arquivo importador esteja
no mesmo diretório do importado.

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