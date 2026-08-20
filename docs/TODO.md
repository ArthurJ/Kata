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

### `:load` falha com funções (lambda com cláusulas)

**Estado:** `:load` no REPL falha com "erro de parse (Pass 1): token
inesperado: esperado \`\`:\` após patterns da cláusula, encontrado
\`<INDENT>\`" quando o arquivo contém funções puras com `lambda` e
cláusulas indentadas. `parse_repl_decls_only` não parseia corretamente
a sintaxe `lambda a b\n  + a b` — espera `:` após patterns da cláusula,
mas encontra `<INDENT>`. `constant` e `@ffi` funcionam; apenas funções
com `lambda` quebram.

**Impacto:** Doctests com `>>> :load arquivo.kata` só carregam
constants e declarações `@ffi`/`Sig`. Não é possível carregar funções
puras definidas com `lambda` no arquivo.

### `import` no REPL

**Estado:** `import` é keyword da linguagem (declaração top-level), não
expressão. `eval_expr` no REPL só processa expressões — `>>> import
foo.(bar)` falha no `handle` porque não é uma expressão válida. Para
que doctests usem `import`, seria necessário fazer o REPL processar
`import` como declaração.

**Distinção `:load` vs `import` (design intencional):**
- `:load` carrega **tudo** do arquivo (declarações + entry point) e
  executa top-level. Side effects acontecem. Não exige `export`. Útil
  quando o doctest testa comportamento de um programa em execução.
- `import` traz **só os símbolos exportados**, sem executar top-level.
  Sem side effects. Exige `export` no módulo. Útil para testar funções
  isoladas (testes unitários de biblioteca).

Ambos têm uso legítimo em doctests. Ambos estão quebrados no contexto
de doctests: `:load` não parseia `lambda`, `import` não é aceito pelo
REPL. Corrigir os dois dá ao autor do doctest a escolha do mecanismo
apropriado.

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