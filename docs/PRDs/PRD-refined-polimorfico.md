# PRD: Refinados Polimórficos sobre Interfaces

## Status

**Status:** Fases 1-7 COMPLETAS e testadas (1751 passed, 0 failed, 1 ignored).
`StructKey::Family` + `Instance` implementadas, `resolve_type_expr` family-aware,
`Self` em interface NUM, two-pass no pass0 (0a registra interfaces/impls,
0b resolve assinaturas), `instantiate_family_for_concrete` resolve
`Family→Instance` no implements, `expand_family_signatures` só expande
Signatures FFI (FunctionDefs mantêm `Family` para dispatch no call-site).
Stdlib migrada: `/` e `mod` exigem `NonZero`, `div` retorna `Result`,
FFI unchecked em `stdlib/core_internals.kata` (`bi_div`/`f_div`/`rat_div`).
Exemplos e testes migrados. `examples/legacy/` excluído do snapshot test.
**Data:** 2026-08-22
**Depende de:** Sistema de refinados atual (tipos refinados concretos), dispatch por Score 2D + fallback `refines`
**Não depende de:** Nenhum PRD pendente

## 1. Objetivo

Permitir que tipos refinados sejam declarados sobre interfaces em vez de tipos
concretos. `data (NUM, != _ (zero _)) as NonZero` define NonZero uma única vez,
aplicável a Int, Float e Rational — sem repetição de declarações por tipo base.

## 2. Motivação

### 2.1. Repetição desnecessária

Hoje, `NonZero` só existe para Int: `data (Int, != _ 0) as NonZero`. Estender
para Float e Rational exige duas declarações idênticas mudando só o tipo base
e o literal de zero:

```
data (Float, != _ 0.0, = _ _) as NonZeroFloat
data (Rational, != _ (rational 0)) as NonZeroRational
```

Cada novo refined sobre NUM (Positive, NonNegative, Between...) multiplicaria
por 3. Isso não escala.

### 2.2. Conceito único, três declarações

"Diferente de zero" significa a mesma coisa para Int, Float e Rational. O type
system deveria expressar o conceito uma vez. Assinaturas como
`/ :: NUM NonZero => NUM` são mais expressivas que 3 overloads dizendo a mesma
coisa.

### 2.3. `zero` pertence a NUM

A identidade aditiva é uma propriedade de todo grupo. `zero :: Self => Self`
enriquece NUM com uma operação que pertence à interface — independente de
refinados polimórficos.

## 3. Design

### 3.1. `zero` na interface NUM

Adicionar `zero :: Self => Self` à interface NUM:

```
interface NUM
    + :: Self Self => Self @associative(0) @commutative
    - :: Self Self => Self
    * :: Self Self => Self @associative(1) @commutative
    div :: Self Self => Result::(Self, Text)
    zero :: Self => Self
    abs :: Self => Self
    = :: Self Self => Boolean @commutative
    != :: Self Self => Boolean @commutative
```

Implementações:

- `Int implements NUM`: `zero :: Int => Int @ffi("kata_rt_bi_zero")` → retorna 0
- `Float implements NUM`: `zero :: Float => Float @ffi("kata_rt_fzero")` → retorna 0.0
- `Rational implements NUM`: `zero :: Rational => Rational @ffi("kata_rt_rat_zero")` → retorna rational 0

Alternativa sem FFI: `zero` como função pura com `lambda _: 0` (Int), mas isso
não funciona para Float/Rational sem literal polimórfico. FFI direto é mais
simples — cada implementação retorna a constante correta.

### 3.2. Declaração refinada sobre interface

```
data (NUM, != _ (zero _)) as NonZero
```

O parser já aceita `TypeExpr::Named("NUM")` como `base_ty`. O resolver
(`pass0.rs`) já chama `resolve_type_expr` que produz `Ty::Interface("NUM")`.
A mudança está em como o sistema reage a `base_ty = Ty::Interface(...)`.

### 3.3. Expansão na declaração

Quando `pass0.rs` encontra `base_ty = Ty::Interface("NUM")`:

1. Consulta `InterfaceRegistry`: quais tipos implementam NUM? → `[Int, Float, Rational]`
2. Para cada tipo concreto `T`, gera um `RefinedDeclInfo`:
   - `name`: `NonZero` (nome público — todos compartilham)
   - `base_ty`: `T` (concreto)
   - `predicates`: o mesmo predicado (`!= _ (zero _)`)
3. Registra no `StructRegistry` um `StructInfo` por instância, com:
   - `alias_of`: nome do tipo concreto (ex: `"Int"`, `"Float"`, `"Rational"`)
   - `predicates`: nomes das funções predicado específicas da instância
   - **Novo campo `is_instance_of: Option<String>`**: `Some("NonZero")` indicando
     que este StructInfo é instância de uma família

### 3.3.1. `StructKey` — identidade estrutural, não string-encoded

O `StructRegistry` usa `(origin, struct_name)` como chave. Hoje `struct_name`
é `String`. Com famílias, três instâncias chamadas `NonZero` na mesma origin
colidem. Em vez de resolver com nomes internos (`NonZero$Int`), introduzir um
tipo estrutural como chave:

```rust
enum StructKey {
    /// Tipo comum: "Pessoa", "Float", "NonZero"
    Plain(String),
    /// Família polimórfica: "NonZero" = data (NUM, ...) as NonZero
    Family(String),
    /// Instância de família: ("NonZero", "Int")
    Instance(String, String),
}
```

O `StructRegistry` passa a ser `HashMap<(String, StructKey), StructInfo>`.

**`Family` vs `Plain`:** `Plain("NonZero")` é ambíguo — serve para struct
concreto e família. `Family("NonZero")` carrega a semântica: "referência a
família, expandir em instâncias concretas". `resolve_type_expr` produz
`Family` quando o `struct_registry.is_family(name)` é true. O `type_env.define`
para famílias polimórficas também registra `Family` (não `Plain`).

**Por que não `String` com `$`:**
- Colisão: nada impede o usuário de declarar `data (...) as NonZero$Int`
- Parsing implícito: cada site que precisa distinguir família vs concreto faria
  `name.contains('$')` — string matching onde deveria ser pattern matching
- Display acoplado à identidade: traduzir `NonZero$Int` → `NonZero` é
  transformação de string frágil

**`Ty::Struct` carrega `String` ou `StructKey`?**

Duas opções:
- **Normalização na fronteira:** `Ty::Struct` continua `String`. O `StructRegistry`
  expõe `fn lookup(&self, origin: &str, name: &str, type_hint: Option<&Ty>)`.
  Quando `name` é uma família, `type_hint` seleciona a instância. Confina a
  mudança ao `StructRegistry` — menor invasão.
- **`StructKey` em `Ty`:** `Ty::Struct(StructKey)` — a distinção família vs
  concreto é estrutural em todo o type system. Mais limpo, mas `Ty::Struct(String)`
  aparece em ~todos os crates — mudança grande.

**Recomendação:** normalização na fronteira (opção 1) para minimizar invasão.
O `StructKey` existe internamente no `StructRegistry`; `Ty::Struct(String)`
continua carregando o nome público `"NonZero"`. Quando o codegen ou inference
precisa do `StructInfo` concreto, consulta o registry com type hint — o
registry resolve `"NonZero"` + hint `Int` → `StructKey::Instance("NonZero", "Int")`.

Quando não há type hint (ex: `fn :: NonZero => Int` sem saber o tipo do arg),
o registry retorna a família — um `StructInfo` virtual que lista as instâncias.
O dispatch então testa cada instância (ver §3.5).

### 3.4. Predicado com `zero`

O predicado `!= _ (zero _)` é sintetizado como `__pred_NonZero_0 :: T => Boolean`
para cada instância `T`. O `zero _` dentro do predicado despacha para a
sobrecarga correta via NUM — `zero x` onde `x :: Int` chama `Int implements NUM`,
onde `x :: Float` chama `Float implements NUM`. O dispatch já resolve isso.

Resultado: `!= x (zero x)` recebe dois args do mesmo tipo `T` — sem widening
implícito, sem literal polimórfico.

### 3.5. NonZero como família no dispatch

`NonZero` é o nome público. Assinaturas usam `NonZero` diretamente:

```
/ :: NUM NonZero => NUM
```

O dispatch precisa matchar `NonZero` (família) contra um argumento concreto.
Quando o caller passa `3::NonZero` com tipo inferido `Int`, o ascription
resolve `NonZero` + hint `Int` → `StructKey::Instance("NonZero", "Int")`. O
tipo do argumento é `Ty::Struct("NonZero")` com `is_instance_of = Some("NonZero")`
e `alias_of = "Int"`.

Matching no dispatch:

1. `match_score` testa `arg == param`: o arg é `Ty::Struct("NonZero")` (nome
   público da instância), o param é `Ty::Struct("NonZero")` — match direto
   se ambos usam o nome público
2. Se o arg é uma instância e o param é a família (ou vice-versa), o fallback
   consulta `StructInfo.is_instance_of` — se `Some("NonZero")` == nome da
   família do param, trata como exact match

Isso é análogo ao `try_refines_fallback` atual: a relação "instância pertence
à família" é seguida pelo fallback, como `refines` já é.

### 3.6. Smart constructor polimórfico

`NonZero(x)` despacha por tipo de `x`:
- `NonZero(3)` → instância `("NonZero", "Int")` (valida `!= 3 (zero 3)` → `!= 3 0` → `True` → `Ok`)
- `NonZero(3.0)` → instância `("NonZero", "Float")` (valida `!= 3.0 (zero 3.0)` → `!= 3.0 0.0` → `True` → `Ok`)
- `NonZero(0)` → instância `("NonZero", "Int")` (valida `!= 0 (zero 0)` → `!= 0 0` → `False` → `Err`)

O construtor é um OverloadSet com 3 overloads, uma por instância concreta:
- `NonZero :: Int => Result::(NonZero, Text)` (instância Int)
- `NonZero :: Float => Result::(NonZero, Text)` (instância Float)
- `NonZero :: Rational => Result::(NonZero, Text)` (instância Rational)

O tipo de retorno `NonZero` é o nome público — o dispatch sabe que é uma
família e resolve a instância concreta pelo tipo do argumento. O dispatch
resolve por tipo do argumento — já funciona hoje.

### 3.7. Ascription

`3::NonZero` despacha o construtor por tipo de `3` (Int) → seleciona a
instância `("NonZero", "Int")`. O tipo resultado é `Ty::Struct("NonZero")`
com `is_instance_of = Some("NonZero")` e `alias_of = "Int"`. Assinaturas que
pedem `NonZero` aceitam via fallback de família.

## 4. Fases

### Fase 1: `zero` em NUM

- Adicionar `zero :: Self => Self` à interface NUM
- Implementar em Int, Float, Rational (FFI ou lambda)
- Testes E2E: `zero 3` → 0, `zero 3.0` → 0.0, `zero (rational 5)` → rational 0
- Verificar: `zero` despacha corretamente via NUM

### Fase 2: Expansão de refined sobre interface

- `pass0.rs`: quando `base_ty` é `Ty::Interface`, expande em instâncias concretas
- `StructInfo`: adicionar campo `is_instance_of: Option<String>`
- `StructRegistry`: introduzir `StructKey` (enum `Plain` / `Instance`) como
  chave interna. Instâncias registradas como `StructKey::Instance("NonZero", "Int")`
- `StructRegistry::lookup`: aceitar type hint para resolver família → instância
- `Ty::Struct` continua carregando `String` (nome público). Normalização
  na fronteira do registry
- Predicados sintetizados por instância (já funciona — `constructors_refined.rs`
  usa `base_ty` concreto)
- Testes E2E: `data (NUM, != _ (zero _)) as NonZero` registra 3 instâncias
  acessíveis via lookup com type hint

### Fase 3: Smart constructor polimórfico

- Construtor `NonZero` vira OverloadSet com 3 overloads
- Dispatch por tipo do argumento (já funciona)
- Testes E2E: `NonZero(3)` → Ok, `NonZero(0)` → Err, `NonZero(3.0)` → Ok

### Fase 4: Família no dispatch

- `match_score` / fallback: matchar `NonZero` (família) contra instância concreta
- Consultar `is_instance_of` no StructRegistry — se o arg é instância da família
  do param, trata como exact match
- Testes E2E: `/ 10 (3::NonZero)` → 3, `/ 10.0 (3.0::NonZero)` → 3.333...

### Fase 5: Overloads polimórficos com `Self` + two-pass — COMPLETA

**Estado:** Completa e testada. `Self` substituído por tipo concreto via
`substitute_self`. Two-pass no pass0 quebra o ciclo interface→NonZero→interface
(0a registra interfaces/impls com assinaturas vazias, 0b resolve após todas
as declarações). `instantiate_family_for_concrete` resolve `Family→Instance`
no contexto do implements.

- Migrar operações aritméticas de NUM para `Self`:
  `+ :: Self Self => Self`, `-`, `*`, `div`, `abs` (substituir `NUM NUM => NUM`)
- Adicionar métodos que usam NonZero como família:
  `/ :: Self NonZero => Self`, `mod :: Self NonZero => Self`
- **Two-pass no pass0:**
  - Passo 1: registrar declarações de dados (interfaces, data, enum) — popula
    `struct_registry`, `type_env`, `interface_registry` com assinaturas
  - Passo 2: resolver assinaturas de métodos de interface e implements — agora
    NonZero é `Family` no `type_env`, `Self` é substituído por tipo concreto
- **Expansão só de Signatures (não FunctionDefs):** funções com corpo Kata
  mantêm `Family("NonZero")` no FunctionDef. O `try_refines_fallback` resolve
  `Family → Instance → concreto` no call site. Funções FFI (sem corpo) são
  expandidas em `Instance` concretas normalmente.
- Dispatch unifica `Self` + `NonZero` simultaneamente (Self → Int,
  NonZero → Instance("NonZero","Int"))
- Testes E2E: `/ 10 (3::NonZero)` e `/ 10.0 (3.0::NonZero)` despacham para a
  mesma assinatura, resolvida por tipo dos args
- Migrar `/ :: Int NonZero => Int` legada para a polimórfica
- Remover `/ :: Int Int => Int` legada (pânica em zero) — agora NonZero é universal

### Fase 6: NonZeroFloat com NaN

- Para Float, adicionar segundo predicado: `data (NUM, != _ (zero _), = _ _) as NonZero`
  - `= _ _` vira `= x x` — `false` para NaN em IEEE 754, rejeitando-o
  - Ou: `zero` para Float poderia retornar NaN? Não — zero é 0.0, NaN é outro
    conceito. O predicado `= _ _` é o correto para rejeitar NaN.
- Testes E2E: `NonZero(0.0)` → Err, `NonZero(NaN)` → Err (via `= NaN NaN` → False)

## 5. Escopo e não-escopo

### No escopo
- `zero :: Self => Self` em NUM
- `data (Interface, predicado) as Nome` — expansão + instâncias
- Smart constructor polimórfico (OverloadSet)
- Dispatch de família (fallback estendido)
- Assinaturas polimórficas (`/ :: NUM NonZero => NUM`)
- NonZero para Float com rejeição de NaN

### Fora de escopo
- Literais numéricos polimórficos (`0` que vira `0.0` por contexto) — não
  necessário com `zero` em NUM
- Refinados sobre interfaces não-NUM (ORD, SHOW, EQ) — mesmo mecanismo, mas
  sem caso de uso imediato
- Tipos polimórficos em assinaturas genéricas (`fn :: A NonZero => A` onde A
  não é interface) — exigeria type classes, fora do escopo
- Monomorfização de refined families — o dispatch resolve dinamicamente, não
  precisa gerar versões monomórficas

## 6. Riscos

### 6.1. Identidade de instâncias

Instâncias de família são identificadas por `StructKey::Instance("NonZero", "Int")`
internamente. O nome público `"NonZero"` é o que aparece em `Ty::Struct` e no
display. Mensagens de erro que precisam distinguir instâncias (ex: "NonZero Int
falhou predicado") usam `is_instance_of` + `alias_of` do `StructInfo` — nunca
parsing de string. O display de tipos sempre mostra o nome público `NonZero`;
quando necessário distinguir, mostra `NonZero (Int)` ou similar — uma decisão de
UI, não de identidade.

### 6.2. Ambiguidade de dispatch

`/ :: NUM NonZero => NUM` com args `(Int, 3.0::NonZero)` — o primeiro arg
resolve NUM → Int, o segundo NonZero → instância Float. O retorno NUM → ?
Qual tipo de retorno? Solução: o tipo de retorno é unificado com o primeiro
arg — se NUM resolve para Int, retorno é Int. Isso é unificação conjunta de
constraints, que o dispatch atual não faz. Pode exigir que a Fase 5 implemente
unificação de type variables junto com interface dispatch.

### 6.3. Compatibilidade

`NonZero` hoje é `data (Int, != _ 0) as NonZero` — concreto sobre Int. A
migração para polimórfico muda o StructInfo de NonZero. Assinaturas existentes
(`/ :: Int NonZero => Int`) continuam funcionando — o dispatch de família
matcha a instância Int contra a família NonZero. A overload legada pode
coexistir durante a migração e ser removida quando a polimórfica estiver testada.

## 7. Documentação

- Manual §12 (Tipos Refinados): adicionar seção sobre refinados polimórficos
- Manual §4.1 (Dispatch): documentar matching de família → instância
- Book cap.12: exemplo de NonZero polimórfico
- TODO.md: remover item NonZero para Float/Rational (resolvido pelo PRD)