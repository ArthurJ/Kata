//! Typeck de expressões CSP.
//!
//! `TransmissionOp` (`<!` / `!>`), e `Select` são inferidos aqui.
//! `channel!()`, `queue!()`, `broadcast!()`, `rxf!()`, `fork!()` são
//! interceptados em `infer_apply` (não despacham para DispatchTable).

use kata_ast::{Expr, ReadMode, SelectArm, Span, Spanned, TransmissionDir};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind, TypedReadMode, TypedSelectArm};

use super::expr::InferCtx;
use super::expr::infer_expr_hinted;
use super::helpers::InferResult;

/// Operador direcional de canal: `source <! dest` (Left) ou `source !> dest` (Right).
///
/// O dado flui na direção da seta. A inference decide se é send ou recv
/// pelo tipo do `source`:
/// - `Sender::T` → send: o source é o canal, o dest é o valor a enviar.
/// - `Receiver::T` → recv: o source é o canal, o dest é o binding (Ident).
///
/// **Unificação bidirecional de T0:** quando o tipo do valor é concreto e o
/// tipo do canal é `Var(T0)`, `T0` é resolvido para o tipo concreto no
/// `TypeEnv`. Isso resolve o bug onde variáveis recebidas via canal ficavam
/// com tipo `Var` não-resolvido.
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_transmission_op(
    source: &Spanned<Expr>,
    direction: TransmissionDir,
    dest: &Spanned<Expr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    // Inferir o source (lado de onde o dado vem).
    let typed_source = infer_expr_hinted(&source.node, &source.span, env, ctx, false, None)?;

    // Despachar pelo tipo do source.
    //
    // Sender como source é ERRO: a semântica do TransmissionOp diz que
    // source é "de onde o dado vem". Sender é endpoint de escrita — dado
    // vai PARA o Sender, nunca sai DELE. Os sends legítimos (tx <! 42,
    // 42 !> tx) têm source como valor concreto e caem no braço `other`,
    // que infere o dest como Sender e chama infer_send_flipped.
    //
    // Receiver como source é RECV: Receiver é endpoint de leitura — dado
    // sai do Receiver. Isto é legítimo (rx !> a, a <! rx).
    match &typed_source.ty {
        Ty::Sender(_) => {
            // Sender como source = tentativa de recv de um endpoint de
            // escrita. Erro de tipo, não UnboundName.
            Err(MiddleError::TypeMismatch {
                expected: "Receiver::T (endpoint de leitura) como source de !>".into(),
                found: "Sender::T — Sender é endpoint de escrita, não pode ser source de dados. \
                     Para enviar, use `tx <! valor` ou `valor !> tx`"
                    .into(),
                span: source.span.into(),
            })
        }
        Ty::Receiver(inner) => {
            let inner_ty = (**inner).clone();
            infer_recv(
                typed_source,
                Box::new(inner_ty),
                dest,
                direction,
                span,
                env,
                ctx,
                tail_pos,
                hint,
            )
        }
        other => {
            // Source não é canal. Se for Var, pode ser um canal não-resolvido.
            if matches!(other, Ty::Var(_)) {
                // Fallback conservador para type params não-resolvidos.
                match direction {
                    TransmissionDir::Left => infer_send(
                        typed_source,
                        Box::new(Ty::Var("__chan_elem__".into())),
                        dest,
                        direction,
                        span,
                        env,
                        ctx,
                        tail_pos,
                    ),
                    TransmissionDir::Right => infer_recv(
                        typed_source,
                        Box::new(Ty::Var("__chan_elem__".into())),
                        dest,
                        direction,
                        span,
                        env,
                        ctx,
                        tail_pos,
                        hint,
                    ),
                }
            } else {
                // Source é um valor concreto (não-canal). O dest deve ser o canal.
                // Isto acontece em `tx <! 42` (Left: dest=tx é o canal, source=42 é o valor)
                // ou `42 !> tx` (Right: dest=tx é o canal, source=42 é o valor).
                // Inferir o dest e checar se é Sender.
                let typed_dest = infer_expr_hinted(&dest.node, &dest.span, env, ctx, false, None)?;
                match &typed_dest.ty {
                    Ty::Sender(inner) => {
                        let elem_ty = (**inner).clone();
                        // Inverter: dest é o canal, source é o valor.
                        // Chamar infer_send com typed_dest como canal e source como valor.
                        infer_send_flipped(
                            typed_dest,
                            Box::new(elem_ty),
                            typed_source,
                            direction,
                            span,
                            env,
                            ctx,
                            tail_pos,
                        )
                    }
                    Ty::Receiver(inner) => {
                        let inner_ty = (**inner).clone();
                        // Inverter: dest é o canal (Receiver), source é o binding.
                        infer_recv_flipped(
                            typed_dest,
                            Box::new(inner_ty),
                            typed_source,
                            direction,
                            span,
                            env,
                            ctx,
                            tail_pos,
                            hint,
                        )
                    }
                    dest_ty => Err(MiddleError::TypeMismatch {
                        expected: "Sender::T ou Receiver::T (canal)".into(),
                        found: format!("source={other:?}, dest={dest_ty:?}"),
                        span: (*span).into(),
                    }),
                }
            }
        }
    }
}

/// Send flipped: o dest é o canal (Sender), o source é o valor.
/// Usado quando `tx <! 42` é parseado como source=42, dest=tx.
/// O `typed_channel` é o dest (já inferido como Sender), `typed_value` é o source.
#[allow(clippy::too_many_arguments)]
fn infer_send_flipped(
    typed_channel: TypedExpr,
    elem_ty: Box<Ty>,
    typed_value: TypedExpr,
    direction: TransmissionDir,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let channel_span = typed_channel.span;
    let value_span = typed_value.span;

    // Proibe Ty::Action em canal.
    if let Ty::Action(..) = &typed_value.ty {
        return Err(MiddleError::TypeMismatch {
            expected: "valor serializável (não-Action)".into(),
            found: format!(
                "Action não é permitida em canal. Tipo: `{}`",
                typed_value.ty
            ),
            span: value_span.into(),
        });
    }

    // Proibe canais como payload.
    if matches!(
        &typed_value.ty,
        Ty::Sender(_) | Ty::Receiver(_) | Ty::ReceiverFactory(_)
    ) {
        return Err(MiddleError::TypeMismatch {
            expected: "valor serializável (não-Canal)".into(),
            found: format!("Canal não é permitido em canal. Tipo: `{}`", typed_value.ty),
            span: value_span.into(),
        });
    }

    // Unificação bidirecional de T0.
    let final_elem_ty = unify_channel_elem(&elem_ty, &typed_value.ty, env);

    if !type_compatible(&typed_value.ty, &final_elem_ty) {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{final_elem_ty:?}"),
            found: format!("{}", typed_value.ty),
            span: value_span.into(),
        });
    }

    let escape = escape_for_channel_send(&typed_value.ty, tail_pos, ctx);
    let typed_value = if escape != typed_value.escape {
        TypedExpr {
            escape,
            ..typed_value
        }
    } else {
        typed_value
    };

    Ok(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos,
        escape,
        kind: TypedExprKind::TransmissionOp {
            source: Box::new(Spanned::new(typed_value, value_span)),
            direction,
            dest: Box::new(Spanned::new(typed_channel, channel_span)),
            elem_ty: final_elem_ty,
            is_send: true,
            bind_name: None,
        },
    })
}

/// Recv flipped: o dest é o canal (Receiver), o source é o binding.
/// Usado quando `a <! rx` é parseado como source=a, dest=rx — mas na verdade
/// rx é o canal e a é o binding. O `typed_channel` é o dest (Receiver).
#[allow(clippy::too_many_arguments, clippy::boxed_local)]
fn infer_recv_flipped(
    typed_channel: TypedExpr,
    inner: Box<Ty>,
    typed_binding: TypedExpr,
    direction: TransmissionDir,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let channel_span = typed_channel.span;
    let recv_ty = match &*inner {
        Ty::Var(_)
            if hint.is_some()
                && !matches!(hint.expect("hint is Some — guard verified"), Ty::Var(_)) =>
        {
            hint.expect("hint is Some — guard verified").clone()
        }
        _ => (*inner).clone(),
    };

    let binding_span = typed_binding.span;

    // O binding deve ser um Ident.
    let bind_name = match &typed_binding.kind {
        TypedExprKind::Ident { name } => name.clone(),
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "identificador (nome do binding de recebimento)".into(),
                found: format!("{:?}", typed_binding.kind),
                span: typed_binding.span.into(),
            });
        }
    };

    if bind_name != "_" {
        env.define(&bind_name, recv_ty.clone(), "__local__");
    }

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty: recv_ty.clone(),
        tail_pos,
        escape,
        kind: TypedExprKind::TransmissionOp {
            source: Box::new(Spanned::new(typed_channel, channel_span)),
            direction,
            dest: Box::new(Spanned::new(typed_binding, binding_span)),
            elem_ty: recv_ty,
            is_send: false,
            bind_name: Some(bind_name),
        },
    })
}

/// Send: `canal <! valor` ou `valor !> canal`.
/// O `typed_channel` já foi inferido. `elem_ty` é o tipo interno do Sender.
#[allow(clippy::too_many_arguments)]
fn infer_send(
    typed_channel: TypedExpr,
    elem_ty: Box<Ty>,
    value_expr: &Spanned<Expr>,
    direction: TransmissionDir,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let channel_span = typed_channel.span;

    // Inferir o valor com hint = elem_ty do canal.
    let typed_value = infer_expr_hinted(
        &value_expr.node,
        &value_expr.span,
        env,
        ctx,
        false,
        Some(&elem_ty),
    )?;

    // Proibe Ty::Action em canal — Actions são comportamento, não informação.
    if let Ty::Action(..) = &typed_value.ty {
        return Err(MiddleError::TypeMismatch {
            expected: "valor serializável (não-Action)".into(),
            found: format!(
                "Action não é permitida em canal — Actions são comportamento, não informação. \
                 Tipo do valor: `{}`",
                typed_value.ty
            ),
            span: value_expr.span.into(),
        });
    }

    // Proibe canais como payload (endpoint mobility).
    if matches!(
        &typed_value.ty,
        Ty::Sender(_) | Ty::Receiver(_) | Ty::ReceiverFactory(_)
    ) {
        return Err(MiddleError::TypeMismatch {
            expected: "valor serializável (não-Action, não-Canal)".into(),
            found: format!(
                "Canal não é permitido em canal — use argumento de fork! ou retorno de Action. \
                 Tipo: `{}`",
                typed_value.ty
            ),
            span: value_expr.span.into(),
        });
    }

    // ── Unificação bidirecional de T0 ──
    // Se elem_ty é Var e o valor é concreto, resolver a Var no TypeEnv.
    let final_elem_ty = unify_channel_elem(&elem_ty, &typed_value.ty, env);

    if !type_compatible(&typed_value.ty, &final_elem_ty) {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{final_elem_ty:?}"),
            found: format!("{}", typed_value.ty),
            span: value_expr.span.into(),
        });
    }

    let escape = escape_for_channel_send(&typed_value.ty, tail_pos, ctx);

    // Override escape: valores compostos precisam sobreviver ao sender.
    let typed_value = if escape != typed_value.escape {
        TypedExpr {
            escape,
            ..typed_value
        }
    } else {
        typed_value
    };

    // Construir source e dest conforme a direção.
    let (source_typed, dest_typed) = match direction {
        TransmissionDir::Left => {
            // dest <! source → channel é dest, value é source
            (
                Spanned::new(typed_value, value_expr.span),
                Spanned::new(typed_channel, channel_span),
            )
        }
        TransmissionDir::Right => {
            // source !> dest → channel é source, value é dest
            (
                Spanned::new(typed_channel, channel_span),
                Spanned::new(typed_value, value_expr.span),
            )
        }
    };

    Ok(TypedExpr {
        span: *span,
        ty: Ty::Unit,
        tail_pos,
        escape,
        kind: TypedExprKind::TransmissionOp {
            source: Box::new(source_typed),
            direction,
            dest: Box::new(dest_typed),
            elem_ty: final_elem_ty,
            is_send: true,
            bind_name: None,
        },
    })
}

/// Recv: `canal !> binding` ou `binding <! canal`.
/// O `typed_channel` já foi inferido. `inner` é o tipo interno do Receiver.
/// `hint` é o tipo esperado pelo contexto (return type, ascription, etc.).
/// Se `inner` é `Var` e `hint` é concreto, `hint` resolve a variável de tipo.
#[allow(clippy::too_many_arguments, clippy::boxed_local)]
fn infer_recv(
    typed_channel: TypedExpr,
    inner: Box<Ty>,
    dest_expr: &Spanned<Expr>,
    direction: TransmissionDir,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let channel_span = typed_channel.span;
    // Se o tipo do canal é Var (não-resolvido), usar o hint se disponível.
    let recv_ty = match &*inner {
        Ty::Var(_)
            if hint.is_some()
                && !matches!(hint.expect("hint is Some — guard verified"), Ty::Var(_)) =>
        {
            hint.expect("hint is Some — guard verified").clone()
        }
        _ => (*inner).clone(),
    };

    // O dest deve ser um Ident (binding name). `_` (Hole) é aceito como descarte.
    let bind_name = match &dest_expr.node {
        Expr::Ident { name } => name.clone(),
        Expr::Hole => "_".into(),
        _ => {
            return Err(MiddleError::TypeMismatch {
                expected: "identificador (nome do binding de recebimento)".into(),
                found: format!("{:?}", dest_expr.node),
                span: dest_expr.span.into(),
            });
        }
    };

    // Criar binding no TypeEnv: bind_name := recv_ty. `_` não cria binding.
    if bind_name != "_" {
        env.define(&bind_name, recv_ty.clone(), "__local__");
    }

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    // O dest no TAST é o binding como Ident.
    let dest_typed = TypedExpr {
        span: dest_expr.span,
        ty: recv_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: bind_name.clone(),
        },
    };

    // Construir source e dest conforme a direção.
    let (source_typed, dest_typed) = match direction {
        TransmissionDir::Left => {
            // binding <! canal → canal é source (RHS), binding é dest (LHS)
            (
                Spanned::new(typed_channel, channel_span),
                Spanned::new(dest_typed, dest_expr.span),
            )
        }
        TransmissionDir::Right => {
            // canal !> binding → canal é source (LHS), binding é dest (RHS)
            (
                Spanned::new(typed_channel, channel_span),
                Spanned::new(dest_typed, dest_expr.span),
            )
        }
    };

    Ok(TypedExpr {
        span: *span,
        ty: recv_ty.clone(),
        tail_pos,
        escape,
        kind: TypedExprKind::TransmissionOp {
            source: Box::new(source_typed),
            direction,
            dest: Box::new(dest_typed),
            elem_ty: recv_ty,
            is_send: false,
            bind_name: Some(bind_name),
        },
    })
}

/// Unifica `elem_ty` com `value_ty` quando um dos dois é `Ty::Var`.
///
/// Se `elem_ty` é `Var(name)` e `value_ty` é concreto, retorna `value_ty`
/// (o tipo concreto substitui a variável de tipo). Caso contrário, retorna
/// `elem_ty` inalterado.
///
/// Esta é a correção do bug de unificação de T0: antes, `type_compatible` apenas
/// checava compatibilidade sem substituir a Var, deixando `T0` não-resolvido.
/// O tipo resoluido é propagado no TAST (`elem_ty` no `TransmissionOp`), e o
/// `infer_recv` extrai o tipo do `Receiver` já resolvido.
fn unify_channel_elem(elem_ty: &Ty, value_ty: &Ty, env: &mut TypeEnv) -> Ty {
    match (elem_ty, value_ty) {
        (Ty::Var(name), concrete) if !matches!(concrete, Ty::Var(_)) => {
            // T0 := concreto. Propagar a substituição para TODOS os bindings
            // no env (incluindo o receiver do mesmo canal).
            let mut subs = std::collections::HashMap::new();
            subs.insert(name.clone(), concrete.clone());
            env.apply_substitutions(&subs);
            concrete.clone()
        }
        _ => elem_ty.clone(),
    }
}
/// `select` com braços de canal, I/O e timeout opcional.
///
/// Cada braço lê de seu canal/handle e executa seu corpo
/// independentemente. Os receivers **não precisam ter o mesmo tipo** —
/// cada braço faz binding do seu próprio `recv_ty`.
/// Os corpos dos braços devem produzir o mesmo tipo (o valor do braço
/// que disparar é o valor do `select`).
/// Braços de I/O: binding recebe `ReadResult::(Bytes)` (read) ou `ReadResult::(Text)` (readline).
pub(crate) fn infer_select(
    arms: &[SelectArm],
    timeout_ms: &Option<Box<Spanned<Expr>>>,
    timeout_body: &Option<Box<Spanned<Expr>>>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let mut typed_arms: Vec<TypedSelectArm> = Vec::new();
    let mut unified_ty: Option<Ty> = None;

    for arm in arms {
        match arm {
            SelectArm::Channel {
                channel,
                bind_name,
                body,
            } => {
                let typed_channel =
                    infer_expr_hinted(&channel.node, &channel.span, env, ctx, false, None)?;

                // Verifica que channel é Receiver::T.
                let recv_ty = match &typed_channel.ty {
                    Ty::Receiver(inner) => (**inner).clone(),
                    other => {
                        return Err(MiddleError::TypeMismatch {
                            expected: "Receiver::T (canal receiver)".into(),
                            found: format!("{other:?}"),
                            span: channel.span.into(),
                        });
                    }
                };

                // ── Escopo único da action: braço NÃO abre escopo filho ──
                // Binding do braço vive na action e evapora no fim do braço
                // (leitura pós-select de binding de braço é UnboundName).
                let keys_before = env.local_keys();
                env.define(bind_name, recv_ty.clone(), "__local__");

                let typed_body =
                    infer_expr_hinted(&body.node, &body.span, env, ctx, tail_pos, None)?;

                // Evaporação dos bindings frescos do braço.
                for key in env.local_keys() {
                    if !keys_before.contains(&key) {
                        env.undefine(&key);
                    }
                }

                // Unifica tipo do BODY entre braços (não do binding).
                if let Some(ref existing) = unified_ty {
                    if !type_compatible(&typed_body.ty, existing) {
                        return Err(MiddleError::TypeMismatch {
                            expected: format!("{existing:?} (tipo do primeiro braço do select)"),
                            found: format!("{}", typed_body.ty),
                            span: body.span.into(),
                        });
                    }
                } else {
                    unified_ty = Some(typed_body.ty.clone());
                }

                typed_arms.push(TypedSelectArm::Channel {
                    channel: Spanned::new(typed_channel, channel.span),
                    recv_ty: recv_ty.clone(),
                    bind_name: bind_name.clone(),
                    body: Spanned::new(typed_body, body.span),
                });
            }
            SelectArm::IoRead {
                handle_expr,
                read_mode,
                bind_name,
                body,
            } => {
                // Typecheck handle_expr — deve ser Ty::File ou Ty::Socket.
                let typed_handle =
                    infer_expr_hinted(&handle_expr.node, &handle_expr.span, env, ctx, false, None)?;
                if !matches!(typed_handle.ty, Ty::File | Ty::Socket) {
                    return Err(MiddleError::TypeMismatch {
                        expected: "File or Socket (handle de I/O)".into(),
                        found: format!("{}", typed_handle.ty),
                        span: handle_expr.span.into(),
                    });
                }

                // Typecheck conforme o modo de leitura.
                let (typed_read_mode, result_ty) = match read_mode {
                    ReadMode::Chunk(chunk_size_expr) => {
                        // read!(handle, n) — chunk_size_expr deve ser Int.
                        let typed_chunk = infer_expr_hinted(
                            &chunk_size_expr.node,
                            &chunk_size_expr.span,
                            env,
                            ctx,
                            false,
                            None,
                        )?;
                        if !type_compatible(&typed_chunk.ty, &Ty::int()) {
                            return Err(MiddleError::TypeMismatch {
                                expected: "Int (tamanho do chunk)".into(),
                                found: format!("{}", typed_chunk.ty),
                                span: chunk_size_expr.span.into(),
                            });
                        }
                        let result_ty =
                            Ty::Generic("ReadResult".to_string(), vec![Ty::Bytes]);
                        (
                            TypedReadMode::Chunk(Box::new(Spanned::new(
                                typed_chunk,
                                chunk_size_expr.span,
                            ))),
                            result_ty,
                        )
                    }
                    ReadMode::Line => {
                        // readline!(handle) — sem chunk_size.
                        // Binding recebe ReadResult::(Text).
                        let result_ty =
                            Ty::Generic("ReadResult".to_string(), vec![Ty::text()]);
                        (TypedReadMode::Line, result_ty)
                    }
                };

                // ── Escopo único da action: braço NÃO abre escopo filho ──
                let keys_before = env.local_keys();
                env.define(bind_name, result_ty.clone(), "__local__");

                let typed_body =
                    infer_expr_hinted(&body.node, &body.span, env, ctx, tail_pos, None)?;

                // Evaporação dos bindings frescos do braço.
                for key in env.local_keys() {
                    if !keys_before.contains(&key) {
                        env.undefine(&key);
                    }
                }

                // Unifica tipo do BODY entre braços (não do binding).
                if let Some(ref existing) = unified_ty {
                    if !type_compatible(&typed_body.ty, existing) {
                        return Err(MiddleError::TypeMismatch {
                            expected: format!("{existing:?} (tipo do primeiro braço do select)"),
                            found: format!("{}", typed_body.ty),
                            span: body.span.into(),
                        });
                    }
                } else {
                    unified_ty = Some(typed_body.ty.clone());
                }

                typed_arms.push(TypedSelectArm::IoRead {
                    handle_expr: Spanned::new(typed_handle, handle_expr.span),
                    read_mode: typed_read_mode,
                    bind_ty: result_ty.clone(),
                    bind_name: bind_name.clone(),
                    body: Spanned::new(typed_body, body.span),
                });
            }
        }
    }

    let select_ty = unified_ty.unwrap_or(Ty::Unit);

    // Typeck do timeout.
    let mut typed_timeout_ms = None;
    let mut typed_timeout_body = None;

    if let Some(tm) = timeout_ms {
        let tm_typed = infer_expr_hinted(&tm.node, &tm.span, env, ctx, false, None)?;
        // timeout_ms deve ser Int.
        if !type_compatible(&tm_typed.ty, &Ty::int()) {
            return Err(MiddleError::TypeMismatch {
                expected: "Int (timeout em milissegundos)".into(),
                found: format!("{}", tm_typed.ty),
                span: tm.span.into(),
            });
        }
        typed_timeout_ms = Some(Box::new(Spanned::new(tm_typed, tm.span)));
    }

    if let (Some(tb), Some(_)) = (timeout_body, &typed_timeout_ms) {
        let tb_typed = infer_expr_hinted(&tb.node, &tb.span, env, ctx, tail_pos, None)?;
        // timeout_body deve produzir o mesmo tipo que os braços.
        if !type_compatible(&tb_typed.ty, &select_ty) {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{select_ty:?} (tipo do select)"),
                found: format!("{}", tb_typed.ty),
                span: tb.span.into(),
            });
        }
        typed_timeout_body = Some(Box::new(Spanned::new(tb_typed, tb.span)));
    }

    let escape = if ctx.ret_ty.is_some() {
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        EscapeTarget::Caller
    };

    Ok(TypedExpr {
        span: *span,
        ty: select_ty,
        tail_pos,
        escape,
        kind: TypedExprKind::Select {
            arms: typed_arms,
            timeout_ms: typed_timeout_ms,
            timeout_body: typed_timeout_body,
        },
    })
}

/// Verifica compatibilidade de tipos — estrutural para tipos concretos,
/// aceita Var como coringa (para type params não-resolvidos).
fn type_compatible(actual: &Ty, expected: &Ty) -> bool {
    if actual == expected {
        return true;
    }
    // Var unifica com qualquer tipo (mesma semântica de fits_return).
    matches!(actual, Ty::Var(_)) || matches!(expected, Ty::Var(_))
}

/// Escape target para `<!` — valor escapa para outro fiber.
///
/// Tipos compostos (Tuple, Struct, List, Array, Dict, Set, Text, etc.)
/// são alocados na arena e precisam sobreviver ao sender → `Caller`
/// (caller_arena = arena do pai direto do sender).
///
/// `Caller` é o LCA de sender e receiver porque canais só existem entre
/// pai-filho e irmãos (topologia enforced em compile-time). O pai só
/// morre depois de todos os filhos (structured concurrency), então a
/// caller_arena cobre o lifetime de ambos.
///
/// Tipos primitivos (Int/SMI, Float, Boolean, Unit) são inline (i64)
/// e não precisam de ARC → `Local` (sem overhead).
fn escape_for_channel_send(ty: &Ty, _tail_pos: bool, _ctx: &InferCtx) -> EscapeTarget {
    match ty {
        // Primitivos inline — sem alocação.
        Ty::Prim(_) | Ty::Unit => EscapeTarget::Local,
        // Action não pode viajar por canal (validado em infer_channel_send).
        Ty::Action(..) => EscapeTarget::Local,
        // Var/InferVar — conservador: Local (não sabemos o tipo concreto).
        Ty::Var(_) | Ty::InferVar(_) => EscapeTarget::Local,
        // Sender/Receiver são handles (i64), não ponteiros.
        Ty::Sender(_) | Ty::Receiver(_) => EscapeTarget::Local,
        // Function é fn_ptr, não ponteiro.
        Ty::Function(..) => EscapeTarget::Local,
        // Compostos — alocados na caller_arena, que sobrevive ao fiber
        // que os envia. O scheduler é structured concurrency: o pai só
        // morre depois de todas as filhas, então a caller_arena (arena
        // do pai) cobre o lifetime de todos os interessados no valor.
        _ => EscapeTarget::Caller,
    }
}
