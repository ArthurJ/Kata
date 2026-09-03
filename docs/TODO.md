# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-03.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### Crashes do JIT em programas semanticamente inválidos (pré-existente)

**Estado:** programas válidos sintaticamente mas rejeitados pelo typeck
podem derrubar o compilador com SIGSEGV/null-deref em vez de erro
gracioso. Compile-time DEVE falhar graciosamente — nunca SIGSEGV/SIGILL
— mesmo em input malformado que escapou do typeck.

**Diagnóstico (2026-09-02):** `.unwrap()` em src/ limpo (27 sites
convertidos para `.expect()`). `Err` com payload null corrigido (A1 —
10 sites agora usam `err_with_msg`). A classe restante é **deref de
ponteiro cru sem null-check** no codegen/runtime que assume invariantes
do typeck. Inventário completo em
`kata-compiler/references/raw-ptr-null-check-audit.md`.

**Inventário (4 categorias):**

- **Cat 1 — `&mut *(rt as *mut Runtime)` sem null-check (17 sites,
  CRÍTICA):** `runtime.rs` (9 sites: `rt_ref`, `depth_inc/dec/get/
  set_limit`, `reset_depth`, `yield_check`), `arena.rs` (7 sites:
  `arena_create/create_tracked/alloc/dealloc/...`), `scheduler/ffi.rs`
  (4 sites: `scheduler_init`, `spawn`, etc.), `marshal/mod.rs` (2
  sites). Boundary FFI — `rt` vem do codegen/JIT driver.
- **Cat 2 — `&*(val as *const T)` sem null-check (~15 sites,
  ALTA):** `rational.rs` (`rat_add/sub/mul/div/eq/lt/gt/neq/le/ge/
  show/to_float` — todas fazem `&*a` sem `is_null()`), `display.rs:58`
  (`print_result` com `TYPE_RATIONAL`).
- **Cat 3 — `read_unaligned(ptr as *const i64)` sem null-check
  (MÉDIA):** `dict/mod.rs`, `dict/hamt.rs`, `cache.rs`,
  `channel/ops.rs`, `channel/ipc.rs`. Ponteiros vêm de alocação
  interna — risco menor.
- **Cat 4 — Já protegidos:** `bigint.rs` (`deref_bigint` ✓),
  `file.rs`, `socket/mod.rs`, `channel/ops.rs`, `channel/ipc.rs`.

**Correção pendente:** criar helpers centralizados com null-check +
panic **unwind** (modelo: `deref_bigint` em `bigint.rs:96`):
`deref_runtime(rt) -> &mut Runtime`, `deref_rational(ptr) -> &BigRational`.
Substituir os 17+15 sites bare deref. Cat 3 é prioridade inferior.
Pré-requisito: verificar `panic` strategy do crate — se `panic = abort`,
panic unwind não funciona (alternativa: retornar erro).

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
não const-avalia — o const_eval cobre TextLit mas o predicado
`>= (len _) 1` não é reduzido pelo `eval_bool_expr` (só lida com
operadores de comparação `= != < > <= >=` entre ConstVals). Text
exige o construtor falível (`NonEmpty "ola"` → `Ok`/`Err` com match).

**Nota (2026-09-03):** `is_literal` agora cobre `Grouping` recursivamente
— `(5)::PositiveInt` e `((-5))::NonZero::Int` funcionavam antes por
outros caminhos, mas o gate `is_literal` não reconhecia parênteses.
Fix: helper `is_literal_expr` em `ascription.rs`, recursivo sobre
`TypedExprKind::Grouping`, consistente com `eval_const` e
`typed_expr_to_const_val`.

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

### Typeck — variante sem payload como argumento (#K-call)
**Estado:** Pendente (medido em `b5e2d9e`, 2026-08-30). `foo None` e
até `foo Optional::None` (qualificado) dão `type.no_overload` mesmo
no 2º nível (`Optional::(Boolean)`). Variante sem payload é
inexpressível como ARGUMENTO — só pattern. Consistente com
ret-directed dispatch (`Expr::Ident` não consulta hint), mas limita
a linguagem. Decisão de design, PRD quando atacar.

### Typeck — enum user-defined como payload de genérico (#K-enum-payload)
**Estado:** Pendente (medido em `b5e2d9e`). `Optional::(Encoding)` →
`type.mismatch` esperado `Encoding`, encontrado `Sum(Encoding) or
Generic(Encoding)` dentro do pattern; `Result::(Int, Encoding)` falha
até sem match (`Err(E|Text)` com E enum). Ortogonal à exaustividade —
provável elaboração de união/payload. PRD próprio quando atacar.

---

## Auditoria de completude — 2026-09-02

Análise conjunta (agente principal GLM-5.2 + sub-agente GLM-5.3 +
sub-agente kimi-k3:cloud) com probes reais. Cada item validado no
código-fonte e, quando possível, executado em ambos backends (JIT
e interpretador).

**Resumo:** 19 achados (A1–A12 + A3b–A3g). Resolvidos: A1, A2, A3b,
A3c, A3e, A3f, A3g, A5, e item adjacente #4 (JIT crash NonZero::Float).
Pendentes: 1 crítico (deref sem null-check), 5 médios, 3 baixos.

### 🟡 Médio — buracos funcionais que limitam a linguagem

#### A3d. `len` em Range → ffi_not_found

**Estado:** `len` despacha via COUNTABLE para Range, mas o codegen
procura `range_len` FFI que não existe. O typeck aprova (COUNTABLE
está implementado para Range), mas o codegen não tem o símbolo.

**Localização:** `crates/kata-codegen/src/` (procura `range_len`,
não registrado), `stdlib/core.kata` (Range implements COUNTABLE
com `len`).

**Reprodução:** `echo!(len [1..10])` → exit 1
(`codegen.ffi_not_found: range_len`). Interp:
`FFI não implementado no interpretador: range_len`.

**Prioridade:** média — typeck aprova, codegen não executa.

#### A6. `@cache` no interp: miss permanente para tipos compostos

**Estado:** (já documentado acima em "Débito Técnico"). Interp só
serializa primitivos; compostos = miss conservador permanente.

#### A7. Variante sem payload como argumento é inexpressível

**Estado:** (já documentado acima em "Typeck — variante sem payload
como argumento (#K-call)"). `foo None` e `foo Optional::None` dão
`type.no_overload`.

#### A8. `show` de Struct incompleto no interpretador

**Estado:** `struct_field_count` retorna 0 — structs com campos têm
show incompleto no interp (`show.rs:265-270`). Funciona no JIT via
`show_synthesis`.

**Localização:** `crates/kata-interp/src/show.rs:265-270`.

**Prioridade:** média — `echo!(show minhaStruct)` não funciona no interp.

#### A9. JIT depth tracking não cobre `BodyKind::CallInner`

**Estado:** `depth_tracking: matches!(body_kind, BodyKind::Clauses)`
— wrappers CallInner não incrementam depth. Wrappers "grátis"
subestimam profundidade real.

**Localização:** `crates/kata-codegen/src/lowering/function_def.rs:501`.

**Prioridade:** média — stack pode exceder antes do contador.

#### A10. Enum user-defined como payload de genérico falha no typeck

**Estado:** (já documentado acima em "Typeck — enum user-defined
como payload de genérico (#K-enum-payload)").

### 🟢 Baixo-médio — assimetrias e gaps menores

#### A11. Ascription refined de Text literal não const-avalia

**Estado:** (já documentado acima em "Ascription refined de Text
literal não const-avalia").

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

### JIT — ascription refined inválida dá `comptime.jit_failure`

**Estado:** `0 :: NonZero::Int` no JIT produz
`comptime.jit_failure: predicado de ascription refined falhou`
(erro interno) em vez de `type.mismatch` gracioso. O comptime pass
(`predicates.rs:47`) retorna `ComptimeError::JitError` com mensagem
de erro interno, e o driver o reporta como erro interno com help
"abra uma issue".

**Localização:** `crates/kata-comptime/src/predicates.rs:47-53`,
`crates/kata-comptime/src/error.rs` (ComptimeError::JitError).

**Impacto:** baixo-médio — o JIT rejeita corretamente (não executa),
mas a mensagem confunde o usuário (parece bug do compilador, não erro
de tipo do programa).

**Prioridade:** baixa — mudar `ComptimeError::JitError` para um
variant de type error gracioso com span e mensagem amigável.

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