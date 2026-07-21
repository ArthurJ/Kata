//! Helpers de construção de TypedExpr para a síntese de `show`.
//!
//! Funções puras que fabricam nós TAST (TextLit, Closure, FieldAccess,
//! string_concat, ffi_call1) usadas por `show_synthesis`. Separadas para
//! reduzir o tamanho de `show_synthesis.rs` sem alterar responsabilidade.

use kata_ast::{Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty};

use crate::typed::{Effect, TypedExpr, TypedExprKind};

/// Produz uma expressão `show <expr>` que despacha para a implementação
/// correta de SHOW do tipo. Usado dentro de `show` sintetizado para enums
/// (para mostrar o payload da variante).
///
/// Para tipos com `show` sintetizado (Struct, Sum), chama o mangled.
/// Para primitivos (Int, Float, Rational, Text), chama a FFI direto.
/// Para `Ty::Var` (enum genérico) onde o type param foi resolvido para
/// um tipo concreto, despacha via iface method (Caminho 0).
/// Para `Ty::Var` onde o type param não foi resolvido (ex: `E` em
/// `Result::Ok 42` — o `Err` nunca é usado), produz o fallback `"?"
/// — não há tipo concreto para despachar.
pub(crate) fn show_expr(arg: Spanned<TypedExpr>, arg_ty: &Ty) -> Spanned<TypedExpr> {
    match arg_ty {
        Ty::Prim(PrimTy::Int) => ffi_call1("kata_rt_bi_show", arg, Ty::text()),
        Ty::Prim(PrimTy::Float) => ffi_call1("kata_rt_float_to_text", arg, Ty::text()),
        Ty::Prim(PrimTy::Rational) => ffi_call1("kata_rt_rat_show", arg, Ty::text()),
        Ty::Prim(PrimTy::Text) => arg, // identity
        Ty::Sum(name) => show_call(arg, name.clone(), arg_ty),
        Ty::Struct(name) => show_call(arg, name.clone(), arg_ty),
        Ty::List(_) => show_call(arg, "List".to_string(), arg_ty),
        Ty::Var(name) => {
            // Ty::Var em enum genérico. Produz `show v` com ffi_symbol: None.
            // O monomorphizador, ao instanciar a função sintetizada, substitui
            // Var("T") → tipo concreto (ex: Int) via apply_subs. O Layer 5
            // (resolução de ffi_symbol post-instantiation) encontra o overload
            // concreto de `show` para o tipo resolvido e preenche ffi_symbol.
            // Se o type param não for resolvido (ex: `E` em `Result::Ok 42`),
            // o Layer 5 não encontra overload e o ffi_symbol fica None —
            // o codegen produz erro gracioso (não SIGSEGV).
            let _ = name;
            let callee_ty = Ty::Function(vec![arg_ty.clone()], Box::new(Ty::text()));
            let callee = TypedExpr {
                span: Span::synthetic(),
                ty: callee_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                effect: Effect::Puro,
                kind: TypedExprKind::Ident {
                    name: "show".to_string(),
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
                        ffi_symbol: None,
                    },
                },
                Span::synthetic(),
            )
        }
        _ => {
            // Fallback para tipos não cobertos — int_to_text como saída segura.
            ffi_call1("kata_rt_int_to_text", arg, Ty::text())
        }
    }
}

/// Constrói chamada a `__kata_show__{Type}` mangled (call direto via ffi_symbol).
pub(crate) fn show_call(
    arg: Spanned<TypedExpr>,
    type_name: String,
    arg_ty: &Ty,
) -> Spanned<TypedExpr> {
    let mangled = format!("__kata_show__{type_name}");
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg_ty.clone()], Box::new(Ty::text())),
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

/// Constrói `FieldAccess { expr: __self, field_index }`.
pub(crate) fn field_access_expr(field_index: usize, field_ty: &Ty) -> Spanned<TypedExpr> {
    let self_expr = TypedExpr {
        span: Span::synthetic(),
        ty: field_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        effect: Effect::Puro,
        kind: TypedExprKind::Ident {
            name: "__self".to_string(),
        },
    };
    let self_spanned = Spanned::new(self_expr, Span::synthetic());

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: field_ty.clone(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::FieldAccess {
                expr: Box::new(self_spanned),
                struct_name: String::new(),
                field_name: String::new(),
                field_index: field_index as u32,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `TextLit(text)`.
pub(crate) fn text_lit(text: String) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::text(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            effect: Effect::Puro,
            kind: TypedExprKind::TextLit { text },
        },
        Span::synthetic(),
    )
}

/// Constrói `Closure { callee=Ident(ffi), args=[arg], ffi_symbol=Some(ffi) }`.
pub(crate) fn ffi_call1(ffi_name: &str, arg: Spanned<TypedExpr>, ret_ty: Ty) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![arg.node.ty.clone()], Box::new(ret_ty.clone())),
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
            ty: ret_ty,
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

/// Constrói `string_concat(left, right)` — FFI call binário.
pub(crate) fn string_concat(
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
