# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-24.

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

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD

### Fiber arena: dealloc individual para fibers long-lived
**Estado:** Avaliar. O runtime tem três arenas:

1. **fiber_arena** (Bump/bumpalo) — dados locais do fiber, reset O(1)
   quando o fiber termina. Sem dealloc individual.
2. **caller_arena** — passada pelo caller; no entry point é a root_arena.
   Valores que escapam para o caller (EscapeTarget::Caller) usam esta.
3. **root_arena** (Tracked) — arena raiz do Runtime, com dealloc individual
   via refcount. Valores que cruzam fronteiras de fibers (canais,
   EscapeTarget::Heap) usam esta.

Cons/HAMT alocam na fiber_arena quando o escape é Local, e na
caller_arena (ou root_arena) quando escapam. O problema é computação
local em fibers long-lived: listas/dicts construídos e descartados
dentro de um loop usam a bump arena, que só libera tudo no reset
quando a fiber termina. Para fibers curtas isso é ótimo. Para fibers
long-lived (loops, servidores) é crescimento sem bound.

**Tensão:** substituir bumpalo pelo modelo Tracked (std::alloc + dealloc
individual por escopo) resolve dados locais, mas Cons/HAMT são
persistent data structures — compartilham estrutura (partilha de células
Cons, HAMT nodes). Não dá para liberar uma célula quando a variável sai
de escopo porque outra parte do programa pode estar apontando para ela.
Só refcount resolve, e refcount na fiber arena significa trocar bumpalo
por algo mais caro.

**Direção:** modelo híbrido — dados lineares (Tuple, Array mutável, bytes)
com dealloc por escopo; persistent data structures com refcount. Ou
aceitar que fibers long-lived precisam de um mecanismo de GC para a
bump arena (periodic compaction, copying collection).

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