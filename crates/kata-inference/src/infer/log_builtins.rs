//! Log builtins — `log!`, `log_recv!`, `log_config!`.
//!
//! Extraído de `action_call.rs` — os três builtins de log são self-contained:
//! compartilham apenas `extract_tuple_elements` e `resolve_log_level`
//! (ambos exclusivos deste módulo), não dependem dos builtins CSP e não
//! chamam `infer_channel_builtin`/`infer_queue_builtin`/`infer_fork_builtin`.

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::action_call::ActionDispatch;
use super::expr::InferCtx;
use super::format_synthesis::infer_format;
use super::helpers::InferResult;
use super::log_template::{log_level_name, parse_placeholder, parse_template};

/// `log!(level, msg, topic_or_file?, policy?)` — desugara para FFI.
///
/// Args posicionais:
/// - 0: LogLevel (VariantQual ex: `LogLevel::Info` → tag i64)
/// - 1: Text (mensagem — tratada como template se for TextLit)
/// - 2: Text (tópico CSP) OU File (write direto) — opcional
/// - 3: Text (policy, só com tópico CSP) — opcional
///
/// Bifurcação por tipo do 3º arg:
/// - `Ty::text()` → `kata_rt_log_publish(topic, level, msg, policy)` (CSP)
/// - `Ty::File` → `kata_rt_file_write_text(file_handle, msg)` (write direto)
/// - Ausente → CSP com config herdada
pub(crate) fn infer_log_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    let elements = extract_tuple_elements(args)?;
    if elements.len() < 2 || elements.len() > 4 {
        return Err(MiddleError::ArityMismatch {
            expected: 4, // aceita 2-4, mas informamos max
            found: elements.len(),
            span: args.span.into(),
            hint: None,
        });
    }

    // Level: VariantQual (LogLevel::Info) → tag i64 via enum_registry.
    let level_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    let level_val = resolve_log_level(&level_typed, ctx, &elements[0].span)?;
    let level_tag = extract_level_tag(&level_val);

    // Msg: Text — tratada como template se for TextLit.
    // Se for TextLit, parsear template com {log_level} e placeholders.
    // Se for outra expr, inferir como Text puro (sem template).
    let msg_typed = if let Expr::TextLit { text } = &elements[1].node {
        // Template: parsear, injetar {log_level}, chamar infer_format.
        synthesize_template_msg(text, level_tag, &elements[1].span, env, ctx)?
    } else {
        // Expr dinâmica — inferir como Text puro (sem template).
        let t = super::expr::infer_expr(&elements[1].node, &elements[1].span, env, ctx, false)?;
        if t.ty != Ty::text() {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{}", Ty::text()),
                found: format!("{}", t.ty),
                span: elements[1].span.into(),
            });
        }
        t
    };

    // 3º arg: Text (CSP) ou File (write direto) — opcional.
    let third_typed = if let Some(elem) = elements.get(2) {
        Some(super::expr::infer_expr(
            &elem.node, &elem.span, env, ctx, false,
        )?)
    } else {
        None
    };

    // 4º arg: policy (Text) — só válido com tópico CSP.
    let policy_typed = if let Some(elem) = elements.get(3) {
        Some(super::expr::infer_expr(
            &elem.node, &elem.span, env, ctx, false,
        )?)
    } else {
        None
    };

    // Bifurcação por tipo do 3º arg.
    let typed = match &third_typed {
        None => {
            // Sem 3º arg → CSP com config herdada (como hoje).
            // Policy não é passado aqui (3º arg ausente = 2 args).
            build_csp_closure(
                args.span,
                &level_val,
                &msg_typed,
                &TypedExpr {
                    span: args.span,
                    ty: Ty::int(),
                    tail_pos: false,
                    escape: kata_core::escape::EscapeTarget::Local,
                    kind: TypedExprKind::Unit,
                },
                &policy_typed,
                &elements,
            )
        }
        Some(t) if t.ty == Ty::text() => {
            // CSP: kata_rt_log_publish(topic, level, msg, policy).
            if let Some(p) = &policy_typed
                && p.ty != Ty::text()
            {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", Ty::text()),
                    found: format!("{}", p.ty),
                    span: elements[3].span.into(),
                });
            }
            build_csp_closure(
                args.span,
                &level_val,
                &msg_typed,
                t,
                &policy_typed,
                &elements,
            )
        }
        Some(t) if t.ty == Ty::File => {
            // File: kata_rt_file_write_text(file_handle, msg).
            // Policy é erro com File.
            if let Some(p) = &policy_typed {
                return Err(MiddleError::TypeMismatch {
                    expected: "sem 4º argumento (policy não é válido com File)".into(),
                    found: format!("{}", p.ty),
                    span: elements[3].span.into(),
                });
            }
            build_file_closure(args.span, t, &msg_typed)
        }
        Some(t) => {
            return Err(MiddleError::TypeMismatch {
                expected: "Text (tópico CSP) ou File (write direto)".into(),
                found: format!("{}", t.ty),
                span: elements[2].span.into(),
            });
        }
    };

    Ok(ActionDispatch::Complete(typed))
}

/// Constrói a Closure para `kata_rt_log_publish` (caminho CSP).
fn build_csp_closure(
    span: Span,
    level_val: &TypedExpr,
    msg_typed: &TypedExpr,
    topic_typed: &TypedExpr,
    policy_typed: &Option<TypedExpr>,
    elements: &[Spanned<Expr>],
) -> TypedExpr {
    let policy = policy_typed.as_ref().map_or(
        TypedExpr {
            span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::Unit,
        },
        |p| p.clone(),
    );

    let callee = TypedExpr {
        span,
        ty: Ty::Function(
            vec![Ty::int(), Ty::text(), Ty::text(), Ty::text()],
            Box::new(Ty::int()),
        ),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_publish".into(),
        },
    };

    TypedExpr {
        span,
        ty: Ty::int(),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, span)),
            // Ordem dos args coincide com a assinatura da FFI:
            // kata_rt_log_publish(topic_ptr, level, msg, policy_ptr).
            args: vec![
                Spanned::new(
                    topic_typed.clone(),
                    elements.get(2).map(|e| e.span).unwrap_or(span),
                ),
                Spanned::new(level_val.clone(), elements[0].span),
                Spanned::new(msg_typed.clone(), elements[1].span),
                Spanned::new(policy, elements.get(3).map(|e| e.span).unwrap_or(span)),
            ],
            ffi_symbol: Some("kata_rt_log_publish".into()),
        },
    }
}

/// Constrói a Closure para `kata_rt_file_write_text` (caminho File).
///
/// `kata_rt_file_write_text(handle: i64, data_ptr: i64) -> i64 (Result box)`
fn build_file_closure(span: Span, file_typed: &TypedExpr, msg_typed: &TypedExpr) -> TypedExpr {
    let callee = TypedExpr {
        span,
        ty: Ty::Function(vec![Ty::int(), Ty::text()], Box::new(Ty::int())),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_file_write_text".into(),
        },
    };

    TypedExpr {
        span,
        ty: Ty::int(),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, span)),
            args: vec![
                Spanned::new(file_typed.clone(), span),
                Spanned::new(msg_typed.clone(), span),
            ],
            ffi_symbol: Some("kata_rt_file_write_text".into()),
        },
    }
}

/// Sintetiza a mensagem como template: parsear placeholders, injetar
/// `{log_level}` como variável sintética, chamar `infer_format`.
///
/// Retorna a expressão tipada que produz `Text` (cadeia de
/// `text_replace_first`).
fn synthesize_template_msg(
    msg: &str,
    level_tag: i64,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> Result<TypedExpr, MiddleError> {
    let (template, placeholders) = parse_template(msg).map_err(|e| MiddleError::TypeMismatch {
        expected: "template válido: {expr}, {{ escapa {".into(),
        found: e,
        span: (*span).into(),
    })?;

    // Constrói Expr::Ident para cada placeholder.
    // {log_level} é resolvido para TextLit com a string do level.
    let mut args = Vec::new();
    for ph in &placeholders {
        if ph == "log_level" {
            // {log_level} → TextLit com a string do level.
            args.push(Spanned::new(
                Expr::TextLit {
                    text: log_level_name(level_tag).to_string(),
                },
                Span::synthetic(),
            ));
        } else {
            let expr = parse_placeholder(ph).map_err(|e| MiddleError::TypeMismatch {
                expected: "placeholder válido: {expr} ou {expr.field}".into(),
                found: e,
                span: (*span).into(),
            })?;
            args.push(Spanned::new(expr, Span::synthetic()));
        }
    }

    // Constrói a chamada para infer_format:
    // format "template {} {}" (arg1, arg2)
    let template_expr = Expr::TextLit { text: template };
    let tuple_expr = if args.is_empty() {
        Expr::Unit
    } else if args.len() == 1 {
        Expr::Grouping {
            inner: Box::new(Spanned::new(
                Expr::Tuple { elements: args },
                Span::synthetic(),
            )),
        }
    } else {
        Expr::Tuple { elements: args }
    };

    let format_args = vec![
        Spanned::new(template_expr, Span::synthetic()),
        Spanned::new(tuple_expr, Span::synthetic()),
    ];

    let (msg_ty, msg_kind) = infer_format(
        &Spanned::new(Expr::Unit, Span::synthetic()),
        &format_args,
        span,
        env,
        ctx,
    )?;

    if msg_ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: "Text".into(),
            found: format!("{msg_ty}"),
            span: (*span).into(),
        });
    }

    Ok(TypedExpr {
        span: *span,
        ty: msg_ty,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: msg_kind,
    })
}

/// Extrai a tag i64 de um `TypedExpr` que é `IntLit`.
///
/// Usado para resolver `{log_level}` — precisa da tag numérica do level
/// para mapear para a string ("Info", "Warn", etc.).
fn extract_level_tag(typed: &TypedExpr) -> i64 {
    if let TypedExprKind::IntLit { text } = &typed.kind {
        text.parse().unwrap_or(1) // default Info
    } else {
        1 // default Info
    }
}

/// `log_recv!(topic)` — desugara para `kata_rt_log_recv`.
pub(crate) fn infer_log_recv_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    let elements = extract_tuple_elements(args)?;
    if elements.len() != 1 {
        return Err(MiddleError::ArityMismatch {
            expected: 1,
            found: elements.len(),
            span: args.span.into(),
            hint: None,
        });
    }

    let topic_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    if topic_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", topic_typed.ty),
            span: elements[0].span.into(),
        });
    }

    let result_ty = Ty::Generic("Result".into(), vec![Ty::text(), Ty::text()]);

    let callee = TypedExpr {
        span: args.span,
        ty: Ty::Function(vec![Ty::text()], Box::new(result_ty.clone())),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_recv".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: result_ty,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, args.span)),
            args: vec![Spanned::new(topic_typed, elements[0].span)],
            ffi_symbol: Some("kata_rt_log_recv".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}

/// `log_config!(topic, policy, level)` — desugara para `kata_rt_log_config`.
pub(crate) fn infer_log_config_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<ActionDispatch> {
    let elements = extract_tuple_elements(args)?;
    if elements.len() != 3 {
        return Err(MiddleError::ArityMismatch {
            expected: 3,
            found: elements.len(),
            span: args.span.into(),
            hint: None,
        });
    }

    let topic_typed =
        super::expr::infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    if topic_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", topic_typed.ty),
            span: elements[0].span.into(),
        });
    }

    let policy_typed =
        super::expr::infer_expr(&elements[1].node, &elements[1].span, env, ctx, false)?;
    if policy_typed.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", policy_typed.ty),
            span: elements[1].span.into(),
        });
    }

    let level_typed =
        super::expr::infer_expr(&elements[2].node, &elements[2].span, env, ctx, false)?;
    let level_val = resolve_log_level(&level_typed, ctx, &elements[2].span)?;

    let callee = TypedExpr {
        span: args.span,
        ty: Ty::Function(vec![Ty::text(), Ty::text(), Ty::int()], Box::new(Ty::Unit)),
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_log_config".into(),
        },
    };

    let typed = TypedExpr {
        span: args.span,
        ty: Ty::Unit,
        tail_pos: false,
        escape: kata_core::escape::EscapeTarget::Caller,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(callee, args.span)),
            args: vec![
                Spanned::new(topic_typed, elements[0].span),
                Spanned::new(policy_typed, elements[1].span),
                Spanned::new(level_val, elements[2].span),
            ],
            ffi_symbol: Some("kata_rt_log_config".into()),
        },
    };

    Ok(ActionDispatch::Complete(typed))
}

/// Extrai elementos de uma tupla, Grouping de tupla, ou Grouping de expr única.
///
/// O parser produz `Grouping(inner)` para `f!(x)` (1 arg) e `Tuple([...])` para
/// `f!(x, y)` (2+ args). Para builtins como `log_recv!(x)` que exigem 1 arg,
/// tratamos `Grouping(x)` como tupla de 1 elemento `[x]` — mesmo padrão que
/// `infer_action_call` aplica ao normalizar args de ActionCall.
fn extract_tuple_elements(args: &Spanned<Expr>) -> Result<Vec<Spanned<Expr>>, MiddleError> {
    match &args.node {
        Expr::Tuple { elements } => Ok(elements.clone()),
        Expr::Grouping { inner } => match &inner.node {
            Expr::Tuple { elements } => Ok(elements.clone()),
            // Grouping de expr única = tupla de 1 elemento.
            // `log_recv!("x")` parseia como Grouping(TextLit("x")).
            other => Ok(vec![Spanned::new(other.clone(), inner.span)]),
        },
        Expr::Unit => Ok(Vec::new()),
        _ => Err(MiddleError::TypeMismatch {
            expected: "Tuple".into(),
            found: format!("{:?}", args.node),
            span: args.span.into(),
        }),
    }
}

/// Resolve uma expressão LogLevel (VariantQual ou IntLit) para tag i64.
fn resolve_log_level(
    typed: &TypedExpr,
    _ctx: &InferCtx,
    span: &Span,
) -> Result<TypedExpr, MiddleError> {
    // Se já é IntLit, retorna.
    if let TypedExprKind::IntLit { .. } = &typed.kind {
        return Ok(typed.clone());
    }

    // Se é VariantQual de LogLevel, extrai a tag.
    if let TypedExprKind::VariantQual {
        enum_name,
        variant: _,
        tag,
        ..
    } = &typed.kind
        && enum_name == "LogLevel"
    {
        return Ok(TypedExpr {
            span: typed.span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: tag.to_string(),
            },
        });
    }

    // Se é VariantConstruct de LogLevel.
    if let TypedExprKind::VariantConstruct {
        enum_name,
        variant: _,
        tag,
        ..
    } = &typed.kind
        && enum_name == "LogLevel"
    {
        return Ok(TypedExpr {
            span: typed.span,
            ty: Ty::int(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: tag.to_string(),
            },
        });
    }

    // Fallback: se o tipo é Int, usa direto.
    if typed.ty == Ty::int() {
        return Ok(typed.clone());
    }

    Err(MiddleError::TypeMismatch {
        expected: "LogLevel variant (Debug, Info, Warn, Error) ou Int".into(),
        found: format!("{}", typed.ty),
        span: (*span).into(),
    })
}
