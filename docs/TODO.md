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
**Estado:** Nível 1 implementado (PRD `docs/PRDs/PRD-refinement-propagation.md`).
Tipos refinados já existem — predicados no `StructRegistry`,
smart constructor falível, `const_eval_predicate` valida literais em
compile-time. **Nível 1 (guards locais) implementado:** `PathConditionCtx`
em `InferCtx` coleta facts de guards e `match` sobre `Boolean` no visitor
de inferência. `try_prove_with_path_conditions` (Z3) prova ascriptions
refinadas sobre não-literais. 8 testes E2E em
`t_refinement_propagation_e2e.rs`. 1823 testes, 0 regressão.
Níveis restantes:
- **Nível 2 — Pattern matches:** facts extraídos de patterns (ex: braço
  `Result::Ok n` de `div` sabe `b ≠ 0`). Exige propagar pré-condições de
  funções, não só guards locais.
- **Nível 3 — Contratos de função:** tipos refinados em assinaturas
  (`div :: Int NonZero => ...`) propagados como path conditions no caller.
  Refinement typing completo — o typeck consulta predicados do
  `StructRegistry` em cada ascription contra as constraints acumuladas.

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


