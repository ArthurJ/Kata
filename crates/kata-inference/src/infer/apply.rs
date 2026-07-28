//! Aplicação prefixa — resolução de callee.
//!
//! Três caminhos de callee:
//! 1. Lambda inline (DoD 31): `(lambda x: ...) 42` — args fornecem tipos
//! 2. DispatchTable: call direto para FFI ou função Kata nomeada
//! 3. TypeEnv: call_indirect para lambda como valor

use kata_ast::{Expr, Span, Spanned};
use kata_core::dispatch::{OverloadInfo, Score, match_score};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use std::collections::HashMap;

use crate::typed::{TypedExpr, TypedExprKind};

use super::apply_lambda::{infer_apply_lambda, infer_apply_lambda_with_hint};
use super::collections_hof::{infer_filter, infer_fold, infer_map};
use super::expr::{InferCtx, infer_expr};
use super::format_synthesis::infer_format;
use super::helpers::{InferResult, dispatch_to_middle_error, peel_grouping_expr};
use super::variant_construct::{VariantCall, expand_spread, infer_variant_construct};
use kata_resolution::resolve_type_expr;

use super::iface_dispatch::try_iface_method_dispatch;

/// Infere uma aplicação prefixa — dois caminhos de callee.
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
) -> InferResult<(Ty, TypedExprKind)> {
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
        // Apply(VariantQual, [arg]) → construção de Sum com payload.
        Expr::VariantQual {
            enum_name,
            variant,
            module_path,
            ..
        } => {
            return infer_variant_construct(
                &VariantCall {
                    enum_name,
                    variant,
                    module_path: module_path.as_deref(),
                    args,
                    span,
                },
                env,
                ctx,
                hint,
            );
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
                let hint_ty = resolve_type_expr(&ty.node, env, ctx.interface_registry);
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
        Expr::DotAccess { expr: inner, index } => {
            // Module access: `mod.fn args` — o callee é `DotAccess`.
            // Tenta resolver como module access primeiro. Se `mod` não é
            // variável local e `mod.fn` está no DispatchTable, usa o nome
            // qualificado. Caso contrário, cai para o erro de non-ident callee.
            if let Expr::Ident { name: mod_name } = &inner.node {
                if let kata_ast::DotIndex::Field(field_name) = index {
                    if env.lookup(mod_name).is_none() {
                        let qual_name = format!("{mod_name}.{field_name}");
                        if ctx.table.get_overloads(&qual_name).is_some() {
                            qual_name
                        } else {
                            return Err(MiddleError::UnboundName {
                                name: format!(
                                    "`{mod_name}.{field_name}` não encontrado no DispatchTable"
                                ),
                                span: callee.span.into(),
                            });
                        }
                    } else {
                        // `mod` é variável local — DotAccess é field access em
                        // struct, não module access. Não é um callee válido.
                        return Err(MiddleError::UnboundName {
                            name: "<non-ident callee>".into(),
                            span: callee.span.into(),
                        });
                    }
                } else {
                    return Err(MiddleError::UnboundName {
                        name: "<non-ident callee>".into(),
                        span: callee.span.into(),
                    });
                }
            } else {
                return Err(MiddleError::UnboundName {
                    name: "<non-ident callee>".into(),
                    span: callee.span.into(),
                });
            }
        }
        _ => {
            return Err(MiddleError::UnboundName {
                name: "<non-ident callee>".into(),
                span: callee.span.into(),
            });
        }
    };

    // `format "template {}" (a, b)` — builtin sintetizado.
    // O typeck intercepta `format` e constrói a cadeia de text_replace_first
    // inline. Não passa pelo DispatchTable.
    if func_name == "format" && args.len() == 2 {
        return infer_format(callee, args, span, env, ctx);
    }

    // Map/filter/fold — interceptados por nome.
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

    // `len tuple` — síntese compile-time.
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
            let expanded_ret = expand_ret(&overload.ret, ctx);
            let callee_ty = Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
            let callee_typed = TypedExpr {
                span: callee.span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: func_name.clone(),
                },
            };
            return Ok((
                expanded_ret,
                TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                    args: typed_args,
                    ffi_symbol: overload.ffi_symbol,
                },
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
                    let expanded_ret = expand_ret(&concrete_ret, ctx);
                    let callee_ty = Ty::Function(oi.params.clone(), Box::new(expanded_ret.clone()));
                    let callee_typed = TypedExpr {
                        span: callee.span,
                        ty: callee_ty,
                        tail_pos: false,
                        escape: EscapeTarget::Local,
                        kind: TypedExprKind::Ident {
                            name: func_name.clone(),
                        },
                    };
                    reject_action_arg_for_pure_fn(oi, &typed_args, span)?;
                    return Ok((
                        expanded_ret,
                        TypedExprKind::Closure {
                            callee: Box::new(Spanned::new(callee_typed, callee.span)),
                            args: typed_args,
                            ffi_symbol: oi.ffi_symbol.clone(),
                        },
                    ));
                }
            }
        }

        // Caminho genérico falhou. Tentar fallback refines (D1 do PRD-refines):
        // se algum arg é tipo refined com delegação refines, substituir pelo
        // tipo base e retentar o dispatch. O refined é alias do base no layout
        // — o codegen não precisa de conversão de bits, mas precisa do tipo
        // base correto para mapear F64/I64 no Cranelift.
        if let Some((fallback_arg_types, fallback_overload)) =
            try_refines_fallback(&func_name, &arg_types, ctx)
        {
            // Converter typed_args para o tipo base onde o fallback substituiu.
            // Isto é um no-op em runtime (mesmos bits), mas garante que o codegen
            // use o Cranelift type correto (F64 para Float, I64 para Int).
            let converted_args: Vec<Spanned<TypedExpr>> = typed_args
                .iter()
                .zip(fallback_arg_types.iter())
                .map(|(arg, fallback_ty)| {
                    if arg.node.ty != *fallback_ty {
                        Spanned::new(
                            TypedExpr {
                                span: arg.span,
                                ty: fallback_ty.clone(),
                                tail_pos: arg.node.tail_pos,
                                escape: arg.node.escape,
                                kind: TypedExprKind::TypeAscription {
                                    expr: Box::new(arg.clone()),
                                    target_ty: fallback_ty.clone(),
                                },
                            },
                            arg.span,
                        )
                    } else {
                        arg.clone()
                    }
                })
                .collect();
            let callee_ty = Ty::Function(
                fallback_overload.params.clone(),
                Box::new(expand_ret(&fallback_overload.ret, ctx)),
            );
            let callee_typed = TypedExpr {
                span: callee.span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: func_name.clone(),
                },
            };
            return Ok((
                expand_ret(&fallback_overload.ret, ctx),
                TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                    args: converted_args,
                    ffi_symbol: fallback_overload.ffi_symbol,
                },
            ));
        }

        return Err(MiddleError::NoOverload {
            name: func_name,
            span: (*span).into(),
        });
    }

    // `$` spread — `f $ (a, b)` expande para `f a b`.
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

    // Caminho 0: Dispatch por método de interface (iface method dispatch).
    // Quando um argumento é tipado como `Interface("SHOW")` e `func_name`
    // é método dessa interface (ex: `show`), aceita o dispatch — o tipo
    // concreto será resolvido pelo monomorphizador ao instanciar a Action
    // polimórfica que contém esta chamada.
    //
    // Procura no InterfaceRegistry pela interface que o arg implementa e
    // verifica se `func_name` é uma de suas signatures. Substitui `Self`
    // pelo tipo da interface na signature para obter o tipo de retorno.
    if let Some(iface_method_ret) =
        try_iface_method_dispatch(&func_name, &arg_types, ctx.interface_registry)
    {
        let expanded_ret = expand_ret(&iface_method_ret, ctx);
        let callee_ty = Ty::Function(arg_types.clone(), Box::new(expanded_ret.clone()));
        let callee_typed = TypedExpr {
            span: callee.span,
            ty: callee_ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::Ident {
                name: func_name.clone(),
            },
        };
        return Ok((
            expanded_ret,
            TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                args: typed_args,
                ffi_symbol: None,
            },
        ));
    }

    // Caminho 1: DispatchTable (call direto para FFI ou função Kata nomeada).
    if ctx.table.has_function(&func_name) {
        // Ret-directed dispatch — se hint é Some(ty), filtra overloads
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
                // Nenhuma overload compatível com o hint tem args que casam
                // via match_score. Pode ser que uma overload genérica casasse
                // via unify (ex: `+ :: List::A List::A => List::A` com args
                // `List(Int)`), mas match_score não unifica type params em
                // Ty::List. Cair para o caminho genérico abaixo em vez de
                // retornar erro imediatamente.
            } else if top_count == 1
                && let Some(oi) = best_overload
            {
                let overload = oi.clone();
                reject_action_arg_for_pure_fn(&overload, &typed_args, span)?;
                let expanded_ret = expand_ret(&overload.ret, ctx);
                let callee_ty =
                    Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
                let callee_typed = TypedExpr {
                    span: callee.span,
                    ty: callee_ty,
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::Ident {
                        name: func_name.clone(),
                    },
                };
                return Ok((
                    expanded_ret,
                    TypedExprKind::Closure {
                        callee: Box::new(Spanned::new(callee_typed, callee.span)),
                        args: typed_args,
                        ffi_symbol: overload.ffi_symbol,
                    },
                ));
            } else {
                // top_count > 1 — ambíguo
                return Err(MiddleError::AmbiguousDispatch {
                    name: func_name,
                    span: (*span).into(),
                });
            }
            // top_count == 0: cair para o caminho genérico abaixo
        }

        // Caminho genérico — se nenhuma overload não-genérica casa,
        // procura overloads com type_params não-vazio e tenta unify.
        let generic_result = ctx
            .table
            .resolve(&func_name, &arg_types, ctx.interface_registry);
        match generic_result {
            Ok(overload) => {
                reject_action_arg_for_pure_fn(&overload, &typed_args, span)?;
                let expanded_ret = expand_ret(&overload.ret, ctx);
                let callee_ty =
                    Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
                let callee_typed = TypedExpr {
                    span: callee.span,
                    ty: callee_ty,
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    kind: TypedExprKind::Ident {
                        name: func_name.clone(),
                    },
                };

                return Ok((
                    expanded_ret,
                    TypedExprKind::Closure {
                        callee: Box::new(Spanned::new(callee_typed, callee.span)),
                        args: typed_args,
                        ffi_symbol: overload.ffi_symbol,
                    },
                ));
            }
            Err(_) => {
                // Tenta caminho genérico: procura overload com type_params não-vazio.
                let mut arity_matched = false;
                let mut unify_failed = false;
                let mut total_candidates = 0u32;
                if let Some(overloads) = ctx.table.get_overloads(&func_name) {
                    for oi in overloads
                        .iter()
                        .filter(|oi| oi.params.len() == arg_types.len())
                    {
                        total_candidates += 1;
                        if oi.type_params.is_empty() {
                            continue;
                        }
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
                                let expanded_ret = expand_ret(&concrete_ret, ctx);
                                reject_action_arg_for_pure_fn(oi, &typed_args, span)?;
                                let callee_ty =
                                    Ty::Function(oi.params.clone(), Box::new(expanded_ret.clone()));
                                let callee_typed = TypedExpr {
                                    span: callee.span,
                                    ty: callee_ty,
                                    tail_pos: false,
                                    escape: EscapeTarget::Local,
                                    kind: TypedExprKind::Ident {
                                        name: func_name.clone(),
                                    },
                                };

                                return Ok((
                                    expanded_ret,
                                    TypedExprKind::Closure {
                                        callee: Box::new(Spanned::new(callee_typed, callee.span)),
                                        args: typed_args,
                                        ffi_symbol: oi.ffi_symbol.clone(),
                                    },
                                ));
                            }
                            Err(_) => {
                                unify_failed = true;
                            }
                        }
                    }
                }

                // Se uma overload genérica com aridade certa foi encontrada mas
                // unify falhou, decidir o erro pela cardinalidade total de
                // overloads com aquela aridade:
                // - 1 overload total: o usuário quis essa overload; unify falhou
                //   por tipos inconsistentes → TypeMismatch.
                // - >1 overloads totais: nenhuma casou (concretas nem genéricas)
                //   → NoOverload — não há como o usuário pretendesse uma específica.
                //
                // ANTES de decidir o erro, tentar fallback refines: se algum arg
                // é refined com delegação, substituir pelo tipo base e retentar.
                if arity_matched && unify_failed {
                    // Tentar fallback refines antes de retornar erro.
                    if let Some((_fallback_arg_types, fallback_overload)) =
                        try_refines_fallback(&func_name, &arg_types, ctx)
                    {
                        let expanded_ret = expand_ret(&fallback_overload.ret, ctx);
                        let callee_ty = Ty::Function(
                            fallback_overload.params.clone(),
                            Box::new(expanded_ret.clone()),
                        );
                        let callee_typed = TypedExpr {
                            span: callee.span,
                            ty: callee_ty,
                            tail_pos: false,
                            escape: EscapeTarget::Local,
                            kind: TypedExprKind::Ident {
                                name: func_name.clone(),
                            },
                        };
                        return Ok((
                            expanded_ret,
                            TypedExprKind::Closure {
                                callee: Box::new(Spanned::new(callee_typed, callee.span)),
                                args: typed_args,
                                ffi_symbol: fallback_overload.ffi_symbol,
                            },
                        ));
                    }

                    if total_candidates > 1 {
                        return Err(MiddleError::NoOverload {
                            name: func_name.clone(),
                            span: (*span).into(),
                        });
                    }
                    // 1 overload total: unify falhou → tipos inconsistentes.
                    // Constrói TypeMismatch com os tipos dos args conflitantes.
                    // Para `duplicate T T => T` com args [Int, Float], expected
                    // = primeiro tipo (Int), found = segundo tipo (Float).
                    let (expected, found) = if arg_types.len() >= 2 {
                        (format!("{}", arg_types[0]), format!("{}", arg_types[1]))
                    } else {
                        (
                            format!("{}", arg_types.first().cloned().unwrap_or(Ty::Unit)),
                            format!("{}", Ty::Unit),
                        )
                    };
                    return Err(MiddleError::TypeMismatch {
                        expected,
                        found,
                        span: (*span).into(),
                    });
                }

                // Caminho genérico falhou. Tentar fallback refines antes de
                // retornar o erro: se algum arg é refined com delegação,
                // substituir pelo tipo base e retentar o dispatch.
                if let Some((_fallback_arg_types, fallback_overload)) =
                    try_refines_fallback(&func_name, &arg_types, ctx)
                {
                    let expanded_ret = expand_ret(&fallback_overload.ret, ctx);
                    let callee_ty = Ty::Function(
                        fallback_overload.params.clone(),
                        Box::new(expanded_ret.clone()),
                    );
                    let callee_typed = TypedExpr {
                        span: callee.span,
                        ty: callee_ty,
                        tail_pos: false,
                        escape: EscapeTarget::Local,
                        kind: TypedExprKind::Ident {
                            name: func_name.clone(),
                        },
                    };
                    return Ok((
                        expanded_ret,
                        TypedExprKind::Closure {
                            callee: Box::new(Spanned::new(callee_typed, callee.span)),
                            args: typed_args,
                            ffi_symbol: fallback_overload.ffi_symbol,
                        },
                    ));
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
                    expected: format!("{}", param_ty),
                    found: format!("{}", arg_ty),
                    span: expanded_args[i].span.into(),
                });
            }
        }

        let callee_typed = TypedExpr {
            span: callee.span,
            ty: Ty::Function(param_types.clone(), ret_ty.clone()),
            tail_pos: false,
            escape: EscapeTarget::Local,
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
            return infer_variant_construct(
                &VariantCall {
                    enum_name,
                    variant: &func_name,
                    module_path: None,
                    args,
                    span,
                },
                env,
                ctx,
                hint,
            );
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

/// Aplica defaults do EnumRegistry no tipo de retorno do dispatch.
/// Expande `Result::(Int)` → `Result::(Int, Text)` quando o enum tem
/// defaults registrados e o tipo retornado tem arity incompleto.
fn expand_ret(ty: &Ty, ctx: &InferCtx) -> Ty {
    ctx.enum_registry.expand_defaults(ty)
}

/// Verifica se algum argumento é `Ty::Action(..)` quando o overload alvo é
/// uma função pura (`is_action: false`). Actions são comportamento e não
/// podem ser passadas como argumento para funções puras. (PRD §3.7)
fn reject_action_arg_for_pure_fn(
    overload: &OverloadInfo,
    typed_args: &[Spanned<TypedExpr>],
    _span: &Span,
) -> Result<(), MiddleError> {
    if overload.is_action {
        return Ok(()); // Actions podem receber Actions como args.
    }
    for arg in typed_args {
        if let Ty::Action(..) = &arg.node.ty {
            return Err(MiddleError::TypeMismatch {
                expected: "argumento de função pura (não-Action)".into(),
                found: format!(
                    "Action não é permitida como argumento de função pura — \
                     Actions são comportamento, não informação. Tipo do argumento: `{}`",
                    arg.node.ty
                ),
                span: arg.span.into(),
            });
        }
    }
    Ok(())
}

/// Fallback `refines` no dispatch (D1 do PRD-refines).
///
/// Quando o dispatch normal falha (NoOverload), verifica se algum arg é tipo
/// refined com delegação `refines`. Se sim, substitui pelo tipo base e
/// retenta o dispatch. Se funcionar, retorna o overload encontrado.
///
/// Regra (D4): todos os args que SÃO refined devem ter `refines` para a
/// interface do método; args não-refined passam direto. A substituição só
/// ocorre se o `func_name` é método de alguma interface delegada por algum
/// arg refined.
fn try_refines_fallback(
    func_name: &str,
    arg_types: &[Ty],
    ctx: &InferCtx,
) -> Option<(Vec<Ty>, kata_core::OverloadInfo)> {
    // Para cada arg, se é refined (Ty::Struct com refines), coletar o tipo base.
    let mut fallback_arg_types = arg_types.to_vec();
    let mut any_substituted = false;

    for arg_ty in &mut fallback_arg_types {
        if let Ty::Struct(name) = arg_ty {
            // Segue a cadeia de alias_of se o tipo não tem refines direto.
            // Ex: Peso é alias de PositiveFloat que tem refines NUM.
            let mut current = name.clone();
            let entries = loop {
                let e = ctx.refines_registry.get(&current);
                if !e.is_empty() {
                    break e;
                }
                // Tenta seguir alias_of
                match ctx.struct_registry.get(&current) {
                    Some(info) if info.alias_of.is_some() => {
                        current = info.alias_of.clone().unwrap();
                    }
                    _ => break &[][..],
                }
            };
            if entries.is_empty() {
                continue;
            }
            // Verificar se func_name é método de alguma interface delegada
            // (incluindo supertraits). Só substitui se pelo menos uma interface
            // delegada tiver func_name como signature direta ou herdada — evita
            // fallback cego em funções fora da interface.
            let delegates_func = entries.iter().any(|entry| {
                ctx.interface_registry
                    .interface_has_method(&entry.interface_name, func_name)
            });
            if !delegates_func {
                continue;
            }
            let base_ty = &entries[0].base_ty;
            *arg_ty = base_ty.clone();
            any_substituted = true;
        }
    }

    if !any_substituted {
        return None;
    }

    // Retentar dispatch com args substituídos.
    let overload = ctx
        .table
        .resolve(func_name, &fallback_arg_types, ctx.interface_registry)
        .ok()?;

    Some((fallback_arg_types, overload))
}
