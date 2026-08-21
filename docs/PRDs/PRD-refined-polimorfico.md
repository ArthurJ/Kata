# PRD: Refinados Polimórficos sobre Interfaces

## Status

**Status:** Pendente
**Data:** 2026-08-20
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

**Nomenclatura interna:** as instâncias são registradas com nomes internos
distintos (`NonZero$Int`, `NonZero$Float`, `NonZero$Rational`) para evitar
colisão no `StructRegistry` (que usa `(origin, struct_name)` como chave). O nome
público `NonZero` mapeia para a família.

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
Quando o caller passa `3::NonZero$Int` (que é `Ty::Struct("NonZero$Int")`):

1. `match_score` testa `arg == param`: `NonZero$Int != NonZero` (nomes distintos)
2. Fallback: testa se `arg` é instância da família `NonZero` — consulta
   `StructInfo.is_instance_of == Some("NonZero")` → sim
3. Trata como exact match (o arg é da família correta)

Isso é análogo ao `try_refines_fallback` atual: `NonZero$Int refines NonZero`
é uma relação que o fallback já sabe seguir, estendida para o caso de família.

### 3.6. Smart constructor polimórfico

`NonZero(x)` despacha por tipo de `x`:
- `NonZero(3)` → `NonZero$Int` (valida `!= 3 (zero 3)` → `!= 3 0` → `True` → `Ok`)
- `NonZero(3.0)` → `NonZero$Float` (valida `!= 3.0 (zero 3.0)` → `!= 3.0 0.0` → `True` → `Ok`)
- `NonZero(0)` → `NonZero$Int` (valida `!= 0 (zero 0)` → `!= 0 0` → `False` → `Err`)

O construtor é um OverloadSet com 3 overloads:
- `NonZero :: Int => Result::(NonZero$Int, Text)`
- `NonZero :: Float => Result::(NonZero$Float, Text)`
- `NonZero :: Rational => Result::(NonZero$Rational, Text)`

O dispatch resolve por tipo do argumento — já funciona hoje.

### 3.7. Ascription

`3::NonZero` despacha o construtor por tipo de `3` (Int) → escolhe `NonZero$Int`.
O tipo resultado é `NonZero$Int`, que é da família `NonZero`. Assinaturas que
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
- `StructRegistry`: registrar instâncias com nomes internos (`Name$Concrete`)
- Predicados sintetizados por instância (já funciona — `constructors_refined.rs`
  usa `base_ty` concreto)
- Testes E2E: `data (NUM, != _ (zero _)) as NonZero` registra 3 instâncias

### Fase 3: Smart constructor polimórfico

- Construtor `NonZero` vira OverloadSet com 3 overloads
- Dispatch por tipo do argumento (já funciona)
- Testes E2E: `NonZero(3)` → Ok, `NonZero(0)` → Err, `NonZero(3.0)` → Ok

### Fase 4: Família no dispatch

- `match_score` / fallback: matchar `NonZero` (família) contra `NonZero$Int` (instância)
- Consultar `is_instance_of` no StructRegistry
- Testes E2E: `/ 10 (3::NonZero)` → 3, `/ 10.0 (3.0::NonZero)` → 3.333...

### Fase 5: Overloads polimórficos

- `/ :: NUM NonZero => NUM` como assinatura única
- Dispatch unifica NUM + NonZero simultaneamente (NUM → Int, NonZero → NonZero$Int)
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

### 6.1. Nomes internos expostos

`NonZero$Int` pode aparecer em mensagens de erro ou output de debug. Mitigação:
display de tipos traduz `NonZero$Int` → `NonZero` quando `is_instance_of` é
`Some("NonZero")`. O `$` é convenção interna, nunca exibida ao usuário.

### 6.2. Ambiguidade de dispatch

`/ :: NUM NonZero => NUM` com args `(Int, NonZero$Float)` — o primeiro arg
resolve NUM → Int, o segundo NonZero → NonZero$Float. O retorno NUM → ?
Qual tipo de retorno? Solução: o tipo de retorno é unificado com o primeiro
arg — se NUM resolve para Int, retorno é Int. Isso é unificação conjunta de
constraints, que o dispatch atual não faz. Pode exigir que a Fase 5 implemente
unificação de type variables junto com interface dispatch.

### 6.3. Compatibilidade

`NonZero` hoje é `data (Int, != _ 0) as NonZero` — concreto sobre Int. A
migração para polimórfico muda o StructInfo de NonZero. Assinaturas existentes
(`/ :: Int NonZero => Int`) continuam funcionando — o dispatch de família
matcha `NonZero$Int` contra `NonZero`. A overload legada pode coexistir durante
a migração e ser removida quando a polimórfica estiver testada.

## 7. Documentação

- Manual §12 (Tipos Refinados): adicionar seção sobre refinados polimórficos
- Manual §4.1 (Dispatch): documentar matching de família → instância
- Book cap.12: exemplo de NonZero polimórfico
- TODO.md: remover item NonZero para Float/Rational (resolvido pelo PRD)