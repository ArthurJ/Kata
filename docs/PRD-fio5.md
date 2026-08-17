# PRD: Fio 5 — Data, Structs, Tuples, alias

## Objetivo

Trazer tipos produto (`data` com campos), field access, index access em
tuplas, aliases (newtypes), smart constructors infalíveis para structs,
`format` (interpolação de strings), `$` spread, e ascription-construção
(promoção de tupla anônima a tipo nominal) para o pipeline Kata5.

Este é o fio de modelagem de dados. Antes dele, `data` só existe como tipo
opaco (`data Int ()` com `@ffi`). Depois dele, o usuário pode declarar
`data Pessoa (nome::Text idade::Int)`, construir valores com `Pessoa "João" 30`,
acessar campos com `pessoa.nome`, indexar tuplas com `t.0` e `t.(-1)`, criar
newtypes com `alias`, formatar strings com `format`, e espalhar tuplas com `$`.

## Depende de

Fio 1 (pipeline end-to-end, TypeEnv, DispatchTable, `Ty::Struct`/`Ty::Tuple`
já existem, `Token::Dot`/`Token::Alias`/`Token::Dollar` já existem no lexer),
Fio 2 (lambdas, match, pattern matching, hole, typeck com hint), Fio 4
(`Ty::Sum` com payload, variantes com dados, match general case).

## Estado herdado

O Fio 1 já deixou infraestrutura pronta:

- **`Ty::Struct(String)`** e **`Ty::Tuple(Vec<Ty>)`** já existem em `kata-core`.
- **`TypeShape::Struct { fields }`** e **`TypeShape::Tuple { elements }`** já
  existem — ambos marcados como heap types (`is_heap = true`).
- **`DataDecl { name, fields: Vec<FieldDecl>, directives }`** na AST.
- **`FieldDecl { name, ty: Spanned<TypeExpr> }`** na AST.
- **Parser `parse_data_decl` + `parse_field_decls`** — já parseia
  `data Pessoa (nome::Text idade::Int)` (campos separados por espaço, cada um
  `nome::Tipo`).
- **`Token::Dot`** — lexado mas nunca consumido pelo parser.
- **`Token::Alias`** — no token set, nunca parseado.
- **`Token::Dollar`** — no token set, nunca parseado.
- **`Expr::Tuple { elements }`** — parser já produz, codegen já aloca na arena
  com store por elemento.
- **`TypedExprKind::Tuple { elements }`** — TAST já tem, inferência já produz
  `Ty::Tuple(element_tys)`.
- **Ascription (`expr::Type`)** — já funciona para rebaixamento de literal
  (`42::Float`) e confirmação de tipo (`x::Int`).
- **`TypedExprKind::TypeAscription { expr, target_ty }`** já existe na TAST.

O que não existe e este PRD cria:

1. Guardar layout de struct (campos + tipos) no TypeEnv
2. Smart constructor de struct (função sintetizada no DispatchTable)
3. StructAlloc + FieldStore no codegen
4. Field access (`expr.nome`) — parser, typeck, codegen
5. Index access em tupla (`t.0`, `t.(-1)`) — parser, typeck, codegen
6. `alias` (newtype) — parser, resolution, smart constructor
7. `format` (builtin sintetizado)
8. ~~`$` spread (interceptado pelo typeck)~~ — **removido 2026-08-17** (redundante: `f $ (a, b)` ≡ `f a b`, sem consumidor real)
9. Ascription-construção (`(a, b)::Struct`)
10. `repr` auto-sintetizado para `data` com campos

## Modelo

### Layout de struct no TypeEnv

O `TypeEnv` hoje mapeia `name → Ty`. Para structs, isso não basta —
precisamos saber os campos, seus tipos, e offsets. Decisão: estender `TypeEnv`
com um catálogo de structs, análogo ao `EnumRegistry`.

`StructRegistry` (novo, em `kata-core`):

```rust
pub struct StructRegistry {
    structs: HashMap<String, StructInfo>,
}

pub struct StructInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,  // em ordem de declaração
}

pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub offset: u32,  // offset em bytes = índice * 8
}
```

`StructRegistry` é populado no resolution (Pass 0) quando `DataDecl` tem campos
não-vazios. `Ty::Struct(name)` no TypeEnv aponta para a entrada no registry.
O `offset` é calculado como `field_index * 8` (todos os campos são words de
8 bytes — structs e tuplas são blocos contíguos de `n * 8` bytes).

`ResolvedModule` ganha `pub struct_registry: StructRegistry` ao lado de
`enum_registry`.

### Smart constructor de struct (função sintetizada)

`data Pessoa (nome::Text idade::Int)` sintetiza uma função
`Pessoa :: Text Int => Pessoa` no DispatchTable. A função é infalível —
todos os campos são aceitos sem validação. A assinatura é determinada pelos
tipos concretos dos campos, não por interfaces (o construtor precisa saber
exatamente qual tipo esperar para alocar e preencher — ver manual §4.2.3).

O smart constructor é uma `TypedFunction` com body sintetizado. O body é um
novo nó TAST `TypedExprKind::StructConstruct { struct_name, values: Vec<Spanned<TypedExpr>> }`.

O codegen lowera `StructConstruct` como:
1. `kata_rt_arena_alloc(handle, n * 8)` — aloca `n * 8` bytes na arena
   (escolha de arena via `EscapeTarget`, igual a tuple)
2. Para cada campo `i`: `store ptr + i*8, value_i`

Isso é semanticamente idêntico ao codegen de tuple — a diferença é que struct
tem identidade nominal (o nome do tipo) e campos nomeados. O layout na arena
é o mesmo: bloco contíguo de words.

**Overloads manuais coexistem.** O usuário pode adicionar
`Pessoa :: Int Int => Pessoa` com um body diferente. O DispatchTable seleciona
por dominância. O construtor sintetizado é uma overload como qualquer outra.

### Field access (`expr.nome`)

Sintaxe: `expr.nome` onde `nome` é um `Ident`.

Parser: após parsear um atom, se o próximo token é `Token::Dot` seguido de
`Token::Ident`, produz `Expr::DotAccess { expr, index: DotIndex::Field(name) }`.
Se seguido de `Token::IntLit(n)` (incluindo negativos via `Token::Minus` antes
de `IntLit`), produz `Expr::DotAccess { expr, index: DotIndex::Int(n) }`.

O parser não decide se é field access ou index access — produz o mesmo nó e
o typeck resolve pelo tipo do receptor.

```rust
// AST (kata-ast)
pub enum DotIndex {
    Field(String),  // expr.nome
    Int(i64),        // expr.0, expr.(-1)
}

Expr::DotAccess {
    expr: Box<Spanned<Expr>>,
    index: DotIndex,
}
```

Typeck: dado `expr.ty`:
- Se `Ty::Struct(name)`: field access. Busca `name` no `StructRegistry`,
  encontra o campo por nome, retorna `field.ty`. Se o campo não existe →
  `UnknownField` error.
- Se `Ty::Tuple(elements)`: index access. `DotIndex::Int(n)` →
  resolve índice negativo (`-1` = `len - 1`), bounds check compile-time,
  retorna `elements[n]`. Se `DotIndex::Field` em tupla → error (tupla
  não tem campos nomeados). Se `n >= len` → `IndexOutOfBounds` error.
- Outro: `NotIndexable` error.

```rust
// TAST (kata-inference)
TypedExprKind::FieldAccess {
    expr: Box<Spanned<TypedExpr>>,
    struct_name: String,
    field_name: String,
    field_index: u32,  // offset em words
}

TypedExprKind::IndexAccess {
    expr: Box<Spanned<TypedExpr>>,
    index: i64,  // já resolvido (negativos normalizados)
    element_index: u32,  // offset em words
}
```

Codegen de `FieldAccess`: `load ptr + field_index * 8` — um `load` por offset.
Codegen de `IndexAccess`: `load ptr + element_index * 8` — idêntico.

### Index access em tupla (`t.0`, `t.(-1)`)

Coberto pelo mecanismo de `DotAccess` acima. O parser produz
`DotAccess { expr, DotIndex::Int(n) }`, o typeck vê `Ty::Tuple` e produz
`TypedExprKind::IndexAccess`. Índices negativos são resolvidos em compile-time:
`t.(-1)` = `t.(len-1)`. Bounds check é compile-time — se `n >= len` ou
`n < -len`, é `IndexOutOfBounds` error.

### `alias` (newtype)

Sintaxe: `alias Target as NewName`

Parser: `Token::Alias` → nome target → `Token::As` → novo nome.
Produz `Item::AliasDecl { target: String, new_name: String }`.

```rust
// AST
Item::AliasDecl {
    target: String,
    new_name: String,
}
```

Resolution:
1. `type_env.define(new_name, Ty::Struct(new_name.clone()))` — o alias é um
   tipo nominal distinto do target.
2. Registra no `StructRegistry` que `new_name` herda o layout de `target`
   (campos idênticos). Para tipos opacos (`Int`, `Float`), o alias não tem
   campos — é um wrapper identity.
3. Sintetiza smart constructor: `NewName :: Target => NewName`.
   - Se target é struct: delega — constrói o target e tag como `NewName`.
   - Se target é opaco/primitivo: identity — retorna o valor com tag `NewName`.

O codegen trata alias como o tipo base (mesmo layout, mesmo ABI). A diferença
é apenas nominal — para type-checking, `Altura` ≠ `Float` (rigidez nominal
nas fronteiras de funções), mas em runtime ocupam o mesmo espaço.

**Orphan rule:** o alias existe para permitir implementar interfaces em tipos
externos encapsulando-os localmente. Sem o alias, não se pode implementar
uma interface de stdlib num tipo de stdlib. Com o alias, cria-se um newtype
local e implementa-se a interface nele.

### `format` (builtin sintetizado)

Sintaxe: `format "template {}" (a, b)` — substitui `{}` na ordem dos argumentos.

`format` é um builtin sintetizado pelo typeck. O parser não trata `format`
especialmente — é um `Ident("format")` que despacha como `Apply` normal. O
typeck detecta que o callee é `format` e sintetiza o body.

Síntese: para cada `{}` no template, chama `kata_rt_text_replace_first` com o
`repr` do argumento. O resultado é a concatenação.

Para Fio 5, `format` suporta argumentos `Int`, `Float`, `Text`, `Boolean`.
Para structs aninhadas, delega para `kata_rt_repr_to_text` (que caminha o
`TypeShape`).

**`repr` é auto-sintetizado.** Para todo `data Nome (campos)` o typeck gera
`repr :: Nome => Text` com body que concatena `Nome(` + campos separados por
`, ` + `)`. Por tipo de campo:
- `Text`: identity
- `Int`: `kata_rt_int_to_text` via FFI
- `Boolean`: `kata_rt_bool_to_text` via FFI
- `Struct` aninhado: `repr` recursivo
- Outros: `kata_rt_repr_to_text` via FFI

### ~~`$` spread~~ — **REMOVIDO 2026-08-17**

> ~~Sintaxe: `f $ (a, b)` → `f` recebe `a` e `b` como args separados.~~
>
> Removido: o único caso funcional (`f $ (1, 2, 3)` ≡ `f 1 2 3`) era tautológico.
> Caso com variáveis nunca foi implementado. Sem consumidor real.

~~Sintaxe: `f $ (a, b)` → `f` recebe `a` e `b` como args separados.~~

~~`$` é interceptado pelo typeck, não chega ao codegen. Quando o typeck vê~~
~~`Apply(callee, args)` onde um dos args é `Ident("$")`, expande a tupla~~
~~seguinte: se o arg seguinte é `Tuple { elements }`, substitui o `$` e a tupla~~
pelos elementos individuais. Se o arg seguinte não é tupla → error.

O parser já produz `Ident("$")` (o lexer produz `Token::Dollar`, mas `Dollar`
ainda não está no match do `parse_expr_atom` para `Ident`). Decisão: o lexer
produz `Token::Dollar` e o parser o converte para `Expr::Spread` na posição
de argumento. `Spread` é um marcador — o typeck expande e remove.

```rust
// AST
Expr::Spread,  // marcador: expandir tupla seguinte

// TAST — não existe. Spread nunca chega à TAST.
// O typeck expande Spread + Tuple → elementos individuais antes de
// construir o TypedExprKind::Closure.
```

### Ascription-construção (`(a, b)::Struct`)

Quando `expr::Type` onde `expr.ty = Ty::Tuple(...)` e `target_ty = Ty::Struct(name)`
e o shape da tupla bate com os campos do struct (mesmo número de elementos,
tipos compatíveis), o typeck produz
`TypedExprKind::StructConstruct { struct_name, values }` em vez de
`TypeAscription`. A tupla anônima é promovida a tipo nominal.

Se o shape não bate → `ShapeMismatch` error.

Validação de shape: para cada elemento `i` da tupla, verifica que
`element_ty` é compatível com `field_i.ty` do struct. A ordem é posicional
(não por nome) — a ascription liga elementos a campos por posição.

## Escopo

### Fase 1: StructRegistry no TypeEnv + resolution

- Criar `StructRegistry`, `StructInfo`, `FieldInfo` em `kata-core`
- `ResolvedModule` ganha `struct_registry: StructRegistry`
- Pass 0: quando `DataDecl` tem campos não-vazios, registrar no
  `StructRegistry` com `FieldInfo { name, ty, offset: index * 8 }`
- `Ty::Struct(name)` no TypeEnv já existe — não muda
- Para `alias`: registrar no `StructRegistry` que `new_name` tem o mesmo
  layout de `target`
- Verificação: `cargo test -p kata-core` e `cargo test -p kata-resolution`

### Fase 2: Smart constructor de struct

- Typeck sintetiza `TypedFunction` para cada struct com campos:
  - Nome: `Pessoa` (mesmo nome do tipo)
  - Assinatura: `field_types => Ty::Struct(name)`
  - Body: `TypedExprKind::StructConstruct { struct_name, values: params }`
- Para alias: sintetiza construtor que delega ao target
- Registra no `DispatchTable` como overload
- `TypedExprKind::StructConstruct` na TAST
- Verificação: `cargo test -p kata-inference`

### Fase 3: StructAlloc + FieldStore no codegen

- Lowering de `StructConstruct`: `arena_alloc(n * 8)` + `store` por campo
- Escolha de arena via `EscapeTarget` (idêntico a tuple)
- FieldAccess: `load ptr + offset`
- Verificação: `cargo test -p kata-codegen` — testes E2E de struct

### Fase 4: Field access + Index access (parser → typeck → codegen)

- `Expr::DotAccess { expr, index: DotIndex }` na AST
- `DotIndex::Field(String)` e `DotIndex::Int(i64)` na AST
- Parser: após atom, se `Token::Dot`, consome e produz `DotAccess`
  - `Token::Ident` após `.` → `DotIndex::Field`
  - `Token::IntLit(n)` após `.` → `DotIndex::Int(n)`
  - `Token::Minus` + `IntLit(n)` após `.` → `DotIndex::Int(-n)`
- Typeck: desambigua por `expr.ty`
  - `Ty::Struct(name)` + `Field` → `TypedExprKind::FieldAccess`
  - `Ty::Tuple(...)` + `Int` → `TypedExprKind::IndexAccess` (bounds check)
  - Erro: `Field` em tupla, `Int` em struct, `DotAccess` em não-struct/tupla
- Codegen: `load ptr + offset` para ambos
- Verificação: testes E2E — `pessoa.nome`, `t.0`, `t.(-1)`, `t.5` (error)

### Fase 5: `alias` (newtype)

- `Item::AliasDecl { target, new_name }` na AST
- Parser: `Token::Alias` → target ident → `Token::As` → new_name ident
- Resolution: registra `new_name` no TypeEnv + `StructRegistry`
- Smart constructor: `NewName :: Target => NewName` (identity ou delega)
- Codegen: mesmo layout do target (transparente)
- Verificação: testes E2E — `alias Float as Altura`, `Altura 1.75`

### Fase 6: `format` + `repr` ✅

- `repr` auto-sintetizado para `data` com campos — TypedFunction com nome mangled `__kata_repr__{name}`, body com árvore de `string_concat` + `int_to_text`/`bool_to_text`/`repr` recursivo
- `format` builtin: typeck intercepta `Ident("format")` em `infer_apply`, sintetiza cadeia de `text_replace_first` inline
- Runtime: `kata_rt_text_replace_first`, `kata_rt_string_concat`, `kata_rt_int_to_text`, `kata_rt_bool_to_text` já existiam
- Verificação: 13 testes E2E ✅

### Fase 7: ~~`$` spread~~ + ascription-construção — spread removido 2026-08-17

- ~~`$` é `Ident("$")` (não `Token::Dollar`) — o lexer já produz `Ident`~~ — removido
- ~~Typeck: `expand_spread` em `infer_apply` detecta `Ident("$")` + `Tuple` seguinte, substitui pelos elementos individuais~~ — removido
- Ascription-construção: `Tuple::Struct` com shape check em `infer_expr_hinted` → `StructConstruct`
- Verificação: 5 testes E2E (ascription apenas) ✅

## DoD

1. **Struct com field access funciona.** `data Pessoa (nome::Text idade::Int)`
   seguido de `let p := Pessoa "João" 30` e `p.nome` retorna `"João"`.

2. **Smart constructor infalível.** `Pessoa "João" 30` despacha para o
   construtor sintetizado e produz `Ty::Struct("Pessoa")`.

3. **Tuple com `.N` e `.(-1)`.** `(10, 20, 30).0` = `10`, `.(-1)` = `30`,
   `.5` = compile-time `IndexOutOfBounds`.

4. **`alias` cria newtype.** `alias Float as Altura` permite `Altura 1.75`.
   `Altura` ≠ `Float` em type-checking (rigidez nominal), mas mesmo ABI.

5. **`format` interpola.** `format "{} {}" (42, "ok")` retorna `"42 ok"`. ✅

6. **`repr` auto-sintetizado.** `repr pessoa` retorna `"Pessoa(João, 30)"`
   para `data Pessoa (nome::Text idade::Int)`. ✅

7. ~~**`$` spread expande.** `f $ (a, b)` = `f a b`. Nunca chega ao codegen.~~ **REMOVIDO 2026-08-17** — redundante, sem consumidor real.

8. **Ascription-construção promove.** `("João", 30)::Pessoa` produz
   `Ty::Struct("Pessoa")`. Shape mismatch → error. ✅

9. **Overloads manuais coexistem.** `Pessoa :: Int Int => Pessoa` (manual)
   coexiste com o sintetizado `Pessoa :: Text Int => Pessoa`. Dispatch por tipo.

## Não faz parte deste PRD

- **Interfaces** (`SHOW`, `ITERABLE`, `INDEXABLE`) — Fio 7. `repr` é
  auto-sintetizado aqui, mas a interface `SHOW` formal é Fio 7.
- **Tipos refinados** (`data (Int, > _ 0) as PositiveInt`) — Fio 6.
- **Smart constructors falíveis** — Fio 6.
- **`for x in`** — Fio 8 (depende de ITERABLE).
- **Generics** (`Ty::Generic`) — Fio 7.
- **Monomorphization** — Fio 7.
- **`.N` em coleções** (List, Array) — Fio 8. Aqui `.N` só funciona em tuplas
  (compile-time bounds check) e structs (field access).

## Casos não cobertos

### Struct aninhada

`data Endereco (rua::Text cidade::Text)` e
`data Pessoa (nome::Text endereco::Endereco)` — field access encadeado
`pessoa.endereco.rua`. O typeck já suporta: `pessoa.endereco` retorna
`Ty::Struct("Endereco")`, `.rua` acessa o campo. O codegen faz dois loads
encadeados. Deve funcionar naturalmente sem código extra.

### Tupla de 1 elemento

`(42,)` — trailing comma obrigatório para tupla de 1 elemento. Já funciona
no parser atual (`parse_paren_expr` aceita trailing comma). `Ty::Tuple([Int])`.
`.0` = `Int` (primeiro e único elemento).

### Struct sem campos (tipo opaco)

`data Int ()` — já existe do Fio 1. Não ganha smart constructor (sem campos
para receber). Continua funcionando como antes. O `StructRegistry` não
registra structs com 0 campos.

### Alias de alias

`alias Float as Altura` seguido de `alias Altura as AlturaValida`. O
construtor de `AlturaValida` delega para `Altura` que delega para `Float`.
Deve funcionar por indução — cada alias aponta para seu target.

### `$` sem tupla following

`f $ 42` onde `42` não é tupla → error. O typeck verifica que o argumento
após `Spread` é `Tuple`. Se não for, `SpreadRequiresTuple` error.

### Field access em tipo errado

`42.nome` → `NotIndexable` error (Int não é struct nem tupla).
`t.nome` onde `t` é tupla → `FieldAccessOnTuple` error (tupla não tem campos).
`p.0` onde `p` é struct → `IndexAccessOnStruct` error (struct não é indexável).