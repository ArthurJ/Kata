# PRD: Refinados Polimórficos sobre Coleções Parametrizadas

## Status

**Implementado.** ✅
**Data:** 2026-09-02
**Depende de:** Refinados polimórficos sobre interfaces (PRD-refined-polimorfico), ascription refined de coleções (commit `37dcd64`)
**Resolve:** A2 (head de lista vazia → SIGABRT), viabiliza `NonEmpty` genérico

**Fases:**
- Fase 1 (pass0 — detecção de base parametrizado): ✅
- Fase 2 (ascription + dispatch): ✅
- Fase 3 (match_score): ✅ (validado, sem alterações necessárias)
- Fase 4 (migrar head/tail no stdlib): ✅
- Fase 5 (codegen + interp): ✅
- Fase 6 (testes + docs): ✅ (2003 passed, 0 failed)

**Limitação pendente:** comptime JIT não suporta `constant x := [1 2 3]::NonEmpty`
(ascription de NonEmpty em compile-time). Ver TODO A2 para detalhes.

## 1. Objetivo

Permitir que tipos refinados sejam declarados sobre coleções parametrizadas
(`List::A`, `Array::A`, `Set::A`, `Dict::K V`) com um type parameter livre,
gerando instâncias concretas por tipo de elemento sob demanda (lazy).

```
data (List::A, >= (len _) 1) as NonEmpty
head :: NonEmpty::A => A
```

`NonEmpty::A` é a instância paramétrica da família — `A` é type variable livre
na assinatura, unificada no call-site com o tipo concreto do argumento.
`head` rejeita `[]` em compile-time e aceita `[1 2 3]` sem downcast manual.

## 2. Motivação

### 2.1. Bug A2 — SIGABRT em programa válido

`head :: List::A => A` promete retornar `A`, mas `head []` retorna 0 (null).
`echo!(head ([] :: [Int]))` chama `deref_bigint(0)` → panic non-unwinding em
`extern "C"` → SIGABRT. O typeck endossa uma assinatura que a runtime não honra.

### 2.2. Infraestrutura existe mas não se aplica a coleções

O commit `37dcd64` adicionou ascription refined sobre coleções — `data ([Int], >= (len _) 1) as NonEmptyList` funciona. Mas `NonEmptyList` é concreto: só
cobre `List::Int`. `head` é polimórfico em `A`; exigir um refined por tipo de
elemento (`NonEmptyList::Int`, `NonEmptyList::Text`, `NonEmptyList::Pessoa`)
não escala.

### 2.3. Famílias polimórficas sobre interfaces não cobrem coleções

`NonZero` funciona porque `NUM` é interface: `interface_registry.implementors_of("NUM")` descobre `[Int, Float, Rational]`, e o pass0
instancia `NonZero::Int`, `NonZero::Float`, `NonZero::Rational` eagerly.

`List::A` não é interface. `A` é type parameter — pode ser qualquer tipo,
incluindo tipos de usuário (`Pessoa`, `Encoding`). Não há registry que
enumere todos os tipos possíveis para `A`. Instanciação eager é impossível.

## 3. Design

### 3.1. Sintaxe

```kata
data (List::A, >= (len _) 1) as NonEmpty
data (Array::A, >= (len _) 1) as NonEmptyArray
data (Text, >= (len _) 1) as NonEmptyText
data (Dict::K V, >= (len _) 0) as NonEmptyDict
```

- O type parameter (`A`, `K`, `V`) é livre — não precisa ser declarado.
- O predicado referencia funções disponíveis no tipo base (`len` via `COUNTABLE`).
- Ascription: `[1 2 3]::NonEmpty` valida o predicado em compile-time sobre
  literais. `NonEmpty(expr)` é construtor falível para não-literais.

### 3.2. Instanciação lazy

Diferente de `NonZero` (eager, via `implementors_of`), `NonEmpty` sobre
`List::A` é instanciado **sob demanda** quando um call-site resolve o tipo
concreto de `A`.

Fluxo:

1. **pass0**: `data (List::A, ...) as NonEmpty` — base é `Ty::List(Ty::Var("A"))`.
   Reconhece type var livre no base → registra como **família polimórfica
   parametrizada** (não concreto). Registra `StructKey::Family("NonEmpty")`
   no type_env. NÃO instancia para tipos concretos.
2. **Call-site** (`head ([1 2 3]::NonEmpty)`): ascription resolve
   `Family("NonEmpty")`. O typeck infere `A = Int` a partir do literal
   `[1 2 3]`. Cria `Instance("NonEmpty", "Int")` on-demand no StructRegistry
   (se ainda não existe), com `alias_of = "List"`.
3. **Dispatch**: `match_score` casa `Instance("NonEmpty", "Int")` com o
   param `NonEmpty` da assinatura de `head`.

### 3.3. Por que lazy e não genérico

A alternativa é manter `Instance("NonEmpty", "A")` com `A` como type param
genérico, unificado no call-site. Mas `A` pode ser um tipo de usuário
(`Pessoa`, `Encoding`) — não apenas primitivos. O `StructKey::Instance` atual
guarda `concrete_type: String` (um nome de tipo). Tipos de usuário são
`StructKey::Plain("Pessoa")`, que não cabe no formato `Instance("NonEmpty", "Pessoa")` sem ambiguidade com `Instance("NonEmpty", "Int")` onde "Int" é
primitivo.

Instanciação lazy resolve isso: cada call-site cria a instância para o tipo
concreto específico, e o `match_score` casa por identidade estrutural do
tipo concreto — o mesmo mecanismo que já funciona para `NonZero::Int`.

### 3.4. pass0 — detecção de base parametrizado

Hoje o pass0 tem dois caminhos:

```
match base_ty {
    Ty::Interface(iface_name) => família polimórfica (eager, implementors_of),
    _ => refined concreto,
}
```

Novo caso intermediário:

```
match base_ty {
    Ty::Interface(iface_name) => família polimórfica eager (igual hoje),
    Ty::List(Ty::Var(_)) | Ty::Array(Ty::Var(_)) | Ty::Set(Ty::Var(_))
    | Ty::Dict(Ty::Var(_), _) | Ty::Dict(_, Ty::Var(_)) => família polimórfica lazy,
    _ => refined concreto,
}
```

No caminho lazy:

- Registrar `StructKey::Family("NonEmpty")` no type_env (igual eager).
- NÃO chamar `register_refined_instance` — as instâncias serão criadas on-demand.
- Guardar `RefinedDeclInfo` com `base_ty = Ty::List(Ty::Var("A"))` para o
  inference saber que é uma família parametrizada e qual o type param.
- Guardar o nome do type parameter (`"A"`) para unificação no call-site.
- `RefinesEntry` não é registrado (famílias lazy não usam `refines` — o
  dispatch casa por `Family`/`Instance`, não por fallback de interface).

### 3.4.1. resolve_type_expr — NonEmpty::A em assinatura

Hoje `resolve_type_expr` resolve `NonZero::Int` (ParamApp com param concreto)
para `Instance("NonZero", "Int")`. Mas `NonEmpty::A` (ParamApp com type var)
cai no caso `_ => ""` — não resolve para Instance.

Para `head :: NonEmpty::A => A` funcionar, `resolve_type_expr` precisa de
um novo caminho: quando o param é `Ty::Var` e a família é lazy, produzir
uma instância paramétrica — `Instance("NonEmpty", "A")` onde `"A"` é
type variable livre (não tipo concreto). O `match_score` e o dispatch
unificam `A` no call-site.

Alternativamente, `Family("NonEmpty")` na assinatura já é compatível com
qualquer `Instance("NonEmpty", X)` pelo `match_score` existente (linha
546-548). Nesse caso, `NonEmpty::A` na assinatura resolveria para
`Family("NonEmpty")` (desprezando o param), e `A` seria coletado como
type param da função pelo `collect_type_params` normal. O type param `A`
no retorno resolve por unificação genérica padrão (igual `List::A => A`).

### 3.5. Ascription — resolução de Family com type param

Quando `expr::NonEmpty` aparece e `NonEmpty` é família lazy:

1. Inferir o tipo de `expr` (ex: `[1 2 3]` → `Ty::List(Ty::Prim(PrimTy::Int))`).
2. Unificar o type param `A` com o tipo de elemento: `A = Int`.
3. Criar `Instance("NonEmpty", "Int")` no StructRegistry se não existe
   (lazy registration).
4. Const-avaliar o predicado `>= (len _) 1` sobre o literal.
5. Se passa: o tipo do resultado é `Instance("NonEmpty", "Int")`.
6. Se falha: `type.mismatch` gracioso.

Para não-literais: o construtor falível `NonEmpty(expr)` retorna
`Result::(Instance("NonEmpty", T), Text)` após avaliar o predicado em runtime.

### 3.6. match_score — casar Instance com Family

Hoje `match_score` já casa `Instance("NonZero", "Int")` com
`Family("NonZero")` (linha 546-548 de dispatch.rs). O mesmo caminho serve
para `NonEmpty`: `Instance("NonEmpty", "Int")` casa com `Family("NonEmpty")`.

Mas `head` teria assinatura `head :: NonEmpty::A => A` (onde `NonEmpty::A`
   resolve para a instância paramétrica da família, com `A` livre). O type
   param `A` é resolvido por unificação com o tipo concreto da instância no
   call-site — não por `match_score`.

A unificação de `A` acontece na ascription (passo 3.5 passo 2). Quando o
arg chega ao dispatch, já é `Instance("NonEmpty", "Int")`, e `A` já é `Int`.

### 3.7. head — mudança de assinatura

```kata
# Antes:
@ffi("kata_rt_list_head")
head :: List::A => A

# Depois:
data (List::A, >= (len _) 1) as NonEmpty

@ffi("kata_rt_list_head")
head :: NonEmpty::A => A
```

- `NonEmpty::A` na assinatura resolve para a instância paramétrica da
  família, com `A` como type variable livre.
- No call-site `head ([1 2 3]::NonEmpty)`, o arg é `Instance("NonEmpty", "Int")`,
  `A` unifica com `Int`, retorno é `Int`.
- `head []` não compila: `[]` não satisfaz `>= (len _) 1` na ascription,
  e sem ascription `[]` é `List::A` (não `NonEmpty`) → `type.no_overload`.
- O símbolo FFI `kata_rt_list_head` não muda — o typeck garante que o ponteiro
  é não-null.

### 3.8. tail e outras funções

`tail :: NonEmpty::A => List::A` — tail de NonEmpty pode retornar lista vazia
(uma lista com 1 elemento tem tail = `[]`). O retorno é `List::A`, não `NonEmpty`.

`cons :: A List::A => NonEmpty` — cons sempre produz lista não-vazia. O
construtor pode ser tipado para retornar `NonEmpty` diretamente (o predicado
`>= (len _) 1` é trivialmente verdadeiro para `cons x xs`).

## 4. Decisões de design

### 4.1. Lazy vs genérico (type param na instância)

**Escolha:** instanciação lazy (criar `Instance("NonEmpty", "Int")` on-demand).

**Alternativa rejeitada:** manter `Instance("NonEmpty", "A")` com `A` como
type param genérico. Motivo: `A` pode ser tipo de usuário (`Pessoa`), e
`StructKey::Instance` guarda `concrete_type: String` — um nome. Tipos de
usuário são `StructKey::Plain("Pessoa")`, não um string simples. A
instância genérica exigiria que `Instance` carregasse um `Ty` completo em
vez de um nome, mudando o `StructKey` e todo o dispatch. Lazy reutiliza
a maquinária existente sem mudar `StructKey`.

### 4.2. Lazy vs eager

**Escolha:** lazy (instanciar no call-site).

**Alternativa rejeitada:** instanciar para todos os tipos conhecidos no pass0.
Motivo: não existe registry de "todos os tipos que `A` pode assumir". Tipos
de usuário são declarados em módulos que podem não ter sido processados ainda
no pass0. Eager exigiria um pass adicional após todos os módulos, e mesmo
assim não capturaria tipos de usuário em outros módulos.

### 4.3. head :: NonEmpty::A => A vs head :: List::A => Result::(A, Text)

**Escolha:** `head :: NonEmpty::A => A` (refined na entrada, não Result no retorno).

**Alternativa rejeitada:** mudar `head` para retornar `Result::(A, Text)`.
Motivo: muda a API de toda função que usa `head` (cada call-site precisa de
match ou `?`). Refined na entrada preserva o tipo de retorno `A` e elimina
o erro em compile-time. `div` seguiu o mesmo padrão: `/ :: Self NonZero => Self`
(refined na entrada) em vez de retornar `Result`.

## 5. Fases

### Fase 1 — pass0: detectar base parametrizado e registrar família lazy

**Escopo:** `crates/kata-resolution/src/pass0.rs`

- Novo caso no match de `base_ty`: `Ty::List(Ty::Var(_))`, `Ty::Array(Ty::Var(_))`,
  `Ty::Set(Ty::Var(_))` → registrar como `Family` no type_env, NÃO instanciar.
- Guardar `RefinedDeclInfo` com `base_ty` preservando o type param.
- Guardar nome do type param para unificação posterior.
- `RefinesEntry` não é registrado (famílias lazy não usam `refines` — o
  dispatch casa por `Family`/`Instance`, não por fallback de interface).

**DoD:** `data (List::A, >= (len _) 1) as NonEmpty` compila e registra
`NonEmpty` como família. `NonEmpty` aparece no type_env como `Family`.

### Fase 2 — Ascription: resolver Family lazy com tipo concreto

**Escopo:** `crates/kata-inference/src/infer/ascription.rs`

- Quando `expr::NonEmpty` e `NonEmpty` é família lazy sobre `List::A`:
  - Inferir tipo de `expr` → `Ty::List(elem_ty)`.
  - Unificar type param `A` com `elem_ty`.
  - Criar `Instance("NonEmpty", elem_ty_name)` no StructRegistry (lazy).
  - Const-avaliar predicado sobre literal (se aplicável).
  - Retornar `Instance("NonEmpty", elem_ty_name)` como tipo.

**DoD:** `[1 2 3]::NonEmpty` resolve para `Instance("NonEmpty", "Int")` e
passa. `[]::NonEmpty` falha com `type.mismatch` gracoso.

### Fase 3 — Dispatch: casar Instance lazy com assinatura

**Escopo:** `crates/kata-core/src/dispatch.rs` (match_score),
`crates/kata-inference/src/infer/apply_dispatch.rs`

- Verificar que `match_score` já casa `Instance("NonEmpty", "Int")` com
  `Family("NonEmpty")` (deve funcionar pelo caminho existente).
- Se necessário, adicionar caso para `Instance` lazy (alias_of = "List"
  em vez de "Int") — o `match_score` atual verifica `arg_key.name() == param_key.name()`, que já cobre pelo nome da família.

**DoD:** `head_ne :: NonEmpty::A => A` (FFI de teste) despacha
`head_ne ([1 2 3]::NonEmpty)` e retorna o primeiro elemento.

### Fase 4 — Migrar head e tail no stdlib

**Escopo:** `stdlib/core.kata`

- Declarar `data (List::A, >= (len _) 1) as NonEmpty` no stdlib.
- Mudar `head :: List::A => A` para `head :: NonEmpty::A => A`.
- `tail :: NonEmpty::A => List::A` (tail pode retornar vazia).
- Migrar exemplos e testes que usam `head`.

**DoD:** `echo!(head ([1 2 3]::NonEmpty))` → `1`. `echo!(head [])` →
`type.no_overload` em compile-time (não SIGABRT).

### Fase 5 — Codegen/interp: verificar passagem

**Escopo:** `crates/kata-codegen`, `crates/kata-interp`

- O codegen despacha `head` para `kata_rt_list_head` — o arg é o ponteiro
  bruto da lista, e o typeck garante que é NonEmpty (não-null). O FFI
  não muda.
- Verificar que o interp faz o mesmo (sem double-tagging, sem
  wrapper extra).

**DoD:** `head ([1 2 3]::NonEmpty)` retorna `1` em ambos backends.

## 6. Estruturas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| resolution | `pass0.rs` | Novo caso: base parametrizado → família lazy |
| inference | `infer/ascription.rs` | Resolver Family lazy: unificar type param, criar Instance on-demand |
| core | `dispatch.rs` | Verificar match_score (provavelmente sem mudança) |
| inference | `infer/apply_dispatch.rs` | Verificar dispatch (provavelmente sem mudança) |
| stdlib | `core.kata` | Declarar `NonEmpty`, mudar `head`/`tail` |
| codegen | (nenhuma mudança esperada) | FFI inalterado |
| interp | (nenhuma mudança esperada) | FFI inalterado |

## 7. Fora do escopo

- **show de NonEmpty** — bug A3c (show de `Optional::None` → ffi_not_found)
  é a mesma classe de problema (monomorphização não instancia show para tipo
  não-concreto). PRD próprio.
- **len em Range** — bug A3d. PRD próprio.
- **Outras coleções** (Array, Set, Dict) — o design suporta, mas a
  implementação inicial foca em `List::A`. Array/Set/Dict seguem o mesmo
  padrão e podem ser adicionadas após validação.
- **cons retornando NonEmpty** — `cons x xs` é trivialmente NonEmpty.
  Tipar `cons :: A List::A => NonEmpty` é uma melhoria de precisão, mas
  pode ser feita depois de `head`.
- **Refinement propagation** — aprender que `head (cons x xs)` retorna
  NonEmpty. PRD-refinement-propagation cobre isso.

## 8. Riscos

### 8.1. Instância lazy não é deduplicada

Se múltiplos call-sites criam `Instance("NonEmpty", "Int")`, o StructRegistry
deve deduplicar (insert é idempotente pela chave). Verificar que
`register_refined_instance` não duplica.

### 8.2. Type param de usuário

`Instance("NonEmpty", "Pessoa")` — `Pessoa` é `StructKey::Plain`. O
`alias_of` da instância seria "List" (não "Pessoa"). O `match_score` casa
pelo nome da família, não pelo alias_of. Verificar que o codegen/interp
não dependem de `alias_of` para despachar o FFI.

### 8.3. Quebra de retrocompatibilidade

Mudar `head :: List::A => A` para `head :: NonEmpty::A => A` quebra todo código
que chama `head` sem ascription. Mitigação: o erro é `type.no_overload` em
compile-time (gracioso, não crash). O usuário precisa adicionar `::NonEmpty`
ou usar `match`/pattern para verificar não-vazio antes.

## 9. Documentação

Ao concluir:
- `docs/TODO.md` — marcar A2 como resolvido.
- `docs/base/sintaxe-mapa.md` — adicionar `NonEmpty` e refined sobre coleções
  parametrizadas na seção de famílias polimórficas.
- `stdlib/core.kata` — atualizar comentário de `head`/`tail`.
- `examples/refined_collections.kata` — adicionar exemplo polimórfico.