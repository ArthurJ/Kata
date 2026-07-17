//! Builders de TypedExpr para smart constructors falíveis de tipos refinados.
//!
//! Construtores de `Result::Ok(v)`, `Result::Err(msg)` (com `Show(v)` via FFI),
//! e match aninhado sobre os predicados. Usados exclusivamente por
//! `synthesize_refined` em `constructors_refined.rs`.

use kata_ast::{Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty};

use crate::typed::{Effect, TypedExpr, TypedExprKind, TypedMatchArm, TypedPattern};

/// Constrói `Result::Ok(v)` como `TypedExpr` com tipo `result_ty`.
pub(crate) fn build_result_ok(
    var_name: &str,
    base_ty: &Ty,
    _type_name: &str,
    result_ty: &Ty,
) -> Spanned<TypedExpr> {
    let payload = TypedExpr {
        span: Span::synthetic(),
        ty: base_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: var_name.into(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: result_ty.clone(),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::VariantConstruct {
                enum_name: "Result".into(),
                variant: "Ok".into(),
                payload: Box::new(Spanned::new(payload, Span::synthetic())),
                tag: 0,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `Result::Err(msg)` onde `msg = StringConcat(Show(v), TextLit(...))`.
///
/// A mensagem é: `"{v} falhou no predicado {pred_str} na construção do {type_name}"`.
/// `Show(v)` despacha via FFI conforme o tipo base (Int→bi_show, Float→float_to_text, Rational→rat_show).
pub(crate) fn build_result_err(
    var_name: &str,
    base_ty: &Ty,
    pred_str: &str,
    type_name: &str,
    result_ty: &Ty,
) -> Spanned<TypedExpr> {
    let suffix = format!(" falhou no predicado {pred_str} na construção do {type_name}");

    // Show(v) — chama o FFI apropriado para converter v em Text.
    let shown = build_show_call(var_name, base_ty);

    // TextLit do sufixo.
    let suffix_lit = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::TextLit { text: suffix },
    };

    // StringConcat(Show(v), suffix_lit) → Text.
    let concat = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Closure {
            callee: Box::new(Spanned::new(
                TypedExpr {
                    span: Span::synthetic(),
                    ty: Ty::Function(vec![Ty::text(), Ty::text()], Box::new(Ty::text())),
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    effect: Effect::Puro,
                    kind: TypedExprKind::Ident {
                        name: "kata_rt_string_concat".into(),
                    },
                },
                Span::synthetic(),
            )),
            args: vec![shown, Spanned::new(suffix_lit, Span::synthetic())],
            ffi_symbol: Some("kata_rt_string_concat".into()),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: result_ty.clone(),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::VariantConstruct {
                enum_name: "Result".into(),
                variant: "Err".into(),
                payload: Box::new(Spanned::new(concat, Span::synthetic())),
                tag: 1,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `Show(v)` — converte o valor `v` do tipo base para Text via FFI.
///
/// Int → `kata_rt_bi_show`, Float → `kata_rt_float_to_text`, Rational → `kata_rt_rat_show`.
fn build_show_call(var_name: &str, base_ty: &Ty) -> Spanned<TypedExpr> {
    let (ffi_name, param_clif) = match base_ty {
        Ty::Prim(PrimTy::Int) => ("kata_rt_bi_show", false),
        Ty::Prim(PrimTy::Float) => ("kata_rt_float_to_text", true),
        Ty::Prim(PrimTy::Rational) => ("kata_rt_rat_show", false),
        _ => ("kata_rt_bi_show", false), // fallback defensivo
    };
    let _ = param_clif;

    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![base_ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: ffi_name.into(),
        },
    };
    let arg = TypedExpr {
        span: Span::synthetic(),
        ty: base_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: var_name.into(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![Spanned::new(arg, Span::synthetic())],
                ffi_symbol: Some(ffi_name.into()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói um match aninhado sobre os predicados.
///
/// `pred_calls[0]` é o scrutinee do match mais externo.
/// Se True → recursa para `pred_calls[1..]`.
/// Se False → `err_bodies[0]`.
///
/// No nível mais profundo (sem mais predicados) → `ok_body`.
pub(crate) fn build_nested_match(
    pred_calls: &[Spanned<TypedExpr>],
    err_bodies: &[Spanned<TypedExpr>],
    ok_body: Spanned<TypedExpr>,
    result_ty: &Ty,
) -> Spanned<TypedExpr> {
    if pred_calls.is_empty() {
        return ok_body;
    }

    let scrutinee = pred_calls[0].clone();
    let err_body = err_bodies[0].clone();

    let inner = build_nested_match(&pred_calls[1..], &err_bodies[1..], ok_body, result_ty);

    // Arm True: recursa para o próximo predicado
    let true_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".into(),
                variant: "True".into(),
                sub_patterns: None,
                tag: 0,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: inner,
    };

    // Arm False: erro
    let false_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".into(),
                variant: "False".into(),
                sub_patterns: None,
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: err_body,
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: result_ty.clone(),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            effect: Effect::Puro,
            kind: TypedExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms: vec![true_arm, false_arm],
            },
        },
        Span::synthetic(),
    )
}
