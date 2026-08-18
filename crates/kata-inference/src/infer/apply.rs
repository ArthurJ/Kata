//! Aplicação prefixa — resolução de callee.
//!
//! Três caminhos de callee:
//! 1. Lambda inline (DoD 31): `(lambda x: ...) 42` — args fornecem tipos
//! 2. DispatchTable: call direto para FFI ou função Kata nomeada
//! 3. TypeEnv: call_indirect para lambda como valor

use kata_ast::{Expr, Span, Spanned};
use kata_core::dispatch::OverloadInfo;
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use super::expr::suggest_similar;
use crate::typed::{TypedExpr, TypedExprKind};

use super::apply_dispatch::try_dispatch_table;
use super::apply_lambda::{infer_apply_lambda, infer_apply_lambda_with_hint};
use super::apply_len_tuple::try_len_tuple;
use super::apply_repr::try_repr;
use super::apply_show_tuple::try_show_tuple;
use super::collections_hof::{infer_filter, infer_fold, infer_map};
use super::expr::{InferCtx, infer_expr};
use super::helpers::{InferResult, peel_grouping_expr};
use super::iface_dispatch::try_iface_method_dispatch;
use super::variant_construct::{VariantCall, infer_variant_construct};
use kata_resolution::resolve_type_expr;

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
                                suggestion: None,
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
                            suggestion: None,
                        });
                    }
                } else {
                    return Err(MiddleError::UnboundName {
                        name: "<non-ident callee>".into(),
                        span: callee.span.into(),
                        suggestion: None,
                    });
                }
            } else {
                return Err(MiddleError::UnboundName {
                    name: "<non-ident callee>".into(),
                    span: callee.span.into(),
                    suggestion: None,
                });
            }
        }
        _ => {
            return Err(MiddleError::UnboundName {
                name: "<non-ident callee>".into(),
                span: callee.span.into(),
                suggestion: None,
            });
        }
    };

    // `format` sem `!` foi removido — agora é `format!` (action call com `!`).
    // Intercepado em action_call.rs como builtin, despacha para infer_format_builtin.

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
    if let Some(result) = try_len_tuple(&func_name, args, span, env, ctx) {
        return result;
    }

    // `repr <expr>` — intercepta antes do dispatch normal.
    // repr é o protocolo round-tripable: cita Text, delega para show nos demais.
    if let Some(result) = try_repr(&func_name, args, span, env, ctx) {
        return result;
    }

    // `show tuple` — Tuple não registra overload de `show` no DispatchTable.
    // Intercepta antes do dispatch normal e gera Closure genérica (ffi_symbol:
    // None). O monomorphizador resolve via `tuple_show.rs::rewrite_show_tuple_call`.
    if let Some(result) = try_show_tuple(&func_name, args, span, env, ctx) {
        return result;
    }

    // Infere tipos dos argumentos recursivamente (tail_pos = false para args).
    let mut typed_args: Vec<Spanned<TypedExpr>> = Vec::with_capacity(args.len());
    let mut arg_types: Vec<Ty> = Vec::with_capacity(args.len());

    for arg in args {
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
    if let Some(result) =
        try_dispatch_table(&func_name, &typed_args, &arg_types, callee, span, ctx, hint)
    {
        return result;
    }

    // Caminho 2c: OverloadSet no TypeEnv (PRD OverloadSet — aplicação parcial
    // ambígua). O binding tem tipo OverloadSet — múltiplas overloads casam
    // com o partial dispatch. Seleciona a overload correta pelo tipo concreto
    // dos args e re-infere o lambda com os tipos selecionados.
    if let Some(Ty::OverloadSet {
        name: ov_name,
        overloads,
    }) = env.lookup(&func_name).cloned()
        && let Some(deferred) = ctx.deferred_lambdas.borrow().get(&func_name).cloned()
    {
        // Seleciona a overload cujos param types casam com os arg types.
        let mut matched: Vec<(Vec<Ty>, Ty)> = Vec::new();
        for (ov_params, ov_ret) in &overloads {
            if ov_params.len() != arg_types.len() {
                continue;
            }
            let score =
                kata_core::dispatch::match_score(&arg_types, ov_params, ctx.interface_registry);
            if score.is_compatible(arg_types.len()) {
                matched.push((ov_params.clone(), ov_ret.clone()));
            }
        }

        if matched.len() == 1 {
            // Única overload casou — re-infere o lambda com os tipos concretos.
            // infer_apply_lambda infere os arg types e os usa como param types.
            return infer_apply_lambda(
                &deferred.patterns,
                &deferred.body,
                &deferred.guards,
                &deferred.with_bindings,
                args,
                span,
                env,
                ctx,
            );
        }

        if matched.is_empty() {
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "uma overload de `{ov_name}` compatível com [{}]",
                    arg_types
                        .iter()
                        .map(|t| t.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                found: "nenhuma overload compatível".into(),
                span: (*span).into(),
            });
        }

        // Múltiplas overloads casam mesmo no call site — ambíguo.
        return Err(MiddleError::AmbiguousDispatch {
            name: ov_name,
            span: (*span).into(),
        });
    }

    // Caminho 2: TypeEnv (call_indirect para lambda como valor).
    // Sub-caminho 2a: lambda deferido (use-site inference). Se o binding
    // tem InferVar nos param_types e está na side table de deferred lambdas,
    // re-inere o lambda com os arg types reais via infer_apply_lambda.
    if let Some(Ty::Function(param_types, ret_ty)) = env.lookup(&func_name).cloned() {
        // Verifica se é um lambda deferido (param_types contêm InferVar).
        let has_infer_vars = param_types.iter().any(|t| matches!(t, Ty::InferVar(_)));
        if has_infer_vars {
            if let Some(deferred) = ctx.deferred_lambdas.borrow().get(&func_name).cloned() {
                // Re-inere o lambda com os arg types reais (síntese bottom-up).
                // infer_apply_lambda já faz arity check, arg inference, e
                // build_lambda_apply com os param types corretos.
                return infer_apply_lambda(
                    &deferred.patterns,
                    &deferred.body,
                    &deferred.guards,
                    &deferred.with_bindings,
                    args,
                    span,
                    env,
                    ctx,
                );
            }
            // Lambda deferido sem entry na side table — não há como resolver.
            return Err(MiddleError::LambdaInferenceFail {
                span: (*span).into(),
                detail: Some(format!(
                    "lambda `{func_name}` tem tipos não-resolvidos e não foi aplicada com args suficientes para inferi-los"
                )),
            });
        }

        // Sub-caminho 2b: lambda com tipos conhecidos (caminho normal).
        // Verifica aridade.
        if arg_types.len() != param_types.len() {
            return Err(MiddleError::ArityMismatch {
                expected: param_types.len(),
                found: arg_types.len(),
                span: (*span).into(),
                hint: None,
            });
        }
        // Verifica tipos dos argumentos.
        for (i, (arg_ty, param_ty)) in arg_types.iter().zip(param_types.iter()).enumerate() {
            if arg_ty != param_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", param_ty),
                    found: format!("{}", arg_ty),
                    span: args[i].span.into(),
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
            suggestion: None,
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
        name: func_name.clone(),
        span: callee.span.into(),
        suggestion: suggest_similar(&func_name, ctx.table.all_names()),
    })
}

/// Aplica defaults do EnumRegistry no tipo de retorno do dispatch.
/// Expande `Result::(Int)` → `Result::(Int, Text)` quando o enum tem
/// defaults registrados e o tipo retornado tem arity incompleto.
pub(crate) fn expand_ret(ty: &Ty, ctx: &InferCtx) -> Ty {
    ctx.enum_registry.expand_defaults(ty)
}

/// Verifica se algum argumento é `Ty::Action(..)` quando o overload alvo é
/// uma função pura (`is_action: false`). Actions são comportamento e não
/// podem ser passadas como argumento para funções puras. (PRD §3.7)
pub(crate) fn reject_action_arg_for_pure_fn(
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
