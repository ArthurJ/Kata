//! Síntese inline de `show` para `Tuple` no monomorphizador.
//!
//! Quando o monomorphizador encontra uma `Closure { callee: Ident("show"),
//! ffi_symbol: None, args: [tuple_expr] }` onde o tipo do arg é `Ty::Tuple`,
//! não há overload concreto no DispatchTable (Tuple não registra show).
//! Este módulo substitui a Closure inteira por uma árvore de `string_concat`
//! que acessa cada elemento via `FieldAccess` e despacha `show` para o tipo
//! concreto de cada elemento.
//!
//! A substituição acontece em `rewrite_typed_expr` (Layer 6), após
//! `resolve_erased_ffi_symbol` (Layer 5) falhar em encontrar o overload.

use kata_ast::{Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::Ty;
use kata_inference::{Effect, TypedExpr, TypedExprKind};

/// Substitui `show (tuple_expr)` por:
/// `string_concat("(", string_concat(show tuple.0, string_concat(", ",
///   string_concat(show tuple.1, ")"))))`
///
/// `tuple_expr` já tem tipo `Ty::Tuple(element_tys)` — os tipos são concretos
/// após a instanciação (o monomorphizador já aplicou `apply_subs`).
pub(crate) fn rewrite_show_tuple_call(
    tuple_expr: &Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    let element_tys = match &tuple_expr.node.ty {
        Ty::Tuple(tys) => tys.clone(),
        _ => {
            // Não deveria acontecer — o caller verifica Ty::Tuple.
            return Spanned::new(
                TypedExpr {
                    span: tuple_expr.span,
                    ty: Ty::text(),
                    tail_pos: false,
                    escape: EscapeTarget::Local,
                    effect: Effect::Puro,
                    kind: TypedExprKind::TextLit {
                        text: "?".to_string(),
                    },
                },
                tuple_expr.span,
            );
        }
    };

    // Tupla vazia: "()"
    if element_tys.is_empty() {
        return Spanned::new(
            TypedExpr {
                span: Span::synthetic(),
                ty: Ty::text(),
                tail_pos: false,
                escape: EscapeTarget::Ancestor(0),
                effect: Effect::Puro,
                kind: TypedExprKind::TextLit {
                    text: "()".to_string(),
                },
            },
            Span::synthetic(),
        );
    }

    let mut parts: Vec<Spanned<TypedExpr>> = Vec::new();
    parts.push(text_lit("("));

    for (i, elem_ty) in element_tys.iter().enumerate() {
        if i > 0 {
            parts.push(text_lit(", "));
        }
        // FieldAccess: tuple_expr.i — carrega o elemento i da tupla.
        let access = TypedExpr {
            span: Span::synthetic(),
            ty: elem_ty.clone(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::FieldAccess {
                expr: Box::new(tuple_expr.clone()),
                struct_name: String::new(),
                field_name: String::new(),
                field_index: i as u32,
            },
        };
        let access_spanned = Spanned::new(access, Span::synthetic());
        parts.push(show_for_type(access_spanned, elem_ty));
    }

    parts.push(text_lit(")"));

    let result = parts.into_iter().reduce(string_concat);
    result.expect("tuple show tem pelo menos 2 parts (abre + fecha)")
}

/// Despacha `show` para o tipo concreto do elemento.
///
/// Para primitivos (Int, Float, Rational, Text), chama a FFI direto.
/// Para Sum/Struct/List, chama o mangled `__kata_show__{Type}`.
/// Para Tuple (aninhada), recursa via `rewrite_show_tuple_call`.
fn show_for_type(arg: Spanned<TypedExpr>, elem_ty: &Ty) -> Spanned<TypedExpr> {
    use kata_core::ty::PrimTy;
    match elem_ty {
        Ty::Prim(PrimTy::Int) => ffi_call1("kata_rt_bi_show", arg),
        Ty::Prim(PrimTy::Float) => ffi_call1("kata_rt_float_to_text", arg),
        Ty::Prim(PrimTy::Rational) => ffi_call1("kata_rt_rat_show", arg),
        Ty::Prim(PrimTy::Text) => arg, // identity
        Ty::Sum(name) => show_call_mangled(arg, name),
        Ty::Struct(name) => show_call_mangled(arg, name),
        Ty::List(_) => show_call_mangled(arg, "List"),
        Ty::Tuple(_) => rewrite_show_tuple_call(&arg),
        Ty::Generic(name, _) => show_call_mangled(arg, name),
        _ => ffi_call1("kata_rt_int_to_text", arg), // fallback gracoso
    }
}

// ── Helpers de construção de TypedExpr ──────────────────────────────

fn text_lit(text: &str) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::TextLit {
                text: text.to_string(),
            },
        },
        Span::synthetic(),
    )
}

fn ffi_call1(ffi_name: &str, arg: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: ffi_name.to_string(),
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
                args: vec![arg],
                ffi_symbol: Some(ffi_name.to_string()),
            },
        },
        Span::synthetic(),
    )
}

fn show_call_mangled(arg: Spanned<TypedExpr>, type_name: &str) -> Spanned<TypedExpr> {
    let mangled = format!("__kata_show__{}", type_name);
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: mangled.clone(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Ancestor(0),
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![arg],
                ffi_symbol: Some(mangled),
            },
        },
        Span::synthetic(),
    )
}

fn string_concat(
    left: Spanned<TypedExpr>,
    right: Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::text(), Ty::text()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "kata_rt_string_concat".to_string(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Ancestor(0),
            effect: Effect::Puro,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![left, right],
                ffi_symbol: Some("kata_rt_string_concat".to_string()),
            },
        },
        Span::synthetic(),
    )
}