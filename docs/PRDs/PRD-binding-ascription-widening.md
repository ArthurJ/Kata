# PRD — Ascription em Bindings `let`/`var` + Widening de Interface

**Status:** ✅ Implementado
**Data:** 2026-09-08
**Depende de:** Interface system ✅ (`implements`/`refines`), TypeEnv ✅
**Não depende de:** Tipos refinados (ortogonal), comptime (ortogonal)
**Relacionado:** TODO.md §"Ascription em binding de `var`"

## 1. Objetivo

Permitir ascription de tipo entre o nome e o `:=` em bindings `let` e `var`:

```kata
let x::Int := 42
var y::Text := "hello"
var z::NUM := 0          -- widening: z aceita qualquer valor que implemente NUM
```

Hoje isso é rejeitado pelo parser. O único mecanismo para travar o tipo de um
binding é ascription sobre o valor: `var l := (0 :: Int)` — indireto, e
incapaz de expressar widening de interface.

### Princípio: anotação de binding, não operação de valor

`::Tipo` entre nome e `:=` é **sintaxe de binding** — anota o tipo do binding,
não opera sobre o RHS. É o mesmo `::` usado em parâmetros (`action f (x::Int)`)
e campos de struct (`data Pessoa (nome::Text)`): anotação do slot, não conversão
do valor.

Contraste com `::` em expressão (`expr::Type`), que é uma **operação** sobre o
valor: converte literal, valida predicado refinado, ou constrói struct. O
token é o mesmo, o papel é diferente — exatamente como o manual já descreve
para os outros contextos de `::` (cap 4, "Os outros papéis de `::`").

### Princípio: widening é restrição, não conversão

`var z::NUM := 0` não converte `0` para `NUM`. Declara que `z` aceita
qualquer valor cujo tipo **implementa** `NUM`. O typeck verifica
compatibilidade do RHS contra a interface — não há coerção, há checagem de
implementação. Se `Int` implementa `NUM`, o binding é válido. Se não, erro
compile-time.

## 2. Sintaxe

### 2.1. Forma geral

```
let nome::Tipo := expr
var nome::Tipo := expr
```

`::Tipo` é **opcional** entre o nome e o `:=`. Se ausente, comportamento atual
se mantém: tipo inferido do RHS, travado no primeiro binding.

### 2.2. Tipo pode ser concreto ou interface

```kata
let x::Int := 42               -- tipo concreto
var y::Text := "hello"         -- tipo concreto
var z::NUM := 0                -- interface (widening)
var w::ORD := "hello"          -- interface (widening)
```

### 2.3. Gramática

```
binding ::= ('let' | 'var') ident ['::' type_expr] ':=' expr
```

`type_expr` é o mesmo não-terminal usado em assinaturas de função, parâmetros,
e ascription de expressão. Sem ambiguidade: após `ident`, o parser verifica
se o próximo token é `::`. Se for, parseia `type_expr`. Em qualquer caso,
espera `:=` em seguida.

## 3. Semântica

### 3.1. Sem ascription (comportamento atual, inalterado)

```kata
var x := 42
var x := + x 1       -- OK: mesmo tipo (Int)
var x := "hello"     -- ERRO: re-binding divergente (TypeMismatch)
```

O tipo é inferido do primeiro binding e travado. Re-bindings devem preservar o
tipo. Esta semântica **não muda** com este PRD.

### 3.2. Com ascription de tipo concreto

```kata
var x::Int := 42
var x := + x 1       -- OK: Int == Int
var x := "hello"     -- ERRO: Int ≠ Text (re-binding divergente)
var x::Int := 3.14   -- ERRO: Float não é Int (RHS incompatível com anotação)
```

A ascription **substitui** a inferência: o tipo do binding é o anotado, não o
inferido do RHS. O typeck verifica que o tipo do RHS é compatível com o tipo
anotado (subtipagem/refinement narrowing normal). Re-bindings sem ascription
preservam o tipo anotado original — `var x := + x 1` não re-abre o tipo para
inferência.

### 3.3. Com ascription de interface (widening)

```kata
var z::NUM := 0          -- OK: Int implementa NUM
var z := 3.14            -- OK: Float implementa NUM (preserva interface)
var z := "hello"         -- ERRO: Text não implementa NUM
var z::NUM := True        -- OK: Boolean implementa NUM (ascription re-declarada)
```

O binding tem **tipo de interface** `NUM`. Valores de qualquer tipo concreto
que implemente `NUM` são aceitos. Re-bindings sem ascription preservam a
interface — `var z := 3.14` verifica `Float` contra `NUM`, não re-infere.

Re-binding com asção explícita pode **estreitar** a interface:

```kata
var z::NUM := 0
var z::ORD := "hello"    -- ERRO: estreitamento de interface não permitido no re-binding
```

Re-binding preserva o tipo (interface) do binding original. Mudar de `NUM`
para `ORD` no re-binding é re-binding divergente — mesmo princípio de 3.1.
Para mudar o tipo do binding, use um nome diferente.

### 3.4. `let` com ascription

```kata
let x::Int := 42         -- x é Int, imutável
```

Mesma semântica de `var`, mas imutável e único por escopo (regra existente
do `let`). Ascription não afeta imutabilidade — apenas anota o tipo.

### 3.5. Ascription de interface não é conversão

```kata
let z::NUM := 0
-- z tem tipo NUM (interface). Usar z em contexto que exige Int:
let n::Int := z          -- ERRO: NUM não é Int (não há downcast implícito)
```

O binding com interface tem tipo de interface. Para recuperar o tipo concreto,
é necessário ascription de valor (`z :: Int`) ou pattern matching. O
widening é unidirecional: concreto → interface, nunca interface → concreto.

## 4. Interação com re-binding existente

### 4.1. Re-binding preserva tipo anotado

```kata
var x::NUM := 0
var x := 3.14            -- OK: Float implementa NUM, preserva NUM
var x::NUM := 42         -- OK: re-declara ascription (mesmo tipo)
```

Re-binding sem ascription não re-abre o tipo para inferência. O tipo
anotado no primeiro binding é o contrato do binding por todo o escopo.

### 4.2. Re-binding com ascription diferente é erro

```kata
var x::NUM := 0
var x::Int := 42         -- ERRO: re-binding divergente (NUM ≠ Int)
```

Mesmo que `Int` implemente `NUM`, o tipo do binding é `NUM`, não `Int`.
Mudar para `Int` é mudar o tipo — re-binding divergente.

**Exceção:** re-binding com a **mesma** ascription é idempotente (OK).

### 4.3. Sem ascription, sem widening

```kata
var x := 0              -- x é Int (inferido)
var x := 3.14           -- ERRO: Int ≠ Float (re-binding divergente)
```

Sem ascription, o tipo é inferido e travado. Não há widening implícito. Se
quer widening, declare a interface explicitamente. Isso mantém o princípio
de que inferência é local e previsível — o tipo do primeiro binding determina
o contrato.

## 5. Implementação

### 5.1. AST

Adicionar campo `ty: Option<Spanned<TypeExpr>>` aos construtores:

```rust
// kata-ast/src/expr.rs
Let {
    name: String,
    ty: Option<Spanned<TypeExpr>>,   // NOVO
    value: Box<Spanned<Expr>>,
},
Var {
    name: String,
    ty: Option<Spanned<TypeExpr>>,   // NOVO
    value: Box<Spanned<Expr>>,
},
```

`None` = comportamento atual (inferência do RHS). `Some(t)` = tipo anotado.

### 5.2. Parser

Em `parse_let` e `parse_var` (`expressions.rs`), após consumir o nome e antes
de `expect(BindAssign)`:

```rust
let ty = if matches!(self.peek(), Token::DoubleColon) {
    self.advance();
    Some(self.parse_type_expr()?)
} else {
    None
};
self.expect(&Token::BindAssign, "`:=`")?;
```

Sem ambiguidade: `::` após ident em posição de binding nunca é VariantQual
(ident em minúsculo, tipo em maiúsculo) nem ascription de expressão (não há
expressão à esquerda do `::` — só o nome).

### 5.3. Typeck

Em `infer/expr.rs`, caminho `Let` e `Var`, antes de inferir o valor:

```rust
if let Some(ty_expr) = &ty {
    let target_ty = resolve_type_expr(
        &ty_expr.node, env,
        ctx.interface_registry, ctx.struct_registry, None,
    );
    // Inferir valor com target_ty como hint (ret-directed)
    let typed_value = infer_expr_hinted(
        &value.node, &value.span, env, ctx, false, Some(&target_ty),
    )?;
    // Verificar compatibilidade
    if !type_implements(&typed_value.ty, &target_ty, ...) {
        return Err(MiddleError::TypeMismatch { ... });
    }
    // Binding fica com target_ty, não com typed_value.ty
    env.define(name, target_ty, "__local__");
} else {
    // Caminho atual: inferir sem hint, definir com tipo do valor
}
```

O hint `Some(&target_ty)` permite que o ret-directed dispatch selecione a
instância correta de família polimórfica quando o target é interface — mesmo
mecanismo que já existe para ascription de expressão.

### 5.4. Re-binding check

O check existente (`existing_ty != val_ty`, linhas 849-858) compara o tipo do
binding com o tipo do valor. Com ascription, o tipo do binding é o anotado
(`target_ty`), não o inferido. O check deve comparar `existing_ty` (que já é
o tipo do binding) com `target_ty` (o novo tipo anotado, se houver) ou com
`val_ty` (se não houver ascription). Se `existing_ty != (target_ty | val_ty)`,
erro de re-binding divergente.

Para interface: `var x::NUM := 0` seguido de `var x := 3.14` — `existing_ty`
é `NUM`, `val_ty` é `Float`. O check deve verificar se `Float` implementa
`NUM`, não se `Float == NUM`. Isso é uma generalização do check existente:
de `==` para `type_implements`.

## 6. Casos de teste

### 6.1. Parser

- `let x::Int := 42` — parseia com `ty = Some(Named("Int"))`
- `var y::Text := "hello"` — parseia com `ty = Some(Named("Text"))`
- `var z::NUM := 0` — parseia com `ty = Some(Named("NUM"))`
- `let x := 42` — parseia com `ty = None` (comportamento atual)
- `var x::Int := "hello"` — parse OK, erro no typeck (não no parser)
- `let (x, y)::Int := expr` — ERRO: ascription em destructuring não suportado
  (parse error: `::` inesperado após `)`)

### 6.2. Typeck — tipo concreto

- `let x::Int := 42` — OK, x: Int
- `let x::Int := 3.14` — ERRO: Float ≠ Int
- `var x::Int := 42; var x := + x 1` — OK, preserva Int
- `var x::Int := 42; var x := "hello"` — ERRO: re-binding divergente

### 6.3. Typeck — widening de interface

- `var z::NUM := 0` — OK, z: NUM (Int implementa NUM)
- `var z::NUM := 3.14` — OK, z: NUM (Float implementa NUM)
- `var z::NUM := "hello"` — ERRO: Text não implementa NUM
- `var z::NUM := 0; var z := 3.14` — OK, preserva NUM
- `var z::NUM := 0; var z::ORD := "hello"` — ERRO: re-binding divergente
- `let z::NUM := 0; let n::Int := z` — ERRO: NUM não é Int (sem downcast)

### 6.4. Sem ascription (regressão)

- `var x := 42; var x := + x 1` — OK (comportamento atual)
- `var x := 42; var x := "hello"` — ERRO (comportamento atual)
- `let x := 42` — OK, x: Int (comportamento atual)

## 7. Decisões de design

### D1: `::` no binding é anotação, não operação

`var x::Int := 0` anota o binding com tipo `Int`. Não converte `0` para `Int`
— `0` já é `Int`. A ascription é uma **restrição** no destino: o binding só
aceita valores compatíveis com `Int`. Se o RHS não for compatível, erro.

**Alternativa considerada:** `var x := 0 :: Int` (ascription de valor). Já
funciona hoje, mas é semântica diferente: opera sobre o valor, não sobre o
binding. Não suporta widening de interface. Rejeitada como substituta.

### D2: Widening requer ascription explícita

Sem ascription, `var` não faz widening implícito. O tipo é inferido e travado.
Para aceitar múltiplos tipos no mesmo binding, declare a interface explicitamente.

**Motivo:** inferência implícita com widening seria imprevisível — `var x := 0`
poderia ser `Int` ou `NUM` dependendo de contexto. Explicit is better than
implicit.

### D6: `var` sem ascription é sempre travado no primeiro binding

`var` sem ascription infere o tipo do primeiro binding e o trava. Re-bindings
devem preservar o tipo (`existing_ty != val_ty` → `TypeMismatch`). Esta não é
uma limitação temporária — é uma decisão de design.

Permitir que `var` mude de tipo livremente entre re-bindings exigiria análise
de fluxo (join de tipos em pontos de convergência após match/loop) que o
typeck atual, sendo um tree walk sem CFG, não pode fazer de forma sound. O
flat scope de Kata (match/loop não abrem escopo filho) agrava o problema:
re-bindings em braços de match persistem no env externo, tornando o tipo de
`var` após um match potencialmente ambíguo.

Uma maquinaria Z3 exclusiva para var (separada do `PathConditionCtx`, que
filtra mutáveis por soundness) poderia resolver o caso de match joins, mas o
caso de loops exige fixpoint — que nem Z3 nem Maranget computam
naturalmente. A solução completa exige CFG, que fica para uma iteração futura.

O widening via ascription de interface (`var z::NUM := 0`) já flexibiliza
suficientemente o uso de `var` com tipos diferentes, sem introduzir
complexidade de análise de fluxo no typeck.

### D3: Re-binding preserva tipo, não estreita

Re-binding sem ascription preserva o tipo do binding original (seja inferido
ou anotado). Re-binding com ascription diferente é erro. Para mudar o tipo,
use nome diferente.

**Alternativa considerada:** permitir estreitamento no re-binding (de `NUM`
para `Int`). Rejeitada: tornaria o tipo do binding imprevisível entre pontos
do escopo, quebrando o join sound que o check atual protege.

### D4: Ascription em destructuring fica de fora

`let (x, y)::(Int, Text) := expr` não é suportado neste PRD. A sintaxe é
ambígua com ascription de tupla (`expr :: (Int, Text)`). Se necessário, PRD
separado.

### D5: `let` com ascription é idempotente com re-declaração

`let` é único por escopo — não pode re-declarar. Então `let x::Int := 42`
seguido de `let x::Int := 99` é `DuplicateDecl`, não `TypeMismatch`. A
ascription não muda a regra de unicidade do `let`.

## 8. Fases

### Fase 1: Parser + AST
- Adicionar `ty: Option<Spanned<TypeExpr>>` em `Let` e `Var`.
- Modificar `parse_let` e `parse_var` para consumir `::Tipo` opcional.
- Atualizar todos os construtores de `Let`/`Var` em desugar, transform,
  reflection, action_hooks (usam `..` para campos não relevantes).
- Testes parser.

### Fase 2: Typeck — tipo concreto
- Em `infer/expr.rs`, caminho `Let` e `Var`: usar `ty` como hint quando
  presente, definir binding com tipo anotado.
- Verificar compatibilidade RHS ↔ tipo anotado.
- Re-binding check compara tipo do binding (anotado) com tipo do valor.
- Testes typeck com tipos concretos.

### Fase 3: Typeck — widening de interface
- Verificar que `type_implements(val_ty, interface_ty)` quando o tipo
  anotado é interface.
- Re-binding check generalizado: `type_implements` em vez de `==`.
- Testes typeck com interfaces (NUM, ORD, EQ, etc.).

### Fase 4: Manual
- Atualizar cap 4 (Bindings e Tipos) com sintaxe `::Tipo` em bindings.
- Adicionar seção sobre widening de interface.
- Exemplos.

### Fase 5: TODO.md
- Remover item "Ascription em binding de `var`" do TODO.