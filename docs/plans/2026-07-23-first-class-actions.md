# First-Class Actions — Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Permitir que Actions sejam valores de primeira classe — referenciadas sem invocação, armazenadas em `let`, passadas como parâmetros para outras Actions, e invocadas indiretamente.

**Architecture:** Nova variante `Ty::Action(Vec<Ty>, Box<Ty>)` separada de `Ty::Function` (ABIs diferentes). O typeck ganha um terceiro fallback no `Expr::Ident` arm: quando `name` está no DispatchTable com `is_action: true`, produz `Ty::Action`. O codegen distingue call direto (lookup em `kata_refs`) de call indireto (fn_ptr da variável via `indirect_callee`). O parser ganha nova variante `TypeExpr::ActionType` para a sintaxe `Action(Params) => Ret`.

**Tech Stack:** Rust, Cranelift, Cargo workspace.

**PRD:** `docs/PRD-first-class-actions.md`

**Decisões fechadas (verificação de premissas):**
- D1: `Ty::Action` separada de `Ty::Function` (ABIs semanticamente diferentes) — confirmado pelo codebase
- D2: `indirect_callee: Option<Box<Spanned<TypedExpr>>>` no `TypedExprKind::ActionCall` (typeck sabe distinguir, codegen faz match simples)
- L1: Sintaxe `Action(Params) => Ret` com nova variante `TypeExpr::ActionType` no parser. `Token::FatArrow` já existe.
- L4: `Ty::Action` mapeia para `TypeShape::Func` (mesmo layout i64). Codegen decide ABI pelo `Ty`, não pelo `TypeShape`.
- Path `escape.rs` corrigido: `crates/kata-core/src/escape.rs` (não `kata-inference/src/infer/escape.rs`).

---

## Fase 1: Core — `Ty::Action`

### Task 1: Adicionar `Ty::Action` ao enum `Ty`

**Objective:** Criar a variante de tipo que representa Actions como valores first-class.

**Files:**
- Modify: `crates/kata-core/src/ty.rs:14-51` (enum `Ty`)

**Step 1: Adicionar variante**

Após `Ty::Function(Vec<Ty>, Box<Ty>)` (linha 24), adicionar:

```rust
/// Action como valor first-class.
/// params: tipos dos parâmetros (sem nomes).
/// ret: tipo de retorno.
/// Separada de Ty::Function porque as ABIs são semanticamente diferentes:
/// Function: (captures_ptr, args) -> ret — pura, sem scheduler
/// Action: (fiber_arena, caller_arena, args_ptr) -> i64 — impura, scheduler M:N
Action(Vec<Ty>, Box<Ty>),
```

**Step 2: Verificar compilação**

```bash
cargo check --workspace --all-targets 2>&1 | head -40
```

Esperado: erros de `non-exhaustive match` em vários arquivos. Anotar a lista de arquivos para as próximas tasks.

**Step 3: Commit**

```bash
git add crates/kata-core/src/ty.rs
git commit -m "feat: add Ty::Action variant for first-class actions"
```

---

### Task 2: Mapear `Ty::Action` em `TypeShape`

**Objective:** O shape de `Ty::Action` é `TypeShape::Func` (fn_ptr é i64, mesmo layout de function pointer).

**Files:**
- Modify: `crates/kata-core/src/shape.rs:64-130` (`impl Ty::to_shape`)

**Step 1: Adicionar arm**

Após o arm `Ty::Function(params, ret)` (linha 76-79), adicionar:

```rust
Ty::Action(params, ret) => TypeShape::Func {
    params: params.iter().map(|t| t.to_shape()).collect(),
    ret: Box::new(ret.to_shape()),
},
```

**Step 2: Verificar**

```bash
cargo check -p kata-core --all-targets
```

Esperado: PASS.

**Step 3: Commit**

```bash
git add crates/kata-core/src/shape.rs
git commit -m "feat: map Ty::Action to TypeShape::Func"
```

---

### Task 3: `apply_subs` para `Ty::Action`

**Objective:** O monomorphizador precisa substituir variáveis de tipo dentro de `Ty::Action`.

**Files:**
- Modify: `crates/kata-inference/src/infer/generics.rs:166-210` (`apply_subs`)

**Step 1: Adicionar arm**

Após o arm `Ty::Function` (linha 188-191), adicionar:

```rust
Ty::Action(params, ret) => Ty::Action(
    params.iter().map(|p| apply_subs(p, subs)).collect(),
    Box::new(apply_subs(ret, subs)),
),
```

**Step 2: Verificar**

```bash
cargo check -p kata-inference --all-targets
```

Esperado: PASS (ou menos erros de non-exhaustive).

**Step 3: Commit**

```bash
git add crates/kata-inference/src/infer/generics.rs
git commit -m "feat: apply_subs for Ty::Action"
```

---

### Task 4: `naming.rs` — nome de monomorfização para `Ty::Action`

**Objective:** O monomorphizador precisa gerar nomes únicos para instâncias de `Ty::Action`.

**Files:**
- Modify: `crates/kata-monomorph/src/naming.rs:37-44` (`ty_to_string`)

**Step 1: Adicionar arm**

Após o arm `Ty::Function(params, ret)` (linha 37-44), adicionar:

```rust
Ty::Action(params, ret) => {
    let p = params
        .iter()
        .map(ty_to_string)
        .collect::<Vec<_>>()
        .join("_");
    format!("Act_{p}_{}", ty_to_string(ret))
}
```

**Step 2: Verificar**

```bash
cargo check -p kata-monomorph --all-targets
```

**Step 3: Commit**

```bash
git add crates/kata-monomorph/src/naming.rs
git commit -m "feat: monomorph naming for Ty::Action"
```

---

### Task 5: Corrigir demais matchs exaustivos em `Ty`

**Objective:** Adicionar arms para `Ty::Action` em todos os matchs exaustivos que o compilador apontar.

**Files (confirmar via `cargo check`):**
- `crates/kata-inference/src/patterns.rs` (pattern matching de tipos)
- `crates/kata-inference/src/infer/generics.rs` (`unify_one` — linha 48)
- Qualquer outro arquivo que o `cargo check` apontar

**Step 1: Rodar cargo check e coletar erros**

```bash
cargo check --workspace --all-targets 2>&1 | grep "non-exhaustive\|not covered"
```

**Step 2: Para cada arquivo com erro, adicionar arm**

Em `patterns.rs:384` (match que rejeita tipos não-patternable):
```rust
Ty::Action(_, _) => Err(MiddleError::TypeMismatch {
    // mensagem existente
}),
```

Em `generics.rs:unify_one` — `Ty::Action` unifica structuralmente (params e ret):
```rust
Ty::Action(a_params, a_ret) => {
    if let Ty::Action(b_params, b_ret) = other {
        if a_params.len() != b_params.len() { return Err(...); }
        for (a, b) in a_params.iter().zip(b_params.iter()) {
            unify(a, b, subs)?;
        }
        unify(a_ret, b_ret, subs)
    } else {
        Err(...)
    }
}
```

**Step 3: Verificar**

```bash
cargo check --workspace --all-targets
```

Esperado: PASS (0 erros de non-exhaustive).

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: handle Ty::Action in all exhaustive matches"
```

---

## Fase 2: Parser — `TypeExpr::ActionType`

### Task 6: Adicionar `TypeExpr::ActionType` ao AST

**Objective:** Nova variante de TypeExpr para representar `Action(Params) => Ret`.

**Files:**
- Modify: `crates/kata-ast/src/expr.rs:329-370` (enum `TypeExpr`)

**Step 1: Adicionar variante**

Após `TypeExpr::Func { params, ret }` (linha 353-356), adicionar:

```rust
/// `Action(Param1, Param2, ...) => Ret` — tipo de Action first-class.
/// Espelha a assinatura de actions, sem nomes dos params.
ActionType {
    params: Vec<Spanned<TypeExpr>>,
    ret: Box<Spanned<TypeExpr>>,
},
```

**Step 2: Verificar**

```bash
cargo check -p kata-ast --all-targets
```

Esperado: erros de non-exhaustive em parsers e resolvers. Anotar para próximas tasks.

**Step 3: Commit**

```bash
git add crates/kata-ast/src/expr.rs
git commit -m "feat: add TypeExpr::ActionType for Action type syntax"
```

---

### Task 7: Parser — reconhecer `Action(Params) => Ret`

**Objective:** O parser de tipos deve reconhecer `Action(Int) => Unit` e produzir `TypeExpr::ActionType`.

**Files:**
- Modify: `crates/kata-parser/src/types.rs:29-177` (`parse_type_expr_inner`)

**Step 1: Adicionar reconhecimento**

No match `Token::Ident(name)` (linha 32), após verificar `Self` e antes de verificar `::`, adicionar:

```rust
Token::Ident(name) if name == "Action" => {
    self.advance(); // consome "Action"
    // Espera ( params )
    self.expect(&Token::LParen, "\"(\" após Action")?;
    let mut params = Vec::new();
    // Skip newlines after (
    while matches!(self.peek(), Token::StmtSe) {
        self.advance();
    }
    if matches!(self.peek(), Token::RParen) {
        self.advance();
    } else {
        loop {
            let ty = self.parse_type_expr()?;
            params.push(ty);
            if matches!(self.peek(), Token::Comma) {
                self.advance();
                while matches!(self.peek(), Token::StmtSe) {
                    self.advance();
                }
                continue;
            }
            break;
        }
        self.expect(&Token::RParen, "\")\"")?;
    }
    // Espera => ret
    self.expect(&Token::FatArrow, "'=>' após Action(params)")?;
    let ret = self.parse_type_expr()?;
    let span = start.cover(ret.span);
    return Ok(Spanned::new(
        TypeExpr::ActionType {
            params,
            ret: Box::new(ret),
        },
        span,
    ));
}
```

Isso precisa ser inserido **antes** do braço `Token::Ident(name)` genérico. Pode ser feito como um guard pattern no match existente ou como um `if` antes do match:

```rust
// Antes do match self.peek().clone():
if matches!(self.peek(), Token::Ident(name) if name == "Action") {
    // ... código acima ...
}
```

Alternativamente, adicionar como primeiro arm no match com guard. Verificar qual abordagem compila melhor com o compilador Rust (guards em arms de match com bindings exigem `@` binding ou re-match).

**Abordagem recomendada:** Inserir um `if` antes do `match`:

```rust
fn parse_type_expr_inner(&mut self) -> Result<Spanned<TypeExpr>, FrontendError> {
    let start = self.peek_span();
    
    // NOVO: Action(Params) => Ret
    if matches!(self.peek(), Token::Ident(name) if name == "Action") {
        self.advance(); // consome "Action"
        // ... código acima ...
    }
    
    match self.peek().clone() {
        // ... match existente ...
    }
}
```

**Step 2: Escrever teste de parser**

Arquivo: `crates/kata-parser/tests/parser_test/action_type_syntax.rs`

```rust
use kata_parser::parse_module;
use kata_ast::{expr::TypeExpr, item::Item};

#[test]
fn parse_action_type_no_params() {
    let src = "action f (g :: Action() => Unit) => Unit\n    g!()\n";
    let module = parse_module(src).expect("parse ok");
    // Verificar que o param tem TypeExpr::ActionType
}

#[test]
fn parse_action_type_with_params() {
    let src = "action f (g :: Action(Int) => Unit, n :: Int) => Unit\n    g!(n)\n";
    let module = parse_module(src).expect("parse ok");
    // Verificar params
}

#[test]
fn parse_action_type_multiple_params() {
    let src = "action f (g :: Action(Int Text) => Boolean) => Unit\n    let b := g!(42 \"hi\")\n";
    let module = parse_module(src).expect("parse ok");
}

#[test]
fn parse_action_type_with_commas() {
    let src = "action f (g :: Action(Int, Text) => Boolean) => Unit\n    g!(42 \"hi\")\n";
    let module = parse_module(src).expect("parse ok");
}
```

**Step 3: Rodar testes**

```bash
cargo test -p kata-parser --test parser_test action_type_syntax 2>&1
```

Esperado: PASS.

**Step 4: Commit**

```bash
git add crates/kata-parser/src/types.rs crates/kata-parser/tests/parser_test/action_type_syntax.rs
git commit -m "feat: parse Action(Params) => Ret type syntax"
```

---

### Task 8: Resolution — `TypeExpr::ActionType` → `Ty::Action`

**Objective:** O `type_resolve.rs` resolve `TypeExpr::ActionType` para `Ty::Action`.

**Files:**
- Modify: `crates/kata-resolution/src/type_resolve.rs:58` (após `TypeExpr::Func`)

**Step 1: Adicionar arm**

Após o arm `TypeExpr::Func { params, ret }` (linha 58-65), adicionar:

```rust
TypeExpr::ActionType { params, ret } => {
    let param_types: Vec<Ty> = params
        .iter()
        .map(|t| resolve_type_expr(&t.node, env, iface_reg))
        .collect();
    let return_type = resolve_type_expr(&ret.node, env, iface_reg);
    Ty::Action(param_types, Box::new(return_type))
}
```

**Step 2: Verificar**

```bash
cargo check -p kata-resolution --all-targets
```

Esperado: PASS.

**Step 3: Commit**

```bash
git add crates/kata-resolution/src/type_resolve.rs
git commit -m "feat: resolve TypeExpr::ActionType to Ty::Action"
```

---

## Fase 3: Inference — referência de Action como valor

### Task 9: `Expr::Ident` fallback para Action no DispatchTable

**Objective:** Quando `Ident { name }` não está em `env` (variável local) nem no `EnumRegistry` (variante), mas está no `DispatchTable` com `is_action: true`, produz `TypedExpr` com `ty: Ty::Action`.

**Files:**
- Modify: `crates/kata-inference/src/infer/expr.rs:136-148` (arm `Expr::Ident`)

**Step 1: Adicionar terceiro fallback**

Substituir o arm `Expr::Ident { name }`:

```rust
Expr::Ident { name } => {
    // Caminho 1: variável local no TypeEnv.
    if let Some(ty) = env.lookup(name).cloned() {
        (
            ty,
            TypedExprKind::Ident { name: name.clone() },
            Effect::Puro,
        )
    } else {
        // Caminho 2: variante unitária desqualificada.
        match resolve_unqual_variant(name, span, ctx)? {
            // ... variante encontrada — retorna como antes ...
            // Se não é variante, cai para Caminho 3.
        }
    }
}
```

**Implementação concreta:** Reescrever o arm completo. O `resolve_unqual_variant` retorna erro se não encontra. Precisamos interceptar esse erro e tentar o caminho 3. Abordagem:

```rust
Expr::Ident { name } => {
    // Caminho 1: variável local no TypeEnv.
    if let Some(ty) = env.lookup(name).cloned() {
        return Ok(TypedExpr {
            span: *span,
            ty,
            tail_pos,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident { name: name.clone() },
        });
    }
    // Caminho 2: variante unitária desqualificada.
    if let Ok(variant) = resolve_unqual_variant(name, span, ctx) {
        return Ok(variant);
    }
    // Caminho 3: Action no DispatchTable (first-class).
    if let Some(overloads) = ctx.table.get_overloads(name) {
        let action_overloads: Vec<_> = overloads.iter().filter(|o| o.is_action).collect();
        if !action_overloads.is_empty() {
            // Primeira versão: se múltiplos overloads com params diferentes,
            // erro (ambíguo). Se um único overload (ou todos com mesmo tipo),
            // usa o primeiro.
            // TODO: overloading de Actions — resolution por tipo esperado.
            let overload = action_overloads[0];
            return Ok(TypedExpr {
                span: *span,
                ty: Ty::Action(
                    overload.params.clone(),
                    Box::new(overload.ret.clone()),
                ),
                tail_pos,
                escape: EscapeTarget::Local, // fn_ptr é i64 inline (D10)
                effect: Effect::Puro, // referenciar não executa
                kind: TypedExprKind::Ident { name: name.clone() },
            });
        }
    }
    // Caminho 4: unbound name.
    Err(MiddleError::UnboundName {
        name: name.clone(),
        span: span.into(),
    })
}
```

**Atenção:** O arm `Expr::Ident` hoje retorna uma tupla `(Ty, TypedExprKind, Effect)`, não `Ok(TypedExpr)`. Precisa seguir o padrão da função `infer_expr_hinted` — verificar se o return type é tupla ou `Result<TypedExpr>`. Adaptar o código acima ao padrão existente.

**Step 2: Escrever teste**

Arquivo: `crates/kata-driver/tests/first_class_actions.rs`

```rust
#[test]
fn action_reference_has_action_type() {
    let src = "
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    echo!(0)  // só para compilar
";
    // Compilar e verificar que `f` tem tipo Ty::Action([Int], Unit)
    // Pode ser um teste de compilação (não precisa executar)
}
```

**Step 3: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions action_reference 2>&1
```

**Step 4: Commit**

```bash
git add crates/kata-inference/src/infer/expr.rs crates/kata-driver/tests/first_class_actions.rs
git commit -m "feat: Ident fallback to Action in DispatchTable (first-class reference)"
```

---

### Task 10: `ActionCall` com `indirect_callee`

**Objective:** Adicionar campo `indirect_callee` ao `TypedExprKind::ActionCall` para suportar invocação indireta.

**Files:**
- Modify: `crates/kata-inference/src/typed.rs:185-194` (`TypedExprKind::ActionCall`)
- Modify: todos os construtores de `ActionCall` no inference

**Step 1: Adicionar campo ao TAST**

Em `typed.rs:185-194`:

```rust
ActionCall {
    callee: String,
    args: Box<Spanned<TypedExpr>>,
    caller_arena: i64,
    ffi_symbol: Option<String>,
    /// None = call direto (lookup em kata_refs).
    /// Some(expr) = call indireto (fn_ptr da expressão).
    indirect_callee: Option<Box<Spanned<TypedExpr>>>,
},
```

**Step 2: Atualizar todos os construtores**

Buscar todos os lugares que constroem `TypedExprKind::ActionCall` e adicionar `indirect_callee: None`:

```bash
cargo check --workspace --all-targets 2>&1 | grep "missing field.*indirect_callee"
```

Para cada arquivo apontado, adicionar `indirect_callee: None` aos construtores existentes.

No inference, quando `callee` é uma variável com `ty: Ty::Action` (não nome estático), o typeck deve:
1. Lowerar a variável para `TypedExpr`
2. Construir `ActionCall` com `indirect_callee: Some(Box::new(Spanned::new(var_expr, span)))`

**Step 3: Verificar**

```bash
cargo check --workspace --all-targets
```

Esperado: PASS.

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: add indirect_callee to ActionCall TAST for indirect invocation"
```

---

### Task 11: Inference — invocação indireta `f!(args)`

**Objective:** Quando `f` é variável com `ty: Ty::Action`, `f!(args)` deve produzir `ActionCall` com `indirect_callee: Some(...)`.

**Files:**
- Modify: `crates/kata-inference/src/infer/action_call.rs` (função de inferência de ActionCall)

**Step 1: Identificar onde ActionCall é inferido**

O `infer_action_call` hoje faz:
1. Extrai `callee` como string de `ActionCall { callee, args }` do AST
2. Faz lookup em `ctx.table.get_overloads(callee)`
3. Valida que é action (`is_action`)
4. Infere args

Adicionar lógica: se `callee` **não** está no DispatchTable mas **está** em `env` com `ty: Ty::Action`, é invocação indireta.

**Implementação:**

No início de `infer_action_call` (ou função equivalente), após falhar o lookup no DispatchTable:

```rust
// Se não está no DispatchTable, tenta invocação indireta (variável com Ty::Action).
if !ctx.table.has_function(callee) {
    if let Some(Ty::Action(param_types, ret_ty)) = env.lookup(callee).cloned() {
        // Invocação indireta.
        let typed_args = infer_expr(&args.node, &args.span, env, ctx, false)?;
        // Validar que args matcham param_types.
        // Construir ActionCall com indirect_callee.
        let callee_expr = TypedExpr {
            span: *span,
            ty: Ty::Action(param_types.clone(), Box::new(ret_ty.clone())),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident { name: callee.to_string() },
        };
        return Ok(ActionDispatch::Complete(TypedExpr {
            span: *span,
            ty: ret_ty,
            tail_pos,
            escape: EscapeTarget::Local,
            effect: Effect::Impuro,
            kind: TypedExprKind::ActionCall {
                callee: callee.to_string(),
                args: Box::new(Spanned::new(typed_args, args.span)),
                caller_arena: 0,
                ffi_symbol: None,
                indirect_callee: Some(Box::new(Spanned::new(callee_expr, *span))),
            },
        }));
    }
}
```

**Step 2: Escrever teste**

```rust
#[test]
fn indirect_invocation() {
    let src = "
action worker (n :: Int) => Unit
    echo!(+ n 1)

action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)

action main => Unit
    dispatcher!(worker, 42)
";
    // Deve compilar e executar: imprime 43
}
```

**Step 3: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions indirect_invocation 2>&1
```

**Step 4: Commit**

```bash
git add crates/kata-inference/src/infer/action_call.rs crates/kata-driver/tests/first_class_actions.rs
git commit -m "feat: indirect Action invocation via variable with Ty::Action"
```

---

### Task 12: Proibir `Ty::Action` em `data`, canal, e função pura

**Objective:** O typeck rejeita `Ty::Action` em posições de `data` e canal.

**Files:**
- Modify: `crates/kata-inference/src/infer/constructors.rs` (StructConstruct)
- Modify: `crates/kata-inference/src/infer/csp.rs` (ChannelSend)
- Modify: `crates/kata-inference/src/infer/expr.rs` (posição de função pura)

**Step 1: Proibir em StructConstruct**

Em `constructors.rs`, onde `StructConstruct` é inferido, adicionar verificação:

```rust
// Rejeitar Ty::Action em campos de struct.
if fields.iter().any(|(_, ty)| matches!(ty, Ty::Action(..))) {
    return Err(MiddleError::TypeMismatch {
        expected: "tipo de dados (sem Action) em campo de data".into(),
        found: "Action não é permitida em data — Actions são comportamento, não informação".into(),
        span: span.into(),
    });
}
```

**Step 2: Proibir em ChannelSend**

Em `csp.rs`, onde `ChannelSend` é inferido, adicionar:

```rust
if matches!(value.node.ty, Ty::Action(..)) {
    return Err(MiddleError::TypeMismatch {
        expected: "tipo de dados (sem Action) em canal".into(),
        found: "Action não é permitida em canal — canais transportam dados, não comportamento".into(),
        span: span.into(),
    });
}
```

**Step 3: Proibir como param de função pura**

Em `expr.rs` ou `apply.rs`, onde funções puras são despachadas, verificar se algum arg tem `ty: Ty::Action`. Se a função destino não é Action (`is_action: false`), rejeitar.

**Step 4: Escrever testes**

```rust
#[test]
#[should_panic] // ou expect compile error
fn action_in_data_rejected() {
    let src = "
action worker (n :: Int) => Unit
    echo!(n)
data Wrapper (job :: Action(Int) => Unit)
";
    // Deve falhar na compilação
}

#[test]
#[should_panic]
fn action_in_channel_rejected() {
    let src = "
action worker (n :: Int) => Unit
    echo!(n)
action main => Unit
    let (tx, rx) := channel!()
    tx !> worker
";
    // Deve falhar na compilação
}
```

**Step 5: Rodar testes**

```bash
cargo test -p kata-driver --test first_class_actions rejection 2>&1
```

**Step 6: Commit**

```bash
git add -A
git commit -m "feat: reject Ty::Action in data, channel, and pure function params"
```

---

## Fase 4: Fork com Action como valor

### Task 13: `Fork` recebe Action como `TypedExpr` (não string)

**Objective:** O `infer_fork_builtin` deve aceitar variável com `ty: Ty::Action` como primeiro argumento, não apenas `Expr::Ident` direto.

**Files:**
- Modify: `crates/kata-inference/src/infer/action_call.rs:300-397` (`infer_fork_builtin`)
- Modify: `crates/kata-inference/src/typed.rs:372-375` (`TypedExprKind::Fork`)

**Step 1: Mudar `Fork` no TAST**

Em `typed.rs:372-375`, mudar de:

```rust
Fork {
    action_name: String,
    args: Box<Spanned<TypedExpr>>,
},
```

Para:

```rust
Fork {
    action_name: String,
    /// Expression que avalia para o fn_ptr da Action.
    /// Para Ident direto, é o Ident. Para variável, é a variável.
    action_expr: Box<Spanned<TypedExpr>>,
    args: Box<Spanned<TypedExpr>>,
},
```

**Step 2: Atualizar `infer_fork_builtin`**

Em `action_call.rs:300-397`, mudar a lógica do primeiro elemento:

```rust
// Antes: apenas Expr::Ident { name } => name.clone()
// Agora: inferir o expression e verificar se tem ty: Ty::Action
let action_expr = infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
let action_name = match &action_expr.kind {
    TypedExprKind::Ident { name } => name.clone(),
    _ => {
        // Variável com Ty::Action — usa nome da variável como action_name
        // (para recursion check e tree shaking). Pode ser melhor nome genérico.
        // Para tree shaking, o action_name precisa ser rastreável.
        // Se é let f := worker; fork!(f, ...), o tree shaking já marcou
        // worker como alcançável no ponto do let.
        format!("__indirect_fork")
    }
};

// Verifica que action_expr tem ty: Ty::Action
if !matches!(action_expr.ty, Ty::Action(..)) {
    return Err(MiddleError::TypeMismatch {
        expected: "Action (Ident ou variável com tipo Action)".into(),
        found: format!("{}", action_expr.ty),
        span: elements[0].span.into(),
    });
}
```

**Step 3: Atualizar todos os construtores de Fork**

```bash
cargo check --workspace --all-targets 2>&1 | grep "missing field.*action_expr"
```

Adicionar `action_expr` a cada construtor de `Fork`.

**Step 4: Atualizar recursion.rs e tree-shaking para novo Fork**

Em `recursion.rs:229`:
```rust
TypedExprKind::Fork { action_name, action_expr, args } => {
    // Se action_name é __indirect_fork, não registra aresta estática.
    // O def-use já tratou a alcançabilidade.
    if action_name != "__indirect_fork" {
        out.push((action_name.clone(), expr.span));
    }
    collect_action_calls(&args.node, &args.span, out);
    // action_expr pode conter ActionCalls? Não — é um Ident.
}
```

Em `tree-shaking/lib.rs:241-247`:
```rust
TypedExprKind::Fork { action_name, action_expr, args, .. } => {
    if action_name != "__indirect_fork" {
        reached_actions.insert(action_name.clone());
    }
    // action_expr já foi processado pelo collect_refs no ponto do let.
    collect_refs(&args.node, reached_fns, reached_actions, fn_names);
}
```

**Step 5: Escrever teste**

```rust
#[test]
fn fork_with_variable() {
    let src = "
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    fork!(f, (42,))
";
    // Deve compilar e executar: fiber spawna worker(42)
}
```

**Step 6: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions fork_with_variable 2>&1
```

**Step 7: Commit**

```bash
git add -A
git commit -m "feat: Fork accepts Action as TypedExpr (first-class fork)"
```

---

## Fase 5: Codegen — invocação indireta

### Task 14: Codegen — lowering de referência de Action (`Ident` com `ty: Ty::Action`)

**Objective:** O codegen lowera `Ident { name, ty: Ty::Action }` para o fn_ptr (i64) da Action.

**Files:**
- Modify: `crates/kata-codegen/src/lowering/expr.rs` (arm `TypedExprKind::Ident`)

**Step 1: Adicionar tratamento**

No arm `TypedExprKind::Ident` em `expr.rs`, verificar se `expr.ty` é `Ty::Action`. Se sim, obter o fn_ptr via `GlobalValue::Symbol` (mesmo mecanismo de Fork):

```rust
TypedExprKind::Ident { name } => {
    if let Ty::Action(_, _) = &expr.ty {
        // First-class Action reference → fn_ptr (i64).
        let key = (name.clone(), /* param_types */, /* ret_ty */);
        let callee_fid = ctx.kata_ids.get(&key).ok_or_else(|| {
            CodegenError::UnsupportedNode(format!(
                "Action reference: `{name}` não encontrado em kata_ids"
            ))
        })?;
        let func_ref = ctx.module.declare_func_in_func(*callee_fid, ctx.builder.func);
        let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
        let func_gv = ctx.builder.func.create_global_value(
            cranelift_codegen::ir::GlobalValueData::Symbol {
                name: ext_func_name,
                offset: 0.into(),
                colocated: true,
                tls: false,
            },
        );
        let fn_ptr = ctx.builder.ins().global_value(
            ctx.module.target_config().pointer_type(),
            func_gv,
        );
        Ok(fn_ptr)
    } else {
        // ... behavior existente para Ident normal ...
    }
}
```

**Nota:** A chave `kata_ids` é `(String, Vec<Ty>, Ty)` — precisa extrair `param_types` e `ret_ty` de `Ty::Action(params, ret)`.

**Step 2: Verificar**

```bash
cargo check -p kata-codegen --all-targets
```

**Step 3: Commit**

```bash
git add crates/kata-codegen/src/lowering/expr.rs
git commit -m "feat: codegen for Action reference (Ident with Ty::Action → fn_ptr)"
```

---

### Task 15: Codegen — lowering de invocação indireta (`ActionCall` com `indirect_callee`)

**Objective:** O codegen emite `call_indirect` quando `indirect_callee` é `Some`.

**Files:**
- Modify: `crates/kata-codegen/src/lowering/action_call.rs:22-162` (`lower_action_call`)

**Step 1: Adicionar branch para invocação indireta**

No início de `lower_action_call`, após extrair os campos, verificar `indirect_callee`:

```rust
pub(crate) fn lower_action_call(
    expr: &TypedExpr,
    callee: &str,
    args: &kata_ast::Spanned<TypedExpr>,
    ffi_symbol: &Option<String>,
    indirect_callee: &Option<Box<Spanned<TypedExpr>>>,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let args_ptr = super::expr::lower_expr(&args.node, ctx)?;

    if let Some(sym_name) = ffi_symbol {
        // ... FFI builtin (código existente) ...
    } else if let Some(callee_expr) = indirect_callee {
        // NOVO: invocação indireta.
        // 1. Lowerar callee_expr → fn_ptr (i64)
        let fn_ptr = super::expr::lower_expr(&callee_expr.node, ctx)?;
        // 2. Preparar args: [fiber_arena, caller_arena, args_ptr]
        let fiber_arena_val = ctx.fiber_arena
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
        let caller_arena_val = match expr.escape {
            EscapeTarget::Local => fiber_arena_val,
            EscapeTarget::Caller => ctx.caller_arena
                .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
        };
        let arg_values = [fiber_arena_val, caller_arena_val, args_ptr];
        // 3. call_indirect
        let call_inst = ctx.builder.ins().call_indirect(
            ctx.module.target_config().pointer_type(),
            fn_ptr,
            &arg_values,
        );
        let result = ctx.builder.inst_results(call_inst)[0];
        if expr.ty == Ty::float() {
            Ok(ctx.builder.ins().bitcast(F64, MemFlagsData::new(), result))
        } else {
            Ok(result)
        }
    } else {
        // ... call direto (código existente) ...
    }
}
```

**Step 2: Verificar**

```bash
cargo check -p kata-codegen --all-targets
```

**Step 3: Escrever teste E2E**

```rust
#[test]
fn dispatch_strategy_e2e() {
    let src = "
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)
    dispatcher!(worker_b, 42)
";
    // Deve compilar e executar: imprime 43 e 44
}
```

**Step 4: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions dispatch_strategy 2>&1
```

**Step 5: Commit**

```bash
git add crates/kata-codegen/src/lowering/action_call.rs crates/kata-driver/tests/first_class_actions.rs
git commit -m "feat: codegen for indirect Action invocation (call_indirect)"
```

---

## Fase 6: Recursion check — def-use interprocedural

### Task 16: Recursion check propaga call sites para params invocados

**Objective:** Quando `dispatcher!(worker_a, 42)` passa `worker_a` como param `job` que é invocado (`job!(payload)`), registrar aresta `dispatcher → worker_a` no call graph.

**Files:**
- Modify: `crates/kata-inference/src/infer/recursion.rs:37-47` (`build_call_graph`)
- Modify: `crates/kata-inference/src/infer/recursion.rs:52-243` (`collect_action_calls`)

**Step 1: Adicionar propagação**

Em `build_call_graph`, após construir o grafo inicial com `collect_action_calls`, fazer uma segunda passagem:

```rust
fn build_call_graph(actions: &[TypedAction]) -> HashMap<String, Vec<(String, kata_ast::Span)>> {
    let mut graph: HashMap<String, Vec<(String, kata_ast::Span)>> = HashMap::new();
    
    // Pass 1: coletar ActionCalls diretos (como hoje).
    for action in actions {
        let mut callees = Vec::new();
        for stmt in &action.body {
            collect_action_calls(&stmt.node, &stmt.span, &mut callees);
        }
        graph.insert(action.name.clone(), callees);
    }
    
    // Pass 2: propagar call sites para params invocados indiretamente.
    // Para cada action A que chama B(... action_ref ...):
    //   se B tem param p com ty: Ty::Action e p é invocado dentro de B,
    //   registrar aresta B → action_ref.
    let indirect_edges = collect_indirect_edges(actions, &graph);
    for (source, target, span) in indirect_edges {
        graph.entry(source).or_default().push((target, span));
    }
    
    graph
}
```

**Implementação de `collect_indirect_edges`:**

```rust
fn collect_indirect_edges(
    actions: &[TypedAction],
    _graph: &HashMap<String, Vec<(String, kata_ast::Span)>>,
) -> Vec<(String, String, kata_ast::Span)> {
    let mut edges = Vec::new();
    
    for action in actions {
        // Descobrir quais params da action têm ty: Ty::Action.
        let action_params: HashMap<String, Ty> = action.params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect();
        let invoked_params: HashSet<String> = action_params
            .iter()
            .filter(|(_, ty)| matches!(ty, Ty::Action(..)))
            .map(|(name, _)| name.clone())
            .filter(|name| is_param_invoked(&action.body, name))
            .collect();
        
        if invoked_params.is_empty() { continue; }
        
        // Para cada ActionCall na action, se um arg é Ident com nome de
        // Action definida pelo usuário, e corresponde a um param invocado,
        // registrar aresta.
        let mut call_args = Vec::new();
        for stmt in &action.body {
            collect_action_call_args(&stmt.node, &stmt.span, &mut call_args);
        }
        for (callee_name, arg_idents, span) in call_args {
            for (param_name, arg_name) in /* zip params with args */ {
                if invoked_params.contains(&param_name) {
                    // arg_name é o nome da action passada como valor.
                    edges.push((callee_name, arg_name, span));
                }
            }
        }
    }
    
    edges
}
```

**Nota:** Esta é a versão primeira — só propaga 1 nível (call site direto). Cadeias de intermediárias (§4.4) ficam para depois.

**Step 2: Escrever teste**

```rust
#[test]
fn indirect_recursion_detected() {
    let src = "
action a (f :: Action(Int) => Unit) => Unit
    f!(1)

action b (n :: Int) => Unit
    a!(b)
";
    // Deve falhar na compilação: a → b → a (ciclo detectado)
}
```

**Step 3: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions indirect_recursion 2>&1
```

**Step 4: Commit**

```bash
git add crates/kata-inference/src/infer/recursion.rs crates/kata-driver/tests/first_class_actions.rs
git commit -m "feat: interprocedural def-use for indirect recursion detection"
```

---

## Fase 7: Tree shaking — `Ident` com `Ty::Action` como aresta

### Task 17: Tree shaking reconhece referência de Action como aresta

**Objective:** `Ident { name, ty: Ty::Action }` no TAST torna a action alcançável — pode ser invocada indiretamente.

**Files:**
- Modify: `crates/kata-tree-shaking/src/lib.rs:196-247` (`collect_refs`)

**Step 1: Adicionar arm**

No `collect_refs`, o arm `TypedExprKind::Ident { name }` hoje é tratado como folha (não faz nada). Adicionar:

```rust
TypedExprKind::Ident { name } => {
    // Se o Ident tem ty: Ty::Action, é referência a Action — aresta.
    if matches!(expr.ty, Ty::Action(..)) {
        reached_actions.insert(name.clone());
    }
    // Ident de variável local — não aresta.
}
```

**Nota:** O `collect_refs` recebe `expr: &TypedExpr`, que tem `expr.ty`. Verificar se o arm atual já tem acesso ao `expr.ty` ou se precisa passar mais informação.

**Step 2: Escrever teste**

```rust
#[test]
fn tree_shaking_preserves_referenced_action() {
    let src = "
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    echo!(0)
    // worker não é invocada diretamente, mas é referenciada.
    // Tree shaking deve preservar worker.
";
    // Verificar que worker sobrevive no módulo após tree shaking.
}
```

**Step 3: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions tree_shaking_preserves 2>&1
```

**Step 4: Commit**

```bash
git add crates/kata-tree-shaking/src/lib.rs crates/kata-driver/tests/first_class_actions.rs
git commit -m "feat: tree shaking recognizes Action reference (Ident with Ty::Action) as edge"
```

---

## Fase 8: Codegen — Fork com fn_ptr de variável

### Task 18: Codegen — Fork com `action_expr` (fn_ptr de variável)

**Objective:** O codegen de `Fork` extrai fn_ptr de `action_expr` em vez de `GlobalValue::Symbol` quando `action_expr` é uma variável.

**Files:**
- Modify: `crates/kata-codegen/src/lowering/csp.rs` (lowering de Fork)

**Step 1: Atualizar lowering de Fork**

Ler o arquivo `csp.rs` para entender o lowering atual de Fork. Hoje usa `GlobalValue::Symbol` para obter fn_ptr. Mudar para:

```rust
TypedExprKind::Fork { action_name, action_expr, args } => {
    let fn_ptr = if action_name == "__indirect_fork" {
        // fn_ptr vem da variável (action_expr).
        super::expr::lower_expr(&action_expr.node, ctx)?
    } else {
        // fn_ptr via GlobalValue::Symbol (como hoje).
        // ... código existente ...
    };
    // ... resto do lowering (spawn, etc.) ...
}
```

**Step 2: Escrever teste E2E**

```rust
#[test]
fn fork_with_variable_e2e() {
    let src = "
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    fork!(f, (42,))
";
    // Deve compilar, executar, e imprimir 42
}
```

**Step 3: Rodar teste**

```bash
cargo test -p kata-driver --test first_class_actions fork_with_variable 2>&1
```

**Step 4: Commit**

```bash
git add crates/kata-codegen/src/lowering/csp.rs crates/kata-driver/tests/first_class_actions.rs
git commit -m "feat: codegen for Fork with variable fn_ptr (first-class fork)"
```

---

## Fase 9: Integração e limpeza

### Task 19: Teste E2E — dispatch/strategy completo

**Objective:** Validar o exemplo canonical do PRD (§13.1) end-to-end.

**Files:**
- Test: `crates/kata-driver/tests/first_class_actions.rs`

**Step 1: Escrever teste completo**

```rust
#[test]
fn dispatch_strategy_complete() {
    let src = "
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)
    dispatcher!(worker_b, 42)
";
    // Compilar, executar, verificar output: 43\n44
}
```

**Step 2: Rodar**

```bash
cargo test -p kata-driver --test first_class_actions dispatch_strategy_complete 2>&1
```

**Step 3: Commit**

```bash
git add crates/kata-driver/tests/first_class_actions.rs
git commit -m "test: E2E dispatch/strategy (PRD §13.1)"
```

---

### Task 20: Teste E2E — seleção por match

**Objective:** Validar §13.3 — match seleciona action em runtime.

**Step 1: Escrever teste**

```rust
#[test]
fn match_select_action() {
    let src = "
action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    let cond := True
    let f := match cond
        Boolean::True: worker_a
        Boolean::False: worker_b
    f!(42)
";
    // Deve compilar, executar, e imprimir 43
}
```

**Step 2: Rodar**

```bash
cargo test -p kata-driver --test first_class_actions match_select 2>&1
```

**Step 3: Commit**

```bash
git add crates/kata-driver/tests/first_class_actions.rs
git commit -m "test: E2E match selects Action (PRD §13.3)"
```

---

### Task 21: Suite de testes — DoDs do PRD

**Objective:** Cobrir todos os 17 DoDs do PRD com testes nomeados por responsabilidade.

**Files:**
- Test: `crates/kata-driver/tests/first_class_actions.rs`

**Step 1: Mapear DoDs para testes**

| DoD | Teste |
|---|---|
| 1 | `action_reference_has_action_type` (Task 9) |
| 2 | `indirect_invocation_via_variable` (Task 11) |
| 3 | `action_as_param_to_action` (Task 11) |
| 4 | `indirect_invocation_via_param` (Task 11) |
| 5 | `fork_with_ident` (Task 13) |
| 6 | `fork_with_variable` (Task 18) |
| 7 | `action_type_syntax_valid` (Task 7) |
| 8 | `action_in_data_rejected` (Task 12) |
| 9 | `action_in_channel_rejected` (Task 12) |
| 10 | `action_as_pure_fn_param_rejected` (Task 12) |
| 11 | `indirect_recursion_detected` (Task 16) |
| 12 | `indirect_recursion_via_chain` (futuro) |
| 13 | `match_with_actions_all_edges` (Task 16) |
| 14 | `tree_shaking_preserves_referenced` (Task 17) |
| 15 | `tree_shaking_removes_unreferenced` |
| 16 | `cargo test --workspace --no-fail-fast` |
| 17 | `cargo clippy --workspace --all-targets -- -D warnings` |

**Step 2: Implementar testes faltantes**

Para DoD 15 (`tree_shaking_removes_unreferenced`):
```rust
#[test]
fn tree_shaking_removes_unreferenced_action() {
    let src = "
action unused (n :: Int) => Unit
    echo!(n)

action main => Unit
    echo!(0)
    // unused não é referenciada nem invocada
";
    // Verificar que unused é removida do módulo após tree shaking.
}
```

**Step 3: Rodar suite completa**

```bash
cargo test --workspace --no-fail-fast 2>&1
```

Esperado: todos PASS, 0 failed.

**Step 4: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1
```

Esperado: limpo.

**Step 5: Commit**

```bash
git add crates/kata-driver/tests/first_class_actions.rs
git commit -m "test: cover all PRD DoDs for first-class actions"
```

---

### Task 22: Atualizar `instantiate.rs` — arm para novos TAST variants

**Objective:** O monomorphizador precisa de arms para `ActionCall { indirect_callee }` e `Fork { action_expr }` em `instantiate_kind`.

**Files:**
- Modify: `crates/kata-monomorph/src/instantiate.rs:191-430` (`instantiate_kind`)

**Step 1: Adicionar arms**

Para `ActionCall` com `indirect_callee`:

```rust
TypedExprKind::ActionCall {
    callee,
    args,
    caller_arena,
    ffi_symbol,
    indirect_callee,
} => TypedExprKind::ActionCall {
    callee: callee.clone(),
    args: Box::new(Spanned::new(
        instantiate_typed_expr(&args.node, subs),
        args.span,
    )),
    caller_arena: *caller_arena,
    ffi_symbol: ffi_symbol.clone(),
    indirect_callee: indirect_callee.as_ref().map(|e| {
        Box::new(Spanned::new(
            instantiate_typed_expr(&e.node, subs),
            e.span,
        ))
    }),
},
```

Para `Fork` com `action_expr`:

```rust
TypedExprKind::Fork {
    action_name,
    action_expr,
    args,
} => TypedExprKind::Fork {
    action_name: action_name.clone(),
    action_expr: Box::new(Spanned::new(
        instantiate_typed_expr(&action_expr.node, subs),
        action_expr.span,
    )),
    args: Box::new(Spanned::new(
        instantiate_typed_expr(&args.node, subs),
        args.span,
    )),
},
```

**Step 2: Verificar**

```bash
cargo check -p kata-monomorph --all-targets
```

**Step 3: Commit**

```bash
git add crates/kata-monomorph/src/instantiate.rs
git commit -m "feat: instantiate new TAST variants (ActionCall indirect, Fork action_expr)"
```

---

### Task 23: Atualizar documentação

**Objective:** Atualizar manual e sintaxe-mapa com a nova sintaxe e semântica.

**Files:**
- Modify: `docs/Kata-lang-manual.md` — seção sobre Actions como first-class values
- Modify: `docs/sintaxe-mapa.md` — tipo `Action(Params) => Ret`
- Modify: `docs/PRD-first-class-actions.md` — corrigir path escape.rs, marcar status como implementado

**Step 1: Atualizar docs**

Ler os arquivos relevantes e adicionar seções sobre:
- Sintaxe `Action(Params) => Ret` no sintaxe-mapa
- Semântica de referência vs invocação no manual
- Restrições (não entra em data, canal, função pura)
- Corrigir `crates/kata-inference/src/infer/escape.rs` → `crates/kata-core/src/escape.rs` no PRD

**Step 2: Commit**

```bash
git add docs/
git commit -m "docs: update manual and sintaxe-mapa for first-class actions"
```

---

### Task 24: Suite final — workspace completo

**Objective:** Validar que tudo passa junto.

**Step 1: Rodar testes**

```bash
cargo test --workspace --no-fail-fast 2>&1
```

Esperado: 0 failed, 0 regressions.

**Step 2: Rodar clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1
```

Esperado: limpo.

**Step 3: Commit final**

```bash
git add -A
git commit -m "feat: first-class actions — complete implementation"
```

---

## Ordem de dependências

```
Fase 1 (Tasks 1-5): Ty::Action no core
  ↓
Fase 2 (Tasks 6-8): Parser + Resolution do tipo
  ↓
Fase 3 (Tasks 9-12): Inference — referência, invocação indireta, proibições
  ↓
Fase 4 (Task 13): Fork com TypedExpr
  ↓
Fase 5 (Tasks 14-15): Codegen — lowering
  ↓
Fase 6 (Task 16): Recursion check interprocedural
  ↓
Fase 7 (Task 17): Tree shaking
  ↓
Fase 8 (Task 18): Codegen Fork
  ↓
Fase 9 (Tasks 19-24): Integração, testes, docs
```

---

## Notas de implementação

- **Task 5 é dinâmica:** os arquivos exatos dependem do output de `cargo check` da Task 1. O implementador deve rodar o check e resolver todos os erros de non-exhaustive match.
- **Task 9:** verificar o padrão de retorno de `infer_expr_hinted` — pode retornar tupla ou `Result<TypedExpr>`. Adaptar o código ao padrão existente.
- **Task 16:** a primeira versão só propaga 1 nível. Cadeias de intermediárias (§4.4 do PRD) ficam para iteração futura. O PRD prevê isso no §4.4 e no risco "Def-use interprocedural é complexo demais".
- **Task 13:** `action_name == "__indirect_fork"` é um sentinel. Alternativa: usar `Option<String>` para `action_name` no Fork TAST (`None` = indireto). Considerar se o sentinel é suficiente.