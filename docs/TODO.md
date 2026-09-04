# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-04 (A6 resolvido).

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

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

## Migração de Exemplos

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD
- **Sistema de supressão de diagnóstico** — braço de match redundante
  agora é erro (decisão 8 do PRD-exaustividade-aninhada); `otherwise`
  inútil é isento, mas patterns não-otherwise redundantes com
  intenção documentada precisam de via de escape (`@allow redundant`?).
  Projetar sintaxe e escopo de supressão.

### Patterns aninhados (Maranget + SMT)
**Estado:** Ativo — `docs/PRDs/PRD-exaustividade-aninhada.md`
(5 fases, revisado 2026-08-30, **Emenda 1** pousada). O buraco NÃO é o
parser (match arms e cláusulas qualificadas já parseiam aninhado
recursivo) — são os 3 CHECKERS (`check_exhaustiveness`,
`check_clause_exhaustiveness`, `pattern_covers`) que ignoram payload,
mais um panic de aridade em `lambda Some True:` desqualificado e um
falso-positivo de redundância. Bugs reproduzidos em `f64eff8`;
**Emenda 1:** F1 encolhe para Fundação (oráculos + bound-check +
parser + fall-through de codegen como no-op provado); a cobertura
recursiva ad-hoc foi REMOVIDA — F2 é motor antes dos consumidores
(`maranget.rs` puro com trait de ambiente, depois 3 consumidores, um
por commit). Fase 3 Z3 na folha, Fase 4 refined Int/Float na folha,
Fase 5 Rational (const-eval de `rational <lit>` + par (num, den) no
Z3). Oráculos adversariais K medidos em `b5e2d9e` (3 níveis, grade
multi-param, arity-tuple).

---

## Auditoria de completude — 2026-09-02

Análise conjunta (agente principal GLM-5.2 + sub-agente GLM-5.3 +
sub-agente kimi-k3:cloud) com probes reais. Cada item validado no
código-fonte e, quando possível, executado em ambos backends (JIT
e interpretador).

**Resumo:** 19 achados (A1–A12 + A3b–A3g). Resolvidos: A1, A2, A3b,
A3c, A3d, A3e, A3f, A3g, A5, A6, A7, A8, A9, A10, A11, e item adjacente #4 (JIT crash NonZero::Float).
Débito técnico de null-check (Cat 1, 2, 3) totalmente resolvido.
Pendentes: 0 médios, 1 baixo.

### 🟡 Médio — buracos funcionais que limitam a linguagem

#### A6. `@cache` no interp: miss permanente para tipos compostos — RESOLVIDO 2026-09-04

**Resolvido:** `serialize_key_part` agora serializa List, Tuple, Struct,
Array, Sum e Unit recursivamente por conteúdo, espelhando
`kata_rt_serialize_key` do runtime. `StructRegistry` passado como
parâmetro adicional. Set/Dict permanecem miss conservador (JIT também
não suporta). 8 testes E2E em `cache_interp_e2e.rs` com paridade
interp↔JIT.

#### A8. `show` de Struct incompleto no interpretador — RESOLVIDO 2026-09-04

**Resolvido:** `struct_field_count` removido; `show_struct` agora consulta o
`StructInfo` real do `struct_registry` (campos com nome e tipo). Introduzido
`repr_value` (quoteia Text dentro de containers) para paridade com
`repr_expr` do codegen. Separadores de List/Array corrigidos de espaço
para `, `. Testes E2E em `show_struct_interp_e2e.rs` (10 casos, paridade
interp↔JIT).

### 🟢 Baixo-médio — assimetrias e gaps menores

#### A12. `spawn!` no Windows é stub

**Estado:** (já documentado abaixo em "TODOs esparsos — kata-rt").

---

## Issues adjacentes descobertos ao resolver A3f/A3g (2026-09-03)

Itens encontrados durante o fix de A3f e A3g que são bugs separados,
fora do escopo da auditoria original.

### Interp — trampoline do scheduler engole erros

**Estado:** `interp_trampoline` (csp.rs:212-218) captura qualquer
`InterpError` (exceto `Return`), imprime no stderr, e retorna `0`.
O scheduler vê `0` como sucesso — o exit code do processo não reflete
o erro. Toda validação de erro gracioso do interp (incluindo A3g)
tem sua mensagem impressa mas não propagada como exit code não-zero.

**Localização:** `crates/kata-interp/src/csp.rs:212-218`.

**Impacto:** médio — qualquer erro runtime dentro de uma action no
interp produz exit 0. Tests E2E não podem confiar em exit code para
detectar falhas do interp (precisam inspecionar stderr).

**Prioridade:** média — o trampoline retorna `i64` (não `Result`),
alinhado à FFI do scheduler. Mudar para propagar erro requer
reformular a interface trampoline/scheduler ou usar um canal lateral
(e.g. célula `Mutex<Option<InterpError>>` no `InterpCtx`).

---

## TODOs esparsos no código (pendentes de reavaliação)

Itens coletados de comentários `TODO` no código-fonte. Ainda não
triados — podem ser obsoletos, redundantes com itens acima, ou
ação imediata. Reavaliar caso a caso.

### kata-rt

- **`src/ipc.rs:157`** — Implementar `spawn` no Windows. Ver
  `docs/PRDs/PRD-portability-windows.md`.

### kata-inference

- **`tests/csp_typeck.rs:221`** — Test placeholder: `select_arms_different_types`
  depende de T0 unification. Corpo vazio, sem assertions. Item de futuro.