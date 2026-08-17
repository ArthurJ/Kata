//! Síntese de `show` para `Array::A` — vetor contíguo indexável.
//!
//! Gera duas TypedFunction genéricas mutuamente recursivas:
//!
//! - `__kata_show__Array :: Array::A => Text`
//!   - `len == 0` → `"[]"`
//!   - `len > 0`  → `"[" + repr(at(arr, 0)) + __kata_show__Array_rest(arr, 1)`
//!
//! - `__kata_show__Array_rest :: Array::A Int => Text`
//!   - `i == len` → `"]"`
//!   - `i < len`  → `", " + repr(at(arr, i)) + __kata_show__Array_rest(arr, i+1)`
//!
//! `=` e `+` são gerados como Closures genéricas (ffi_symbol: None) — o
//! monomorphizador resolve via DispatchTable. `kata_rt_array_get_checked`
//! retorna um Result Sum; fazemos Match aninhado para extrair o Ok.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::interface_registry::{ImplEntry, ImplMethodInfo, InterfaceRegistry};
use kata_core::ty::Ty;

use crate::typed::{
    TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedMatchArm, TypedPattern,
};

use super::show_synthesis_helpers::{ffi_call1, repr_expr, string_concat, text_lit};

/// Sintetiza `show` para `Array::A`.
pub(crate) fn synthesize_array_show_functions(
    dispatch_table: &mut DispatchTable,
    interface_registry: &mut InterfaceRegistry,
) -> Vec<TypedFunction> {
    let type_param = "A";
    let elem_ty = Ty::Var(type_param.to_string());
    let array_ty = Ty::Array(Box::new(elem_ty.clone()));
    let ret_ty = Ty::text();
    let int_ty = Ty::int();
    let bool_ty = Ty::Sum("Boolean".to_string());

    // ── show :: Array::A => Text ──
    dispatch_table.insert(OverloadInfo {
        name: "show".to_string(),
        params: vec![array_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some("__kata_show__Array".to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![type_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── __kata_show__Array_rest :: Array::A Int => Text ──
    let rest_name = "__kata_show__Array_rest";
    dispatch_table.insert(OverloadInfo {
        name: rest_name.to_string(),
        params: vec![array_ty.clone(), int_ty.clone()],
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

    // ── Array implements SHOW ──
    interface_registry
        .register_impl(ImplEntry {
            origin: "__synthesis".to_string(),
            type_name: "Array".to_string(),
            type_params: vec![type_param.to_string()],
            interface_name: "SHOW".to_string(),
            iface_params: vec![],
            methods: vec![ImplMethodInfo {
                name: "show".to_string(),
                params: vec![array_ty.clone()],
                ret: ret_ty.clone(),
                ffi_symbol: Some("__kata_show__Array".to_string()),
            }],
        })
        .ok();

    let show_array = build_show_func(
        "__kata_show__Array",
        &array_ty,
        &ret_ty,
        &elem_ty,
        &bool_ty,
        true,
    );
    let show_rest = build_show_func(rest_name, &array_ty, &ret_ty, &elem_ty, &bool_ty, false);

    vec![show_array, show_rest]
}

// ════════════════════════════════════════════════════════════════
// Construção das TypedFunctions
// ════════════════════════════════════════════════════════════════

fn build_show_func(
    func_name: &str,
    array_ty: &Ty,
    ret_ty: &Ty,
    elem_ty: &Ty,
    bool_ty: &Ty,
    is_main: bool,
) -> TypedFunction {
    let (param_tys, patterns, body) = if is_main {
        let body = build_main_body(array_ty, elem_ty, bool_ty);
        (
            vec![array_ty.clone()],
            vec![ident_pattern("__self", array_ty)],
            body,
        )
    } else {
        let body = build_rest_body(array_ty, elem_ty, bool_ty);
        (
            vec![array_ty.clone(), Ty::int()],
            vec![
                ident_pattern("__self", array_ty),
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

/// `__kata_show__Array :: Array::A => Text`
///
/// ```text
/// match (= (len __self) 0)
///     True: "[]"
///     False:
///         match (at __self 0)
///             Ok(h): "[" + repr(h) + rest(__self, 1)
///             Err(_): "[]"   -- não deve acontecer
/// ```
fn build_main_body(array_ty: &Ty, elem_ty: &Ty, bool_ty: &Ty) -> TypedExpr {
    let self_spanned = self_expr(array_ty);
    let len_call = ffi_call1("kata_rt_array_len", self_spanned.clone(), Ty::int());
    let eq_scrutinee = eq_closure(len_call, int_lit_expr(0));
    let _ = bool_ty;

    // Braço True: "[]"
    let true_arm = boolean_arm("True", 0, text_lit("{}".to_string()));

    // Braço False: match (at __self 0) { Ok(h): ... ; Err(_): "[]" }
    let at_call = array_get_checked_call(self_spanned.clone(), int_lit_expr(0));
    let result_ty = Ty::Generic("Result".to_string(), vec![elem_ty.clone(), Ty::Unit]);

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
    let rest_call = rest_call_expr(self_spanned, int_lit_expr(1), array_ty);
    let ok_body = string_concat(text_lit("{".to_string()), string_concat(repr_h, rest_call));

    let ok_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
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
    let err_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Result".to_string(),
                variant: "Err".to_string(),
                sub_patterns: Some(vec![Spanned::new(
                    TypedPattern::Wildcard,
                    Span::synthetic(),
                )]),
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: text_lit("{}".to_string()),
    };

    let inner_match = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(at_call),
            arms: vec![ok_arm, err_arm],
        },
    };
    let _ = result_ty;

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

/// `__kata_show__Array_rest :: Array::A Int => Text`
///
/// ```text
/// match (= i (len __self))
///     True: "]"
///     False:
///         match (at __self i)
///             Ok(h): ", " + repr(h) + rest(__self, + i 1)
///             Err(_): "]"
/// ```
fn build_rest_body(array_ty: &Ty, elem_ty: &Ty, bool_ty: &Ty) -> TypedExpr {
    let self_spanned = self_expr(array_ty);
    let i_spanned = ident_expr("i", &Ty::int());
    let len_call = ffi_call1("kata_rt_array_len", self_spanned.clone(), Ty::int());
    let eq_scrutinee = eq_closure(i_spanned.clone(), len_call);
    let _ = bool_ty;

    let true_arm = boolean_arm("True", 0, text_lit("}".to_string()));

    // False: match (at __self i) { Ok(h): ... ; Err(_): "]" }
    let at_call = array_get_checked_call(self_spanned.clone(), i_spanned.clone());
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
    let rest_call = rest_call_expr(self_spanned, plus_call, array_ty);
    let ok_body = string_concat(text_lit(", ".to_string()), string_concat(repr_h, rest_call));

    let ok_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
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
    let err_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Result".to_string(),
                variant: "Err".to_string(),
                sub_patterns: Some(vec![Spanned::new(
                    TypedPattern::Wildcard,
                    Span::synthetic(),
                )]),
                tag: 1,
            },
            Span::synthetic(),
        )),
        guard: None,
        body: text_lit("}".to_string()),
    };

    let inner_match = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::text(),
        tail_pos: true,
        escape: EscapeTarget::Caller,
        kind: TypedExprKind::Match {
            scrutinee: Box::new(at_call),
            arms: vec![ok_arm, err_arm],
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
/// O monomorphizador resolve o overload de `=` para Int no DispatchTable.
/// Retorna Ty::Sum("Boolean") — o resultado é um Boolean Sum.
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

/// Constrói `kata_rt_array_get_checked(ptr, idx_smi)` — retorna Result Sum.
/// O idx (IntLit ou Ident) já é SMI-tagged pelo codegen.
fn array_get_checked_call(
    array: Spanned<TypedExpr>,
    idx: Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_array_get_checked".to_string(),
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
                args: vec![array, idx],
                ffi_symbol: Some("kata_rt_array_get_checked".to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `__kata_show__Array_rest __self idx` — chamada para a função rest.
fn rest_call_expr(
    self_spanned: Spanned<TypedExpr>,
    idx: Spanned<TypedExpr>,
    array_ty: &Ty,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![array_ty.clone(), Ty::int()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "__kata_show__Array_rest".to_string(),
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
                ffi_symbol: Some("__kata_show__Array_rest".to_string()),
            },
        },
        Span::synthetic(),
    )
}
