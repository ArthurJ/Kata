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

### `--emit-ir` não implementado

**Estado:** O manual descreve `--emit-ir` em `kata run` e `kata eval` para
imprimir a CLIF canônica antes da execução. A flag não existe no código
(`clap` CLI nem a declara). O manual foi mantido descrevendo-a como contrato.

**Impacto:** Baixo. Útil para depuração do codegen. Sem ela, usar
`eprintln!("{}", ctx.func.display())` no código.

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

## Migração de Exemplos

### `parallel.kata` (Cluster 4)

**Estado:** `spawn!` totalmente implementado (parser, inference, codegen, runtime). 10 testes E2E passando. Falta apenas criar `examples/parallel.kata` migrando `examples/legacy/test_parallel.kata` (ajustar sintaxe `n :: Int` → `n::Int`).

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD
- `@restart` (retry policy para Actions)