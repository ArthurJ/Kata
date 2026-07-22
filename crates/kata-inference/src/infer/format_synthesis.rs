//! `format` builtin — interceptado no typeck, não no DispatchTable.
//!
//! `format "template {} {}" (a, b)` sintetiza a cadeia:
//!   text_replace_first(
//!     text_replace_first("template {} {}", repr_a),
//!     repr_b
//!   )
//!
//! Para cada argumento na tupla, converte para Text (como `repr` faz) e
//! substitui a primeira ocorrência de `{}` no template acumulado.

use kata_ast::{Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Intercepta `format "template" (args_tuple)` e sintetiza a cadeia de
/// `text_replace_first` com a representação Text de cada argumento.
pub(crate) fn infer_format(
    _callee: &Spanned<Expr>,
    args: &[Spanned<Expr>],
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Arg 0: template (deve ser Text)
    let template_expr = infer_expr(&args[0].node, &args[0].span, env, ctx, false)?;
    if template_expr.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", template_expr.ty),
            span: args[0].span.into(),
        });
    }
    let template_typed = Spanned::new(template_expr, args[0].span);

    // Arg 1: tupla de valores a interpolar
    let tuple_core = &args[1].node;
    let elements = match tuple_core {
        Expr::Tuple { elements } => elements,
        Expr::Unit => &[][..], // tupla vazia `()` = sem args
        Expr::Grouping { inner } => {
            // Grouping de uma tupla: (a, b) dentro de parênteses extras
            match &inner.node {
                Expr::Tuple { elements } => elements,
                _ => {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Tuple".into(),
                        found: format!("{:?}", inner.node),
                        span: args[1].span.into(),
                    });
                }
            }
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Tuple".into(),
                found: format!("{:?}", tuple_core),
                span: args[1].span.into(),
            });
        }
    };

    // Infere cada elemento da tupla e converte para Text
    let mut text_parts: Vec<Spanned<TypedExpr>> = Vec::new();
    for elem in elements {
        let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
        let text_typed = convert_to_text(Spanned::new(typed, elem.span));
        text_parts.push(text_typed);
    }

    // Constrói a cadeia de text_replace_first:
    //   text_replace_first(template, part0)
    //   text_replace_first(^, part1)
    //   ...
    let mut result = template_typed;
    for part in text_parts {
        result = text_replace_first(result, part);
    }

    Ok((Ty::text(), result.node.kind, Effect::Puro))
}

/// Converte um TypedExpr para Text, baseado no tipo.
///
/// - Text: identity
/// - Int: int_to_text (FFI)
/// - Boolean: bool_to_text (FFI)
/// - Struct: repr (call direto via ffi_symbol mangled)
/// - Outros: int_to_text como fallback
fn convert_to_text(expr: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    let ty = &expr.node.ty;
    match ty {
        Ty::Prim(PrimTy::Text) => expr,
        Ty::Prim(PrimTy::Int) => ffi_call1("kata_rt_int_to_text", expr, Ty::text()),
        Ty::Prim(PrimTy::Rational) => ffi_call1("kata_rt_rat_show", expr, Ty::text()),
        Ty::Prim(PrimTy::Float) => {
            // Sem FFI de float_to_text — fallback para int_to_text
            ffi_call1("kata_rt_int_to_text", expr, Ty::text())
        }
        Ty::Sum(name) if name == "Boolean" => {
            // Boolean ganha `show` sintetizado (variantes True/False).
            let mangled = format!("__kata_show__{name}");
            repr_call(expr, mangled)
        }
        Ty::Sum(name) => {
            // Enum — chama `__kata_show__{name}` via ffi_symbol mangled.
            let mangled = format!("__kata_show__{name}");
            repr_call(expr, mangled)
        }
        Ty::Struct(name) => {
            // Struct — chama `__kata_show__{name}` via ffi_symbol mangled.
            let mangled = format!("__kata_show__{name}");
            repr_call(expr, mangled)
        }
        _ => ffi_call1("kata_rt_int_to_text", expr, Ty::text()),
    }
}

/// Constrói `Closure { callee=Ident(ffi), args=[arg], ffi_symbol=Some(ffi) }`.
fn ffi_call1(ffi_name: &str, arg: Spanned<TypedExpr>, ret_ty: Ty) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(ret_ty.clone())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: ffi_name.to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: ret_ty,
            tail_pos: false,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![arg],
                ffi_symbol: Some(ffi_name.to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `text_replace_first(template, replacement)` — FFI call binário.
fn text_replace_first(
    template: Spanned<TypedExpr>,
    replacement: Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::text(), Ty::text()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "kata_rt_text_replace_first".to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![template, replacement],
                ffi_symbol: Some("kata_rt_text_replace_first".to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói chamada a `repr` (call direto via ffi_symbol mangled).
fn repr_call(field_access: Spanned<TypedExpr>, mangled: String) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![field_access.node.ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: mangled.clone(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![field_access],
                ffi_symbol: Some(mangled),
            },
        },
        Span::synthetic(),
    )
}
