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

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr_hinted};
use super::helpers::InferResult;

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
    let name = match &callback.node {
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
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    if args.len() != 2 {
        return Err(MiddleError::ArityMismatch {
            expected: 2,
            found: args.len(),
            span: (*span).into(),
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
    let hint = Ty::Function(vec![elem_ty.clone()], Box::new(Ty::InferVar(999)));
    let callback_typed = match resolve_operator_callback(&args[0], 1, env, ctx, &hint) {
        Some(result) => result?,
        None => infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?,
    };

    // 4. Extrair ret_ty do callback (B).
    let cb_ret = match &callback_typed.ty {
        Ty::Function(_, ret) => (**ret).clone(),
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{:?}", callback_typed.ty),
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
    };

    Ok((map_ret, kind, Effect::Puro))
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
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    if args.len() != 2 {
        return Err(MiddleError::ArityMismatch {
            expected: 2,
            found: args.len(),
            span: (*span).into(),
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
    match &callback_typed.ty {
        Ty::Function(_, ret) if **ret == Ty::Sum("Boolean".into()) => {}
        Ty::Function(_, ret) => {
            return Err(MiddleError::TypeMismatch {
                expected: "Boolean".into(),
                found: format!("{:?}", ret),
                span: args[0].span.into(),
            });
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{:?}", callback_typed.ty),
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
    };

    Ok((filter_ret, kind, Effect::Puro))
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
) -> InferResult<(Ty, TypedExprKind, Effect)> {
    if args.len() != 3 {
        return Err(MiddleError::ArityMismatch {
            expected: 3,
            found: args.len(),
            span: (*span).into(),
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
    let hint = Ty::Function(
        vec![acc_ty.clone(), elem_ty.clone()],
        Box::new(acc_ty.clone()),
    );
    let callback_typed = match resolve_operator_callback(&args[0], 2, env, ctx, &hint) {
        Some(result) => result?,
        None => infer_expr_hinted(&args[0].node, &args[0].span, env, ctx, false, Some(&hint))?,
    };

    // 4. Verificar que o callback retorna o tipo do acumulador.
    match &callback_typed.ty {
        Ty::Function(_, ret) if **ret == acc_ty => {}
        Ty::Function(_, ret) => {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{acc_ty:?}"),
                found: format!("{:?}", ret),
                span: args[0].span.into(),
            });
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Function".into(),
                found: format!("{:?}", callback_typed.ty),
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
    };

    Ok((fold_ret, kind, Effect::Puro))
}
