# PRD — STEPPABLE Polimórfico via Refined Types

**Data:** 2026-08-18
**Status:** 📝 Rascunho
**Autor:** Arthur + Hermes
**Depende de:** PRD-range-degenerate-pipe-limit ✅ (is_neutral + check compile-time), PRD-constant ✅ (comptime pass, pending_predicates), PRD-refines ✅ (refined types, ascription)
**Supera:** limitação de "tipos customizados não verificados" do PRD-range-degenerate-pipe-limit

## Contexto

O PRD-range-degenerate-pipe-limit introduziu `is_neutral` na interface STEPPABLE
e um check de neutralidade do step em compile-time. Mas o check é hardcoded:
`check_neutral_step` (collections.rs:355) faz match em `IntLit`/`FloatLit`, e
`step_default_literal` (collections.rs:326) hardcodeia Int→1, Float→1.0.

A interface STEPPABLE mente: promete dispatch polimórfico, entrega hardcoded
por tipo. Tipos customizados que implementam STEPPABLE não têm `is_neutral`
avaliado, nem step default funcional.

A limitação de "step dinâmico não verificável" também é inaceitável: sem
garantia em compile-time, o step pode ser neutro em runtime → loop infinito.
Não há fallback — se o compilador não pode provar que o step é seguro, é
erro de compilação.

## Decisão de design: refined types

O step do range deve ter um **tipo refinado** que exclui o neutro. O predicado
do refinamento é `!is_neutral _`. O sistema de refined types existente —
`const_eval_predicate` para literais, `pending_predicates` para constants, e
tipo de retorno na assinatura para valores dinâmicos — cobre todos os casos.

Não há multi-pass novo. Não há JIT para step dinâmico. A assinatura é a prova.

### Como cada caso é resolvido

| Caso | Mecanismo | JIT? |
|---|---|---|
| Literal `[0..2..10]` | `const_eval_predicate` avalia `!is_neutral 2` | não |
| Literal neutro `[0..0..10]` | `const_eval_predicate` avalia `!is_neutral 0` → `false` → erro | não |
| Default `[0..10]` (Hole) | impl de `step` retorna NonNeutral → tipo já é a prova | não* |
| Constant `constant s := 0; [0..s..10]` | `pending_predicates` → comptime pass substitui Ident → valida | sim (comptime) |
| Constant seguro `constant s := 2; [0..s..10]` | `pending_predicates` → comptime pass valida → ok | sim (comptime) |
| Dinâmico `f()` onde `f :: Int => NonNeutral` | tipo de retorno é a prova | **não** |
| Dinâmico `f()` onde `f :: Int => Int` | tipo não é NonNeutral → erro de tipo | não (erro) |

*Step default: se a impl declara `step :: Int => NonNeutral`, o typeck recebe
NonNeutral diretamente. Se a impl declara `step :: Int => Int`, o typeck
tenta ascription `step(start)::NonNeutral` — se step(start) é foldado pelo
comptime pass para literal, `const_eval` resolve. Se não é foldado (step
depende de valor runtime), é dinâmico → erro. Sem fallback.

## 1. Função `not` no prelude

### Problema

O sistema de `pending_predicates` valida que o predicado retorna `True`
(predicates.rs:46). `is_neutral` retorna `True` quando o step **é** neutro —
o caso inválido. O predicado do refined type precisa ser a **negação** de
`is_neutral`: deve retornar `True` quando o step **não** é neutro (válido).

### Solução: `not` como função no prelude

`not` é uma função regular em core.kata, como `and` já existe:

```kata
not :: Boolean => Boolean
lambda a:
    a: False
    otherwise: True
```

O predicado do refined type é `not (is_neutral _)`:
```kata
data (Int, not (is_neutral _)) as NonNeutral
```

- `not` é despachada pelo JIT como qualquer função — sem caso especial
- `const_eval_predicate` precisa reconhecer `not` (ver §1a)
- `validate_pending_predicates` JIT-executa `not (is_neutral _)` normalmente
  — `not` é uma função no DispatchTable, `is_neutral` é uma lambda nas impls

### Mudanças

#### 1a. const_eval: avaliar `not expr`

`eval_bool_expr` (const_eval.rs:51) adiciona caso para `not`:

```rust
Expr::Apply { callee, args } if args.len() == 1
    && matches!(callee.node, Expr::Ident { name } if name == "not") =>
{
    let inner = eval_bool_expr(&args[0])?;
    Some(!inner)
}
```

Isso permite que `const_eval` resolva `not (is_neutral _)` quando a inner
expression é avaliável. Para Int/Float, `is_neutral` é `= _ 0` — `const_eval`
já avalia `=`. Então `not (= 0 0)` → `not true` → `false` → erro. Tudo em
compile-time, sem JIT.

Para tipos customizados onde `is_neutral` é uma lambda complexa,
`const_eval` retorna `None` → `pending_predicates` → comptime pass
JIT-executa `not (is_neutral value)` → despacha ambas as funções.

#### 1b. Parser: sem mudança

`not` é um identificador regular. `parse_expr_for_predicate` já chama
`parse_expr`, que trata `not` como function application prefixa:
`not (is_neutral _)` → `Apply { Ident("not"), [Apply { Ident("is_neutral"), [Hole] }] }`.

`is_predicate_start` (type_decls.rs:125) já reconhece `Token::Ident("not")`
como início de predicado (é um Ident).

#### 1c. validate_pending_predicates: sem mudança

O JIT já executa expressões Kata arbitrárias. `not (is_neutral _)` é
`Apply { Ident("not"), [Apply { Ident("is_neutral"), [Hole] }] }` — o JIT
despacha `is_neutral` (lambda da impl), depois `not` (lambda do prelude).
Se o resultado é `False` (tag 0) → predicado falhou → erro.

## 2. Refined type NonNeutral no prelude

```kata
# ── Refined: NonNeutral ───────────────────────────────────────
# Step de range não-neutro. Predicado é a negação de is_neutral.
# Usado pelo typeck do range: step deve ter tipo NonNeutral.

data (Int, not (is_neutral _)) as NonNeutral
data (Float, not (is_neutral _)) as NonNeutralFloat
```

- `NonNeutral` é alias de `Int` com predicado `not (is_neutral _)`
- `NonNeutralFloat` é alias de `Float` com predicado `not (is_neutral _)`
- `const_eval_predicate` avalia `not (is_neutral literal)`:
  - Substitui Hole por literal → `not (is_neutral 2)` → `not (= 2 0)` → `not false` → `true` → ok
  - `not (is_neutral 0)` → `not (= 0 0)` → `not true` → `false` → erro
- Tipos customizados declaram seu próprio refined type:
  ```kata
  data (MeuTipo, not (is_neutral _)) as MeuTipoNonNeutral
  ```

## 3. Interface STEPPABLE e impls

### Tipo de retorno de `step` na impl

Para que o step default (Hole) seja automaticamente NonNeutral sem ascription,
a impl de Int/Float declara `step` com retorno refinado:

```kata
Int implements STEPPABLE
    step :: Int => NonNeutral
    lambda x: 1
    is_neutral :: Int => Boolean
    lambda x: = x 0

Float implements STEPPABLE
    step :: Float => NonNeutralFloat
    lambda x: 1.0
    is_neutral :: Float => Boolean
    lambda x: = x 0.0
```

A interface continua declarando `step :: Self => Self` (polimórfica). A impl
**estreita** o tipo de retorno para `NonNeutral` (covariância de retorno).
Isso é permitido em linguagens com dispatch polimórfico — o tipo de retorno
da impl pode ser um subtipo do declarado na interface.

**Decisão aberta:** Kata permite estreitamento de tipo de retorno na impl?
Se não, a impl declara `step :: Int => Int` e o typeck do range gera ascription
`step(start)::NonNeutral` (que funciona se step(start) é foldado para literal).

### Para tipos customizados

```kata
MeuTipo implements STEPPABLE
    step :: MeuTipo => MeuTipoNonNeutral
    lambda x: ...
    is_neutral :: MeuTipo => Boolean
    lambda x: ...
```

A função que produz step dinâmico deve retornar `MeuTipoNonNeutral`:
```kata
f :: MeuTipo => MeuTipoNonNeutral
```
Se retorna `MeuTipo` (sem refinamento), o typeck do range rejeita.

## 4. Typeck do range

### Remover hardcode

- `step_default_literal` (collections.rs:326) — **removido**. Step default
  vem da impl da interface. O typeck emite a chamada `STEPPABLE::step(start)`
  despachada para o tipo concreto. Se a impl declara retorno `NonNeutral`,
  o tipo já é refinado. O comptime pass folda para literal quando possível.

- `check_neutral_step` (collections.rs:355) — **removido**. O check de
  neutralidade é feito pelo sistema de refined types via ascription, não
  por match hardcoded em IntLit/FloatLit.

### Novo fluxo de `infer_range_lit`

1. Inferir start e end (como hoje)
2. Verificar que elem_ty implementa STEPPABLE (como hoje)
3. Resolver step:
   - Se Hole: emitir chamada `STEPPABLE::step(start)` despachada para elem_ty.
     O tipo de retorno é o da impl (NonNeutral se a impl declara).
   - Se explícito: inferir e verificar tipo (como hoje)
4. Exigir que o step tenha tipo NonNeutral (ou equivalente do tipo base):
   - Se já tem tipo NonNeutral → ok
   - Se tem tipo base (Int/Float) → gerar ascription `step::NonNeutral`:
     - Literal → `const_eval_predicate` resolve → ok/erro
     - Ident de constant → `pending_predicates` → comptime pass valida
     - Dinâmico → erro (não pode ascribed valor dinâmico a refined)
   - Se tem tipo incompatível → erro de tipo
5. Produzir `RangeLit` na TAST

### Verificação do tipo do step

O typeck não procura um refined type por nome. Examina os predicados
do tipo do step e procura referência a `is_neutral`.

Fluxo:
1. Step tem tipo `T`
2. Se `T` é um refined type (`Ty::Struct` com `predicates` no StructRegistry):
   - Buscar os predicados em `refined_decls` (por nome do tipo)
   - Para cada predicado, fazer walk da AST procurando `Ident("is_neutral")`
   - Se algum predicado referencia `is_neutral` → o tipo carrega a garantia
     de não-neutralidade → ok
   - Se nenhum predicado referencia `is_neutral` → o tipo não prova
     não-neutralidade → erro
3. Se `T` não é um refined type → erro (sem prova de não-neutralidade)

A validação semântica do predicado (que ele retorna `True` para não-neutros)
é responsabilidade do sistema de refined types — `const_eval` para literais,
`pending_predicates` para constants, assinatura para dinâmicos. O typeck
do range só confirma que o tipo **tem** a garantia, não a revalida.

Isso permite que qualquer refined type com `is_neutral` nos predicados
seja usado como tipo de step — não precisa se chamar `NonNeutral`:
```kata
data (Int, not (is_neutral _), > _ 0) as PositiveStep
```
`PositiveStep` também serve — tem `is_neutral` nos predicados.

### Conexão data/behavior

A separação é mantida:
- **Data:** `data (Int, not (is_neutral _)) as NonNeutral` — declaração de
  tipo refinado no reino de data. O predicado é uma expressão que referencia
  o método `is_neutral` da interface STEPPABLE — referência, não definição.
- **Behavior:** `Int implements STEPPABLE` com `is_neutral :: Int => Boolean`
  — definição do comportamento no reino de behavior.
- **Typeck:** examina os predicados (data) e procura referência a `is_neutral`
  (behavior). É a ponte entre os dois reinos — não mistura, conecta.

## 5. Remoção do hardcode de step default

### Estado atual

`step_default_literal` (collections.rs:326) hardcodeia:
- Int → `IntLit { text: "1" }`
- Float → `FloatLit { text: "1.0" }`
- Outros → erro

### Novo comportamento

Quando step é Hole, o typeck emite a chamada ao método `step` da interface
STEPPABLE despachada para `elem_ty`, com `start` como argumento. O tipo de
retorno é o da impl concreta (NonNeutral se declarado na impl).

O comptime pass resolve:
- Se `step(start)` é foldável (args literais, função pura) → JIT-executa →
  substitui por literal NonNeutral. `const_eval_predicate` valida.
- Se não é foldável (step depende de valor runtime) → step fica como chamada.
  Se a impl declara retorno NonNeutral, o tipo já é a prova → ok.
  Se a impl declara retorno Self (Int) → step é dinâmico com tipo Int →
  ascription falha → erro de compilação.

**Sem fallback.** Se o compilador não prova que o step é seguro, erro.

## 6. Interação com o pipeline

```
infer_module → comptime_pass → ... → codegen
                   ↓
           Fase 1: evaluate_constants
           Fase 2: fixpoint fold_literal_calls
                   (folda step(start) para literal se possível)
           Fase 3: fold_constant_refs (functions/actions)
           Fase 3b: fold em corpos de functions
           Fase 4: validate_pending_predicates
                   (valida !is_neutral para constants que geraram pending)
```

Nenhum pass novo. O sistema de `pending_predicates` existente já valida
predicados após o comptime fold. A única adição é o reconhecimento de `!`
como negação no predicado.

### REPL

O REPL já chama `run_comptime_pass` após `infer_module`. `pending_predicates`
já são validadas. Sem mudança.

## 7. Mudanças por camada

| Camada | Arquivo | Mudança |
|---|---|---|
| const_eval | `kata-inference/src/infer/const_eval.rs` | `eval_bool_expr` avalia `not expr` → inverte resultado |
| Inferência | `kata-inference/src/infer/collections.rs` | Remover `step_default_literal` e `check_neutral_step`. Step Hole → chamada `STEPPABLE::step(start)`. Exigir tipo NonNeutral via ascription |
| Prelude | `stdlib/core.kata` | Adicionar `not :: Boolean => Boolean`. Adicionar `data (Int, not (is_neutral _)) as NonNeutral` e `data (Float, not (is_neutral _)) as NonNeutralFloat`. Impl de Int/Float declara `step :: Int => NonNeutral` |

## 8. Fases

### Fase 1: Função `not` + predicado `not` em refined types

- Adicionar `not :: Boolean => Boolean` ao prelude (core.kata)
- `const_eval`: `eval_bool_expr` avalia `not expr` → inverte resultado
- `validate_pending_predicates`: sem mudança (JIT despacha `not` como função)
- **DoD:** `data (Int, not (is_neutral _)) as NonNeutral` parseia e valida.
  `5::NonNeutral` → ok. `0::NonNeutral` → erro.
- **Testes:** parser, const_eval, pending_predicates

### Fase 2: Refined types NonNeutral no prelude

- Declarar `data (Int, not (is_neutral _)) as NonNeutral` e
  `data (Float, not (is_neutral _)) as NonNeutralFloat`
- Impl de Int/Float declara `step :: Int => NonNeutral` (se estreitamento
  de retorno é permitido)
- **DoD:** `1::NonNeutral` → ok. `0::NonNeutral` → erro.
  `step :: Int => NonNeutral` typechecks.
- **Testes:** ascription refined, inferência de tipo de retorno

### Fase 3: Typeck do range usa refined types

- Remover `step_default_literal` e `check_neutral_step`
- Step Hole → chamada `STEPPABLE::step(start)` despachada para elem_ty
- Exigir que step tenha tipo NonNeutral (ou base + ascription)
- Step literal → ascription → const_eval
- Step constant → ascription → pending → comptime pass
- Step dinâmico sem NonNeutral → erro de tipo
- **DoD:**
  - `[0..10]` → ok (step default = NonNeutral via impl)
  - `[0..2..10]` → ok (literal 2, ascription NonNeutral, const_eval passa)
  - `[0..0..10]` → erro (literal 0, ascription NonNeutral, const_eval falha)
  - `constant s := 0; [0..s..10]` → erro (pending → comptime pass valida → falha)
  - `constant s := 2; [0..s..10]` → ok (pending → comptime pass valida → ok)
  - `f()` onde `f :: Int => NonNeutral` → ok (tipo é a prova)
  - `f()` onde `f :: Int => Int` → erro (não pode ascribed dinâmico a refined)
  - Tipo customizado com STEPPABLE + NonNeutral → ok
  - Tipo customizado sem NonNeutral declarado → erro
- **Testes E2E:** todos os casos acima

## 9. Não-objetivos

- **Multi-pass para check de neutralidade** — substituído por refined types.
  O sistema de `pending_predicates` existente já cobre constants. A
  assinatura cobre step dinâmico. Sem novo pass.
- **Avaliador de lambdas genérico no typeck** — não é necessário. O typeck
  usa tipos refinados, não avalia lambdas.
- **Check de degeneração em runtime** — compile-time only. Sem fallback.
- **`!` como operador geral da linguagem** — se Optão 2 (contida a
  predicados), `!` não é adicionado como operador de expressões gerais.
- **Range infinito como tipo distinto** — fora de escopo.

## 10. Decisões abertas

- **D2: Estreitamento de tipo de retorno na impl — funciona hoje.**
  - Kata não valida compatibilidade de retorno entre interface e impl.
    `validate_impls_after_merge` (interface_registry.rs:354) só verifica
    que a interface existe. O tipo de retorno da impl (pass0.rs:399) é
    independente do tipo de retorno da interface.
  - Cada método da impl vira uma `Signature` flat no DispatchTable com
    o tipo de retorno **da impl** (pass0.rs:449). O dispatch usa esta
    signature, não a `InterfaceSignature` da interface.
  - Conclusão: a impl pode declarar `step :: Int => NonNeutral` quando a
    interface declara `step :: Self => Self`. O DispatchTable retorna
    `NonNeutral`. O typeck do range recebe `NonNeutral` direto — sem
    ascription, sem comptime pass.
  - Funciona por omissão (ausência de validação), não por design explícito.
    Se validação de covariância for adicionada no futuro, deve permitir
    estreitamento (subtipo de retorno) explicitamente.

- **D3: Verificação por conteúdo dos predicados, não por nome.**
  - O typeck examina os predicados do tipo do step e procura referência a
    `is_neutral` (walk da AST do predicado procurando `Ident("is_neutral")`).
  - Qualquer refined type cujos predicados referenciem `is_neutral` serve
    como tipo de step — não precisa se chamar `NonNeutral`.
  - Mantém a separação data/behavior: o refined type (data) referencia
    `is_neutral` (behavior) nos seus predicados. O typeck conecta os dois
    reinos sem misturá-los.
  - Se o tipo não é refined ou nenhum predicado referencia `is_neutral`,
    erro: tipo não prova não-neutralidade.