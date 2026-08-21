# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-20.

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

### `NonZero` só existe para Int — divisão por zero não é estaticamente segura para Float/Rational

**Estado:** `NonZero` (refined `data (Int, != _ 0) as NonZero`) só é definido
para Int. `/ :: Int NonZero => Int` usa o refined para garantir divisor
não-zero em compile-time. Float e Rational não têm `NonZero` equivalente:

- Float: `/ :: Float Float => Float` é `a / b` direto — `0.0 / 0.0` produz
  NaN em runtime, invisibilizado no display (`float_to_text` converte NaN/Inf
  para 0). `div` verifica `= b 0.0` mas não captura NaN de outras fontes.
- Rational: `/ :: Rational Rational => Rational` panica em runtime se o
  divisor for zero. `div` retorna `Result` mas a overload `/` é insegura.
- Int: existe também `/ :: Int Int => Int` (legada) que panica em zero —
  mantida para compatibilidade.

**Impacto:** Médio. A linguagem oferece `div` (retorna `Result`) como
alternativa segura para todos os tipos, mas a overload `/` sem `NonZero`
para Float/Rational é uma armadilha — parece exata mas panica ou produz NaN.

**Quando surgir caso de uso real:** estender `NonZero` para Float
(`data (Float, != _ 0.0) as NonZeroFloat`) e Rational
(`data (Rational, != _ (rational 0)) as NonZeroRational`). As overloads
`/ Float NonZeroFloat => Float` e `/ Rational NonZeroRational => Rational`
garantiriam divisão exata segura em compile-time para todos os tipos NUM.
A overload legada `/ Int Int => Int` pode ser removida quando `NonZero`
for a única forma de dividir sem `Result`.

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