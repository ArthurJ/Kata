# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-29.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### Guards via `with`: prova Z3 de exaustividade sempre falha

**Estado:** Confirmado empiricamente (2026-08-29, análise do compilador).
O `Z3Translator` de `guard_completeness.rs` traduz a condição do guard
para Z3 **sem resolver os bindings do `with`**. Um guard escrito como

```
lambda x:
    neg: -1
    ...
    with
        neg := < x 0
```

vira `Ident("neg")` na `TypedGuardClause` — o tradutor cria um Bool
livre `"neg"` no solver e a prova de tautologia **falha sempre**, mesmo
quando a disjunção é tautológica. A forma inline (`< x 0: -1` diretamente
na cláusula) funciona.

**Prova:** programa `sign` com guards `x<0 / x=0 / x>0` via `with` →
`type.non_exhaustive_match`; a mesma disjunção inline compila e roda.
Testes existentes (`kata-inference/tests/guard_completeness.rs`) só
cobrem a forma inline — falso verde.

**Impacto:** Médio-alto. O idiom `with` é a forma canônica (README,
fizzbuzz de `examples/`) e todo programa que usa guards via `with`
**sem** `otherwise` é rejeitado com erro falso. Workaround: escrever
guards inline ou manter `otherwise`.

**Fix natural:** expandir os with-bindings na condição antes de chamar
o solver. O `Z3PathTranslator` (`infer/path_conditions.rs`) já tem o
mecanismo (`try_inline` + `InlineFnTable`), mas é um tradutor duplicado
— o fix correto é unificar os dois tradutores em um só com resolução
de bindings, em vez de replicar a lógica de novo.

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