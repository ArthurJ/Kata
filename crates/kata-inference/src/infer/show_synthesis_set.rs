//! Síntese de `show` para `Set::A` — hash set persistente (HAMT-backed).
//!
//! Gera duas TypedFunction genéricas mutuamente recursivas:
//!
//! - `__kata_show__Set :: Set::A => Text`
//!   - `len == 0` → `"{|}"`
//!   - `len > 0`  → `"{|" + repr(next(set, 0)) + __kata_show__Set_rest(set, 1)`
//!
//! - `__kata_show__Set_rest :: Set::A Int => Text`
//!   - `i == len` → `"|}"`
//!   - `i < len`  → `", " + repr(next(set, i)) + __kata_show__Set_rest(set, i+1)`
//!
//! Iteração via `kata_rt_set_next(set_ptr, iter_state, arena)` — arena
//! auto-injetada pelo codegen (ffi_needs_arena). `iter_state` é parâmetro
//! explícito (0=init, N=Nth key, N>=count=None). Retorna `Optional::K`
//! como Sum box: tag=0 Some(key), tag=1 None.
//!
//! `=` e `+` são Closures genéricas (ffi_symbol: None) — o monomorphizador
//! resolve via DispatchTable.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::{ImplEntry, ImplMethodInfo, InterfaceRegistry};
use kata_core::ty::Ty;

use crate::typed::{
    TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedMatchArm, TypedPattern,
};

use super::show_synthesis_helpers::{ffi_call1, repr_expr, string_concat, text_lit};

/// Sintetiza `show` para `Set::A`.
pub(crate) fn synthesize_set_show_functions(
    dispatch_table: &mut DispatchTable,
    interface_registry: &mut InterfaceRegistry,
) -> Vec<TypedFunction> {
    let type_param = "A";
    let elem_ty = Ty::Var(type_param.to_string());
    let set_ty = Ty::Set(Box::new(elem_ty.clone()));
    let ret_ty = Ty::text();
    let int_ty = Ty::int();

    // ── show :: Set::A => Text ──
    dispatch_table.insert(OverloadInfo {
        name: "show".to_string(),
        params: vec![set_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some("__kata_show__Set".to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![type_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── __kata_show__Set_rest :: Set::A Int => Text ──
    let rest_name = "__kata_show__Set_rest";
    dispatch_table.insert(OverloadInfo {
        name: rest_name.to_string(),
        params: vec![set_ty.clone(), int_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some(rest_name.to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![type_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── Set implements SHOW ──
    interface_registry
        .register_impl(ImplEntry {
            origin: "__synthesis".to_string(),
            type_name: "Set".to_string(),
            type_params: vec![type_param.to_string()],
            interface_name: "SHOW".to_string(),
            iface_params: vec![],
            methods: vec![ImplMethodInfo {
                name: "show".to_string(),
                params: vec![set_ty.clone()],
                ret: ret_ty.clone(),
                ffi_symbol: Some("__kata_show__Set".to_string()),
            }],
        })
        .ok();

    let show_set = build_show_func("__kata_show__Set", &set_ty, &ret_ty, &elem_ty, true);
    let show_rest = build_show_func(rest_name, &set_ty, &ret_ty, &elem_ty, false);

    vec![show_set, show_rest]
}

// ════════════════════════════════════════════════════════════════
// Construção das TypedFunctions
// ════════════════════════════════════════════════════════════════

fn build_show_func(
    func_name: &str,
    set_ty: &Ty,
    ret_ty: &Ty,
    elem_ty: &Ty,
    is_main: bool,
) -> TypedFunction {
    let (param_tys, patterns, body) = if is_main {
        let body = build_main_body(set_ty, elem_ty);
        (
            vec![set_ty.clone()],
            vec![ident_pattern("__self", set_ty)],
            body,
        )
    } else {
        let body = build_rest_body(set_ty, elem_ty);
        (
            vec![set_ty.clone(), Ty::int()],
            vec![
                ident_pattern("__self", set_ty),
                ident_pattern("i", &Ty::int()),
            ],
            body,
        )
    };

    TypedFunction {
        name: func_name.to_string(),
        param_types: param_tys,
        ret_ty: ret_ty.clone(),
        clauses: vec![TypedLambdaClause {
            synthetic_pre: Vec::new(),
            synthetic_post: Vec::new(),
            patterns,
            body: Spanned::new(body, Span::synthetic()),
            guards: Vec::new(),
            with_bindings: Vec::new(),
        }],
        cache_spec: None,
        timer_spec: None,
    }
}

// ════════════════════════════════════════════════════════════════
// Bodies
// ════════════════════════════════════════════════════════════════

/// `__kata_show__Set :: Set::A => Text`
///
/// ```text
/// match (= (len __self) 0)
///     True: "{|}"
///     False:
///         match (kata_rt_set_next __self 0)
///             Some(h): "{|" + repr(h) + rest(__self, 1)
///             None: "{|}"   -- não deve acontecer (len > 0)
/// ```
fn build_main_body(set_ty: &Ty, elem_ty: &Ty) -> TypedExpr {
    let self_spanned = self_expr(set_ty);
    let len_call = ffi_call1("kata_rt_set_len", self_spanned.clone(), Ty::int());
    let eq_scrutinee = eq_closure(len_call, int_lit_expr(0));

    // Braço True: "{|}"
    let true_arm = boolean_arm("True", 0, text_lit("{|}".to_string()));

    // Braço False: match (set_next __self 0) { Some(h): ... ; None: "{|}" }
    let next_call = set_next_call(self_spanned.clone(), int_lit_expr(0));

    let h_expr = TypedExpr {
        span: Span::synthetic(),
        ty: elem_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "h".to_string(),
        },
    };
    let h_spanned = Spanned::new(h_expr, Span::synthetic());
    let repr_h = repr_expr(h_spanned, elem_ty);
    let rest_call = rest_call_expr(self_spanned, int_lit_expr(1), set_ty);
    let ok_body = string_concat(text_lit("{|".to_string()), string_concat(repr_h, rest_call));

    let some_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Optional".to_string(),
                variant: "Some".to_string(),
                sub_patterns: Some(vec![Spanned::new(
                    TypedPattern::Ident {
                        name: "h".to_string(),
                        ty: elem_ty.clone(),
                    },
                    Span::synthetic(),
                )]),
                tag: 0,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: ok_body,
    };
    let none_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Optional".to_string(),
                variant: "None".to_string(),
                sub_patterns: None,
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: text_lit("{|}".to_string()),
    };

    let inner_match = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(next_call),
            arms: vec![some_arm, none_arm],
        },
    };

    let false_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".to_string(),
                variant: "False".to_string(),
                sub_patterns: None,
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: Spanned::new(inner_match, Span::synthetic()),
    };

    TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(eq_scrutinee),
            arms: vec![true_arm, false_arm],
        },
    }
}

/// `__kata_show__Set_rest :: Set::A Int => Text`
///
/// ```text
/// match (= i (len __self))
///     True: "|}"
///     False:
///         match (kata_rt_set_next __self i)
///             Some(h): ", " + repr(h) + rest(__self, + i 1)
///             None: "|}"
/// ```
fn build_rest_body(set_ty: &Ty, elem_ty: &Ty) -> TypedExpr {
    let self_spanned = self_expr(set_ty);
    let i_spanned = ident_expr("i", &Ty::int());
    let len_call = ffi_call1("kata_rt_set_len", self_spanned.clone(), Ty::int());
    let eq_scrutinee = eq_closure(i_spanned.clone(), len_call);

    let true_arm = boolean_arm("True", 0, text_lit("|}".to_string()));

    // False: match (set_next __self i) { Some(h): ... ; None: "|}" }
    let next_call = set_next_call(self_spanned.clone(), i_spanned.clone());

    let h_expr = TypedExpr {
        span: Span::synthetic(),
        ty: elem_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "h".to_string(),
        },
    };
    let h_spanned = Spanned::new(h_expr, Span::synthetic());
    let repr_h = repr_expr(h_spanned, elem_ty);

    // + i 1
    let plus_call = plus_closure(i_spanned, int_lit_expr(1));
    let rest_call = rest_call_expr(self_spanned, plus_call, set_ty);
    let ok_body = string_concat(text_lit(", ".to_string()), string_concat(repr_h, rest_call));

    let some_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Optional".to_string(),
                variant: "Some".to_string(),
                sub_patterns: Some(vec![Spanned::new(
                    TypedPattern::Ident {
                        name: "h".to_string(),
                        ty: elem_ty.clone(),
                    },
                    Span::synthetic(),
                )]),
                tag: 0,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: ok_body,
    };
    let none_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Optional".to_string(),
                variant: "None".to_string(),
                sub_patterns: None,
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: text_lit("|}".to_string()),
    };

    let inner_match = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(next_call),
            arms: vec![some_arm, none_arm],
        },
    };

    let false_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".to_string(),
                variant: "False".to_string(),
                sub_patterns: None,
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: Spanned::new(inner_match, Span::synthetic()),
    };

    TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(eq_scrutinee),
            arms: vec![true_arm, false_arm],
        },
    }
}

// ════════════════════════════════════════════════════════════════
// Helpers de construção de TypedExpr
// ════════════════════════════════════════════════════════════════

fn self_expr(ty: &Ty) -> Spanned<TypedExpr> {
    ident_expr("__self", ty)
}

fn ident_expr(name: &str, ty: &Ty) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: ty.clone(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::Ident {
                name: name.to_string(),
            },
        },
        Span::synthetic(),
    )
}

fn int_lit_expr(n: i64) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::int(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::IntLit {
                text: n.to_string(),
            },
        },
        Span::synthetic(),
    )
}

fn ident_pattern(name: &str, ty: &Ty) -> Spanned<TypedPattern> {
    Spanned::new(
        TypedPattern::Ident {
            name: name.to_string(),
            ty: ty.clone(),
        },
        Span::synthetic(),
    )
}

fn boolean_arm(variant: &str, tag: usize, body: Spanned<TypedExpr>) -> TypedMatchArm {
    TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Boolean".to_string(),
                variant: variant.to_string(),
                sub_patterns: None,
                tag,
            },
            Span::synthetic(),
        )),
        guard: None,
        body,
    }
}

/// Constrói `= a b` como Closure genérica (ffi_symbol: None).
fn eq_closure(lhs: Spanned<TypedExpr>, rhs: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    generic_op_closure("=", lhs, rhs, Ty::int(), Ty::Sum("Boolean".to_string()))
}

/// Constrói `+ a b` como Closure genérica (ffi_symbol: None).
fn plus_closure(lhs: Spanned<TypedExpr>, rhs: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    generic_op_closure("+", lhs, rhs, Ty::int(), Ty::int())
}

fn generic_op_closure(
    op: &str,
    lhs: Spanned<TypedExpr>,
    rhs: Spanned<TypedExpr>,
    arg_ty: Ty,
    ret_ty: Ty,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(
            vec![arg_ty.clone(), arg_ty.clone()],
            Box::new(ret_ty.clone()),
        ),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: op.to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: ret_ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![lhs, rhs],
                ffi_symbol: None,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `kata_rt_set_next(set_ptr, iter_state)` — FFI call.
/// O codegen injeta arena automaticamente (ffi_needs_arena).
/// Retorna Optional Sum box (i64).
fn set_next_call(set: Spanned<TypedExpr>, iter_state: Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    // O tipo de retorno é Optional::A — representado como Sum box i64.
    // O codegen trata FFIs como retornando i64.
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_set_next".to_string(),
        },
    };

    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::int(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::Closure {
                callee: Box::new(Spanned::new(callee, Span::synthetic())),
                args: vec![set, iter_state],
                ffi_symbol: Some("kata_rt_set_next".to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `__kata_show__Set_rest __self idx` — chamada para a função rest.
fn rest_call_expr(
    self_spanned: Spanned<TypedExpr>,
    idx: Spanned<TypedExpr>,
    set_ty: &Ty,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![set_ty.clone(), Ty::int()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "__kata_show__Set_rest".to_string(),
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
                args: vec![self_spanned, idx],
                ffi_symbol: Some("__kata_show__Set_rest".to_string()),
            },
        },
        Span::synthetic(),
    )
}
