# PRD: Fio 6 — Tipos Refinados, Ascription, Ret-Directed Dispatch

## Objetivo

Trazer tipos refinados (`data (Int, > _ 0) as PositiveInt`), smart constructors
falíveis (guard chain sintetizado → `Result`), variantes predicadas de enum
(`enum IMC` com `Magreza(< _ 18.5)`), ascription de expressão com validação
compile-time (`5::PositiveInt`), ret-directed dispatch (hint de retorno
seleciona sobrecarga), coerção contextual no `|`, e grouped ascription
(`((expr))::Type`) para o pipeline Kata5.

Este é o fio de **atrito sadio**: o tipo que obriga prova compile-time na
fronteira, mas mantém interoperabilidade estrutural na base. Antes dele,
`data` só produz structs infalíveis. Depois dele, o usuário pode declarar
tipos que validam predicados, despachar construtores falíveis que retornam
`Result`, e usar ascription para prova compile-time sem `Result`.

## Depende de

- **Fio 1** (TypeEnv, DispatchTable com scoring, `Ty`, `data` opaco, `enum` unitário)
- **Fio 2** (lambdas, guards, match, Hole `_`, `hint: Option<&Ty>` top-down)
- **Fio 4** (`Ty::Sum` com payload, `Result::(T, E)`, `Optional::T`, `|` fallback)
- **Fio 5** (`Ty::Struct`, `StructRegistry`, smart constructor infalível,
  `StructConstruct` na TAST, ascription-construção `(a, b)::Pessoa`)

## Estado herdado

O Fio 5 já deixou infraestrutura pronta:

- **`StructRegistry`** em `kata-core` com `StructInfo { name, fields, alias_of }`.
  O campo `alias_of: Option<String>` já existe para distinguir alias de struct
  nativo. Fio 6 estende `StructInfo` com `predicates: Option<Vec<Spanned<Expr>>>`.
- **`Ty::Struct(String)`** já existe. Refined types são `Ty::Struct(name)` —
  a distinção refined vs alias vs struct é feita pelo `StructInfo` no registry,
  não pelo `Ty`. Isto reusa o mecanismo de alias: `alias Float as Altura` já
  prova que `Ty::Struct` com `alias_of` funciona sem variante nova em `Ty`.
- **Smart constructor infalível** — `kata-inference/src/infer/mod.rs` já
  sintetiza `TypedFunction` com body `StructConstruct` para structs com campos.
  Fio 6 adiciona a versão falível: body com guard chain → `Result::Ok/Err`.
- **`TypedExprKind::StructConstruct`** já existe na TAST e o codegen já lowera
  (`arena_alloc` + `store` por campo). Refined types sem campos (só predicado)
  não usam `StructConstruct` — o construtor falível produz `VariantConstruct`
  (`Result::Ok(v)` / `Result::Err(msg)`) via guard chain.
- **Ascription (`expr::Type`)** — já funciona para rebaixamento de literal
  (`42::Float`), confirmação de tipo (`x::Int`), e ascription-construção
  (`(a, b)::Pessoa`). Fio 6 adiciona o 4º modo: ascription-refined
  (`5::PositiveInt` valida predicado em compile-time).
- **`TypedExprKind::TypeAscription { expr, target_ty }`** já existe.
- **`VariantDecl`** na AST tem `name` e `payload: Option<Spanned<TypeExpr>>`.
  Fio 6 adiciona `predicate: Option<Spanned<Expr>>`.
- **`EnumRegistry`** com `VariantInfo { name, payload_ty }`. Fio 6 estende
  `VariantInfo` com `predicate: Option<Spanned<Expr>>`.
- **Guard clauses** — `GuardClause { condition, body }` já existe na AST.
  O typeck já infere guards em lambdas. O construtor falível de refined type
  é um lambda com guard chain — exatamente o que o typeck já sabe fazer.
- **Hole `_`** — já funciona em `Apply` (partial dispatch, desugar para
  lambda). Fio 6 reusa: o predicado `> _ 0` usa `_` como placeholder que o
  typeck substitui pelo literal na avaliação constante.
- **`hint: Option<&Ty>` top-down** — já existe em `infer_expr_hinted`.
  Fio 6 reusa para ret-directed dispatch: o hint de retorno da ascription
  participa da seleção de overload, não apenas da inferência de lambda.
- **`InferCtx`** — já carrega `table`, `enum_registry`, `struct_registry`.
  Fio 6 adiciona `refined_registry: &'a RefinedRegistry`.

O que não existe e este PRD cria:

1. `predicates: Option<Vec<Spanned<Expr>>>` em `StructInfo` (kata-core)
2. `predicate: Option<Spanned<Expr>>` em `VariantDecl` (kata-ast) e
   `VariantInfo` (kata-core)
3. Parser para `data (Base, pred1 pred2 ...) as Name` (refined declaration)
4. Parser para `EnumVariant(pred)` — predicado em variante de enum
5. `RefinedRegistry` em `ResolvedModule` — guarda predicados por tipo refined
6. Smart constructor falível (guard chain sintetizado → `Result::Ok/Err`)
7. Avaliação constante de predicados (typeck local, NÃO comptime)
8. Ascription-refined: `5::PositiveInt` valida predicado, entrega
   `Ty::Struct("PositiveInt")` direto (sem `Result`)
9. Ret-directed dispatch: hint de retorno na ascription seleciona overload
10. Coerção contextual no `|`: fallback literal validado em compile-time
11. Grouped ascription: `((expr))::Type` — barreira vs strip

## Modelo

### Tipos Refinados como `Ty::Struct` com predicados

Um tipo refinado `data (Int, > _ 0) as PositiveInt` é representado como:

```rust
// Ty::Struct("PositiveInt") — mesmo que qualquer newtype/alias
// StructInfo no StructRegistry:
StructInfo {
    name: "PositiveInt",
    fields: [],                    // sem campos — é wrapper de Int
    alias_of: Some("Int"),          // ABI = i64 (mesmo mecanismo de alias)
    predicates: Some(vec![          // ÚNICO acréscimo
        Spanned::new(Expr::Apply {
            callee: Box::new(Spanned::new(Expr::Ident { name: ">".into() }, ...)),
            args: vec![
                Spanned::new(Expr::Hole, ...),       // _ (placeholder)
                Spanned::new(Expr::IntLit { text: "0".into() }, ...),
            ],
        }, ...),
    ]),
}
```

**Por que `Ty::Struct` e não `Ty::Refined`**:

1. **Alias já prova o caminho.** `alias Float as Altura` é `Ty::Struct("Altura")`
   com `alias_of: Some("Float")` e `fields: []`. O codegen já trata isso
   corretamente — usa a ABI do target (`f64`), não heap. Refined é um alias
   com predicados. Se o codegen já sabe desambiguar alias de struct nativo
   via `StructInfo`, também sabe desambiguar refined.
2. **Zero match arms novos.** `Ty` aparece em `match` em ~20 arquivos.
   `Ty::Refined` obrigaria a tocar todos. `Ty::Struct` com `predicates` em
   `StructInfo` não muda nenhum match arm de `Ty`.
3. **Uniformidade nominal.** Em Kata, `Int` é `data Int ()` com `@ffi` —
   não é primitivo. Todo tipo nominal é `Ty::Struct`. `Ty::Refined` bifurca
   essa uniformidade sem ganho semântico — a diferença entre refined e alias
   é a presença de predicados, não uma categoria de tipo diferente.
4. **`TypeShape` já projeta.** Para alias com `fields: []` e
   `alias_of: Some("Float")`, o `TypeShape` projeta para o shape do target.
   Refined seria o mesmo — projeta para o shape de `Int`.

**Codegen**: refined type sem campos usa a ABI do `alias_of`. Se `alias_of`
é `Some("Int")`, a representação é `i64` — não heap, não arena_alloc. O
construtor falível produz `Result::(Ty::Struct("PositiveInt"), Error)` via
`VariantConstruct` (`Ok(v)` / `Err(msg)`) — o `v` interno é `i64`.

### `RefinedRegistry`

O `RefinedRegistry` vive no `ResolvedModule` (kata-resolution), que já
depende de `kata-ast`. Isso permite guardar `Spanned<Expr>` diretamente,
sem duplicar a representação de expressões em `kata-core`.

```rust
// kata-resolution/src/lib.rs
pub struct RefinedRegistry {
    /// refined_type_name → predicados (expressões com Hole como placeholder)
    refined: HashMap<String, RefinedInfo>,
}

pub struct RefinedInfo {
    pub name: String,
    pub base_ty: Ty,           // Int, Float, etc.
    pub predicates: Vec<Spanned<Expr>>,  // > _ 0, <= _ 100, etc.
}
```

O `InferCtx` ganha `refined_registry: &'a RefinedRegistry`.

**Por que no `ResolvedModule` e não no `kata-core`**: `kata-core` não pode
depender de `kata-ast` (leaf crate). `Spanned<Expr>` vive em `kata-ast`.
O `ResolvedModule` já depende de `kata-ast` e já carrega `StructRegistry`
e `EnumRegistry` (de `kata-core`). O `RefinedRegistry` é o único registry
que carrega `Expr` — vive no nível do resolution, não na fundação.

**Alternativa considerada**: `PredicateExpr` serializado em `kata-core`
(`PredicateExpr::IntLit(i64)`, `Apply { op, args }`). Rejeitado porque
duplica a representação de expressões que já existe em `kata-ast`, e o
typeck já sabe avaliar `Expr` — não precisa de uma representação paralela.

### Smart constructor falível

`data (Int, > _ 0) as PositiveInt` sintetiza:

```kata
PositiveInt :: Int => Result::(PositiveInt, Error)
lambda v:
    > v 0: Result::Ok(v)
    otherwise: Result::Err("predicado > _ 0 falhou em PositiveInt")
```

A síntese reusa a maquinaria de guard chain que já existe:

1. **Resolution** registra `PositiveInt` no `RefinedRegistry` com
   `base_ty = Int` e `predicates = [> _ 0]`.
2. **Resolution** registra a assinatura `PositiveInt :: Int => Result`
   no `DispatchTable` (mesmo mecanismo de smart constructor infalível,
   mas o retorno é `Result::(T, Error)`, não `T` direto).
3. **Inference** sintetiza o body: um `TypedFunction` com:
   - Parâmetro: `v: Int`
   - Guards: um guard por predicado. Cada guard substitui `_` por `v`
     na expressão do predicado, avalia como `Boolean`, e se `True`
     retorna `Result::Ok(v)`.
   - `otherwise`: retorna `Result::Err("predicado falhou")`.
4. **Codegen** lowera o guard chain como lowera qualquer lambda com guards
   — não há nada novo no codegen. O `Result::Ok(v)` é `VariantConstruct`
   (já existe), `Result::Err(msg)` é `VariantConstruct` (já existe).

**Múltiplos predicados**: `data (Int, > _ 0, <= _ 100) as Percentage`
sintetiza guard chain com 2 guards (AND lógico — primeiro que falha
aborta):

```kata
Percentage :: Int => Result::(Percentage, Error)
lambda v:
    > v 0: <= v 100: Result::Ok(v)    # guard composto (AND)
    otherwise: Result::Err("predicado falhou em Percentage")
```

### Enum predicado

`enum IMC` com variantes predicadas:

```kata
enum IMC
    Magreza(< _ 18.5)
    Normal(<= _ 25.0)
    Sobrepeso(<= _ 30.0)
    Obesidade
```

O construtor sintetizado despacha para a variante cujo predicado satisfaz:

```kata
IMC :: Float => IMC
lambda x:
    < x 18.5: Magreza(x)
    <= x 25.0: Normal(x)
    <= x 30.0: Sobrepeso(x)
    otherwise: Obesidade(x)
```

**Diferença de refined**: enum predicado retorna `Sum` direto (não `Result`)
— a variante default garante cobertura total. Refined retorna `Result`
porque pode falhar (nenhum predicado satisfeito).

**`VariantDecl`** ganha `predicate: Option<Spanned<Expr>>`:

```rust
pub struct VariantDecl {
    pub name: String,
    pub payload: Option<Spanned<TypeExpr>>,
    /// Predicado da variante. None = variante sem predicado.
    /// `Magreza(< _ 18.5)` → predicate = Some(Apply { >, [Hole, 18.5] })
    /// `Obesidade` → predicate = None (default/fallback)
    pub predicate: Option<Spanned<Expr>>,
}
```

**`VariantInfo`** em `kata-core` ganha `predicate_expr: Option<String>`
(serialização mínima — o predicado como `Expr` vive no `RefinedRegistry`
em kata-resolution; o `EnumRegistry` em kata-core só guarda um marcador
para indicar que a variante tem predicado). Alternativa: o
`RefinedRegistry` também guarda predicados de enum, indexado por
`(enum_name, variant_name)`.

**Decisão**: `RefinedRegistry` guarda tanto refined types quanto enum
predicados. O `EnumRegistry` em `kata-core` não muda — só ganha um método
`has_predicate(enum_name, variant)` que consulta o `RefinedRegistry`.
Mas `EnumRegistry` não pode depender de `RefinedRegistry` (que está em
kata-resolution). Então: o `InferCtx` carrega ambos, e o typeck consulta
o `RefinedRegistry` quando precisa do predicado.

### Ascription-refined (4º modo)

`5::PositiveInt` — o typeck:

1. Vê `TypeAscription { expr: IntLit("5"), ty: Named("PositiveInt") }`.
2. Resolve `PositiveInt` → `Ty::Struct("PositiveInt")`.
3. Consulta `RefinedRegistry`: `PositiveInt` é refined? Sim.
4. Extrai `base_ty = Int`, `predicates = [> _ 0]`.
5. Verifica que `expr` é literal (IntLit). Se não é literal → type error
   ("ascription refined exige literal; use construtor para expr não-literal").
6. **Avaliação constante**: substitui `_` por `5` no predicado `> _ 0`,
   reduz `> 5 0` → `Boolean::True`. Predicado satisfeito.
7. Retorna `TypedExprKind::TypeAscription { expr, target_ty: Ty::Struct("PositiveInt") }`.
   O tipo é `Ty::Struct("PositiveInt")` — direto, sem `Result`.

Se o predicado falha (`(-5)::PositiveInt`):

6. Substitui `_` por `-5`, reduz `> (-5) 0` → `Boolean::False`.
7. **Type error**: `"predicado > _ 0 falhou em PositiveInt para valor -5"`.
   O programa não compila. Não há `Result`, não há runtime.

**Avaliação constante local ao typeck (NÃO comptime)**:

A redução de `> 5 0` → `Boolean::True` é feita por um avaliador constante
minimal no typeck. Ele não usa JIT-and-execute (Fio 12). Ele lida com:

- `IntLit(n)` → valor inteiro
- `FloatLit(n)` → valor float
- `Apply { Ident(op), [literal, literal] }` → despacha `op` (=, <, >, <=, >=)
  com os dois literais, retorna `Boolean::True` ou `Boolean::False`
- `Apply { Ident(op), [Hole, literal] }` → substitui Hole pelo literal
  sendo avaliado, avalia como acima

O avaliador é intencionalmente limitado: só operações de comparação sobre
literais numéricos. Não tenta avaliar chamadas de função, pattern matching,
ou expressões complexas. Se o predicado é muito complexo para o avaliador
constante, o typeck rejeita com "predicado muito complexo para validação
compile-time" — o usuário deve usar o construtor falível (runtime) neste caso.

### Ret-directed dispatch

`(/ 1 3)::Int` — o hint de retorno (`Int`) participa da seleção de overload.

Hoje, o `hint: Option<&Ty>` só propaga para dentro de lambdas (DoD 29).
Fio 6 faz o hint também participar da seleção de overload no dispatch:

1. `TypeAscription { expr: Apply { /, [1, 3] }, ty: Named("Int") }`
2. O typeck propaga `target_ty = Int` como hint para `infer_expr_hinted`
   da expressão interna (já faz isso hoje).
3. `infer_apply` recebe o hint. Hoje ele ignora o hint na seleção de
   overload. Fio 6 faz: se o hint é `Some(ty)`, filtra candidatos cujo
   retorno é compatível com `ty`. Entre os compatíveis, seleciona por
   dominância (scoring dos argumentos).
4. Se `/` tem uma única overload (`Int Int => Rational`), o hint `Int`
   filtra e nenhum candidato sobra → type error (o usuário pediu `Int`
   mas `/` só entrega `Rational`). O hint `Rational` é compatível →
   seleciona a única overload (redundante, mas válido).
5. Se a mesma função tem múltiplas overloads com mesmo shape de args
   mas retorno diferente, o hint desambigua. Exemplo: se `show` tem
   `Int => Text` e `Int => DetailedText`, o hint `Text` seleciona a
   primeira, o hint `DetailedText` seleciona a segunda.

**Mudança no dispatch**: `infer_apply` consulta `hint` ao lado dos
argumentos. O scoring por dominância já existe — a adição é um filtro
pré-dispatch: se `hint` é `Some(ty)`, descarta candidatos cujo `return_type`
não é compatível com `ty`. Se nenhum candidato sobra → type error (o
usuário pediu um retorno que a função não oferece). Se sobram múltiplos →
scoring por dominância entre os remanescentes. Se sobra 1 → seleciona.

**Risco**: o hint pode sobre-restringir. Se o hint é `Int` mas a única
overload disponível retorna `Rational`, o typeck rejeita. Isso é
correto — a ascription é uma afirmação do usuário; se a função não
entrega aquele tipo, é type error.

### Coerção contextual no `|`

`PositiveInt 25 | 0` — o `|` desempacota o payload da variante não-cauda.
Se o payload é um tipo refinado e o fallback é um literal do tipo base,
o compilador valida os predicados do fallback em compile-time.

Hoje o `|` é desugared para `Match` no typeck. Fio 6 adiciona:
ao lowerar `PipeFallback { lhs, rhs }`, se o `lhs` produz
`Result::(RefinedT, Error)` e o `rhs` é um literal do `base_ty` de
`RefinedT`, o typeck valida os predicados do `rhs` como se fosse
`rhs::RefinedT`. Se os predicados passam, o `rhs` é aceito. Se falham,
type error.

### Grouped ascription (`((expr))::Type`)

`((expr))::Type` — grouping duplo cria barreira. O typeck não propaga
o hint de `::Type` para dentro de `((expr))` — o grouping triplo (ou
duplo) é uma barreira explícita.

Hoje, `(expr)::Type` propaga o hint `Type` para dentro de `expr`. Isso
é desejável para `(/ 1 3)::Int` (ret-directed dispatch). Mas pode ser
indesejado se o usuário quer que o `expr` seja inferido independentemente
e a ascription só valida no final.

`((expr))::Type` funciona assim:

- Parser: `((expr))` é `Grouping(Grouping(expr))` — dois níveis de
  grouping.
- Typeck: ao processar `TypeAscription { expr: Grouping(Grouping(inner)) }`,
  o typeck infere `inner` **sem hint** (o grouping duplo é a barreira).
  Depois valida o resultado contra `target_ty` (como confirmação de tipo
  ou rebaixamento, não como ret-directed dispatch).

Isso só faz sentido se o ret-directed dispatch (que propaga hint) já
existe. Sem ret-directed, `(expr)::Type` e `((expr))::Type` são
semanticamente idênticos (o hint não influencia nada). Com ret-directed,
`(expr)::Type` propaga o hint (seleciona overload por retorno) e
`((expr))::Type` não propaga (seleciona overload por argumentos, depois
valida).

## Escopo

### Fase 1: Parser — refined declaration + enum predicado

- `data (Base, pred1 pred2 ...) as Name` — nova forma de `data`
  - **Disambiguação via `as`**: o parser parseia o conteúdo de `()`
    genericamente. Se após `)` encontra `as` → é refined (primeiro
    elemento é `TypeExpr` base, restante são predicados `Expr`). Se
    não encontra `as` → é struct (elementos são `FieldDecl`s). O `as`
    é o marcador explícito — não precisa de lookahead trickery.
  - Dentro dos parênteses do refined: `(Int, > _ 0)` — vírgula separa o
    primeiro elemento (TypeExpr — a base) dos predicados (Exprs). Se há
    múltiplos predicados: `(Int, > _ 0, <= _ 100)` — também separados por
    vírgula. Parser lê: TypeExpr `,` Expr (`,` Expr)* `)` `as` Ident.
  - Produz `Item::DataDecl { name, fields: vec![], directives, refined: Some(RefinedDecl { base_ty, predicates }) }`
  - Alternativa: novo `Item::RefinedDecl` para não sobrecarregar `DataDecl`.
    **Decisão**: estender `DataDecl` com `refined: Option<RefinedDecl>`.
    Mantém o item único, o tipo de declaração é determinado pelo `as`.
- `EnumVariant(pred)` — predicado em variante de enum
  - Parser: após nome da variante, se `(` seguido de expressão (não
    `TypeExpr`), é predicado. Se `(` seguido de `TypeExpr` e `)`, é
    payload (como hoje).
  - Disambiguação: predicado é uma `Expr` (pode ter `>`, `<`, `<=`, `_`).
    Payload é um `TypeExpr` (Named, etc). O parser faz lookahead: se
    o conteúdo de `()` é um operador de comparação seguido de `_` e
    literal, é predicado. Se é um `TypeExpr`, é payload.
  - Alternativa: `VariantDecl` ganha `predicate: Option<Spanned<Expr>>`
    ao lado de `payload: Option<Spanned<TypeExpr>>`.
- `Token::As` já existe (usado em alias). Reusado em refined declaration.
- Verificação: `cargo test -p kata-parser`

### Fase 2: Resolution — RefinedRegistry + registro

- Criar `RefinedRegistry` e `RefinedInfo` em `kata-resolution`
- `ResolvedModule` ganha `refined_registry: RefinedRegistry`
- Pass 0: quando `DataDecl` tem `refined: Some(...)`:
  - Registra no `StructRegistry` com `alias_of: Some(base_ty_name)`,
    `fields: []`, `predicates: Some(...)`
  - Registra no `RefinedRegistry` com `base_ty` e `predicates` (como `Expr`)
  - Registra `Ty::Struct(name)` no `TypeEnv`
- Pass 0: quando `EnumDecl` tem variantes com `predicate: Some(...)`:
  - Registra no `EnumRegistry` normalmente (variantes com payload)
  - Registra predicados no `RefinedRegistry` indexado por
    `(enum_name, variant_name)`
- Pass 1: registra assinatura do construtor falível:
  - Refined: `Name :: BaseTy => Result::(Name, Error)` no `DispatchTable`
  - Enum predicado: `EnumName :: PayloadTy => EnumName` no `DispatchTable`
- `InferCtx` ganha `refined_registry: &'a RefinedRegistry`
- Verificação: `cargo test -p kata-resolution`

### Fase 3: Inference — smart constructor falível + ascription-refined

- Sintetizar `TypedFunction` para construtor falível de refined type:
  - Body: `Lambda` com 1 parâmetro (`v: BaseTy`), guard chain com 1 guard
    por predicado (substitui `_` por `v`, avalia, se `True` → `Ok(v)`),
    `otherwise` → `Err(msg)`
  - O `Ok(v)` é `VariantConstruct { enum_name: "Result", variant: "Ok",
    payload: v, tag: 0 }` (já existe)
  - O `Err(msg)` é `VariantConstruct { enum_name: "Result", variant:
    "Err", payload: TextLit(msg), tag: 1 }` (já existe)
- Sintetizar `TypedFunction` para construtor de enum predicado:
  - Body: `Lambda` com 1 parâmetro (`x: PayloadTy`), guard chain com 1
    guard por variante predicada (substitui `_` por `x`, avalia, se
    `True` → `VariantConstruct(variant, x)`), `otherwise` →
    `VariantConstruct(default, x)`
- Avaliação constante de predicados:
  - Criar `const_eval.rs` em `kata-inference/src/infer/`
  - `const_eval_predicate(pred: &Expr, value: &Expr) -> Option<bool>`
  - Substitui `Hole` por `value`, reduz expressão booleana
  - Suporta: `Apply { Ident(op), [IntLit, IntLit] }` para `=`, `<`, `>`,
    `<=`, `>=` e `Apply { Ident(op), [FloatLit, FloatLit] }` idem
  - Retorna `None` se não consegue avaliar (predicado muito complexo)
- Ascription-refined em `infer_expr_hinted`:
  - Ao processar `TypeAscription { expr, ty }` onde `ty` resolve para
    `Ty::Struct(name)` e `name` está no `RefinedRegistry`:
    - Se `expr` é literal (IntLit/FloatLit): avalia predicados com
      `const_eval_predicate`. Se todos passam → `TypeAscription` com
      `target_ty`. Se algum falha → type error. Se não consegue avaliar
      → type error ("predicado muito complexo para ascription; use
      construtor").
    - Se `expr` não é literal → type error ("ascription refined exige
      literal; use construtor para expr não-literal").
- Verificação: `cargo test -p kata-inference`

### Fase 4: Codegen — construtor falível E2E

- O construtor falível é um `TypedFunction` com guard chain — o codegen
  já lowera guard chains de lambdas. Não há nada novo no codegen.
- `VariantConstruct` para `Ok` e `Err` já é lowerado (Fio 4).
- Testes E2E:
  - `PositiveInt 42 ?` em Action → imprime 42
  - `PositiveInt (-5)` → `Result::Err` → `?` propaga
  - `match (PositiveInt 42) Result::Ok(v): echo!(v)` → imprime 42
  - `IMC 17.0` → produz `Magreza(17.0)` → `match` despacha corretamente
  - `IMC 22.0` → produz `Normal(22.0)`
  - `IMC 35.0` → produz `Obesidade(35.0)`
- Verificação: testes E2E no driver

### Fase 5: Ret-directed dispatch

- `infer_apply` ganha parâmetro `hint: Option<&Ty>` (já recebe via
  `infer_expr_hinted`, mas hoje ignora na seleção de overload)
- Antes do scoring por dominância, se `hint` é `Some(ty)`:
  - Filtra candidatos do `DispatchTable` cujo `return_type` é compatível
    com `ty` (usando `fits_return` que já existe)
  - Se sobra 1 candidato → seleciona diretamente
  - Se sobram múltiplos → scoring por dominância entre os remanescentes
  - Se sobra 0 → type error (nenhuma overload compatível com o hint)
- O ret-directed dispatch só é útil quando a mesma função tem múltiplas
  overloads com mesmo shape de argumentos mas retorno diferente. Se a
  função tem uma única overload, o hint é redundante (mas ainda valida
  compatibilidade).
- Testes E2E:
  - `(/ 1 3)::Rational` → OK (hint compatível com única overload)
  - `(/ 1 3)::Int` → type error (hint incompatível — `/` só entrega
    Rational)
  - `(/ 1 3)` sem hint → despacha normalmente (única overload)
  - Caso com múltiplas overloads de mesmo shape: criar duas overloads
    de uma função de teste com retorno diferente e verificar que o
    hint seleciona a correta
- Verificação: testes E2E + `cargo test -p kata-inference`

### Fase 6: Coerção contextual no `|` + grouped ascription

- Coerção contextual no `|`:
  - Ao desugar `PipeFallback { lhs, rhs }` para `Match`:
    - Se o payload da variante não-cauda é um tipo refinado, o
      fallback é implicitamente tratado como o mesmo refined type
      (não tipo base puro). O predicado do refined é validado no
      fallback via `const_eval_predicate`. Se passa, aceito. Se
      falha, type error.
    - Não exige ascription explícita no fallback — o tipo é inferido
      do payload da variante não-cauda.
  - `|` NÃO é estendido para `Result`. `Err` tem payload, não é cauda
    unitária. Tratamento de `Result` em lambdas usa pattern-clause
    (já implementado desde Fio 2): `lambda Result::Ok(v): ...
    lambda Result::Err(_): ...`
- Grouped ascription `((expr))::Type`:
  - Parser já produz `Grouping(Grouping(inner))` para `((expr))`
  - Typeck: ao processar `TypeAscription { expr: Grouping(g) }` onde
    `g` é `Grouping(inner)`:
    - Infere `inner` **sem hint** (passa `None` em vez de propagar
      `target_ty`)
    - Depois valida o resultado contra `target_ty` (confirmação ou
      rebaixamento, não ret-directed dispatch)
  - Distinção: `(expr)::Type` propaga hint (ret-directed).
    `((expr))::Type` não propaga (barreira).
- Testes E2E:
  - `Optional::Some(5::PositiveInt) | 1` → desempacota 5 (fallback `1`
    validado como PositiveInt, predicado `> _ 0` satisfeito)
  - `Optional::Some(5::PositiveInt) | 0` → type error (predicado falha)
  - `((/ 1 3))::Rational` → OK (grouped: sem ret-directed, `/ 1 3`
    despacha normalmente, confirma Rational)
  - `((/ 1 3))::Int` → type error (grouped: sem ret-directed, `/ 1 3`
    despacha Rational, Rational ≠ Int)
- Verificação: testes E2E

## DoD

1. **`5::PositiveInt` é `PositiveInt` direto.** Ascription refined valida
   predicado em compile-time, entrega `Ty::Struct("PositiveInt")` sem
   `Result`.

2. **`(-5)::PositiveInt` é type error.** Predicado `> _ 0` falha em
   compile-time. O programa não compila.

3. **`PositiveInt 25 ?` desempacota.** Construtor falível retorna
   `Result::Ok(25)`, `?` extrai 25.

4. **`PositiveInt (-5)` retorna `Result::Err`.** Construtor falível
   retorna `Result::Err("predicado > _ 0 falhou")`. `?` propaga erro.

5. **Coerção contextual no `|` com enums de cauda unitária.** O `|`
   só funciona com enums cuja última variante é unitária (cauda sem
   payload). `Result` não é compatível (`Err` tem payload) — `|` não
   é estendido para `Result`. A coerção contextual se aplica quando o
   payload da variante não-cauda é um tipo refinado: o fallback é
   implicitamente tratado como o mesmo refined type (não tipo base
   puro), e o predicado é validado em compile-time. Exemplo:
   `Optional::Some(5::PositiveInt) | 1` — desempacota o `PositiveInt`
   e o fallback `1` é validado como `PositiveInt` (predicado `> _ 0`
   satisfeito). `| 0` seria type error (predicado falha). Não exige
   ascription explícita no fallback — o tipo é inferido do payload.
   **Tratamento de `Result` em lambdas**: usar pattern-clause
   (`lambda Result::Ok(v): ... lambda Result::Err(_): ...`), já
   implementado desde Fio 2. `?` em Actions propaga. `match` é
   alternativa explícita.

6. **`(/ 1 3)::Int` seleciona idiv.** Ret-directed dispatch: o hint
   `Int` filtra overloads de `/` e seleciona `Int Int => Int`.

7. **`(/ 1 3)::Rational` seleciona divisão exata.** Hint `Rational`
   seleciona `Int Int => Rational`.

8. **`(/ 1 3)` sem hint com múltiplas overloads é `AmbiguousDispatch`.**
   Se a mesma função `/` tem duas overloads com `[Int, Int]` mas retorno
   diferente (`Rational` e `Int`), ambas pontuam `exact: 2` — empate
   perfeito. O dispatch retorna `AmbiguousDispatch` (não type error).
   Isso é o comportamento correto do multiple dispatch: o scoring de
   argumentos não consegue distinguir. O ret-directed dispatch é o
   escape — o hint de retorno quebra o empate. Se `/` tem uma única
   overload, despacha normalmente sem ambiguidade.

9. **Enum predicado `IMC(17.0)` despacha para `Magreza`.** O
   construtor sintetizado avalia predicados em runtime e despacha
   para a variante correta.

10. **`IMC(22.0)` despacha para `Normal`.** Segundo predicado
    satisfeito.

11. **`IMC(35.0)` despacha para `Obesidade`.** Nenhum predicado
    satisfeito → fallback (default variant).

12. **`((/ 1 3))::Int` é type error.** Grouped ascription: o grouping
    duplo é barreira, não propaga hint. `/ 1 3` sem hint despacha
    normalmente (única overload → `Rational`). A ascription `::Int`
    valida o resultado contra `Int` — como `Rational ≠ Int` e não há
    rebaixamento de Rational para Int, é type mismatch. Para
    desambiguar via hint, use `(/ 1 3)::Type` (sem grouping duplo).

## Não faz parte deste PRD

- **Interfaces** (`SHOW`, `ITERABLE`, `INDEXABLE`) — Fio 7. `repr` é
  auto-sintetizado em Fio 5, mas a interface `SHOW` formal é Fio 7.
- **Generics** (`Ty::Generic`) — Fio 7.
- **Monomorphization** — Fio 7.
- **Comptime** (`@comptime`) — Fio 12. A avaliação constante de
  predicados é local ao typeck, não usa comptime.
- **`for x in`** — Fio 8.
- **`.N` em coleções** — Fio 8.

## Casos não cobertos

### Refined type com base struct (não primitiva)

`data (Pessoa, > _.idade 18) as Adulto` — predicado acessa campo de
struct. O avaliador constante precisa suportar `DotAccess` em
predicados. **Decisão**: fora do escopo inicial. O avaliador constante
suporta apenas predicados sobre literais numéricos (comparação de
Int/Float). Predicados com field access exigem um avaliador mais
complexo — adiar para quando houver caso de uso real.

### Refined type com múltiplas bases

`data (Int, Float, > _ 0) as Coordenada` — múltiplas bases. Fora do
escopo. Refined type tem uma base única.

### Predicado com chamada de função

`data (Int, is_prime _) as Prime` — predicado chama função definida
pelo usuário. O avaliador constante não suporta chamadas de função
(não é um interpretador de TAST). **Decisão**: o typeck rejeita com
"predicado muito complexo para validação compile-time; use construtor
falível (runtime)". O construtor falível em runtime despacha a função
normalmente.

### Alias de refined

`alias PositiveInt as PositiveNonZero` — o alias herda a semântica
falível. O construtor de `PositiveNonZero` delega para
`PositiveInt`. Já funciona pela mecânica de alias de Fio 5 (alias de
alias delega recursivamente). O `RefinedRegistry` registra
`PositiveNonZero` como refined com os mesmos predicados de
`PositiveInt` (ou o alias aponta para `PositiveInt` no registry).

### Refined type em match pattern

`match x PositiveInt(v): ...` — pattern matching contra refined
type. O `v` dentro do pattern é do tipo base (`Int`), não do refined.
O typeck já suporta `Variant` patterns; refined não é variante, é
struct. **Decisão**: pattern matching contra refined types não é
suportado diretamente. O usuário faz `match x` com guard
`> x 0: ...` ou usa o construtor falível + `Result` pattern.