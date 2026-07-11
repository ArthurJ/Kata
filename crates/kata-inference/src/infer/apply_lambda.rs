//! DoD 31: Apply de lambda inline e fatoração do body de lambda.
//!
//! Três funções extraídas do núcleo de inferência:
//! - `infer_apply_lambda`: `(lambda x: ...) 42` — args fornecem tipos dos params
//! - `infer_apply_lambda_with_hint`: `((lambda ...)::(Int -> Int)) 42` — hint da ascription
//! - `infer_lambda_body`: fatoração do body com/sem guards (compartilhada)
//!
//! As duas funções de apply convergem em `build_lambda_apply`, que recebe
//! os `param_tys` já resolvidos (seja por síntese dos args, seja por hint)
//! e executa o pipeline comum: push_scope → check_patterns →
//! process_with_bindings → infer_lambda_body → montar Closure.

use kata_ast::{Expr, GuardClause, Pattern, Span, Spanned, WithBinding};
use kata_core::dispatch::DispatchTable;
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind, TypedGuardClause, TypedLambdaClause};

use super::expr::{InferCtx, infer_expr};
use super::helpers::{InferResult, check_patterns, process_with_bindings};

/// Infere `(lambda x: ...) 42` — args fornecem tipos dos parâmetros.
///
/// Síntese bottom-up: infere cada arg, usa `arg_tys` como tipos dos params
/// do lambda. Infere o body com tipos conhecidos. Retorna `Closure` com
/// `ffi_symbol=None` (call_indirect no codegen).
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_apply_lambda(
    patterns: &[Spanned<Pattern>],
    body: &Spanned<Expr>,
    guards: &[GuardClause],
    with_bindings: &[WithBinding],
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Verifica aridade.
    if args.len() != patterns.len() {
        return Err(MiddleError::ArityMismatch {
            expected: patterns.len(),
            found: args.len(),
            span: (*span).into(),
        });
    }

    // Infere args (síntese bottom-up).
    let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(args.len());
    let mut arg_tys: Vec<Ty> = Vec::with_capacity(args.len());
    for arg in args {
        let typed = infer_expr(&arg.node, &arg.span, env, ctx, false)?;
        arg_tys.push(typed.ty.clone());
        typed_args.push(Spanned::new(typed, arg.span));
    }

    build_lambda_apply(
        patterns,
        body,
        guards,
        with_bindings,
        arg_tys,
        typed_args,
        None,
        span,
        env,
        ctx,
    )
}

/// Infere `((lambda ...)::(Int -> Int)) 42` — hint da ascription fornece os
/// tipos dos params, args são verificados contra eles.
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_apply_lambda_with_hint(
    patterns: &[Spanned<Pattern>],
    body: &Spanned<Expr>,
    guards: &[GuardClause],
    with_bindings: &[WithBinding],
    args: &[Spanned<Expr>],
    hint_ty: &Ty,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Verifica aridade.
    if args.len() != patterns.len() {
        return Err(MiddleError::ArityMismatch {
            expected: patterns.len(),
            found: args.len(),
            span: (*span).into(),
        });
    }

    // Extrai param types do hint.
    let (hint_params, hint_ret) = match hint_ty {
        Ty::Function(params, ret) => (params.clone(), (**ret).clone()),
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{hint_ty:?}"),
                span: (*span).into(),
            });
        }
    };

    if hint_params.len() != patterns.len() {
        return Err(MiddleError::ArityMismatch {
            expected: patterns.len(),
            found: hint_params.len(),
            span: (*span).into(),
        });
    }

    // Infere args e verifica contra hint_params.
    let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let typed = infer_expr(&arg.node, &arg.span, env, ctx, false)?;
        if typed.ty != hint_params[i] {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{:?}", hint_params[i]),
                found: format!("{:?}", typed.ty),
                span: arg.span.into(),
            });
        }
        typed_args.push(Spanned::new(typed, arg.span));
    }

    build_lambda_apply(
        patterns,
        body,
        guards,
        with_bindings,
        hint_params,
        typed_args,
        Some(&hint_ret),
        span,
        env,
        ctx,
    )
}

/// Pipeline comum aos dois caminhos de apply de lambda inline.
///
/// Recebe `param_tys` já resolvidos (por síntese dos args ou por hint) e
/// `typed_args` já inferidos. Monta o escopo filho, casa padrões, processa
/// with bindings, infere o body, e produz a `Closure`.
///
/// `ret_check`: `Some(expected)` verifica que o tipo de retorno inferido do
/// body bate com o esperado (caminho com hint). `None` pula a verificação
/// (caminho por síntese — o tipo de retorno é o que o body produz).
#[allow(clippy::too_many_arguments)]
fn build_lambda_apply(
    patterns: &[Spanned<Pattern>],
    body: &Spanned<Expr>,
    guards: &[GuardClause],
    with_bindings: &[WithBinding],
    param_tys: Vec<Ty>,
    typed_args: Vec<Spanned<TypedExpr>>,
    ret_check: Option<&Ty>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    // Cria escopo filho e define params com tipos conhecidos.
    let mut lambda_env = env.push_scope();
    let typed_patterns = check_patterns(patterns, &param_tys, ctx.enum_registry, &mut lambda_env)?;

    // Processa with bindings.
    let typed_with_bindings = process_with_bindings(with_bindings, &mut lambda_env, ctx)?;

    // Infere o body.
    let (ret_ty, typed_body, typed_guards) = infer_lambda_body(body, guards, &mut lambda_env, ctx)?;

    // Verifica o tipo de retorno se um esperado foi fornecido (caminho com hint).
    if let Some(expected_ret) = ret_check
        && ret_ty != *expected_ret
    {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{:?}", expected_ret),
            found: format!("{:?}", ret_ty),
            span: body.span.into(),
        });
    }

    // O tipo do lambda usa o tipo de retorno verificado (com hint) ou inferido (sem hint).
    let lambda_ret = ret_check.cloned().unwrap_or(ret_ty.clone());
    let lambda_ty = Ty::Function(param_tys.clone(), Box::new(lambda_ret.clone()));

    let lambda_kind = TypedExprKind::Lambda {
        func_name: None,
        param_types: param_tys,
        ret_ty: lambda_ret.clone(),
        clauses: vec![TypedLambdaClause {
            patterns: typed_patterns,
            body: Spanned::new(typed_body, body.span),
            guards: typed_guards,
            with_bindings: typed_with_bindings,
        }],
    };

    let callee_typed = TypedExpr {
        span: *span,
        ty: lambda_ty,
        tail_pos: false,
        effect: Effect::Puro,
        kind: lambda_kind,
    };

    Ok((
        lambda_ret,
        TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee_typed, *span)),
            args: typed_args,
            ffi_symbol: None,
            captures: Vec::new(),
            escapes: false,
        },
        Effect::Puro,
    ))
}

/// Infere o body de um lambda (com ou sem guards) — fatorado de infer_lambda.
pub(crate) fn infer_lambda_body(
    body: &Spanned<Expr>,
    guards: &[GuardClause],
    lambda_env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExpr, Vec<TypedGuardClause>)> {
    let mut typed_guards: Vec<TypedGuardClause> = Vec::new();
    let (ret_ty, typed_body) = if guards.is_empty() {
        let typed_body = infer_expr(&body.node, &body.span, lambda_env, ctx, true)?;
        (typed_body.ty.clone(), typed_body)
    } else {
        let mut guard_ret_ty: Option<Ty> = None;
        for guard in guards {
            if let Some(cond) = &guard.condition {
                let cond_typed = infer_expr(&cond.node, &cond.span, lambda_env, ctx, false)?;
                if cond_typed.ty != Ty::boolean() {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Boolean".into(),
                        found: format!("{:?}", cond_typed.ty),
                        span: cond.span.into(),
                    });
                }
                let body_typed =
                    infer_expr(&guard.body.node, &guard.body.span, lambda_env, ctx, true)?;
                if let Some(ref existing) = guard_ret_ty {
                    if *existing != body_typed.ty {
                        return Err(MiddleError::TypeMismatch {
                            expected: format!("{:?}", existing),
                            found: format!("{:?}", body_typed.ty),
                            span: guard.body.span.into(),
                        });
                    }
                } else {
                    guard_ret_ty = Some(body_typed.ty.clone());
                }
                typed_guards.push(TypedGuardClause {
                    condition: Some(Spanned::new(cond_typed, cond.span)),
                    body: Spanned::new(body_typed, guard.body.span),
                });
                continue;
            }
            let body_typed = infer_expr(&guard.body.node, &guard.body.span, lambda_env, ctx, true)?;
            if let Some(ref existing) = guard_ret_ty {
                if *existing != body_typed.ty {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{:?}", existing),
                        found: format!("{:?}", body_typed.ty),
                        span: guard.body.span.into(),
                    });
                }
            } else {
                guard_ret_ty = Some(body_typed.ty.clone());
            }
            typed_guards.push(TypedGuardClause {
                condition: None,
                body: Spanned::new(body_typed, guard.body.span),
            });
        }
        (
            guard_ret_ty
                .clone()
                .expect("pelo menos um guard deve existir"),
            TypedExpr {
                span: body.span,
                ty: guard_ret_ty.expect("pelo menos um guard"),
                tail_pos: true,
                effect: Effect::Puro,
                kind: TypedExprKind::Unit,
            },
        )
    };
    Ok((ret_ty, typed_body, typed_guards))
}
