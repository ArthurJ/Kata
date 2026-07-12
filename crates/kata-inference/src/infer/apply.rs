//! Aplicação prefixa — resolução de callee.
//!
//! Três caminhos de callee (Fio 2):
//! 1. Lambda inline (DoD 31): `(lambda x: ...) 42` — args fornecem tipos
//! 2. DispatchTable: call direto para FFI ou função Kata nomeada
//! 3. TypeEnv: call_indirect para lambda como valor

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::apply_lambda::{infer_apply_lambda, infer_apply_lambda_with_hint};
use super::expr::{InferCtx, infer_expr};
use super::helpers::{
    InferResult, dispatch_to_middle_error, peel_grouping_expr, resolve_type_expr,
};

/// Infere uma aplicação prefixa — dois caminhos de callee (Fio 2).
///
/// 1. Callee é nome no DispatchTable: `table.resolve(name, arg_types)` → call direto.
/// 2. Callee é variável no TypeEnv com `Ty::Function`: `call_indirect` no codegen.
///
/// DispatchTable vence se encontrado em ambos (call direto é mais eficiente).
pub(crate) fn infer_apply(
    callee: &Spanned<Expr>,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // DoD 31: Apply de lambda inline — se o callee é um lambda (possivelmente
    // envolto em Grouping ou TypeAscription), inferir args primeiro (síntese
    // bottom-up), usar arg_tys como tipos dos parâmetros do lambda, e inferir
    // o body com tipos conhecidos.
    let callee_core = peel_grouping_expr(&callee.node);
    match callee_core {
        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => {
            return infer_apply_lambda(patterns, body, guards, with_bindings, args, span, env, ctx);
        }
        // Fase 5: Apply(VariantQual, [arg]) → construção de Sum com payload.
        Expr::VariantQual { enum_name, variant } => {
            return infer_variant_construct(enum_name, variant, args, span, env, ctx);
        }
        Expr::TypeAscription { expr: inner, ty } => {
            // Ascription em lambda: `((lambda ...)::(Int -> Int)) 42`.
            // O hint da ascription fornece os tipos dos params.
            let inner_core = peel_grouping_expr(&inner.node);
            if let Expr::Lambda {
                patterns,
                body,
                guards,
                with_bindings,
            } = inner_core
            {
                let hint_ty = resolve_type_expr(&ty.node, env);
                return infer_apply_lambda_with_hint(
                    patterns,
                    body,
                    guards,
                    with_bindings,
                    args,
                    &hint_ty,
                    span,
                    env,
                    ctx,
                );
            }
            // Ascription em non-lambda: não é apply de lambda inline.
            // Cair para o caminho de Ident abaixo (que vai falhar com UnboundName).
        }
        _ => {}
    }

    let func_name = match callee_core {
        Expr::Ident { name } => name.clone(),
        _ => {
            return Err(MiddleError::UnboundName {
                name: "<non-ident callee>".into(),
                span: callee.span.into(),
            });
        }
    };

    // Infere tipos dos argumentos recursivamente (tail_pos = false para args).
    let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(args.len());
    let mut arg_types: Vec<Ty> = Vec::with_capacity(args.len());

    for arg in args {
        let typed = infer_expr(&arg.node, &arg.span, env, ctx, false)?;
        arg_types.push(typed.ty.clone());
        typed_args.push(Spanned::new(typed, arg.span));
    }

    // Caminho 1: DispatchTable (call direto para FFI ou função Kata nomeada).
    if ctx.table.has_function(&func_name) {
        let overload = ctx
            .table
            .resolve(&func_name, &arg_types)
            .map_err(|e| dispatch_to_middle_error(e, *span))?;

        let callee_ty = Ty::Function(overload.params.clone(), Box::new(overload.ret.clone()));
        let callee_typed = TypedExpr {
            span: callee.span,
            ty: callee_ty,
            tail_pos: false,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident {
                name: func_name.clone(),
            },
        };

        return Ok((
            overload.ret,
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: typed_args,
                ffi_symbol: overload.ffi_symbol,
                captures: Vec::new(),
                escapes: false,
            },
            Effect::Puro,
        ));
    }

    // Caminho 2: TypeEnv (call_indirect para lambda como valor).
    if let Some(Ty::Function(param_types, ret_ty)) = env.lookup(&func_name).cloned() {
        // Verifica aridade.
        if arg_types.len() != param_types.len() {
            return Err(MiddleError::ArityMismatch {
                expected: param_types.len(),
                found: arg_types.len(),
                span: (*span).into(),
            });
        }
        // Verifica tipos dos argumentos.
        for (i, (arg_ty, param_ty)) in arg_types.iter().zip(param_types.iter()).enumerate() {
            if arg_ty != param_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", param_ty),
                    found: format!("{:?}", arg_ty),
                    span: args[i].span.into(),
                });
            }
        }

        let callee_typed = TypedExpr {
            span: callee.span,
            ty: Ty::Function(param_types.clone(), ret_ty.clone()),
            tail_pos: false,
            effect: Effect::Puro,
            kind: TypedExprKind::Ident {
                name: func_name.clone(),
            },
        };

        return Ok((
            (*ret_ty).clone(),
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: typed_args,
                ffi_symbol: None, // call_indirect — sem FFI symbol
                captures: Vec::new(),
                escapes: false,
            },
            Effect::Puro,
        ));
    }

    // Não encontrado em nenhum lugar.
    Err(MiddleError::UnboundName {
        name: func_name,
        span: callee.span.into(),
    })
}

/// Fase 5: Infere `Apply(VariantQual("Enum", "Variant"), [arg])` —
/// construção de Sum com payload.
///
/// Verifica que a variante existe no EnumRegistry, que tem payload,
/// e que o tipo do argumento é compatível com o tipo do payload.
/// Produz `TypedExprKind::VariantConstruct { enum_name, variant, payload }`.
fn infer_variant_construct(
    enum_name: &str,
    variant: &str,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    use kata_core::ty::Ty;

    // Verifica que o enum e a variante existem.
    if !ctx.enum_registry.is_variant(enum_name, variant) {
        return Err(MiddleError::UnboundName {
            name: format!("{}::{}", enum_name, variant),
            span: (*span).into(),
        });
    }

    // Verifica que a variante tem payload.
    let payload_ty = ctx
        .enum_registry
        .payload_ty(enum_name, variant)
        .ok_or_else(|| MiddleError::TypeMismatch {
            expected: "variante com payload".into(),
            found: format!("{}::{} é unitária", enum_name, variant),
            span: (*span).into(),
        })?;

    // Fase 5: exatamente 1 argumento.
    if args.len() != 1 {
        return Err(MiddleError::ArityMismatch {
            expected: 1,
            found: args.len(),
            span: (*span).into(),
        });
    }

    // Infere o argumento (tail_pos = false — é computação local).
    let typed_arg = infer_expr(&args[0].node, &args[0].span, env, ctx, false)?;

    // Verifica que o tipo do argumento é compatível com o payload.
    if typed_arg.ty != *payload_ty {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{:?}", payload_ty),
            found: format!("{:?}", typed_arg.ty),
            span: args[0].span.into(),
        });
    }

    Ok((
        Ty::Sum(enum_name.to_string()),
        TypedExprKind::VariantConstruct {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            payload: Box::new(Spanned::new(typed_arg, args[0].span)),
            tag: ctx
                .enum_registry
                .variant_index(enum_name, variant)
                .unwrap_or(0),
        },
        Effect::Puro,
    ))
}
