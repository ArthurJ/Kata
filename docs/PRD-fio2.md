# PRD: Fio 2 — Funções, Lambdas, Guards, Match, Hole

## Objetivo

Estender o pipeline do Fio 1 com funções puras nomeadas (múltiplas cláusulas),
lambdas anônimos, pattern matching com verificação de exaustividade, guards,
hole (`_`), e pipeline (`|>`). TCO delegado ao Cranelift via `tail_pos: bool`.

## Depende de

Fio 1 (pipeline end-to-end, TypeEnv, DispatchTable, Ty::Function já existe).

## Estado herdadado do Fio 1

O Fio 1 já deixou infraestrutura pronta para este fio:

- **Tokens**: `Lambda`, `Match`, `Otherwise`, `With`, `PipeForward`, `Colon`,
  `ThinArrow` já existem em `Token` e são produzidos pelo lexer.
- **`Ty::Function(Vec<Ty>, Box<Ty>)`** já existe em `kata-core` (vazio desde Fio 1).
- **`TypeExpr::Func { params, ret }`** já existe em `kata-ast` (parser já
  reconhece `(A -> B)` em assinaturas).
- **`Item::Sig.body: Option<Spanned<Expr>>`** — campo já existe, sempre `None`
  em Fio 1. Fio 2 popula com cláusulas lambda.
- **`TypedExpr.tail_pos: bool`** e **`TypedExpr.effect: Effect`** — já em cada nó
  da TAST desde Fio 1. `tail_pos` é `true` por padrão; Fio 2 propaga
  corretamente para sub-expressões.
- **`Effect::Puro`** é o único efeito existente. Fio 2 não adiciona efeitos
  novos — todas as construções deste fio são puras.
- **Lexer `λ`**: o caractere unicode `λ` já produz `Token::Lambda`.

## Escopo

### Funções nomeadas (múltiplas cláusulas)

```kata
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat $(- n 1) $(* n acc)
```

Uma assinatura (`Sig`) seguida de zero ou mais cláusulas `lambda <padrões>:
<corpo>`. Zero cláusulas = definição FFI (corpo suprido por `@ffi`, já funciona
em Fio 1). Múltiplas cláusulas = função pura nomeada com dispatch por padrão.

- As cláusulas seguem a assinatura por indentação.
- Cada cláusula começa com `lambda` (ou `λ`), seguido de padrões separados por
  espaço, `:` e o corpo.
- A primeira cláusula cujos padrões encaixam vence. Nenhuma encaixa → runtime
  trap.
- Exaustividade: reusada do `match` (ver abaixo).

### Lambda anônimo (uma cláusula)

```kata
let soma_dez := lambda x: + 10 x
let inc := λ n: + n 1
```

Lambda em posição de expressão é uma cláusula única. Não há múltiplas cláusulas
em lambda anônimo — só após `Sig`.

- Padrões: um ou mais, separados por espaço, antes do `:`.
- O tipo do lambda é `Ty::Function(params, ret)` inferido pelo typeck.

### Guards

```kata
lambda x:
    > x 0: x
    otherwise: - 0 x
```

Guards são condições booleanas que aparecem como cláusulas dentro de um lambda.
Cada guard é `<expr booleana>: <corpo>`. `otherwise` é o guard de fallback
obrigatório quando o typeck não prova exaustividade.

- Guards vivem dentro do corpo de uma cláusula lambda (bloco indentado
  após `:`). Se o body é uma expressão única na mesma linha do `:`, não
  há guards.
- Um guard é um braço: condição à esquerda do `:`, corpo à direita.
- `otherwise` é syntactic sugar para `Boolean::True` (sempre passa).
- O typeck verifica que `otherwise` está presente quando necessário (tipos
  infinitos como `Int`, `Float`, `Text` exigem catch-all; tipos finitos como
  `Boolean` com cobertura total NÃO exigem `otherwise` — ver DoD 15).
- O typeck verifica que a condição do guard é `Boolean`.
- Guards sem `otherwise` em tipo infinito produzem `MissingOtherwise`.

### Match

```kata
match expr
    True: "sim"
    False: "não"
    otherwise: "impossível"
```

`match` avalia um scrutinee e despacha para o primeiro braço cujo padrão
encaixa.

- Sintaxe: `match <expr>` seguido de braços indentados.
- Cada braço: `<pattern>: <corpo>` ou `<guard>: <corpo>`.
- `otherwise` é o braço de fallback (equivale a wildcard `_`).
- Verificação de exaustividade: o typeck verifica que os braços cobrem todas as
  variantes do tipo do scrutinee (para `Sum`) ou exige `otherwise` (para tipos
  infinitos).
- Patterns e guards podem coexistir no mesmo `match`.
- O scrutinee é avaliado uma vez (eager).
- **Variantes sem qualificação**: quando um `enum` está no escopo, suas
  variantes são acessíveis diretamente sem `Enum::`. `True` em vez de
  `Boolean::True`, `False` em vez de `Boolean::False`. O typeck resolve o
  nome da variante procurando no `enum` do tipo do scrutinee. Ambas as formas
  são válidas — `True` e `Boolean::True` são equivalentes.

### Patterns

Patterns são reusados integralmente entre `match` e cláusulas lambda.

```rust
pub enum Pattern {
    /// `x` — liga o valor ao nome
    Ident(String),
    /// `_` — wildcard, aceita qualquer valor
    Wildcard,
    /// `42`, `"texto"`, `3.14` — literal exato
    Literal(Spanned<Expr>),
    /// `Boolean::True`, `Result::Ok` — variante de enum
    Variant { enum_name: String, variant: String },
    /// `(a, b, c)` — tupla
    Tuple(Vec<Spanned<Pattern>>),
    /// `[h : t]` — cons (cabeça : cauda). `[]` para lista vazia.
    /// Fio 2 reconhece a sintaxe; Fio 8 (List) dá semântica de runtime.
    /// Em Fio 2, pattern Cons/Nil só funciona se List existir (não existe
    /// ainda — stub que produz erro limpo).
    Cons { head: Box<Spanned<Pattern>>, tail: Box<Spanned<Pattern>> },
}
```

Em Fio 2, os patterns suportados em runtime são: `Ident`, `Wildcard`,
`Literal`, `Variant`, `Tuple`. `Cons` é reconhecido pelo parser mas o typeck
produz erro (List é Fio 8). `Tuple` funciona porque `Ty::Tuple` é antecipado
para Fio 2 (sem field access — só tipo estrutural para patterns).

### Hole (`_`)

```kata
let soma_dez := + 10 _        # gera closure de aridade 1
let sub_dez := - _ 10          # hole na primeira posição
let soma_dez_e_cinco := + _ _  # dois holes — mesmo argumento? NÃO — ver abaixo
```

Hole é currying explícito. `+ 10 _` vira `lambda x: + 10 x`.

- `_` em posição de argumento de `Apply` congela a aplicação, gerando um lambda
  que aguarda o argumento faltante.
- Múltiplos holes: cada `_` é um parâmetro distinto. `+ _ _` vira
  `lambda a b: + a b` (aridade 2), NÃO `lambda a: + a a`.
- `_` em posição de pattern é wildcard (semântica diferente — aceita qualquer
  valor sem ligar nome).
- O desugar é total no typeck: a TAST nunca contém `Hole`. O codegen não
  precisa de mudanças para Hole (vira Lambda normal).

### Pipeline `|>`

```kata
5 |> + 10 _           # 15 — desugars para + 10 5
5 |> + _ 10           # 15 — desugars para + 5 10
5 |> + 1 _ |> * 2 _   # 12 — left-assoc: (* 2 (+ 1 5))
```

- Precedência: mais baixa que aplicação de função. Associativo à esquerda.
- Com Hole: o typeck substitui o `_` pela AST da esquerda.
- Sem Hole: o resultado da esquerda é injetado como primeiro argumento da
  função à direita.
- O desugar é total no typeck: a TAST nunca contém `Pipe`. O codegen não
  precisa de mudanças para Pipe.

### `with` block

```kata
lambda x:
    > x 0: x
    otherwise: - 0 x
    with
        y := + x 1
```

`with` é um bloco de computações prévias nomeadas que aparece **depois dos
guards** no fim da cláusula lambda (como `where` em Haskell). Os bindings do
`with` são visíveis em **todos os guards da cláusula**, mesmo sendo escritos
depois — a ordem é visual (legibilidade), a semântica é que os bindings são
avaliados antes dos guards e estão disponíveis em todo o escopo da cláusula.

- Sintaxe: `with` seguido de bindings indentados (`nome := expr`, sem keyword
  `let`). A ausência de `let` é visual — distingue o bloco `with` do corpo
  principal da cláusula.
- Os bindings são avaliados antes dos guards, em ordem top-down.
- Escopo: bindings do `with` são visíveis em todos os guards da cláusula
  (não apenas nos que vêm depois — `with` é pós-escrito mas pré-avaliado,
  como `where` em Haskell).
- Em Fio 2, `with` também é usado para restrições de genéricos (placeholder —
  genéricos são Fio 7; o parser reconhece, o typeck ignora as restrições).
- Semântica: açúcar sintático para `let` chain no escopo da cláusula. O
  typeck desugars `with` para `let` bindings antes de processar os guards.
- Os bindings do `with` são imutáveis (mesma semântica de `let`).

## Crates Afetadas

```
kata-core/           Novo: Ty::Tuple (antecipado de Fio 5), EnumRegistry
                    (catálogo de variantes por enum)
kata-ast/           Novos: Expr::Lambda, Expr::Match, Expr::Hole, Expr::Pipe,
                    Pattern enum, LambdaClause struct, GuardClause struct,
                    MatchArm struct, WithBinding struct
                    Modificado: Item::Sig.body muda de
                    Option<Spanned<Expr>> para Option<Vec<Spanned<LambdaClause>>>
kata-lexer/         Nenhuma mudança necessária (todos os tokens já existem)
kata-parser/        Novo: parse_lambda, parse_match, parse_pattern,
                    parse_guard_clauses, parse_with, parse_pipe
                    Modificado: parse_sig (parsear cláusulas lambda após Sig),
                    parse_expr_atom (reconhecer lambda, match, hole),
                    can_start_expr (adicionar Lambda, Match),
                    parse_expr (reconhecer |> infixo)
kata-resolution/    Modificado: Sig com body=Some(clauses) → registrar função
                    no DispatchTable como overload com corpo (não-FFI)
                    Novo: popular EnumRegistry a partir de EnumDecl
kata-inference/     Novo: infer_lambda, infer_match, infer_pattern,
                    check_exhaustiveness, desugar_hole, desugar_pipe,
                    propagate tail_pos para TCO
                    Renomear: TypedExprKind::Apply → Closure (com campos
                    captures: Vec<CaptureInfo> = vazio, escapes: bool = false
                    em Fio 2; preenchidos em Fio 9)
kata-codegen/       Novo: lower_lambda (definição de função Cranelift),
                    lower_match (branch chain com brif),
                    lower_closure_call (call direto),
                    TCO via tail_pos
                    Modificado: lower_apply → lower_closure
kata-rt/            Nada novo (lambdas são código nativo puro, sem runtime)
```

## Maquinaria de Tipos Construída

### kata-ast

#### Novos variants de `Expr`

```rust
/// `lambda <padrões>: <corpo>` — lambda anônimo (cláusula única).
///
/// Se `guards` é vazio: `body` é a expressão única após `:`.
///   `lambda x: + x 1`
///
/// Se `guards` é não-vazio: o corpo é um bloco indentado de guard clauses.
///   `lambda x:`
///       `> x 0: x`
///       `otherwise: - 0 x`
///   Neste caso, `body` é ignorado (ou pode ser usado como fallback final).
Expr::Lambda {
    patterns: Vec<Spanned<Pattern>>,
    body: Box<Spanned<Expr>>,
    /// Guards opcionais dentro do corpo. Se não-vazio, o corpo é
    /// uma sequência de guard clauses (bloco indentado após `:`).
    guards: Vec<GuardClause>,
    /// with block opcional (bindings prévios)
    with_bindings: Vec<WithBinding>,
},

/// `match <scrutinee>` com braços indentados.
Expr::Match {
    scrutinee: Box<Spanned<Expr>>,
    arms: Vec<MatchArm>,
},

/// `_` em posição de argumento — hole para currying.
/// O parser produz `Expr::Hole` quando encontra `Ident("_")` em posição
/// de argumento de `Apply`. Em posição de pattern, o parser produz
/// `Pattern::Wildcard` — a disambiguação é no parser, não no typeck.
/// Desugared pelo typeck em Lambda. Nunca chega à TAST.
Expr::Hole,

/// `lhs |> rhs` — pipeline.
/// Desugared pelo typeck. Nunca chega à TAST.
Expr::Pipe {
    lhs: Box<Spanned<Expr>>,
    rhs: Box<Spanned<Expr>>,
},
```

**Nota sobre `Hole` vs `Ident("_")`**: o parser distingue por contexto. `_` em
posição de argumento de `Apply` → `Expr::Hole`. `_` em posição de pattern →
`Pattern::Wildcard`. A disambiguação acontece no parser (que sabe se está
parseando um argumento ou um pattern), não no typeck. Isto é diferente de `$`
(que é `Ident("$")` até o typeck) porque `$` é genuinamente ambíguo (spread vs
standalone exige contexto de tipos), enquanto `_` em args é unambiguamente
hole.

#### `Pattern` enum

```rust
pub enum Pattern {
    Ident(String),
    Wildcard,
    Literal(Spanned<Expr>),
    Variant { enum_name: String, variant: String },
    Tuple(Vec<Spanned<Pattern>>),
    Cons { head: Box<Spanned<Pattern>>, tail: Box<Spanned<Pattern>> },
}
```

#### Estruturas auxiliares

```rust
/// Uma cláusula lambda após uma assinatura.
pub struct LambdaClause {
    pub patterns: Vec<Spanned<Pattern>>,
    pub body: Spanned<Expr>,
    pub guards: Vec<GuardClause>,
    pub with_bindings: Vec<WithBinding>,
}

/// Um guard: `condição: corpo` ou `otherwise: corpo`.
pub struct GuardClause {
    /// None = `otherwise` (sempre passa)
    pub condition: Option<Spanned<Expr>>,
    pub body: Spanned<Expr>,
}

/// Um braço de match: `pattern: corpo` ou `guard: corpo`.
pub struct MatchArm {
    pub pattern: Option<Spanned<Pattern>>,  // None = otherwise
    pub guard: Option<Spanned<Expr>>,        // guard opcional após pattern
    pub body: Spanned<Expr>,
}

/// Binding de `with` block.
pub struct WithBinding {
    pub name: String,
    pub value: Spanned<Expr>,
}
```

#### Modificação em `Item::Sig`

```rust
pub enum Item {
    Sig {
        name: String,
        params: Vec<Spanned<TypeExpr>>,
        ret: Spanned<TypeExpr>,
        directives: Vec<Directive>,
        // Fio 1: sempre None (FFI)
        // Fio 2: Some(clauses) = função pura com corpo Kata
        body: Option<Vec<Spanned<LambdaClause>>>,
    },
    // ...
}
```

**Mudança de tipo**: `body` muda de `Option<Spanned<Expr>>` para
`Option<Vec<Spanned<LambdaClause>>>`. Em Fio 1, `body` era sempre `None` (FFI
sem corpo Kata). A mudança de tipo é breaking mas não requer migração de
dados — todos os `body` existentes são `None`.

**Motivo para `Vec<LambdaClause>` e não `Spanned<Expr>`**: uma função nomeada é
formada por múltiplas cláusulas lambda. Cada cláusula é um par
padrões→corpo. Representar como um único `Expr` exigiria um novo variant
`Expr::MultiLambda` que não faz sentido fora de `Sig`. As cláusulas são uma
propriedade da definição da função nomeada, não uma expressão genérica.

### kata-core

`Ty::Function` já existe. `TypeEnv` já suporta escopos aninhados via
`push_scope()`.

**Antecipação de `Ty::Tuple`**: Fio 2 adiciona `Ty::Tuple(Vec<Ty>)` ao enum
`Ty` (planejado para Fio 5 no roadmap, mas antecipado porque tuple patterns
precisam do tipo para verificação). Sem struct fields, sem `.N` — só o tipo
estrutural para suportar patterns. Fio 5 trará field access e `.N`.

```rust
pub enum Ty {
    Prim(PrimTy),
    Unit,
    Struct(String),
    Sum(String),
    Function(Vec<Ty>, Box<Ty>),
    /// Antecipado de Fio 5. Sem field access, sem .N — só tipo estrutural
    /// para suportar tuple patterns em match/lambda.
    Tuple(Vec<Ty>),
    InferVar(u32),
}
```

**Catálogo de variantes de enum**: Fio 1 registra `enum Boolean` como
`Ty::Sum("Boolean")` no TypeEnv, mas não cataloga as variantes individuais.
Fio 2 precisa saber que `True` e `False` são variantes de `Boolean` para:

1. Resolver `True` (sem qualificação) em pattern como `Variant`, não `Ident`.
2. Verificar exaustividade de match em `Sum`.

```rust
/// Catálogo de variantes por enum.
/// Populado no resolution (Pass 0) a partir de `EnumDecl`.
pub struct EnumRegistry {
    /// enum_name → lista de nomes de variantes
    variants: HashMap<String, Vec<String>>,
}

impl EnumRegistry {
    pub fn new() -> Self { ... }
    /// Registra um enum com suas variantes.
    pub fn register(&mut self, enum_name: &str, variants: Vec<String>) { ... }
    /// Verifica se um nome é variante de um enum.
    pub fn is_variant(&self, enum_name: &str, variant: &str) -> bool { ... }
    /// Lista as variantes de um enum (para verificação de exaustividade).
    pub fn variants_of(&self, enum_name: &str) -> &[String] { ... }
}
```

`EnumRegistry` é populado no `kata-resolution` (Pass 0) a partir de
`Item::EnumDecl` e preservado no `ResolvedModule`. O `kata-inference` consome
para resolver patterns desqualificados e verificar exaustividade.

### `TypedExprKind::Closure` (renomeação de `Apply`)

O manual (§17) especifica que "na TAST, toda chamada de função é
`TExpr::Closure`". Fio 2 renomeia `TypedExprKind::Apply` para
`TypedExprKind::Closure` e adiciona campos que serão preenchidos em fios
posteriores:

```rust
/// Chamada de função na TAST. Substitui `Apply` de Fio 1.
///
/// Em Fio 2: `captures` é sempre vazio (sem captura léxica — Fio 9),
/// `escapes` é sempre `false` (sem escape analysis — Fio 9).
/// A renomeação prepara a TAST para Fio 9 sem retrofit.
Closure {
    callee: Box<Spanned<TypedExpr>>,
    args: Vec<Spanned<TypedExpr>>,
    /// Símbolo FFI resolvido pelo DispatchTable.
    /// `None` para funções Kata puras (corpo no próprio módulo).
    ffi_symbol: Option<String>,
    /// Variáveis capturadas do escopo externo (Fio 9).
    /// Sempre vazio em Fio 2.
    captures: Vec<CaptureInfo>,
    /// Se a closure escapa → alocação heap/Arc (Fio 9).
    /// Sempre `false` em Fio 2.
    escapes: bool,
},
```

`CaptureInfo` é definida em Fio 2 como struct placeholder:

```rust
/// Informação sobre uma variável capturada por uma closure.
/// Preenchida em Fio 9 (escape analysis). Placeholder em Fio 2.
#[derive(Debug, Clone)]
pub struct CaptureInfo {
    pub name: String,
    pub ty: Ty,
    /// Stack ou Heap (Fio 9). Sempre Stack em Fio 2.
    pub storage: CaptureStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStorage {
    Stack,
    Heap,
}
```

**Impacto da renomeação**: todos os usos de `TypedExprKind::Apply` em
`infer.rs` e `lowering.rs` passam a usar `TypedExprKind::Closure`. O campo
`ffi_symbol` é preservado. Os campos `captures` e `escapes` são inicializados
vazio/false.

### kata-resolution

`Sig` com `body = Some(clauses)` registra uma função não-FFI no
`DispatchTable`:

- `ffi_symbol = None` (corpo é Kata, não C).
- `is_action = false` (função pura).
- A assinatura de tipos vem dos `params` e `ret` do `Sig` (já parseados em
  Fio 1).
- O corpo (cláusulas) é preservado no `ResolvedModule` para o inference
  processar.

`EnumDecl` popula o `EnumRegistry`:

- `enum Boolean { True, False }` → `registry.register("Boolean", ["True",
  "False"])`.
- O `EnumRegistry` é preservado no `ResolvedModule` para o inference consumir.

`ResolvedModule` ganha um campo:

```rust
pub struct ResolvedModule {
    pub type_env: TypeEnv,
    pub signatures: Vec<Signature>,
    /// Catálogo de variantes por enum (Fio 2).
    pub enum_registry: EnumRegistry,
}
```

### kata-inference

#### Novos variants de `TypedExprKind`

```rust
/// Lambda — função pura com corpo Kata.
/// Pode ser anônimo (em posição de expressão) ou nomeado (cláusulas de Sig).
Lambda {
    /// Nome da função no JITModule (para call direto).
    /// None para lambda anônimo ainda não compilado como função separada.
    func_name: Option<String>,
    /// Tipos dos parâmetros (da assinatura ou inferidos dos padrões).
    param_types: Vec<Ty>,
    /// Tipo de retorno.
    ret_ty: Ty,
    /// Cláusulas (padrões + corpo). 1 cláusula = lambda anônimo.
    /// Múltiplas = função nomeada.
    clauses: Vec<TypedLambdaClause>,
},

/// Match — pattern matching com verificação de exaustividade.
Match {
    scrutinee: Box<Spanned<TypedExpr>>,
    arms: Vec<TypedMatchArm>,
},
```

#### Lambda inference

`infer_lambda` recebe `Vec<LambdaClause>` + tipos da assinatura e produz:

1. Um `TypedExpr` com `ty = Ty::Function(params, ret)`.
2. Para cada cláusula: cria escopo filho no `TypeEnv`, liga padrões aos
   parâmetros, infere o corpo.
3. Verifica que todas as cláusulas retornam o mesmo tipo (o `ret` da
   assinatura).
4. Verifica exaustividade (reusa `check_exhaustiveness` do match).
5. Verifica sobreposição (`RedundantClause` se cláusula B é sombreada por A).

#### Closure inference (renomeação de Apply)

`infer_expr` para `Expr::Apply` muda em Fio 2. Em Fio 1, o callee era sempre
um nome no DispatchTable. Em Fio 2, há dois caminhos:

1. **Callee é nome no DispatchTable**: `table.resolve(name, arg_types)`
   retorna overload com `ffi_symbol` (FFI) ou `func_name` (função Kata).
   Produz `TypedExprKind::Closure { ffi_symbol, ... }`.

2. **Callee é variável no TypeEnv com `Ty::Function`**: `env.lookup(name)`
   retorna `Ty::Function(params, ret)`. O typeck verifica que os tipos dos
   argumentos batem com `params`. Produz `TypedExprKind::Closure {
   ffi_symbol: None, ... }` — o codegen usa `call_indirect` com o function
   pointer da variável.

A disambiguação: o typeck tenta primeiro o DispatchTable. Se não encontra,
tenta o TypeEnv. Se não encontra em nenhum, `UnboundName`. Se encontra em
ambos (nome de função também é visível como variável), o DispatchTable vence
(call direto é mais eficiente que call_indirect).

**Importante**: funções Kata nomeadas (`fat`) são registradas no TypeEnv como
`Ty::Function` pelo resolution. Isto permite atribuir `let g := fat` e
chamar `g` via `call_indirect`.

#### Match inference

`infer_match` recebe o scrutinee tipado e os braços:

1. Infere o tipo do scrutinee.
2. Para cada braço: cria escopo filho, liga pattern bindings, infere body.
3. Verifica que todos os braços retornam o mesmo tipo.
4. Verifica exaustividade.
5. Marca `tail_pos` no body de cada braço se o `match` está em tail position.

#### Pattern checking

`check_pattern(pattern, scrutinee_ty, env)`:

- `Ident`: se o nome é uma variante do `enum` do scrutinee (ex: `True` quando
  scrutinee é `Ty::Sum("Boolean")`), resolve como `Variant`. Senão, liga o
  nome ao tipo do scrutinee no escopo. O typeck precisa consultar o `enum`
  para saber quais nomes são variantes.
- `Wildcard`: aceita qualquer tipo.
- `Literal`: verifica que o literal é do mesmo tipo que o scrutinee.
- `Variant`: verifica que o enum existe e a variante existe. Para Fio 2,
  variantes unitárias (Boolean) — payload vem em Fio 4. Aceita tanto
  `Boolean::True` (qualificado) quanto a forma desqualificada se o parser
  produzir `Variant` (mas o parser produz `Ident("True")` e o typeck resolve).
- `Tuple`: verifica cada elemento contra o tipo correspondente. Requer
  `Ty::Tuple` — antecipado para Fio 2 (sem field access, sem `.N`). O typeck
  infere `Ty::Tuple` para `Expr::Tuple` e verifica cada sub-pattern contra
  cada sub-tipo.
- `Cons`: stub — produz erro "List patterns são Fio 8".

**Resolução de variantes sem qualificação**: o parser produz
`Pattern::Ident("True")` para `True` em posição de pattern. O typeck, ao
verificar o pattern contra `Ty::Sum("Boolean")`, consulta as variantes do
enum e descobre que `True` é uma variante. Converte para
`Pattern::Variant { enum_name: "Boolean", variant: "True" }`. Se o nome não
é variante do enum, trata como `Ident` (binding). Isto requer que o `enum`
esteja registrado no `TypeEnv` com suas variantes — em Fio 1 o `enum` é
registrado como `Ty::Sum`, mas as variantes individuais não são catalogadas.
Fio 2 precisa de um catálogo de variantes por enum (ver maquinaria abaixo).

#### Exaustividade

`check_exhaustiveness(arms, scrutinee_ty)`:

- `Ty::Sum(name)`: coleta variantes cobertas pelos braços. Se todas as
  variantes estão cobertas → exaustivo. Se não → exige `otherwise` ou
  wildcard. Retorna `NonExhaustiveMatch` se faltam variantes e não há
  fallback.
- `Ty::Prim(_)`, `Ty::Unit`, outros: tipos infinitos → exige `otherwise` ou
  wildcard. Retorna `MissingOtherwise` se não há fallback.
- `Ty::Function`, `Ty::InferVar`: não faz sentido fazer match → type error.

#### Hole desugar

`desugar_hole(expr)`:

- Percorre a AST procurando `Expr::Hole` em posição de argumento de `Apply`.
- Conta holes. Se 1 hole: gera `Expr::Lambda` com 1 parâmetro.
- Se N holes: gera `Expr::Lambda` com N parâmetros (cada hole vira um
  parâmetro distinto).
- Substitui cada `Hole` pelo `Ident` do parâmetro correspondente.
- O desugar acontece antes do type-check do corpo da aplicação.
- Importante: `Pattern::Wildcard` em posição de pattern NÃO é desugared — é
  wildcard, aceita qualquer valor sem ligar nome. A disambiguação já foi
  feita no parser (produz tipos diferentes de nó).

#### Pipe desugar

`desugar_pipe(expr)`:

- `lhs |> rhs`:
  - Se `rhs` é `Apply { callee, args }` com `Hole` em algum arg: substitui
    o `Hole` por `lhs`.
  - Se `rhs` é `Apply { callee, args }` sem `Hole`: injeta `lhs` como
    primeiro argumento.
  - Se `rhs` é `Ident` (função nua): vira `Apply { callee: rhs, args: [lhs] }`.
- Left-assoc: `a |> b |> c` = `(a |> b) |> c`.
- O desugar é total — `Pipe` nunca chega à TAST.

#### `tail_pos` propagação para TCO

- Entry point: `tail_pos = true` (já em Fio 1).
- Última expressão de um lambda body: `tail_pos = true` se o lambda está em
  tail position.
- Body de cada braço de `match` em tail position: `tail_pos = true`.
- Sub-expressões de `Let` value: `tail_pos = false`.
- Argumentos de `Apply`: `tail_pos = false`.
- A chamada recursiva em tail position é marcada para o Cranelift otimizar
  (TCO).

### kata-codegen

#### Lower lambda (definição de função)

Cada lambda com corpo Kata (não-FFI) vira uma função Cranelift separada:

1. Declara uma nova `Function` no `JITModule` com assinatura correspondente
   aos tipos dos parâmetros e retorno.
2. Cria entry block com `block_params` para os parâmetros.
3. Lowera o corpo (pattern matching das cláusulas → branch chain).
4. Para múltiplas cláusulas: emite branch chain — testa pattern da cláusula 1,
   se encaixa jump para block do corpo 1, senão testa cláusula 2, etc.
5. Retorna o `FuncId` (ou ponteiro) como valor na função caller.

#### Lower match

`lower_match(scrutinee, arms, ctx)`:

1. Lowera o scrutinee → `Value`.
2. Para cada braço:
   a. Se pattern é `Variant` (Boolean::True/False): emite `brif` comparando
      o valor.
   b. Se pattern é `Literal`: emite `brif` comparando com o literal.
   c. Se pattern é `Ident`/`Wildcard`: jump incondicional para o block do
      corpo, def_var com o valor do scrutinee.
3. Cria um block por braço + um block de continuação.
4. Cada block de corpo lowera o body e jump para o block de continuação com
   o resultado.
5. O block de continuação recebe o resultado via `block_param`.

#### Lower lambda call

Dois caminhos de chamada no codegen:

| Callee | Resolução | Codegen |
|---|---|---|
| Nome no DispatchTable (FFI ou Kata) | `table.resolve(name, arg_types)` | `call` direto para `FuncId` |
| Variável com `Ty::Function` | `env.lookup(name)` → `Ty::Function` | `call_indirect` com function pointer da variável |

**Call direto** (Fio 1 + funções Kata nomeadas): callee é um nome no
DispatchTable. O codegen já tem o `FuncId` da função compilada. Emite `call`
para o `FuncRef`.

**Call indireto** (lambdas como valores): callee é um `Ident` que resolve no
TypeEnv para `Ty::Function`. O valor da variável é um function pointer (i64 ou
`pointer_type` do Cranelift). O codegen emite `call_indirect` com a
assinatura correspondente aos `param_types` e `ret_ty` do `Ty::Function`.

```kata
let soma_dez := + 10 _       # soma_dez é Ty::Function([Int], Int)
soma_dez 5                    # callee é Ident("soma_dez"), resolve no TypeEnv
                              # codegen: call_indirect com function pointer
```

- Lambda anônimo (`let f := lambda x: + x 1`): o codegen compila o lambda como
  função separada no JITModule, obtém o `FuncId`, e armazena o ponteiro da
  função como valor da variável `f`. Chamadas subsequentes a `f` usam
  `call_indirect`.
- Função nomeada (`fat`): se chamada pelo nome (`fat 5 1`), o typeck resolve
  no DispatchTable e o codegen faz `call` direto. Se atribuída a variável
  (` let g := fat`), `g` carrega o function pointer e chamadas a `g` usam
  `call_indirect`.
- `call_indirect` com `CaptureBox` (Arc, captures) é Fio 9. Fio 2 só tem
  function pointer nu — sem captura, sem Arc.

#### TCO via `tail_pos`

- Quando `Apply` tem `tail_pos = true` e o callee é uma função Kata (não FFI):
  o Cranelift pode otimizar como tail call. O codegen emite `call` normal;
  o Cranelift decide TCO na pass de otimização.
- Para Fio 2, o TCO é delegado ao Cranelift — não há pass próprio. O
  `tail_pos: bool` na TAST é o marcador que o codegen repassa como hint.

## Sintaxe

### Tokens (lexer)

O lexer já produz todos os tokens necessários para Fio 2:

- `Lambda` (keyword `lambda` e unicode `λ`) — já implementado.
- `Match` (keyword `match`) — já no enum `Token`.
- `Otherwise` (keyword `otherwise`) — já no enum `Token`.
- `With` (keyword `with`) — já no enum `Token`.
- `PipeForward` (`|>`) — já no enum `Token`.
- `Colon` (`:`) — já no enum `Token`.
- `ThinArrow` (`->`) — já no enum `Token`.

**`_` (Hole/Wildcard)**: `_` é produzido pelo lexer como `Ident("_")`. O
parser distingue por contexto: em posição de argumento de `Apply` →
`Expr::Hole`; em posição de pattern → `Pattern::Wildcard`. A disambiguação é
no parser (que sabe se está parseando um argumento ou um pattern), não no
typeck.

### Gramática (extensões)

```
item        ::= sig lambda_clause*
              | ...

sig         ::= ident '::' type_expr+ '=>' type_expr   -- já existe

lambda_clause ::= 'lambda' pattern+ ':' body
                 guard_clause* ('with' with_binding+)?

guard_clause ::= expr ':' body            -- guard com condição
               | 'otherwise' ':' body     -- fallback

body        ::= expr                       -- expressão única
              | INDENT stmt+ DEDENT        -- bloco indentado (guards)

match       ::= 'match' expr INDENT match_arm+ DEDENT

match_arm   ::= pattern ':' body
              | 'otherwise' ':' body
              | pattern 'if' guard ':' body   -- guard opcional após pattern

pattern     ::= ident                       -- Ident
              | '_'                         -- Wildcard
              | literal                     -- Literal
              | enum '::' variant           -- Variant
              | '(' pattern+ ')'            -- Tuple
              | '[' pattern ':' pattern ']' -- Cons (stub em Fio 2)
              | '[' ']'                     -- Nil (stub em Fio 2)

expr        ::= ... (existente)
              | 'lambda' pattern+ ':' body  -- lambda anônimo
              | 'match' expr INDENT arm+ DEDENT
              | '_'                         -- Hole (em posição de argumento)
              | expr '|>' expr              -- pipeline

with_binding ::= ident ':=' expr            -- binding sem keyword 'let'
```

### Precedência

```
|>   — mais baixa que aplicação de função, left-assoc
:    — separa pattern/guard do corpo
::   — ascription de tipo (já existe)
```

Tudo else é aplicação prefixa greedy (já existe).

## Exemplos

### Fatorial recursivo (TCO via Cranelift)

```kata
# examples/fatorial.kata
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat (- n 1) (* n acc)

fat 5 1
```

DoD: executa sem stack overflow. Resultado: `120`.

### Fatorial com lambda anônimo

```kata
# examples/lambda_anon.kata
let fat := lambda n:
    match n
        0: 1
        otherwise: * n (fat (- n 1))

fat 5
```

DoD: resultado `120`.

### Guards

```kata
# examples/guards.kata
abs :: Int => Int
lambda x:
    > x 0: x
    otherwise: - 0 x

abs (-5)
```

DoD: resultado `5`.

### Match em Boolean

```kata
# examples/match_boolean.kata
match = 1 1
    True: "igual"
    False: "diferente"
```

DoD: resultado `"igual"`. Variantes sem qualificação — `True` e `False`
resolvidos pelo typeck como variantes de `Boolean`.

### Hole (currying)

```kata
# examples/hole.kata
let soma_dez := + 10 _
soma_dez 5
```

DoD: resultado `15`.

### Lambda como valor (call_indirect)

```kata
# examples/lambda_value.kata
let inc := lambda x: + x 1
let g := inc
g 41
```

DoD: resultado `42`. `inc` e `g` são `Ty::Function([Int], Int)`. `g 41`
resolve `g` no TypeEnv (não no DispatchTable) e o codegen emite
`call_indirect`.

### Pipeline

```kata
# examples/pipeline.kata
5 |> + 1 _ |> * 2 _
```

DoD: resultado `12`.

### with block

```kata
# examples/with.kata
classify :: Int => Text
lambda x:
    > doubled 10: "grande"
    otherwise: "pequeno"
    with
        doubled := * x 2

classify 3
```

DoD: resultado `"pequeno"` (3*2=6 < 10).

### Match exaustivo sem otherwise (Boolean)

```kata
# examples/exhaustive.kata
match Boolean::True
    True: 1
    False: 0
```

DoD: resultado `1`. Sem `otherwise` — exaustividade provada pelo typeck (as
duas variantes de Boolean estão cobertas). Notar que o scrutinee usa
`Boolean::True` (qualificado) mas os patterns usam `True`/`False`
(desqualificado) — ambas as formas são válidas.

### Erro: match não-exaustivo

```kata
# examples/non_exhaustive.kata
match Boolean::True
    True: 1
```

DoD: erro de compilação `NonExhaustiveMatch` — falta `False` ou `otherwise`.

## Prelude

O prelude não muda em Fio 2. Os operadores `+`, `-`, `*`, `/`, `=`, `<`, `>`
já estão definidos via `@ffi` em Fio 1. Fio 2 adiciona a capacidade de definir
funções com corpo Kata, mas o prelude continua usando `@ffi` para tudo.

## Definition of Done

1. `kata run examples/fatorial.kata` imprime `120` (fatorial recursivo com
   TCO).
2. `kata run examples/guards.kata` imprime `5` (guards com `otherwise`).
3. `kata run examples/match_boolean.kata` imprime `"igual"` (match exaustivo
   em Boolean).
4. `kata run examples/hole.kata` imprime `15` (currying via Hole).
5. `kata run examples/pipeline.kata` imprime `12` (pipeline `|>`).
6. `kata run examples/with.kata` imprime `"pequeno"` (`with` block).
7. Match não-exaustivo em Boolean produz `NonExhaustiveMatch` (erro
   compile-time, não runtime trap).
8. `let soma_dez := + 10 _` gera closure de aridade 1. `soma_dez 5` imprime
   `15`.
9. Fatorial recursivo executa sem stack overflow (TCO via Cranelift com
   `tail_pos`).
10. `|>` é total no typeck — TAST nunca contém `Pipe`.
11. `Expr::Hole` é total no typeck — TAST nunca contém `Hole` (vira Lambda).
12. Cláusulas lambda com sobreposição produzem `RedundantClause`.
13. Guards sem `otherwise` em tipo infinito (Int) produzem
    `MissingOtherwise`.
14. `match` em tipo finito (Boolean) sem cobertura total e sem `otherwise`
    produz `NonExhaustiveMatch`.
15. `match` em tipo finito (Boolean) com cobertura total (ambas variantes)
    não exige `otherwise`.
16. Lambda anônimo tem aridade igual ao número de padrões antes do `:`.
17. Múltiplos holes (`+ _ _`) geram lambda de aridade 2 (parâmetros
    distintos, não repetidos).
18. `with` bindings são visíveis nos guards da mesma cláusula.
19. Pattern `Cons` é reconhecido pelo parser mas rejeitado pelo typeck com
    erro "List patterns são Fio 8".
20. Lambda atribuído a variável (`let g := inc`) pode ser chamado (`g 41`)
    via `call_indirect` no codegen.
21. Função nomeada atribuída a variável (`let g := fat`) carrega function
    pointer e é chamável via `call_indirect`.
22. Tuple patterns funcionam em match e lambda (`match (1, 2) (a, b): a`
    produz `1`).
23. `Ty::Tuple(Vec<Ty>)` existe em `kata-core` (antecipado de Fio 5, sem
    field access nem `.N`).
24. Variantes de Enum são acessíveis sem qualificação (`True` em vez de
    `Boolean::True`) quando o Enum está no escopo. Ambas as formas são
    válidas em patterns e expressões.
25. `EnumRegistry` cataloga variantes por enum e é usado para resolver
    patterns desqualificados e verificar exaustividade.
26. Manual atualizado se implementação divergiu do PRD.

### Inferência de tipos de parâmetros (bidirecional limitada)

27. **Partial dispatch**: `+ 10 _` despacha com apenas o primeiro argumento
    tipado (`Int`). O DispatchTable retorna o overload único que casa e os
    tipos esperados para as posições ausentes. O hole recebe o tipo do
    parâmetro correspondente do overload casado. Se mais de um overload
    casa (ambíguo), o typeck não resolve — exige contexto (DoD 29 ou 30).
28. **Holes com ascription**: `_::Int` em posição de argumento fornece o
    tipo do hole diretamente. `+ 10 _::Int` funciona sem partial dispatch
    (a ascription resolve). `+ _::Int _::Float` desambigua: o segundo arg
    exclui overloads Int e Rational, deixando só Float, que por partial
    dispatch resolve o primeiro hole como Float.
29. **Hint top-down via ascription em lambda**: `(lambda x: + x
    1)::(Int -> Int)` extrai `x: Int` do tipo anotado. O typeck propaga
    o tipo esperado (`hint: Option<&Ty>`) pela recursão de `infer_expr`.
    Quando o hint é `Ty::Function(params, ret)`, `infer_lambda` define os
    parâmetros com os tipos de `params` em vez de criar `InferVar`.
30. **`LambdaInferenceFail`**: quando nenhum mecanismo (partial dispatch,
    ascription de hole, hint top-down, assinatura de Sig) fornece o tipo
    de um parâmetro de lambda, o typeck produz `LambdaInferenceFail` com
    span do lambda — não `NoOverload` opaco apontando para uma operação
    arbitrária dentro do corpo.
31. **Apply de lambda inline**: `(lambda x: + x 1) 42` infere os
    argumentos primeiro (síntese bottom-up: `42 → Int`), define `x: Int`
    no escopo do lambda a partir do tipo do argumento, e infere o corpo
    com o tipo conhecido. Equivalente ao partial dispatch para callee
    lambda em vez de nome no DispatchTable.

## Não Inclui

- Actions/return/`;`/`?` (Fio 3)
- Enums com payload/Result/Optional/`|` (Fio 4)
- Structs com campos/Tuples/alias (Fio 5)
- Tipos refinados/Ascription de expressão como validação (Fio 6 — Fio 2
  usa ascription apenas como hint de tipo para inferência bidirecional,
  não como validação de predicados)
- Interfaces/Generics/Dispatch polimórfico (Fio 7)
- Coleções/ITERABLE/Stream Fusion (Fio 8)
- Closures com captura/Escape Analysis/ARC/TRMA (Fio 9)
- `|` fallback local (Fio 4 — exige Result/Optional)
- `?` fail-fast (Fio 3 — exige Actions)
- List patterns em runtime (Fio 8 — exige List)
- Anotação de tipo em `Pattern::Ident` (`lambda x::Int: ...`) — futuro
  (exige mudança no AST e parser; Fio 2 usa ascription no lambda inteiro
  ou partial dispatch para resolver tipos de parâmetros)
- Unificação Hindley-Milner com union-find (não planejado — Fio 2 usa
  bidirecional limitada: partial dispatch + hint top-down)

## Arquitetura

### Pipeline do lambda nomeado

```
Sig "fat :: Int Int => Int"
  + clauses [lambda 0 acc: acc, lambda n acc: ...]
    │
    ▼
kata-resolution
  registra "fat" no DispatchTable com ffi_symbol=None
  preserva clauses no ResolvedModule
    │
    ▼
kata-inference
  infer_lambda(clauses, [Int, Int], Int)
  → TypedExprKind::Lambda { func_name: "fat", param_types: [Int, Int],
                           ret_ty: Int, clauses: [...] }
  verifica exaustividade, sobreposição, tipos
  propaga tail_pos
    │
    ▼
kata-codegen
  declara função "fat" no JITModule
  para cada cláusula: pattern test → branch
  body lowerado em block próprio
  retorna FuncId
  calls to "fat" usam call direto
```

### Pipeline do match

```
Expr::Match { scrutinee, arms }
    │
    ▼
kata-inference
  infer_expr(scrutinee) → TypedExpr com ty
  check_exhaustiveness(arms, scrutinee.ty)
  para cada arm: check_pattern, infer body
  → TypedExprKind::Match { scrutinee, arms, ty }
    │
    ▼
kata-codegen
  lower_expr(scrutinee) → Value
  para cada arm: pattern test → brif → block
  block de continuação recebe resultado
```

### Desugar Hole

```
+ 10 _              Expr::Apply { callee: +, args: [10, Hole] }
    │
    ▼ desugar_hole (intercepta Expr::Hole em args)
Expr::Lambda { patterns: [Ident("a")], body: Apply(+, [10, Ident("a")]) }
    │
    ▼ infer
TypedExpr::Closure { ... ty: Function([Int], Int) }
    │
    ▼ codegen
function "lambda_1" { entry(a: i64) → return bi_add(10, a) }
```

### Desugar Pipe

```
5 |> + 1 _
    │
    ▼ desugar_pipe
+ 1 5                     -- substitui Hole por lhs
    │
    ▼ desugar_hole (não há Hole restante)
Expr::Apply { callee: +, args: [1, 5] }
    │
    ▼ infer + codegen (Fio 1)
```

## Riscos

1. **TCO via Cranelift**: O Cranelift faz TCO quando a chamada está em tail
   position. O `tail_pos: bool` na TAST é o marcador. Mas o Cranelift pode
   não fazer TCO em todos os casos (ex: chamada via `call_indirect`).
   Fio 2 só tem call direto — TCO deve funcionar. Se não funcionar em algum
   caso, o fatorial recursivo profundo stack overflows. Testar com profundidade
   alta (ex: `fat 100000 1`).

2. **Pattern matching em Boolean**: Boolean é o único `Sum` em Fio 2 (variantes
   unitárias). O codegen emite `brif` comparando i64 (0=False, 1=True). Isto
   é trivial, mas é a fundação para match em enums com payload (Fio 4).

3. **Múltiplas cláusulas como branch chain**: Cada cláusula lambda é um test
   de pattern. Para `fat` com 2 cláusulas, é um `brif` + 2 blocks. Para
   funções com muitas cláusulas, o branch chain pode ser ineficiente. O
   Cranelift pode otimizar para switch se os patterns são literais
   consecutivos. Não otimizar prematuramente em Fio 2.

4. **Lambda como valor e `call_indirect`**: Lambdas são valores e podem ser
   atribuídos a variáveis. O codegen precisa de `call_indirect` (Cranelift)
   para chamar via function pointer. A assinatura para `call_indirect` é
   construída a partir do `Ty::Function` (param_types + ret_ty). O function
   pointer é armazenado como `pointer_type` do Cranelift. Sem captura, sem
   Arc — só o ponteiro nu. Fio 9 trará `CaptureBox` com captures e Arc.
   Risco: o Cranelift pode não fazer TCO via `call_indirect` (tail call para
   função desconhecida). Se o fatorial for chamado via variável (`let g :=
   fat; g 100000 1`), pode stack overflow. Testar ambos os caminhos.

5. **Hole desugar e inferência de aridade**: O desugar precisa contar holes
   corretamente e gerar parâmetros distintos. O parser produz `Expr::Hole`
   em posição de argumento e `Pattern::Wildcard` em posição de pattern — a
   disambiguação é no parser. Se o parser não marcar a posição corretamente,
   o desugar gera lambda errado.

6. **`with` como açúcar**: `with` é açúcar para `let` chain. Mas o escopo dos
   bindings é o da cláusula, não global. O typeck precisa criar escopo filho
   para `with` bindings antes de processar guards.

7. **Escala do fio**: Este é o maior fio até agora. Novos variants em `Expr`,
   `TypedExprKind`, novos enums (`Pattern`), modificações em parser, inference,
   codegen. O PRD-fio1 tinha 18 itens de DoD; este tem 20. O risco é
   tentar fazer tudo de uma vez. Abordagem incremental: (1) lambda anônimo
   primeiro, (2) funções nomeadas com múltiplas cláusulas, (3) match, (4)
   guards, (5) hole, (6) pipe, (7) with.

8. **Dump da AST no driver**: O driver deve ter um subcomando `kata ast
   <file>` (ou `kata ast -e <expr>`) que imprime a AST (pretty-printed ou
   `Debug`) logo após o parse, antes de qualquer pass posterior. Isto é
   infraestrutura de debugging essencial — sem ela, diagnosticar erros como
   `let x := 5 in + x 1` (onde `in` é tokenizado como `Ident("in")` e
   consumido greedy pelo `parse_apply` do `let`) requer testes ad hoc com
   `eprintln!`. O driver já tem `lex` e `parse` como subcomandos; `ast` é
   o complemento natural. Considerar também um flag `--dump-ast` para
   `eval`/`run` que imprime a AST antes de falhar.