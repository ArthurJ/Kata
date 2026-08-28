//! Síntese de `show` para `Dict::K V` — hash map persistente (HAMT-backed).
//!
//! Gera duas TypedFunction genéricas mutuamente recursivas:
//!
//! - `__kata_show__Dict :: Dict::K V => Text`
//!   - `len == 0` → `"{}"`
//!   - `len > 0`  → `"{" + repr(K(next_smi(dict, 0))) + ": " + repr(V(next_smi(dict, 0))) + rest(dict, 1)`
//!
//! - `__kata_show__Dict_rest :: Dict::K V Int => Text`
//!   - `i == len` → `"}"`
//!   - `i < len`  → `", " + repr(K(next_smi(dict, i))) + ": " + repr(V(next_smi(dict, i))) + rest(dict, i+1)`
//!
//! Iteração via `kata_rt_dict_next_smi(dict_ptr, iter_state_smi, arena)` —
//! arena auto-injetada pelo codegen (ffi_needs_arena). `iter_state` é
//! parâmetro explícito (0=init, N=Nth entry, N>=count=None), passado como
//! SMI-tagged (IntLit → encode_smi). O wrapper decodifica SMI e chama
//! `kata_rt_dict_next` com valor bruto.
//!
//! Retorna `Optional::(K, V)` como Sum box: tag=0 Some(tuple_ptr), tag=1 None.
//! `tuple_ptr` aponta para 16 bytes: key@0, value@8. Extração via
//! `FieldAccess(kv, field_index=0)` para K, `FieldAccess(kv, field_index=1)`
//! para V — o codegen faz `load ptr + field_index * 8`.
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

/// Sintetiza `show` para `Dict::K V`.
pub(crate) fn synthesize_dict_show_functions(
    dispatch_table: &mut DispatchTable,
    interface_registry: &mut InterfaceRegistry,
) -> Vec<TypedFunction> {
    let k_param = "K";
    let v_param = "V";
    let k_ty = Ty::Var(k_param.to_string());
    let v_ty = Ty::Var(v_param.to_string());
    let dict_ty = Ty::Dict(Box::new(k_ty.clone()), Box::new(v_ty.clone()));
    let ret_ty = Ty::text();
    let int_ty = Ty::int();

    // ── show :: Dict::K V => Text ──
    dispatch_table.insert(OverloadInfo {
        name: "show".to_string(),
        params: vec![dict_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some("__kata_show__Dict".to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![k_param.to_string(), v_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── __kata_show__Dict_rest :: Dict::K V Int => Text ──
    let rest_name = "__kata_show__Dict_rest";
    dispatch_table.insert(OverloadInfo {
        name: rest_name.to_string(),
        params: vec![dict_ty.clone(), int_ty.clone()],
        ret: ret_ty.clone(),
        ffi_symbol: Some(rest_name.to_string()),
        is_action: false,
        is_generic: true,
        is_constructor: false,
        associative_neutral: None,
        type_params: vec![k_param.to_string(), v_param.to_string()],
        substitutions: None,
        param_names: vec![],
        param_defaults: vec![],
    });

    // ── Dict implements SHOW ──
    interface_registry
        .register_impl(ImplEntry {
            origin: "__synthesis".to_string(),
            type_name: "Dict".to_string(),
            type_params: vec![k_param.to_string(), v_param.to_string()],
            interface_name: "SHOW".to_string(),
            iface_params: vec![],
            methods: vec![ImplMethodInfo {
                name: "show".to_string(),
                params: vec![dict_ty.clone()],
                ret: ret_ty.clone(),
                ffi_symbol: Some("__kata_show__Dict".to_string()),
            }],
        })
        .ok();

    let show_dict = build_show_func("__kata_show__Dict", &dict_ty, &ret_ty, &k_ty, &v_ty, true);
    let show_rest = build_show_func(rest_name, &dict_ty, &ret_ty, &k_ty, &v_ty, false);

    vec![show_dict, show_rest]
}

// ════════════════════════════════════════════════════════════════
// Construção das TypedFunctions
// ════════════════════════════════════════════════════════════════

fn build_show_func(
    func_name: &str,
    dict_ty: &Ty,
    ret_ty: &Ty,
    k_ty: &Ty,
    v_ty: &Ty,
    is_main: bool,
) -> TypedFunction {
    let (param_tys, patterns, body) = if is_main {
        let body = build_main_body(dict_ty, k_ty, v_ty);
        (
            vec![dict_ty.clone()],
            vec![ident_pattern("__self", dict_ty)],
            body,
        )
    } else {
        let body = build_rest_body(dict_ty, k_ty, v_ty);
        (
            vec![dict_ty.clone(), Ty::int()],
            vec![
                ident_pattern("__self", dict_ty),
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

/// `__kata_show__Dict :: Dict::K V => Text`
///
/// ```text
/// match (= (len __self) 0)
///     True: "{}"
///     False:
///         match (dict_next_smi __self 0)
///             Some(kv): "{" + repr(kv.0) + ": " + repr(kv.1) + rest(__self, 1)
///             None: "{}"   -- não deve acontecer (len > 0)
/// ```
fn build_main_body(dict_ty: &Ty, k_ty: &Ty, v_ty: &Ty) -> TypedExpr {
    let self_spanned = self_expr(dict_ty);
    let len_call = ffi_call1("kata_rt_dict_len", self_spanned.clone(), Ty::int());
    let eq_scrutinee = eq_closure(len_call, int_lit_expr(0));

    // Braço True: "{}"
    let true_arm = boolean_arm("True", 0, text_lit("{}".to_string()));

    // Braço False: match (dict_next_smi __self 0) { Some(kv): ... ; None: "{}" }
    let next_call = dict_next_smi_call(self_spanned.clone(), int_lit_expr(0));

    // kv é o payload do Sum box = tuple_ptr (16 bytes: key@0, value@8)
    let kv_ty = Ty::int(); // tuple_ptr é i64
    let kv_expr = TypedExpr {
        span: Span::synthetic(),
        ty: kv_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kv".to_string(),
        },
    };
    let kv_spanned = Spanned::new(kv_expr, Span::synthetic());

    // Extrair K e V da tupla via FieldAccess: kv.0 = key, kv.1 = value
    let key_expr = field_access(kv_spanned.clone(), k_ty, 0);
    let val_expr = field_access(kv_spanned.clone(), v_ty, 1);

    let repr_key = repr_expr(key_expr, k_ty);
    let repr_val = repr_expr(val_expr, v_ty);
    let rest_call = rest_call_expr(self_spanned, int_lit_expr(1), dict_ty);

    // "{" + repr(K) + ": " + repr(V) + rest
    let ok_body = string_concat(
        text_lit("{".to_string()),
        string_concat(
            repr_key,
            string_concat(
                text_lit(": ".to_string()),
                string_concat(repr_val, rest_call),
            ),
        ),
    );

    let some_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Optional".to_string(),
                variant: "Some".to_string(),
                sub_patterns: Some(vec![Spanned::new(
                    TypedPattern::Ident {
                        name: "kv".to_string(),
                        ty: kv_ty.clone(),
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
        body: text_lit("{}".to_string()),
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

/// `__kata_show__Dict_rest :: Dict::K V Int => Text`
///
/// ```text
/// match (= i (len __self))
///     True: "}"
///     False:
///         match (dict_next_smi __self i)
///             Some(kv): ", " + repr(kv.0) + ": " + repr(kv.1) + rest(__self, + i 1)
///             None: "}"
/// ```
fn build_rest_body(dict_ty: &Ty, k_ty: &Ty, v_ty: &Ty) -> TypedExpr {
    let self_spanned = self_expr(dict_ty);
    let i_spanned = ident_expr("i", &Ty::int());
    let len_call = ffi_call1("kata_rt_dict_len", self_spanned.clone(), Ty::int());
    let eq_scrutinee = eq_closure(i_spanned.clone(), len_call);

    let true_arm = boolean_arm("True", 0, text_lit("}".to_string()));

    // False: match (dict_next_smi __self i) { Some(kv): ... ; None: "}" }
    let next_call = dict_next_smi_call(self_spanned.clone(), i_spanned.clone());

    let kv_ty = Ty::int();
    let kv_expr = TypedExpr {
        span: Span::synthetic(),
        ty: kv_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kv".to_string(),
        },
    };
    let kv_spanned = Spanned::new(kv_expr, Span::synthetic());

    let key_expr = field_access(kv_spanned.clone(), k_ty, 0);
    let val_expr = field_access(kv_spanned.clone(), v_ty, 1);

    let repr_key = repr_expr(key_expr, k_ty);
    let repr_val = repr_expr(val_expr, v_ty);

    // + i 1
    let plus_call = plus_closure(i_spanned, int_lit_expr(1));
    let rest_call = rest_call_expr(self_spanned, plus_call, dict_ty);

    // ", " + repr(K) + ": " + repr(V) + rest
    let ok_body = string_concat(
        text_lit(", ".to_string()),
        string_concat(
            repr_key,
            string_concat(
                text_lit(": ".to_string()),
                string_concat(repr_val, rest_call),
            ),
        ),
    );

    let some_arm = TypedMatchArm {
        pattern: Some(Spanned::new(
            TypedPattern::Variant {
                enum_name: "Optional".to_string(),
                variant: "Some".to_string(),
                sub_patterns: Some(vec![Spanned::new(
                    TypedPattern::Ident {
                        name: "kv".to_string(),
                        ty: kv_ty.clone(),
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
        body: text_lit("}".to_string()),
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

/// Constrói `kv.field_index` — FieldAccess para extrair K (index 0) ou V (index 1)
/// do tuple_ptr retornado por `kata_rt_dict_next_smi`.
///
/// O codegen faz `load ptr + field_index * 8`:
/// - field_index=0: lê key (offset 0)
/// - field_index=1: lê value (offset 8)
fn field_access(expr: Spanned<TypedExpr>, field_ty: &Ty, field_index: u32) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: field_ty.clone(),
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind: TypedExprKind::FieldAccess {
                expr: Box::new(expr),
                struct_name: String::new(),
                field_name: String::new(),
                field_index,
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `kata_rt_dict_next_smi(dict_ptr, iter_state)` — FFI call.
/// O codegen injeta arena automaticamente (ffi_needs_arena).
/// Retorna Optional Sum box (i64): Some(tuple_ptr) ou None.
fn dict_next_smi_call(
    dict: Spanned<TypedExpr>,
    iter_state: Spanned<TypedExpr>,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "kata_rt_dict_next_smi".to_string(),
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
                args: vec![dict, iter_state],
                ffi_symbol: Some("kata_rt_dict_next_smi".to_string()),
            },
        },
        Span::synthetic(),
    )
}

/// Constrói `__kata_show__Dict_rest __self idx` — chamada para a função rest.
fn rest_call_expr(
    self_spanned: Spanned<TypedExpr>,
    idx: Spanned<TypedExpr>,
    dict_ty: &Ty,
) -> Spanned<TypedExpr> {
    let callee = TypedExpr {
        span: Span::synthetic(),
        ty: Ty::Function(vec![dict_ty.clone(), Ty::int()], Box::new(Ty::text())),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: "__kata_show__Dict_rest".to_string(),
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
                ffi_symbol: Some("__kata_show__Dict_rest".to_string()),
            },
        },
        Span::synthetic(),
    )
}
