//! DotAccess — field access em struct + index access em tupla.
//!
//! Extraído de `expr.rs` — `infer_dot_access` é self-contained: chama
//! `infer_expr` mas não `infer_expr_hinted`, e tem seu próprio match
//! independente sobre `(Ty, DotIndex)`.

use kata_ast::{DotIndex, Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::generics::{apply_subs, unify};
use super::helpers::InferResult;

/// Infere `expr.nome` (field access) ou `expr.N` (index access).
///
/// Desambiguação pelo tipo do receptor:
/// - `Ty::Struct(name)` + `DotIndex::Field` → `FieldAccess`
/// - `Ty::Struct(name)` + `DotIndex::Int` → erro `IndexAccessOnStruct`
/// - `Ty::Tuple(elements)` + `DotIndex::Int(n)` → `IndexAccess` (negativos
///   normalizados, bounds check compile-time)
/// - `Ty::Tuple(elements)` + `DotIndex::Field` → erro `FieldAccessOnTuple`
/// - `Ty::List(A)` / `Ty::Array(A)` + `DotIndex::Int(n)` → desugar para
///   `at receptor n` via INDEXABLE dispatch (retorna `Result::(A, Err)`)
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
        // .N em List/Array → desugar para `at receptor N` via INDEXABLE.
        // O dispatch retorna Result::(A, Err) — access checked.
        // `at` tem type_params (A é genérico), então precisa do caminho
        // genérico: percorrer overloads e fazer unify.
        (Ty::List(_) | Ty::Array(_), DotIndex::Int(n)) => {
            let arg_types = vec![inner.ty.clone(), Ty::int()];
            // Tenta caminho não-genérico primeiro.
            let overload = ctx.table.resolve("at", &arg_types, ctx.interface_registry);
            let (ret_ty, ffi_symbol, params) = match overload {
                Ok(oi) => (oi.ret, oi.ffi_symbol, oi.params),
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
                            found = Some((concrete_ret, oi.ffi_symbol.clone(), oi.params.clone()));
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
        // Range não implementa INDEXABLE — .N é type error.
        (Ty::Range(_), DotIndex::Int(_)) => Err(MiddleError::NotIndexable {
            ty: format!("{}", inner.ty),
            span: (*span).into(),
        }),
        // Field access em coleção não faz sentido.
        (Ty::List(_) | Ty::Array(_) | Ty::Range(_), DotIndex::Field(_)) => {
            Err(MiddleError::FieldAccessOnTuple {
                span: (*span).into(),
            })
        }
        (other_ty, _) => Err(MiddleError::NotIndexable {
            ty: format!("{other_ty:?}"),
            span: (*span).into(),
        }),
    }
}
