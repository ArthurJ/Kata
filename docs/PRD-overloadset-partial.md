# PRD — OverloadSet em Aplicação Parcial

**Status:** Rascunho
**Data:** 2026-08-10
**Pré-requisito:** `@commutative` cross-type fix (commit `34c8154`), overloads
cross-type no prelude (em andamento)

## 0. Resumo

Hoje, `+ _ 2` (aplicação parcial) falha com `LambdaInferenceFail` quando
múltiplas overloads de `+` cascam com `[None, Int]`. O `resolve_partial`
retorna `AmbiguousDispatch` e o typeck não tem como escolher.

Este PRD propõe que o partial dispatch, em vez de falhar com ambíguo,
**projete** as overloads compatíveis para o tipo do lambda e produza um
`Ty::OverloadSet`. O lambda fica deferido e o dispatch é resolvido no call
site, pelo tipo concreto dos argumentos.

## 1. Problema

### 1.1. Estado atual

`+ _ 2` desugara para `lambda __hole_0: + __hole_0 2`. O typeck chama
`try_partial_dispatch` que constrói `partial_args = [None, Some(Int)]` e
chama `resolve_partial("+", [None, Int])`.

Antes dos overloads cross-type: só `+ :: Int Int => Int` casava → hole = Int,
unívoco.

Agora: `+ :: Int Int => Int` casa (posição 2 = Int). Swap comutativo tenta
`[Int, None]` → `+ :: Int Float => Float` e `+ :: Int Rational => Rational`
também casam. Resultado: `AmbiguousDispatch`.

`try_partial_dispatch` retorna `Failed(Ambiguous)`. `extract_partial` retorna
`(vec![], Some(failure))`. `lambda.rs:89` rejeita com `LambdaInferenceFail`.

### 1.2. Por que deferir com InferVar não resolve

Se tratarmos `Ambiguous` como `NotApplicable`, o lambda fica com `InferVar`
nos parâmetros e é registrado na side table de deferred lambdas. No call site
`f 10`, o caminho 2a em `apply.rs` re-infere o lambda com `__hole_0 = Int`.

Isso funciona para chamada direta, mas **quebra em HOFs**: `map f [1 2 3]`
recebe `f` com tipo `Function([InferVar(0)], InferVar(1))`. O `infer_map`
não consegue determinar o tipo do callback — `InferVar` é "não sei", não "é
um destes N". O typeck não tem informação para selecionar a overload.

### 1.3. Por que OverloadSet resolve

`Ty::OverloadSet` já existe e já é tratado em:
- `match_score` (dispatch.rs) — faz match de OverloadSet vs Action
- `infer_map`/`infer_fold` (collections_hof.rs) — seleciona overload por tipo
- `action_call.rs` — dispatch por args quando callee é OverloadSet
- `expr.rs` caminho 3b — produz OverloadSet para funções com múltiplas overloads

A diferença: hoje o OverloadSet é produzido quando uma **função nomeada** é
referenciada como valor (`(+)`, `map (+) [1 2 3]`). Este PRD estende para
quando uma **aplicação parcial** de uma função despachada resulta em
múltiplas projeções válidas.

O tipo de `f` seria `OverloadSet { overloads: [(Int) → Int, (Float) → Float,
(Rational) → Rational] }` — as projeções das overloads de `+` que casam com
`[None, Int]`, filtradas para as posições desconhecidas.

## 2. Design

### 2.1. Projeção de OverloadSet

Dado `partial_args = [None, Some(Int)]` e overloads compatíveis:

| Overload de `+` | Posições casadas | Projeção (tipo do lambda) |
|---|---|---|
| `Int Int → Int` | pos 2 = Int | `(Int) → Int` |
| `Int Float → Float @commutative` | swap: pos 1 = Int, pos 2 = hole | `(Float) → Float` |
| `Int Rational → Rational @commutative` | swap: pos 1 = Int, pos 2 = hole | `(Rational) → Rational` |

A projeção:
1. Para cada overload compatível, extrai os tipos das posições `None` como
   os **parâmetros** do lambda.
2. O **tipo de retorno** do lambda é o tipo de retorno da overload.
3. O conjunto de projeções vira `Vec<(Vec<Ty>, Ty)>` no `OverloadSet`.

### 2.2. Mudanças no partial dispatch

**`PartialDispatchOutcome`** — novo variant:

```rust
pub(crate) enum PartialDispatchOutcome {
    Inferred(Vec<Ty>),           // já existe: único overload, tipos extraídos
    Ambiguous(Vec<(Vec<Ty>, Ty)>), // NOVO: múltiplas projeções
    Failed(PartialDispatchFailure), // já existe: nenhuma overload
    NotApplicable,               // já existe: body não é Apply
}
```

`Ambiguous` carrega as projeções — `Vec<(param_types, ret_type)>` — que
viram o conteúdo do `OverloadSet`.

**`resolve_partial`** — quando múltiplas overloads casam (incluindo via swap
comutativo), em vez de retornar `AmbiguousDispatch`, retorna as projeções.

Isso exige mudar `resolve_partial_inner` para coletar as projeções em vez
de falhar. O fluxo:

1. Coletar todos os overloads compatíveis (incluindo via swap).
2. Se 1 overload → retornar `Ok(PartialDispatchResult)` (caminho existente).
3. Se >1 overload → retornar `Ok(PartialDispatchResult { overload: None,
   projections: Vec<(Vec<Ty>, Ty)> })` — ou um novo tipo de retorno.
4. Se 0 overloads → retornar `Err(AmbiguousDispatch)` ou `Err(NoOverload)`.

**Alternativa mais limpa:** `resolve_partial` retorna um novo tipo
`PartialResolveResult { unique: Option<PartialDispatchResult>, projections:
Vec<(Vec<Ty>, Ty)> }`. Se `unique.is_some()`, é o caminho existente. Se
`unique.is_none() && !projections.is_empty()`, é ambíguo com projeções.

### 2.3. Mudanças em `infer_lambda`

Quando `try_partial_dispatch` retorna `Ambiguous(projections)`:

1. Constrói `Ty::OverloadSet { name: callee_name, overloads: projections }`.
2. Os parâmetros do lambda ficam como `InferVar` (placeholder — o tipo real
   é determinado pelo OverloadSet).
3. O AST do lambda é guardado na side table de deferred lambdas (para
   monomorphização posterior).
4. O tipo do lambda no TypeEnv é `OverloadSet`, não `Function([InferVar], ...)`.

**`extract_partial`** — novo braço:

```rust
PartialDispatchOutcome::Ambiguous(projections) => {
    // Não é falha — é deferral com informação preservada.
    // Retorna Vec vazio (sem hints diretos) e None (sem contexto de erro).
    // O caller (infer_lambda) verifica o outcome diretamente para
    // construir o OverloadSet.
    (Vec::new(), None)
}
```

Mas `extract_partial` só retorna `(Vec<Ty>, Option<Failure>)`. O caller
precisa acessar as projeções. Solução: `infer_lambda` faz match no
`partial_outcome` antes de chamar `extract_partial`:

```rust
let overload_set = match &partial_outcome {
    PartialDispatchOutcome::Ambiguous(projections) => {
        Some(Ty::OverloadSet {
            name: callee_name.clone(),
            overloads: projections.clone(),
        })
    }
    _ => None,
};
let (param_type_hints, failure_ctx) = extract_partial(partial_outcome);
```

Se `overload_set.is_some()`, o lambda é construído com tipo `OverloadSet`
e registrado na side table. O caminho de `LambdaInferenceFail` (linha 89)
é pulado.

### 2.4. Mudanças em `infer_apply` — caminho OverloadSet no TypeEnv

Hoje, `infer_apply` tem:
- Caminho 0: iface method dispatch
- Caminho 1: DispatchTable (try_dispatch_table)
- Caminho 2a: InferVar no TypeEnv (lambda deferido)
- Caminho 2b: Function conhecida no TypeEnv
- Caminho 3: EnumRegistry fallback

**Novo caminho 2c:** `OverloadSet` no TypeEnv.

Quando `env.lookup(&func_name)` retorna `Ty::OverloadSet`:
1. Itera overloads do OverloadSet.
2. Faz `match_score(arg_types, overload_params, iface_reg)` para cada uma.
3. Seleciona a única compatível (ou erro se ambíguo/incompatível).
4. Re-infere o lambda com os tipos concretos dos parâmetros (via
   `infer_apply_lambda` com os AST da side table).
5. O resultado é o body do lambda inferido com tipos concretos — o codegen
   gera código para a versão monomorfizada.

Isso é o mesmo padrão do caminho 2a (InferVar), mas em vez de "não sei o
tipo, descubra com os args", é "sei que são estes N tipos, selecione um
com os args".

### 2.5. Mudanças em HOFs (map/filter/fold)

Hoje, `infer_map` já lida com `OverloadSet` em callbacks — é assim que
`map (+) [1 2 3]` funciona. O `(+)` produz `OverloadSet` no caminho 3b de
`expr.rs`, e `infer_map` seleciona a overload pelo tipo do elemento da lista.

Com este PRD, `map f [1 2 3]` onde `f := + _ 2` também funciona: `f` tem
tipo `OverloadSet`, e `infer_map` seleciona `(Int) → Int` pelo tipo do
elemento. **Nenhuma mudança necessária em infer_map** — o OverloadSet chega
pronto.

A peça que falta: garantir que `infer_map` trate `OverloadSet` no callback
corretamente quando o callback vem do TypeEnv (não do DispatchTable). Hoje,
`infer_map` faz `resolve_operator_callback` que faz peel de Grouping e
verifica se é `Expr::Ident`. Se `f` é `Expr::Ident`, o typeck procura no
TypeEnv e encontra `OverloadSet`. O `infer_map` precisa fazer o dispatch
por args do OverloadSet — selecionar `(Int) → Int` e instanciar o lambda.

### 2.6. Monomorphização

O lambda deferido é monomorphizado quando o call site seleciona uma
overload concreta. O monomorphizador já instancia lambdas com tipos
concretos via `infer_apply_lambda`. A diferença: o lambda com OverloadSet
pode ser instanciado múltiplas vezes (uma por tipo concreto no call site).

Exemplo:
```kata
let f := + _ 2
let a := f 10       # instancia f com __hole_0 = Int → + 10 2
let b := f 3.14     # instancia f com __hole_0 = Float → + 3.14 2 (via swap)
```

O monomorphizador gera duas versões: `__kata_f_Int` e `__kata_f_Float`.

### 2.7. `OverloadSet` carrega só tipos

O `OverloadSet` hoje carrega `Vec<(Vec<Ty>, Ty)>` — só tipos. O `ffi_symbol`
de cada overload não está no OverloadSet. Para o codegen gerar a chamada
correta, o monomorphizador consulta o DispatchTable pelo nome da função
original + tipos concretos para obter o `ffi_symbol`.

Isso mantém `OverloadSet` leve e evita duplicar `OverloadInfo`. O
monomorphizador já tem acesso ao DispatchTable.

## 3. Fases

### Fase 1: `resolve_partial` retorna projeções

- Mudar `resolve_partial_inner` para coletar projeções quando múltiplas
  overloads casam (incluindo via swap comutativo).
- Novo tipo de retorno ou campo em `PartialDispatchResult` para as projeções.
- `try_partial_dispatch` retorna `Ambiguous(projections)` em vez de
  `Failed(Ambiguous)`.

**DoD:** `resolve_partial("+", [None, Int])` retorna 3 projeções em vez de
`AmbiguousDispatch`.

### Fase 2: `infer_lambda` produz OverloadSet

- `infer_lambda` faz match em `Ambiguous(projections)` antes de
  `extract_partial`.
- Constrói `Ty::OverloadSet` com as projeções.
- Registra o lambda na side table de deferred.
- O tipo do lambda no TypeEnv é `OverloadSet`.

**DoD:** `let f := + _ 2` type-checka com `f` tendo tipo `OverloadSet`. Não
há `LambdaInferenceFail`.

### Fase 3: `infer_apply` caminho OverloadSet no TypeEnv

- Novo caminho 2c em `infer_apply`: quando `env.lookup` retorna
  `OverloadSet`, faz dispatch por args, seleciona overload, re-infere
  lambda.
- Reusa `infer_apply_lambda` com os AST da side table.

**DoD:** `f 10` onde `f := + _ 2` type-checka e retorna `Int`. `f 3.14`
retorna `Float`.

### Fase 4 ✅: HOFs com OverloadSet de aplicação parcial

- `infer_map`/`infer_fold`/`infer_filter` tratam `Ty::OverloadSet` no callback:
  selecionam a overload cujos params casam com `elem_ty` (map/filter) ou
  `(acc_ty, elem_ty)` (fold) via `match_score`.
- `extract_callback_sig` no codegen trata OverloadSet sem panicar.
- Testes: `overloadset_hof_inference.rs` (5 testes inference),
  `overloadset_e2e.rs` (6 testes E2E incluindo map e fold).

**DoD:** `map f [1 2 3]` onde `f := + _ 2` type-checka e executa, retornando `[3 4 5]`. ✅

### Fase 5 ✅: Codegen + JIT via re-inferência

- Quando `infer_map`/`infer_fold` detecta `Ident` com `OverloadSet` no TypeEnv
  e lambda deferido na side table, re-infere o lambda com hint concreto
  (`Function([elem_ty], ?)` para map, `Function([acc_ty, elem_ty], acc_ty)`
  para fold). O `hint_has_params` check em `infer_lambda` faz o hint ter
  prioridade sobre o caminho Ambiguous — desambigua as overloads e produz
  `Function([concreto], ret_ty)`. O codegen recebe um Lambda normal que
  sabe resolver via `kata_ids`.
- `fold f 0 [1 2 3]` com `f := + _ _` (OverloadSet verdadeiro) executa sem
  SIGSEGV e retorna 6.

**DoD:** E2E test: `let f := + _ 2` seguido de `f 10` executa e retorna 12. ✅
`f 3.14` executa e retorna 5.14. ✅
`fold f 0 [1 2 3]` com `f := + _ _` retorna 6. ✅

### Fase 6 ✅: Atualizar testes

- Snapshots TAST (5): `cargo insta accept` — overloads cross-type mudaram
  o dispatch table visível nos snapshots.
- Partial dispatch (8): `+ 10 _` agora produz `OverloadSet` (literal primeiro
  casa com `Int Int`, `Int Float`, `Int Rational`). `+ _ _` agora succeed
  com OverloadSet. Testes atualizados para esperar OverloadSet.
- Semântica mudada (4): `+ 1 3.14` e `+ _::Int _::Float` agora succeed.
  `lambda x y: + x y` agora succeed com OverloadSet.
- Contagens (2): dispatch_table e prelude atualizadas para 12 overloads de `+`.
- Tree-shaker (1): functions.len() atualizado.
- REPL (1): contagem atualizada.
- Snapshots TAST: `cargo insta accept --all`.
- Contagens de overloads: atualizar.

**DoD:** `cargo test --workspace --no-fail-fast` passa com 0 falhas.

## 4. Casos de teste

### 4.1. Aplicação parcial cross-type

```kata
let f := + _ 2
# f : OverloadSet [(Int)→Int, (Float)→Float, (Rational)→Rational]

let a := f 10
# a : Int = 12

let b := f 3.14
# b : Float = 5.14

let c := f (from_int 5)
# c : Rational = 7
```

### 4.2. HOF com aplicação parcial

```kata
let inc := + _ 1
let a := map inc [1 2 3]
# a : List::Int = [2 3 4]

let add_half := + _ (from_int 1/2)
let b := map add_half [1 2 3]
# b : List::Rational = [3/2 5/2 7/2]
```

### 4.3. Aplicação parcial com `-` (não-comutativo)

```kata
let dec := - _ 1
# dec : OverloadSet [(Int)→Int, (Float)→Float, (Rational)→Rational]
# (-) não é @commutative, mas overloads cross-type existem

let a := dec 10
# a : Int = 9
```

### 4.4. Dois holes

```kata
let add := + _ _
# add : OverloadSet [(Int, Int)→Int, (Float, Float)→Float, ...]

let a := add 10 20
# a : Int = 30
```

### 4.5. Aplicação parcial sem overload compatível

```kata
let bad := foo _ 2
# foo não existe no DispatchTable → LambdaInferenceFail (NotApplicable)
```

## 5. Não-mudanças

- **Diretivas não são constraints de filtragem** — `@associative`,
  `@commutative` são propriedades atreladas às overloads, não critérios de
  seleção. O typeck seleciona por tipo; o otimizador consulta diretivas na
  overload selecionada.
- **Sem coerção implícita** — o OverloadSet preserva os tipos de cada
  projeção. Se não há overload `(Int, Float) → Float`, o OverloadSet não
  inclui essa projeção. O usuário precisa declarar a overload.
- **`OverloadSet` continua carregando só tipos** — `ffi_symbol` é resolvido
  no monomorphizador via DispatchTable lookup.

## 6. Arquivos modificados

- `crates/kata-core/src/dispatch.rs` — `resolve_partial` retorna projeções
- `crates/kata-inference/src/infer/partial_dispatch.rs` —
  `PartialDispatchOutcome::Ambiguous`, `try_partial_dispatch` retorna
  projeções
- `crates/kata-inference/src/infer/lambda.rs` — match em `Ambiguous`,
  constrói `OverloadSet`, registra na side table
- `crates/kata-inference/src/infer/apply.rs` — caminho 2c: OverloadSet no
  TypeEnv
- `crates/kata-inference/src/infer/collections_hof.rs` — garantir que
  OverloadSet do TypeEnv é tratado em callbacks
- `crates/kata-inference/src/infer/expr.rs` — `infer_let` registra lambda
  com OverloadSet na side table
- Testes: `dod27_partial_dispatch.rs`, `dod28_hole_ascription.rs`,
  `infer_test.rs`, `prelude_test.rs`, snapshots TAST

## 7. Riscos

- **Caminho 2c vs 2a:** ambos lidam com lambdas deferidos. A diferença é
  que 2c tem informação (OverloadSet) e 2a não (InferVar). Garantir que 2c
  tem precedência sobre 2a — se o tipo no TypeEnv é `OverloadSet`, não cair
  no caminho `InferVar`.
- **Swap comutativo em partial dispatch:** o swap cria projeções
  espúrias se não for cuidado. A projeção deve extrair os tipos das
  posições `None` **da overload original** (não da swapada), mas o swap
  indica que as posições conhecidas estão invertidas. A projeção precisa
  des-inverter ao extrair.
- **Monomorphização de múltiplas instâncias:** se `f` é usado com 3 tipos
  diferentes, 3 versões são geradas. O tree-shaker precisa eliminar as
  instâncias não-usadas.