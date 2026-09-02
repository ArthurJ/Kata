//! DispatchTable dispatch logic (Caminho 1).
//!
//! Wraps the DispatchTable dispatch path: hint-directed dispatch, concrete
//! dispatch (match_score), generic dispatch (unify), refines fallback, and
//! error reporting.

use kata_ast::{Expr, Span, Spanned};
use kata_core::StructKey;
use kata_core::dispatch::{OverloadInfo, Score, match_score};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty};
use kata_diagnostics::MiddleError;
use std::collections::HashMap;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::InferCtx;
use super::helpers::{InferResult, dispatch_to_middle_error};

/// Tenta dispatch via DispatchTable (call direto para FFI ou função Kata
/// nomeada). Retorna `Some(Ok(..))` se dispatch sucede, `Some(Err(..))` se
/// falha com erro definitivo, e `None` se `func_name` não está no
/// DispatchTable (caller cai para TypeEnv).
pub(crate) fn try_dispatch_table(
    func_name: &str,
    typed_args: &[Spanned<TypedExpr>],
    arg_types: &[Ty],
    callee: &Spanned<Expr>,
    span: &Span,
    ctx: &InferCtx,
    hint: Option<&Ty>,
) -> Option<InferResult<(Ty, TypedExprKind)>> {
    // Caminho 1: DispatchTable (call direto para FFI ou função Kata nomeada).
    if !ctx.table.has_function(func_name) {
        return None;
    }

    // Se todas as overloads de `func_name` são Actions, o usuário está
    // chamando uma action sem `!` (sintaxe de função pura). Actions são
    // comportamento — devem ser chamadas com `!` (ActionCall), não com
    // aplicação prefix (Closure). Rejeita com mensagem acionável em vez
    // de produzir um TypedExprKind::Closure que o codegen não sabe lowerar.
    if let Some(overloads) = ctx.table.get_overloads(func_name)
        && !overloads.is_empty()
        && overloads.iter().all(|oi| oi.is_action)
    {
        return Some(Err(MiddleError::TypeMismatch {
            expected: "função pura chamada sem `!`".to_string(),
            found: format!("{func_name} é uma action — use {func_name}!(...) para chamá-la"),
            span: (*span).into(),
        }));
    }

    // Ret-directed dispatch — se hint é Some(ty), filtra overloads
    // cujo retorno é compatível com ty (via fits_return) antes do scoring.
    if let Some(hint_ty) = hint {
        let overloads = ctx
            .table
            .get_overloads(func_name)
            .expect("has_function retornou true, overloads deve existir");
        let compatible: Vec<&OverloadInfo> = overloads
            .iter()
            .filter(|oi| oi.params.len() == arg_types.len())
            .filter(|oi| super::expr::fits_return(&oi.ret, hint_ty))
            .collect();

        if compatible.is_empty() {
            return Some(Err(MiddleError::TypeMismatch {
                expected: format!("{hint_ty:?} (hint de retorno)"),
                found: format!(
                    "nenhuma overload de {func_name} retorna tipo compatível com {hint_ty:?}"
                ),
                span: (*span).into(),
            }));
        }

        // Scoring por dominância entre as overloads compatíveis com o hint.
        // Replicar a lógica de resolve_inner mas só entre os compatíveis.
        let mut best_overload: Option<&OverloadInfo> = None;
        let mut best_score: Option<Score> = None;
        let mut top_count = 0;
        for oi in &compatible {
            let score = match_score(arg_types, &oi.params, ctx.interface_registry);
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
            // Nenhuma overload compatível com o hint tem args que casam na
            // ordem original. Se a função é comutativa e tem 2 args, tenta
            // swap: inverte os args e re-faz scoring ainda filtrado por hint.
            // Isto permite que @commutative desambigue via hint de retorno
            // quando ambas as direções de retorno existem (ex: Float×Rational
            // pode retornar Float ou Rational — o hint escolhe).
            if arg_types.len() == 2 && ctx.table.is_commutative(func_name) {
                let swapped = vec![arg_types[1].clone(), arg_types[0].clone()];
                for oi in &compatible {
                    let score = match_score(&swapped, &oi.params, ctx.interface_registry);
                    if !score.is_compatible(2) {
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
                if top_count == 1
                    && let Some(oi) = best_overload
                {
                    let overload = oi.clone();
                    if let Err(e) =
                        super::apply::reject_action_arg_for_pure_fn(&overload, typed_args, span)
                    {
                        return Some(Err(e));
                    }
                    let expanded_ret = super::apply::expand_ret(&overload.ret, ctx);
                    let callee_ty =
                        Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
                    let callee_typed = TypedExpr {
                        span: callee.span,
                        ty: callee_ty,
                        tail_pos: false,
                        escape: EscapeTarget::Local,
                        kind: TypedExprKind::Ident {
                            name: func_name.to_string(),
                        },
                    };
                    // Reordenar typed_args: o swap inverteu os tipos para casar
                    // com a overload, mas os args na TAST estão na ordem original.
                    let final_args = vec![typed_args[1].clone(), typed_args[0].clone()];
                    return Some(Ok((
                        expanded_ret,
                        TypedExprKind::Closure {
                            callee: Box::new(Spanned::new(callee_typed, callee.span)),
                            args: final_args,
                            ffi_symbol: overload.ffi_symbol,
                        },
                    )));
                }
                if top_count > 1 {
                    return Some(Err(MiddleError::AmbiguousDispatch {
                        name: func_name.to_string(),
                        span: (*span).into(),
                    }));
                }
            }
            // top_count ainda == 0: pode ser que uma overload genérica casasse
            // via unify (ex: `+ :: List::A List::A => List::A` com args
            // `List(Int)`), mas match_score não unifica type params em
            // Ty::List. Cair para o caminho genérico abaixo em vez de
            // retornar erro imediatamente.
        } else if top_count == 1
            && let Some(oi) = best_overload
        {
            let overload = oi.clone();
            if let Err(e) = super::apply::reject_action_arg_for_pure_fn(&overload, typed_args, span)
            {
                return Some(Err(e));
            }
            // Se o overload tem type_params e o arg é Instance de família
            // (ex: Instance("NonEmpty", "Int") casando com Family("NonEmpty")),
            // o match_score casa por nome da família mas não unifica o type
            // param. Tentar unify para resolver o tipo de retorno.
            let expanded_ret = if !overload.type_params.is_empty() {
                let mut subs: super::generics::Substitutions = HashMap::new();
                match super::generics::unify(
                    &overload.params,
                    arg_types,
                    &overload.type_params,
                    &mut subs,
                ) {
                    Ok(_) => {
                        let concrete_ret = super::generics::apply_subs(&overload.ret, &subs);
                        super::apply::expand_ret(&concrete_ret, ctx)
                    }
                    Err(_) => super::apply::expand_ret(&overload.ret, ctx),
                }
            } else {
                super::apply::expand_ret(&overload.ret, ctx)
            };
            let callee_ty = Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
            let callee_typed = TypedExpr {
                span: callee.span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: func_name.to_string(),
                },
            };
            return Some(Ok((
                expanded_ret,
                TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                    args: typed_args.to_vec(),
                    ffi_symbol: overload.ffi_symbol,
                },
            )));
        } else {
            // top_count > 1 — ambíguo
            return Some(Err(MiddleError::AmbiguousDispatch {
                name: func_name.to_string(),
                span: (*span).into(),
            }));
        }
        // top_count == 0: cair para o caminho genérico abaixo
    }

    // Caminho genérico — se nenhuma overload não-genérica casa,
    // procura overloads com type_params não-vazio e tenta unify.
    let generic_result = ctx
        .table
        .resolve_with_swap(func_name, arg_types, ctx.interface_registry);
    match generic_result {
        Ok(outcome) => {
            let overload = outcome.overload;
            if let Err(e) = super::apply::reject_action_arg_for_pure_fn(&overload, typed_args, span)
            {
                return Some(Err(e));
            }
            let expanded_ret = super::apply::expand_ret(&overload.ret, ctx);
            let callee_ty = Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
            let callee_typed = TypedExpr {
                span: callee.span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: func_name.to_string(),
                },
            };

            // Reordenar typed_args se o dispatch resolveu via commutative swap.
            // O swap inverte os tipos para casar com a overload, mas os args
            // na TAST estão na ordem original. O codegen precisa deles na ordem
            // esperada pela overload (que é a ordem swapada).
            let final_args = if outcome.swapped && typed_args.len() == 2 {
                vec![typed_args[1].clone(), typed_args[0].clone()]
            } else {
                typed_args.to_vec()
            };

            Some(Ok((
                expanded_ret,
                TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                    args: final_args,
                    ffi_symbol: overload.ffi_symbol,
                },
            )))
        }
        Err(_) => {
            // Tenta caminho genérico: procura overload com type_params não-vazio.
            let mut arity_matched = false;
            let mut unify_failed = false;
            let mut total_candidates = 0u32;
            if let Some(overloads) = ctx.table.get_overloads(func_name) {
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
                    match super::generics::unify(&oi.params, arg_types, &oi.type_params, &mut subs)
                    {
                        Ok(_) => {
                            // Aplica substitutions no tipo de retorno.
                            let concrete_ret = super::generics::apply_subs(&oi.ret, &subs);
                            let expanded_ret = super::apply::expand_ret(&concrete_ret, ctx);
                            if let Err(e) =
                                super::apply::reject_action_arg_for_pure_fn(oi, typed_args, span)
                            {
                                return Some(Err(e));
                            }
                            let callee_ty =
                                Ty::Function(oi.params.clone(), Box::new(expanded_ret.clone()));
                            let callee_typed = TypedExpr {
                                span: callee.span,
                                ty: callee_ty,
                                tail_pos: false,
                                escape: EscapeTarget::Local,
                                kind: TypedExprKind::Ident {
                                    name: func_name.to_string(),
                                },
                            };

                            return Some(Ok((
                                expanded_ret,
                                TypedExprKind::Closure {
                                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                                    args: typed_args.to_vec(),
                                    ffi_symbol: oi.ffi_symbol.clone(),
                                },
                            )));
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
                    try_refines_fallback(func_name, arg_types, ctx)
                {
                    let expanded_ret = super::apply::expand_ret(&fallback_overload.ret, ctx);
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
                            name: func_name.to_string(),
                        },
                    };
                    return Some(Ok((
                        expanded_ret,
                        TypedExprKind::Closure {
                            callee: Box::new(Spanned::new(callee_typed, callee.span)),
                            args: typed_args.to_vec(),
                            ffi_symbol: fallback_overload.ffi_symbol,
                        },
                    )));
                }

                if total_candidates > 1 {
                    return Some(Err(MiddleError::NoOverload {
                        name: func_name.to_string(),
                        span: (*span).into(),
                    }));
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
                return Some(Err(MiddleError::TypeMismatch {
                    expected,
                    found,
                    span: (*span).into(),
                }));
            }

            // Caminho genérico falhou. Tentar fallback refines antes de
            // retornar o erro: se algum arg é refined com delegação,
            // substituir pelo tipo base e retentar o dispatch.
            if let Some((_fallback_arg_types, fallback_overload)) =
                try_refines_fallback(func_name, arg_types, ctx)
            {
                let expanded_ret = super::apply::expand_ret(&fallback_overload.ret, ctx);
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
                        name: func_name.to_string(),
                    },
                };
                return Some(Ok((
                    expanded_ret,
                    TypedExprKind::Closure {
                        callee: Box::new(Spanned::new(callee_typed, callee.span)),
                        args: typed_args.to_vec(),
                        ffi_symbol: fallback_overload.ffi_symbol,
                    },
                )));
            }

            // Caminho genérico falhou — retorna o erro original do dispatch.
            Some(Err(dispatch_to_middle_error(
                ctx.table
                    .resolve(func_name, arg_types, ctx.interface_registry)
                    .unwrap_err(),
                *span,
            )))
        }
    }
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
pub(crate) fn try_refines_fallback(
    func_name: &str,
    arg_types: &[Ty],
    ctx: &InferCtx,
) -> Option<(Vec<Ty>, kata_core::OverloadInfo)> {
    // Overloads de aridade compatível — usadas para verificar se um arg
    // refined já é exact match com o param correspondente em alguma overload.
    // Se for, o arg não precisa ser substituído: o dispatch normal já deveria
    // tê-lo casado por exact match naquela posição. A substituição cega destrói
    // o match que já existia (ex: `foo :: Int PositiveInt` chamado com
    // `(PositiveInt, PositiveInt)` — o segundo arg já casa com PositiveInt,
    // só o primeiro precisa ser substituído para Int).
    let overloads = ctx.table.get_overloads(func_name);
    let arity_params: Vec<&[Ty]> = overloads
        .iter()
        .flat_map(|ovs| ovs.iter())
        .filter(|oi| oi.params.len() == arg_types.len())
        .map(|oi| oi.params.as_slice())
        .collect();

    // Para cada arg, se é refined (Ty::Struct com refines), coletar o tipo base.
    let mut fallback_arg_types = arg_types.to_vec();
    let mut any_substituted = false;

    for i in 0..fallback_arg_types.len() {
        let Ty::Struct(key) = &fallback_arg_types[i] else {
            continue;
        };
        let key = key.clone();
        // Se o arg já é exact match com o param na posição i de alguma
        // overload de aridade compatível, não substituir — o dispatch
        // normal já deveria ter casado este arg por exact match.
        let already_exact = arity_params.iter().any(|params| params[i] == arg_types[i]);
        if already_exact {
            continue;
        }

        // Segue a cadeia de alias_of se o tipo não tem refines direto.
        // Ex: Peso é alias de PositiveFloat que tem refines NUM.
        let mut current = key.name().to_string();
        let entries = loop {
            let e = ctx.refines_registry.get(&current);
            if !e.is_empty() {
                break e;
            }
            // Tenta seguir alias_of via TypeGraph (classificação).
            match ctx.type_graph.alias_target(&current) {
                Some(target) => {
                    current = target.to_string();
                }
                None => break &[][..],
            }
        };
        if entries.is_empty() {
            // Sem refines e sem instância polimórfica — não há fallback.
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
        // Para Instance de família polimórfica, o alias_of já é o tipo
        // concreto — usar diretamente em vez do base_ty do refines_registry
        // (que é arbitrário para famílias).
        if let StructKey::Instance(_, concrete) = &key {
            let instance_ty = match concrete.as_str() {
                "Int" => Ty::Prim(PrimTy::Int),
                "Float" => Ty::Prim(PrimTy::Float),
                "Rational" => Ty::Prim(PrimTy::Rational),
                "Text" => Ty::Prim(PrimTy::Text),
                other => Ty::Struct(StructKey::Plain(other.to_string())),
            };
            fallback_arg_types[i] = instance_ty;
            any_substituted = true;
            continue;
        }
        let base_ty = &entries[0].base_ty;
        fallback_arg_types[i] = base_ty.clone();
        any_substituted = true;
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

/// Direção B — wrapper que extrai params da overload do resultado do dispatch.
///
/// O resultado do dispatch é `(Ty, TypedExprKind)` onde `TypedExprKind::Closure`
/// contém o callee tipado com `ty: Ty::Function(params, ret)`. Esta função
/// extrai os params e delega para `extract_preconditions`.
pub(crate) fn extract_preconditions_from_result(
    result: &(Ty, TypedExprKind),
    typed_args: &[Spanned<TypedExpr>],
    refined_decls: &[kata_resolution::RefinedDeclInfo],
    env: &mut kata_core::ty::TypeEnv,
    ctx: &InferCtx,
) {
    let TypedExprKind::Closure { callee, .. } = &result.1 else {
        return;
    };
    let Ty::Function(params, _) = &callee.node.ty else {
        return;
    };
    extract_preconditions(params, typed_args, refined_decls, env, ctx);
}

/// Direção B (Nível 3 — aprendizado de predicados após chamada):
///
/// Após dispatch bem-sucedido, se a assinatura tem parâmetros refined e o
/// argumento correspondente é `Ident(name)` (variável do caller), adiciona
/// o predicado substituído como path condition no escopo do caller.
///
/// Para cada (param, arg) onde param é refined e arg é `Ident`:
/// 1. Consulta `refined_decls` para obter os predicados do tipo refined.
/// 2. Substitui Hole `_` pelo arg na AST do predicado.
/// 3. Tipa o predicado substituído via `infer_expr_hinted`.
/// 4. Adiciona como fact nas `path_conditions` do caller.
///
/// Se o arg não é `Ident` (ex: literal, expressão complexa), não adiciona
/// — não há variável para propagar.
pub(crate) fn extract_preconditions(
    params: &[Ty],
    typed_args: &[Spanned<TypedExpr>],
    refined_decls: &[kata_resolution::RefinedDeclInfo],
    env: &mut kata_core::ty::TypeEnv,
    ctx: &InferCtx,
) {
    for (param, typed_arg) in params.iter().zip(typed_args.iter()) {
        // Extrai o nome da variável do arg. Aceita Ident direto ou
        // TypeAscription { expr: Ident } (quando o arg foi ascriptado
        // explicitamente ou via Direção A).
        let var_name = match &typed_arg.node.kind {
            TypedExprKind::Ident { name } => name.clone(),
            TypedExprKind::TypeAscription { expr, .. } => {
                if let TypedExprKind::Ident { name } = &expr.node.kind {
                    name.clone()
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        // O tipo base do arg é o tipo antes do refining. Se o arg é
        // TypeAscription, o tipo base está dentro do expr.
        let arg_base_ty = match &typed_arg.node.kind {
            TypedExprKind::TypeAscription { expr, .. } => &expr.node.ty,
            _ => &typed_arg.node.ty,
        };
        for rd in refined_decls {
            if rd.base_ty != *arg_base_ty {
                continue;
            }
            let param_refined_name = match param {
                Ty::Struct(StructKey::Family(name)) | Ty::Struct(StructKey::Instance(name, _)) => {
                    name.clone()
                }
                _ => continue,
            };
            if rd.name != param_refined_name {
                continue;
            }
            let arg_expr = Spanned::new(
                Expr::Ident {
                    name: var_name.clone(),
                },
                typed_arg.span,
            );
            for pred in &rd.predicates {
                let substituted = super::const_eval::substitute_hole(pred, &arg_expr);
                let typed_pred = match super::expr::infer_expr_hinted(
                    &substituted.node,
                    &substituted.span,
                    env,
                    ctx,
                    false,
                    Some(&Ty::Sum("Boolean".to_string())),
                ) {
                    Ok(tp) => tp,
                    Err(_) => continue,
                };
                ctx.path_conditions
                    .borrow_mut()
                    .add_learned_fact(typed_pred);
            }
        }
    }
}

/// Direção A (Nível 3 — pré-condições inter-procedurais):
///
/// Quando o dispatch falha porque um argumento é tipo base (`Int`) e o
/// parâmetro correspondente na assinatura é tipo refined (`NonZero`), tenta
/// provar via Z3 que o argumento satisfaz o predicado do refined usando as
/// path conditions do caller. Se provado, aceita o argumento inserindo
/// ascription implícita e retenta o dispatch.
///
/// Retorna:
/// - `Some(Ok(...))` — predicado provado, dispatch aceito.
/// - `Some(Err(...))` — predicado refutado (Some(false) do Z3).
/// - `None` — Z3 não decidiu, ou não há mismatch base→refined, ou sem
///   path conditions. O caller deve usar o erro original do dispatch.
#[allow(clippy::too_many_arguments)] // 7 params: Z3 precondition probe precisa de caller+callee+ctx
pub(crate) fn try_refined_precondition(
    func_name: &str,
    args: &[Spanned<Expr>],
    typed_args: &[Spanned<TypedExpr>],
    callee: &Spanned<Expr>,
    span: &Span,
    env: &mut kata_core::ty::TypeEnv,
    ctx: &InferCtx,
) -> Option<InferResult<(Ty, TypedExprKind)>> {
    // Sem path conditions, o Z3 não pode provar nada.
    if ctx.path_conditions.borrow().is_empty() {
        return None;
    }

    // Procura overloads de func_name com aridade correta.
    let overloads = ctx.table.get_overloads(func_name)?;
    let candidates: Vec<&OverloadInfo> = overloads
        .iter()
        .filter(|oi| oi.params.len() == typed_args.len())
        .collect();
    if candidates.is_empty() {
        return None;
    }

    // Para cada overload candidata, identifica posições onde o param é
    // refined e o arg é tipo base. Se todas as posições refined são
    // provadas, aceita.
    for oi in &candidates {
        let mut refined_positions: Vec<usize> = Vec::new();
        for (i, param) in oi.params.iter().enumerate() {
            // O param é refined? (Family ou Instance de refined)
            let param_refined_name = match param {
                Ty::Struct(StructKey::Family(name)) | Ty::Struct(StructKey::Instance(name, _)) => {
                    Some(name.clone())
                }
                _ => None,
            };
            let Some(rname) = param_refined_name else {
                continue;
            };
            // O arg é tipo base (não-refined)?
            let arg_is_base = !matches!(&typed_args[i].node.ty, Ty::Struct(_));
            if !arg_is_base {
                continue;
            }
            // O arg é compatível com o base_ty do refined?
            let base_match = ctx
                .refined_decls
                .iter()
                .filter(|rd| rd.name == rname)
                .any(|rd| rd.base_ty == typed_args[i].node.ty);
            if !base_match {
                continue;
            }
            refined_positions.push(i);
        }
        if refined_positions.is_empty() {
            continue;
        }

        // Para cada posição refined, prova o predicado via Z3.
        let mut all_proven = true;
        let mut any_refuted = false;
        for &i in &refined_positions {
            let rname = match &oi.params[i] {
                Ty::Struct(StructKey::Family(name)) | Ty::Struct(StructKey::Instance(name, _)) => {
                    name.clone()
                }
                _ => unreachable!(),
            };
            // Pega os predicados do refined_decls cujo base_ty casa com o arg.
            let refined_decls: Vec<_> = ctx
                .refined_decls
                .iter()
                .filter(|rd| rd.name == rname && rd.base_ty == typed_args[i].node.ty)
                .collect();
            if refined_decls.is_empty() {
                all_proven = false;
                break;
            }
            // Para cada predicado, substitui Hole pelo arg e prova via Z3.
            for rd in &refined_decls {
                for pred in &rd.predicates {
                    let substituted = super::const_eval::substitute_hole(pred, &args[i]);
                    let typed_pred = match super::expr::infer_expr_hinted(
                        &substituted.node,
                        &substituted.span,
                        env,
                        ctx,
                        false,
                        Some(&Ty::Sum("Boolean".to_string())),
                    ) {
                        Ok(tp) => tp,
                        Err(_) => {
                            all_proven = false;
                            break;
                        }
                    };
                    match super::path_conditions::try_prove_with_path_conditions(
                        &typed_pred,
                        &ctx.path_conditions.borrow(),
                        ctx.inline_fns,
                    ) {
                        Some(true) => {}
                        Some(false) => {
                            any_refuted = true;
                            all_proven = false;
                            break;
                        }
                        None => {
                            all_proven = false;
                            break;
                        }
                    }
                }
                if !all_proven {
                    break;
                }
            }
            if !all_proven {
                break;
            }
        }

        if any_refuted {
            return Some(Err(MiddleError::TypeMismatch {
                expected: "argumento satisfaz predicado do tipo refined (path conditions refutam)"
                    .to_string(),
                found: "path conditions implicem negação do predicado".to_string(),
                span: (*span).into(),
            }));
        }
        if !all_proven {
            continue;
        }

        // Todos os predicados provados. Constrói args com ascription implícita
        // e retenta o dispatch.
        let mut new_arg_types: Vec<Ty> = typed_args.iter().map(|ta| ta.node.ty.clone()).collect();
        let mut new_typed_args = typed_args.to_vec();
        for &i in &refined_positions {
            let refined_ty = oi.params[i].clone();
            new_arg_types[i] = refined_ty.clone();
            let ascripted = TypedExpr {
                span: typed_args[i].span,
                ty: refined_ty,
                tail_pos: typed_args[i].node.tail_pos,
                escape: typed_args[i].node.escape,
                kind: TypedExprKind::TypeAscription {
                    expr: Box::new(typed_args[i].clone()),
                    target_ty: oi.params[i].clone(),
                    pending_predicates: Vec::new(),
                },
            };
            new_typed_args[i] = Spanned::new(ascripted, typed_args[i].span);
        }

        // Retenta o dispatch com os novos arg types.
        let retry = ctx
            .table
            .resolve_with_swap(func_name, &new_arg_types, ctx.interface_registry);
        if let Ok(outcome) = retry {
            let overload = outcome.overload;
            let expanded_ret = super::apply::expand_ret(&overload.ret, ctx);
            let callee_ty = Ty::Function(overload.params.clone(), Box::new(expanded_ret.clone()));
            let callee_typed = TypedExpr {
                span: callee.span,
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: TypedExprKind::Ident {
                    name: func_name.to_string(),
                },
            };
            let final_args = if outcome.swapped && new_typed_args.len() == 2 {
                vec![new_typed_args[1].clone(), new_typed_args[0].clone()]
            } else {
                new_typed_args
            };
            return Some(Ok((
                expanded_ret,
                TypedExprKind::Closure {
                    callee: Box::new(Spanned::new(callee_typed, callee.span)),
                    args: final_args,
                    ffi_symbol: overload.ffi_symbol,
                },
            )));
        }
    }

    None
}
