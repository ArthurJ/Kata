//! DotAccess — field access em struct + index access em tupla.
//!
//! Extraído de `expr.rs` — `infer_dot_access` é self-contained: chama
//! `infer_expr` mas não `infer_expr_hinted`, e tem seu próprio match
//! independente sobre `(Ty, DotIndex)`.
//!
//! Desugaring de DotIndex::Int em coleções: `b.0` vira `at b 0` via
//! INDEXABLE dispatch (retorna `Result::(A, Err)`).
//! Desugaring de DotIndex::Range: `b.[1..3]` vira `slice b 1 3` via
//! SLICEABLE dispatch. Se `inclusive=true` (`..=`), o typeck envolve
//! `end` em `end + 1` antes de despachar (runtime espera exclusive).

use kata_ast::{DotIndex, Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::generics::{apply_subs, unify};
use super::helpers::InferResult;
use kata_resolution::resolve_type_expr;

// ── Reflection fields ──────────────────────────────────────────
/// Fields de reflexão disponíveis em functions e actions via DotAccess.
const REFLECTION_FIELDS: &[&str] = &["name", "arity", "param_types", "return_type", "is_action"];

/// Retorna `true` se o field name é um field de reflexão de função/action.
fn is_reflection_field(field: &str) -> bool {
    REFLECTION_FIELDS.contains(&field)
}

/// Resolve um field de reflexão para uma `TypedExpr` escalar constante
/// em compile-time — uma única overload.
///
/// Usado no caso estático desambiguado (`f.(Int Int).arity`) e como
/// elemento do `ListLit` no caso estático sem desambiguação.
fn resolve_reflection_field(
    field: &str,
    name: &str,
    params: &[Ty],
    ret: &Ty,
    is_action: bool,
    span: &Span,
) -> TypedExpr {
    let (ty, kind) = match field {
        "name" => (
            Ty::text(),
            TypedExprKind::TextLit {
                text: name.to_string(),
            },
        ),
        "arity" => (
            Ty::int(),
            TypedExprKind::IntLit {
                text: params.len().to_string(),
            },
        ),
        "param_types" => {
            let elements: Vec<Spanned<TypedExpr>> = params
                .iter()
                .map(|p| {
                    Spanned::new(
                        TypedExpr {
                            span: *span,
                            ty: Ty::text(),
                            tail_pos: false,
                            escape: EscapeTarget::Local,
                            kind: TypedExprKind::TextLit { text: p.to_text() },
                        },
                        *span,
                    )
                })
                .collect();
            (
                Ty::List(Box::new(Ty::text())),
                TypedExprKind::ListLit { elements },
            )
        }
        "return_type" => (
            Ty::text(),
            TypedExprKind::TextLit {
                text: ret.to_text(),
            },
        ),
        "is_action" => {
            // Boolean::True (tag 0) ou Boolean::False (tag 1)
            // O enum Boolean é definido no prelude: True=0, False=1
            let (variant, tag) = if is_action {
                ("True".to_string(), 0)
            } else {
                ("False".to_string(), 1)
            };
            (
                Ty::boolean(),
                TypedExprKind::VariantQual {
                    enum_name: "Boolean".into(),
                    variant,
                    tag,
                    module_path: None,
                },
            )
        }
        _ => unreachable!("is_reflection_field deve ser chamado antes"),
    };

    TypedExpr {
        span: *span,
        ty,
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind,
    }
}

/// Tipo do elemento escalar para cada field de reflexão.
fn reflection_field_elem_ty(field: &str) -> Ty {
    match field {
        "name" | "return_type" => Ty::text(),
        "arity" => Ty::int(),
        "param_types" => Ty::List(Box::new(Ty::text())),
        "is_action" => Ty::boolean(),
        _ => unreachable!("is_reflection_field deve ser chamado antes"),
    }
}

/// Resolve um field de reflexão para `ListLit` — um elemento por overload.
///
/// Usado no caso estático sem desambiguação (`f.name`, `f.arity`, etc.).
/// Consulta o DispatchTable para coletar todas as overloads do nome.
/// Sempre produz lista, mesmo com uma única overload — o tipo não muda
/// quando overloads são adicionadas.
fn resolve_reflection_field_list(
    field: &str,
    name: &str,
    overloads: &[kata_core::dispatch::OverloadInfo],
    span: &Span,
) -> TypedExpr {
    let elem_ty = reflection_field_elem_ty(field);
    let list_ty = Ty::List(Box::new(elem_ty));

    let elements: Vec<Spanned<TypedExpr>> = overloads
        .iter()
        .map(|oi| {
            Spanned::new(
                resolve_reflection_field(
                    field,
                    name,
                    &oi.params,
                    &oi.ret,
                    oi.is_action,
                    span,
                ),
                *span,
            )
        })
        .collect();

    TypedExpr {
        span: *span,
        ty: list_ty,
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::ListLit { elements },
    }
}

/// Infere `expr.nome` (field access) ou `expr.N` (index access).
///
/// Desambiguação pelo tipo do receptor:
/// - `Ty::Struct(name)` + `DotIndex::Field` → `FieldAccess`
/// - `Ty::Struct(name)` + `DotIndex::Int` → erro `IndexAccessOnStruct`
/// - `Ty::Tuple(elements)` + `DotIndex::Int(n)` → `IndexAccess` (negativos
///   normalizados, bounds check compile-time)
/// - `Ty::Tuple(elements)` + `DotIndex::Field` → erro `FieldAccessOnTuple`
/// - `Ty::List(A)` / `Ty::Array(A)` / `Ty::Bytes` / `Ty::Text` +
///   `DotIndex::Int(n)` → desugar para `at receptor n` via INDEXABLE
///   dispatch (retorna `Result::(A, Err)`)
/// - `Ty::List(A)` / `Ty::Array(A)` / `Ty::Bytes` / `Ty::Text` +
///   `DotIndex::Range` → desugar para `slice receptor start end` via
///   SLICEABLE dispatch
/// - `Ty::Range(_)` + `DotIndex::Int(_)` → erro (Range não implementa INDEXABLE)
/// - Coleção + `DotIndex::Field` → erro `FieldAccessOnCollection`
/// - Outro → erro `NotIndexable`
pub(crate) fn infer_dot_access(
    expr: &Spanned<Expr>,
    index: &DotIndex,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    // ── Module access: `mod.fn` ──────────────────────────────
    // Se o receptor é `Ident("mod_name")` e `mod_name` não está no TypeEnv
    // (não é variável local), verificar se existe `mod_name.field` no
    // DispatchTable. Se sim, resolver como `Ident { name: "mod.field" }`.
    //
    // Isso permite `mock_math.dobrar 21` onde `mock_math` é um módulo
    // importado via `import mock_math` (WholeModule). O merge_imports
    // registra cada item exportado com nome qualificado `mock_math.dobrar`.
    if let Expr::Ident { name } = &expr.node
        && let DotIndex::Field(field_name) = index
        && env.lookup(name).is_none()
    {
        // `name` não é variável local — pode ser módulo.
        let qual_name = format!("{name}.{field_name}");
        if let Some(overloads) = ctx.table.get_overloads(&qual_name) {
            // Encontrou `mod.fn` no DispatchTable — é module access.
            let overload = &overloads[0];
            return Ok(TypedExpr {
                span: *span,
                ty: Ty::Function(overload.params.clone(), Box::new(overload.ret.clone())),
                tail_pos,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident { name: qual_name },
            });
        }

        // ── Action reflection (caso 4a) ──
        // Se `name` não está no TypeEnv, module access falhou, e o field é
        // um field de reflexão, tentar buscar `name` no DispatchTable.
        // Actions não são first-class — reflexão de actions é sempre estática.
        // Sempre lista: um elemento por overload (DoD 1-2).
        if is_reflection_field(field_name) {
            if let Some(overloads) = ctx.table.get_overloads(name) {
                // Só resolver se há overloads que são actions.
                // (O nome pode existir no DispatchTable como function, não action.)
                let has_action = overloads.iter().any(|oi| oi.is_action);
                if has_action {
                    let action_overloads: Vec<_> = overloads
                        .iter()
                        .filter(|oi| oi.is_action)
                        .cloned()
                        .collect();
                    return Ok(resolve_reflection_field_list(
                        field_name,
                        name,
                        &action_overloads,
                        span,
                    ));
                }
            }
        }
    }

    let inner = infer_expr(&expr.node, &expr.span, env, ctx, false)?;
    let inner_spanned = Spanned::new(inner.clone(), expr.span);
    let inner_box = Box::new(inner_spanned);

    match (&inner.ty, index) {
        (Ty::Struct(struct_name), DotIndex::Field(field_name)) => {
            let info =
                ctx.struct_registry
                    .get(struct_name)
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: format!("struct `{struct_name}` não registrado no StructRegistry"),
                        span: (*span).into(),
                    })?;
            let (field_index, field_info) =
                info.find_field(field_name)
                    .ok_or_else(|| MiddleError::UnknownField {
                        struct_name: struct_name.clone(),
                        field_name: field_name.clone(),
                        span: (*span).into(),
                    })?;
            let ty = field_info.ty.clone();
            Ok(TypedExpr {
                span: *span,
                ty,
                tail_pos,
                escape: inner.escape,
                kind: TypedExprKind::FieldAccess {
                    expr: inner_box,
                    struct_name: struct_name.clone(),
                    field_name: field_name.clone(),
                    field_index,
                },
            })
        }
        (Ty::Struct(_), DotIndex::Int(_)) => Err(MiddleError::IndexAccessOnStruct {
            span: (*span).into(),
        }),
        (Ty::Tuple(elements), DotIndex::Int(n)) => {
            let len = elements.len() as i64;
            // Normaliza negativo: -1 = len-1, -2 = len-2, etc.
            let resolved = if *n < 0 { len + n } else { *n };
            if resolved < 0 || resolved >= len {
                return Err(MiddleError::IndexOutOfBounds {
                    index: *n,
                    len: len as usize,
                    span: (*span).into(),
                });
            }
            let element_index = resolved as u32;
            let ty = elements[resolved as usize].clone();
            Ok(TypedExpr {
                span: *span,
                ty,
                tail_pos,
                escape: inner.escape,
                kind: TypedExprKind::IndexAccess {
                    expr: inner_box,
                    index: *n,
                    element_index,
                },
            })
        }
        (Ty::Tuple(_), DotIndex::Field(_)) => Err(MiddleError::FieldAccessOnTuple {
            span: (*span).into(),
        }),
        // .N em List/Array/Bytes/Text → desugar para `at receptor N` via INDEXABLE.
        // O dispatch retorna Result::(A, Err) — access checked.
        // `at` tem type_params (A é genérico), então precisa do caminho
        // genérico: percorrer overloads e fazer unify.
        (Ty::List(_) | Ty::Array(_) | Ty::Bytes | Ty::Prim(PrimTy::Text), DotIndex::Int(n)) => {
            let arg_types = vec![inner.ty.clone(), Ty::int()];
            // Tenta caminho não-genérico primeiro.
            let overload = ctx.table.resolve("at", &arg_types, ctx.interface_registry);
            let (ret_ty, ffi_symbol, params) = match overload {
                Ok(oi) => (
                    ctx.enum_registry.expand_defaults(&oi.ret),
                    oi.ffi_symbol,
                    oi.params,
                ),
                Err(_) => {
                    // Caminho genérico: procura overload com type_params e faz unify.
                    let overloads =
                        ctx.table
                            .get_overloads("at")
                            .ok_or_else(|| MiddleError::UnboundName {
                                name: "at".into(),
                                span: (*span).into(),
                            })?;
                    let mut found = None;
                    for oi in overloads.iter().filter(|oi| {
                        oi.params.len() == arg_types.len() && !oi.type_params.is_empty()
                    }) {
                        let mut subs = std::collections::HashMap::new();
                        if unify(&oi.params, &arg_types, &oi.type_params, &mut subs).is_ok() {
                            let concrete_ret = apply_subs(&oi.ret, &subs);
                            let expanded_ret = ctx.enum_registry.expand_defaults(&concrete_ret);
                            found = Some((expanded_ret, oi.ffi_symbol.clone(), oi.params.clone()));
                            break;
                        }
                    }
                    found.ok_or_else(|| MiddleError::TypeMismatch {
                        expected: format!("`at` dispatch via INDEXABLE para {}", inner.ty),
                        found: "nenhuma overload genérica de `at` unifica".into(),
                        span: (*span).into(),
                    })?
                }
            };

            // Constrói TypedExpr para o índice (IntLit com o valor n).
            let index_typed = TypedExpr {
                span: *span,
                ty: Ty::int(),
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::IntLit {
                    text: n.to_string(),
                },
            };
            let index_spanned = Spanned::new(index_typed, *span);

            let callee_ty = Ty::Function(params, Box::new(ret_ty.clone()));
            let callee_typed = TypedExpr {
                span: *span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident { name: "at".into() },
            };

            Ok(TypedExpr {
                span: *span,
                ty: ret_ty,
                tail_pos,
                escape: inner.escape,
                kind: TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, *span)),
                    args: vec![*inner_box.clone(), index_spanned],
                    ffi_symbol,
                },
            })
        }
        // .[start..end] em List/Array/Bytes/Text → desugar para
        // `slice receptor start end` via SLICEABLE.
        // Se `inclusive=true` (`..=`), envolve `end` em `end + 1` antes de
        // despachar (runtime espera end exclusive).
        (
            Ty::List(_) | Ty::Array(_) | Ty::Bytes | Ty::Prim(PrimTy::Text),
            DotIndex::Range {
                start,
                end,
                inclusive,
            },
        ) => {
            // Infer start e end como Int.
            let start_typed = infer_expr(&start.node, &start.span, env, ctx, false)?;
            // Verifica que start é Int (ou unificável).
            let start_typed = if start_typed.ty == Ty::int() {
                start_typed
            } else {
                // Tenta unify com Int.
                let mut subs = std::collections::HashMap::new();
                if unify(
                    std::slice::from_ref(&start_typed.ty),
                    &[Ty::int()],
                    &[],
                    &mut subs,
                )
                .is_ok()
                {
                    start_typed
                } else {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Int".into(),
                        found: format!("{}", start_typed.ty),
                        span: start.span.into(),
                    });
                }
            };

            let end_typed = infer_expr(&end.node, &end.span, env, ctx, false)?;
            let end_typed = if end_typed.ty == Ty::int() {
                end_typed
            } else {
                let mut subs = std::collections::HashMap::new();
                if unify(
                    std::slice::from_ref(&end_typed.ty),
                    &[Ty::int()],
                    &[],
                    &mut subs,
                )
                .is_ok()
                {
                    end_typed
                } else {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Int".into(),
                        found: format!("{}", end_typed.ty),
                        span: end.span.into(),
                    });
                }
            };

            // Se inclusive (`..=`), envolve end em `end + 1`.
            // Cria um TypedExpr que soma 1 ao end.
            let end_final = if *inclusive {
                TypedExpr {
                    span: end.span,
                    ty: Ty::int(),
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::Closure {
                        callee: Box::new(Spanned::new(
                            TypedExpr {
                                span: end.span,
                                ty: Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())),
                                tail_pos: false,
                                escape: EscapeTarget::Local,
                                kind: TypedExprKind::Ident { name: "+".into() },
                            },
                            end.span,
                        )),
                        args: vec![
                            Spanned::new(end_typed, end.span),
                            Spanned::new(
                                TypedExpr {
                                    span: end.span,
                                    ty: Ty::int(),
                                    tail_pos: false,
                                    escape: EscapeTarget::Local,
                                    kind: TypedExprKind::IntLit { text: "1".into() },
                                },
                                end.span,
                            ),
                        ],
                        ffi_symbol: Some("kata_rt_bi_add".into()),
                    },
                }
            } else {
                end_typed
            };

            // Despacha `slice receptor start end` via SLICEABLE.
            let arg_types = vec![inner.ty.clone(), Ty::int(), Ty::int()];
            let overload = ctx
                .table
                .resolve("slice", &arg_types, ctx.interface_registry);
            let (ret_ty, ffi_symbol, params) = match overload {
                Ok(oi) => (
                    ctx.enum_registry.expand_defaults(&oi.ret),
                    oi.ffi_symbol,
                    oi.params,
                ),
                Err(_) => {
                    // Caminho genérico: procura overload com type_params e faz unify.
                    let overloads = ctx.table.get_overloads("slice").ok_or_else(|| {
                        MiddleError::UnboundName {
                            name: "slice".into(),
                            span: (*span).into(),
                        }
                    })?;
                    let mut found = None;
                    for oi in overloads.iter().filter(|oi| {
                        oi.params.len() == arg_types.len() && !oi.type_params.is_empty()
                    }) {
                        let mut subs = std::collections::HashMap::new();
                        if unify(&oi.params, &arg_types, &oi.type_params, &mut subs).is_ok() {
                            let concrete_ret = apply_subs(&oi.ret, &subs);
                            let expanded_ret = ctx.enum_registry.expand_defaults(&concrete_ret);
                            found = Some((expanded_ret, oi.ffi_symbol.clone(), oi.params.clone()));
                            break;
                        }
                    }
                    found.ok_or_else(|| MiddleError::TypeMismatch {
                        expected: format!("`slice` dispatch via SLICEABLE para {}", inner.ty),
                        found: "nenhuma overload genérica de `slice` unifica".into(),
                        span: (*span).into(),
                    })?
                }
            };

            let callee_ty = Ty::Function(params, Box::new(ret_ty.clone()));
            let callee_typed = TypedExpr {
                span: *span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: "slice".into(),
                },
            };

            Ok(TypedExpr {
                span: *span,
                ty: ret_ty,
                tail_pos,
                escape: inner.escape,
                kind: TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, *span)),
                    args: vec![
                        *inner_box.clone(),
                        Spanned::new(start_typed, start.span),
                        Spanned::new(end_final, end.span),
                    ],
                    ffi_symbol,
                },
            })
        }
        // Range não implementa INDEXABLE — .N é type error.
        (Ty::Range(_), DotIndex::Int(_)) => Err(MiddleError::NotIndexable {
            ty: format!("{}", inner.ty),
            span: (*span).into(),
        }),
        // ── Function reflection (caso 4b) ──────────────────────
        // `f.name` onde `f` é `Ident` direto para função nomeada.
        // Resolve em compile-time para ListLit — um elemento por overload.
        // Consulta o DispatchTable (não o TypeEnv) para coletar todas as
        // overloads do nome. Sempre lista (DoD 1-2).
        //
        // Distinção estático vs dinâmico: o Ident é estático quando seu
        // nome existe no DispatchTable como function (é a função nomeada
        // original). Se o nome só existe no TypeEnv (é uma variável local
        // que aponta para uma função), cai no caso dinâmico (Fase 5).
        (Ty::Function(params, ret), DotIndex::Field(field_name))
            if is_reflection_field(field_name)
                && matches!(
                    &expr.node,
                    Expr::Ident { name }
                    if env.lookup(name).is_some()
                    && ctx.table.get_overloads(name).is_some_and(|ols| ols.iter().any(|oi| !oi.is_action))
                ) =>
        {
            if let Expr::Ident { name } = &expr.node {
                // Coletar todas as overloads do nome no DispatchTable.
                // Só overloads que são functions (não actions).
                if let Some(overloads) = ctx.table.get_overloads(name) {
                    let func_overloads: Vec<_> = overloads
                        .iter()
                        .filter(|oi| !oi.is_action)
                        .cloned()
                        .collect();
                    if !func_overloads.is_empty() {
                        return Ok(resolve_reflection_field_list(
                            field_name,
                            name,
                            &func_overloads,
                            span,
                        ));
                    }
                }
                // Fallback: TypeEnv tem a função mas DispatchTable não.
                // Usa o params/ret do Ty::Function como única overload.
                let single = kata_core::dispatch::OverloadInfo {
                    name: name.to_string(),
                    params: params.clone(),
                    ret: (**ret).clone(),
                    ffi_symbol: None,
                    is_action: false,
                    is_generic: false,
                    is_constructor: false,
                    associative_neutral: None,
                    type_params: vec![],
                    substitutions: None,
                    param_names: vec![],
                };
                return Ok(resolve_reflection_field_list(
                    field_name,
                    name,
                    std::slice::from_ref(&single),
                    span,
                ));
            }
            unreachable!("guard garante que expr é Ident");
        }
        // ── Function reflection (caso 4b-lambda) ───────────────
        // `g.name` onde `g` é variável local (let binding) com Ty::Function,
        // NÃO está no DispatchTable, e NÃO tem `fn_alias` (não é alias para
        // função nomeada). É uma lambda anônima atribuída via
        // `let g := lambda...`. Resolve em compile-time para ListLit com 1
        // elemento usando o nome do binding (DoD 7).
        (Ty::Function(params, ret), DotIndex::Field(field_name))
            if is_reflection_field(field_name)
                && matches!(
                    &expr.node,
                    Expr::Ident { name }
                    if env.lookup(name).is_some()
                    && ctx.table.get_overloads(name).is_none()
                    && env.fn_alias_of(name).is_none()
                ) =>
        {
            if let Expr::Ident { name } = &expr.node {
                let single = kata_core::dispatch::OverloadInfo {
                    name: name.to_string(),
                    params: params.clone(),
                    ret: (**ret).clone(),
                    ffi_symbol: None,
                    is_action: false,
                    is_generic: false,
                    is_constructor: false,
                    associative_neutral: None,
                    type_params: vec![],
                    substitutions: None,
                    param_names: vec![],
                };
                return Ok(resolve_reflection_field_list(
                    field_name,
                    name,
                    std::slice::from_ref(&single),
                    span,
                ));
            }
            unreachable!("guard garante que expr é Ident");
        }
        // ── Function reflection (caso 5 — dinâmico) ────────────
        // `g.name` onde `g` é variável/expressão com `Ty::Function`.
        // O typeck não tem provenance — emite chamada FFI para binary search
        // na sidecar table em runtime.
        (Ty::Function(_, _), DotIndex::Field(field_name)) if is_reflection_field(field_name) => {
            let field_id = match field_name.as_str() {
                "name" => 0i64,
                "arity" => 1,
                "param_types" => 2,
                "return_type" => 3,
                "is_action" => 4,
                _ => unreachable!("is_reflection_field garante field válido"),
            };
            // Tipo de retorno depende do field
            let ret_ty = match field_name.as_str() {
                "name" | "return_type" => Ty::text(),
                "arity" => Ty::int(),
                "param_types" => Ty::List(Box::new(Ty::text())),
                "is_action" => Ty::boolean(),
                _ => unreachable!(),
            };
            // Constrói: kata_rt_fn_meta_lookup(fn_ptr, field_id)
            // O fn_ptr é o valor do receptor (inner), que já foi inferido
            // como Ty::Function → I64 na ABI.
            let field_id_typed = TypedExpr {
                span: *span,
                ty: Ty::int(),
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::IntLit {
                    text: field_id.to_string(),
                },
            };
            let callee_typed = TypedExpr {
                span: *span,
                ty: Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())),
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: "kata_rt_fn_meta_lookup".into(),
                },
            };
            Ok(TypedExpr {
                span: *span,
                ty: ret_ty,
                tail_pos,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, *span)),
                    args: vec![*inner_box.clone(), Spanned::new(field_id_typed, *span)],
                    ffi_symbol: Some("kata_rt_fn_meta_lookup".into()),
                },
            })
        }
        // ── Desambiguação de overload: `f.(Int Int)` ─────────
        // `f.(Type1 Type2 ...)` seleciona uma overload específica do
        // DispatchTable por tipos de parâmetros. Retorna um `Ident` com
        // `Ty::Function` concreta (a overload selecionada). A partir daí,
        // `.arity` é escalar (não lista).
        //
        // Só funciona para `Expr::Ident` cujo nome está no DispatchTable.
        // Se 0 matches: erro NoOverloadForTypes. Se 2+ matches (mesmos
        // params, ret diferente): erro AmbiguousOverload.
        (inner_ty, DotIndex::Type(type_exprs)) => {
            // Resolver TypeExprs → Ty.
            let requested: Vec<Ty> = type_exprs
                .iter()
                .map(|te| resolve_type_expr(&te.node, env, ctx.interface_registry))
                .collect();

            // Só funciona para Ident direto de função nomeada.
            if let Expr::Ident { name } = &expr.node {
                if let Some(overloads) = ctx.table.get_overloads(name) {
                    // Filtrar overloads (não-actions) por params == requested.
                    let matching: Vec<_> = overloads
                        .iter()
                        .filter(|oi| !oi.is_action && oi.params == requested)
                        .collect();

                    return match matching.len() {
                        0 => Err(MiddleError::TypeMismatch {
                            expected: format!(
                                "overload de `{name}` com params {:?}",
                                requested
                            ),
                            found: "nenhuma overload compatível".into(),
                            span: (*span).into(),
                        }),
                        1 => {
                            let oi = matching[0];
                            Ok(TypedExpr {
                                span: *span,
                                ty: Ty::Function(
                                    oi.params.clone(),
                                    Box::new(oi.ret.clone()),
                                ),
                                tail_pos,
                                escape: EscapeTarget::Local,
                                kind: TypedExprKind::Ident {
                                    name: name.clone(),
                                },
                            })
                        }
                        _ => Err(MiddleError::TypeMismatch {
                            expected: format!("overload única de `{name}` com params {:?}", requested),
                            found: "múltiplas overloads com mesmos params — ambígua".into(),
                            span: (*span).into(),
                        }),
                    };
                }
            }
            // Não é Ident ou não está no DispatchTable — erro.
            Err(MiddleError::NotIndexable {
                ty: format!("{inner_ty}"),
                span: (*span).into(),
            })
        }
        // Field access em coleção não faz sentido.
        (
            Ty::List(_) | Ty::Array(_) | Ty::Range(_) | Ty::Bytes | Ty::Prim(PrimTy::Text),
            DotIndex::Field(_),
        ) => Err(MiddleError::FieldAccessOnTuple {
            span: (*span).into(),
        }),
        (other_ty, _) => Err(MiddleError::NotIndexable {
            ty: format!("{other_ty:?}"),
            span: (*span).into(),
        }),
    }
}
