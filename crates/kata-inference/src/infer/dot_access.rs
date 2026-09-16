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

use kata_ast::{DotIndex, Expr, Span, Spanned, TensorAxis};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::generics::{apply_subs, unify};
use super::helpers::InferResult;

/// Avalia uma expressão como literal inteiro em compile-time.
/// Usado para índices de range em `tensor.(0..2 1)`.
/// Suporta apenas `Expr::IntLit` — variáveis e expressões complexas
/// retornam erro (primeira implementação: apenas literais).
fn eval_int_literal(expr: &Expr) -> Result<i64, MiddleError> {
    match expr {
        Expr::IntLit { text } => {
            text.parse::<i64>()
                .map_err(|_| MiddleError::TypeMismatch {
                    expected: "literal inteiro".into(),
                    found: format!("não-integer literal: {}", text),
                    span: Span::zero().into(),
                })
        }
        _ => Err(MiddleError::TypeMismatch {
            expected: "literal inteiro em índice de range de tensor".into(),
            found: "expressão não-literal".into(),
            span: Span::zero().into(),
        }),
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
    }

    let inner = infer_expr(&expr.node, &expr.span, env, ctx, false)?;
    let inner_spanned = Spanned::new(inner.clone(), expr.span);
    let inner_box = Box::new(inner_spanned);

    match (&inner.ty, index) {
        (Ty::Struct(key), DotIndex::Field(field_name)) => {
            // Para Instance(family, concrete), resolver campos do tipo
            // concreto (ex: NonZero::MyNum → campos de MyNum).
            let lookup_name = key.concrete_type().unwrap_or_else(|| key.name());
            let info =
                ctx.struct_registry
                    .get(lookup_name)
                    .ok_or_else(|| MiddleError::UnboundName {
                        suggestion: None,
                        name: format!("struct `{}` não registrado no StructRegistry", lookup_name),
                        span: (*span).into(),
                    })?;
            let (field_index, field_info) =
                info.find_field(field_name)
                    .ok_or_else(|| MiddleError::UnknownField {
                        struct_name: lookup_name.to_string(),
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
                    struct_name: lookup_name.to_string(),
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
        // .N em List/Array/Bytes/Text/Tensor → desugar para `at receptor N` via INDEXABLE.
        // O dispatch retorna Result::(A, Err) — access checked.
        // `at` tem type_params (A é genérico), então precisa do caminho
        // genérico: percorrer overloads e fazer unify.
        (Ty::List(_) | Ty::Array(_) | Ty::Bytes | Ty::Prim(PrimTy::Text) | Ty::Tensor(_), DotIndex::Int(n)) => {
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
                                suggestion: None,
                            })?;
                    let mut found = None;
                    for oi in overloads.iter().filter(|oi| {
                        oi.params.len() == arg_types.len() && !oi.type_params.is_empty()
                    }) {
                        let mut subs = std::collections::HashMap::new();
                        if unify(
                            &oi.params,
                            &arg_types,
                            &oi.type_params,
                            &mut subs,
                            ctx.refines_registry,
                            ctx.interface_registry,
                        )
                        .is_ok()
                        {
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
                    ctx.refines_registry,
                    ctx.interface_registry,
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
                    ctx.refines_registry,
                    ctx.interface_registry,
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
                            suggestion: None,
                        }
                    })?;
                    let mut found = None;
                    for oi in overloads.iter().filter(|oi| {
                        oi.params.len() == arg_types.len() && !oi.type_params.is_empty()
                    }) {
                        let mut subs = std::collections::HashMap::new();
                        if unify(
                            &oi.params,
                            &arg_types,
                            &oi.type_params,
                            &mut subs,
                            ctx.refines_registry,
                            ctx.interface_registry,
                        )
                        .is_ok()
                        {
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
        // Field access em coleção não faz sentido.
        (
            Ty::List(_) | Ty::Array(_) | Ty::Range(_) | Ty::Bytes | Ty::Prim(PrimTy::Text)
            | Ty::Tensor(_),
            DotIndex::Field(_),
        ) => Err(MiddleError::FieldAccessOnTuple {
            span: (*span).into(),
        }),
        // `tensor.(idx0 idx1 ...)` — indexação N-D em Tensor.
        (Ty::Tensor(inner_ty), DotIndex::Tuple(axes)) => {
            // Verifica se todos os eixos são Int (caso escalar) ou se há
            // Range/Wildcard (caso sub-tensor).
            let all_int = axes.iter().all(|a| matches!(a, TensorAxis::Int(_)));

            if all_int {
                // Caso escalar: todos os eixos são Int → Result::T
                let int_indices: Vec<i64> = axes
                    .iter()
                    .map(|a| {
                        if let TensorAxis::Int(n) = a {
                            *n
                        } else {
                            unreachable!("all_int verificado acima")
                        }
                    })
                    .collect();

                let result_ty = Ty::Generic(
                    "Result".to_string(),
                    vec![(**inner_ty).clone(), Ty::Prim(PrimTy::Text)],
                );

                Ok(TypedExpr {
                    span: *span,
                    ty: result_ty,
                    tail_pos,
                    escape: inner.escape,
                    kind: TypedExprKind::TensorIndex {
                        expr: inner_box,
                        elem_ty: (**inner_ty).clone(),
                        is_scalar: true,
                        int_indices,
                        starts: Vec::new(),
                        ends: Vec::new(),
                        collapse_mask: 0,
                        n_axes: axes.len() as i64,
                    },
                })
            } else {
                // Caso sub-tensor: algum eixo é Range/Wildcard → Tensor
                let mut starts: Vec<i64> = Vec::new();
                let mut ends: Vec<i64> = Vec::new();
                let mut collapse_mask: i64 = 0;

                for (i, axis) in axes.iter().enumerate() {
                    match axis {
                        TensorAxis::Int(n) => {
                            starts.push(*n);
                            ends.push(*n + 1);
                            collapse_mask |= 1 << i;
                        }
                        TensorAxis::Wildcard => {
                            starts.push(0);
                            ends.push(-1); // sentinela: runtime usa shape[axis]
                        }
                        TensorAxis::Range {
                            start,
                            end,
                            inclusive,
                        } => {
                            let start_val = eval_int_literal(&start.node)?;
                            let end_val = eval_int_literal(&end.node)?;
                            starts.push(start_val);
                            ends.push(if *inclusive { end_val + 1 } else { end_val });
                        }
                    }
                }

                let result_ty = Ty::Tensor(inner_ty.clone());

                Ok(TypedExpr {
                    span: *span,
                    ty: result_ty,
                    tail_pos,
                    escape: inner.escape,
                    kind: TypedExprKind::TensorIndex {
                        expr: inner_box,
                        elem_ty: (**inner_ty).clone(),
                        is_scalar: false,
                        int_indices: Vec::new(),
                        starts,
                        ends,
                        collapse_mask,
                        n_axes: axes.len() as i64,
                    },
                })
            }
        }
        // DotIndex::Tuple em tipo não-Tensor é erro.
        (_, DotIndex::Tuple(_)) => Err(MiddleError::NotIndexable {
            ty: format!("{}", inner.ty),
            span: (*span).into(),
        }),
        (other_ty, _) => Err(MiddleError::NotIndexable {
            ty: format!("{other_ty:?}"),
            span: (*span).into(),
        }),
    }
}
