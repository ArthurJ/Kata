//! `format!` builtin — interceptado no typeck como action call com `!`.
//!
//! `format!("template {} {}", (a, b))` sintetiza a cadeia (posicional):
//!   text_replace_first(
//!     text_replace_first("template {} {}", repr_a),
//!     repr_b
//!   )
//!
//! `format!{"{x} {y}", "x": a, "y": b}` sintetiza a cadeia (nomeado):
//!   text_replace_first(
//!     text_replace_first("{x} {y}", repr_a),
//!     repr_b
//!   )
//!
//! Para cada argumento, converte para Text (como `repr` faz) e substitui
//! a primeira ocorrência de `{}` (posicional) ou `{key}` (nomeado) no
//! template acumulado.

use kata_ast::{Expr, Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Pré-processa escape `{{` → `{` e `}}` → `}` em um template TextLit.
///
/// Regra: `{{` sempre vira `{` literal, `}}` sempre vira `}` literal,
/// `{key}` (chaves simples) é interpolação e permanece intacto.
///
/// Isto permite escrever `{{chave}}` para produzir o texto literal `{chave}`,
/// distinguindo de `{chave}` que seria interpolação da key `chave`.
fn preprocess_escape(text: &str) -> String {
    text.replace("{{", "\x00")
        .replace("}}", "\x01")
        .replace('\x00', "{")
        .replace('\x01', "}")
}

/// `format!` builtin — recebe o `args` cru do ActionCall.
///
/// Pode ser `Expr::Tuple`/`Expr::Grouping` (posicional) ou `Expr::DictLit`
/// (nomeado). O 1º elemento/sempre é o template (Text).
///
/// **Posicional**: `format!("tpl {} {}", (a, b))` — substitui `{}` na ordem.
/// **Nomeado**: `format!{"{x} {y}", "x": a, "y": b}` — substitui `{x}` pela
/// key `"x"` do dict.
pub(crate) fn infer_format_builtin(
    args: &Spanned<Expr>,
    _span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    match &args.node {
        Expr::DictLit { entries } => infer_format_named(args.span, entries, env, ctx),
        // Tupla posicional ou Grouping — extrai elements como antes.
        Expr::Tuple { .. } | Expr::Grouping { .. } | Expr::Unit => {
            let elements = extract_positional_elements(args);
            infer_format_positional(args.span, &elements, env, ctx)
        }
        // Expr única sem grouping — auto-wrap como tupla de 1.
        other => {
            let elements = vec![Spanned::new(other.clone(), args.span)];
            infer_format_positional(args.span, &elements, env, ctx)
        }
    }
}

/// Interpolação nomeada via DictLit.
///
/// `format!{"{x} {y}", "x": a, "y": b}` — o 1º entry é o template,
/// os demais são pares `"key": expr`.
fn infer_format_named(
    span: Span,
    entries: &[(Spanned<Expr>, Spanned<Expr>)],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    if entries.is_empty() {
        return Err(MiddleError::ArityMismatch {
            expected: 2,
            found: 0,
            span: span.into(),
            hint: Some("format! nomeado precisa de template + pelo menos 1 par key:value".into()),
        });
    }

    // 1º entry: template (key é descartada, value é o template Text).
    let (_tpl_key, tpl_val) = &entries[0];
    let template_expr = infer_expr(&tpl_val.node, &tpl_val.span, env, ctx, false)?;
    if template_expr.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", template_expr.ty),
            span: tpl_val.span.into(),
        });
    }
    // Pré-processa escape {{ }} se o template é TextLit.
    // {{ → { literal, }} → } literal, {key} (chaves simples) = interpolação.
    let template_expr = if let TypedExprKind::TextLit { text } = &template_expr.kind {
        let processed = preprocess_escape(text);
        TypedExpr {
            span: tpl_val.span,
            ty: Ty::text(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::TextLit { text: processed },
        }
    } else {
        template_expr
    };
    let mut result = Spanned::new(template_expr, tpl_val.span);

    // Demais entries: pares "key": expr.
    // Para cada par, converte expr para Text e substitui {key} no template.
    for (key, val) in &entries[1..] {
        // Key deve ser TextLit (string literal).
        let key_text = match &key.node {
            Expr::TextLit { text } => text.clone(),
            _ => {
                return Err(MiddleError::TypeMismatch {
                    expected: "Text literal como chave do dict".into(),
                    found: format!("{:?}", key.node),
                    span: key.span.into(),
                });
            }
        };

        // Converte value para Text.
        let typed_val = infer_expr(&val.node, &val.span, env, ctx, false)?;
        let text_val = convert_to_text(Spanned::new(typed_val, val.span));

        // Substitui {key} por text_val no template acumulado.
        // text_replace_first(result, text_val) — mas precisamos substituir
        // {key} específico, não {}. Como text_replace_first substitui a
        // primeira ocorrência de {}, e o template tem {key}, precisamos
        // primeiro substituir {key} por {} e depois {} por text_val.
        // Melhor: construir o placeholder "{key}" como Text e fazer
        // text_replace_first com o placeholder como "agulha" — mas
        // text_replace_first só substitui {}. Então precisamos de uma
        // abordagem diferente: substituir diretamente {key} por text_val.
        //
        // Como text_replace_first substitui a primeira ocorrência de {}
        // (literal), e o template nomeado tem {key} (não {}), precisamos
        // de uma FFI que substitua {key} específico.
        //
        // Alternativa: pré-processar o template em compile-time, trocando
        // {key} por {} na ordem das entries, e usar text_replace_first
        // posicional. Mas isso requer que o template seja TextLit.
        //
        // Como o template geralmente é TextLit (string literal), podemos
        // pré-processar em compile-time.
        let placeholder = format!("{{{key_text}}}");
        // Constrói text_replace_first com placeholder como "agulha" —
        // mas text_replace_first só substitui {}. Então precisamos de
        // text_replace(template, placeholder, replacement) — 3 args.
        //
        // Por ora, usar a abordagem de pré-processamento do template:
        // se o template é TextLit, reescrever {key} → {} na ordem.
        result = text_replace_named(result, &placeholder, text_val);
    }

    Ok((Ty::text(), result.node.kind))
}

/// Interpolação posicional via tupla.
///
/// `format!("tpl {} {}", (a, b))` — substitui `{}` na ordem dos elements.
fn infer_format_positional(
    span: Span,
    elements: &[Spanned<Expr>],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    if elements.is_empty() {
        return Err(MiddleError::ArityMismatch {
            expected: 2,
            found: 0,
            span: span.into(),
            hint: Some("format! precisa de template + args".into()),
        });
    }

    // Arg 0: template (deve ser Text)
    let template_expr = infer_expr(&elements[0].node, &elements[0].span, env, ctx, false)?;
    if template_expr.ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", Ty::text()),
            found: format!("{}", template_expr.ty),
            span: elements[0].span.into(),
        });
    }
    // Pré-processa escape {{ }} se o template é TextLit.
    let template_expr = if let TypedExprKind::TextLit { text } = &template_expr.kind {
        let processed = preprocess_escape(text);
        TypedExpr {
            span: elements[0].span,
            ty: Ty::text(),
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            kind: TypedExprKind::TextLit { text: processed },
        }
    } else {
        template_expr
    };
    let mut result = Spanned::new(template_expr, elements[0].span);

    // Demais elements: valores a interpolar posicionalmente.
    for elem in &elements[1..] {
        let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
        let text_typed = convert_to_text(Spanned::new(typed, elem.span));
        result = text_replace_first(result, text_typed);
    }

    Ok((Ty::text(), result.node.kind))
}

/// Extrai elements de tupla, Grouping, ou auto-wrap de expr única.
/// Trata `()` (Unit) como tupla vazia — `format!("tpl", ())` = sem args.
/// Trata `(arg,)` (tupla de 1) como auto-wrap — o arg é o valor, não uma tupla.
fn extract_positional_elements(args: &Spanned<Expr>) -> Vec<Spanned<Expr>> {
    match &args.node {
        Expr::Tuple { elements } => extract_tuple_elems(elements),
        Expr::Unit => Vec::new(),
        Expr::Grouping { inner } => match &inner.node {
            Expr::Tuple { elements } => extract_tuple_elems(elements),
            // Grouping de expr única — auto-wrap como tupla de 1.
            _ => vec![Spanned::new(inner.node.clone(), inner.span)],
        },
        other => vec![Spanned::new(other.clone(), args.span)],
    }
}

/// Extrai elements de uma tupla, tratando casos especiais:
/// - `("tpl", ())` — 2 elems onde 2º é Unit = template sem args de interpolação.
/// - `("tpl", (arg,))` — 2 elems onde 2º é Tuple de 1 = template + 1 arg (auto-wrap).
fn extract_tuple_elems(elements: &[Spanned<Expr>]) -> Vec<Spanned<Expr>> {
    if elements.len() == 2 {
        // `("tpl", ())` — sem args.
        if matches!(elements[1].node, Expr::Unit) {
            return vec![elements[0].clone()];
        }
        // `("tpl", (arg,))` — auto-wrap de tupla de 1.
        if let Expr::Tuple { elements: inner } = &elements[1].node
            && inner.len() == 1
        {
            return vec![elements[0].clone(), inner[0].clone()];
        }
    }
    elements.to_vec()
}

/// Converte um TypedExpr para Text, baseado no tipo.
///
/// - Text: identity
/// - Int: int_to_text (FFI)
/// - Boolean: bool_to_text (FFI)
/// - Struct/Enum: repr (call direto via ffi_symbol mangled)
/// - Tuple/List/Outros compostos: `show` genérico (sem ffi_symbol) —
///   o monomorphizador resolve o overload e instancia para o tipo concreto.
///   Isto garante que `format "..." (_args,)` onde `_args` é `Tuple(Int)`
///   não crash com SIGSEGV tratando o ponteiro da tupla como Int.
fn convert_to_text(expr: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    let ty = &expr.node.ty;
    match ty {
        Ty::Prim(PrimTy::Text) => expr,
        Ty::Prim(PrimTy::Int) => ffi_call1("kata_rt_int_to_text", expr, Ty::text()),
        Ty::Prim(PrimTy::Rational) => ffi_call1("kata_rt_rat_show", expr, Ty::text()),
        Ty::Prim(PrimTy::Float) => {
            // Sem FFI de float_to_text — fallback para int_to_text
            ffi_call1("kata_rt_int_to_text", expr, Ty::text())
        }
        Ty::Sum(name) if name == "Boolean" => {
            // Boolean ganha `show` sintetizado (variantes True/False).
            let mangled = format!("__kata_show__{name}");
            repr_call(expr, mangled)
        }
        Ty::Sum(name) => {
            // Enum — chama `__kata_show__{name}` via ffi_symbol mangled.
            let mangled = format!("__kata_show__{name}");
            repr_call(expr, mangled)
        }
        Ty::Struct(key) => {
            // Struct — chama `__kata_show__{name}` via ffi_symbol mangled.
            let mangled = format!("__kata_show__{}", key.name());
            repr_call(expr, mangled)
        }
        // Tipos compostos (Tuple, List, etc.) não têm overload concreto
        // no DispatchTable na inference. Gerar `show` genérico (callee =
        // Ident("show"), ffi_symbol = None) — o monomorphizador resolve.
        Ty::Tuple(_) | Ty::List(_) | Ty::Array(_) | Ty::Generic(..) => show_generic_call(expr),
        _ => ffi_call1("kata_rt_int_to_text", expr, Ty::text()),
    }
}

/// Constrói `show <expr>` como Closure genérica (sem ffi_symbol).
/// O monomorphizador encontra o overload `show` na DispatchTable,
/// instancia para o tipo concreto, e reescreve o callee.
/// Para Tuple, `tuple_show.rs` no monomorph sintetiza a árvore de
/// string_concat acessando cada elemento.
fn show_generic_call(arg: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "show".to_string(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![arg],
                ffi_symbol: None,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `Closure { callee=Ident(ffi), args=[arg], ffi_symbol=Some(ffi) }`.
fn ffi_call1(ffi_name: &str, arg: Spanned<TypedExpr>, ret_ty: Ty) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(ret_ty.clone())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: ffi_name.to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: ret_ty,
            tail_pos: false,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![arg],
                ffi_symbol: Some(ffi_name.to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `text_replace(template, needle, replacement)` — FFI call 3-arg.
/// Substitui a primeira ocorrência de `needle` por `replacement` no template.
fn text_replace_named(
    template: Spanned<TypedExpr>,
    needle: &str,
    replacement: Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    // Aloca o needle como string literal no TAST.
    let needle_typed = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::TextLit {
            text: needle.to_string(),
        },
    };

    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(
            vec![Ty::text(), Ty::text(), Ty::text()],
            Box::new(Ty::text()),
        ),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_text_replace".to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![
                    template,
                    Spanned::new(needle_typed, Span::synthetic()),
                    replacement,
                ],
                ffi_symbol: Some("kata_rt_text_replace".to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `text_replace_first(template, replacement)` — FFI call binário.
fn text_replace_first(
    template: Spanned<TypedExpr>,
    replacement: Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::text(), Ty::text()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_text_replace_first".to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![template, replacement],
                ffi_symbol: Some("kata_rt_text_replace_first".to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói chamada a `repr` (call direto via ffi_symbol mangled).
fn repr_call(field_access: Spanned<TypedExpr>, mangled: String) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![field_access.node.ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: mangled.clone(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![field_access],
                ffi_symbol: Some(mangled),
            },
        },
        Span::synthetic(),
    )
}
