//! Desugar de operadores especiais — `?`, `|`, `assert!`.
//!
//! Cada função constrói um match sintético e delega para `infer_match`.
//! São chamadas pelos arms correspondentes em `infer_expr_hinted`.
//!
//! (Nome `sugar` para evitar colisão com `crate::desugar` já importado em mod.rs.)

use kata_ast::{Expr, MatchArm, Pattern, Span, Spanned, TypeExpr};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::TypedExpr;

use super::_match::infer_match;
use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Desugar `expr ?` para `match expr { Ok(v) => v, Err(e) => return Err(e) }`.
///
/// `?` é fail-fast: se o scrutinee é `Result<T, E>`, desempacota `Ok(v)` ou
/// aborta com `return Err(e)`. Se é `Optional<T>`, desempacota `Some(v)` ou
/// aborta com `return None`.
///
/// O `?` só é válido dentro de Action (precisa `ctx.ret_ty`).
/// O tipo de retorno da Action deve ser compatível com a variante de erro.
pub(crate) fn infer_question(
    inner: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    _tail_pos: bool,
) -> InferResult<TypedExpr> {
    // `?` só funciona dentro de Action.
    let _ret_ty = ctx.ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
        expected: "return dentro de Action".into(),
        found: "? fora de Action".into(),
        span: (*span).into(),
    })?;

    // Infere o scrutinee.
    let typed_scrutinee = infer_expr(&inner.node, &inner.span, env, ctx, false)?;

    // Determina o enum e constrói os braços do match sintético.
    // Aplica defaults do EnumRegistry: `Result::(T)` → `Result::(T, Text)`.
    let scrutinee_ty = if let Ty::Generic(name, args) = &typed_scrutinee.ty {
        if let Some(expanded) = ctx.enum_registry.apply_defaults(name, args) {
            Ty::Generic(name.clone(), expanded)
        } else {
            typed_scrutinee.ty.clone()
        }
    } else {
        typed_scrutinee.ty.clone()
    };
    let (enum_name, ok_variant, err_variant, _ok_payload, err_payload) = match &scrutinee_ty {
        Ty::Generic(name, args) if name == "Result" && args.len() == 2 => (
            "Result",
            "Ok",
            "Err",
            Some(args[0].clone()),
            Some(args[1].clone()),
        ),
        Ty::Generic(name, args) if name == "Optional" && args.len() == 1 => {
            ("Optional", "Some", "None", Some(args[0].clone()), None)
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "Result<T, E> ou Optional<T>".into(),
                found: format!("{}", scrutinee_ty),
                span: inner.span.into(),
            });
        }
    };

    // Nomes frescos para bindings.
    let ok_binding = "__q_ok";
    let err_binding = "__q_err";

    // Constrói pattern Ok(v) / Some(v).
    let ok_pattern = Pattern::Variant {
        enum_name: enum_name.to_string(),
        variant: ok_variant.to_string(),
        payload: Some(vec![Spanned::new(
            Pattern::Ident(ok_binding.to_string()),
            inner.span,
        )]),
    };

    // Constrói pattern Err(e) / None.
    let err_pattern = match err_payload {
        Some(_) => Pattern::Variant {
            enum_name: enum_name.to_string(),
            variant: err_variant.to_string(),
            payload: Some(vec![Spanned::new(
                Pattern::Ident(err_binding.to_string()),
                inner.span,
            )]),
        },
        None => Pattern::Variant {
            enum_name: enum_name.to_string(),
            variant: err_variant.to_string(),
            payload: None,
        },
    };

    // Constrói body do braço Ok: `v` (o valor desempacotado).
    let ok_body = Spanned::new(
        Expr::Ident {
            name: ok_binding.to_string(),
        },
        inner.span,
    );

    // Constrói body do braço Err: `return Err(e)` ou `return None`.
    let err_body_expr: Expr = match err_payload {
        Some(_) => Expr::Return(Box::new(Spanned::new(
            Expr::Apply {
                callee: Box::new(Spanned::new(
                    Expr::VariantQual {
                        enum_name: enum_name.to_string(),
                        variant: err_variant.to_string(),
                        module_path: None,
                    },
                    inner.span,
                )),
                args: vec![Spanned::new(
                    Expr::Ident {
                        name: err_binding.to_string(),
                    },
                    inner.span,
                )],
            },
            inner.span,
        ))),
        None => Expr::Return(Box::new(Spanned::new(
            Expr::VariantQual {
                enum_name: enum_name.to_string(),
                variant: err_variant.to_string(),
                module_path: None,
            },
            inner.span,
        ))),
    };
    let err_body = Spanned::new(err_body_expr, inner.span);

    // Constrói os arms do match sintético.
    let arms = vec![
        MatchArm {
            pattern: Some(Spanned::new(ok_pattern, inner.span)),
            guard: None,
            body: ok_body,
        },
        MatchArm {
            pattern: Some(Spanned::new(err_pattern, inner.span)),
            guard: None,
            body: err_body,
        },
    ];

    // Infere o match sintético.
    // Passa o inner (expr original) como scrutinee e os arms construídos.
    // infer_match re-infere o scrutinee, mas isso é seguro — o typeck é idempotente.
    let (match_ty, match_kind) = infer_match(inner, &arms, span, env, ctx, false, None)?;

    Ok(TypedExpr {
        span: *span,
        ty: match_ty,
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: match_kind,
    })
}

/// Desugar `lhs | rhs` para `match lhs { Ok(v) => v, Err(_) => rhs }`.
///
/// `|` é coalescência de erro: desempacota a variante não-cauda (Ok/Some),
/// avalia `rhs` se a variante é a cauda (Err/None). O payload da cauda é
/// descartado — o programador escolheu `|` em vez de `match`, indicando que
/// não precisa do erro. Diferença crucial do `?`: o braço de erro é `rhs`
/// (uma expressão), não `return Err(e)`. Não aborta — é pura.
///
/// Funciona em qualquer contexto (funções puras e Actions). Não precisa
/// de `ctx.ret_ty`.
pub(crate) fn infer_pipe_fallback(
    lhs: &Spanned<Expr>,
    rhs: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<TypedExpr> {
    // Infere o scrutinee (lhs).
    let typed_scrutinee = infer_expr(&lhs.node, &lhs.span, env, ctx, false)?;

    // Extrai o nome do enum do tipo do scrutinee.
    // Ty::Sum("Boolean") para enums não-genéricos, Ty::Generic("Optional", [T]) para genéricos.
    let enum_name = match &typed_scrutinee.ty {
        Ty::Sum(name) => name.clone(),
        Ty::Generic(name, _) => name.clone(),
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "enum com cauda unitaria".into(),
                found: format!("{}", typed_scrutinee.ty),
                span: lhs.span.into(),
            });
        }
    };

    // Busca as variantes no EnumRegistry.
    let variants =
        ctx.enum_registry
            .all_variants(&enum_name)
            .ok_or_else(|| MiddleError::TypeMismatch {
                expected: format!("enum conhecido: {enum_name}"),
                found: "tipo desconhecido pelo EnumRegistry".into(),
                span: lhs.span.into(),
            })?;

    if variants.is_empty() {
        return Err(MiddleError::TypeMismatch {
            expected: "enum com pelo menos uma variante".into(),
            found: format!("{enum_name} sem variantes"),
            span: lhs.span.into(),
        });
    }

    // Valida a invariante do `|`: todas as variantes exceto a última carregam
    // payload. A última (cauda) pode ter payload (ex: Result::Err(E)) — nesse
    // caso, o payload é descartado e o fallback (rhs) é avaliado. Enums onde
    // uma variante não-cauda não tem payload não são compatíveis com `|`.
    let last_idx = variants.len() - 1;
    for (i, v) in variants.iter().enumerate() {
        let is_last = i == last_idx;
        if !is_last && v.payload_ty.is_none() {
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "variante {} com payload (apenas a ultima pode ser cauda)",
                    v.name
                ),
                found: format!("{}::{} sem payload", enum_name, v.name),
                span: lhs.span.into(),
            });
        }
    }

    // Constrói os braços do match sintético.
    // Coerção contextual no `|`. Se o payload da primeira variante
    // não-cauda é um tipo refined (Struct com predicates), o fallback (rhs)
    // é implicitamente tratado como o mesmo refined type. Envolver rhs em
    // `TypeAscription { rhs::RefinedType }` valida o predicado em compile-time.
    //
    // Para enums genéricos, o payload_ty no EnumRegistry é Ty::Var("T").
    // Precisamos instanciar com o tipo concreto do scrutinee: se o scrutinee
    // é Generic("Optional", [Struct("PositiveInt")]), o payload Var("T")
    // resolve para Struct("PositiveInt").
    let binding = "__pipe_v";
    let type_args = match &typed_scrutinee.ty {
        Ty::Generic(_, args) => Some(args.clone()),
        _ => None,
    };
    let type_params = ctx.enum_registry.type_params_of(&enum_name);
    let refined_name: Option<String> =
        variants
            .iter()
            .find(|v| v.payload_ty.is_some())
            .and_then(|v| {
                let concrete_payload = v
                    .payload_ty
                    .as_ref()
                    .expect("find(|v| v.payload_ty.is_some()) garante Some");
                // Instancia Ty::Var com type_args do scrutinee.
                let resolved = match (concrete_payload, &type_args, &type_params) {
                    (Ty::Var(name), Some(args), Some(params)) => params
                        .iter()
                        .position(|p| p == name)
                        .and_then(|idx| args.get(idx).cloned())
                        .unwrap_or(concrete_payload.clone()),
                    _ => concrete_payload.clone(),
                };
                if let Ty::Struct(ref name) = resolved
                    && let Some(info) = ctx.struct_registry.get(name)
                    && info.predicates.is_some()
                {
                    return Some(name.clone());
                }
                None
            });

    let mut arms = Vec::with_capacity(variants.len());

    for (i, v) in variants.iter().enumerate() {
        let is_last = i == last_idx;

        let pattern = if is_last {
            // Cauda: a variante que ativa o fallback. Pode ter payload ou não.
            // Se tem payload, usamos Wildcard no sub-pattern para descartá-lo.
            Pattern::Variant {
                enum_name: enum_name.clone(),
                variant: v.name.clone(),
                payload: v
                    .payload_ty
                    .as_ref()
                    .map(|_| vec![Spanned::new(Pattern::Wildcard, lhs.span)]),
            }
        } else {
            // Variante com payload: Variant(v) — liga o payload.
            Pattern::Variant {
                enum_name: enum_name.clone(),
                variant: v.name.clone(),
                payload: Some(vec![Spanned::new(
                    Pattern::Ident(binding.to_string()),
                    lhs.span,
                )]),
            }
        };

        let body = if is_last {
            // Cauda: avalia o fallback (rhs).
            // Se o payload é refined, envolver rhs em ascription
            // refined para validar o predicado em compile-time.
            if let Some(ref rname) = refined_name {
                Spanned::new(
                    Expr::TypeAscription {
                        expr: Box::new(rhs.clone()),
                        ty: Spanned::new(TypeExpr::Named(rname.clone()), rhs.span),
                    },
                    rhs.span,
                )
            } else {
                rhs.clone()
            }
        } else {
            // Não-cauda: retorna o valor desempacotado.
            Spanned::new(
                Expr::Ident {
                    name: binding.to_string(),
                },
                lhs.span,
            )
        };

        arms.push(MatchArm {
            pattern: Some(Spanned::new(pattern, lhs.span)),
            guard: None,
            body,
        });
    }

    // Infere o match sintético.
    let (match_ty, match_kind) = infer_match(lhs, &arms, span, env, ctx, false, None)?;

    Ok(TypedExpr {
        span: *span,
        ty: match_ty,
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: match_kind,
    })
}

/// Desugar `assert!(cond, "msg")` para `match cond { True: Unit, False: panic!(msg) }`.
///
/// `assert!` recebe uma condição (Boolean) e uma mensagem opcional (Text).
/// Se a condição é falsa, chama `panic!(msg)`. O desugar constrói um match
/// sintético sobre `cond` com dois braços.
///
/// Com 1 arg (sem msg): `assert!(cond)` → `match cond { True: Unit, False: panic!("assertion failed") }`.
/// Com 2 args: `assert!(cond, "msg")` → `match cond { True: Unit, False: panic!("msg") }`.
pub(crate) fn infer_assert(
    args: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<TypedExpr> {
    // Extrai elementos da tupla de args.
    let (cond_expr, msg_expr) = match &args.node {
        Expr::Tuple { elements } => {
            if elements.len() == 2 {
                (elements[0].clone(), elements[1].clone())
            } else if elements.len() == 1 {
                // assert!(cond) — msg default
                (
                    elements[0].clone(),
                    Spanned::new(
                        Expr::TextLit {
                            text: "assertion failed".into(),
                        },
                        args.span,
                    ),
                )
            } else {
                return Err(MiddleError::TypeMismatch {
                    expected: "assert!(cond, msg?) — 1 ou 2 args".into(),
                    found: format!("{} args", elements.len()),
                    span: (*span).into(),
                });
            }
        }
        Expr::Grouping { inner } => {
            // assert!(cond) — parêntese sem vírgula = Grouping
            (
                inner.as_ref().clone(),
                Spanned::new(
                    Expr::TextLit {
                        text: "assertion failed".into(),
                    },
                    args.span,
                ),
            )
        }
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "tupla de args para assert!".into(),
                found: format!("{:?}", args.node),
                span: (*span).into(),
            });
        }
    };

    // Constrói panic!(msg) como ActionCall.
    let panic_call = Spanned::new(
        Expr::ActionCall {
            callee: "panic".into(),
            args: Box::new(Spanned::new(
                Expr::Tuple {
                    elements: vec![msg_expr],
                },
                args.span,
            )),
        },
        args.span,
    );

    // Constrói os braços do match sintético.
    // True: Unit
    let true_arm = MatchArm {
        pattern: Some(Spanned::new(
            Pattern::Variant {
                enum_name: "Boolean".into(),
                variant: "True".into(),
                payload: None,
            },
            cond_expr.span,
        )),
        guard: None,
        body: Spanned::new(Expr::Unit, cond_expr.span),
    };
    // False: panic!(msg)
    let false_arm = MatchArm {
        pattern: Some(Spanned::new(
            Pattern::Variant {
                enum_name: "Boolean".into(),
                variant: "False".into(),
                payload: None,
            },
            cond_expr.span,
        )),
        guard: None,
        body: panic_call,
    };

    let arms = vec![true_arm, false_arm];

    // Infere o match sintético.
    let (match_ty, match_kind) = infer_match(&cond_expr, &arms, span, env, ctx, false, None)?;

    Ok(TypedExpr {
        span: *span,
        ty: match_ty,
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: match_kind,
    })
}
