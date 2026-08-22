# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-22.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

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

### Parser deve validar que `refines` só aceita interface

**Estado:** `refines` deve SEMPRE ser sucedido de uma interface. Qualquer
coisa diferente (família, tipo concreto, outra refined) deve ser erro de
sintaxe. `PositiveInt refines NonZero` (família refina família) é ilegal.
O parser atual não valida isto — precisa adicionar check semântico (em pass0
ou no parser) que rejeita `refines <não-interface>` com erro claro.

**Impacto:** Baixo. `refines` em família já causa erro de dispatch
(comportamento correto), mas a mensagem é confusa — um erro de sintaxe
na declaração seria mais claro.

---

### Parser deve rejeitar nomes de função/action começando com `__`

**Estado:** `__` é prefixo reservado para símbolos gerados pelo compilador
(`__kata_fn_N`, `__kata_entry`, `__kata_test_*`, `__kata_show__*`,
`__pred_*`, `__local__`, etc.). Funções/actions definidas pelo usuário
NÃO devem começar com `__` — colidem com o namespace do compilador. O
parser atual não valida isto.

**Impacto:** Baixo. Colisão real é improvável (usuário raramente usa `__`),
mas a validação é trivial e preventiva.

---

### Tree-shaking por instância de família polimórfica

**Estado:** O tree-shaking remove funções por **nome** — se uma função
com overloads polimórficas é alcançada, **todas** as instâncias expandidas
sobrevivem, mesmo as nunca chamadas com aquele tipo concreto. Ex:
`mod :: Int Instance("NonZero","Float") => Int` sobrevive mesmo se `mod`
só é chamado com `Instance("NonZero","Int")`.

**Impacto:** Baixo. A expansão gera N overloads concretas por família; o
tree-shaking por nome preserva todas as N quando o nome é alcançado. As
overloads extras são inócuas em runtime (nunca executadas), mas ocupam
espaço no binário e tempo de compilação (Cranelift compila cada uma). Para
famílias pequenas (3 instâncias) e poucas funções, o custo é negligenciável.
Só seria significativo com famílias grandes (centenas de instâncias) e
corpos pesados — cenário extremo e improvável na prática.

**Quando surgir caso de uso real:** propagar o tipo do argumento até o
`collect_refs` do tree-shaking para distinguir qual instância específica
uma chamada refere-se a, permitindo remover overloads não-usadas antes
do codegen.

---

### Import implícito de stdlib via `mod.kata` (substituir `load_prelude` + `merge_two`)

**Estado:** O prelude (`stdlib/core.kata`) é carregado via `include_str!` em
`prelude_sigs.rs` e injetado no módulo do usuário via `merge_two`. Isto
bypassa o sistema de módulos — `filter_exports` não é aplicado, então tudo
no core é visível para o usuário (incluindo funções internas como
`bi_div`, `f_div`, `rat_div` de `core_internals`).

O sistema de módulos já suporta `mod.kata` (gateway com `export` seletivo)
e `filter_exports` com fechamento transitivo (exportar `Int` traz
automaticamente `NUM`, `EQ`, `+`, `show`, etc.). O que falta:

1. Criar `stdlib/mod.kata` com `import core` + `export` dos símbolos públicos
   (tipos + funções standalone). `core_internals` não é re-exportado.
2. Substituir `load_prelude() + merge_two()` por `ModuleLoader::load_imports()`
   com import implícito de `stdlib`. Aplicar em 3 callers:
   - `pipeline.rs` (linha 299+318)
   - `repl/mod.rs` (REPL)
   - `kata-lsp/src/analysis.rs` (linha 62+64)
3. Remover `prelude_sigs.rs` (substituído pelo ModuleLoader).
4. Decidir: stdlib embedded no binário (`include_str!`) vs filesystem em
   runtime. Hoje o ModuleLoader lê do filesystem; o prelude é embedded.
5. O `module_loader.rs` linha 337 (`is_core` bypass) é removido — o core
   passa a ter imports reais (`import core_internals`).

**Impacto:** Médio-alto. Resolve a visibilidade de funções internas,
unifica prelude e módulos no mesmo caminho de código, e prepara para
múltiplos módulos stdlib (math, complex, stdio) com visibilidade controlada.

---

### `with` sem guards e `let` em lambdas — RESOLVIDO

**Estado:** RESOLVIDO. O bloco `with` sem guards no path indentado já era aceito
pelo parser. O bug real era que `let` em lambdas era descartado pelo parser
(`parse_lambda_body_block` sobrescrevia todas as expressões sem guards exceto
a última). Corrigido: múltiplas expressões sem guards agora produzem `Expr::Block`.

Os 4 workarounds `otherwise:` em `math.kata` (`asin`, `acos`, `atan`, `atanh`
Complex) foram substituídos por `let` direto no body.

`with` same-line (após expressão na mesma linha do `lambda x:`) não é
suportado — o parser rejeita com mensagem específica indicando usar `let`
no body indentado ou `with` no path indentado.

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
- Renomear `@trace` de volta para `@log` (diretiva de telemetria)