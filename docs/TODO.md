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

**Resumo:** 19 achados (A1–A12 + A3b–A3g). A1, A2, A3b, A4, A3e, A3f, A3g resolvidos.
Item adjacente #4 (JIT crash em NonZero::Float) resolvido.
1 crítico pendente, 1 alto, 6 médios, 3 baixos.

### 🟠 Alto — gaps que travam ou bloqueiam uso real

#### A3f. `len` em Text no interp retorna valor errado (double SMI tag)

**Estado:** ✅ Resolvido (2026-09-03). `kata_rt_text_len` (slice.rs) já
retorna `tag_smi(count)` — SMI-tagged. O dispatch do interp
(`ffi_dispatch.rs:328`) aplicava `encode_smi` sobre o retorno,
causando double tagging. Removido o `encode_smi` redundante.

**Correção:** uma linha — `encode_smi(rt::kata_rt_text_len(args[0]))`
→ `rt::kata_rt_text_len(args[0])`. Testes E2E em
`kata-driver/tests/text_len_interp_e2e.rs` (5 casos: vazio, ASCII,
acento, CJK, variável).

#### A3g. Interp não valida ascription refined — aceita `0 :: NonZero`

**Estado:** ✅ Resolvido (2026-09-03). O typeck produz
`pending_predicates` quando o predicado é complexo (ex: `!= _ (zero _)`)
— o comptime pass do JIT os valida via `jit_execute_expr`, mas o interp
não tem comptime pass. O `eval` do interp agora valida
`pending_predicates` no ponto de uso: avalia cada predicado com `eval`
e verifica se retorna `Boolean::True` (tag 1). Se falhar, emite
`InterpError::Runtime` gracioso em vez de SIGABRT.

**Correção:** branch `TypeAscription` em `eval.rs` — capturar
`pending_predicates` (antes descartado com `..`) e avaliar antes de
prosseguir. Equivalente interp do `validate_pending_predicates` do
comptime pass.

**Nota:** o trampoline do scheduler (csp.rs:212-218) engole erros e
retorna 0 — o exit code não reflete o erro (impresso no stderr). Bug
separado do trampoline, não do A3g.

**Restante (JIT):** o JIT ainda falha com `comptime.jit_failure` (erro
interno) em vez de `type.mismatch` gracioso. Isso é um issue do comptime
pass, não do interp.

Testes E2E em `kata-driver/tests/refined_ascription_interp_e2e.rs`
(4 casos: zero rejeitado, válido, unit, PositiveInt).

#### A5. `echo!(None)` rejeitado pelo codegen JIT

**Estado:** `Closure` com callee não-Ident não é suportado. O interp
executa `echo!(None)` (exit 0); o JIT rejeita com
`codegen.unsupported`. Discrepância de backend.

**Localização:** `crates/kata-codegen/src/lowering/closure.rs:349-355`.

**Reprodução:** `echo!(None)` → JIT: exit 1 (codegen.unsupported);
interp: exit 0.

**Prioridade:** alta — expressão válida rejeitada por um backend.

### 🟡 Médio — buracos funcionais que limitam a linguagem

#### A3c. `show Optional::None` → ffi_not_found no JIT / placeholder no interp

**Estado:** `show Optional::None` falha no JIT com
`codegen.ffi_not_found: __kata_show__Optional` — o monomorphizador
não instancia show quando o type param não é concreto (None não
carrega tipo). No interp, retorna placeholder
`<show:Generic("Optional", [Var("T")])>` em vez do nome da
variante. `show (Some 0)` e `show (Ok 0)` funcionam normalmente.

**Localização:** `crates/kata-monomorph/src/overload_resolution.rs`
(não instancia show para Optional sem tipo concreto),
`crates/kata-interp/src/show.rs:75` (placeholder para tipo não
resolvido).

**Reprodução:** `echo!(show Optional::None)` → JIT: exit 1
(ffi_not_found); interp: `<show:Generic("Optional", [Var("T")])>`.

**Prioridade:** média — `Optional::None` é o valor nulo de Kata;
`show` dele deveria imprimir `None`.

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

#### A3e. Range com step 0 dinâmico → loop infinito

**Estado:** ✅ Resolvido. Defense in depth — compile-time + runtime.

**Compile-time:** `check_neutral_step` generalizado via
`ConstVal::zero_for_ty` (espelha a função `zero` da interface NUM).
Cobre Int, Float, e Rational literals. Step literal 0 →
`TypeMismatch` gracioso.

**Runtime:** `range_check_step` inserido em 5 sites de iteração
sobre Range (for_in, map, filter, fold, fused_stream). Step dinâmico
0 → `kata_rt_panic` com mensagem clara.

**Interp:** já era safe — `RangeLit` com step 0 produz lista vazia
(não itera).

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

### Typeck — literal negativo não reconhecido como literal em ascription refined

**Estado:** `(-5) :: NonZero::Int` é rejeitado pelo typeck com
"literal para ascription refined NonZero (use construtor para expr
não-literal)". O typeck tipa `-5` como `Closure { Ident("-"),
[IntLit("5")] }`, e o check `is_literal` em `ascription.rs:331` só
cobre `IntLit`/`FloatLit`/`TextLit`/`ListLit`/etc — não
`Closure`. O `const_eval` (`eval_const`) suporta negativos
(`Apply { -, [IntLit] }` → `ConstVal::Int(-N)`), mas o gate
`is_literal` barra antes de chegar lá.

**Localização:** `crates/kata-inference/src/infer/ascription.rs:331`
(`is_literal` check), linha 347 (gate que rejeita não-literais sem
path conditions).

**Impacto:** baixo — `(-5) :: NonZero::Int` exige construtor
(`NonZero (-5)`) em vez de ascription direta. Assimetria ergonômica
com `5 :: NonZero::Int` que funciona.

**Prioridade:** baixa — estender `is_literal` para reconhecer
`Closure { Ident("-"), [IntLit/FloatLit] }` como literal negativo,
ou usar `typed_expr_to_const_val` (que já suporta negativos) como
gate em vez de `is_literal`.

### ✅ JIT — `0::NonZero::Float` e `3.14::NonZero::Float` crasham no codegen

**Estado:** ✅ Resolvido (2026-09-03). Três bugs corrigidos:

1. **Comptime pass compilava actions desnecessariamente** —
   `validate_pending_predicates` passava `ctx.actions` (TODAS as
   actions do módulo) para `jit_execute_expr`, fazendo o codegen
   compilar `echo!` (que chama `show` de Instance) ao validar
   predicados. Predicados de refined só usam operadores e funções de
   interface, nunca actions. Fix: `&[]` em `predicates.rs`.

2. **Show synthesis não registrava show por Instance concreta** —
   `show_synthesis` registrava `show :: Plain("NonZero") => Text`
   mas valores têm tipo `Instance("NonZero", "Int")`. O
   monomorphizador faz match exato e não encontrava. Fix: bloco
   dedicado em `show_synthesis.rs` que registra
   `show :: Instance(family, concrete) => Text` para cada Instance.

3. **`match_score` tratava Instance↔Plain como exact match** —
   `Instance("NonZero", "Int")` casava com `Plain("NonZero")` com
   mesmo score que `Instance("NonZero", "Int")` exato, causando
   `AmbiguousDispatch`. Fix: Instance↔Plain agora é `iface` (não
   `exact`), dando prioridade ao match Instance↔Instance exato.

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