//! Aplicação prefixa — resolução de callee.
//!
//! Três caminhos de callee (Fio 2):
//! 1. Lambda inline (DoD 31): `(lambda x: ...) 42` — args fornecem tipos
//! 2. DispatchTable: call direto para FFI ou função Kata nomeada
//! 3. TypeEnv: call_indirect para lambda como valor

use kata_ast::{Expr, Span, Spanned};
use kata_core::dispatch::{OverloadInfo, Score, match_score};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::apply_lambda::{infer_apply_lambda, infer_apply_lambda_with_hint};
use super::expr::{InferCtx, infer_expr};
use super::format_synthesis::infer_format;
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
    hint: Option<&Ty>,
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

    // Fase 6: `format "template {}" (a, b)` — builtin sintetizado.
    // O typeck intercepta `format` e constrói a cadeia de text_replace_first
    // inline. Não passa pelo DispatchTable.
    if func_name == "format" && args.len() == 2 {
        return infer_format(callee, args, span, env, ctx);
    }

    // Fase 7: `$` spread — `f $ (a, b)` expande para `f a b`.
    // Se um arg é `Ident("$")`, o próximo arg deve ser `Tuple` — substitui
    // ambos pelos elementos individuais da tupla.
    let expanded_args = expand_spread(args, span)?;

    // Infere tipos dos argumentos recursivamente (tail_pos = false para args).
    let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(expanded_args.len());
    let mut arg_types: Vec<Ty> = Vec::with_capacity(expanded_args.len());

    for arg in &expanded_args {
        let typed = infer_expr(&arg.node, &arg.span, env, ctx, false)?;
        arg_types.push(typed.ty.clone());
        typed_args.push(Spanned::new(typed, arg.span));
    }

    // Caminho 1: DispatchTable (call direto para FFI ou função Kata nomeada).
    if ctx.table.has_function(&func_name) {
        // Fase 5: Ret-directed dispatch — se hint é Some(ty), filtra overloads
        // cujo retorno é compatível com ty (via fits_return) antes do scoring.
        if let Some(hint_ty) = hint {
            let overloads = ctx.table.get_overloads(&func_name).unwrap();
            let compatible: Vec<&OverloadInfo> = overloads
                .iter()
                .filter(|oi| oi.params.len() == arg_types.len())
                .filter(|oi| super::expr::fits_return(&oi.ret, hint_ty))
                .collect();

            if compatible.is_empty() {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{hint_ty:?} (hint de retorno)"),
                    found: format!(
                        "nenhuma overload de {func_name} retorna tipo compatível com {hint_ty:?}"
                    ),
                    span: (*span).into(),
                });
            }

            // Scoring por dominância entre as overloads compatíveis com o hint.
            // Replicar a lógica de resolve_inner mas só entre os compatíveis.
            let mut best_overload: Option<&OverloadInfo> = None;
            let mut best_score: Option<Score> = None;
            let mut top_count = 0;
            for oi in &compatible {
                let score = match_score(&arg_types, &oi.params);
                if !score.is_compatible(arg_types.len()) {
                    continue;
                }
                let score = Score {
                    is_generic_origin: oi.is_generic,
                    ..score
                };
                match best_score {
                    None => {
                        best_score = Some(score);
                        best_overload = Some(oi);
                        top_count = 1;
                    }
                    Some(bs) if score > bs => {
                        best_score = Some(score);
                        best_overload = Some(oi);
                        top_count = 1;
                    }
                    Some(bs) if score == bs => {
                        top_count += 1;
                    }
                    _ => {}
                }
            }

            if top_count == 0 {
                // Nenhuma overload compatível com o hint tem args que casam.
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{hint_ty:?} (hint de retorno) com args compatíveis"),
                    found: format!(
                        "nenhuma overload de {func_name} com retorno {hint_ty:?} aceita os argumentos fornecidos"
                    ),
                    span: (*span).into(),
                });
            }

            if top_count == 1
                && let Some(oi) = best_overload
            {
                let overload = oi.clone();
                let callee_ty =
                    Ty::Function(overload.params.clone(), Box::new(overload.ret.clone()));
                let callee_typed = TypedExpr {
                    span: callee.span,
                    ty: callee_ty,
                    tail_pos: false,
                    escape: EscapeTarget::Local,
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
                    },
                    Effect::Puro,
                ));
            }

            return Err(MiddleError::AmbiguousDispatch {
                name: func_name,
                span: (*span).into(),
            });
        }

        let overload = ctx
            .table
            .resolve(&func_name, &arg_types)
            .map_err(|e| dispatch_to_middle_error(e, *span))?;

        let callee_ty = Ty::Function(overload.params.clone(), Box::new(overload.ret.clone()));
        let callee_typed = TypedExpr {
            span: callee.span,
            ty: callee_ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
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
                    span: expanded_args[i].span.into(),
                });
            }
        }

        let callee_typed = TypedExpr {
            span: callee.span,
            ty: Ty::Function(param_types.clone(), ret_ty.clone()),
            tail_pos: false,
            escape: EscapeTarget::Local,
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
            },
            Effect::Puro,
        ));
    }

    // Não encontrado em DispatchTable nem TypeEnv.
    // Fallback: pode ser variante com payload desqualificada (ex: `Ok 42`,
    // `Some 42`). Busca no EnumRegistry.
    let candidates = ctx.enum_registry.find_enums_with_variant(&func_name);
    if candidates.len() == 1 {
        let enum_name = candidates[0];
        if ctx
            .enum_registry
            .payload_ty(enum_name, &func_name)
            .is_some()
        {
            return infer_variant_construct(enum_name, &func_name, args, span, env, ctx);
        }
    }
    if candidates.len() > 1 {
        return Err(MiddleError::UnboundName {
            name: format!(
                "variante '{}' é ambígua — existe em: {}. Qualifique (ex: {}::{})",
                func_name,
                candidates.join(", "),
                candidates[0],
                func_name
            ),
            span: callee.span.into(),
        });
    }
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
///
/// Fase 6: Se o enum é genérico, o payload_ty pode ser `Ty::Var("T")`.
/// Nesse caso, unifica `Ty::Var("T")` com `arg.ty` → binding `T = arg.ty`.
/// Produz `Ty::Generic(enum_name, type_args)` onde type_args são os
/// type params instanciados (não-inferidos ficam como `Ty::Var`).
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

    // Fase 6: unificação com Ty::Var.
    if ctx.enum_registry.is_generic(enum_name) {
        let type_params = ctx
            .enum_registry
            .type_params_of(enum_name)
            .expect("is_generic true implica type_params_of Some");

        // Unifica payload_ty (que pode ser Ty::Var) com typed_arg.ty.
        let arg_ty = &typed_arg.ty;
        let mut type_args: Vec<Ty> = Vec::with_capacity(type_params.len());

        for param_name in type_params {
            // Se o payload_ty é Ty::Var(param_name), o arg fornece o tipo concreto.
            if payload_ty == &Ty::Var(param_name.to_string()) {
                type_args.push(arg_ty.clone());
            } else {
                // Type param não-inferido por esta variante — mantém como Ty::Var.
                type_args.push(Ty::Var(param_name.to_string()));
            }
        }

        // Verifica compatibilidade: se payload_ty é Ty::Var, aceita qualquer tipo.
        // Se payload_ty é concreto (não deveria acontecer em enum genérico, mas
        // pode se o payload não usa o type param), compara estruturalmente.
        let compatible = match payload_ty {
            Ty::Var(_) => true,
            _ => payload_ty == arg_ty,
        };
        if !compatible {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{:?}", payload_ty),
                found: format!("{:?}", typed_arg.ty),
                span: args[0].span.into(),
            });
        }

        let result_ty = Ty::Generic(enum_name.to_string(), type_args);
        return Ok((
            result_ty,
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
        ));
    }

    // Fase 5: enum não-genérico — comparação estrutural.
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

/// Fase 7: Expande `$` spread em argumentos de Apply.
///
/// `f $ (a, b)` → `f a b`. Se um arg é `Ident("$")`, o próximo arg deve ser
/// `Expr::Tuple` — substitui ambos (`$` + `Tuple`) pelos elementos individuais.
/// Se `$` não é seguido por tupla → `SpreadRequiresTuple` error.
fn expand_spread(
    args: &[Spanned<Expr>],
    _span: &kata_ast::Span,
) -> Result<Vec<Spanned<Expr>>, MiddleError> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < args.len() {
        // Verifica se é `Ident("$")`
        if let Expr::Ident { name } = &args[i].node
            && name == "$"
        {
            // Próximo arg deve ser Tuple
            if i + 1 >= args.len() {
                return Err(MiddleError::UnboundName {
                    name: "$ spread requires a following tuple".into(),
                    span: args[i].span.into(),
                });
            }
            match &args[i + 1].node {
                Expr::Tuple { elements } => {
                    result.extend(elements.iter().cloned());
                }
                Expr::Grouping { inner } => {
                    if let Expr::Tuple { elements } = &inner.node {
                        result.extend(elements.iter().cloned());
                    } else {
                        return Err(MiddleError::TypeMismatch {
                            expected: "Tuple".into(),
                            found: format!("{:?}", inner.node),
                            span: args[i + 1].span.into(),
                        });
                    }
                }
                _ => {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Tuple after $".into(),
                        found: format!("{:?}", args[i + 1].node),
                        span: args[i + 1].span.into(),
                    });
                }
            }
            i += 2; // pula $ e a tupla
            continue;
        }
        result.push(args[i].clone());
        i += 1;
    }
    Ok(result)
}
