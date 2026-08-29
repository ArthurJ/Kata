# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-29.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### Facts de path conditions sobre `var` são insound (pré-existente)

**Estado:** `PathConditionCtx` coleta facts de guard mesmo quando o
scrutinee referencia variável `var` (mutável). `var d := n` + guard
`> d 0` no braço + `d := 0` (reassign) + `d::PosInt`: o fact `> d 0`
continua asserido e o Z3 "prova" `d::PosInt` quando `d` vale 0.
Probe P4 (2026-08-29) compilou e imprimiu 0 — prova errada aceita.

**Causa:** facts são TypedExprs que referenciam o NOME `d` em duas
leituras (guard e ascription) mas o valor mudou entre elas. Sem SSA
não há como conectar as duas leituras ao mesmo valor.

**Correção provável:** ao coletar fact de guard, checar se o scrutinee
referencia binding mutável (`env.is_mutable`) e descartar o fact
(conservador) — a prova sobre mutáveis nunca é sound sem SSA.

**Prioridade:** média — o caso exige reassign do mesmo `var` entre
guard e ascription, padrão raro, mas o resultado é aceite silenciosa
de código errado (pior que falso negativo).

### Vazamento de binding em escopo filho no codegen JIT (pré-existente)

**Estado:** o JIT aloca slots de variáveis como escopo PLANO — binding
nascido em braço de match/for/pattern sobrescreve o slot do binding
externo de mesmo nome, e o valor interno vaza para depois do braço.
O interp (oráculo) está correto em todos os casos.

Probes (2026-08-29, `main!(7)`, esperado `5/7`, JIT dá `5/5`):
- P3b let-let aninhado, P7b let externo + var interno
- P15 for-binding sobre let externo (esperado `1 2 7`, JIT dá `1 2 2`)
- P16 pattern binding sobre let externo (esperado `2 1`, JIT dá `2 2`)
- P18 var sobre var aninhado, P19 pattern sobre var externo

**Causa provável:** alocação de slot por NOME (não por escopo/decl) no
codegen — braço e corpo compartilham o slot.

**Prioridade:** média-alta — divergência JIT/interp em código legal;
`scripts/diff_interp_jit.sh` pode transformar os probes em oráculo.

### `examples/refined_types.kata` quebrado no bloco 4 (pré-existente)

**Estado:** bloco 4 usa `< a b` sobre `PositiveInt` — não há overload
de `<` para refined sem `refines ORD` (comportamento documentado:
"`refines NUM` não dá comparadores"). O exemplo não compila desde a
migração (47ee99d). Provavelmente o exemplo queria demonstrar downcast
(`a::Int`) ou o bloco deve migrar para `refines ORD`.

**Prioridade:** baixa — exemplo didático, não código de produção.

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

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD

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

### kata-interp

- (resolvido 2026-08-29) ~~`eval.rs:994/1004` — DictLit/SetLit stubs~~.
  Interpretador agora insere keys/values com hash/eq_fn resolvidos por tipo,
  espelhando o codegen.

### kata-rt

- **`src/ipc.rs:157`** — Implementar `spawn` no Windows. Ver
  `docs/PRDs/PRD-portability-windows.md`.

### kata-inference

- **`tests/csp_typeck.rs:221`** — Test placeholder: `select_arms_different_types`
  depende de T0 unification. Corpo vazio, sem assertions. Item de futuro.