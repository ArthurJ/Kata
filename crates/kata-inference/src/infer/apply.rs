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
use std::collections::HashMap;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::apply_lambda::{infer_apply_lambda, infer_apply_lambda_with_hint};
use super::collections_hof::{infer_filter, infer_fold, infer_map};
use super::expr::{InferCtx, infer_expr};
use super::format_synthesis::infer_format;
use super::helpers::{
    InferResult, dispatch_to_middle_error, peel_grouping_expr, resolve_type_expr,
};
use super::variant_construct::{expand_spread, infer_variant_construct};

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

    // Fio 8 Fase 8: map/filter/fold — interceptados por nome.
    // Não passam pelo DispatchTable. O typeck descobre o tipo concreto
    // do container e produz nó TAST dedicado.
    if func_name == "map" && args.len() == 2 {
        return infer_map(args, span, env, ctx);
    }
    if func_name == "filter" && args.len() == 2 {
        return infer_filter(args, span, env, ctx);
    }
    if func_name == "fold" && args.len() == 3 {
        return infer_fold(args, span, env, ctx);
    }

    // Fio 8: `len tuple` — síntese compile-time.
    // Tuple não implementa COUNTABLE, mas `len (10, 20)` deve retornar 2
    // em compile-time. Intercepta antes do dispatch normal.
    if func_name == "len" && args.len() == 1 {
        let typed_arg = infer_expr(&args[0].node, &args[0].span, env, ctx, false)?;
        if let Ty::Tuple(elements) = &typed_arg.ty {
            return Ok((
                Ty::int(),
                TypedExprKind::IntLit {
                    text: elements.len().to_string(),
                },
                Effect::Puro,
            ));
        }
        // Não é Tuple — cai para o dispatch normal (COUNTABLE) abaixo.
        // Reusar o typed_arg já inferido para evitar dupla inferência.
        let typed_args = vec![Spanned::new(typed_arg, args[0].span)];
        let arg_types: Vec<Ty> = typed_args.iter().map(|t| t.node.ty.clone()).collect();

        // Tenta dispatch normal (match_score).
        if let Ok(overload) = ctx
            .table
            .resolve(&func_name, &arg_types, ctx.interface_registry)
        {
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

        // Tenta caminho genérico: overloads com type_params não-vazio.
        if let Some(overloads) = ctx.table.get_overloads(&func_name) {
            for oi in overloads
                .iter()
                .filter(|oi| oi.params.len() == arg_types.len() && !oi.type_params.is_empty())
            {
                let mut subs: super::generics::Substitutions = HashMap::new();
                if super::generics::unify(&oi.params, &arg_types, &oi.type_params, &mut subs)
                    .is_ok()
                {
                    let concrete_ret = super::generics::apply_subs(&oi.ret, &subs);
                    let callee_ty = Ty::Function(oi.params.clone(), Box::new(concrete_ret.clone()));
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
                        concrete_ret,
                        TypedExprKind::Closure {
                            callee: Box::new(Spanned::new(callee_typed, callee.span)),
                            args: typed_args,
                            ffi_symbol: oi.ffi_symbol.clone(),
                        },
                        Effect::Puro,
                    ));
                }
            }
        }

        return Err(MiddleError::NoOverload {
            name: func_name,
            span: (*span).into(),
        });
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
            let overloads = ctx
                .table
                .get_overloads(&func_name)
                .expect("has_function retornou true, overloads deve existir");
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
                let score = match_score(&arg_types, &oi.params, ctx.interface_registry);
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

        // Fase 5: caminho genérico — se nenhuma overload não-genérica casa,
        // procura overloads com type_params não-vazio e tenta unify.
        let generic_result = ctx
            .table
            .resolve(&func_name, &arg_types, ctx.interface_registry);
        match generic_result {
            Ok(overload) => {
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
            Err(_) => {
                // Tenta caminho genérico: procura overload com type_params não-vazio.
                let mut arity_matched = false;
                let mut unify_failed = false;
                if let Some(overloads) = ctx.table.get_overloads(&func_name) {
                    for oi in overloads.iter().filter(|oi| {
                        oi.params.len() == arg_types.len() && !oi.type_params.is_empty()
                    }) {
                        arity_matched = true;
                        let mut subs: super::generics::Substitutions = HashMap::new();
                        match super::generics::unify(
                            &oi.params,
                            &arg_types,
                            &oi.type_params,
                            &mut subs,
                        ) {
                            Ok(_) => {
                                // Aplica substitutions no tipo de retorno.
                                let concrete_ret = super::generics::apply_subs(&oi.ret, &subs);
                                let callee_ty =
                                    Ty::Function(oi.params.clone(), Box::new(concrete_ret.clone()));
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
                                    concrete_ret,
                                    TypedExprKind::Closure {
                                        callee: Box::new(Spanned::new(callee_typed, callee.span)),
                                        args: typed_args,
                                        ffi_symbol: oi.ffi_symbol.clone(),
                                    },
                                    Effect::Puro,
                                ));
                            }
                            Err(_) => {
                                unify_failed = true;
                            }
                        }
                    }
                }

                // Se uma overload genérica com aridade certa foi encontrada mas
                // unify falhou, o erro é TypeMismatch (inconsistência de tipos).
                if arity_matched && unify_failed {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("argumentos consistentes com type params de {func_name}"),
                        found: format!("unify falhou para {func_name} com args {:?}", arg_types),
                        span: (*span).into(),
                    });
                }

                // Caminho genérico falhou — retorna o erro original do dispatch.
                return Err(dispatch_to_middle_error(
                    ctx.table
                        .resolve(&func_name, &arg_types, ctx.interface_registry)
                        .unwrap_err(),
                    *span,
                ));
            }
        }
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
