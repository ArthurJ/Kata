# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-02.

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
do typeck.

**Correção pendente:** sweep de sites de deref de ponteiro cru sem
null-check no codegen/runtime — substituir por checks que emitem
`codegen.unsupported`/erro interno gracioso quando a suposição não
segura.

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

**Resumo:** 19 achados (A1–A12 + A3b–A3g). A1 resolvido. 2 críticos
pendentes, 5 altos, 8 médios, 3 baixos.

### 🔴 Crítico — SIGSEGV em programas válidos

#### A2. `head` de lista vazia → SIGSEGV/panic

**Estado:** `head :: List::A => A` (não Result) retorna 0 para
lista vazia (`list.rs:48-50`). `echo!(head [])` chama
`kata_rt_bi_show(0)` → `deref_bigint(0)` → panic non-unwinding →
SIGABRT. A assinatura mente: promete `A` mas retorna null.

**Localização:** `crates/kata-rt/src/list.rs:48-50`,
`crates/kata-rt/src/bigint.rs:96-99`, `stdlib/core.kata:652`.

**Reprodução:** `echo!(head ([] :: [Int]))` → exit -6 (SIGABRT) em
ambos backends.

**Prioridade:** crítica — typeck endossa assinatura que runtime
não honra.

#### A3b. Stack overflow em literals de lista profundamente aninhados

**Estado:** `[[[...[]...]]]` com ~800 níveis de aninhamento causa stack
overflow no parser (recursive descent sem limite de profundidade). O
crash ocorre em ambos backends (é frontend, não codegen). 400 níveis
funciona; 800 não. Não há `max_depth` ou limitador no parser.

**Localização:** `crates/kata-parser/src/expr_containers.rs:49`
(`parse_list_or_range` — recursivo sem limite).

**Reprodução:** `var x := [[[...800 níveis...[]...]]]` →
"thread has overflowed its stack / stack overflow, aborting" em
ambos backends.

**Prioridade:** alta — mesma classe do bug de recursão sem limitador,
mas no parser em vez de runtime.

### 🟠 Alto — gaps que travam ou bloqueiam uso real

#### A3f. `len` em Text no interp retorna valor errado (double SMI tag)

**Estado:** `len "abc"` retorna 3 no JIT (correto) mas 7 no interp.
`kata_rt_text_len` em `slice.rs:49` retorna `tag_smi(count)` (já
SMI-tagged), mas `ffi_dispatch.rs:328` chama `encode_smi()` sobre
o resultado — double tagging. `encode_smi(3) = 7`.

**Localização:** `crates/kata-interp/src/ffi_dispatch.rs:328`
(`encode_smi` sobre valor já SMI-tagged),
`crates/kata-rt/src/slice.rs:56` (`tag_smi(count)` no retorno).

**Reprodução:** `echo!(len "abc")` → JIT: 3; interp: 7.

**Prioridade:** alta — incorreção silenciosa: todo `len` de Text
no interp retorna o dobro+1 do valor correto.

#### A3g. Interp não valida ascription refined — aceita `0 :: NonZero`

**Estado:** `let z := 0 :: NonZero::Int` no interp **passa
silenciosamente** — o interp não executa const-eval de predicados
refined. O JIT rejeita (comptime.jit_failure, embora com mensagem
de erro interno em vez de erro de tipo gracioso). A consequência
direta: `/ 10 z` com z=0::NonZero causa **panic de divisão por
zero** no interp (bigint.rs:317, non-unwinding → SIGABRT).

**Localização:** `crates/kata-interp/src/eval.rs` (sem
const-eval de ascription refined),
`crates/kata-rt/src/bigint.rs:317` (panic divisão por zero).

**Reprodução:**
```
let z := 0 :: NonZero::Int
echo!(/ 10 z)
```
JIT: exit 1 (comptime.jit_failure — erro interno, não type error);
interp: SIGABRT (panic divisão por zero).

**Prioridade:** alta — o interp aceita tipos inválidos que o JIT
rejeita, e o resultado é crash. O JIT também falha: deveria ser
`type.mismatch` gracioso, não `comptime.jit_failure` (erro interno).

#### A4. `loop` infinito sem fuel no interpretador

**Estado:** `loop { ... }` no interp é `loop {}` Rust cru — sem
fuel, timeout, ou yield. O JIT tem `kata_rt_yield_check` no header
do loop (cooperativo + timeout); o interp não tem nada. `loop`
sem `break` trava o processo.

**Localização:** `crates/kata-interp/src/eval.rs` —
`TypedExprKind::Loop { body } => loop { ... }`.

**Prioridade:** alta — mesma classe do bug de recursão sem limitador.

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

**Estado:** `check_neutral_step` só verifica step **literal** em
compile-time. Step variável com valor 0 escapa e produz um range
infinito no runtime (step 0 = próximo == atual → nunca termina).

**Localização:** `crates/kata-inference/src/infer/collections.rs:350`
(`check_neutral_step` — apenas literais).

**Reprodução:** `let s := 0; for x in [1..s..10] echo!(x)` →
loop infinito (imprime `1` repetidamente até timeout/kill).

**Prioridade:** média — typeck rejeita step literal 0, mas não
step dinâmico 0. Runtime não tem guard.

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