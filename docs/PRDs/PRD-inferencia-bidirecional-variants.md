# PRD: Inferência Bidirecional de Type Params em Variants

## Motivo

Quando um enum genérico como `Result(T, E)` tem uma variante que não menciona
todos os type params (ex: `Ok(T)` não menciona `E`), a inferência atual só
resolve os params que aparecem no payload da variante. Os demais ficam como
`Ty::Var("E")` — não-resolvidos.

Isto causa erros de tipo em dois cenários:

1. **Construção de variant em função nomeada:**
   `div :: Int Int => Result::(Int, Text)` — o body constrói `Result::Ok val`,
   que infere `T=Int` mas deixa `E=Ty::Var("E")`. O typeck rejeita porque
   `Result::(Int, Var("E"))` ≠ `Result::(Int, Text)`.

2. **Match onde o scrutinee tem type args incompletos:**
   O scrutinee vem como `Generic("Result", [Int, Var("E")])`. O braço `Ok v`
   recebe `type_args=[Int, Var("E")]` do scrutinee. O `instantiate_variant`
   substitui `T→Int` corretamente, mas `E` nunca é resolvido.

## Design

**Type checking bidirecional top-down.** O tipo esperado (da assinatura da
função, hint de retorno, ou tipo do scrutinee em match) é propagado para
`infer_variant_construct` e usado para preencher type params não-inferidos
pelo payload da variante.

### Princípio

Quando `infer_variant_construct` constrói `Ok val`:
1. Infere `val.ty` → `Int`
2. Unifica `payload_ty = Var("T")` com `val.ty = Int` → `T = Int`
3. **NOVO:** Se há um `expected_ty = Generic("Result", [Int, Text])`:
   - Extrai type args do expected: `[Int, Text]`
   - Mapeia pelos type_params do enum: `T→Int, E→Text`
   - Para cada param não-inferido pelo payload (ex: `E`), usa o valor do expected
4. Resultado: `Generic("Result", [Int, Text])` — completo

### Ortogonalidade

Esta correção é **ortogonal ao default type params** (`Err(E=Text)`). O
default preenche params quando o usuário não fornece nem o contexto fornece.
A inferência bidirecional preenche quando o contexto (assinatura) fornece.
Ambos cooperam: se o expected_ty tem `E=Text` (via default), a inferência
bidirecional propaga esse `Text` para a construção da variante.

## Sites de mudança

### 1. `infer_variant_construct` — `variant_construct.rs`

Assinatura atual:
```rust
pub(crate) fn infer_variant_construct(
    enum_name: &str,
    variant: &str,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)>
```

Nova assinatura:
```rust
pub(crate) fn infer_variant_construct(
    enum_name: &str,
    variant: &str,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    expected_ty: Option<&Ty>,  // NOVO
) -> InferResult<(Ty, TypedExprKind)>
```

Lógica adicionada após o loop de unificação (linha ~98):
```rust
// Se há expected_ty e é Generic do mesmo enum, preenche params não-inferidos.
if let Some(Ty::Generic(exp_name, exp_args)) = expected_ty
    && exp_name == enum_name
    && exp_args.len() == type_args.len()
{
    for (i, arg) in type_args.iter_mut().enumerate() {
        if matches!(arg, Ty::Var(_)) {
            *arg = exp_args[i].clone();
        }
    }
}
```

### 2. `infer_apply` — `apply.rs`

Linha 57: passar `hint` para `infer_variant_construct`:
```rust
Expr::VariantQual { enum_name, variant } => {
    return infer_variant_construct(enum_name, variant, args, span, env, ctx, hint);
}
```

Também no caminho de Ident que resolve para variant (linha ~655):
```rust
return infer_variant_construct(enum_name, &func_name, args, span, env, ctx, hint);
```

### 3. `infer_expr` em `expr.rs` — VariantQual fora de Apply

Se `VariantQual` aparece sem args (variante unitária como `None`, `True`), o
caminho em `infer_expr_hinted` resolve via `resolve_unqual_variant`. Esse
caminho também precisa aceitar e usar o hint.

### 4. Match — `_match.rs` + `patterns.rs`

O scrutinee já fornece type_args completos quando o tipo é conhecido. O
problema é quando o scrutinee também tem `Ty::Var` não-resolvido. Neste caso,
o type_args do scrutinee é o melhor disponível — não há expected_ty adicional
para o pattern. A correção do match é indireta: se o scrutinee vem de uma
expressão que recebeu hint (ex: argumento de função com tipo conhecido), o
hint já foi aplicado na inferência do scrutinee.

**Caso especial:** match em função nomeada onde o scrutinee é construído
inline e a assinatura conhece o tipo. Ex:
```
div :: Int Int => Result::(Int, Text)
lambda a b:
    match (NonZero b)
        Result::Ok nz: Result::Ok (/ a nz)
        Result::Err _: Result::Err("...")
```
Aqui `(NonZero b)` é o scrutinee. `NonZero` é um refined que retorna
`Result::(NonZero, Text)`. O scrutinee deve vir com `E=Text` já resolvido
pelo construtor falível. Se não vem, é um bug no construtor falível, não no
match.

## Testes

### Teste 1: Construção de Ok em função com assinatura Result

```kata
ok_id :: Int => Result::(Int, Text)
lambda x: Result::Ok x
```
Debe inferir `Result::(Int, Text)`, não `Result::(Int, Var("E"))`.

### Teste 2: Construção de Err em função com assinatura Result

```kata
err_str :: Text => Result::(Int, Text)
lambda msg: Result::Err msg
```
Debe inferir `Result::(Int, Text)`, não `Result::(Var("T"), Text)`.

### Teste 3: Match com ambos os braços

```kata
unwrap :: Result::(Int, Text) => Int
lambda r:
    match r
        Result::Ok v: v
        Result::Err _: 0
```
O scrutinee `r` tem tipo `Result::(Int, Text)` (da assinatura). Os braços
devem inferir corretamente `v: Int`.

### Teste 4: Construção sem hint (lambda anônimo)

```kata
lambda x: Result::Ok x
```
Sem hint, `E` fica como `Ty::Var("E")`. Isto é aceitável — o lambda anônimo
não tem assinatura. Se for aplicado a um contexto que espera
`Result::(Int, Text)`, o hint do apply resolve.

## Definições de done

- [x] `infer_variant_construct` aceita `expected_ty: Option<&Ty>`
- [x] Type params não-inferidos pelo payload são preenchidos pelo expected_ty
- [x] `infer_apply` propaga `hint` para `infer_variant_construct`
- [x] Caminho de Ident→variant em `apply.rs` propaga hint
- [x] Teste 1 (Ok em função nomeada) passa
- [x] Teste 2 (Err em função nomeada) passa
- [x] Teste 3 (match com scrutinee tipado) passa
- [x] `cargo test --workspace` passa (0 failures)

## Conclusão

Implementado em 3 commits (`50c78ef`, `0686787`, `bfca94f`) + default type params
(`d160232`). A inferência bidirecional preenche type params não-inferidos pelo
payload usando o tipo esperado do contexto (assinatura, hint de retorno).
O default type params (`Err(E=Text)`) coopera com a inferência bidirecional:
quando nem o contexto fornece o param, o default preenche.