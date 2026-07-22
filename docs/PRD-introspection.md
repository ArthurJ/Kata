# PRD — Introspecção: `type!()`

**Status:** 📝 Rascunho
**Data:** 2026-07-22
**Depende de:** Typeck ✅ (Ty em cada TypedExpr), Monomorphização ✅, SHOW ✅, Refined types ✅, First-class actions 📝 (PRD separado — `Ty::Action` precisa existir)
**Não depende de:** Type table runtime (não implementada), `kata_rt_typeof` (não implementado), `invoke!()` (PRD separado, não escrito)

## 1. Objetivo

Permitir que código Kata consulte o tipo de uma expressão em compile-time,
retornando o nome nominal do tipo como `Text`. Isto possibilita:

- Verificar que downcast preserva o tipo da variável original
- Debugar tipos em sessões interativas (REPL)
- Asserts de tipo em testes
- Documentação executável (o tipo aparece no output sem precisar ler o TAST)

```kata
let a := 10::PositiveInt
echo!(type!(a))        # "PositiveInt"
let n := a::Int
echo!(type!(n))        # "Int"
echo!(type!(a))        # ainda "PositiveInt" — a não foi mutada
```

A introspecção é **estática** — o tipo é resolvido em compile-time a
partir do `TypedExpr.ty`. Não cria aresta no call graph, não interfere no
tree-shaking, e não viola a proibição de reflexão dinâmica da spec
original.

## 2. Sintaxe

### 2.1. Forma

```
type!(expr)
```

- `type` é keyword do lexer (não é identificador comum)
- `!` marca como bang-call (mesma convenção de `echo!`, `panic!`)
- `(expr)` é a expressão cujo tipo será consultado
- Retorna `Text`

### 2.2. Posição

`type!()` é uma expressão. Pode aparecer em qualquer posição de expressão
dentro de Actions:

```kata
let t := type!(x)              # binding
echo!(type!(x))                # argumento de action call
match (type!(x))               # scrutinee
    "PositiveInt": ...
    otherwise: ...
```

`type!()` só aparece em Actions — Actions já não existem em lambdas
(funções puras), então a restrição é automática, não precisa enforcement
especial no parser ou typeck.

### 2.3. `type!(f)` — referência sem chamada

`type!()` aceita qualquer expressão, incluindo referências sem chamada.
Com first-class actions (PRD separado), tanto funções puras quanto actions
são valores e podem ser introspeccionados:

```kata
soma :: Int Int => Int
    + _ _

action worker (n :: Int) => Unit
    echo!(n)

action mostrar_tipos => Unit
    echo!(type!(soma))      # "(Int Int -> Int)"   — função pura
    echo!(type!(worker))    # "Action(Int) => Unit" — action
```

O typeck já tem o tipo de ambas em `TypedExpr.ty` — `Ty::Function` para
funções puras, `Ty::Action` para actions (após PRD de first-class actions).
`type!()` lê o tipo e formata. Não há chamada, não há execução, só
introspecção da assinatura.

## 3. Semântica

### 3.1. Resolução compile-time

O typeck já carrega o tipo de cada expressão em `TypedExpr.ty`. Quando o
monomorphizador encontra `type!(expr)`:

1. Consulta `expr.ty` no TAST
2. Formata o tipo para string (ver §3.3)
3. Avalia `expr` para side-effects (ver §3.2)
4. O resultado de `type!(expr)` é `TextLit(string_formatada)`

O codegen emite código para avaliar `expr` (preservando side-effects) mas
o valor de `expr` é descartado — o valor retornado é a constante `Text`.

### 3.2. Avaliação do argumento

`type!(expr)` **avalia `expr`** — side-effects acontecem. O tipo é
resolvido em compile-time, mas a expressão não é morta.

```kata
action exemplo => Unit
    type!(f!())     # f!() executa; type!() retorna o tipo do resultado
```

Isto é consistente com `echo!(expr)` — `echo!` avalia `expr` e imprime.
`type!()` avalia `expr` e retorna seu tipo. A diferença é que o tipo é
conhecido antes da avaliação, mas a avaliação acontece normalmente.

Se o usuário quer só o tipo sem executar, usa uma referência sem chamada:
`type!(f)` retorna o tipo da função/action, `type!(x)` retorna o tipo da
variável — nenhuma das duas executa nada.

### 3.3. Formatação de tipos

A string retornada usa a **própria sintaxe de tipo da linguagem** — o
mesmo formato que o usuário escreve em assinaturas. Uma função
`ty_display(&Ty) -> String` percorre o `Ty` recursivamente:

| `Ty` | `type!()` retorna | Sintaxe da linguagem |
|---|---|---|
| `Prim(Int)` | `"Int"` | `Int` |
| `Prim(Float)` | `"Float"` | `Float` |
| `Prim(Text)` | `"Text"` | `Text` |
| `Prim(Rational)` | `"Rational"` | `Rational` |
| `Unit` | `"Unit"` | `Unit` |
| `Struct("Pessoa")` | `"Pessoa"` | `Pessoa` |
| `Struct("PositiveInt")` | `"PositiveInt"` | `PositiveInt` (refined — nome nominal) |
| `Sum("Boolean")` | `"Boolean"` | `Boolean` |
| `Generic("Optional", [Int])` | `"Optional::Int"` | `Optional::Int` |
| `Generic("Result", [Int, Text])` | `"Result::(Int, Text)"` | `Result::(Int, Text)` |
| `Function([Int, Int], Int)` | `"(Int Int -> Int)"` | `(Int Int -> Int)` |
| `Action([Int], Unit)` | `"Action(Int) => Unit"` | `Action(Int) => Unit` |
| `Tuple([Int, Text])` | `"(Int, Text)"` | `(Int, Text)` |
| `List(Int)` | `"[Int]"` | `[Int]` (açúcar para `List::Int`) |
| `Array(Int)` | `"{Int}"` | `{Int}` (açúcar para `Array::Int`) |
| `Range(Int)` | `"[a..s..b]"` | Range — formato literal |
| `Sender(Int)` | `"Sender::Int"` | `Sender::Int` |
| `Receiver(Int)` | `"Receiver::Int"` | `Receiver::Int` |
| `Interface("NUM")` | `"NUM"` | Só aparece se typeck não resolveu para concreto |
| `InferVar(_)` | Erro compile-time | type!() em tipo não-resolvido é bug do typeck |
| `Var("T")` | `"T"` | Parâmetro de tipo genérico |

### 3.4. Refined types retornam nome nominal

`type!()` retorna o nome que o usuário deu ao tipo, não o tipo base:

```kata
data (Int, > _ 0) as PositiveInt

let a := 10::PositiveInt
echo!(type!(a))        # "PositiveInt" — não "Int"

let n := a::Int
echo!(type!(n))        # "Int" — downcast muda o tipo nominal
echo!(type!(a))        # "PositiveInt" — a não foi mutada
```

O typeck representa `PositiveInt` como `Ty::Struct("PositiveInt")` com
`StructInfo { alias_of: Some("Int"), predicates: Some([...]) }`. O nome
na variante `Struct` é o nome nominal — `type!()` lê esse nome.

### 3.5. Actions vs Functions — formatos distintos

`Ty::Function` e `Ty::Action` produzem strings diferentes, refletindo
a distinção semântica da linguagem:

| Tipo | `type!()` retorna | Exemplo |
|---|---|---|
| `Function([Int, Int], Int)` | `"(Int Int -> Int)"` | Função pura — `->` |
| `Action([Int], Unit)` | `"Action(Int) => Unit"` | Action — `Action(...)` com `=>` |

Isto espelha a sintaxe de declaração: funções usam `->` em tipo de valor,
actions usam `Action(...)` com `=>` como type annotation.

### 3.6. Interação com tree-shaking

`type!()` não cria aresta no call graph. O tree shaker (`collect_refs`)
trata `type!()` como folha — o nó `TypeOf` não tem callee de Action nem
função nomeada para coletar. Se `expr` interna contém calls, essas são
coletadas normalmente pela recursão de `collect_refs` sobre `expr` —
`type!()` é transparente para o shaker.

## 4. Pipeline — componentes afetados

```
crates/kata-lexer/src/ident.rs          # "type" → Token::Type (keyword)
crates/kata-ast/src/token.rs            # variante Type + Display
crates/kata-ast/src/expr.rs             # Expr::TypeOf { expr: Box<Spanned<Expr>> }
crates/kata-parser/src/expressions.rs   # parse type!(expr) como TypeOf
crates/kata-parser/tests/parser_test/   # testes do parser
crates/kata-inference/src/infer/mod.rs  # infer TypeOf: ty = Text
crates/kata-monomorph/src/lib.rs        # resolver TypeOf → avaliar expr + TextLit
crates/kata-core/src/ty.rs              # ty_display(&Ty) -> String (nova função)
crates/kata-codegen/                    # não alterado — vê expr + TextLit
crates/kata-tree-shaking/               # não alterado — TypeOf transparente para collect_refs
crates/kata-driver/tests/               # testes E2E
docs/Kata-lang-manual.md                # documentar type!()
docs/sintaxe-mapa.md                    # adicionar type!()
```

### 4.1. Por que nó AST próprio, não ActionCall

`echo!` é parseado como `ActionCall { callee: "echo", args: ... }` e
despachado via DispatchTable (é uma action real da stdlib). `type!()`
não pode seguir o mesmo padrão porque:

1. `type!()` precisa de `TypedExpr.ty` do argumento — informação do
   typeck, não disponível na stdlib
2. Não há interface `TYPE` para despachar — o tipo é uma propriedade
   estática, não um método
3. O monomorphizador precisa reconhecer o nó para substituir por
   `TextLit` — um ActionCall genérico não carrega a semântica de
   "retorne o tipo do argumento"

Portanto, `type!()` é um nó AST próprio (`Expr::TypeOf`), tratado
especialmente pelo typeck e monomorphizador. `type` é keyword do lexer
para o parser reconhecer o nó sem ambiguidade.

### 4.2. Transformação no monomorphizador

O monomorphizador transforma `type!(expr)` em:

1. Avalia `expr` (preserva side-effects, resultado descartado)
2. Retorna `TextLit(ty_display(&expr.ty))`

Em posição de statement, isto é direto: `expr` vira statement, `TextLit`
é o valor. Em posição aninhada (`echo!(type!(f!()))`), o monomorphizador
precisa hoistar `f!()` para statement precedente e substituir `type!(f!())`
por `TextLit` dentro do `echo!`.

### 4.3. `ty_display` — formatação de `Ty`

Nova função em `crates/kata-core/src/ty.rs`:

```rust
impl Ty {
    pub fn display(&self) -> String {
        match self {
            Ty::Prim(PrimTy::Int) => "Int".into(),
            Ty::Prim(PrimTy::Float) => "Float".into(),
            Ty::Prim(PrimTy::Text) => "Text".into(),
            Ty::Prim(PrimTy::Rational) => "Rational".into(),
            Ty::Unit => "Unit".into(),
            Ty::Struct(name) => name.clone(),
            Ty::Sum(name) => name.clone(),
            Ty::Generic(name, params) => {
                if params.len() == 1 {
                    format!("{name}::{}", params[0].display())
                } else {
                    let params_str = params.iter()
                        .map(|p| p.display())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}::({params_str})")
                }
            }
            Ty::Function(params, ret) => {
                let params_str = params.iter()
                    .map(|p| p.display())
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("({params_str} -> {})", ret.display())
            }
            Ty::Action(params, ret) => {
                let params_str = params.iter()
                    .map(|p| p.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Action({params_str}) => {}", ret.display())
            }
            Ty::Tuple(elements) => {
                let elems_str = elements.iter()
                    .map(|e| e.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({elems_str})")
            }
            Ty::List(t) => format!("[{}]", t.display()),
            Ty::Array(t) => format!("{{{}}}", t.display()),
            Ty::Range(_) => "[a..s..b]".into(),
            Ty::Sender(t) => format!("Sender::{}", t.display()),
            Ty::Receiver(t) => format!("Receiver::{}", t.display()),
            Ty::Interface(name) => name.clone(),
            Ty::Var(name) => name.clone(),
            Ty::InferVar(_) => panic!("type!() em tipo não-resolvido — bug do typeck"),
            // ... demais variantes
        }
    }
}
```

A função é recursiva — `Optional<List<Int>>` formata como
`"Optional::[Int]"`.

## 5. Fora do escopo

- **`type!()` runtime (type table)** — consultar `type_id` de um valor em
  runtime via `kata_rt_typeof`. Requer implementar a type table (3
  tabelas: SHARED_ARC, SHARED_ARENA, PER_FIBER_ARENA), `kata_rt_typeof`
  no runtime, e `register_type` emitido pelo codegen. Ver §6 (D9) para
  justificativa de exclusão.
- **`invoke!()`** — dispatch dinâmico por string. Ortogonal a `type!()`.
  Interage com tree-shaking (aresta dinâmica). PRD separado se houver
  caso de uso concreto.
- **Comparação de tipos** — `type!(x) = "PositiveInt"` funciona (compara
  Text), mas não há operador `is` ou `typeof` como em TypeScript. O
  usuário usa `match` sobre o `Text` retornado.

## 6. DoDs (Definitions of Done)

1. `type!(x)` retorna `Text` com o tipo nominal de `x`.
2. `type!(a)` onde `a := 10::PositiveInt` retorna `"PositiveInt"`.
3. `type!(n)` onde `n := a::Int` retorna `"Int"`.
4. `type!(a)` após downcast de `a` ainda retorna `"PositiveInt"` —
   downcast não muta a variável original.
5. `type!(42)` retorna `"Int"` (literal Int).
6. `type!("hello")` retorna `"Text"` (literal Text).
7. `type!(())` retorna `"Unit"`.
8. `type!(x)` onde `x :: Boolean` retorna `"Boolean"`.
9. `type!(x)` onde `x :: Optional<Int>` retorna `"Optional::Int"`.
10. `type!(x)` onde `x :: Result<Int, Text>` retorna `"Result::(Int, Text)"`.
11. `type!(x)` onde `x :: [Int]` retorna `"[Int]"`.
12. `type!(f)` onde `f` é função pura `Int Int => Int` retorna `"(Int Int -> Int)"`.
13. `type!(a)` onde `a` é referência de action `Action(Int) => Unit` retorna
    `"Action(Int) => Unit"`.
14. `type!(f!())` executa `f!()` — side-effects acontecem.
15. `type!(expr)` é substituído no monomorphizador por avaliação de
     `expr` + `TextLit`.
16. Tree shaking não é afetado — `type!()` não cria aresta no call graph.
17. `cargo test --workspace --no-fail-fast` passa sem regressão.
18. `cargo clippy --workspace --all-targets -- -D warnings` limpo.

## 7. Arquitetura — componentes afetados

```
# Lexer
crates/kata-lexer/src/ident.rs              # "type" → keyword
crates/kata-ast/src/token.rs                # Token::Type + Display impl

# AST
crates/kata-ast/src/expr.rs                 # Expr::TypeOf { expr: Box<Spanned<Expr>> }

# Parser
crates/kata-parser/src/expressions.rs       # parse type!(expr) → Expr::TypeOf
crates/kata-parser/tests/parser_test/       # testes: parse type!(), type!(f), type!(f!())

# Core — formatação de tipos
crates/kata-core/src/ty.rs                  # Ty::display() -> String (recursiva)

# Inference
crates/kata-inference/src/infer/mod.rs      # infer TypeOf: ty = Text

# Monomorphization
crates/kata-monomorph/src/lib.rs            # resolver TypeOf → avaliar expr + TextLit

# Tree shaking — não alterado
# Codegen — não alterado

# Testes E2E
crates/kata-driver/tests/                   # type!() em action, refined, downcast, generics, action ref

# Docs
docs/Kata-lang-manual.md                    # §type!() — introspecção compile-time
docs/sintaxe-mapa.md                        # adicionar type!() na tabela de sintaxe
```

## 8. Decisões de design

| # | Decisão | Racional |
|---|---|---|
| D1 | `type!()` resolve o tipo em compile-time | O typeck já tem o tipo em `TypedExpr.ty`. A string é construída antes do codegen. Zero consulta a runtime, zero type table. |
| D2 | Argumento é avaliado (side-effects preservados) | Consistente com `echo!(expr)`. O tipo é conhecido antes da avaliação, mas a expressão executa normalmente. `type!(f)` (referência sem chamada) não executa nada — só consulta a assinatura. `type!(f!())` executa `f!()`. |
| D3 | Nó AST próprio (`Expr::TypeOf`), `type` é keyword | `type!()` precisa de `TypedExpr.ty` do typeck, não disponível na stdlib. Não há interface para despachar. Keyword evita ambiguidade com identificadores e simplifica o parser. |
| D4 | Refined retorna nome nominal | `type!(a)` onde `a :: PositiveInt` retorna `"PositiveInt"`, não `"Int"`. O nome nominal é o que o typeck atribuiu. O tipo base é acessível via downcast + `type!()`. |
| D5 | `type` é keyword do lexer | Mais simples que tratar como ActionCall. O parser reconhece `type!` diretamente sem lookup no DispatchTable. Evita ambiguidade com identificadores chamados `type`. |
| D6 | Restrição de Action-only é automática | Actions já não existem em lambdas (funções puras). `type!()` só pode aparecer onde Actions aparecem. Não precisa enforcement especial. |
| D7 | Parâmetros de tipo são incluídos | `type!(x)` retorna `"Optional::Int"`, não `"Optional"`. Usa a própria sintaxe de tipo da linguagem (`::` para params, `::(...)` para múltiplos, `[]` para List, `{}` para Array). `Ty::display` percorre `Ty` recursivamente. |
| D8 | `type!()` não viola a proibição de reflexão | A spec antiga proíbe reflexão e invocação dinâmica para garantir tree-shaking determinístico. `type!()` é compile-time: não cria aresta no call graph, não invoca por string, não consulta runtime. É equivalente a `@comptime` — informação resolvida antes do codegen. |
| D9 | Modo runtime fica fora do escopo | Em Kata, todo valor tem tipo estaticamente conhecido. O type system é nominal e estático: não há `Any`, type erasure, tipagem gradual, loading dinâmico, nem coleções heterogêneas sem parâmetro de tipo. Canais são tipados (`Sender<T>`), `Fork` não retorna valor com tipo dinâmico, `Optional<T>` carrega `T` na variante. Runtime `typeof` sempre retornaria o mesmo que compile-time `type!()`. A type table documentada no manual (`SHARED_ARC` / `SHARED_ARENA` / `PER_FIBER_ARENA`) tem propósito primário de ARC decref walk (liberação type-directed de filhos), não introspecção do usuário — e nem ela está implementada (scaffolding morto, `#![allow(dead_code)]`). Implementar 3 tabelas + `kata_rt_typeof` + codegen emitindo `register_type` é trabalho real para zero benefício observável dado o type system atual. Quando `@parallel` (serialização TypeShape) ou debugger interativo existirem, o modo runtime se justifica — PRD separado. |
| D10 | `Function` e `Action` têm formatos distintos | `Function` usa `->` (sintaxe de tipo de função como valor). `Action` usa `Action(...)` com `=>` (sintaxe de type annotation de action). Reflete a distinção semântica: funções são puras, actions são impuras com scheduler. |
| D11 | Depende de first-class actions | `type!(worker)` onde `worker` é action só funciona se actions são valores com `Ty::Action`. Sem first-class actions, `worker` sem `!()` não é uma expressão válida. O PRD de first-class actions deve ser implementado primeiro. |

## 9. Riscos

| Risco | Mitigação |
|---|---|
| `type` como keyword quebra código existente que usa `type` como identificador | Buscar em stdlib + examples + testes por `type` como nome de variável/função. Se houver, renomear. |
| Monomorphizador não consegue resolver `TypeOf` se o tipo for `InferVar` não-resolvido | O typeck deve resolver todos os tipos antes do monomorph. Se chegar `InferVar`, é bug do typeck — panic com mensagem clara. |
| Hoisting de side-effects em posição aninhada (`echo!(type!(f!()))`) | Monomorphizador precisa extrair `f!()` para statement precedente. Complexidade moderada — mesmo padrão de hoisting que outras transformações monomorph. |
| `Ty::display` não cobre um `Ty` | Match deve ser exaustivo. Se faltar variante, panic em desenvolvimento (não em produção — todas as variantes são conhecidas). |

## 10. Exemplos

### 10.1. Verificação de downcast

```kata
data (Int, > _ 0) as PositiveInt

action verificar_downcast => Unit
    let a := 10::PositiveInt
    let n := a::Int
    echo!(type!(a))        # "PositiveInt"
    echo!(type!(n))        # "Int"
```

### 10.2. Tipos parametrizados

```kata
action debug_generics => Unit
    let x := Optional::Some 42
    let y := Result::Ok "hello"
    let z := [1, 2, 3]
    echo!(type!(x))        # "Optional::Int"
    echo!(type!(y))        # "Result::(Int, Text)"
    echo!(type!(z))        # "[Int]"
```

### 10.3. Referência de função sem chamada

```kata
soma :: Int Int => Int
    + _ _

action mostrar_tipo => Unit
    echo!(type!(soma))     # "(Int Int -> Int)"
```

### 10.4. Referência de action (primeira versão — requer first-class actions)

```kata
action worker (n :: Int) => Unit
    echo!(n)

action mostrar_tipo_action => Unit
    echo!(type!(worker))  # "Action(Int) => Unit"
```

### 10.5. Asserts de tipo em testes

```kata
@test
action test_soma_preserva_refined => Unit
    let a := 5::PositiveInt
    let b := 3::PositiveInt
    let r := PositiveInt (+ a b)
    match r
        Ok v:
            assert!(= (type!(v)) "PositiveInt")
        Err _:
            panic!("soma falhou")
```

### 10.6. Avaliação com side-effects

```kata
action debug_com_execucao => Unit
    type!(echo!("side effect"))   # imprime "side effect", retorna "Unit"
```