# PRD — Capabilities de Tipos: Unificação com refines + Correções de Soundness

**Status:** ✅ Implementado (Fase 1 + Fase 2)
**Data:** 2026-09-06
**Depende de:** `refines` ✅ (PRD-refines), Refinement Propagation ✅ (PRD-refinement-propagation), `RefinesRegistry` ✅, `InterfaceRegistry` ✅, `InlineFnTable` ✅
**Não depende de:** Reorganização arquitetural (documento separado)

## 1. Objetivo

Duas frentes que expandem o que o sistema de tipos faz com refined types,
sem mudar a sintaxe existente nem a representação de tipos (`Ty`):

1. **Unificação com refines via lookup** (Capability nova): type params que
   são interfaces (`Ty::Interface("NUM")`) aceitam args refined que delegam
   aquela interface, resolvendo para o tipo base no binding.
2. **Correções de soundness na fronteira de entrada** (Fase 1): o mecanismo
   `try_refined_precondition` já existe e prova base→refined via Z3 na
   fronteira de entrada, mas tem dois bugs que impedem os casos de uso
   canônicos de funcionar, além de uma fragilidade no translator.

## 2. Motivação

### 2.1. Unificação com refines

Hoje, o ramo `Ty::Interface` de `unify_one` (generics.rs:79-92) binda
qualquer arg sem consultar o `InterfaceRegistry` — `soma :: NUM NUM => NUM`
aceita `PositiveInt` (e até `Text`) no typeck, falhando apenas no codegen
com "Closure sem ffi_symbol". O problema é **permissividade demais**, não
falta de binding: o type param `NUM` binda com `PositiveInt` (tipo refined),
não com `Int` (base canônico), e nenhuma verificação de capability ocorre.

A Fase 2 resolve três coisas: (a) normalizar o binding de refined→base
(`NUM` binda com `Int`, não `PositiveInt`); (b) diagnosticar base que não
implementa a interface ("Text não implementa NUM") antes do codegen; (c)
fazer o tipo de retorno sair concreto (`Int`) sem depender de `expand_ret`.

### 2.2. Bugs na fronteira de entrada

O mecanismo `try_refined_precondition` (apply_dispatch.rs:695-886) prova
via Z3 que um argumento base satisfaz o predicado de um refined esperado
pelo parâmetro, insere ascription implícita e retenta o dispatch. Mas
dois bugs impedem os exemplos canônicos:

**Bug A — `base_match` com refineds polimórficos:** as linhas 745 e 769
exigem `rd.base_ty == typed_args[i].node.ty` (igualdade estrita de `Ty`).
Para `NonEmpty` sobre `List::A`, o `base_ty` é polimórfico (`List::A`),
mas o arg `[1 2 3]` é `List(Int)`. A igualdade falha e o probe rejeita o
caso. Ambos os pontos precisam ser corrigidos.

**Bug B — Gate `is_empty()` ignora `let_bindings`:** o gate
`path_conditions.is_empty()` (linha 705) só conta `facts` e
`learned_facts`, não `let_bindings`. Se não há facts acumulados, o probe
retorna `None` antes de o Z3 ser consultado — mesmo que `let_bindings`
tenham o seeding necessário para provar o predicado via inlining. O
mesmo gate existe em `try_prove_with_path_conditions`
(path_conditions.rs:214) — corrigir um sem o outro não funciona: o probe
passaria pelo gate externo (apply_dispatch.rs:705) e morreria no interno
(path_conditions.rs:214). A correção de `is_empty()` afeta também a
ascription (ascription.rs:341, 421), que passa pelo mesmo gate interno —
mudança desejada (ascription também se beneficia de seeding sem facts).

### 2.3. Translator mapeia por nome, não por origem

O `Z3Translator` mapeia `Closure { Ident("+"), ... }` para `Int::add` no
Z3 pelo **nome** do operador. Se o usuário redefine `+` com um `@ffi`
diferente, o translator mapeia incorretamente. O `ffi_symbol` no TAST
(`TypedExprKind::Closure { ffi_symbol: Option<String> }`) é a prova de
origem — já está disponível, é ignorado.

### 2.4. Translator não trata `TypeAscription`

O `Z3Translator` não tem caso para `TypedExprKind::TypeAscription` —
cai em `_ => None` (opaco). `let a := 5::PositiveInt` produz
`TypeAscription{IntLit(5)}` no TAST; ao semear o let binding,
`translate_int` retorna `None`, e `a` vira variável livre opaca. O
predicado `> r 0` se traduz como `> __opaque_N 0` com conjunção `true`
— UNSAT nunca; a prova falha mesmo com o gate corrigido.

Isso é um pré-requisito da Fase 1: o translator precisa ser translucent
a `TypeAscription` (desembrulhar e traduzir o `expr` interno), ou a
ascription precisa aprender o predicado como `learned_fact` (§7).

## 3. Design

### 3.1. Unificação com refines via lookup

Interfaces em Kata são type parameters. `echo :: SHOW => Unit` funciona
porque `collect_type_params` coleta `Ty::Interface("SHOW")` e `unify_one`
binda para o arg. Hoje o binding é irrestrito — qualquer arg binda sem
checar se implementa a interface. A Fase 2 adiciona normalização e
verificação: quando o arg é um refined type que delega a interface,
resolver para o tipo base antes de bindar; quando o arg não implementa a
interface (direta nem via refined), retornar erro.

```kata
soma :: NUM NUM => NUM
soma (10 :: PositiveInt) (20 :: PositiveInt)
# PositiveInt delega NUM (RefinesRegistry), dispatch_base é Int
# NUM binda com Int (base), não PositiveInt
# Corpo usa + — dispatcha em Int, funciona
# Resultado: Int (tipo base, como hoje)
```

`normalize_refined` consulta delegações diretas apenas. Casos
transitivos (supertraits) caem no `try_refines_fallback` existente, que
já percorre supertraits ao verificar se `func_name` é método de alguma
interface delegada.

### 3.2. Correção do `base_match` (Bug A)

Trocar a igualdade estrita `rd.base_ty == typed_args[i].node.ty` por uma
verificação que aceita refineds polimórficos. Para `NonEmpty` sobre
`List::A`, o `base_ty` é `List::A` (com type param) e o arg é
`List(Int)`. A verificação precisa unificar `A → Int` em vez de exigir
igualdade estrutural.

### 3.3. Gate considera `let_bindings` (Bug B)

O gate `is_empty()` precisa considerar `let_bindings` além de
`facts`/`learned_facts`. Se há `let_bindings` que podem alimentar o
seeding do Z3, o probe deve ser tentado — mesmo sem `facts` explícitos.

O comentário atual em `is_empty()` (path_conditions.rs:133-136) diz:
"bindings são definições, não restrições — sem facts a conjunção seria
`true` e `true ⟹ ¬pred` refutaria qualquer predicado não-tautológico."
Isso é correto para o caso geral (variáveis livres sem premissas), mas
ignora o caso onde o seeding + inlining prova o predicado sem facts
explícitos — o `let_binding` conecta o binding ao corpo da função
inlinable, e o Z3 prova simbolicamente.

### 3.4. Translator mapeia por `ffi_symbol`

O `Z3Translator` mapeia operadores para termos Z3 nativos (`+` →
`Int::add`, `>` → comparação Z3, etc.). Hoje o mapeamento é por nome do
operador. A correção é mapear por `ffi_symbol` — a prova de origem já
presente no TAST.

A tabela de mapeamento cobre os símbolos do runtime (`kata_rt_bi_add` →
`Int::add`, `kata_rt_bi_sub` → `Int::sub`, etc.). FFI arbitrário do
usuário (`@ffi("minha_funcao")`) permanece opaco — cai no caminho
`try_inline` ou `None`.

## 4. Decisões de design

### 4.1. `normalize_refined` consulta delegações diretas apenas

**Escolhido:** delegações diretas. Casos transitivos (supertraits) caem
no `try_refines_fallback`.
**Alternativa rejeitada:** percorrer supertraits em `normalize_refined` —
duplicaria a lógica que já existe no fallback, sem benefício observável
(o tipo de retorno sai igual nos dois caminhos via `expand_ret`).

### 4.2. FFI do runtime é mapeável, FFI do usuário é opaco

**Escolhido:** mapear símbolos conhecidos do runtime (`kata_rt_bi_*`) para
operadores Z3 nativos via `ffi_symbol`. FFI do usuário permanece opaco.
**Alternativa rejeitada:** mapear por nome do operador — frágil, pois o
usuário pode redefinir `+` com `@ffi("outra_coisa")` e o translator
mapeia incorretamente.

### 4.3. Sem constraint explícita (`T implements NUM`)

**Escolhido:** interface é type param, é resolvida no dispatch. A
"constraint" é o próprio dispatch: se o tipo bindado não tem os métodos
que o corpo usa, o dispatch falha com erro natural.
**Alternativa rejeitada:** `T implements NUM` — alias de `NUM` com uma
camada de enforcement que o sistema não precisa.

### 4.4. Diagnóstico de erro é parte da capability

`normalize_refined` consulta o `InterfaceRegistry` para verificar se o
base implementa a interface. Se a resposta é não, retorna erro ali mesmo
("Text não implementa NUM") antes de deixar o dispatch falhar com
mensagem ruim ("`+` não encontrou overload para Text").

## 5. Fases

### Fase 1 — Correções de soundness na fronteira de entrada

**Escopo:** corrigir os dois bugs e a fragilidade do translator que
impedem `try_refined_precondition` de funcionar nos casos canônicos.

1. **Bug A — `base_match` polimórfico:** trocar igualdade estrita por
   unificação em `try_refined_precondition` (apply_dispatch.rs:745 e
   769). Para refineds polimórficos (`NonEmpty` sobre `List::A`), o
   `base_ty` tem type param; o arg é concreto. A verificação precisa
   unificar `A → Int` em vez de exigir `List::A == List(Int)`. Ambos os
   pontos (745 para identificar posições refined, 769 para buscar
   predicados) precisam ser corrigidos.

2. **Bug B — Gate `is_empty()`:** fazer o gate considerar `let_bindings`.
   Se há `let_bindings` não-vazios, o probe deve ser tentado mesmo sem
   `facts`/`learned_facts` — o seeding pode provar via inlining.
   Corrigir nos dois pontos: `try_refined_precondition`
   (apply_dispatch.rs:705) e `try_prove_with_path_conditions`
   (path_conditions.rs:214). A mudança de `is_empty()` afeta a
   ascription (ascription.rs:341, 421) — desejado.

3. **Translator por `ffi_symbol`:** o `Z3Translator` mapeia operadores
   para Z3 nativos. Hoje mapeia por nome; corrigir para mapear por
   `ffi_symbol` (campo já presente em `TypedExprKind::Closure`). Tabela
   fina de símbolos do runtime (`kata_rt_bi_add` → `Int::add`, etc.).

4. **Translator translucent a `TypeAscription`:** adicionar caso para
   `TypedExprKind::TypeAscription` no `Z3Translator` — desembrulhar e
   traduzir o `expr` interno. Sem isto, `let a := 5::PositiveInt`
   produz `TypeAscription{IntLit(5)}` que o translator trata como opaco,
   bloqueando o Exemplo 2 mesmo com o gate corrigido.

**DoD:** os dois exemplos abaixo compilam sem ascription explícita.

**Oráculos:**

```kata
# Exemplo 1 — refined polimórfico
data (List::Int, >= (len _) 1) as NonEmptyList
head :: NonEmptyList::A => A
echo!(head [1 2 3])
# 1 — compila sem ::NonEmptyList

# Exemplo 2 — path condition de operação pura
data (Int, > _ 0) as PositiveInt
f :: PositiveInt => Int
lambda x: x
let a := 5::PositiveInt
let b := 3::PositiveInt
let r := + a b
echo!(f r)
# 8 — compila sem ascription; Z3 prova > r 0 via seeding + inlining
```

### Fase 2 — Unificação com refines via lookup

**Escopo:** passo de normalização em `unify_one` (generics.rs) antes do
binding do type param.

1. **`normalize_refined`:** nova função que consulta `RefinesRegistry`:
   se o arg é um refined que delega a interface `name`, retorna
   `dispatch_base(arg)`. Se não, retorna o arg inalterado. Consulta
   delegações diretas apenas.

2. **Ramo `Ty::Interface` em `unify_one`:** antes de bindar, chamar
   `normalize_refined`. Se o arg normaliza, bindar com o tipo base.

3. **Mudança de assinatura de `unify`:** `unify`/`unify_one` hoje
   recebem apenas `(params, args, type_params, subs)`.
   `normalize_refined` exige `RefinesRegistry` e `InterfaceRegistry`.
   Propagar esses registries (ou `&InferCtx`) como parâmetros.
   ~6 call sites: `apply_dispatch.rs` (×4), `apply_len_tuple.rs`,
   `dot_access.rs` (×3), `collections.rs`. `cargo check` (E0061) guia
   a propagação.

4. **Diagnóstico:** se o arg é refined que delega a interface mas o base
   não implementa a interface, retornar erro claro ("Text não implementa
   NUM") antes de deixar o dispatch falhar.

5. **Codegen:** se `NUM` binda com `Int` mas os `typed_args` continuam
   tipados `PositiveInt`, a monomorfização exige coerção no TAST
   (downcast/ascription dos args) — ou substituição de arg types no
   estilo `try_refines_fallback`. Sem isto, o codegen repete o bug de
   "Closure sem ffi_symbol" que existe hoje.

**DoD:** `soma :: NUM NUM => NUM` aceita `PositiveInt` e retorna `Int`
concreto (sem `expand_ret`).

**Oráculos:**

```kata
data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM

soma :: NUM NUM => NUM
lambda a b: + a b

echo!(soma (10::PositiveInt) (20::PositiveInt))
# 30 — NUM binda com Int, + dispatcha em Int Int => Int

# Erro claro para base que não implementa a interface
# soma "a" "b"  →  erro: Text não implementa NUM
```

## 6. Estruturas afetadas

| Arquivo | Camada | Mudança |
|---|---|---|
| `kata-inference/src/infer/generics.rs` | inference | Novo ramo `Ty::Interface` com `normalize_refined`; mudança de assinatura de `unify`/`unify_one` (Fase 2) |
| `kata-inference/src/infer/apply_dispatch.rs` | inference | Corrigir `base_match` em `try_refined_precondition` linhas 745 e 769 (Fase 1) |
| `kata-inference/src/infer/path_conditions.rs` | inference | Gate `is_empty()` considera `let_bindings`; afeta `try_refined_precondition` e `try_prove_with_path_conditions` (Fase 1) |
| `kata-inference/src/z3_translate.rs` | inference | Mapear por `ffi_symbol` em vez de nome; adicionar caso para `TypeAscription` (translucent) (Fase 1) |
| `kata-inference/src/infer/apply_dispatch.rs` | inference | Call sites de `unify`: propagar registries (Fase 2) |
| `kata-inference/src/infer/apply_len_tuple.rs` | inference | Call site de `unify`: propagar registries (Fase 2) |
| `kata-inference/src/infer/dot_access.rs` | inference | Call sites de `unify`: propagar registries (Fase 2) |
| `kata-inference/src/infer/collections.rs` | inference | Call site de `unify`: propagar registries (Fase 2) |
| `kata-core/src/refines_registry.rs` | core | Sem mudança — `normalize_refined` consulta API existente |
| `kata-core/src/interface_registry.rs` | core | Sem mudança — diagnóstico consulta API existente |
| `kata-parser` | parser | Sem mudança |
| `kata-core/src/ty.rs` | core | Sem mudança — nenhuma variante nova em `Ty` |

## 7. Fora do escopo

- **Reorganização arquitetural de tipos** — documento separado.
- **Ascription aprende predicado como `learned_fact`** — alternativa
  ao item 4 da Fase 1 (translator translucent a `TypeAscription`).
  Se a translucidez do translator for complexa, a ascription pode
  aprender `> a 0` como `learned_fact` após prova bem-sucedida —
  uma linha em `ascription.rs` após `Some(true)`. As duas abordagens
  não são mutuamente exclusivas, mas só uma é necessária para o
  Exemplo 2.
- **`T implements NUM` (constraint explícita)** — rejeitado (§4.3).
- **Mapeamento de FFI arbitrário do usuário para Z3** — permanece opaco.
  Apenas símbolos do runtime são mapeados.

## 8. Riscos

**Custo de compile-time:** o gate ampliado (considera `let_bindings`)
pode disparar mais probes Z3. O `rlimit` (10000) limita cada probe, mas
a frequência pode aumentar. Medir probes por função após a Fase 1.

**Interação com `try_refines_fallback`:** a Fase 2 adiciona um caminho
que rebaixa refined para base (normalização no binding). O fallback
existente já faz isso na falha de dispatch. Os dois caminhos não
conflitam — normalização age no binding do type param (antes do
dispatch falhar), fallback age na falha de match_score (depois). Mas a
regra de supertraits precisa ser clara: normalização olha delegações
diretas, fallback percorre supertraits.

**Translator por `ffi_symbol`:** a tabela de símbolos do runtime é fina
(~12 entradas). Se o runtime ganhar novos símbolos mapeáveis, a tabela
precisa ser estendida. Manutenção baixa — símbolos do runtime raramente
mudam.

**Regression sweep na Fase 2:** hoje qualquer refined (ou tipo não-
implementador) binda contra qualquer interface, falhando apenas no
codegen. A Fase 2 restringe o binding — programas que "funcionavam" até
codegen podem passar a falhar cedo com mensagem diferente. Isso é
desejado, mas exige `cargo test` completo para identificar regressões
intencionais.

**Codegen na Fase 2:** se `NUM` binda com `Int` mas os `typed_args`
continuam tipados `PositiveInt`, a monomorfização exige coerção no TAST.
Sem isto, o codegen repete o bug de "Closure sem ffi_symbol". O item 5
da Fase 2 cobre isso, mas é a parte mais propensa a surpresas.