//! Inferência de map/filter/fold — interceptação no infer_apply.
//!
//! Estas funções não passam pelo DispatchTable. O typeck descobre
//! o tipo concreto do container (List/Array/Range), extrai elem_ty,
//! infere o callback, e produz nó TAST dedicado.
//!
//! **Assinatura (Arthur, 2026-07-17):**
//! - `map :: (A -> B) List::A => List::B`
//! - `filter :: (A -> Boolean) List::A => List::A`
//! - `fold :: (A B -> A) A List::B => A`
//!
//! map/filter retornam sempre List. Se o input era Array, o codegen
//! converte List→Array no final. Stream fusion é fase separada.

use kata_ast::{Expr, Pattern, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr_hinted};
use super::helpers::{InferResult, peel_grouping_expr};
use super::lambda::infer_lambda;

/// Resolve um callback que é `Expr::Ident` referenciando uma função do
/// DispatchTable (ex: `+`, `*`, `<` usado como callback standalone).
///
/// Constrói um lambda sintético `lambda __hof_a __hof_b ...: f __hof_a __hof_b ...`
/// que o pipeline normal sabe inferir e gerar. O operador só é resolvido
/// via DispatchTable quando aparece como callee em `Apply` — como callback
/// standalone, `infer_expr_hinted` não o encontra.
///
/// `num_params` é o número de parâmetros esperados pelo hint (1 para
/// map/filter, 2 para fold).
fn resolve_operator_callback(
    callback: &Spanned<Expr>,
    num_params: usize,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    hint: &Ty,
) -> Option<InferResult<TypedExpr>> {
    // Peel Grouping — `(+)` produz Grouping(Ident("+")), não Ident direto.
    // Grouping de uma única função = tratar a função como valor.
    let core = peel_grouping_expr(&callback.node);
    let name = match core {
        Expr::Ident { name } => name.clone(),
        _ => return None,
    };
    if !ctx.table.has_function(&name) {
        return None;
    }

    // Constrói lambda sintético: lambda __hof_0 __hof_1 ...: name __hof_0 __hof_1 ...
    let synth_span = Span::synthetic();
    let patterns: Vec<Spanned<Pattern>> = (0..num_params)
        .map(|i| Spanned::new(Pattern::Ident(format!("__hof_{i}")), synth_span))
        .collect();
    let args: Vec<Spanned<Expr>> = (0..num_params)
        .map(|i| {
            Spanned::new(
                Expr::Ident {
                    name: format!("__hof_{i}"),
                },
                synth_span,
            )
        })
        .collect();
    let body = Spanned::new(
        Expr::Apply {
            callee: Box::new(Spanned::new(Expr::Ident { name: name.clone() }, synth_span)),
            args,
        },
        synth_span,
    );
    let synth_lambda = Expr::Lambda {
        patterns,
        body: Box::new(body),
        guards: Vec::new(),
        with_bindings: Vec::new(),
    };

    Some(infer_expr_hinted(
        &synth_lambda,
        &callback.span,
        env,
        ctx,
        false,
        Some(hint),
    ))
}

/// Extrai o tipo do elemento de uma coleção.
/// `List(A)` → A, `Array(A)` → A, `Range(A)` → A.
fn extract_elem_ty(coll_ty: &Ty) -> Option<Ty> {
    match coll_ty {
        Ty::List(elem) | Ty::Array(elem) | Ty::Range(elem) => Some((**elem).clone()),
        _ => None,
    }
}

// ── Map ──────────────────────────────────────────────────────

/// `map f coll` — aplica f a cada elemento, retorna List(B).
///
/// args[0] = callback (A -> B)
/// args[1] = collection (List/Array/Range de A)
pub(crate) fn infer_map(
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    if args.len() != 2 {
        return Err(MiddleError::ArityMismatch {
            expected: 2,
            found: args.len(),
            span: (*span).into(),
            hint: None,
        });
    }

    // 1. Inferir a coleção (args[1]) — descobre coll_ty.
    let coll_typed = infer_expr_hinted(&args[1].node, &args[1].span, env, ctx, false, None)?;
    let coll_ty = coll_typed.ty.clone();

    // 2. Extrair elem_ty de coll_ty.
    let elem_ty = extract_elem_ty(&coll_ty).ok_or_else(|| MiddleError::TypeMismatch {
        expected: "List | Array | Range".into(),
        found: format!("{coll_ty:?}"),
        span: args[1].span.into(),
    })?;

    // 3. Inferir o callback (args[0]) com hint = Function([elem_ty], InferVar).
    //    O hint faz o lambda inferir seus params a partir do tipo do elemento.
    //    Se o callback é um operador standalone (Expr::Ident no DispatchTable),
    //    desugar para lambda sintético antes de inferir.
    //
    //    Se o callback é Ident("f") com tipo OverloadSet no TypeEnv e
    //    tem lambda deferido na side table, re-infere o lambda com hint concreto
    //    Function([elem_ty], InferVar). O hint desambigua as overloads e produz
    //    Function([elem_ty], ret_ty) em vez de OverloadSet. O codegen então
    //    recebe um Lambda normal que sabe resolver.
    let hint = Ty::Function(vec![elem_ty.clone()], Box::new(Ty::InferVar(999)));

    // Tentar resolver OverloadSet via lambda deferido antes de infer_expr_hinted.
    let callback_typed = match resolve_operator_callback(&args[0], 1, env, ctx, &hint) {
        Some(result) => result?,
        None => {
            // Verificar se é Ident com OverloadSet e lambda deferido.
            let core = peel_grouping_expr(&args[0].node);
            if let Expr::Ident { name } = core {
                if env
                    .lookup(name)
                    .is_some_and(|ty| matches!(ty, Ty::OverloadSet { .. }))
                {
                    if let Some(deferred) = ctx.deferred_lambdas.borrow().get(name).cloned() {
                        let (cb_ty, cb_kind) = infer_lambda(
                            &deferred.patterns,
                            &deferred.body,
                            &deferred.guards,
                            &deferred.with_bindings,
                            &args[0].span,
                            env,
                            ctx,
                            Some(&hint),
                        )?;
                        // infer_lambda com hint concreto produz Function, não OverloadSet.
                        // Se ainda produziu OverloadSet, o hint não desambiguou —
                        // cai para o caminho de seleção no passo 4.
                        let cb_typed = TypedExpr {
                            span: args[0].span,
                            ty: cb_ty,
                            tail_pos: false,
                            escape: kata_core::escape::EscapeTarget::Caller,
                            kind: cb_kind,
                        };
                        // Se infer_lambda com hint ainda retornou OverloadSet,
                        // usar o caminho 4 (seleção manual).
                        if !matches!(cb_typed.ty, Ty::OverloadSet { .. }) {
                            // Hint resolveu — callback é Function normal.
                            let cb_ret = match &cb_typed.ty {
                                Ty::Function(_, ret) => (**ret).clone(),
                                _ => unreachable!("infer_lambda com hint deve produzir Function"),
                            };
                            let map_ret = Ty::List(Box::new(cb_ret.clone()));
                            let kind = TypedExprKind::Map {
                                callback: Box::new(Spanned::new(cb_typed, args[0].span)),
                                collection: Box::new(Spanned::new(coll_typed, args[1].span)),
                                coll_ty,
                                elem_ty,
                                ret_ty: map_ret.clone(),
                                limit: None,
                            };
                            return Ok((map_ret, kind));
                        }
                        // OverloadSet ainda — cai para passo 4 com cb_typed
                        cb_typed
                    } else {
                        infer_expr_hinted(
                            &args[0].node,
                            &args[0].span,
                            env,
                            ctx,
                            false,
                            Some(&hint),
                        )?
                    }
                } else {
                    infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?
                }
            } else {
                infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?
            }
        }
    };

    // 4. Extrair ret_ty do callback (B).
    //    Se o callback é OverloadSet (lambda deferido com partial dispatch
    //    ambíguo), seleciona a overload cujo primeiro param casa com elem_ty.
    let cb_ret = match &callback_typed.ty {
        Ty::Function(_, ret) => (**ret).clone(),
        Ty::OverloadSet { name, overloads } => {
            // Selecionar overload por elem_ty.
            let matched: Vec<&(Vec<Ty>, Ty)> = overloads
                .iter()
                .filter(|(params, _)| {
                    params.len() == 1
                        && kata_core::dispatch::match_score(
                            std::slice::from_ref(&elem_ty),
                            params,
                            ctx.interface_registry,
                        )
                        .is_compatible(1)
                })
                .collect();
            match matched.len() {
                1 => matched[0].1.clone(),
                0 => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("uma overload de `{name}` compatível com [{elem_ty}]"),
                        found: "nenhuma overload compatível".into(),
                        span: args[0].span.into(),
                    });
                }
                _ => {
                    return Err(MiddleError::AmbiguousDispatch {
                        name: name.clone(),
                        span: args[0].span.into(),
                    });
                }
            }
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{}", callback_typed.ty),
                span: args[0].span.into(),
            });
        }
    };

    // 5. ret_ty do Map = List(B) — sempre List.
    let map_ret = Ty::List(Box::new(cb_ret.clone()));

    let kind = TypedExprKind::Map {
        callback: Box::new(Spanned::new(callback_typed, args[0].span)),
        collection: Box::new(Spanned::new(coll_typed, args[1].span)),
        coll_ty,
        elem_ty,
        ret_ty: map_ret.clone(),
        limit: None,
    };

    Ok((map_ret, kind))
}

// ── Filter ───────────────────────────────────────────────────

/// `filter f coll` — filtra elementos por predicado, retorna List(A).
///
/// args[0] = callback (A -> Boolean)
/// args[1] = collection (List/Array/Range de A)
pub(crate) fn infer_filter(
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    if args.len() != 2 {
        return Err(MiddleError::ArityMismatch {
            expected: 2,
            found: args.len(),
            span: (*span).into(),
            hint: None,
        });
    }

    // 1. Inferir a coleção.
    let coll_typed = infer_expr_hinted(&args[1].node, &args[1].span, env, ctx, false, None)?;
    let coll_ty = coll_typed.ty.clone();

    // 2. Extrair elem_ty.
    let elem_ty = extract_elem_ty(&coll_ty).ok_or_else(|| MiddleError::TypeMismatch {
        expected: "List | Array | Range".into(),
        found: format!("{coll_ty:?}"),
        span: args[1].span.into(),
    })?;

    // 3. Inferir o callback com hint = Function([elem_ty], Boolean).
    //    Boolean é Sum("Boolean") no Kata5.
    //    Se o callback é um operador standalone, desugar para lambda.
    let hint = Ty::Function(vec![elem_ty.clone()], Box::new(Ty::Sum("Boolean".into())));
    let callback_typed = match resolve_operator_callback(&args[0], 1, env, ctx, &hint) {
        Some(result) => result?,
        None => infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?,
    };

    // 4. Verificar que o callback retorna Boolean.
    //    Se o callback é OverloadSet (lambda deferido), seleciona a overload
    //    cujo param casa com elem_ty e retorna Boolean.
    match &callback_typed.ty {
        Ty::Function(_, ret) if **ret == Ty::Sum("Boolean".into()) => {}
        Ty::Function(_, ret) => {
            return Err(MiddleError::TypeMismatch {
                expected: "Boolean".into(),
                found: format!("{}", ret),
                span: args[0].span.into(),
            });
        }
        Ty::OverloadSet { name, overloads } => {
            // Selecionar overload por elem_ty com retorno Boolean.
            let boolean = Ty::Sum("Boolean".into());
            let matched: Vec<&(Vec<Ty>, Ty)> = overloads
                .iter()
                .filter(|(params, ret)| {
                    params.len() == 1
                        && *ret == boolean
                        && kata_core::dispatch::match_score(
                            std::slice::from_ref(&elem_ty),
                            params,
                            ctx.interface_registry,
                        )
                        .is_compatible(1)
                })
                .collect();
            match matched.len() {
                1 => {}
                0 => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!(
                            "uma overload de `{name}` compatível com [{elem_ty}] => Boolean"
                        ),
                        found: "nenhuma overload compatível".into(),
                        span: args[0].span.into(),
                    });
                }
                _ => {
                    return Err(MiddleError::AmbiguousDispatch {
                        name: name.clone(),
                        span: args[0].span.into(),
                    });
                }
            }
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{}", callback_typed.ty),
                span: args[0].span.into(),
            });
        }
    }

    // 5. ret_ty do Filter = List(A) — sempre List.
    let filter_ret = Ty::List(Box::new(elem_ty.clone()));

    let kind = TypedExprKind::Filter {
        callback: Box::new(Spanned::new(callback_typed, args[0].span)),
        collection: Box::new(Spanned::new(coll_typed, args[1].span)),
        coll_ty,
        elem_ty: elem_ty.clone(),
        ret_ty: filter_ret.clone(),
        limit: None,
    };

    Ok((filter_ret, kind))
}

// ── Fold ──────────────────────────────────────────────────────

/// `fold f init coll` — reduz coleção com função e acumulador.
///
/// args[0] = callback (A B -> A) — A = tipo do acumulador, B = tipo do elemento
/// args[1] = initial (A) — valor inicial do acumulador
/// args[2] = collection (List/Array/Range de B)
pub(crate) fn infer_fold(
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    if args.len() != 3 {
        return Err(MiddleError::ArityMismatch {
            expected: 3,
            found: args.len(),
            span: (*span).into(),
            hint: None,
        });
    }

    // 1. Inferir a coleção (args[2]) — descobre coll_ty e elem_ty.
    let coll_typed = infer_expr_hinted(&args[2].node, &args[2].span, env, ctx, false, None)?;
    let coll_ty = coll_typed.ty.clone();
    let elem_ty = extract_elem_ty(&coll_ty).ok_or_else(|| MiddleError::TypeMismatch {
        expected: "List | Array | Range".into(),
        found: format!("{coll_ty:?}"),
        span: args[2].span.into(),
    })?;

    // 2. Inferir initial (args[1]) — descobre acc_ty (tipo do acumulador).
    let init_typed = infer_expr_hinted(&args[1].node, &args[1].span, env, ctx, false, None)?;
    let acc_ty = init_typed.ty.clone();

    // 3. Inferir o callback (args[0]) com hint = Function([acc_ty, elem_ty], acc_ty).
    //    O callback recebe (acumulador, elemento) e retorna o novo acumulador.
    //    Se o callback é um operador standalone, desugar para lambda.
    //
    //    Se o callback é Ident com OverloadSet e lambda deferido,
    //    re-infere o lambda com hint concreto. O hint desambigua as overloads.
    let hint = Ty::Function(
        vec![acc_ty.clone(), elem_ty.clone()],
        Box::new(acc_ty.clone()),
    );
    let callback_typed = match resolve_operator_callback(&args[0], 2, env, ctx, &hint) {
        Some(result) => result?,
        None => {
            let core = peel_grouping_expr(&args[0].node);
            if let Expr::Ident { name } = core {
                if env
                    .lookup(name)
                    .is_some_and(|ty| matches!(ty, Ty::OverloadSet { .. }))
                {
                    if let Some(deferred) = ctx.deferred_lambdas.borrow().get(name).cloned() {
                        let (cb_ty, cb_kind) = infer_lambda(
                            &deferred.patterns,
                            &deferred.body,
                            &deferred.guards,
                            &deferred.with_bindings,
                            &args[0].span,
                            env,
                            ctx,
                            Some(&hint),
                        )?;
                        let cb_typed = TypedExpr {
                            span: args[0].span,
                            ty: cb_ty,
                            tail_pos: false,
                            escape: kata_core::escape::EscapeTarget::Caller,
                            kind: cb_kind,
                        };
                        if !matches!(cb_typed.ty, Ty::OverloadSet { .. }) {
                            // Hint resolveu — callback é Function normal.
                            let fold_ret = acc_ty.clone();
                            let kind = TypedExprKind::Fold {
                                callback: Box::new(Spanned::new(cb_typed, args[0].span)),
                                initial: Box::new(Spanned::new(init_typed, args[1].span)),
                                collection: Box::new(Spanned::new(coll_typed, args[2].span)),
                                coll_ty,
                                elem_ty,
                                ret_ty: fold_ret.clone(),
                                limit: None,
                            };
                            return Ok((fold_ret, kind));
                        }
                        cb_typed
                    } else {
                        infer_expr_hinted(
                            &args[0].node,
                            &args[0].span,
                            env,
                            ctx,
                            false,
                            Some(&hint),
                        )?
                    }
                } else {
                    infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?
                }
            } else {
                infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?
            }
        }
    };

    // 4. Verificar que o callback retorna o tipo do acumulador.
    //    Se o callback é OverloadSet (lambda deferido), seleciona a overload
    //    cujos params casam com (acc_ty, elem_ty).
    match &callback_typed.ty {
        Ty::Function(_, ret) if **ret == acc_ty => {}
        Ty::Function(_, ret) => {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{acc_ty:?}"),
                found: format!("{}", ret),
                span: args[0].span.into(),
            });
        }
        Ty::OverloadSet { name, overloads } => {
            // Selecionar overload por (acc_ty, elem_ty).
            let matched: Vec<&(Vec<Ty>, Ty)> = overloads
                .iter()
                .filter(|(params, ret)| {
                    params.len() == 2
                        && *ret == acc_ty
                        && kata_core::dispatch::match_score(
                            &[acc_ty.clone(), elem_ty.clone()],
                            params,
                            ctx.interface_registry,
                        )
                        .is_compatible(2)
                })
                .collect();
            match matched.len() {
                1 => {}
                0 => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!(
                            "uma overload de `{name}` compatível com [{acc_ty}, {elem_ty}] => {acc_ty}"
                        ),
                        found: "nenhuma overload compatível".into(),
                        span: args[0].span.into(),
                    });
                }
                _ => {
                    return Err(MiddleError::AmbiguousDispatch {
                        name: name.clone(),
                        span: args[0].span.into(),
                    });
                }
            }
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{}", callback_typed.ty),
                span: args[0].span.into(),
            });
        }
    }

    // 5. ret_ty do Fold = acc_ty (tipo do acumulador).
    let fold_ret = acc_ty.clone();

    let kind = TypedExprKind::Fold {
        callback: Box::new(Spanned::new(callback_typed, args[0].span)),
        initial: Box::new(Spanned::new(init_typed, args[1].span)),
        collection: Box::new(Spanned::new(coll_typed, args[2].span)),
        coll_ty,
        elem_ty,
        ret_ty: fold_ret.clone(),
        limit: None,
    };

    Ok((fold_ret, kind))
}
