# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-23.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

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

### Refinement propagation (path conditions no typeck)
**Estado:** Nível 1 e Nível 2 implementados (PRD
`docs/PRDs/PRD-refinement-propagation.md`). Tipos refinados já existem —
predicados no `StructRegistry`, smart constructor falível,
`const_eval_predicate` valida literais em compile-time. **Nível 1
(guards locais) implementado:** `PathConditionCtx` em `InferCtx`
coleta facts de guards e `match` sobre `Boolean` no visitor de
inferência. `try_prove_with_path_conditions` (Z3) prova ascriptions
refinadas sobre não-literais. **Nível 2 (post-condições
inter-procedurais) implementado:** `PostCondTable` extraída de
funções com guards, consumo no `_match.rs`, `InlineFnTable` para
inlining de funções puras no Z3 translator, payload binding
conecta binding do pattern ao argumento. 13 testes E2E em
`t_refinement_propagation_e2e.rs`. 1841 testes, 0 regressão.
Níveis restantes:
- **Nível 3 — Contratos de função:** pré-condições inter-procedurais.
  Design especificado no PRD §9.9 (duas direções: A=aceitação via path
  conditions no dispatch, B=aprendizado de predicados após chamada).
  Reusa `refined_decls` e `try_prove_with_path_conditions` — sem
  desbloqueio. A implementar.

### TypeGraph: migrar 6 sites de classificação ad-hoc para o grafo
**Estado:** Pendente (consolidação de dívida técnica).
O `TypeGraph` (`kata-core/src/type_graph.rs`) centraliza a
classificação "este tipo é Instance ou Generic desta família?" num
único grafo construído após o Pass 0. Hoje essa decisão está
duplicada em checks ad-hoc contra `StructRegistry` em ~6 sites do
inference: `ascription.rs`, `dispatch.rs`, `apply_dispatch.rs`,
e provavelmente mais em `fits_return` e `try_refines_fallback`.
Migrar para consultar o grafo elimina classificação inconsistente
(um site trata `NonZero` como Instance, outro como Generic → bug
silencioso de dispatch). Para implementar: adicionar
`type_graph: &TypeGraph` ao `InferCtx` e migrar cada site.
Prioridade: baixa (o grafo já funciona para `resolve_type_expr`;
a migração é consolidação, não desbloqueio de feature).

### Patterns aninhados (Maranget + SMT)
**Estado:** Avaliar. O PRD-exaustividade §9 exclui patterns aninhados.
Hoje `Some(True)` não é verificado contra `Some(False)` — `Some` é tratado
como átomo no produto cartesiano. O parser atual não suporta patterns
profundamente aninhados. Se/when patterns aninhados forem suportados,
o algoritmo de Maranget completo beneficia de SMT para decidir cobertura
com guards mistos. Corner case — avaliar quando houver demanda real.

---

## TODOs esparsos no código (pendentes de reavaliação)

Itens coletados de comentários `TODO` no código-fonte. Ainda não
triados — podem ser obsoletos, redundantes com itens acima, ou
ação imediata. Reavaliar caso a caso.

### kata-rt

- **`src/ipc.rs:157`** — Implementar `spawn` no Windows. Ver
  `docs/PRDs/PRD-portability-windows.md`.


