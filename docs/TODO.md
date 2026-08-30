# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-30.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

Resolvidos em 2026-08-29/30 (ver `git log`): escopo plano (typeck+interp),
facts de path conditions sobre `var` (Z3 ignora var), `@cache` no interp,
`refined_types.kata` bloco 4 (dual refines NUM+ORD), show de refined no
interp, `Text implements EQ`.

---

## Débito Técnico

### Crashes do JIT em programas semanticamente inválidos (pré-existente)

**Estado:** programas válidos sintaticamente mas rejeitados pelo typeck
podem derrubar o compilador com SIGSEGV/null-deref em vez de erro
gracioso. Reprodução conhecida: P19-like com `if` inválido → crash em
`bigint.rs` (null-deref). Compile-time DEVE falhar graciosamente —
nunca SIGSEGV/SIGILL — mesmo em input malformado que escapou do typeck.

**Causa provável:** janelas no codegen onde uma suposição do typeck
(retorno sempre presente, tag sempre definida) é dereferenciada sem
checagem. O crash some quando o programa é corrigido — a classe fica.

**Correção provável:** sweep de sites de deref em codegen que assumem
invariantes do typeck; falhar com `codegen.unsupported`/erro interno
gracioso quando a suposição não segura.

**Prioridade:** alta — crash do compilador é a pior classe de falha.

### Paridade de cache: tipos compostos na key do interp

**Estado:** o interp serializa a key de `@cache` por conteúdo apenas
para primitivos (Int/Float/Text). List/Struct/Tuple/Sum caem em miss
conservador PERMANENTE (executa + não insere) — o cache nunca aquece
para esses tipos. O JIT cobre compostos via type descriptor completo
(`cache_key.rs`).

**Impacto:** baixo hoje (compostos em função `@cache` perdem memoização
no interp, divergem do JIT só em custo, não em valor). Mas fib-like
sobre List penduraria o interp e não o JIT.

**Quando surgir caso de uso real:** estender `serialize_key_part` com o
mesmo type descriptor do codegen — a serialização é struct-tag-value
recursiva, sem dependência de codegen (dá para mover para crate
compartilhada).

**Prioridade:** média.

### Ascription refined de Text literal não const-avalia

**Estado:** `"ola"::NonEmpty` (literal de Text em ascription refined)
não const-avalia — o scanner de ascription refined cobre apenas
literals numéricos (IntLit/FloatLit). Text exige o construtor falível
(`NonEmpty "ola"` → `Ok`/`Err` com match).

**Impacto:** baixo — assimetria ergonômica entre Int e Text, não
incorreção (o construtor é a via geral e é sound).

**Prioridade:** baixa — unificar estenderia o const-eval para TextLit,
avaliando o predicado em compile-time sobre o literal.

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

### Codegen — `echo!(None)` (bug #2b)
**Estado:** Pendente. `Closure` com callee não-Ident
(`kata-codegen/src/lowering/closure.rs:349`) rejeita `echo!(None)`.
Bug separado da classe de exaustividade — PRD próprio quando atacar.

### Typeck — variante sem payload como argumento (#K-call)
**Estado:** Pendente (medido em `b5e2d9e`, 2026-08-30). `foo None` e
até `foo Optional::None` (qualificado) dão `type.no_overload` mesmo
no 2º nível (`Optional::(Boolean)`). Variante sem payload é
inexpressável como ARGUMENTO — só pattern. Consistente com
ret-directed dispatch (`Expr::Ident` não consulta hint), mas limita
a linguagem. Decisão de design, PRD quando atacar.

### Typeck — enum user-defined como payload de genérico (#K-enum-payload)
**Estado:** Pendente (medido em `b5e2d9e`). `Optional::(Encoding)` →
`type.mismatch` esperado `Encoding`, encontrado `Sum(Encoding) or
Generic(Encoding)` dentro do pattern; `Result::(Int, Encoding)` falha
até sem match (`Err(E|Text)` com E enum). Ortogonal à exaustividade —
provável elaboração de união/payload. PRD próprio quando atacar.

### Parser — parêntese interno em braço de match (#K-paren)
**Estado:** Pendente (medido em `b5e2d9e`). `Some (Some True):` →
parse error; o greedy de braço só aceita a forma qualificada interna
(`Some Optional::Some True:`). Limite sintático documentado
(probeK_deep_paren); mudar o parser de pattern é decisão de
linguagem.

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