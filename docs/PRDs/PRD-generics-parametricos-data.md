# PRD — Generics Paramétricos para `data`

**Status:** 🔴 Pendente
**Data:** 2026-09-21

## Objetivo

Permitir que tipos `data` sejam parametrizados por type params, eliminando a
necessidade de uma declaração monomórfica separada para cada combinação de
tipos de campo. `data Complex (re::T im::T) where T implements SCALAR` declara
um único tipo paramétrico; o monomorphizer instancia métodos on-demand para
cada combinação de type args usada em call sites alcançados.

## Motivação

Hoje, `data Complex (re::Float im::Float)` é monomórfico — cada combinação de
tipos de campo exige uma declaração separada. Tipos numéricos polimórficos
(Complex sobre Int/Float/Rational, matrizes genéricas) precisam parametrização.

O mecanismo existente de famílias/instâncias (refined polimórfico) é
semanticamente inadequado: famílias representam visões refinadas de tipos base
existentes (`NonZero` é um `Int` restrito, com `alias_of = Some("Int")` e
`predicates = Some(...)`), não tipos novos com layout parametrizado
(`Complex::(Int, Int)` é um produto novo com fields próprios, `alias_of = None`).
Adaptar famílias para Complex exigiria `alias_of = Some("Int")` — semanticamente
falso, pois Complex não é alias de Int.

## Depende de

- **Monomorphizer** ✅ — fixpoint em `kata-monomorph/src/lib.rs:86` já percorre
  TAST, instancia call sites genéricos, e remove templates após monomorfização.
- **`unify`** ✅ — `kata-inference/src/infer/generics.rs:87` já binda
  `Ty::Var(name)` em `type_params` para tipos concretos dos argumentos.
- **`collect_type_params`** ✅ — `kata-resolution/src/type_resolve.rs:355` já
  coleta `Ty::Var` e `Ty::Interface` recursivamente em assinaturas.
- **Interfaces com supertraits** ✅ — `InterfaceRegistry::iface_inherits`
  percorre hierarquia. `SCALAR extends NUM` funciona sem novo mecanismo.

## Design

### Decisão central

Generics paramétricos com monomorfização on-demand. Uma única entrada no
StructRegistry; uma única entrada no InterfaceRegistry. O struct em si não
precisa de monomorfização — só os métodos. O invariante que preserva isso é
`offset = field_index * 8`: todo valor Kata ocupa um word de 8 bytes no
layout físico — primitivos inline (Int = i64, Float = f64) ou ponteiro para
a arena bumpalo (Rational, structs, tuplas, coleções). O codegen
(`StructConstruct` em `lowering/expr.rs`) aloca `N * 8` bytes e armazena
cada campo em `i * 8`; `FieldAccess` carrega em `field_index * 8` com o
tipo CLIF correto (`resolve_clif_ty`: I64 para Int/Text/Rational/ponteiros,
F64 para Float). O tipo CLIF do load/store varia (I64 vs F64), mas o
tamanho físico é sempre 8 bytes. `Complex::(Int, Float)` e
`Complex::(Float, Float)` têm layout idêntico: 16 bytes, dois words. O
invariante `offset = field_index * 8` preserva-se universalmente — não é
específico a SCALAR-bounded params, vale para qualquer type param.

### Sintaxe

#### Forma compartilhada — type param com bound

```kata
data Complex (re::T im::T) where T implements SCALAR
```

- `(re::T im::T)` — campos com type param T (PascalCase em posição de tipo,
  como `Ok(T)` em enums).
- `where T implements SCALAR` — bound: T precisa implementar SCALAR. `where`
  é novo keyword no lexer.
- T é o mesmo tipo em ambos os campos.

#### Forma independente — vars anônimas com bound

```kata
data Complex (re::SCALAR im::SCALAR)
```

- `SCALAR` na posição de tipo do campo é interpretado no pass0 como "var
  fresca anônima com bound SCALAR".
- Cada ocorrência de SCALAR é uma var distinta — desugar para
  `Complex (re::A im::B) where A implements SCALAR, B implements SCALAR`.
  Isso permite `Complex (re::Int im::Float)` — tipos diferentes em cada
  campo. Para tipos coordenados onde re e im devem ser o mesmo tipo (caso
  canônico de Complex numérico), usar a forma compartilhada
  `data Complex (re::T im::T) where T implements SCALAR`. A forma
  independente é açúcar para bounds independentes, não para params
  compartilhados.
- Mecanismo análogo ao que `data (NUM, ...) as NonZero` já usa: `NUM` na
  posição de tipo significa "tipo que implementa NUM". A diferença é que sem
  `as Name` e sem predicados, é data paramétrico, não família refined.

#### Sem bounds — type param livre

```kata
data Par (first::A second::B)
```

- A e B são type params livres (sem bound). PascalCase em posição de tipo,
  detectados no pass0.
- Construtor aceita qualquer par de tipos.

#### Gramática

```
DataDecl ::= 'data' Name '(' Fields ')' WhereClause?
           | 'data' '(' IfacePreds ')' 'as' Name                     # família refined (atual)

WhereClause   ::= 'where' Bound (',' Bound)*
Bound         ::= PascalName 'implements' ALL_CAPS
```

Type params são detectados implicitamente: PascalName em posição de tipo nos
fields é type param (como `Ok(T)` em enums). Sem lista explícita `::(...)`.

#### Token novo: `where`

- `where` é lowercase keyword. Não colide com PascalCase (tipos), ALL_CAPS
  (interfaces), nem snake_case (funções/variáveis).
- Adição ao lexer: uma linha em `ident.rs` (`"where" => Token::Where`).
- Adição ao AST: `Where` no enum `Token` + Display + match de exaustividade
  (~3 linhas em `token.rs`).

### Detecção de type params no pass0

Hoje, `pass0.rs:470-484` percorre fields de `DataDecl` e resolve cada tipo com
`resolve_type_expr`, registrando `FieldInfo { ty, ... }` no StructRegistry.
Para generics, o pass0 precisa:

1. **Detectar type params:** PascalName em posição de tipo nos fields é type
   param. `is_type_param_name` (`type_resolve.rs:344`) já implementa essa
   convenção: `name.chars().all(|c| c.is_ascii_uppercase()) && name != "Self"`.

2. **Distinguir interface de type param na forma independente:** Quando o
   field tem `SCALAR` na posição de tipo, `is_type_param_name("SCALAR")`
   retorna `true` (todas maiúsculas). O pass0 precisa consultar
   `InterfaceRegistry` antes de tratar como type param: se `SCALAR` é
   interface registrada, é bound (gera var anônima com bound); se não é
   interface, é type param livre. Essa consulta acontece antes de
   `resolve_type_expr`, no momento de classificar o field.

3. **Registrar `Ty::Var("T")` nos FieldInfo:** Em vez de resolver o tipo do
   field para um tipo concreto, o pass0 registra `Ty::Var("T")` quando detecta
   que `T` é type param. O `StructInfo` armazena os campos com vars, e
   `lookup_instantiated` substitui na consulta.

4. **Extrair bounds do `where` clause:** Se a declaração tem `where T
   implements SCALAR`, o pass0 extrai `type_params` e `where_bounds` da AST e
   os armazena no `StructInfo`.

5. **Rejeitar type params em família refined:** `data (NUM, ...) as NonZero`
   com type params nos fields é rejeitado — família refined não aceita type
   params. São mecanismos ortogonais.

### StructRegistry

#### Entrada única

Uma única entrada para `Complex` com campos `Ty::Var("T")`. O offset continua
`field_index * 8` (invariante).

```rust
StructInfo {
    name: "Complex",
    fields: [
        FieldInfo { name: "re", ty: Ty::Var("T"), offset: 0 },
        FieldInfo { name: "im", ty: Ty::Var("T"), offset: 8 },
    ],
    type_params: Some(vec![TypeParamDecl { name: "T", bound: Some("SCALAR") }]),
    ...
}
```

`StructInfo` ganha campo `type_params: Option<Vec<TypeParamDecl>>`.
`TypeParamDecl { name: String, bound: Option<String> }` — nome do param
(detectado por PascalCase nos fields) e interface do bound (`None` = livre,
`Some("SCALAR")` = do `where`).

#### Nova variante: `StructKey::Generic`

```rust
StructKey::Generic(String, Vec<Ty>)
```

Isso preserva o invariante atual: structs `data` vivem em `Ty::Struct`, enums
e intrínsecos em `Ty::Generic`. Sites que fazem match `Ty::Struct(key)`
continuam funcionando — extraem type args quando `key` é `Generic`. Usar
`Ty::Generic` para structs paramétricos quebraria esse invariante e exigiria
auditar todo site que distingue `Ty::Struct` de `Ty::Generic` (cache_key,
snapshot, shape, caps, etc.).

`StructKey` deriva `Eq, Hash` — `Vec<Ty>` implementa ambos (Ty é Eq+Hash), e a
nova variante é automaticamente compatível com BTreeMap/HashMap existentes.

#### Instanciação

```rust
fn lookup_instantiated(&self, name: &str, type_args: &[Ty]) -> Option<InstantiatedStructInfo>
```

`InstantiatedStructInfo` substitui `Ty::Var("T")` pelos type args concretos nos
tipos dos campos. O struct físico (layout) é idêntico — só os tipos annotados
mudam para o type checker.

#### Ortogonalidade com Family/Instance

`Complex` paramétrico é `StructKey::Generic("Complex", args)` com
`type_params: Some(...)` no `StructInfo`. Famílias refined
(`StructKey::Instance`) continuam funcionando como hoje. São mecanismos
ortogonais: famílias expandem predicados sobre tipos base, generics
paramétricos parametrizam layout de tipos novos. Um tipo não pode ser ambos.

### Smart constructors

#### Overload genérico

Um único overload genérico é registrado no DispatchTable:

```kata
Complex :: T T => Complex    # onde T implements SCALAR
```

O `unify` em `generics.rs` já binda `Ty::Var("T")` em `type_params` para tipos
concretos dos argumentos. Dado `Complex 3 4`, unify binda `T → Int`, retorna
`Complex` instanciado como `Complex::(Int, Int)`. Dado `Complex 1.0 2.0`,
binda `T → Float`, retorna `Complex::(Float, Float)`.

#### Verificação de bound

Após bindar T, o type checker verifica o bound: `Int implements SCALAR?` →
consulta `InterfaceRegistry::type_implements("Int", "SCALAR")`. Se falha, erro
cedo: `"Text não implementa SCALAR"`.

Para a forma independente (`data Complex (re::SCALAR im::SCALAR)`), o desugar
gera dois params anônimos A e B, cada um com bound SCALAR. O construtor
`Complex :: A B => Complex` verifica ambos os bounds.

A verificação de bounds fica no inference (após `unify` bindar T), não no
`match_score`. `match_score` já consulta `InterfaceRegistry` para o caso
`iface` (param é `Ty::Interface`), mas não consulta bounds de `where`-clause
— esses são verificados após o bind. `apply_dispatch` rejeita após bind se o
bound falha. A rejeição é **terminal**: o erro pós-bind deve ser formatado
como `TypeMismatch` com `"não implementa"` no campo `found`, para que o
filter em `apply_dispatch.rs:475` (`found.contains("não implementa")`)
propague imediatamente sem cair para o próximo overload. Isso coloca o
bound-check de `where` no mesmo status semântico do check de `Ty::Interface`
em `unify` (que também rejeita terminalmente).

#### match_score

`match_score` pontua `exact > iface > generic` — argumentos concretos casam
com params genéricos com score `generic` (via `is_generic_origin: true` no
`Score`). Overloads específicos (`Complex :: Int Int => Complex::(Int, Int)`)
casam com score `exact` e ganham prioridade.

### Interface impls

#### ImplEntry com type_bounds

```kata
Complex implements RING
    + :: Complex Complex => Complex
    lambda a b: Complex (+ a.re b.re) (+ a.im b.im)
```

Uma única entrada no InterfaceRegistry:

```rust
ImplEntry {
    type_name: "Complex",
    type_params: vec!["T"],           // já existe
    interface_name: "RING",
    type_bounds: vec![("T", "SCALAR")],   // campo novo — do `where`
    ...
}
```

`ImplEntry` já tem `type_params: Vec<String>` (campo existente). Ganha
`type_bounds: Vec<(String, String)>` — pares (param_name, iface_name),
extraídos da cláusula `where` do `data`. Vazio para tipos não-genéricos ou
sem bounds.

#### type_implements estendido

Hoje `type_implements(&self, type_name: &str, iface_name: &str) -> bool`
compara strings. Precisa de uma versão que aceita type args:

```rust
fn type_implements_generic(
    &self,
    type_name: &str,
    type_args: &[Ty],
    iface_name: &str,
) -> bool
```

Lógica: encontra `ImplEntry` com `type_name` e `interface_name`. Se o
ImplEntry tem `type_bounds`, extrai type args do `Ty::Struct(StructKey::Generic(...))`,
verifica que cada arg satisfaz o bound correspondente via
`type_implements(arg_type_name, bound_iface)`. Se todos satisfazem, retorna
true.

`type_implements` original permanece intacto. `type_implements_generic` é uma
função nova separada. A separação é arquitetural, não workaround: as duas
funções respondem queries semanticamente distintas — `type_implements`
pergunta "tipo X implementa iface Y?" (semântica nominal, sem type args);
`type_implements_generic` pergunta "tipo X aplicado a args implementa iface
Y?" (semântica de instanciação, com verificação de bounds). Os ~9 sites
existentes que chamam `type_implements` não têm type args em mãos no call
site — estão em contextos onde o tipo foi normalizado para um nome. Unificar
exigiria churn cosmético (`&[]` em todos os sites) ou seria impossível (os
call sites não têm type args para passar).

Sites que devem chamar `type_implements_generic`: apenas sites que lidam com
`StructKey::Generic` (verificação de bound pós-unify em `apply_dispatch`,
verificação de conformidade de impl paramétrico). Sites que lidam com tipos
concretos (Int, Float, NonZero/Int) continuam chamando `type_implements`.

Para evitar divergência futura (feature adicionada em uma função e esquecida
na outra), considerar extrair `impl_matches_bounds(entry, type_args)` como
helper compartilhado entre as duas.

#### Métodos paramétricos — uma definição, instanciada on-demand

`+ :: Complex Complex => Complex` é uma definição (vira `TypedFunction` no
TAST via `pass0.rs:1041-1052`). O monomorphizer cria instâncias concretas
(`+ :: Complex::(Int, Int) Complex::(Int, Int) => Complex::(Int, Int)`) para
cada combinação de type args que aparece em call sites alcançados.

O body `(+ a.re b.re)` usa dispatch normal: `a.re : T → + :: T T => T` resolve
no DispatchTable. Após monomorfização, T é substituído por Int/Float/etc., e
`+ :: Int Int => Int` é encontrado normalmente.

#### Sobrecargas cobrem coerção

Overloads explícitos como `+ :: Complex Int => Complex` continuam funcionando
como hoje — são overloads adicionais com score `exact` nos args mistos. Não
são o caminho genérico principal.

### Recursão em `Ty::Struct` — 4 funções a estender

Quatro funções hoje não recursam em `Ty::Struct`, caindo no braço `_` (curinga).
Com `StructKey::Generic("Complex", [Var("T")])`, é preciso substituir/coletar
dentro dos type args. Cada correção é um braço de match adicional:

1. **`apply_subs`** (`generics.rs:367`) — substitui `Ty::Var` por tipos
   concretos. Hoje `_ => ty.clone()` para `Ty::Struct`. Precisa recursar nos
   args de `StructKey::Generic`:
   ```rust
   Ty::Struct(key) => Ty::Struct(apply_subs_struct_key(key, subs))
   ```
   Sem isso, a instanciação de métodos não substitui `Var("T")` dentro de
   tipos `Complex::(T, T)` nos corpos.

2. **`contains_var`** (`ty.rs:168`) — detecta se um tipo contém `Ty::Var`.
   Hoje `_ => false` para `Ty::Struct`. O monomorphizer usa isso em
   `lib.rs:127` para filtrar templates. Precisa recursar nos args de
   `StructKey::Generic`. Sem isso, métodos com param
   `Ty::Struct(StructKey::Generic("Complex", [Var("T")]))` não são detectados
   como template e não são filtrados após monomorfização.

3. **`collect_type_params`** (`type_resolve.rs:355`) — coleta type params de
   assinaturas. Hoje `_ => {}` para `Ty::Struct` (exceto `Instance` com
   concrete type param). Precisa recursar nos args de
   `StructKey::Generic`. Sem isso, o construtor `Complex :: T T => Complex`
   não tem `type_params: ["T"]` e o `unify` não binda.

4. **`substitute_self`** (`ty.rs:196`) — substitui `Ty::Var("Self")` por tipo
   concreto. Hoje `_ => self.clone()` para `Ty::Struct`. Precisa recursar nos
   args de `StructKey::Generic`. Sem isso, métodos de impl que usam `Self` com
   tipo paramétrico não instanciam corretamente.

### Monomorphização

#### Quando instanciar

O monomorphizer (`kata-monomorph/src/lib.rs:86`) já percorre a TAST
procurando call sites genéricos e gera instâncias concretas em fixpoint. Para
`Complex::(T, T)`, os call sites são:

1. Construtores: `Complex 3 4` → instancia `Complex::(Int, Int)`
2. Methods: `+ z1 z2` onde `z1 : Complex::(Int, Int)` → instancia
   `+ :: Complex::(Int, Int) Complex::(Int, Int) => Complex::(Int, Int)`
3. Field access: `z.re` onde `z : Complex::(Int, Int)` → `re : Int`

O fixpoint já existe. A extensão é: quando o monomorphizer encontra
`Ty::Struct(StructKey::Generic("Complex", [Int, Int]))`, `apply_subs`
(substituição estendida) substitui `Ty::Var("T")` por `Ty::Prim(Int)` nos
corpos dos métodos instanciados.

#### O que não precisa de instância

O struct em si — layout, offsets, alocação na arena — é idêntico para todas
as combinações. O codegen não precisa de versões diferentes de
`Complex::(Int, Int)` vs `Complex::(Float, Float)` para construir/destruir o
struct. Só os métodos (que despacham para operações de T) precisam de
instâncias concretas.

#### Tree-shaking

O tree-shaker já remove funções/actions não alcançadas. Métodos instanciados
pelo monomorphizer que não são chamados são removidos. As entradas no
StructRegistry e InterfaceRegistry permanecem (como já acontece hoje), mas
são uma única entrada para Complex, não uma por implementor.

### Por que não Family/Instance

#### Diferença semântica: refined view vs tipo novo

- **Família** = visão refinada de um tipo base existente. `NonZero` é um
  `Int` com predicado `!= _ (zero _)`. O `StructInfo` reflete isso:
  `alias_of = Some("Int")`, `predicates = Some(...)`, `fields = []`. O layout
  é o do tipo base — NonZero não tem campos próprios.
- **Generics paramétricos** = tipo novo cujo layout é parametrizado.
  `Complex::(Int, Int)` tem dois campos `re::Int` e `im::Int`. Não é uma
  restrição de Int — é um produto. `alias_of = None`, `fields = [re, im]`.

Adaptar famílias para Complex exigiria fingir que Complex é "uma família sobre
SCALAR" — semanticamente falso. `StructInfo.is_instance_of` e `alias_of`
codificariam a semântica errada: `alias_of = Some("Int")` quando Complex não
é alias de Int.

#### Bounds por-parâmetro

`where T implements SCALAR, R implements NAT` escala para múltiplos type
params com bounds independentes. O mecanismo de famílias não tem onde
pendurar bounds por-parâmetro — só tem "a interface da família", que é um
único bound para um único base.

## Estruturas afetadas

| Camada | Arquivo | Mudança |
|---|---|---|
| **Lexer** | `kata-lexer/src/ident.rs:23` | `"where" => Token::Where` — uma linha no match de keywords |
| **AST** | `kata-ast/src/token.rs` | `Where` no enum `Token` + Display + exaustividade (~3 linhas) |
| **AST** | `kata-ast/src/item.rs:56` | `DataDecl` ganha `where_bounds: Vec<(String, String)>` |
| **AST** | `kata-ast/src/item.rs` | `TypeParamDecl { name: String, bound: Option<String> }` — struct nova |
| **Core** | `kata-core/src/struct_registry.rs:31` | `StructKey::Generic(String, Vec<Ty>)` — nova variante |
| **Core** | `kata-core/src/struct_registry.rs:44,53` | `name()` e `concrete_type()` — novo braço em cada |
| **Core** | `kata-core/src/struct_registry.rs:76` | `StructInfo` ganha `type_params: Option<Vec<TypeParamDecl>>` |
| **Core** | `kata-core/src/struct_registry.rs:151,157,179,205` | 4 sites de construção ganham `type_params: None` |
| **Core** | `kata-core/src/struct_registry.rs` | `lookup_instantiated(name, type_args)` — método novo |
| **Core** | `kata-core/src/interface_registry.rs:56` | `ImplEntry` ganha `type_bounds: Vec<(String, String)>` |
| **Core** | `kata-core/src/interface_registry.rs:255` | 1 site de construção ganha `type_bounds: vec![]` |
| **Core** | `kata-core/src/interface_registry.rs` | `type_implements_generic(type_name, type_args, iface)` — função nova |
| **Core** | `kata-core/src/ty.rs:168` | `contains_var` — adicionar braço `Ty::Struct(StructKey::Generic(..))` |
| **Core** | `kata-core/src/ty.rs:196` | `substitute_self` — adicionar braço `Ty::Struct(StructKey::Generic(..))` |
| **Core** | `kata-core/src/ty.rs:262,360` | Display + display — novo braço para `StructKey::Generic` |
| **Core** | `kata-core/src/dispatch/mod.rs:524` | `match_score` — novo braço para `StructKey::Generic` |
| **Inference** | `kata-inference/src/infer/generics.rs:367` | `apply_subs` — adicionar braço `Ty::Struct(StructKey::Generic(..))` |
| **Inference** | `kata-inference/src/infer/generics.rs:118` | `unify_one` — novo braço para casar `StructKey::Generic` com `StructKey::Generic` |
| **Inference** | `kata-inference/src/infer/apply_dispatch.rs` | Após unify, verificar bounds antes de aceitar |
| **Resolution** | `kata-resolution/src/type_resolve.rs:355` | `collect_type_params` — adicionar braço `Ty::Struct(StructKey::Generic(..))` |
| **Resolution** | `kata-resolution/src/pass0.rs:469` | Detectar type params em fields, registrar `Ty::Var` nos FieldInfo, extrair `where_bounds` |
| **Resolution** | `kata-resolution/src/pass0.rs:469` | Forma independente: consultar InterfaceRegistry para distinguir interface de type param |
| **Resolution** | `kata-resolution/src/pass0.rs:297` | Rejeitar type params em família refined |
| **Monomorph** | `kata-monomorph/src/lib.rs` | Sem mudança estrutural — `apply_subs` estendido faz a substituição |
| **Stdlib** | `stdlib/complex.kata` | Migração para `data Complex (re::T im::T) where T implements SCALAR` |
| **Stdlib** | `stdlib/core.kata` | `interface SCALAR extends NUM` com `one :: Self => Self` |

### Sites com match exaustivo em `StructKey`

`StructKey` tem 3 variantes hoje. Adicionar `Generic` força o compilador Rust
a reportar todo match exaustivo que não cobre a nova variante. Há **~20 sites
de produção** (excluindo testes) que fazem match em `StructKey`:

- `struct_registry.rs`: `get`, `lookup`, `all_instances`, `iter_all`,
  `retain_by_closure`, `merge` — 16 ocorrências
- `dispatch/mod.rs`: `match_score` — 6
- `ty.rs`: Display + display — 6
- `apply_dispatch.rs`: 10
- `pass0.rs`: 12
- `generics.rs`: 5 (unify_one)
- `type_resolve.rs`: 9
- `ascription.rs`: 9
- `constructors_refined.rs`: 7
- codegen: `cache_key.rs`, `type_table.rs`, `lowering/mod.rs` — 7

Rust força exaustividade, então o compilador pega todos. É mecânico, não
conceitual — cada site ganha um braço `StructKey::Generic(name, args) => ...`
que extrai o nome e/ou type args conforme necessário.

## Fases

### Fase 1 — Token `where` e AST

- Adicionar `Token::Where` no lexer (`ident.rs`) e AST (`token.rs`).
- Adicionar `TypeParamDecl` e `where_bounds` em `DataDecl` (`item.rs`).
- Parser parseia `data Name (fields) where Bounds` e detecta type params por
  PascalCase nos fields.
- **DoD:** `cargo check` limpo. Parser aceita e rejeita sintaxe corretamente.

### Fase 2 — StructRegistry e recursão em `Ty::Struct`

- `StructKey::Generic(String, Vec<Ty>)` — nova variante.
- `StructInfo` ganha `type_params: Option<Vec<TypeParamDecl>>`.
- `lookup_instantiated(name, type_args)` substitui vars nos tipos dos campos.
- pass0 registra struct com `Ty::Var` nos campos e extrai `where_bounds`.
- **Estender 4 funções recursivas:** `apply_subs`, `contains_var`,
  `collect_type_params`, `substitute_self` ganham braço
  `Ty::Struct(StructKey::Generic(..))` que recursa nos type args.
- **DoD:** `cargo check` limpo (todos matches exaustivos cobertos).
  StructRegistry consulta `Complex::(Int, Int)` e retorna campos com tipos
  concretos.

### Fase 3 — Smart constructor

- Overload genérico registrado no DispatchTable.
- `unify` binda type params a partir dos argumentos (já funciona — verificar).
- `unify_one` ganha braço para casar `StructKey::Generic` com
  `StructKey::Generic` (unifica type args recursivamente).
- Verificação de bound após bind em `apply_dispatch`.
- **DoD:** `Complex 3 4` tipa como `Complex::(Int, Int)`.
  `Complex "a" "b"` falha com "Text não implementa SCALAR".

### Fase 4 — SCALAR com `one` ✅

- `interface SCALAR extends NUM` com `one :: Self => Self`.
- `zero :: Self => Self` já está em RING (herdado por NUM/FIELD/SCALAR).
- Int e Float implementam SCALAR com `one` via lambda (`lambda _: 1`, `lambda _: 1.0`).
- Rational também implementa SCALAR (`lambda _: rational 1`).
- **DoD:** `(one x)` tipa como Int quando `x : Int`, Float quando `x : Float`. ✅
- **Bug fix:** `is_overridden` em `pass0.rs:1154` não verificava signatures já
  registradas por outros blocos `implements` do mesmo tipo. Quando SCALAR
  (que herda de NUM→FIELD) era processado, o default method `//` de FIELD
  era regenerado com Self=Int, produzindo `int(Int)` sem overload. Corrigido
  verificando também `signatures.iter().any(...)`.

### Fase 5 — InterfaceRegistry

- `ImplEntry` ganha `type_bounds`.
- `type_implements_generic` unifica type args com bounds.
- `Complex::(T, T) implements RING` registra uma única entrada.
- **DoD:** `type_implements_generic("Complex", [Int, Int], "RING")` retorna
  true.

### Fase 6 — Monomorphização de métodos

- Monomorphizer substitui `Ty::Var` por type args concretos nos corpos (via
  `apply_subs` estendido na Fase 2).
- Instância on-demand por combinação de type args usada.
- **DoD:** `+ z1 z2` onde `z1 : Complex::(Int, Int)` despacha para
  `+ :: Complex::(Int, Int) Complex::(Int, Int) => Complex::(Int, Int)` com
  body `(+ a.re b.re)` onde `a.re : Int`.

### Fase 7 — Migração da stdlib

- `complex.kata` migra para generics paramétricos.
- Overloads específicos (`+ :: Complex Int => Complex`) mantidos como
  overloads adicionais.
- **DoD:** `cargo test` passa. `kata run` em exemplos de Complex produz
  output correto.

## Fora do escopo

- **Higher-kinded types:** `Functor::(F)` onde F é um type constructor não
  entra. Type params são sempre de kind `*`.
- **Generics em enum:** `enum Result` já é genérico via convenção UPPER_CASE.
  Este PRD é sobre `data` only.
- **Const generics:** Dimensões no tipo (ex: `Matrix::(3, 3)`) não entram.
  Shapes ficam em runtime.
- **Default methods em interface:** Interfaces com `default_body` já existem.
  Não muda com este PRD.
- **Refinados polimórficos:** `data (NUM, ...) as NonZero` continua
  funcionando. É mecanismo ortogonal (expande predicados, não parametriza
  layout).

## Riscos

### R1: Cascata de novo campo em StructInfo/ImplEntry

`StructInfo` ganha `type_params` e `ImplEntry` ganha `type_bounds`. Novo
campo em struct/enum toca sites de clonagem, comparação, serde, testes.
**Mitigação:** usar `Option<Vec<...>>` com `None` como default para tipos
não-genéricos — minimiza mudança em sites existentes (4 sites em
StructInfo, 1 em ImplEntry).

### R2: `type_implements` string-based

`type_implements` é chamado em ~9 sites de produção com `&str`. A versão
genérica (`type_implements_generic`) é uma função nova, não um patch. A
separação é arquitetural: as queries são semanticamente distintas (nominal
vs instanciação com bounds). Os call sites existentes não têm type args
para passar.
**Mitigação:** manter `type_implements` original intacto;
`type_implements_generic` como função separada. Documentar quais sites
chamam qual (ver seção "type_implements estendido"). Considerar extrair
`impl_matches_bounds(entry, type_args)` como helper compartilhado para
evitar divergência futura.

### R3: Interação com famílias refined

Famílias (`StructKey::Instance`) e generics paramétricos
(`StructKey::Generic`) coexistem. Um tipo não pode ser ambos.
**Mitigação:** pass0 rejeita `data (NUM, ...) as NonZero` com type params nos
fields — família refined não aceita type params.

### R4: SCALAR quebra builtins

Redefinir `interface SCALAR` parcialmente pode sombrear métodos do prelude.
**Mitigação:** SCALAR é declarada uma vez no prelude com todos os métodos.
Implementors de SCALAR (Int, Float, Rational) fornecem todos os métodos.

### R5: `match_score` com verificação de bounds

Hoje `match_score` já consulta `InterfaceRegistry` para o caso `iface`
(param é `Ty::Interface`, arg implementa via `type_implements`). Ligar
verificação de bounds de `where`-clause ao `match_score` exigiria consultar
`InterfaceRegistry` com type args no meio do scoring — atualmente
`match_score` não recebe type args.
**Mitigação:** a verificação de bounds de `where` fica no inference (após
`unify` bindar T), não no `match_score`. `match_score` continua sem
consultar bounds de `where`-clause. `apply_dispatch` rejeita
terminalmente após bind se o bound falha (erro formatado como "não
implementa" para cair no filter de propagação imediata).

### R6: Ambiguidade SCALAR como interface vs type param

`is_type_param_name("SCALAR")` retorna `true` (todas maiúsculas). Na forma
independente `data Complex (re::SCALAR im::SCALAR)`, o pass0 precisa
distinguir "SCALAR é interface registrada" (→ bound) de "SCALAR é type param
livre" (→ `data Par (first::A second::B)`).
**Mitigação:** pass0 consulta `InterfaceRegistry` antes de tratar como type
param. Se `SCALAR` é interface registrada, é bound; se não é interface, é
type param livre. A consulta é feita no momento de classificar o field, antes
de `resolve_type_expr`.

### R7: Volume de matches exaustivos em `StructKey`

~20 sites de produção fazem match exaustivo em `StructKey`. Adicionar
`Generic` força um novo braço em cada um.
**Mitigação:** Rust força exaustividade — o compilador reporta todos os sites
não cobertos. Cada site ganha um braço mecânico
`StructKey::Generic(name, args) => ...`. É volume, não complexidade conceitual.

## Documentação

- `docs/base/kata-book/` — atualizar seções sobre `data` com type params e
  `where` clause
- `docs/base/sintaxe-mapa.md` — atualizar tabela de sintaxe de `data`
- `docs/base/Kata-lang-manual.md` — atualizar seção de tipos `data`
- `docs/TODO.md` — adicionar/remover item sobre generics paramétricos