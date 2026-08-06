# TODO — Extrair dispatch de reflexão de `dot_access.rs`

**Data:** 2026-08-05
**Arquivos afetados:** `crates/kata-inference/src/infer/dot_access.rs` (788 linhas), `crates/kata-inference/src/infer/mod.rs`
**Status:** Não iniciado
**Prioridade:** Antes de implementar diretivas customizadas

## Problema

`infer_dot_access` acumulou 8+ caminhos semânticos sem princípio organizador:

1. Struct field access (`Ty::Struct` + `Field`)
2. Tuple index access (`Ty::Tuple` + `Int`)
3. List/Array/Bytes/Text indexing via INDEXABLE (`Ty::List` + `Int` → desugar `at`)
4. Range slicing via SLICEABLE (`Ty::List` + `Range` → desugar `slice`)
5. Function reflection estático — Ident direto no TypeEnv + DispatchTable
6. Function reflection estático — lambda binding sem `fn_alias`
7. Function reflection dinâmico — variável com `Ty::Function` (sidecar table)
8. `DotIndex::Type` — desambiguação de overload

Cada braço é individualmente justificado, mas a função está virando um
dispatch table para dispatch tables. Os braços de reflexão (5-8) têm guards
de quatro condições encadeadas e estão misturados com o dispatch de
coleções/struct/tupla (1-4), que é o propósito original de `dot_access`.

Se as diretivas customizadas adicionarem mais complexidade ao typeck, o
motor de inferência se torna o gargalo de complexidade do projeto.

## Proposta

Extrair o dispatch de reflexão para um módulo próprio:
`crates/kata-inference/src/infer/reflection.rs`.

### Interface

```rust
// reflection.rs

/// Tenta resolver reflexão de função/action via DotAccess.
/// Retorna `Some(TypedExpr)` se o `expr.index` é reflexão (e foi resolvido),
/// `None` caso contrário (deixa `dot_access` continuar).
///
/// Chamado por `infer_dot_access` ANTES do match principal.
pub(crate) fn infer_reflection(
    expr: &Spanned<Expr>,
    index: &DotIndex,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> Option<InferResult<TypedExpr>>
```

### O que migra para `reflection.rs`

- `REFLECTION_FIELDS`, `is_reflection_field`
- `resolve_reflection_field` (escalar constante — caso estático desambiguado)
- `resolve_reflection_field_list` (ListLit — caso estático sem desambiguação)
- `reflection_field_elem_ty`
- Caso 4a: action reflection (antes de `infer_expr`, após module access)
- Caso 4b: function reflection estático (Ident no TypeEnv + DispatchTable)
- Caso 4b-lambda: function reflection estático (lambda binding sem `fn_alias`)
- Caso 5: function reflection dinâmico (variável → `kata_rt_fn_meta_lookup`)
- Caso `DotIndex::Type`: desambiguação de overload

### O que fica em `dot_access.rs`

- Module access (`mod.fn`) — tenta antes de tudo, não é reflexão
- Struct field access
- Tuple index access
- List/Array/Bytes/Text indexing via INDEXABLE
- Range slicing via SLICEABLE
- Casos de erro (`NotIndexable`, `FieldAccessOnTuple`, etc.)

### Padrão de chamada em `dot_access.rs`

```rust
pub(crate) fn infer_dot_access(...) -> InferResult<TypedExpr> {
    // ── Module access: `mod.fn` ──
    // (código existente — tenta antes de tudo)

    // ── Reflection: action estática + function estática/dinâmica ──
    if let Some(result) = infer_reflection(expr, index, span, env, ctx) {
        return result;
    }

    // ── Dispatch por tipo do receptor (match existente) ──
    let inner = infer_expr(...)?;
    match (&inner.ty, index) {
        (Ty::Struct(...), DotIndex::Field(...)) => ...,
        (Ty::Tuple(...), DotIndex::Int(...)) => ...,
        (Ty::List(...) | ..., DotIndex::Int(...)) => ...,
        (Ty::List(...) | ..., DotIndex::Range { ... }) => ...,
        // ...
    }
}
```

## Motivação

O split de `apply.rs` em `apply_dispatch.rs` + `apply_len_tuple.rs` foi
proativo — separou algoritmos distintos. O mesmo padrão se aplica aqui:
`dot_access` faz dispatch por tipo de coleção/struct/tupla; `reflection`
faz dispatch de metadados de função/action. São responsabilidades
distintas e devem estar em módulos distintos.

Isto reduz `dot_access.rs` ao que era antes da reflexão (~500 linhas de
dispatch de coleções/struct/tupla) e isola a complexidade de reflexão
num módulo com responsabilidade clara.

## Verificação

```bash
cargo test -p kata-inference -- dot_access
cargo test --workspace --no-fail-fast
```